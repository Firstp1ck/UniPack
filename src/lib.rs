//! `UniPack`: cross-backend package listing and a [`ratatui`] event loop.
//!
//! The binary entry point is thin; this library holds application state and rendering.

#![allow(clippy::missing_docs_in_private_items)]

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::path::Path;
use std::process::Command;
use std::time::Instant;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Cell, Gauge, Paragraph, Row, Table, Tabs};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use unicode_width::UnicodeWidthStr;

mod all_upgradables;
mod package_cache;
mod pkg_manager;

pub use all_upgradables::{
    UpgradableRow, collect_all_upgradables, collect_upgradables_from_cached_lists,
};
use pkg_manager::{PackageManager, merge_packages_with_latest_map};

/// Recoverable failures surfaced to the user or propagated from subprocess I/O.
#[derive(Error, Debug)]
pub enum AppError {
    /// Underlying filesystem or pipe error.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    /// User-facing message from a package-manager invocation.
    #[error("Package manager error: {0}")]
    PkgMgr(String),
    /// JSON encode/decode for the on-disk package cache.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

impl From<String> for AppError {
    fn from(s: String) -> Self {
        Self::PkgMgr(s)
    }
}

impl From<&str> for AppError {
    fn from(s: &str) -> Self {
        Self::PkgMgr(s.to_string())
    }
}

/// Convenient [`Result`] alias using [`AppError`].
pub type AppResult<T> = Result<T, AppError>;

type UpgradeMapChannelMsg = (usize, u64, AppResult<HashMap<String, String>>);
type UpgradeMapSender = std::sync::mpsc::Sender<UpgradeMapChannelMsg>;
type UpgradeMapReceiver = std::sync::mpsc::Receiver<UpgradeMapChannelMsg>;

type PreloadChannelMsg = (u64, usize, AppResult<Vec<Package>>);
type PreloadSender = std::sync::mpsc::Sender<PreloadChannelMsg>;
type PreloadReceiver = std::sync::mpsc::Receiver<PreloadChannelMsg>;
type SingleUpgradeChannelMsg = (String, AppResult<String>);
type SingleUpgradeSender = std::sync::mpsc::Sender<SingleUpgradeChannelMsg>;
type SingleUpgradeReceiver = std::sync::mpsc::Receiver<SingleUpgradeChannelMsg>;
type MultiUpgradeSender = std::sync::mpsc::Sender<MultiUpgradeProgressEvent>;
type MultiUpgradeReceiver = std::sync::mpsc::Receiver<MultiUpgradeProgressEvent>;

/// Row filter for the package table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FilterMode {
    /// No status filter.
    #[default]
    All,
    /// Only packages reported as installed.
    Installed,
    /// Only packages not installed (when the backend exposes that).
    Available,
    /// Only packages marked outdated after update metadata is applied.
    Outdated,
}

/// Column used when sorting the filtered list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortField {
    /// Sort by package name.
    #[default]
    Name,
    /// Sort by version string.
    Version,
    /// Sort by reported size (often zero when unknown).
    Size,
    /// Sort by [`PackageStatus`] rank.
    Status,
}

/// One row in the package table for the active backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Package {
    /// Package or application name.
    pub name: String,
    /// Installed or listed version string.
    pub version: String,
    /// When set, an update is available (`version` is current, this is target).
    pub latest_version: Option<String>,
    /// Installation/update state for display and filtering.
    pub status: PackageStatus,
    /// Size in bytes when the backend provides it (often `0`).
    pub size: u64,
    /// Short description when available.
    pub description: String,
    /// Repository or source label (e.g. `homebrew`, `aur`).
    pub repository: Option<String>,
    /// Optional hint for which tool installed the package.
    pub installed_by: Option<String>,
}

/// Coarse lifecycle state shown in the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PackageStatus {
    /// Currently installed.
    #[default]
    Installed,
    /// Available from a remote index but not installed.
    Available,
    /// Installed but a newer version is reported.
    Outdated,
    /// Local or non-repo package (backend-specific).
    Local,
}

impl std::fmt::Display for PackageStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Installed => write!(f, "installed"),
            Self::Available => write!(f, "available"),
            Self::Outdated => write!(f, "outdated"),
            Self::Local => write!(f, "local"),
        }
    }
}

/// Applies [`merge_packages_with_latest_map`] in slices between frames so the UI stays responsive.
struct PendingUpgradeMerge {
    pm_index: usize,
    map: HashMap<String, String>,
    next_pkg_index: usize,
}

/// Max packages to upgrade-annotate per main-loop iteration (progressive “live” merge).
const PACKAGE_UPGRADE_MERGE_CHUNK: usize = 400;

