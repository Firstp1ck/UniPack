//! `UniPack`: cross-backend package listing and a [`ratatui`] event loop.
//!
//! The binary entry point is thin; this library crate holds application state, rendering, and
//! input handling. See the submodules for implementation:
//!
//! - [`app`]: central `App` state and per-tab lifecycle methods.
//! - [`detect`]: distro/PM detection and pre-TUI sudo warm-up.
//! - [`ui`]: Ratatui rendering (header, body, footer, overlay, info strip, version diff).
//! - [`overlay`]: all-upgradables overlay state mutations and key dispatch.
//! - [`workers`]: background worker orchestration and channel drains.
//! - [`run_loop`]: terminal lifecycle and the draw/input main loop.
//! - [`model`]: shared data types and channel aliases.

#![allow(clippy::missing_docs_in_private_items)]

mod all_upgradables;
mod app;
mod detect;
mod model;
mod overlay;
mod package_cache;
mod pkg_manager;
mod run_loop;
mod ui;
mod workers;

pub use all_upgradables::{
    UpgradableRow, collect_all_upgradables, collect_upgradables_from_cached_lists,
};
pub use app::App;
pub use detect::detect_distro;
pub use model::{
    AllUpgradablesOverlay, AppError, AppResult, FilterMode, MultiUpgradeProgress, Package,
    PackageStatus, SingleUpgradeProgress, SortField,
};
pub use run_loop::run;
