//! TUI entry point: terminal lifecycle, channel wiring, and the draw / input loop.

use std::collections::BTreeSet;
use std::time::Instant;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};

use crate::all_upgradables::collect_upgradables_from_cached_lists;
use crate::app::{App, PendingMirrorRetry};
use crate::detect::{offer_sudo_warm_before_tui, pip_pacman_op_arg};
use crate::model::{
    AllUpgradablesOverlay, LIST_SCROLL_STEP, MultiUpgradeProgressEvent, MultiUpgradeReceiver,
    MultiUpgradeSender, PackageListReceiver, PackageListSender, PreloadChannelMsg, PreloadReceiver,
    SingleUpgradeProgress, SingleUpgradeReceiver, SingleUpgradeSender, UpdateCountReceiver,
    UpdateCountSender, UpgradeMapChannelMsg, UpgradeMapReceiver,
};
use crate::overlay::{handle_all_upgradables_key, refresh_overlay_opened_metadata};
use crate::ui::render_app;
use crate::workers::{
    advance_upgrade_merge_chunk, cycle_active_pm, maybe_show_privilege_hint, pump_preloads,
    refresh_preload_queue, spawn_update_refresh, try_recv_package_list_results,
    try_recv_preload_results, try_recv_upgrade_metadata,
};

/// Accumulator for per-iteration bulk-upgrade aggregation.
#[derive(Default)]
struct MultiUpgradeAgg {
    ok: usize,
    errors: Vec<String>,
    successes: Vec<(usize, String)>,
    full_system_successes: BTreeSet<usize>,
}

/// All MPSC endpoints threaded through the main loop.
struct RunChannels {
    upgrade_rx: UpgradeMapReceiver,
    pkg_rx: PackageListReceiver,
    single_upgrade_tx: SingleUpgradeSender,
    single_upgrade_rx: SingleUpgradeReceiver,
    multi_upgrade_tx: MultiUpgradeSender,
    multi_upgrade_rx: MultiUpgradeReceiver,
    preload_rx: PreloadReceiver,
    update_tx: UpdateCountSender,
    update_rx: UpdateCountReceiver,
}

impl RunChannels {
    /// Creates all channels used by the main loop and wires the app's sender handles.
    fn new(app: &mut App) -> (Self, PackageListSender) {
        let (upgrade_tx, upgrade_rx) = std::sync::mpsc::channel::<UpgradeMapChannelMsg>();
        app.upgrade_map_tx = Some(upgrade_tx);

        let (pkg_tx, pkg_rx) = std::sync::mpsc::channel::<crate::model::PackageListChannelMsg>();
        let (single_upgrade_tx, single_upgrade_rx) = std::sync::mpsc::channel();
        let (multi_upgrade_tx, multi_upgrade_rx) = std::sync::mpsc::channel();

        let (preload_tx, preload_rx) = std::sync::mpsc::channel::<PreloadChannelMsg>();
        app.preload_result_tx = Some(preload_tx);

        let (update_tx, update_rx) = std::sync::mpsc::channel();

        (
            Self {
                upgrade_rx,
                pkg_rx,
                single_upgrade_tx,
                single_upgrade_rx,
                multi_upgrade_tx,
                multi_upgrade_rx,
                preload_rx,
                update_tx,
                update_rx,
            },
            pkg_tx,
        )
    }
}

/// Initializes the terminal, runs the main keyboard/draw loop, then restores the screen.
///
/// # Panics
///
/// Panics if [`App::new`] fails (for example invalid internal state during startup).
pub fn run() {
    if handle_help_flag() {
        return;
    }

    let mut app = App::new().expect("Failed to create app");
    warm_sudo_if_chosen(&mut app);

    let mut terminal = ratatui::init();
    enter_alternate_screen(&mut terminal);

    let (channels, pkg_tx) = RunChannels::new(&mut app);
    start_initial_background_work(&mut app, &pkg_tx, &channels.update_tx);
    drop(pkg_tx);

    let mut agg = MultiUpgradeAgg::default();

    while !tick(&mut app, &mut terminal, &channels, &mut agg) {}

    leave_alternate_screen(&mut terminal);
    app.persist_package_disk_cache_best_effort();
    ratatui::restore();
}