/// Max concurrent background installs-only preloads (other package manager tabs).
const MAX_PARALLEL_PRELOADS: usize = 2;

/// Lines to move the cursor on Ctrl+d / Ctrl+u (and terminal EOT/NAK where applicable).
const LIST_SCROLL_STEP: usize = 20;

/// Multiselect overlay listing upgradable packages across all detected backends.
pub struct AllUpgradablesOverlay {
    /// Background scan in progress.
    pub loading: bool,
    /// Sorted rows for display and upgrade.
    pub rows: Vec<UpgradableRow>,
    /// Cursor into [`Self::rows`].
    pub cursor: usize,
    /// Row indices selected for upgrade.
    pub selected: BTreeSet<usize>,
}

/// UI state for one package upgrade triggered via `u`.
pub struct SingleUpgradeProgress {
    /// Package currently being upgraded.
    pub package_name: String,
    /// Wall-clock start for indeterminate progress animation.
    pub started_at: Instant,
}

/// Running state for bulk upgrade execution from the all-upgradables overlay.
pub struct MultiUpgradeProgress {
    /// Number of selected rows scheduled for upgrade.
    pub total: usize,
    /// Completed attempts (success + failure).
    pub done: usize,
    /// Package currently being upgraded.
    pub current_package: Option<String>,
    /// Start instant for currently running package step.
    pub current_started_at: Option<Instant>,
}

enum MultiUpgradeProgressEvent {
    StepStart { package_name: String },
    StepDone {
        pm_index: usize,
        package_name: String,
        result: AppResult<String>,
    },
    Finished,
}

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
    pending_list_load_req: Option<u64>,
    /// Monotonic id for background list requests (used to drop stale thread results).
    list_load_counter: u64,
    /// Sender for `(pm_index, request_id, upgrade map)`; set when the TUI event loop starts.
    upgrade_map_tx: Option<UpgradeMapSender>,
    /// Per-backend expected request id for an in-flight upgrade-metadata fetch (`None` if none).
    pending_upgrade_fetch_rid: Vec<Option<u64>>,
    /// Per-backend monotonic id for upgrade-metadata fetches (each [`schedule_upgrade_metadata_fetch`] bumps one slot).
    upgrade_fetch_gen: Vec<u64>,
    /// Staged upgrade map applied incrementally to [`Self::per_pm_packages`].
    pending_upgrade_merge: Option<PendingUpgradeMerge>,
    /// Maps waiting to merge after [`Self::pending_upgrade_merge`] finishes another backend.
    upgrade_merge_backlog: VecDeque<(usize, HashMap<String, String>)>,
    /// `PackageManager` index currently loading via [`Self::begin_background_list_load`].
    pending_primary_list_pm: Option<usize>,
    /// Indices waiting for a background installs-only preload (center-out from active tab).
    preload_queue: VecDeque<usize>,
    /// Count of preload worker threads not yet reported back.
    preload_in_flight: usize,
    /// Indices currently being preloaded (excluded from queue rebuild duplicates).
    preload_inflight_indices: BTreeSet<usize>,
    /// Bumped on tab change to ignore stale preload completions.
    preload_op_epoch: u64,
    /// Sender for preload worker results; set in [`run`].
    preload_result_tx: Option<PreloadSender>,
}

