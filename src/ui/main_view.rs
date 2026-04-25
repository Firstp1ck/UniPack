//! Header, info strip, body table, and footer for the primary `UniPack` view.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Cell, Gauge, Paragraph, Row, Table, Tabs};

use crate::app::App;
use crate::model::{LIST_SCROLL_STEP, Package, PackageStatus};
use crate::pkg_manager::pip_uses_arch_pacman_for_global;
use crate::workers::privilege_hint_needs_sudo_reminder;

use super::progress::single_upgrade_percent;
use super::scroll::compute_scroll;
use super::text::{clip_display_width, footer_col_line, highlight_ascii_matches};
use super::theme::{COLORS, footer_hint, footer_key};
use super::version_diff::version_cell_line;

/// Body text shown beneath the info strip when the pip tab lists pacman python packages.
const PIP_PACMAN_INFO_NOTE: &str =
    "pacman present: pip tab lists system `python-*` packages (pacman/yay/paru), not PyPI/pip.";

/// True when the pip tab is showing system `python-*` packages instead of `PyPI`.
fn pip_tab_uses_pacman_python_packages(app: &App) -> bool {
    app.package_managers
        .get(app.active_pm_index)
        .is_some_and(|p| p.name == "pip" && pip_uses_arch_pacman_for_global())
}

/// Rows reserved for the info strip: 2 when the pip-pacman note is shown, otherwise 1.
pub fn info_strip_height(app: &App) -> u16 {
    if pip_tab_uses_pacman_python_packages(app) {
        2
    } else {
        1
    }
}

/// Splits an info-strip area into a main row and an optional pip-pacman note row.
fn split_info_strip(area: Rect, show_note: bool) -> (Rect, Option<Rect>) {
    if !show_note {
        return (area, None);
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);
    (rows[0], Some(rows[1]))
}

/// Renders the info strip (or progress gauge while a single upgrade runs).
pub fn render_info_strip(f: &mut Frame, app: &App, area: Rect) {
    let show_note = pip_tab_uses_pacman_python_packages(app) && area.height >= 2;
    let (main_area, note_row) = split_info_strip(area, show_note);

    if let Some(progress) = app.single_upgrade.as_ref() {
        render_single_upgrade_gauge(f, &progress.package_name, progress.started_at, main_area);
    } else {
        render_static_info_line(f, app, main_area);
    }

    if let Some(nr) = note_row {
        render_pip_pacman_note(f, nr);
    }
}

/// Draws the single-upgrade progress gauge into `area`.
fn render_single_upgrade_gauge(
    f: &mut Frame,
    package_name: &str,
    started_at: std::time::Instant,
    area: Rect,
) {
    let elapsed_millis = u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX);
    let elapsed_seconds = elapsed_millis / 1_000_u64;
    let pct = single_upgrade_percent(elapsed_millis);
    let gauge = Gauge::default()
        .gauge_style(Style::default().fg(COLORS.accent).bg(COLORS.surface))
        .label(format!(
            "Upgrading {package_name} · {elapsed_seconds}s elapsed"
        ))
        .percent(pct);
    f.render_widget(gauge, area);
}

/// Draws the static info line (selected package or message) into `area`.
fn render_static_info_line(f: &mut Frame, app: &App, area: Rect) {
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
    f.render_widget(text, area);
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
    selected_package_summary(app)
}

/// Builds the "name version[ -> latest] · status" string for the currently-selected package.
fn selected_package_summary(app: &App) -> String {
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
pub fn render_header(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(20),
            Constraint::Min(0),
            Constraint::Length(20),
        ])
        .split(area);

    f.render_widget(bordered_label(" UniPack ", COLORS.primary), chunks[0]);
    f.render_widget(pm_tabs_block(app), chunks[1]);
    f.render_widget(
        bordered_label(app.distro.as_str(), COLORS.secondary),
        chunks[2],
    );
}

