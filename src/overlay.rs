//! All-upgradables overlay state mutations and key dispatch.
//!
//! Split into small helpers so each key handler stays well below the cognitive-complexity
//! threshold. The overlay renderer in [`crate::ui`] consumes [`overlay_filtered_rows`].

use crossterm::event::{KeyCode, KeyModifiers};

use crate::all_upgradables::UpgradableRow;
use crate::app::App;
use crate::model::{
    AllUpgradablesOverlay, LIST_SCROLL_STEP, MultiUpgradeProgress, MultiUpgradeProgressEvent,
    MultiUpgradeSender,
};
use crate::pkg_manager::PackageManager;

/// Launches the bulk-upgrade worker thread for whatever is currently selected.
pub fn upgrade_all_upgradables_selection(app: &mut App, multi_upgrade_tx: &MultiUpgradeSender) {
    let Some(overlay) = app.all_upgradables.as_mut() else {
        return;
    };
    if overlay.loading || overlay.selected.is_empty() || app.multi_upgrade.is_some() {
        return;
    }
    let tasks = collect_upgrade_tasks(overlay, &app.package_managers);
    if tasks.is_empty() {
        return;
    }
    overlay.selected.clear();
    app.multi_upgrade = Some(MultiUpgradeProgress {
        total: tasks.len(),
        done: 0,
        current_package: None,
        current_started_at: None,
    });
    app.message = Some(format!("Starting upgrade of {} package(s)...", tasks.len()));
    spawn_multi_upgrade_worker(tasks, multi_upgrade_tx.clone());
}

/// Extracts `(pm_index, pm, op_arg, display_name)` tuples for the currently selected rows.
fn collect_upgrade_tasks(
    overlay: &AllUpgradablesOverlay,
    managers: &[PackageManager],
) -> Vec<(usize, PackageManager, String, String)> {
    let mut tasks = Vec::with_capacity(overlay.selected.len());
    for idx in overlay.selected.iter().copied() {
        let Some(row) = overlay.rows.get(idx) else {
            continue;
        };
        let Some(pm) = managers.get(row.pm_index) else {
            continue;
        };
        let op_arg = row
            .upgrade_package_name
            .clone()
            .unwrap_or_else(|| row.name.clone());
        tasks.push((row.pm_index, pm.clone(), op_arg, row.name.clone()));
    }
    tasks
}

/// Spawns a single worker thread that upgrades the tasks sequentially and reports events.
fn spawn_multi_upgrade_worker(
    tasks: Vec<(usize, PackageManager, String, String)>,
    tx: MultiUpgradeSender,
) {
    std::thread::spawn(move || {
        for (pm_index, pm, op_arg, display_name) in tasks {
            let _ = tx.send(MultiUpgradeProgressEvent::StepStart {
                package_name: display_name.clone(),
            });
            let result = pm.upgrade_package(&op_arg);
            let _ = tx.send(MultiUpgradeProgressEvent::StepDone {
                pm_index,
                package_name: display_name,
                result,
            });
        }
        let _ = tx.send(MultiUpgradeProgressEvent::Finished);
    });
}

/// Selects every row in the overlay (ignoring any active search filter).
pub fn overlay_select_all_rows(app: &mut App) {
    if let Some(o) = app.all_upgradables.as_mut() {
        o.selected.clear();
        for i in 0..o.rows.len() {
            o.selected.insert(i);
        }
    }
}

/// Clears the overlay selection set.
pub fn overlay_deselect_all_rows(app: &mut App) {
    if let Some(o) = app.all_upgradables.as_mut() {
        o.selected.clear();
    }
}

/// Moves the overlay cursor by [`LIST_SCROLL_STEP`], clamped to list ends.
pub fn overlay_scroll_page(app: &mut App, down: bool) {
    let Some(o) = app.all_upgradables.as_mut() else {
        return;
    };
    let filtered_count = overlay_filtered_rows(o).len();
    if filtered_count == 0 {
        return;
    }
    let max = filtered_count - 1;
    o.cursor = if down {
        o.cursor.saturating_add(LIST_SCROLL_STEP).min(max)
    } else {
        o.cursor.saturating_sub(LIST_SCROLL_STEP)
    };
}

