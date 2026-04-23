//! Ratatui rendering: colors, small text helpers, version-diff highlighting, and all `render_*`.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Cell, Gauge, Paragraph, Row, Table, Tabs};
use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::model::{AllUpgradablesOverlay, LIST_SCROLL_STEP, Package, PackageStatus};
use crate::overlay::overlay_filtered_rows;
use crate::pkg_manager::pip_uses_arch_pacman_for_global;
use crate::workers::privilege_hint_needs_sudo_reminder;

/// Palette used across every widget in `UniPack`.
struct AppColors {
    bg: Color,
    fg: Color,
    primary: Color,
    secondary: Color,
    accent: Color,
    warning: Color,
    error: Color,
    surface: Color,
    border: Color,
}

impl AppColors {
    const fn new() -> Self {
        Self {
            bg: Color::Rgb(26, 27, 38),
            fg: Color::Rgb(169, 177, 214),
            primary: Color::Rgb(122, 162, 247),
            secondary: Color::Rgb(187, 154, 247),
            accent: Color::Rgb(158, 206, 106),
            warning: Color::Rgb(224, 175, 104),
            error: Color::Rgb(247, 118, 142),
            surface: Color::Rgb(36, 40, 59),
            border: Color::Rgb(65, 72, 104),
        }
    }
}

const COLORS: AppColors = AppColors::new();

#[inline]
fn footer_key(label: &str) -> Span<'_> {
    Span::styled(label, Style::default().fg(COLORS.primary))
}

#[inline]
fn footer_hint(text: &str) -> Span<'_> {
    Span::styled(text, Style::default().fg(COLORS.secondary))
}

/// Heuristic progress for single-package upgrades.
///
/// We do not receive granular progress updates from package managers, so this returns a
/// monotonic estimate that moves quickly at first and then gradually slows, capped at 95%
/// until the worker reports completion.
//
// The cast to `u16` is safe: the computed percent is bounded to 8..=95 in every branch.
#[allow(clippy::cast_possible_truncation)]
pub const fn single_upgrade_percent(elapsed_ms: u64) -> u16 {
    // 0s..10s => 8%..80%, then 10s..45s => 80%..95%, after that clamp at 95%.
    if elapsed_ms <= 10_000 {
        let pct = 8_u64 + ((elapsed_ms * 72_u64) / 10_000_u64);
        return pct as u16;
    }
    if elapsed_ms <= 45_000 {
        let tail_ms = elapsed_ms - 10_000_u64;
        let pct = 80_u64 + ((tail_ms * 15_u64) / 35_000_u64);
        return pct as u16;
    }
    95
}

/// Truncates to fit `max_cols` display columns, then appends `…` when shortened.
fn clip_display_width(s: &str, max_cols: u16) -> String {
    let max = usize::from(max_cols);
    if max == 0 {
        return String::new();
    }
    if s.width() <= max {
        return s.to_owned();
    }
    let budget = max.saturating_sub(1);
    let mut out = String::new();
    let mut used = 0usize;
    for ch in s.chars() {
        let w = unicode_width::UnicodeWidthChar::width(ch)
            .unwrap_or(0)
            .max(1);
        if used + w > budget {
            break;
        }
        out.push(ch);
        used += w;
    }
    if out.is_empty() {
        "…".to_string()
    } else {
        format!("{out}…")
    }
}

/// One keybinding row inside a footer column: keys (accent) + description (muted).
fn footer_col_line<'a, I>(key_spans: I, hint: &'a str) -> Line<'a>
where
    I: IntoIterator<Item = Span<'a>>,
{
    let mut spans: Vec<Span<'a>> = key_spans.into_iter().collect();
    spans.push(footer_hint(hint));
    Line::from(spans)
}

/// Highlight for characters that differ in the installed version.
const DIFF_VERSION_RED: Color = Color::Rgb(247, 118, 142);
/// Highlight for characters that differ in the available version.
const DIFF_VERSION_GREEN: Color = Color::Rgb(158, 206, 106);

