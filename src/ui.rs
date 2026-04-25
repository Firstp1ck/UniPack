//! Ratatui rendering entry point.
//!
//! This module is intentionally thin: the heavy lifting lives in the focused submodules below.
//! [`render_app`] is the only public surface used by the run loop.
//!
//! - [`theme`]: shared color palette and footer span helpers.
//! - [`text`]: display-width clipping and footer line composition.
//! - [`version_diff`]: LCS-based version diff highlighting.
//! - [`progress`]: heuristic single/multi upgrade progress percentages.
//! - [`scroll`]: scroll-window math for table bodies.
//! - [`main_view`]: header, info strip, body, and main footer.
//! - [`overlay_view`]: all-upgradables overlay body, footer, and status row.

mod main_view;
mod overlay_view;
mod progress;
mod scroll;
mod text;
mod theme;
mod version_diff;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};

use crate::app::App;

/// What: Top-level frame layout: header, info strip, body, and footer.
///
/// Inputs:
/// - `frame`: Ratatui frame to render into.
/// - `app`: Current application state used to drive every section.
///
/// Output:
/// - Side effect: widgets are rendered into `frame` covering the full area.
///
/// Details:
/// - When the all-upgradables overlay is active, the body and footer switch to the overlay
///   variants; otherwise the standard main-view body and footer are drawn.
pub fn render_app(frame: &mut Frame, app: &App) {
    let info_h = main_view::info_strip_height(app);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(info_h),
            Constraint::Min(0),
            Constraint::Length(4),
        ])
        .split(frame.area());

    main_view::render_header(frame, app, chunks[0]);
    main_view::render_info_strip(frame, app, chunks[1]);
    if let Some(ref overlay) = app.all_upgradables {
        overlay_view::render_all_upgradables_body(frame, overlay, chunks[2]);
        overlay_view::render_all_upgradables_footer(frame, app, chunks[3]);
    } else {
        main_view::render_body(frame, app, chunks[2]);
        main_view::render_footer(frame, app, chunks[3]);
    }
}