/// Returns the rows visible under the current overlay search query.
pub fn overlay_filtered_rows(overlay: &AllUpgradablesOverlay) -> Vec<(usize, &UpgradableRow)> {
    let query = if overlay.search_query.is_empty() {
        None
    } else {
        Some(overlay.search_query.to_lowercase())
    };
    overlay
        .rows
        .iter()
        .enumerate()
        .filter(|(_, row)| {
            query
                .as_deref()
                .is_none_or(|needle| overlay_row_matches_search(row, needle, overlay.search_fuzzy))
        })
        .collect()
}

/// Returns whether one overlay row matches the current query.
fn overlay_row_matches_search(row: &UpgradableRow, query: &str, fuzzy: bool) -> bool {
    overlay_search_match(row.name.as_str(), query, fuzzy)
        || overlay_search_match(row.pm_name.as_str(), query, fuzzy)
        || overlay_search_match(row.old_version.as_str(), query, fuzzy)
        || overlay_search_match(row.new_version.as_str(), query, fuzzy)
}

/// Returns whether one overlay field matches `query` in normal or fuzzy mode.
fn overlay_search_match(field: &str, query: &str, fuzzy: bool) -> bool {
    let lowered = field.to_lowercase();
    if fuzzy {
        overlay_fuzzy_subsequence_match(lowered.as_str(), query)
    } else {
        lowered.contains(query)
    }
}

/// Returns true when each `needle` character appears in order within `haystack`.
fn overlay_fuzzy_subsequence_match(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let mut needle_chars = needle.chars();
    let Some(mut current) = needle_chars.next() else {
        return true;
    };
    for h in haystack.chars() {
        if h == current {
            if let Some(next) = needle_chars.next() {
                current = next;
            } else {
                return true;
            }
        }
    }
    false
}

/// Clamps the cursor to the last visible row after the filter changes.
fn overlay_clamp_cursor(overlay: &mut AllUpgradablesOverlay) {
    let count = overlay_filtered_rows(overlay).len();
    if count > 0 {
        overlay.cursor = overlay.cursor.min(count - 1);
    } else {
        overlay.cursor = 0;
    }
}

/// Toggles overlay rows whose backend label starts with `letter` (ASCII, case-insensitive).
///
/// If every matching row is already selected, deselects all of them; otherwise selects all
/// matching rows.
fn overlay_select_rows_for_pm_first_letter(app: &mut App, letter: char) {
    let Some(o) = app.all_upgradables.as_mut() else {
        return;
    };
    let letter_lower = letter.to_ascii_lowercase();
    let matching: Vec<usize> = o
        .rows
        .iter()
        .enumerate()
        .filter_map(|(idx, row)| {
            let first = row.pm_name.chars().next()?;
            (first.to_ascii_lowercase() == letter_lower).then_some(idx)
        })
        .collect();
    if matching.is_empty() {
        return;
    }
    let all_selected = matching.iter().all(|&idx| o.selected.contains(&idx));
    if all_selected {
        for idx in matching {
            o.selected.remove(&idx);
        }
    } else {
        for idx in matching {
            o.selected.insert(idx);
        }
    }
}

/// Dispatches a key press while the overlay is open, delegating to helpers by keystroke group.
pub fn handle_all_upgradables_key(
    app: &mut App,
    code: KeyCode,
    modifiers: KeyModifiers,
    multi_upgrade_tx: &MultiUpgradeSender,
) {
    if app
        .all_upgradables
        .as_ref()
        .is_some_and(|overlay| overlay.search_mode)
    {
        handle_overlay_search_key(app, code, modifiers);
        return;
    }

    if handle_overlay_navigation(app, code, modifiers) {
        return;
    }
    let _ = handle_overlay_selection(app, code, modifiers, multi_upgrade_tx);
}

/// Handles key presses while the overlay search is active. Falls back to no-op for unknowns.
fn handle_overlay_search_key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    let Some(overlay) = app.all_upgradables.as_mut() else {
        return;
    };
    match code {
        KeyCode::Esc | KeyCode::Char('\u{1b}') => {
            overlay.search_mode = false;
            overlay.search_query.clear();
            overlay.cursor = 0;
        }
        KeyCode::Enter => {
            overlay.search_mode = false;
            overlay_clamp_cursor(overlay);
        }
        KeyCode::Backspace => {
            overlay.search_query.pop();
            overlay_clamp_cursor(overlay);
        }
        KeyCode::Char('f' | 'F') if modifiers.contains(KeyModifiers::CONTROL) => {
            overlay.search_fuzzy = !overlay.search_fuzzy;
            overlay_clamp_cursor(overlay);
        }
        KeyCode::Char(c) => {
            overlay.search_query.push(c);
            overlay_clamp_cursor(overlay);
        }
        _ => {}
    }
}

