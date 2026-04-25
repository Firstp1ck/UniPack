//! Background workers, privilege toasts, PM-tab switching, and channel drain helpers.
//!
//! This module centralizes the coordination logic invoked from the TUI main loop: spawning
//! update-count / preload threads, draining their results, merging upgrade metadata in chunks,
//! clamping selections after list changes, and rendering the per-tab privilege reminder toast.

use std::collections::{BTreeSet, HashMap, VecDeque};

use crate::app::App;
use crate::model::{
    AppResult, MAX_PARALLEL_PRELOADS, PACKAGE_UPGRADE_MERGE_CHUNK, Package, PackageListReceiver,
    PendingUpgradeMerge, PreloadReceiver, UpdateCountSender, UpgradeMapReceiver,
};
use crate::pkg_manager::{
    PackageManager, merge_packages_with_latest_map, pip_uses_arch_pacman_for_global,
};

/// Clamps the active-tab selection index into `0..filtered_packages().len()`.
pub fn clamp_pm_selection(app: &mut App) {
    let count = app.filtered_packages().len();
    if count > 0 {
        app.selected_package_index = app.selected_package_index.min(count - 1);
    } else {
        app.selected_package_index = 0;
    }
}

/// Runs the privilege-hint toast and triggers a blocking list-load when the tab has no data yet.
pub fn handle_pm_switch(app: &mut App) {
    maybe_show_privilege_hint(app);
    if matches!(app.per_pm_packages.get(app.active_pm_index), Some(Some(_))) {
        clamp_pm_selection(app);
        return;
    }
    app.load_packages_sync();
    clamp_pm_selection(app);
}

/// True when `message` is the per-PM privilege line ("run sudo -v" or "sudo is enabled").
#[must_use]
pub fn is_privilege_hint_toast(msg: &str) -> bool {
    privilege_hint_needs_sudo_reminder(msg) || privilege_hint_sudo_enabled_line(msg)
}

/// True when `message` is the "run sudo -v first" toast.
#[must_use]
pub fn privilege_hint_needs_sudo_reminder(msg: &str) -> bool {
    msg.contains("actions may require sudo") && msg.contains("sudo -v")
}

/// True when `message` is the "sudo is enabled" confirmation toast.
#[must_use]
fn privilege_hint_sudo_enabled_line(msg: &str) -> bool {
    msg.contains("sudo is enabled, packages can be updated")
}

/// Shows the per-tab sudo hint toast on first visit or when replacing our own previous toast.
pub fn maybe_show_privilege_hint(app: &mut App) {
    let Some(pm) = app.package_managers.get(app.active_pm_index) else {
        return;
    };
    let needs_sudo_hint = matches!(pm.name.as_str(), "apt" | "pacman" | "aur" | "rpm" | "snap")
        || (pm.name == "pip" && pip_uses_arch_pacman_for_global());
    if !needs_sudo_hint {
        if app.message.as_deref().is_some_and(is_privilege_hint_toast) {
            app.message = None;
        }
        return;
    }
    // `insert` is false after the first visit to this PM in the session, but we still want the
    // hint when returning from a non-sudo tab (message was cleared) or when replacing our own toast.
    let first_visit_this_pm = app.shown_privilege_hint_for.insert(pm.name.clone());
    let message_ok_to_replace = app.message.as_deref().is_none_or(is_privilege_hint_toast);
    if first_visit_this_pm || message_ok_to_replace {
        app.message = Some(if app.sudo_session_enabled {
            format!("{}: sudo is enabled, packages can be updated.", pm.name)
        } else {
            format!(
                "{} actions may require sudo. Run `sudo -v` in terminal first.",
                pm.name
            )
        });
    }
}

/// Moves to the next/previous PM tab and refreshes preload queue + toast for the new tab.
pub fn cycle_active_pm(app: &mut App, forward: bool) {
    let pm_count = app.package_managers.len();
    if pm_count == 0 {
        return;
    }
    if forward {
        app.active_pm_index = (app.active_pm_index + 1) % pm_count;
    } else {
        app.active_pm_index = (app.active_pm_index + pm_count - 1) % pm_count;
    }
    handle_pm_switch(app);
    refresh_preload_queue(app, true);
}

/// Spawns one thread per package manager to refresh its pending-updates count.
pub fn spawn_update_refresh(managers: &[PackageManager], tx: &UpdateCountSender) {
    for (idx, pm) in managers.iter().cloned().enumerate() {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let count = pm.count_pending_updates().ok();
            let _ = tx.send((idx, count));
        });
    }
}

/// Returns `true` when the backend at `i` is eligible for a background preload right now.
fn slot_needs_preload(app: &App, i: usize) -> bool {
    if !app.package_managers.get(i).is_some_and(|p| p.available) {
        return false;
    }
    if app.pending_primary_list_pm == Some(i) {
        return false;
    }
    if app.preload_inflight_indices.contains(&i) {
        return false;
    }
    if app.loading && i == app.active_pm_index {
        return false;
    }
    app.per_pm_packages
        .get(i)
        .and_then(|x| x.as_ref())
        .is_none()
}