/// Switches to the alternate screen buffer, ignoring failures.
fn enter_alternate_screen(terminal: &mut ratatui::DefaultTerminal) {
    let backend = terminal.backend_mut();
    let _ = execute!(backend, EnterAlternateScreen);
}

/// Returns to the main screen buffer, ignoring failures.
fn leave_alternate_screen(terminal: &mut ratatui::DefaultTerminal) {
    let backend = terminal.backend_mut();
    let _ = execute!(backend, LeaveAlternateScreen);
}

/// Prints the CLI help when `-h` / `--help` is in `argv`. Returns `true` when help was printed.
fn handle_help_flag() -> bool {
    let args: Vec<String> = std::env::args().collect();
    if !args.iter().any(|a| a == "-h" || a == "--help") {
        return false;
    }
    print_help();
    true
}

/// Writes the keyboard-shortcut help text to stdout.
fn print_help() {
    println!("📦 UniPack - Package Manager TUI");
    println!();
    println!("Usage: unipack [OPTIONS]");
    println!();
    println!("Options:");
    println!("  -h, --help     Show this help message");
    println!();
    println!("Keyboard Shortcuts:");
    println!("  ↑/↓ or j/k    Navigate package list (wraps)");
    println!(
        "  Ctrl+d / Ctrl+u    Move cursor down/up by {LIST_SCROLL_STEP} lines (clamped to list ends)"
    );
    println!("  /             Toggle search mode");
    println!("  Ctrl+f        Toggle normal/fuzzy search (while searching)");
    println!("  a             All upgradable packages (from cached lists; visit tabs to fill)");
    println!(
        "                In that view: Space toggle, a all, d none, Shift+PM letter toggles that PM, u upgrade (full-update where eligible), Ctrl+d/u page, Esc/q close"
    );
    println!("  u             Upgrade selected package");
    println!("  Delete        Remove selected package");
    println!("  Tab           Switch package manager (forward)");
    println!("  Shift+Tab     Switch package manager (backward)");
    println!("  r             Refresh package list");
    println!("  o             Toggle show only upgradable packages");
    println!("  Esc           Leave search, or quit when not searching");
    println!("  q             Quit when not in search (in search, types into query)");
    println!();
    println!("Supported Package Managers:");
    println!("  pip, npm, pnpm, bun, cargo, apt, pacman, aur, rpm, flatpak, snap, brew");
    println!();
    println!("Privilege note:");
    println!(
        "  UniPack runs package commands non-interactively. For apt/pacman/aur/rpm/snap actions,"
    );
    println!(
        "  and the pip tab when pacman is present (python-* packages), you can opt in to `sudo -v`"
    );
    println!("  at startup (interactive terminal), or run `sudo -v` yourself before upgrading.");
}