/// Builds a centered, bordered label paragraph.
fn bordered_label(text: &str, fg: ratatui::style::Color) -> Paragraph<'_> {
    Paragraph::new(text)
        .style(Style::default().fg(fg))
        .block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(COLORS.border)),
        )
        .alignment(Alignment::Center)
}

/// Builds the package-manager `Tabs` widget (with pending-update counts) wrapped in a border.
fn pm_tabs_block(app: &App) -> Tabs<'_> {
    let tabs = if app.package_managers.is_empty() {
        Tabs::new(vec!["No PMs".to_string()])
            .style(Style::default().fg(COLORS.fg))
            .select(0)
    } else {
        Tabs::new(pm_tab_labels(app))
            .style(Style::default().fg(COLORS.fg))
            .select(app.active_pm_index)
            .highlight_style(Style::default().fg(COLORS.primary).bg(COLORS.surface))
    };
    tabs.block(
        Block::bordered()
            .title(" Package Managers ")
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(COLORS.border)),
    )
}

/// Builds the user-visible labels for each PM tab, suffixing pending counts when nonzero.
fn pm_tab_labels(app: &App) -> Vec<String> {
    app.package_managers
        .iter()
        .enumerate()
        .map(
            |(i, pm)| match app.pm_pending_updates.get(i).copied().flatten() {
                Some(n) if n > 0 => format!("{name} ({n})", name = pm.name),
                _ => pm.name.clone(),
            },
        )
        .collect()
}

/// Main package table body (or a loading/empty placeholder).
pub fn render_body(f: &mut Frame, app: &App, area: Rect) {
    if app.loading {
        render_centered_message(f, "Loading packages...", COLORS.fg, area);
        return;
    }

    let filtered = app.filtered_packages();
    if filtered.is_empty() {
        let (msg, color) = body_empty_message(app);
        render_centered_message(f, msg, color, area);
        return;
    }

    let window = compute_scroll(area.height, filtered.len(), app.selected_package_index);
    let selected_idx = filtered
        .iter()
        .map(|(i, _)| *i)
        .nth(app.selected_package_index);

    let rows: Vec<Row<'_>> = filtered
        .iter()
        .skip(window.offset)
        .take(window.visible_rows)
        .map(|(idx, pkg)| body_row(pkg, Some(*idx) == selected_idx, app))
        .collect();

    f.render_widget(packages_table(rows), area);
}

/// Picks the empty-body message and its color for the current `App` state.
const fn body_empty_message(app: &App) -> (&'static str, ratatui::style::Color) {
    if app.package_managers.is_empty() {
        ("No package managers detected", COLORS.error)
    } else if app.search_query.is_empty() {
        ("No packages found", COLORS.warning)
    } else {
        ("No packages match your search", COLORS.warning)
    }
}

/// Draws a centered single-line message into `area`.
fn render_centered_message(f: &mut Frame, msg: &str, fg: ratatui::style::Color, area: Rect) {
    let p = Paragraph::new(msg)
        .style(Style::default().fg(fg))
        .alignment(Alignment::Center);
    f.render_widget(p, area);
}

/// Builds a single body table row for a package.
fn body_row<'a>(pkg: &'a Package, is_selected: bool, app: &'a App) -> Row<'a> {
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
    let highlighted_name = highlight_ascii_matches(
        pkg.name.as_str(),
        if app.search_mode {
            app.search_query.as_str()
        } else {
            ""
        },
        app.search_mode && app.search_fuzzy,
        style,
        style.fg(COLORS.warning),
    );

    Row::new(vec![
        Cell::from(highlighted_name),
        Cell::from(version_cell_line(pkg, style)),
        Cell::from(pkg.status.to_string()).style(style.fg(status_color)),
    ])
}