/// For each character index, `true` if that character is part of an LCS alignment (unchanged).
fn lcs_char_match_flags(old: &str, new: &str) -> (Vec<bool>, Vec<bool>) {
    let oldc: Vec<char> = old.chars().collect();
    let newc: Vec<char> = new.chars().collect();
    let n = oldc.len();
    let m = newc.len();
    let mut dp = vec![vec![0usize; m.saturating_add(1)]; n.saturating_add(1)];
    for i in 1..=n {
        for j in 1..=m {
            if oldc[i - 1] == newc[j - 1] {
                dp[i][j] = dp[i - 1][j - 1] + 1;
            } else {
                dp[i][j] = dp[i - 1][j].max(dp[i][j - 1]);
            }
        }
    }
    let mut old_matched = vec![false; n];
    let mut new_matched = vec![false; m];
    let mut i = n;
    let mut j = m;
    while i > 0 && j > 0 {
        if oldc[i - 1] == newc[j - 1] {
            old_matched[i - 1] = true;
            new_matched[j - 1] = true;
            i -= 1;
            j -= 1;
        } else if dp[i - 1][j] >= dp[i][j - 1] {
            i -= 1;
        } else {
            j -= 1;
        }
    }
    (old_matched, new_matched)
}

/// Appends `text` to `spans`, merging contiguous "diff" characters into a single styled span.
fn append_colored_chars(
    spans: &mut Vec<Span<'static>>,
    text: &str,
    matched: &[bool],
    base: Style,
    diff_fg: Color,
) {
    let mut buf = String::new();
    let mut prev_diff: Option<bool> = None;
    for (i, ch) in text.chars().enumerate() {
        let unchanged = matched.get(i).copied().unwrap_or(false);
        let is_diff = !unchanged;
        if Some(is_diff) != prev_diff && !buf.is_empty() {
            let style = if prev_diff == Some(true) {
                base.fg(diff_fg)
            } else {
                base
            };
            spans.push(Span::styled(std::mem::take(&mut buf), style));
        }
        prev_diff = Some(is_diff);
        buf.push(ch);
    }
    if !buf.is_empty() {
        let style = if prev_diff == Some(true) {
            base.fg(diff_fg)
        } else {
            base
        };
        spans.push(Span::styled(buf, style));
    }
}

/// Renders the version cell with LCS-based red/green diff highlighting when an upgrade is known.
fn version_cell_line(pkg: &Package, base: Style) -> Line<'static> {
    let Some(ref latest) = pkg.latest_version else {
        return Line::from(vec![Span::styled(pkg.version.clone(), base)]);
    };
    let (old_m, new_m) = lcs_char_match_flags(&pkg.version, latest);
    let mut spans = Vec::new();
    append_colored_chars(&mut spans, &pkg.version, &old_m, base, DIFF_VERSION_RED);
    spans.push(Span::styled(" -> ", base));
    append_colored_chars(&mut spans, latest, &new_m, base, DIFF_VERSION_GREEN);
    Line::from(spans)
}

/// Second info-strip line when the pip tab lists `python-*` pacman packages instead of `PyPI`.
fn pip_tab_uses_pacman_python_packages(app: &App) -> bool {
    app.package_managers
        .get(app.active_pm_index)
        .is_some_and(|p| p.name == "pip" && pip_uses_arch_pacman_for_global())
}

/// Rows reserved for the info strip: 2 when the pip-pacman note is shown, otherwise 1.
fn info_strip_height(app: &App) -> u16 {
    if pip_tab_uses_pacman_python_packages(app) {
        2
    } else {
        1
    }
}

/// Top-level frame layout: header, info strip, body (or overlay body), footer (or overlay footer).
pub fn render_app(frame: &mut Frame, app: &App) {
    let info_h = info_strip_height(app);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(info_h),
            Constraint::Min(0),
            Constraint::Length(4),
        ])
        .split(frame.area());

    render_header(frame, app, chunks[0]);
    render_info_strip(frame, app, chunks[1]);
    if let Some(ref overlay) = app.all_upgradables {
        render_all_upgradables_body(frame, overlay, chunks[2]);
        render_all_upgradables_footer(frame, app, chunks[3]);
    } else {
        render_body(frame, app, chunks[2]);
        render_footer(frame, app, chunks[3]);
    }
}

/// Single-line summary for the currently selected package (or active message/toast).
fn current_info_text(app: &App) -> String {
    if let Some(progress) = app.single_upgrade.as_ref() {
        let elapsed_s = progress.started_at.elapsed().as_secs();
        return format!(
            "Upgrading {} · {}s elapsed",
            progress.package_name, elapsed_s
        );
    }
    if let Some(msg) = app.message.as_ref() {
        return msg.clone();
    }
    app.filtered_packages()
        .get(app.selected_package_index)
        .map_or_else(
            || "— none —".to_string(),
            |(_, pkg)| {
                pkg.latest_version.as_ref().map_or_else(
                    || format!("{} {} · {}", pkg.name, pkg.version, pkg.status),
                    |latest| format!("{} {} → {} · {}", pkg.name, pkg.version, latest, pkg.status),
                )
            },
        )
}

