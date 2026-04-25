//! Shared data types, channel aliases, and constants used across the crate.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::all_upgradables::UpgradableRow;

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

pub type UpgradeMapChannelMsg = (usize, u64, AppResult<HashMap<String, String>>);
pub type UpgradeMapSender = std::sync::mpsc::Sender<UpgradeMapChannelMsg>;
pub type UpgradeMapReceiver = std::sync::mpsc::Receiver<UpgradeMapChannelMsg>;

pub type PreloadChannelMsg = (u64, usize, AppResult<Vec<Package>>);
pub type PreloadSender = std::sync::mpsc::Sender<PreloadChannelMsg>;
pub type PreloadReceiver = std::sync::mpsc::Receiver<PreloadChannelMsg>;

pub type SingleUpgradeChannelMsg = (String, AppResult<String>);
pub type SingleUpgradeSender = std::sync::mpsc::Sender<SingleUpgradeChannelMsg>;
pub type SingleUpgradeReceiver = std::sync::mpsc::Receiver<SingleUpgradeChannelMsg>;

pub type MultiUpgradeSender = std::sync::mpsc::Sender<MultiUpgradeProgressEvent>;
pub type MultiUpgradeReceiver = std::sync::mpsc::Receiver<MultiUpgradeProgressEvent>;

pub type PackageListChannelMsg = (usize, u64, AppResult<Vec<Package>>);
pub type PackageListSender = std::sync::mpsc::Sender<PackageListChannelMsg>;
pub type PackageListReceiver = std::sync::mpsc::Receiver<PackageListChannelMsg>;

pub type UpdateCountChannelMsg = (usize, Option<usize>);
pub type UpdateCountSender = std::sync::mpsc::Sender<UpdateCountChannelMsg>;
pub type UpdateCountReceiver = std::sync::mpsc::Receiver<UpdateCountChannelMsg>;

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

/// Applies `merge_packages_with_latest_map` in slices between frames so the UI stays responsive.
pub struct PendingUpgradeMerge {
    pub pm_index: usize,
    pub map: HashMap<String, String>,
    pub next_pkg_index: usize,
}

/// Max packages to upgrade-annotate per main-loop iteration (progressive "live" merge).
pub const PACKAGE_UPGRADE_MERGE_CHUNK: usize = 400;

/// Max concurrent background installs-only preloads (other package manager tabs).
pub const MAX_PARALLEL_PRELOADS: usize = 2;

/// Lines to move the cursor on Ctrl+d / Ctrl+u (and terminal EOT/NAK where applicable).
pub const LIST_SCROLL_STEP: usize = 20;

/// Multiselect overlay listing upgradable packages across all detected backends.
pub struct AllUpgradablesOverlay {
    /// Background scan in progress.
    pub loading: bool,
    /// Sorted rows for display and upgrade.
    pub rows: Vec<UpgradableRow>,
    /// Number of rows that were present when the overlay opened.
    pub opened_row_count: usize,
    /// Backend row counts captured when the overlay opened (keyed by `pm_index`).
    pub opened_backend_counts: BTreeMap<usize, usize>,
    /// Cursor into [`Self::rows`].
    pub cursor: usize,
    /// Row indices selected for upgrade.
    pub selected: BTreeSet<usize>,
    /// Substring filter while search mode is active.
    pub search_query: String,
    /// When true, typed keys append to `search_query` instead of triggering actions.
    pub search_mode: bool,
    /// When true, the query uses fuzzy subsequence matching instead of plain substring matching.
    pub search_fuzzy: bool,
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

/// Progress events from a bulk-upgrade worker thread to the UI loop.
pub enum MultiUpgradeProgressEvent {
    StepStart {
        package_name: String,
    },
    StepDone {
        pm_index: usize,
        package_name: String,
        used_full_system_update: bool,
        result: AppResult<String>,
    },
    Finished,
}