impl App {
    /// Detects distro and available package managers, then builds empty package lists.
    ///
    /// # Errors
    ///
    /// Returns [`AppError::Io`] if reading OS metadata fails in an unexpected way (currently unused).
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
        })
    }

    /// Cancels any in-flight background list load so its result is ignored.
    const fn cancel_pending_list_load(&mut self) {
        self.pending_list_load_req = None;
        self.pending_primary_list_pm = None;
    }

    /// Drops staged merge work and invalidates in-flight upgrade-metadata fetches.
    fn bump_upgrade_epoch(&mut self) {
        self.pending_upgrade_merge = None;
        self.upgrade_merge_backlog.clear();
        for s in &mut self.pending_upgrade_fetch_rid {
            *s = None;
        }
        for g in &mut self.upgrade_fetch_gen {
            *g = g.wrapping_add(1);
        }
    }

    /// Spawns [`PackageManager::list_installed_packages`] for `pm_index` and tracks completion via `pending_list_load_req`.
    fn begin_background_list_load(
        &mut self,
        pm_index: usize,
        pm: PackageManager,
        tx: &std::sync::mpsc::Sender<(usize, u64, AppResult<Vec<Package>>)>,
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
    fn schedule_upgrade_metadata_fetch(&mut self, pm_index: usize) {
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
                persist_package_disk_cache_best_effort(self);
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
                persist_package_disk_cache_best_effort(self);
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

/// Best-effort OS name from `/etc/os-release` or common marker files.
#[must_use]
pub fn detect_distro() -> String {
    if cfg!(target_os = "windows") {
        return "Windows".to_string();
    }
    if cfg!(target_os = "macos") {
        return "macOS".to_string();
    }

    if Path::new("/etc/os-release").exists()
        && let Ok(content) = std::fs::read_to_string("/etc/os-release")
    {
        for line in content.lines() {
            if line.starts_with("PRETTY_NAME=") {
                return line
                    .trim_start_matches("PRETTY_NAME=")
                    .trim_matches('"')
                    .to_string();
            }
        }
    }
    if Path::new("/etc/arch-release").exists() {
        return "Arch Linux".to_string();
    }
    if Path::new("/etc/debian_version").exists() {
        return "Debian".to_string();
    }
    if Path::new("/etc/fedora-release").exists() {
        return "Fedora".to_string();
    }
    "Unknown Linux".to_string()
}

fn detect_package_managers() -> Vec<PackageManager> {
    const PM_CONFIGS: &[(&str, &str, &str, bool)] = &[
        ("pip", "pip3", "pip", false),
        ("npm", "npm", "npm", false),
        ("bun", "bun", "bun", false),
        ("cargo", "cargo", "cargo", false),
        ("brew", "brew", "brew", false),
        ("apt", "apt", "dpkg", true),
        ("pacman", "pacman", "pacman", true),
        ("aur", "yay", "yay", false),
        ("rpm", "rpm", "rpm", true),
        ("flatpak", "flatpak", "flatpak", true),
        ("snap", "snap", "snap", false),
    ];

    let sudo_ok = is_command_available("sudo");

    let results: Vec<(&str, &str, &str, bool, bool)> = std::thread::scope(|s| {
        let mut handles = Vec::with_capacity(PM_CONFIGS.len());
        for &(name, cmd, list_cmd, needs_root) in PM_CONFIGS {
            handles.push(s.spawn(move || {
                let available = is_command_available(cmd);
                (name, cmd, list_cmd, needs_root, available)
            }));
        }
        handles.into_iter().filter_map(|h| h.join().ok()).collect()
    });

    let mut managers = Vec::new();
    for (name, cmd, list_cmd, needs_root, available) in results {
        if available {
            managers.push(PackageManager {
                name: name.to_string(),
                command: cmd.to_string(),
                list_command: list_cmd.to_string(),
                available: true,
                needs_root: needs_root || sudo_ok,
            });
        }
    }

    managers
}

fn is_command_available(cmd: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("command -v {cmd}")])
        .output()
        .is_ok_and(|o| o.status.success())
}

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

fn render_app(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(4),
        ])
        .split(frame.area());

    render_header(frame, app, chunks[0]);
    if let Some(ref overlay) = app.all_upgradables {
        render_all_upgradables_body(frame, overlay, chunks[1]);
        render_all_upgradables_footer(frame, app, chunks[2]);
    } else {
        render_body(frame, app, chunks[1]);
        render_footer(frame, app, chunks[2]);
    }
}

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

fn render_footer(f: &mut Frame, app: &App, area: Rect) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Length(1)])
        .split(area);

    let hints_area = rows[0];
    let status_area = rows[1];

    if app.search_mode {
        let q = if app.search_query.is_empty() {
            "…"
        } else {
            app.search_query.as_str()
        };
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
            .split(hints_area);
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
    } else {
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
            .split(hints_area);

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
            footer_col_line([footer_key("i")], " install (search) "),
            footer_col_line([footer_key("Ctrl+R")], " refresh "),
            footer_col_line([footer_key("q"), footer_key(" Esc")], " quit "),
        ]);
        f.render_widget(Paragraph::new(col_move).alignment(Alignment::Left), cols[0]);
        f.render_widget(Paragraph::new(col_view).alignment(Alignment::Left), cols[1]);
        f.render_widget(Paragraph::new(col_pkg).alignment(Alignment::Left), cols[2]);
        f.render_widget(Paragraph::new(col_sys).alignment(Alignment::Left), cols[3]);
    }

    if let Some(progress) = app.single_upgrade.as_ref() {
        let elapsed_ms = progress.started_at.elapsed().as_millis() as u64;
        let cycle_ms = 1_800_u64;
        let half_cycle = cycle_ms / 2;
        let phase = elapsed_ms % cycle_ms;
        let ramp = if phase <= half_cycle {
            phase
        } else {
            cycle_ms - phase
        };
        let animated = 20_u16 + ((ramp * 75_u64) / half_cycle.max(1)) as u16;
        let gauge = Gauge::default()
            .gauge_style(Style::default().fg(COLORS.accent).bg(COLORS.surface))
            .label(format!("Upgrading {}...", progress.package_name))
            .percent(animated);
        f.render_widget(gauge, status_area);
    } else {
        let status = app
            .filtered_packages()
            .get(app.selected_package_index)
            .map_or_else(
                || "— none —".to_string(),
                |(_, pkg)| {
                    pkg.latest_version.as_ref().map_or_else(
                        || format!("{} {} · {}", pkg.name, pkg.version, pkg.status),
                        |latest| format!("{} {} → {} · {}", pkg.name, pkg.version, latest, pkg.status),
                    )
                },
            );

        let status_fit = clip_display_width(&status, status_area.width);
        let status_text = Paragraph::new(status_fit)
            .style(Style::default().fg(COLORS.accent))
            .alignment(Alignment::Right);

        f.render_widget(status_text, status_area);
    }
}