const PIP_PACMAN_INFO_NOTE: &str =
    "pacman present: pip tab lists system `python-*` packages (pacman/yay/paru), not PyPI/pip.";

/// Renders the info strip (or progress gauge while a single upgrade runs).
fn render_info_strip(f: &mut Frame, app: &App, area: Rect) {
    let show_pip_pacman_note = pip_tab_uses_pacman_python_packages(app) && area.height >= 2;
    let note_area = |full: Rect| -> (Rect, Option<Rect>) {
        if show_pip_pacman_note {
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Length(1)])
                .split(full);
            (rows[0], Some(rows[1]))
        } else {
            (full, None)
        }
    };

    if let Some(progress) = app.single_upgrade.as_ref() {
        let elapsed_millis =
            u64::try_from(progress.started_at.elapsed().as_millis()).unwrap_or(u64::MAX);
        let elapsed_seconds = elapsed_millis / 1_000_u64;
        let pct = single_upgrade_percent(elapsed_millis);
        let gauge = Gauge::default()
            .gauge_style(Style::default().fg(COLORS.accent).bg(COLORS.surface))
            .label(format!(
                "Upgrading {} · {}s elapsed",
                progress.package_name, elapsed_seconds
            ))
            .percent(pct);
        let (main_area, note_row) = note_area(area);
        f.render_widget(gauge, main_area);
        if let Some(nr) = note_row {
            render_pip_pacman_note(f, nr);
        }
        return;
    }

    let raw_info = current_info_text(app);
    let info = clip_display_width(&raw_info, area.width);
    let info_color = if privilege_hint_needs_sudo_reminder(&raw_info) {
        COLORS.warning
    } else {
        COLORS.accent
    };
    let text = Paragraph::new(info)
        .style(Style::default().fg(info_color).bg(COLORS.surface))
        .alignment(Alignment::Left);

    let (main_area, note_row) = note_area(area);
    f.render_widget(text, main_area);
    if let Some(nr) = note_row {
        render_pip_pacman_note(f, nr);
    }
}

/// Draws the small "pip tab uses pacman" explanatory note into `area`.
fn render_pip_pacman_note(f: &mut Frame, area: Rect) {
    let note = clip_display_width(PIP_PACMAN_INFO_NOTE, area.width);
    f.render_widget(
        Paragraph::new(note)
            .style(Style::default().fg(COLORS.secondary).bg(COLORS.surface))
            .alignment(Alignment::Left),
        area,
    );
}

/// Top header row: title, PM tabs (with pending-update counts), and distro label.
fn render_header(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(20),
            Constraint::Min(0),
            Constraint::Length(20),
        ])
        .split(area);

    let title = Paragraph::new(" UniPack ")
        .style(Style::default().fg(COLORS.primary))
        .block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(COLORS.border)),
        )
        .alignment(Alignment::Center);

    let distro = Paragraph::new(app.distro.as_str())
        .style(Style::default().fg(COLORS.secondary))
        .block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(COLORS.border)),
        )
        .alignment(Alignment::Center);

    let pm_tabs = if app.package_managers.is_empty() {
        Tabs::new(vec!["No PMs".to_string()])
            .style(Style::default().fg(COLORS.fg))
            .select(0)
    } else {
        let names: Vec<String> = app
            .package_managers
            .iter()
            .enumerate()
            .map(
                |(i, pm)| match app.pm_pending_updates.get(i).copied().flatten() {
                    Some(n) if n > 0 => format!("{name} ({n})", name = pm.name),
                    _ => pm.name.clone(),
                },
            )
            .collect();
        Tabs::new(names)
            .style(Style::default().fg(COLORS.fg))
            .select(app.active_pm_index)
            .highlight_style(Style::default().fg(COLORS.primary).bg(COLORS.surface))
    };

    let pm_block = pm_tabs.block(
        Block::bordered()
            .title(" Package Managers ")
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(COLORS.border)),
    );

    f.render_widget(title, chunks[0]);
    f.render_widget(pm_block, chunks[1]);
    f.render_widget(distro, chunks[2]);
}

