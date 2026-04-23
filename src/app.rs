//! Central `App` state (backends, per-tab lists, selection, overlays) and its lifecycle methods.

use std::collections::{BTreeSet, HashMap, VecDeque};

use crate::detect::{detect_distro, detect_package_managers};
use crate::model::{
    AllUpgradablesOverlay, AppResult, FilterMode, MultiUpgradeProgress, Package, PackageListSender,
    PackageStatus, PendingUpgradeMerge, PreloadSender, SingleUpgradeProgress, SortField,
    UpgradeMapSender,
};
use crate::package_cache;
use crate::pkg_manager::PackageManager;
use crate::workers::merge_installed_list_for_pm;

/// Mutable TUI application state: backends, list contents, selection, and search.
#[allow(clippy::struct_excessive_bools)]
pub struct App {
    /// Backends detected on this machine at startup.
    pub package_managers: Vec<PackageManager>,
    /// Index into [`Self::package_managers`] for the active tab.
    pub active_pm_index: usize,
    /// Cached package list per backend (`None` until first successful load for that tab).
    pub per_pm_packages: Vec<Option<Vec<Package>>>,
    /// Index into [`Self::filtered_packages`] for keyboard selection.
    pub selected_package_index: usize,
    /// Substring filter while search mode is active.
    pub search_query: String,
    /// When true, typed keys append to `search_query` instead of triggering actions.
    pub search_mode: bool,
    /// Status filter for the table.
    pub filter_mode: FilterMode,
    /// Active sort column.
    pub sort_field: SortField,
    /// When true, sort ascending; when false, descending.
    pub sort_ascending: bool,
    /// Spinner / blocking state while a package list load runs.
    pub loading: bool,
    /// Transient toast after install/remove/upgrade (not yet wired to all UI paths).
    pub message: Option<String>,
    /// Restrict the table to rows with a known upgrade target.
    pub show_outdated_only: bool,
    /// Human-readable OS name for the header.
    pub distro: String,
    /// Last known terminal dimensions `(cols, rows)`.
    pub terminal_size: (u16, u16),
    /// Per-backend pending update counts from background threads (`None` while unknown).
    pub pm_pending_updates: Vec<Option<usize>>,
    /// Full-system upgradable list overlay (opened with `a`).
    pub all_upgradables: Option<AllUpgradablesOverlay>,
    /// In-flight progress for a multi-package overlay upgrade (`a` then `u`).
    pub multi_upgrade: Option<MultiUpgradeProgress>,
    /// In-flight single-package upgrade requested via `u`.
    pub single_upgrade: Option<SingleUpgradeProgress>,
    /// Id of an in-flight background installed-package list (`None` if cancelled).
    pub pending_list_load_req: Option<u64>,
    /// Monotonic id for background list requests (used to drop stale thread results).
    pub list_load_counter: u64,
    /// Sender for `(pm_index, request_id, upgrade map)`; set when the TUI event loop starts.
    pub upgrade_map_tx: Option<UpgradeMapSender>,
    /// Per-backend expected request id for an in-flight upgrade-metadata fetch (`None` if none).
    pub pending_upgrade_fetch_rid: Vec<Option<u64>>,
    /// Per-backend monotonic id for upgrade-metadata fetches (bumped per request).
    pub upgrade_fetch_gen: Vec<u64>,
    /// Staged upgrade map applied incrementally to [`Self::per_pm_packages`].
    pub pending_upgrade_merge: Option<PendingUpgradeMerge>,
    /// Maps waiting to merge after [`Self::pending_upgrade_merge`] finishes another backend.
    pub upgrade_merge_backlog: VecDeque<(usize, HashMap<String, String>)>,
    /// `PackageManager` index currently loading via [`Self::begin_background_list_load`].
    pub pending_primary_list_pm: Option<usize>,
    /// Indices waiting for a background installs-only preload (center-out from active tab).
    pub preload_queue: VecDeque<usize>,
    /// Count of preload worker threads not yet reported back.
    pub preload_in_flight: usize,
    /// Indices currently being preloaded (excluded from queue rebuild duplicates).
    pub preload_inflight_indices: BTreeSet<usize>,
    /// Bumped on tab change to ignore stale preload completions.
    pub preload_op_epoch: u64,
    /// Sender for preload worker results; set in `run_loop::run`.
    pub preload_result_tx: Option<PreloadSender>,
    /// Manager names that already showed the one-time sudo hint this session.
    pub shown_privilege_hint_for: BTreeSet<String>,
    /// User chose to run `sudo -v` before the TUI and it succeeded.
    pub sudo_session_enabled: bool,
}

