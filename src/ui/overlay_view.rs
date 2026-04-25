//! All-upgradables overlay: body table, status row, and overlay-specific footer hints.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Cell, Gauge, Paragraph, Row, Table};

use crate::app::App;
use crate::model::{AllUpgradablesOverlay, LIST_SCROLL_STEP};
use crate::overlay::{full_update_candidate_backend_names, overlay_filtered_rows};

use super::main_view::render_search_banner_and_hints;
use super::progress::multi_upgrade_percent;
use super::scroll::compute_scroll;
use super::text::{footer_col_line, highlight_ascii_matches};
use super::theme::{COLORS, footer_hint, footer_key};
use super::version_diff::version_cell_line;

/// Renders the all-upgradables overlay table into `area`.
pub fn render_all_upgradables_body(f: &mut Frame, overlay: &AllUpgradablesOverlay, area: Rect) {
    let filtered = overlay_filtered_rows(overlay);
    if filtered.is_empty() {
        render_overlay_empty(f, overlay, area);
        return;
    }

    let window = compute_scroll(area.height, filtered.len(), overlay.cursor);
    let rows: Vec<Row<'_>> = filtered
        .iter()
        .skip(window.offset)
        .take(window.visible_rows)
        .enumerate()
        .map(|(visible_idx, (idx, row))| {
            let is_cursor = visible_idx + window.offset == overlay.cursor;
            let is_selected = overlay.selected.contains(idx);
            overlay_row(row, is_cursor, is_selected, overlay)
        })
        .collect();

    f.render_widget(overlay_table(rows), area);
}

/// Draws the empty-overlay message into `area`.
fn render_overlay_empty(f: &mut Frame, overlay: &AllUpgradablesOverlay, area: Rect) {
    let msg = if overlay.search_query.is_empty() {
        "No upgradable packages found"
    } else {
        "No packages match your search"
    };
    let p = Paragraph::new(msg)
        .style(Style::default().fg(COLORS.warning))
        .alignment(Alignment::Center);
    f.render_widget(p, area);
}

/// Builds a single overlay row (selection mark, PM, name, version).
fn overlay_row<'a>(
    row: &'a crate::all_upgradables::UpgradableRow,
    is_cursor: bool,
    is_selected: bool,
    overlay: &'a AllUpgradablesOverlay,
) -> Row<'a> {
    let mark = if is_selected { "[x]" } else { "[ ]" };
    let style = if is_cursor {
        Style::default().fg(COLORS.bg).bg(COLORS.primary)
    } else {
        Style::default().fg(COLORS.fg)
    };
    let pkg = row.as_package_for_display();
    let highlighted_name = highlight_ascii_matches(
        row.name.as_str(),
        if overlay.search_mode {
            overlay.search_query.as_str()
        } else {
            ""
        },
        overlay.search_mode && overlay.search_fuzzy,
        style,
        style.fg(COLORS.warning),
    );
    Row::new(vec![
        Cell::from(mark).style(style),
        Cell::from(row.pm_name.clone()).style(style),
        Cell::from(highlighted_name),
        Cell::from(version_cell_line(&pkg, style)),
    ])
}

/// Wraps overlay rows into the bordered `All upgradable packages` table widget.
fn overlay_table(rows: Vec<Row<'_>>) -> Table<'_> {
    Table::new(
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
    .column_spacing(1)
}

/// Overlay footer: search banner in search mode, otherwise 3 hint columns + status/progress row.
pub fn render_all_upgradables_footer(f: &mut Frame, app: &App, area: Rect) {
    let Some(overlay) = app.all_upgradables.as_ref() else {
        return;
    };

    if overlay.search_mode {
        render_search_banner_and_hints(f, &overlay.search_query, overlay.search_fuzzy, area);
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
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(34),
            Constraint::Percentage(33),
            Constraint::Percentage(33),
        ])
        .split(area);

    f.render_widget(
        Paragraph::new(col_overlay_nav()).alignment(Alignment::Left),
        cols[0],
    );
    f.render_widget(
        Paragraph::new(col_overlay_sel()).alignment(Alignment::Left),
        cols[1],
    );
    f.render_widget(
        Paragraph::new(col_overlay_act()).alignment(Alignment::Left),
        cols[2],
    );
}

/// Navigation column for the overlay footer.
fn col_overlay_nav() -> Text<'static> {
    let step = LIST_SCROLL_STEP;
    Text::from(vec![
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
    ])
}

/// Selection column for the overlay footer.
fn col_overlay_sel() -> Text<'static> {
    Text::from(vec![
        footer_col_line([footer_key("Space")], " toggle row "),
        footer_col_line([footer_key("a")], " select all "),
        footer_col_line([footer_key("d")], " select none "),
    ])
}

/// Action column for the overlay footer.
fn col_overlay_act() -> Text<'static> {
    Text::from(vec![
        footer_col_line([footer_key("Shift+letter")], " toggle PM "),
        footer_col_line(
            [footer_key("u")],
            " upgrade selected (full-update where eligible) ",
        ),
        Line::from(""),
    ])
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
        let full_update_candidates = full_update_candidate_backend_names(app, overlay);
        let status = if full_update_candidates.is_empty() {
            format!("{} selected", overlay.selected.len())
        } else {
            format!(
                "{} selected · full-update candidate: {}",
                overlay.selected.len(),
                full_update_candidates.join(", ")
            )
        };
        let count_line = Paragraph::new(status)
            .style(Style::default().fg(COLORS.accent))
            .alignment(Alignment::Right);
        f.render_widget(count_line, area);
    }
}