/// Main package table body (or a loading/empty placeholder).
fn render_body(f: &mut Frame, app: &App, area: Rect) {
    if app.loading {
        let msg = Paragraph::new("Loading packages...")
            .style(Style::default().fg(COLORS.fg))
            .alignment(Alignment::Center);
        f.render_widget(msg, area);
        return;
    }

    let filtered = app.filtered_packages();

    if filtered.is_empty() {
        let (msg, empty_fg) = if app.package_managers.is_empty() {
            ("No package managers detected", COLORS.error)
        } else if app.search_query.is_empty() {
            ("No packages found", COLORS.warning)
        } else {
            ("No packages match your search", COLORS.warning)
        };

        let msg = Paragraph::new(msg)
            .style(Style::default().fg(empty_fg))
            .alignment(Alignment::Center);
        f.render_widget(msg, area);
        return;
    }

    let selected_idx = filtered
        .iter()
        .map(|(i, _)| *i)
        .nth(app.selected_package_index);

    let visible_rows = (area.height as usize).saturating_sub(2);
    let max_scroll = filtered.len().saturating_sub(visible_rows);
    let half_visible = visible_rows / 2;
    let scroll_offset = app
        .selected_package_index
        .saturating_sub(half_visible)
        .min(max_scroll);

    let rows: Vec<_> = filtered
        .iter()
        .skip(scroll_offset)
        .take(visible_rows)
        .map(|(idx, pkg)| {
            let is_selected = Some(*idx) == selected_idx;
            let status_color = match pkg.status {
                PackageStatus::Installed => COLORS.accent,
                PackageStatus::Available => COLORS.fg,
                PackageStatus::Outdated => COLORS.warning,
                PackageStatus::Local => COLORS.secondary,
            };
            let style = if is_selected {
                Style::default().fg(COLORS.bg).bg(COLORS.primary)
            } else {
                Style::default().fg(COLORS.fg)
            };

            Row::new(vec![
                Cell::from(pkg.name.as_str()).style(style),
                Cell::from(version_cell_line(pkg, style)),
                Cell::from(pkg.status.to_string()).style(style.fg(status_color)),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        vec![
            Constraint::Percentage(35),
            Constraint::Percentage(25),
            Constraint::Percentage(20),
            Constraint::Min(0),
        ],
    )
    .block(
        Block::bordered()
            .title(" Packages ")
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(COLORS.border)),
    )
    .header(Row::new(vec!["Name", "Version", "Status"]).style(Style::default().fg(COLORS.primary)))
    .column_spacing(1);

    f.render_widget(table, area);
}

/// Main footer hints: either the search banner + 3 columns, or 4 columns of keybindings.
fn render_footer(f: &mut Frame, app: &App, area: Rect) {
    if app.search_mode {
        render_search_footer(f, &app.search_query, area);
        return;
    }
    render_keybinding_footer(f, app, area);
}

/// Footer when the main view is in search mode: top banner plus a 3-column hint row.
fn render_search_footer(f: &mut Frame, query: &str, area: Rect) {
    render_search_banner_and_hints(f, query, area);
}

/// Draws the yellow SEARCH banner over `area` and the shared search hint columns beneath it.
fn render_search_banner_and_hints(f: &mut Frame, query: &str, area: Rect) {
    let q = if query.is_empty() { "…" } else { query };
    let banner = Paragraph::new(Line::from(vec![
        Span::styled(
            " SEARCH  ",
            Style::default().fg(COLORS.bg).bg(COLORS.warning),
        ),
        Span::styled(
            format!(" {q} "),
            Style::default().fg(COLORS.warning).bg(COLORS.surface),
        ),
    ]))
    .alignment(Alignment::Left);

    let hint_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(2)])
        .split(area);
    f.render_widget(banner, hint_rows[0]);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(34),
            Constraint::Percentage(33),
            Constraint::Percentage(33),
        ])
        .split(hint_rows[1]);
    let c0 = Text::from(vec![
        footer_col_line([footer_key("Enter")], " keep filter "),
        footer_col_line([footer_key("Esc")], " clear search "),
    ]);
    let c1 = Text::from(vec![footer_col_line([footer_key("type")], " filter name ")]);
    let c2 = Text::from(vec![footer_col_line([footer_key("Bksp")], " delete char ")]);
    f.render_widget(Paragraph::new(c0).alignment(Alignment::Left), cols[0]);
    f.render_widget(Paragraph::new(c1).alignment(Alignment::Left), cols[1]);
    f.render_widget(Paragraph::new(c2).alignment(Alignment::Left), cols[2]);
}