impl App {
    /// Detects distro and available package managers, then builds empty package lists.
    ///
    /// # Errors
    ///
    /// Returns [`crate::AppError::Io`] if reading OS metadata fails in an unexpected way
    /// (currently unused).
    pub fn new() -> AppResult<Self> {
        let package_managers = detect_package_managers();
        let distro = detect_distro();
        let pm_count = package_managers.len();
        let pm_pending_updates = vec![None; pm_count];
        let per_pm_packages = package_cache::load_disk_cache(&package_managers)
            .unwrap_or_else(|| vec![None; pm_count]);

        Ok(Self {
            package_managers,
            active_pm_index: 0,
            per_pm_packages,
            selected_package_index: 0,
            search_query: String::new(),
            search_mode: false,
            filter_mode: FilterMode::All,
            sort_field: SortField::Name,
            sort_ascending: true,
            loading: false,
            message: None,
            show_outdated_only: false,
            distro,
            terminal_size: (80, 24),
            pm_pending_updates,
            all_upgradables: None,
            multi_upgrade: None,
            single_upgrade: None,
            pending_list_load_req: None,
            list_load_counter: 0,
            upgrade_map_tx: None,
            pending_upgrade_fetch_rid: vec![None; pm_count],
            upgrade_fetch_gen: vec![0; pm_count],
            pending_upgrade_merge: None,
            upgrade_merge_backlog: VecDeque::new(),
            pending_primary_list_pm: None,
            preload_queue: VecDeque::new(),
            preload_in_flight: 0,
            preload_inflight_indices: BTreeSet::new(),
            preload_op_epoch: 0,
            preload_result_tx: None,
            shown_privilege_hint_for: BTreeSet::new(),
            sudo_session_enabled: false,
        })
    }

    /// Cancels any in-flight background list load so its result is ignored.
    pub const fn cancel_pending_list_load(&mut self) {
        self.pending_list_load_req = None;
        self.pending_primary_list_pm = None;
    }

    /// Drops staged merge work and invalidates in-flight upgrade-metadata fetches.
    pub fn bump_upgrade_epoch(&mut self) {
        self.pending_upgrade_merge = None;
        self.upgrade_merge_backlog.clear();
        for s in &mut self.pending_upgrade_fetch_rid {
            *s = None;
        }
        for g in &mut self.upgrade_fetch_gen {
            *g = g.wrapping_add(1);
        }
    }

    /// Spawns [`PackageManager::list_installed_packages`] for `pm_index` and tracks completion.
    pub fn begin_background_list_load(
        &mut self,
        pm_index: usize,
        pm: PackageManager,
        tx: &PackageListSender,
    ) {
        self.cancel_pending_list_load();
        self.bump_upgrade_epoch();
        self.list_load_counter = self.list_load_counter.wrapping_add(1);
        let rid = self.list_load_counter;
        self.pending_list_load_req = Some(rid);
        self.pending_primary_list_pm = Some(pm_index);
        self.loading = true;
        let tx = tx.clone();
        std::thread::spawn(move || {
            let res = pm.list_installed_packages();
            let _ = tx.send((pm_index, rid, res));
        });
    }

    /// Starts a background [`PackageManager::fetch_upgrade_versions_map`] for `pm_index`.
    pub fn schedule_upgrade_metadata_fetch(&mut self, pm_index: usize) {
        let Some(tx) = self.upgrade_map_tx.as_ref() else {
            return;
        };
        if !self
            .package_managers
            .get(pm_index)
            .is_some_and(|p| p.available)
        {
            return;
        }
        if self
            .pending_upgrade_merge
            .as_ref()
            .is_some_and(|m| m.pm_index == pm_index)
        {
            self.pending_upgrade_merge = None;
        }
        self.upgrade_merge_backlog.retain(|(p, _)| *p != pm_index);
        let Some(fetch_gen) = self.upgrade_fetch_gen.get_mut(pm_index) else {
            return;
        };
        *fetch_gen = fetch_gen.wrapping_add(1);
        let rid = *fetch_gen;
        if let Some(slot) = self.pending_upgrade_fetch_rid.get_mut(pm_index) {
            *slot = Some(rid);
        }
        let pm = self.package_managers[pm_index].clone();
        let tx = tx.clone();
        std::thread::spawn(move || {
            let res = pm.fetch_upgrade_versions_map();
            let _ = tx.send((pm_index, rid, res));
        });
    }

    /// Saves the per-backend package list cache to disk, ignoring I/O errors.
    pub fn persist_package_disk_cache_best_effort(&self) {
        let _ = package_cache::save_disk_cache(&self.package_managers, &self.per_pm_packages);
    }

    /// Installed (and listable) packages for the active backend, if loaded.
    #[must_use]
    pub fn active_packages(&self) -> &[Package] {
        self.per_pm_packages
            .get(self.active_pm_index)
            .and_then(|slot| slot.as_deref())
            .unwrap_or(&[])
    }