fn render_all_upgradables_body(f: &mut Frame, overlay: &AllUpgradablesOverlay, area: Rect) {
    if overlay.rows.is_empty() {
        let msg = Paragraph::new("No upgradable packages found")
            .style(Style::default().fg(COLORS.warning))
            .alignment(Alignment::Center);
        f.render_widget(msg, area);
        return;
    }

    let visible_rows = (area.height as usize).saturating_sub(2);
    let max_scroll = overlay.rows.len().saturating_sub(visible_rows);
    let half_visible = visible_rows / 2;
    let scroll_offset = overlay.cursor.saturating_sub(half_visible).min(max_scroll);

    let rows: Vec<_> = overlay
        .rows
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(visible_rows)
        .map(|(idx, row)| {
            let is_cursor = idx == overlay.cursor;
            let mark = if overlay.selected.contains(&idx) {
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

fn render_all_upgradables_footer(f: &mut Frame, app: &App, area: Rect) {
    let overlay = app
        .all_upgradables
        .as_ref()
        .expect("overlay footer only rendered while overlay exists");
    let n_sel = overlay.selected.len();
    let step = LIST_SCROLL_STEP;
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Length(1)])
        .split(area);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(34),
            Constraint::Percentage(33),
            Constraint::Percentage(33),
        ])
        .split(rows[0]);

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

    if let Some(progress) = app.multi_upgrade.as_ref() {
        let pct = if progress.total == 0 {
            0
        } else {
            // Smooth progress within each package step:
            // done packages count fully, current one contributes up to 95%.
            let elapsed_ms = progress
                .current_started_at
                .as_ref()
                .map_or(0_u128, |t| t.elapsed().as_millis());
            let sub_progress_per_mille = ((elapsed_ms * 1000) / 7000).min(950) as usize;
            let units_per_mille = progress.done.saturating_mul(1000) + sub_progress_per_mille;
            ((units_per_mille.saturating_mul(100)) / (progress.total.saturating_mul(1000))) as u16
        };
        let label = progress.current_package.as_ref().map_or_else(
            || format!("{}/{} complete", progress.done, progress.total),
            |pkg| format!("{}/{} · updating {}", progress.done, progress.total, pkg),
        );
        let gauge = Gauge::default()
            .gauge_style(Style::default().fg(COLORS.accent).bg(COLORS.surface))
            .label(label)
            .percent(pct);
        f.render_widget(gauge, rows[1]);
    } else {
        let count_line = Paragraph::new(format!("{n_sel} selected"))
            .style(Style::default().fg(COLORS.accent))
            .alignment(Alignment::Right);
        f.render_widget(count_line, rows[1]);
    }
}

fn upgrade_all_upgradables_selection(
    app: &mut App,
    multi_upgrade_tx: &MultiUpgradeSender,
) {
    let Some(overlay) = app.all_upgradables.as_mut() else {
        return;
    };
    if overlay.loading || overlay.selected.is_empty() || app.multi_upgrade.is_some() {
        return;
    }
    let indices: Vec<usize> = overlay.selected.iter().copied().collect();
    let mut tasks: Vec<(usize, PackageManager, String)> = Vec::with_capacity(indices.len());
    for idx in indices {
        let Some(row) = overlay.rows.get(idx) else {
            continue;
        };
        let Some(pm) = app.package_managers.get(row.pm_index) else {
            continue;
        };
        tasks.push((row.pm_index, pm.clone(), row.name.clone()));
    }
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
    let tx = multi_upgrade_tx.clone();
    std::thread::spawn(move || {
        for (pm_index, pm, name) in tasks {
            let _ = tx.send(MultiUpgradeProgressEvent::StepStart {
                package_name: name.clone(),
            });
            let result = pm.upgrade_package(&name);
            let _ = tx.send(MultiUpgradeProgressEvent::StepDone {
                pm_index,
                package_name: name,
                result,
            });
        }
        let _ = tx.send(MultiUpgradeProgressEvent::Finished);
    });
}