/// Prompts the user to run `sudo -v`; exits the process on sudo errors.
fn warm_sudo_if_chosen(app: &mut App) {
    match offer_sudo_warm_before_tui(&app.package_managers) {
        Ok(true) => app.sudo_session_enabled = true,
        Ok(false) => {}
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}

/// Kicks off the initial list load, preload queue, privilege hint, and update-count refresh.
fn start_initial_background_work(
    app: &mut App,
    pkg_tx: &PackageListSender,
    update_tx: &UpdateCountSender,
) {
    match app.package_managers.get(app.active_pm_index) {
        Some(pm) if pm.available => {
            let pm = pm.clone();
            app.begin_background_list_load(0, pm, pkg_tx);
        }
        Some(pm) => {
            app.message = Some(format!("{} is not available", pm.name));
        }
        None => {}
    }

    refresh_preload_queue(app, false);
    maybe_show_privilege_hint(app);
    spawn_update_refresh(&app.package_managers, update_tx);
}

/// One iteration of the main loop. Returns `true` when the loop should exit.
fn tick(
    app: &mut App,
    terminal: &mut ratatui::DefaultTerminal,
    channels: &RunChannels,
    agg: &mut MultiUpgradeAgg,
) -> bool {
    drain_background_events(app, channels, agg);
    let _ = terminal.draw(|f| render_app(f, app));
    poll_and_handle_input(app, channels)
}

/// Drains all background MPSC channels in the exact order the original loop used.
fn drain_background_events(app: &mut App, channels: &RunChannels, agg: &mut MultiUpgradeAgg) {
    drain_update_counts(app, &channels.update_rx);
    drain_single_upgrade(app, &channels.single_upgrade_rx, &channels.update_tx);
    drain_multi_upgrade(app, channels, agg);
    try_recv_package_list_results(app, &channels.pkg_rx);
    try_recv_preload_results(app, &channels.preload_rx);
    pump_preloads(app);
    try_recv_upgrade_metadata(app, &channels.upgrade_rx);
    advance_upgrade_merge_chunk(app);
}

/// Copies per-PM pending-update counts from worker threads into `app.pm_pending_updates`.
fn drain_update_counts(app: &mut App, update_rx: &UpdateCountReceiver) {
    while let Ok((idx, count)) = update_rx.try_recv() {
        if let Some(slot) = app.pm_pending_updates.get_mut(idx) {
            *slot = count;
        }
    }
}

/// Completes a single-package upgrade: toast, refresh list + pending counts.
fn drain_single_upgrade(app: &mut App, rx: &SingleUpgradeReceiver, update_tx: &UpdateCountSender) {
    while let Ok((name, result)) = rx.try_recv() {
        app.single_upgrade = None;
        match result {
            Ok(_) => {
                app.message = Some(format!("Upgraded {name}"));
                app.load_packages_sync();
                for slot in &mut app.pm_pending_updates {
                    *slot = None;
                }
                spawn_update_refresh(&app.package_managers, update_tx);
            }
            Err(e) => {
                let err_text = e.to_string();
                if should_offer_mirror_retry(&err_text) {
                    maybe_prepare_mirror_retry(app, &name);
                    continue;
                }
                app.message = Some(format!("Error: {e}"));
            }
        }
    }
}

/// Detects the localized pacman/yay "already current -- reinstalling" warning.
fn should_offer_mirror_retry(err: &str) -> bool {
    let lower = err.to_lowercase();
    lower.contains("ist aktuell") && lower.contains("reinstall")
}

/// Stores retry context and asks for `y/n` confirmation in the info strip.
fn maybe_prepare_mirror_retry(app: &mut App, display_name: &str) {
    let Some((_, p)) = app
        .filtered_packages()
        .get(app.selected_package_index)
        .copied()
    else {
        app.message = Some(
            "Update appears out-of-sync. Refresh mirrors manually, then retry upgrade.".to_string(),
        );
        return;
    };
    let Some(pm) = app.active_pm() else {
        app.message =
            Some("Update appears out-of-sync. Retry after selecting a package.".to_string());
        return;
    };
    let op_arg = pip_pacman_op_arg(&pm, p);
    app.pending_mirror_retry = Some(PendingMirrorRetry {
        pm,
        package_display: display_name.to_string(),
        package_op_arg: op_arg,
    });
    app.message = Some(
        "Package is reported as current. Refresh mirrors and retry upgrade? [y/N]".to_string(),
    );
}

/// Handles start / done / finished events for an in-progress bulk overlay upgrade.
fn drain_multi_upgrade(app: &mut App, channels: &RunChannels, agg: &mut MultiUpgradeAgg) {
    while let Ok(event) = channels.multi_upgrade_rx.try_recv() {
        match event {
            MultiUpgradeProgressEvent::StepStart { package_name } => {
                on_multi_upgrade_step_start(app, &package_name);
            }
            MultiUpgradeProgressEvent::StepDone {
                pm_index,
                package_name,
                used_full_system_update,
                result,
            } => {
                on_multi_upgrade_step_done(
                    app,
                    agg,
                    pm_index,
                    package_name,
                    used_full_system_update,
                    result,
                );
            }
            MultiUpgradeProgressEvent::Finished => {
                on_multi_upgrade_finished(app, agg, &channels.update_tx);
            }
        }
    }
}

/// Records the currently-running package and renders the in-progress toast.
fn on_multi_upgrade_step_start(app: &mut App, package_name: &str) {
    if let Some(progress) = app.multi_upgrade.as_mut() {
        progress.current_package = Some(package_name.to_string());
        progress.current_started_at = Some(Instant::now());
    }
    app.message = Some(format!("Upgrading {package_name}..."));
}

/// Increments the counters and captures success/failure for the completed step.
fn on_multi_upgrade_step_done(
    app: &mut App,
    agg: &mut MultiUpgradeAgg,
    pm_index: usize,
    package_name: String,
    used_full_system_update: bool,
    result: crate::model::AppResult<String>,
) {
    if let Some(progress) = app.multi_upgrade.as_mut() {
        progress.done = progress.done.saturating_add(1);
        progress.current_started_at = None;
    }
    match result {
        Ok(_) => {
            agg.ok = agg.ok.saturating_add(1);
            if used_full_system_update {
                agg.full_system_successes.insert(pm_index);
            } else {
                agg.successes.push((pm_index, package_name));
            }
        }
        Err(e) => {
            agg.errors.push(format!("{package_name}: {e}"));
        }
    }
}

/// Finalizes a bulk upgrade: summary toast, refresh, and prune already-upgraded overlay rows.
fn on_multi_upgrade_finished(
    app: &mut App,
    agg: &mut MultiUpgradeAgg,
    update_tx: &UpdateCountSender,
) {
    app.multi_upgrade = None;
    app.message = Some(multi_upgrade_summary(agg));
    app.load_packages_sync();
    for slot in &mut app.pm_pending_updates {
        *slot = None;
    }
    spawn_update_refresh(&app.package_managers, update_tx);
    prune_overlay_after_bulk_upgrade(app, &agg.successes, &agg.full_system_successes);
    agg.ok = 0;
    agg.errors.clear();
    agg.successes.clear();
    agg.full_system_successes.clear();
}

/// Human-readable summary for the "upgrade finished" toast.
fn multi_upgrade_summary(agg: &MultiUpgradeAgg) -> String {
    if !agg.errors.is_empty() && agg.ok == 0 {
        format!("Upgrade failed: {}", agg.errors.join("; "))
    } else if agg.errors.is_empty() {
        format!("Upgraded {} package(s)", agg.ok)
    } else {
        format!(
            "Upgraded {} package(s); {} error(s): {}",
            agg.ok,
            agg.errors.len(),
            agg.errors.join("; ")
        )
    }
}

/// Removes already-upgraded rows from the overlay, keeping the cursor in range.
fn prune_overlay_after_bulk_upgrade(
    app: &mut App,
    successes: &[(usize, String)],
    full_system_successes: &BTreeSet<usize>,
) {
    let Some(overlay) = app.all_upgradables.as_mut() else {
        return;
    };
    overlay.rows.retain(|row| {
        if full_system_successes.contains(&row.pm_index) {
            return false;
        }
        !successes
            .iter()
            .any(|(pm_idx, name)| row.pm_index == *pm_idx && row.name == *name)
    });
    overlay.selected.clear();
    if overlay.rows.is_empty() {
        overlay.cursor = 0;
    } else {
        overlay.cursor = overlay.cursor.min(overlay.rows.len() - 1);
    }
    refresh_overlay_opened_metadata(overlay);
}

/// Polls for a keyboard event (100ms timeout) and dispatches it. Returns `true` to quit.
fn poll_and_handle_input(app: &mut App, channels: &RunChannels) -> bool {
    if !crossterm::event::poll(std::time::Duration::from_millis(100)).unwrap_or(false) {
        return false;
    }
    let Ok(Event::Key(KeyEvent {
        code,
        modifiers,
        kind: KeyEventKind::Press,
        ..
    })) = crossterm::event::read()
    else {
        return false;
    };

    if app.pending_mirror_retry.is_some() {
        return handle_pending_mirror_retry_key(app, code, &channels.single_upgrade_tx);
    }

    if app.all_upgradables.is_some() {
        handle_all_upgradables_key(app, code, modifiers, &channels.multi_upgrade_tx);
        return false;
    }

    if app.search_mode {
        handle_search_key(app, code, modifiers);
        return false;
    }

    handle_main_key(app, code, modifiers, channels)
}

/// Handles `y/n` decision for mirror refresh + retry prompt. Returns quit flag.
fn handle_pending_mirror_retry_key(
    app: &mut App,
    code: KeyCode,
    single_upgrade_tx: &SingleUpgradeSender,
) -> bool {
    match code {
        KeyCode::Char('y' | 'Y') => {
            let Some(pending) = app.pending_mirror_retry.take() else {
                return false;
            };
            app.single_upgrade = Some(SingleUpgradeProgress {
                package_name: pending.package_display.clone(),
                started_at: Instant::now(),
            });
            app.message = Some(format!(
                "Refreshing mirrors and retrying {}...",
                pending.package_display
            ));
            let tx = single_upgrade_tx.clone();
            std::thread::spawn(move || {
                let result = pending
                    .pm
                    .refresh_mirrors_and_upgrade_package(&pending.package_op_arg);
                let _ = tx.send((pending.package_display, result));
            });
            false
        }
        KeyCode::Esc | KeyCode::Char('n' | 'N') => {
            app.pending_mirror_retry = None;
            app.message = Some("Mirror refresh canceled.".to_string());
            false
        }
        KeyCode::Char('q') => true,
        _ => false,
    }
}

/// Handles a key press while the main search-mode banner is active.
fn handle_search_key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    match code {
        KeyCode::Esc => {
            app.search_mode = false;
            app.search_query.clear();
        }
        KeyCode::Enter => {
            app.search_mode = false;
        }
        KeyCode::Backspace => {
            app.search_query.pop();
        }
        KeyCode::Char('f' | 'F') if modifiers.contains(KeyModifiers::CONTROL) => {
            app.search_fuzzy = !app.search_fuzzy;
        }
        KeyCode::Char(c) => {
            app.search_query.push(c);
        }
        _ => {}
    }
}