/// Wraps a precomputed set of rows into the styled `Packages` table widget.
fn packages_table(rows: Vec<Row<'_>>) -> Table<'_> {
    Table::new(
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
    .column_spacing(1)
}

/// Main footer hints: either the search banner + 3 columns, or 4 columns of keybindings.
pub fn render_footer(f: &mut Frame, app: &App, area: Rect) {
    if app.search_mode {
        render_search_banner_and_hints(f, &app.search_query, app.search_fuzzy, area);
    } else {
        render_keybinding_footer(f, app, area);
    }
}

/// Draws the yellow SEARCH banner over `area` and the shared search hint columns beneath it.
pub fn render_search_banner_and_hints(f: &mut Frame, query: &str, fuzzy: bool, area: Rect) {
    let q = if query.is_empty() { "…" } else { query };
    let mode_label = if fuzzy { "FUZZY" } else { "NORMAL" };
    let banner = Paragraph::new(Line::from(vec![
        Span::styled(
            " SEARCH ",
            Style::default().fg(COLORS.bg).bg(COLORS.warning),
        ),
        Span::styled(
            format!(" {mode_label} "),
            Style::default().fg(COLORS.bg).bg(COLORS.accent),
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
    render_search_hint_columns(f, fuzzy, hint_rows[1]);
}

/// Three-column layout under the SEARCH banner: keep filter / type / backspace hints.
fn render_search_hint_columns(f: &mut Frame, fuzzy: bool, area: Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(34),
            Constraint::Percentage(33),
            Constraint::Percentage(33),
        ])
        .split(area);
    let c0 = Text::from(vec![
        footer_col_line([footer_key("Enter")], " keep filter "),
        footer_col_line([footer_key("Esc")], " clear search "),
    ]);
    let toggle_hint = if fuzzy {
        " switch to normal "
    } else {
        " switch to fuzzy "
    };
    let c1 = Text::from(vec![
        footer_col_line([footer_key("type")], " filter name "),
        footer_col_line([footer_key("Ctrl+f")], toggle_hint),
    ]);
    let c2 = Text::from(vec![footer_col_line([footer_key("Bksp")], " delete char ")]);
    f.render_widget(Paragraph::new(c0).alignment(Alignment::Left), cols[0]);
    f.render_widget(Paragraph::new(c1).alignment(Alignment::Left), cols[1]);
    f.render_widget(Paragraph::new(c2).alignment(Alignment::Left), cols[2]);
}

/// Four-column main-view keybinding hint row.
fn render_keybinding_footer(f: &mut Frame, app: &App, area: Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ])
        .split(area);

    let outdated_hint = if app.show_outdated_only {
        " show all "
    } else {
        " upgradable only "
    };

    f.render_widget(
        Paragraph::new(col_move_keys()).alignment(Alignment::Left),
        cols[0],
    );
    f.render_widget(
        Paragraph::new(col_view_keys(outdated_hint)).alignment(Alignment::Left),
        cols[1],
    );
    f.render_widget(
        Paragraph::new(col_pkg_keys()).alignment(Alignment::Left),
        cols[2],
    );
    f.render_widget(
        Paragraph::new(col_sys_keys()).alignment(Alignment::Left),
        cols[3],
    );
}

/// Movement-related footer column.
fn col_move_keys() -> Text<'static> {
    let step = LIST_SCROLL_STEP;
    Text::from(vec![
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
    ])
}

/// View-related footer column (search, outdated filter, switch PM).
fn col_view_keys(outdated_hint: &str) -> Text<'_> {
    Text::from(vec![
        footer_col_line([footer_key("/")], " search "),
        footer_col_line([footer_key("o")], outdated_hint),
        footer_col_line([footer_key("Tab"), footer_key(" S-Tab")], " switch PM "),
    ])
}

/// Package-action footer column.
fn col_pkg_keys() -> Text<'static> {
    Text::from(vec![
        footer_col_line([footer_key("a")], " all upgrades "),
        footer_col_line([footer_key("u")], " upgrade "),
        footer_col_line([footer_key("Del")], " remove "),
    ])
}

/// System-action footer column (refresh, quit).
fn col_sys_keys() -> Text<'static> {
    Text::from(vec![
        footer_col_line([footer_key("r")], " refresh "),
        footer_col_line([footer_key("q"), footer_key(" Esc")], " quit "),
    ])
}