fn overlay_select_all_rows(app: &mut App) {
    if let Some(o) = app.all_upgradables.as_mut() {
        o.selected.clear();
        for i in 0..o.rows.len() {
            o.selected.insert(i);
        }
    }
}

fn overlay_deselect_all_rows(app: &mut App) {
    if let Some(o) = app.all_upgradables.as_mut() {
        o.selected.clear();
    }
}

fn overlay_scroll_page(app: &mut App, down: bool) {
    let Some(o) = app.all_upgradables.as_mut() else {
        return;
    };
    if o.rows.is_empty() {
        return;
    }
    let max = o.rows.len() - 1;
    o.cursor = if down {
        o.cursor.saturating_add(LIST_SCROLL_STEP).min(max)
    } else {
        o.cursor.saturating_sub(LIST_SCROLL_STEP)
    };
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

fn handle_all_upgradables_key(
    app: &mut App,
    code: KeyCode,
    modifiers: KeyModifiers,
    multi_upgrade_tx: &MultiUpgradeSender,
) {
    match code {
        KeyCode::Esc | KeyCode::Char('q') => {
            app.all_upgradables = None;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if let Some(o) = app.all_upgradables.as_mut()
                && !o.rows.is_empty()
            {
                let n = o.rows.len();
                o.cursor = (o.cursor + n - 1) % n;
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if let Some(o) = app.all_upgradables.as_mut()
                && !o.rows.is_empty()
            {
                let n = o.rows.len();
                o.cursor = (o.cursor + 1) % n;
            }
        }
        KeyCode::Char('d' | 'D') if modifiers.contains(KeyModifiers::CONTROL) => {
            overlay_scroll_page(app, true);
        }
        KeyCode::Char('\x04') => {
            overlay_scroll_page(app, true);
        }
        KeyCode::Char('u' | 'U') if modifiers.contains(KeyModifiers::CONTROL) => {
            overlay_scroll_page(app, false);
        }
        KeyCode::Char('\x15') => {
            overlay_scroll_page(app, false);
        }
        KeyCode::Char(' ') => {
            if let Some(o) = app.all_upgradables.as_mut()
                && !o.rows.is_empty()
            {
                let idx = o.cursor;
                if !o.selected.remove(&idx) {
                    o.selected.insert(idx);
                }
            }
        }
        KeyCode::Char(c) if modifiers.contains(KeyModifiers::SHIFT) && c.is_ascii_alphabetic() => {
            overlay_select_rows_for_pm_first_letter(app, c);
        }
        KeyCode::Char('a' | 'A') if !modifiers.contains(KeyModifiers::SHIFT) => {
            overlay_select_all_rows(app);
        }
        KeyCode::Char('d' | 'D')
            if !modifiers.contains(KeyModifiers::SHIFT)
                && !modifiers.contains(KeyModifiers::CONTROL) =>
        {
            overlay_deselect_all_rows(app);
        }
        KeyCode::Char('u')
            if !modifiers.contains(KeyModifiers::SHIFT)
                && !modifiers.contains(KeyModifiers::CONTROL) =>
        {
            upgrade_all_upgradables_selection(app, multi_upgrade_tx);
        }
        _ => {}
    }
}

fn clamp_pm_selection(app: &mut App) {
    let count = app.filtered_packages().len();
    if count > 0 {
        app.selected_package_index = app.selected_package_index.min(count - 1);
    } else {
        app.selected_package_index = 0;
    }
}

fn handle_pm_switch(app: &mut App) {
    if matches!(app.per_pm_packages.get(app.active_pm_index), Some(Some(_))) {
        clamp_pm_selection(app);
        return;
    }
    app.load_packages_sync();
    clamp_pm_selection(app);
}

fn cycle_active_pm(app: &mut App, forward: bool) {
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

fn spawn_update_refresh(
    managers: &[PackageManager],
    tx: &std::sync::mpsc::Sender<(usize, Option<usize>)>,
) {
    for (idx, pm) in managers.iter().cloned().enumerate() {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let count = pm.count_pending_updates().ok();
            let _ = tx.send((idx, count));
        });
    }
}

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

/// Rebuilds the preload queue from the active tab. When `bump_epoch`, in-flight preload results are ignored.
fn refresh_preload_queue(app: &mut App, bump_epoch: bool) {
    if bump_epoch {
        app.preload_op_epoch = app.preload_op_epoch.wrapping_add(1);
    }
    app.preload_queue = build_preload_queue_indices(app);
}

fn pump_preloads(app: &mut App) {
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

fn try_recv_preload_results(app: &mut App, rx: &PreloadReceiver) {
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
            persist_package_disk_cache_best_effort(app);
        }
    }
}