/// Handles a key press on the main package view. Returns `true` when the loop should quit.
fn handle_main_key(
    app: &mut App,
    code: KeyCode,
    modifiers: KeyModifiers,
    channels: &RunChannels,
) -> bool {
    if let Some(quit) = handle_main_navigation_key(app, code, modifiers) {
        return quit;
    }
    let _ = handle_main_action_key(app, code, modifiers, channels);
    false
}

/// Navigation / quit / search / scroll keys. Returns `Some(quit_flag)` when the key was consumed.
fn handle_main_navigation_key(
    app: &mut App,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> Option<bool> {
    match code {
        KeyCode::Esc => {
            if app.search_query.is_empty() {
                return Some(true);
            }
            app.search_query.clear();
            Some(false)
        }
        KeyCode::Char('q') if !app.search_mode => Some(true),
        KeyCode::Up | KeyCode::Char('k') => {
            app.select_previous();
            Some(false)
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.select_next();
            Some(false)
        }
        KeyCode::Char('d' | 'D') if modifiers.contains(KeyModifiers::CONTROL) => {
            app.down(LIST_SCROLL_STEP);
            Some(false)
        }
        KeyCode::Char('\x04') => {
            app.down(LIST_SCROLL_STEP);
            Some(false)
        }
        KeyCode::Char('u' | 'U') if modifiers.contains(KeyModifiers::CONTROL) => {
            app.up(LIST_SCROLL_STEP);
            Some(false)
        }
        KeyCode::Char('\x15') => {
            app.up(LIST_SCROLL_STEP);
            Some(false)
        }
        KeyCode::Char('/') if !app.search_mode => {
            app.search_mode = true;
            Some(false)
        }
        _ => None,
    }
}

/// Package / PM action keys (upgrade, remove, overlay open, refresh, tab switch).
fn handle_main_action_key(
    app: &mut App,
    code: KeyCode,
    modifiers: KeyModifiers,
    channels: &RunChannels,
) -> bool {
    match code {
        KeyCode::Char('u') if !app.search_mode && !modifiers.contains(KeyModifiers::CONTROL) => {
            start_single_upgrade(app, &channels.single_upgrade_tx);
            true
        }
        KeyCode::Delete if !app.search_mode => {
            remove_selected_package(app);
            true
        }
        KeyCode::Char('o') if !app.search_mode => {
            toggle_outdated_only(app);
            true
        }
        KeyCode::Char('a') if !app.search_mode => {
            open_all_upgradables_overlay(app);
            true
        }
        KeyCode::Char('r') if !app.search_mode && !modifiers.contains(KeyModifiers::CONTROL) => {
            refresh_active_pm(app, &channels.update_tx);
            true
        }
        KeyCode::BackTab => {
            cycle_active_pm(app, false);
            true
        }
        KeyCode::Tab if modifiers.contains(KeyModifiers::SHIFT) => {
            cycle_active_pm(app, false);
            true
        }
        KeyCode::Tab => {
            cycle_active_pm(app, true);
            true
        }
        _ => false,
    }
}

/// Spawns a worker to upgrade the currently selected package, if any.
fn start_single_upgrade(app: &mut App, single_upgrade_tx: &SingleUpgradeSender) {
    if app.single_upgrade.is_some() {
        app.message = Some("Another package upgrade is already running".to_string());
        return;
    }
    let Some((_, p)) = app
        .filtered_packages()
        .get(app.selected_package_index)
        .copied()
    else {
        return;
    };
    let Some(pm) = app.active_pm() else {
        return;
    };
    let display = p.name.clone();
    let op_arg = pip_pacman_op_arg(&pm, p);
    app.single_upgrade = Some(SingleUpgradeProgress {
        package_name: display.clone(),
        started_at: Instant::now(),
    });
    app.message = Some(format!("Upgrading {display}..."));
    let tx = single_upgrade_tx.clone();
    std::thread::spawn(move || {
        let result = pm.upgrade_package(&op_arg);
        let _ = tx.send((display, result));
    });
}

/// Synchronously removes the selected package and surfaces the result as a toast.
fn remove_selected_package(app: &mut App) {
    let Some((_, p)) = app
        .filtered_packages()
        .get(app.selected_package_index)
        .copied()
    else {
        return;
    };
    let Some(pm) = app.active_pm() else {
        return;
    };
    let display = p.name.clone();
    let op_arg = pip_pacman_op_arg(&pm, p);
    match pm.remove_package(&op_arg) {
        Ok(_) => {
            app.message = Some(format!("Removed {display}"));
            app.load_packages_sync();
        }
        Err(e) => {
            app.message = Some(format!("Error: {e}"));
        }
    }
}

/// Toggles the "upgradable only" filter and clamps the cursor to the new filtered length.
fn toggle_outdated_only(app: &mut App) {
    app.show_outdated_only = !app.show_outdated_only;
    let count = app.filtered_packages().len();
    if count > 0 {
        app.selected_package_index = app.selected_package_index.min(count - 1);
    } else {
        app.selected_package_index = 0;
    }
}

/// Opens the all-upgradables overlay from the already-cached per-tab lists.
fn open_all_upgradables_overlay(app: &mut App) {
    let rows = collect_upgradables_from_cached_lists(&app.package_managers, &app.per_pm_packages);
    let opened_row_count = rows.len();
    let mut opened_backend_counts = std::collections::BTreeMap::new();
    for row in &rows {
        *opened_backend_counts.entry(row.pm_index).or_insert(0usize) += 1;
    }
    app.all_upgradables = Some(AllUpgradablesOverlay {
        loading: false,
        rows,
        opened_row_count,
        opened_backend_counts,
        cursor: 0,
        selected: BTreeSet::new(),
        search_query: String::new(),
        search_mode: false,
        search_fuzzy: false,
    });
}

/// Re-runs a blocking list load on the active PM and kicks off an update-count refresh.
fn refresh_active_pm(app: &mut App, update_tx: &UpdateCountSender) {
    app.load_packages_sync();
    for slot in &mut app.pm_pending_updates {
        *slot = None;
    }
    spawn_update_refresh(&app.package_managers, update_tx);
}