/// Four-column main-view keybinding hint row.
fn render_keybinding_footer(f: &mut Frame, app: &App, area: Rect) {
    let o_hint = if app.show_outdated_only {
        " show all "
    } else {
        " upgradable only "
    };
    let step = LIST_SCROLL_STEP;
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ])
        .split(area);

    let col_move = Text::from(vec![
        footer_col_line([footer_key("↑↓"), footer_key(" j k")], " move (wrap) "),
        Line::from(vec![
            footer_key("Ctrl+d"),
            footer_hint(" "),
            footer_key("Ctrl+u"),
            Span::styled(
                format!(" page ±{step} "),
                Style::default().fg(COLORS.secondary),
            ),
        ]),
        Line::from(""),
    ]);
    let col_view = Text::from(vec![
        footer_col_line([footer_key("/")], " search "),
        footer_col_line([footer_key("o")], o_hint),
        footer_col_line([footer_key("Tab"), footer_key(" S-Tab")], " switch PM "),
    ]);
    let col_pkg = Text::from(vec![
        footer_col_line([footer_key("a")], " all upgrades "),
        footer_col_line([footer_key("u")], " upgrade "),
        footer_col_line([footer_key("r")], " remove "),
    ]);
    let col_sys = Text::from(vec![
        footer_col_line([footer_key("Ctrl+R")], " refresh "),
        footer_col_line([footer_key("q"), footer_key(" Esc")], " quit "),
    ]);
    f.render_widget(Paragraph::new(col_move).alignment(Alignment::Left), cols[0]);
    f.render_widget(Paragraph::new(col_view).alignment(Alignment::Left), cols[1]);
    f.render_widget(Paragraph::new(col_pkg).alignment(Alignment::Left), cols[2]);
    f.render_widget(Paragraph::new(col_sys).alignment(Alignment::Left), cols[3]);
}

/// Renders the all-upgradables overlay table into `area`.
fn render_all_upgradables_body(f: &mut Frame, overlay: &AllUpgradablesOverlay, area: Rect) {
    let filtered = overlay_filtered_rows(overlay);

    if filtered.is_empty() {
        let msg = Paragraph::new(if overlay.search_query.is_empty() {
            "No upgradable packages found"
        } else {
            "No packages match your search"
        })
        .style(Style::default().fg(COLORS.warning))
        .alignment(Alignment::Center);
        f.render_widget(msg, area);
        return;
    }

    let visible_rows = (area.height as usize).saturating_sub(2);
    let max_scroll = filtered.len().saturating_sub(visible_rows);
    let half_visible = visible_rows / 2;
    let scroll_offset = overlay.cursor.saturating_sub(half_visible).min(max_scroll);

    let rows: Vec<_> = filtered
        .iter()
        .skip(scroll_offset)
        .take(visible_rows)
        .enumerate()
        .map(|(visible_idx, (idx, row))| {
            let is_cursor = visible_idx + scroll_offset == overlay.cursor;
            let mark = if overlay.selected.contains(idx) {
                "[x]"
            } else {
                "[ ]"
            };
            let style = if is_cursor {
                Style::default().fg(COLORS.bg).bg(COLORS.primary)
            } else {
                Style::default().fg(COLORS.fg)
            };
            let pkg = row.as_package_for_display();
            Row::new(vec![
                Cell::from(mark).style(style),
                Cell::from(row.pm_name.as_str()).style(style),
                Cell::from(row.name.as_str()).style(style),
                Cell::from(version_cell_line(&pkg, style)),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(4),
            Constraint::Length(8),
            Constraint::Percentage(30),
            Constraint::Min(0),
        ],
    )
    .block(
        Block::bordered()
            .title(" All upgradable packages ")
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(COLORS.border)),
    )
    .header(Row::new(vec!["", "PM", "Name", "Version"]).style(Style::default().fg(COLORS.primary)))
    .column_spacing(1);

    f.render_widget(table, area);
}

/// Overlay footer: search banner in search mode, otherwise 3 hint columns + status/progress row.
fn render_all_upgradables_footer(f: &mut Frame, app: &App, area: Rect) {
    let overlay = app
        .all_upgradables
        .as_ref()
        .expect("overlay footer only rendered while overlay exists");

    if overlay.search_mode {
        render_search_banner_and_hints(f, &overlay.search_query, area);
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Length(1)])
        .split(area);

    render_overlay_keybinding_columns(f, rows[0]);
    render_overlay_status_row(f, app, overlay, rows[1]);
}