/// Handles close/scroll/movement keys. Returns `true` when the key was consumed.
fn handle_overlay_navigation(app: &mut App, code: KeyCode, modifiers: KeyModifiers) -> bool {
    match code {
        KeyCode::Esc | KeyCode::Char('\u{1b}') => {
            if let Some(overlay) = app.all_upgradables.as_mut() {
                if overlay.search_query.is_empty() {
                    app.all_upgradables = None;
                } else {
                    overlay.search_query.clear();
                    overlay.cursor = 0;
                }
            }
            true
        }
        KeyCode::Char('q') => {
            app.all_upgradables = None;
            true
        }
        KeyCode::Up | KeyCode::Char('k') => {
            overlay_cursor_step(app, -1);
            true
        }
        KeyCode::Down | KeyCode::Char('j') => {
            overlay_cursor_step(app, 1);
            true
        }
        KeyCode::Char('d' | 'D') if modifiers.contains(KeyModifiers::CONTROL) => {
            overlay_scroll_page(app, true);
            true
        }
        KeyCode::Char('\x04') => {
            overlay_scroll_page(app, true);
            true
        }
        KeyCode::Char('u' | 'U') if modifiers.contains(KeyModifiers::CONTROL) => {
            overlay_scroll_page(app, false);
            true
        }
        KeyCode::Char('\x15') => {
            overlay_scroll_page(app, false);
            true
        }
        KeyCode::Char('/') => {
            if let Some(o) = app.all_upgradables.as_mut() {
                o.search_mode = true;
            }
            true
        }
        _ => false,
    }
}

/// Moves the overlay cursor by `delta` (±1), wrapping around the filtered row count.
fn overlay_cursor_step(app: &mut App, delta: isize) {
    let Some(o) = app.all_upgradables.as_mut() else {
        return;
    };
    let n = overlay_filtered_rows(o).len();
    if n == 0 {
        return;
    }
    o.cursor = if delta >= 0 {
        (o.cursor + 1) % n
    } else {
        (o.cursor + n - 1) % n
    };
}

/// Handles Space / a / d / u / Shift+letter keys. Returns `true` when the key was consumed.
fn handle_overlay_selection(
    app: &mut App,
    code: KeyCode,
    modifiers: KeyModifiers,
    multi_upgrade_tx: &MultiUpgradeSender,
) -> bool {
    match code {
        KeyCode::Char(' ') => {
            overlay_toggle_cursor_row(app);
            true
        }
        KeyCode::Char(c) if modifiers.contains(KeyModifiers::SHIFT) && c.is_ascii_alphabetic() => {
            overlay_select_rows_for_pm_first_letter(app, c);
            true
        }
        KeyCode::Char('a' | 'A') if !modifiers.contains(KeyModifiers::SHIFT) => {
            overlay_select_all_rows(app);
            true
        }
        KeyCode::Char('d' | 'D')
            if !modifiers.contains(KeyModifiers::SHIFT)
                && !modifiers.contains(KeyModifiers::CONTROL) =>
        {
            overlay_deselect_all_rows(app);
            true
        }
        KeyCode::Char('u')
            if !modifiers.contains(KeyModifiers::SHIFT)
                && !modifiers.contains(KeyModifiers::CONTROL) =>
        {
            upgrade_all_upgradables_selection(app, multi_upgrade_tx);
            true
        }
        _ => false,
    }
}

/// Toggles the selection state for the row currently under the cursor.
fn overlay_toggle_cursor_row(app: &mut App) {
    let Some(o) = app.all_upgradables.as_mut() else {
        return;
    };
    let filtered = overlay_filtered_rows(o);
    if filtered.is_empty() {
        return;
    }
    if let Some((row_idx, _)) = filtered.get(o.cursor) {
        let idx = *row_idx;
        if !o.selected.remove(&idx) {
            o.selected.insert(idx);
        }
    }
}