    /// Loads packages for the active manager on the calling thread (blocking I/O).
    pub fn load_packages_sync(&mut self) {
        if self.active_pm_index >= self.package_managers.len() {
            return;
        }

        self.cancel_pending_list_load();
        self.bump_upgrade_epoch();

        let pm = &self.package_managers[self.active_pm_index];

        if !pm.available {
            self.message = Some(format!("{name} is not available", name = pm.name));
            return;
        }

        self.loading = true;
        if let Some(slot) = self.per_pm_packages.get_mut(self.active_pm_index) {
            *slot = None;
        }

        match pm.list_installed_packages() {
            Ok(pkgs) => {
                if let Some(slot) = self.per_pm_packages.get_mut(self.active_pm_index) {
                    *slot = Some(pkgs);
                }
                let idx = self.active_pm_index;
                self.schedule_upgrade_metadata_fetch(idx);
                self.persist_package_disk_cache_best_effort();
            }
            Err(e) => {
                self.message = Some(format!("Error loading packages: {e}"));
            }
        }

        self.loading = false;
    }

    /// Reloads packages on a blocking thread pool worker (for async contexts).
    pub async fn load_packages(&mut self) {
        let pm = self.package_managers[self.active_pm_index].clone();
        let idx = self.active_pm_index;

        let pkgs = tokio::task::spawn_blocking(move || pm.list_installed_packages())
            .await
            .unwrap_or(Ok(Vec::new()));

        if let Ok(pkgs) = pkgs {
            let updated = merge_installed_list_for_pm(self, idx, pkgs);
            if updated {
                self.schedule_upgrade_metadata_fetch(idx);
                self.persist_package_disk_cache_best_effort();
            }
        }
    }

    /// Returns a clone of the currently selected [`PackageManager`], if any.
    #[must_use]
    pub fn active_pm(&self) -> Option<PackageManager> {
        self.package_managers.get(self.active_pm_index).cloned()
    }

    /// Applies search, filter, and sort settings; returns `(source_index, package)` pairs.
    #[must_use]
    pub fn filtered_packages(&self) -> Vec<(usize, &Package)> {
        let mut filtered: Vec<_> = self
            .active_packages()
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                let matches_search = if self.search_query.is_empty() {
                    true
                } else {
                    let query = self.search_query.to_lowercase();
                    p.name.to_lowercase().contains(&query)
                        || p.description.to_lowercase().contains(&query)
                };

                let matches_filter = match self.filter_mode {
                    FilterMode::All => true,
                    FilterMode::Installed => p.status == PackageStatus::Installed,
                    FilterMode::Available => p.status == PackageStatus::Available,
                    FilterMode::Outdated => p.status == PackageStatus::Outdated,
                };

                let matches_upgradable_only = if self.show_outdated_only {
                    p.latest_version.is_some()
                } else {
                    true
                };

                matches_search && matches_filter && matches_upgradable_only
            })
            .collect();

        filtered.sort_by(|a, b| {
            let cmp = match self.sort_field {
                SortField::Name => a.1.name.cmp(&b.1.name),
                SortField::Version => a.1.version.cmp(&b.1.version),
                SortField::Size => a.1.size.cmp(&b.1.size),
                SortField::Status => {
                    let a_status = match a.1.status {
                        PackageStatus::Installed => 0,
                        PackageStatus::Available => 1,
                        PackageStatus::Outdated => 2,
                        PackageStatus::Local => 3,
                    };
                    let b_status = match b.1.status {
                        PackageStatus::Installed => 0,
                        PackageStatus::Available => 1,
                        PackageStatus::Outdated => 2,
                        PackageStatus::Local => 3,
                    };
                    a_status.cmp(&b_status)
                }
            };
            if self.sort_ascending {
                cmp
            } else {
                cmp.reverse()
            }
        });

        filtered
    }

    /// Moves selection to the next filtered row (wraps).
    pub fn select_next(&mut self) {
        let count = self.filtered_packages().len();
        if count > 0 {
            self.selected_package_index = (self.selected_package_index + 1) % count;
        }
    }

    /// Moves selection to the previous filtered row (wraps).
    pub fn select_previous(&mut self) {
        let count = self.filtered_packages().len();
        if count > 0 {
            self.selected_package_index = if self.selected_package_index == 0 {
                count - 1
            } else {
                self.selected_package_index - 1
            };
        }
    }

    /// Resets the selection cursor to the first filtered row.
    pub const fn select_first(&mut self) {
        self.selected_package_index = 0;
    }

    /// Moves selection up by `amt` rows within the filtered list.
    pub fn up(&mut self, amt: usize) {
        let count = self.filtered_packages().len();
        if count > 0 {
            self.selected_package_index = self.selected_package_index.saturating_sub(amt);
        }
    }

    /// Moves selection down by `amt` rows within the filtered list.
    pub fn down(&mut self, amt: usize) {
        let count = self.filtered_packages().len();
        if count > 0 {
            self.selected_package_index = (self.selected_package_index + amt).min(count - 1);
        }
    }
}