/// Draws the three overlay hint columns (navigation, selection, actions) into `area`.
fn render_overlay_keybinding_columns(f: &mut Frame, area: Rect) {
    let step = LIST_SCROLL_STEP;
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(34),
            Constraint::Percentage(33),
            Constraint::Percentage(33),
        ])
        .split(area);

    let col_nav = Text::from(vec![
        footer_col_line(
            [footer_key("Esc"), footer_hint("/"), footer_key("q")],
            " close ",
        ),
        footer_col_line([footer_key("↑↓"), footer_key(" j k")], " move "),
        Line::from(vec![
            footer_key("Ctrl+d"),
            footer_hint(" "),
            footer_key("Ctrl+u"),
            Span::styled(
                format!(" page ±{step} "),
                Style::default().fg(COLORS.secondary),
            ),
        ]),
    ]);
    let col_sel = Text::from(vec![
        footer_col_line([footer_key("Space")], " toggle row "),
        footer_col_line([footer_key("a")], " select all "),
        footer_col_line([footer_key("d")], " select none "),
    ]);
    let col_act = Text::from(vec![
        footer_col_line([footer_key("Shift+letter")], " toggle PM "),
        footer_col_line([footer_key("u")], " upgrade selected "),
        Line::from(""),
    ]);

    f.render_widget(Paragraph::new(col_nav).alignment(Alignment::Left), cols[0]);
    f.render_widget(Paragraph::new(col_sel).alignment(Alignment::Left), cols[1]);
    f.render_widget(Paragraph::new(col_act).alignment(Alignment::Left), cols[2]);
}

/// Status row below the overlay hint columns: either a bulk-upgrade gauge or "N selected".
fn render_overlay_status_row(
    f: &mut Frame,
    app: &App,
    overlay: &AllUpgradablesOverlay,
    area: Rect,
) {
    if let Some(progress) = app.multi_upgrade.as_ref() {
        let pct = multi_upgrade_percent(progress.total, progress.done, progress.current_started_at);
        let label = progress.current_package.as_ref().map_or_else(
            || format!("{}/{} complete", progress.done, progress.total),
            |pkg| format!("{}/{} · updating {}", progress.done, progress.total, pkg),
        );
        let gauge = Gauge::default()
            .gauge_style(Style::default().fg(COLORS.accent).bg(COLORS.surface))
            .label(label)
            .percent(pct);
        f.render_widget(gauge, area);
    } else {
        let count_line = Paragraph::new(format!("{} selected", overlay.selected.len()))
            .style(Style::default().fg(COLORS.accent))
            .alignment(Alignment::Right);
        f.render_widget(count_line, area);
    }
}

/// Computes the bulk-upgrade gauge percent. Done steps count fully; the current one ramps to 95%.
fn multi_upgrade_percent(
    total: usize,
    done: usize,
    current_started_at: Option<std::time::Instant>,
) -> u16 {
    if total == 0 {
        return 0;
    }
    let elapsed_ms = current_started_at.map_or(0_u128, |t| t.elapsed().as_millis());
    let sub_progress_per_mille =
        usize::try_from(((elapsed_ms * 1000) / 7000).min(950)).unwrap_or(950);
    let units_per_mille = done.saturating_mul(1000) + sub_progress_per_mille;
    let pct_usize = (units_per_mille.saturating_mul(100)) / (total.saturating_mul(1000));
    u16::try_from(pct_usize).unwrap_or(100)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_upgrade_progress_is_monotonic_and_capped() {
        let checkpoints = [0_u64, 1_000, 5_000, 10_000, 20_000, 45_000, 120_000];
        let mut prev = 0_u16;
        for ms in checkpoints {
            let pct = single_upgrade_percent(ms);
            assert!(pct >= prev, "progress regressed at {ms}ms");
            prev = pct;
        }
        assert_eq!(single_upgrade_percent(0), 8);
        assert_eq!(single_upgrade_percent(10_000), 80);
        assert_eq!(single_upgrade_percent(45_000), 95);
        assert_eq!(single_upgrade_percent(120_000), 95);
    }
}