/// Builds a `VecDeque` of PM indices to preload, center-out from the active tab.
fn build_preload_queue_indices(app: &App) -> VecDeque<usize> {
    let len = app.package_managers.len();
    let mut q = VecDeque::new();
    if len == 0 {
        return q;
    }
    let active = app.active_pm_index;
    let mut seen = BTreeSet::new();
    if slot_needs_preload(app, active) && seen.insert(active) {
        q.push_back(active);
    }
    for step in 1..len {
        let r = (active + step) % len;
        let l = (active + len - step) % len;
        if r == l {
            if slot_needs_preload(app, r) && seen.insert(r) {
                q.push_back(r);
            }
        } else {
            if slot_needs_preload(app, r) && seen.insert(r) {
                q.push_back(r);
            }
            if slot_needs_preload(app, l) && seen.insert(l) {
                q.push_back(l);
            }
        }
    }
    q
}

/// Rebuilds the preload queue from the active tab. When `bump_epoch`, in-flight results are ignored.
pub fn refresh_preload_queue(app: &mut App, bump_epoch: bool) {
    if bump_epoch {
        app.preload_op_epoch = app.preload_op_epoch.wrapping_add(1);
    }
    app.preload_queue = build_preload_queue_indices(app);
}

/// Spawns up to `MAX_PARALLEL_PRELOADS` concurrent preload workers from the queue.
pub fn pump_preloads(app: &mut App) {
    let Some(tx) = app.preload_result_tx.as_ref() else {
        return;
    };
    'more: while app.preload_in_flight < MAX_PARALLEL_PRELOADS {
        while let Some(&idx) = app.preload_queue.front() {
            if slot_needs_preload(app, idx) {
                app.preload_queue.pop_front();
                let epoch = app.preload_op_epoch;
                let pm = app.package_managers[idx].clone();
                app.preload_inflight_indices.insert(idx);
                app.preload_in_flight = app.preload_in_flight.saturating_add(1);
                let tx = tx.clone();
                std::thread::spawn(move || {
                    let res = pm.list_installed_packages();
                    let _ = tx.send((epoch, idx, res));
                });
                continue 'more;
            }
            app.preload_queue.pop_front();
        }
        break;
    }
}

/// Drains preload worker results and applies them to per-tab caches (respecting the epoch).
pub fn try_recv_preload_results(app: &mut App, rx: &PreloadReceiver) {
    while let Ok((epoch, idx, res)) = rx.try_recv() {
        app.preload_in_flight = app.preload_in_flight.saturating_sub(1);
        app.preload_inflight_indices.remove(&idx);
        if epoch != app.preload_op_epoch {
            continue;
        }
        if let Ok(pkgs) = res
            && let Some(slot) = app.per_pm_packages.get_mut(idx)
            && slot.is_none()
        {
            *slot = Some(pkgs);
            app.schedule_upgrade_metadata_fetch(idx);
            app.persist_package_disk_cache_best_effort();
        }
    }
}

/// True when two package lists contain the same `(name, version)` multi-set.
pub fn installed_lists_equivalent(existing: &[Package], fresh: &[Package]) -> bool {
    fn name_version_pairs(pkgs: &[Package]) -> Vec<(String, String)> {
        let mut v: Vec<_> = pkgs
            .iter()
            .map(|p| (p.name.clone(), p.version.clone()))
            .collect();
        v.sort();
        v
    }
    name_version_pairs(existing) == name_version_pairs(fresh)
}

/// When the fresh install-only list matches what we already show, keep cached rows (including upgrade fields).
///
/// **Returns** `true` when data was replaced and upgrade metadata should be re-fetched.
pub fn merge_installed_list_for_pm(app: &mut App, pm_index: usize, fresh: Vec<Package>) -> bool {
    let Some(slot) = app.per_pm_packages.get_mut(pm_index) else {
        return false;
    };
    match slot.as_ref() {
        Some(existing) if installed_lists_equivalent(existing, &fresh) => false,
        _ => {
            *slot = Some(fresh);
            true
        }
    }
}

/// Counts rows marked as outdated in one cached package list.
fn cached_outdated_count(pkgs: &[Package]) -> usize {
    pkgs.iter()
        .filter(|p| p.status == crate::model::PackageStatus::Outdated)
        .count()
}

/// Updates one PM tab badge from the currently cached list (if present).
fn sync_pending_count_from_cached_list(app: &mut App, pm_index: usize) {
    let count = app
        .per_pm_packages
        .get(pm_index)
        .and_then(|slot| slot.as_ref())
        .map(|pkgs| cached_outdated_count(pkgs));
    if let Some(slot) = app.pm_pending_updates.get_mut(pm_index) {
        *slot = count;
    }
}