fn installed_lists_equivalent(existing: &[Package], fresh: &[Package]) -> bool {
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
fn merge_installed_list_for_pm(app: &mut App, pm_index: usize, fresh: Vec<Package>) -> bool {
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

fn persist_package_disk_cache_best_effort(app: &App) {
    let _ = package_cache::save_disk_cache(&app.package_managers, &app.per_pm_packages);
}

fn try_recv_package_list_results(
    app: &mut App,
    pkg_rx: &std::sync::mpsc::Receiver<(usize, u64, AppResult<Vec<Package>>)>,
) {
    while let Ok((idx, rid, res)) = pkg_rx.try_recv() {
        if app.pending_list_load_req != Some(rid) {
            continue;
        }
        app.pending_list_load_req = None;
        app.pending_primary_list_pm = None;
        app.loading = false;
        match res {
            Ok(pkgs) => {
                let updated = merge_installed_list_for_pm(app, idx, pkgs);
                if updated {
                    app.schedule_upgrade_metadata_fetch(idx);
                    persist_package_disk_cache_best_effort(app);
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
}

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

fn try_recv_upgrade_metadata(app: &mut App, upgrade_rx: &UpgradeMapReceiver) {
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

fn advance_upgrade_merge_chunk(app: &mut App) {
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
        persist_package_disk_cache_best_effort(app);
        if let Some((pm, map)) = app.upgrade_merge_backlog.pop_front() {
            app.pending_upgrade_merge = Some(PendingUpgradeMerge {
                pm_index: pm,
                map,
                next_pkg_index: 0,
            });
        }
    }
}

/// Initializes the terminal, runs the main keyboard/draw loop, then restores the screen.
///
/// # Panics
///
/// Panics if [`App::new`] fails (for example invalid internal state during startup).
#[allow(clippy::too_many_lines)]
#[allow(clippy::cognitive_complexity)]
pub fn run() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
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
        println!("  a             All upgradable packages (from cached lists; visit tabs to fill)");
        println!(
            "                In that view: Space toggle, a all, d none, Shift+PM letter toggles that PM, u upgrade, Ctrl+d/u page, Esc/q close"
        );
        println!("  u             Upgrade selected package");
        println!("  r             Remove selected package");
        println!("  i             Install package (type name in search)");
        println!("  Tab           Switch package manager (forward)");
        println!("  Shift+Tab     Switch package manager (backward)");
        println!("  Ctrl+R        Refresh package list");
        println!("  o             Toggle show only upgradable packages");
        println!("  Esc           Leave search, or quit when not searching");
        println!("  q             Quit when not in search (in search, types into query)");
        println!();
        println!("Supported Package Managers:");
        println!("  pip, npm, bun, cargo, apt, pacman, aur, rpm, flatpak, snap, brew");
        return;
    }

    let mut app = App::new().expect("Failed to create app");
    let mut terminal = ratatui::init();

    {
        let backend = terminal.backend_mut();
        let _ = execute!(backend, EnterAlternateScreen);
    }

    let (upgrade_tx, upgrade_rx) = std::sync::mpsc::channel::<UpgradeMapChannelMsg>();
    app.upgrade_map_tx = Some(upgrade_tx);

    let (pkg_tx, pkg_rx) = std::sync::mpsc::channel::<(usize, u64, AppResult<Vec<Package>>)>();
    let (single_upgrade_tx, single_upgrade_rx): (SingleUpgradeSender, SingleUpgradeReceiver) =
        std::sync::mpsc::channel();
    let (multi_upgrade_tx, multi_upgrade_rx): (MultiUpgradeSender, MultiUpgradeReceiver) =
        std::sync::mpsc::channel();

    let (preload_tx, preload_rx) = std::sync::mpsc::channel::<PreloadChannelMsg>();
    app.preload_result_tx = Some(preload_tx);

    match app.package_managers.get(app.active_pm_index) {
        Some(pm) if pm.available => {
            let pm = pm.clone();
            app.begin_background_list_load(0, pm, &pkg_tx);
        }
        Some(pm) => {
            app.message = Some(format!("{} is not available", pm.name));
        }
        None => {}
    }

    refresh_preload_queue(&mut app, false);

    let (update_tx, update_rx) = std::sync::mpsc::channel::<(usize, Option<usize>)>();
    spawn_update_refresh(&app.package_managers, &update_tx);

    let mut should_quit = false;
    let mut multi_upgrade_ok = 0usize;
    let mut multi_upgrade_errors: Vec<String> = Vec::new();
    let mut multi_upgrade_successes: Vec<(usize, String)> = Vec::new();

    while !should_quit {
        while let Ok((idx, count)) = update_rx.try_recv() {
            if let Some(slot) = app.pm_pending_updates.get_mut(idx) {
                *slot = count;
            }
        }
        while let Ok((name, result)) = single_upgrade_rx.try_recv() {
            app.single_upgrade = None;
            match result {
                Ok(_) => {
                    app.message = Some(format!("Upgraded {name}"));
                    app.load_packages_sync();
                    for slot in &mut app.pm_pending_updates {
                        *slot = None;
                    }
                    spawn_update_refresh(&app.package_managers, &update_tx);
                }
                Err(e) => {
                    app.message = Some(format!("Error: {e}"));
                }
            }
        }
        while let Ok(event) = multi_upgrade_rx.try_recv() {
            match event {
                MultiUpgradeProgressEvent::StepStart { package_name } => {
                    if let Some(progress) = app.multi_upgrade.as_mut() {
                        progress.current_package = Some(package_name.clone());
                        progress.current_started_at = Some(Instant::now());
                    }
                    app.message = Some(format!("Upgrading {package_name}..."));
                }
                MultiUpgradeProgressEvent::StepDone {
                    pm_index,
                    package_name,
                    result,
                } => {
                    if let Some(progress) = app.multi_upgrade.as_mut() {
                        progress.done = progress.done.saturating_add(1);
                        progress.current_started_at = None;
                    }
                    match result {
                        Ok(_) => {
                            multi_upgrade_ok = multi_upgrade_ok.saturating_add(1);
                            multi_upgrade_successes.push((pm_index, package_name));
                        }
                        Err(e) => {
                            multi_upgrade_errors.push(format!("{package_name}: {e}"));
                        }
                    }
                }
                MultiUpgradeProgressEvent::Finished => {
                    app.multi_upgrade = None;
                    app.message = if !multi_upgrade_errors.is_empty() && multi_upgrade_ok == 0 {
                        Some(format!("Upgrade failed: {}", multi_upgrade_errors.join("; ")))
                    } else if multi_upgrade_errors.is_empty() {
                        Some(format!("Upgraded {multi_upgrade_ok} package(s)"))
                    } else {
                        Some(format!(
                            "Upgraded {multi_upgrade_ok} package(s); {} error(s): {}",
                            multi_upgrade_errors.len(),
                            multi_upgrade_errors.join("; ")
                        ))
                    };
                    multi_upgrade_ok = 0;
                    multi_upgrade_errors.clear();
                    app.load_packages_sync();
                    for slot in &mut app.pm_pending_updates {
                        *slot = None;
                    }
                    spawn_update_refresh(&app.package_managers, &update_tx);
                    if let Some(overlay) = app.all_upgradables.as_mut() {
                        overlay.rows.retain(|row| {
                            !multi_upgrade_successes
                                .iter()
                                .any(|(pm_idx, name)| row.pm_index == *pm_idx && row.name == *name)
                        });
                        overlay.selected.clear();
                        if overlay.rows.is_empty() {
                            overlay.cursor = 0;
                        } else {
                            overlay.cursor = overlay.cursor.min(overlay.rows.len() - 1);
                        }
                    }
                    multi_upgrade_successes.clear();
                }
            }
        }
        try_recv_package_list_results(&mut app, &pkg_rx);
        try_recv_preload_results(&mut app, &preload_rx);
        pump_preloads(&mut app);
        try_recv_upgrade_metadata(&mut app, &upgrade_rx);
        advance_upgrade_merge_chunk(&mut app);

        let _ = terminal.draw(|f| render_app(f, &app));

        if crossterm::event::poll(std::time::Duration::from_millis(100)).unwrap_or(false)
            && let Ok(Event::Key(KeyEvent {
                code,
                modifiers,
                kind: KeyEventKind::Press,
                ..
            })) = crossterm::event::read()
        {
            if app.all_upgradables.is_some() {
                handle_all_upgradables_key(&mut app, code, modifiers, &multi_upgrade_tx);
                continue;
            }

            if app.search_mode {
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
                    KeyCode::Char(c) => {
                        app.search_query.push(c);
                    }
                    _ => {}
                }
                continue;
            }

            match code {
                KeyCode::Esc => {
                    if !app.search_query.is_empty() {
                        app.search_query.clear();
                    } else {
                        should_quit = true;
                    }
                }
                KeyCode::Char('q') if !app.search_mode => {
                    should_quit = true;
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    app.select_previous();
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    app.select_next();
                }
                KeyCode::Char('d' | 'D') if modifiers.contains(KeyModifiers::CONTROL) => {
                    app.down(LIST_SCROLL_STEP);
                }
                KeyCode::Char('\x04') => {
                    app.down(LIST_SCROLL_STEP);
                }
                KeyCode::Char('u' | 'U') if modifiers.contains(KeyModifiers::CONTROL) => {
                    app.up(LIST_SCROLL_STEP);
                }
                KeyCode::Char('\x15') => {
                    app.up(LIST_SCROLL_STEP);
                }
                KeyCode::Char('/') if !app.search_mode => {
                    app.search_mode = true;
                }
                KeyCode::Char('u')
                    if !app.search_mode && !modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    if app.single_upgrade.is_some() {
                        app.message = Some("Another package upgrade is already running".to_string());
                        continue;
                    }
                    let pkg_name = app
                        .filtered_packages()
                        .get(app.selected_package_index)
                        .map(|(_, p)| p.name.clone());
                    if let (Some(name), Some(pm)) = (pkg_name, app.active_pm()) {
                        app.single_upgrade = Some(SingleUpgradeProgress {
                            package_name: name.clone(),
                            started_at: Instant::now(),
                        });
                        app.message = Some(format!("Upgrading {name}..."));
                        let tx = single_upgrade_tx.clone();
                        std::thread::spawn(move || {
                            let result = pm.upgrade_package(&name);
                            let _ = tx.send((name, result));
                        });
                    }
                }
                KeyCode::Char('r') if !app.search_mode => {
                    let pkg_name = app
                        .filtered_packages()
                        .get(app.selected_package_index)
                        .map(|(_, p)| p.name.clone());
                    if let (Some(name), Some(pm)) = (pkg_name, app.active_pm()) {
                        let result = pm.remove_package(&name);
                        match result {
                            Ok(_) => {
                                app.message = Some(format!("Removed {name}"));
                                app.load_packages_sync();
                            }
                            Err(e) => {
                                app.message = Some(format!("Error: {e}"));
                            }
                        }
                    }
                }
                KeyCode::Char('i') if !app.search_mode && !app.search_query.is_empty() => {
                    let name = app.search_query.clone();
                    if let Some(pm) = app.active_pm() {
                        let result = pm.install_package(&name);
                        match result {
                            Ok(_) => {
                                app.message = Some(format!("Installed {name}"));
                                app.load_packages_sync();
                            }
                            Err(e) => {
                                app.message = Some(format!("Error: {e}"));
                            }
                        }
                    }
                    app.search_query.clear();
                }
                KeyCode::Char('o') if !app.search_mode => {
                    app.show_outdated_only = !app.show_outdated_only;
                    let count = app.filtered_packages().len();
                    if count > 0 {
                        app.selected_package_index = app.selected_package_index.min(count - 1);
                    } else {
                        app.selected_package_index = 0;
                    }
                }
                KeyCode::Char('a') if !app.search_mode => {
                    let rows = collect_upgradables_from_cached_lists(
                        &app.package_managers,
                        &app.per_pm_packages,
                    );
                    app.all_upgradables = Some(AllUpgradablesOverlay {
                        loading: false,
                        rows,
                        cursor: 0,
                        selected: BTreeSet::new(),
                    });
                }
                KeyCode::Char('R') if modifiers.contains(KeyModifiers::CONTROL) => {
                    app.load_packages_sync();
                    for slot in &mut app.pm_pending_updates {
                        *slot = None;
                    }
                    spawn_update_refresh(&app.package_managers, &update_tx);
                }
                KeyCode::BackTab => {
                    cycle_active_pm(&mut app, false);
                }
                KeyCode::Tab if modifiers.contains(KeyModifiers::SHIFT) => {
                    cycle_active_pm(&mut app, false);
                }
                KeyCode::Tab => cycle_active_pm(&mut app, true),
                _ => {}
            }
        }
    }

    {
        let backend = terminal.backend_mut();
        let _ = execute!(backend, LeaveAlternateScreen);
    }
    persist_package_disk_cache_best_effort(&app);
    ratatui::restore();
}

#[cfg(test)]
mod installed_list_cache_tests {
    use super::*;

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