/// Drains background list-load results into per-tab caches.
pub fn try_recv_package_list_results(app: &mut App, pkg_rx: &PackageListReceiver) {
    while let Ok((idx, rid, res)) = pkg_rx.try_recv() {
        if app.pending_list_load_req != Some(rid) {
            continue;
        }
        app.pending_list_load_req = None;
        app.pending_primary_list_pm = None;
        app.loading = false;
        apply_package_list_result(app, idx, res);
    }
}

/// Applies one package-list result into the per-tab cache, emitting a toast on error.
fn apply_package_list_result(app: &mut App, idx: usize, res: AppResult<Vec<Package>>) {
    match res {
        Ok(pkgs) => {
            let updated = merge_installed_list_for_pm(app, idx, pkgs);
            if updated {
                if let Some(slot) = app.pm_pending_updates.get_mut(idx) {
                    *slot = None;
                }
                app.schedule_upgrade_metadata_fetch(idx);
                app.persist_package_disk_cache_best_effort();
            } else {
                sync_pending_count_from_cached_list(app, idx);
            }
            if idx == app.active_pm_index {
                clamp_pm_selection(app);
            }
            refresh_preload_queue(app, false);
        }
        Err(e) => {
            app.message = Some(format!("Error loading packages: {e}"));
            refresh_preload_queue(app, false);
        }
    }
}

/// Stores an incoming upgrade-map as the active merge slot, or defers it in the backlog.
fn enqueue_pending_upgrade_merge(app: &mut App, pm_index: usize, map: HashMap<String, String>) {
    if app.pending_upgrade_merge.is_none() {
        app.pending_upgrade_merge = Some(PendingUpgradeMerge {
            pm_index,
            map,
            next_pkg_index: 0,
        });
    } else {
        app.upgrade_merge_backlog.push_back((pm_index, map));
    }
}

/// Drains upgrade-metadata worker results and enqueues them for progressive merging.
pub fn try_recv_upgrade_metadata(app: &mut App, upgrade_rx: &UpgradeMapReceiver) {
    while let Ok((pm_index, rid, res)) = upgrade_rx.try_recv() {
        let expected = app
            .pending_upgrade_fetch_rid
            .get(pm_index)
            .copied()
            .flatten();
        if expected != Some(rid) {
            continue;
        }
        if let Some(slot) = app.pending_upgrade_fetch_rid.get_mut(pm_index) {
            *slot = None;
        }
        match res {
            Ok(map) if !map.is_empty() => {
                enqueue_pending_upgrade_merge(app, pm_index, map);
            }
            Ok(_) | Err(_) => {}
        }
    }
}

/// Applies the next chunk of `PACKAGE_UPGRADE_MERGE_CHUNK` upgrade rows and advances the slot.
pub fn advance_upgrade_merge_chunk(app: &mut App) {
    let Some(mut slot) = app.pending_upgrade_merge.take() else {
        return;
    };
    let pm_index = slot.pm_index;
    let Some(pm) = app.package_managers.get(pm_index) else {
        return;
    };
    let Some(pkgs_vec) = app.per_pm_packages.get_mut(pm_index) else {
        return;
    };
    let Some(pkgs) = pkgs_vec.as_mut() else {
        app.pending_upgrade_merge = Some(slot);
        return;
    };
    let end = slot
        .next_pkg_index
        .saturating_add(PACKAGE_UPGRADE_MERGE_CHUNK)
        .min(pkgs.len());
    let chunk = &mut pkgs[slot.next_pkg_index..end];
    merge_packages_with_latest_map(pm, chunk, &slot.map);
    slot.next_pkg_index = end;
    if slot.next_pkg_index < pkgs.len() {
        app.pending_upgrade_merge = Some(slot);
    } else {
        sync_pending_count_from_cached_list(app, pm_index);
        app.persist_package_disk_cache_best_effort();
        if let Some((pm, map)) = app.upgrade_merge_backlog.pop_front() {
            app.pending_upgrade_merge = Some(PendingUpgradeMerge {
                pm_index: pm,
                map,
                next_pkg_index: 0,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PackageStatus;

    fn pkg(name: &str, version: &str, latest: Option<&str>) -> Package {
        Package {
            name: name.to_string(),
            version: version.to_string(),
            latest_version: latest.map(String::from),
            status: PackageStatus::Installed,
            size: 0,
            description: String::new(),
            repository: None,
            installed_by: None,
        }
    }

    #[test]
    fn installed_equivalent_matches_sorted_name_version() {
        let a = vec![pkg("b", "2", None), pkg("a", "1", Some("9"))];
        let b = vec![pkg("a", "1", None), pkg("b", "2", Some("3"))];
        assert!(installed_lists_equivalent(&a, &b));
    }

    #[test]
    fn installed_equivalent_rejects_version_change() {
        let a = vec![pkg("x", "1", None)];
        let b = vec![pkg("x", "2", None)];
        assert!(!installed_lists_equivalent(&a, &b));
    }
}
