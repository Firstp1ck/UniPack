//! Shell-backed listing and mutating commands for each supported package backend.
//!
//! The public surface (`PackageManager`, `pip_uses_arch_pacman_for_global`, and
//! `merge_packages_with_latest_map`) lives here; per-backend implementation details live in
//! the private submodules.
#![allow(clippy::missing_docs_in_private_items)]

use std::collections::HashMap;

use crate::{AppResult, Package, PackageStatus};

mod commands;
mod counts;
mod latest;
mod list;
mod util;

pub use util::pip_uses_arch_pacman_for_global;

/// Identifies a backend (pip, apt, pacman, …) and how to invoke it.
#[derive(Clone)]
pub struct PackageManager {
    /// Short label shown in the UI (e.g. `pip`, `aur`).
    pub name: String,
    /// Executable used for install/remove/upgrade-style actions.
    pub command: String,
    /// Tool used when listing installed packages (may differ from `command`).
    pub list_command: String,
    /// Whether the backend binary was found on `PATH`.
    pub available: bool,
    /// Whether privileged operations are expected for this backend.
    pub needs_root: bool,
}

impl PackageManager {
    /// Builds a manager record; `list_command` defaults to `command` until overridden.
    pub fn new(name: &str, command: &str, needs_root: bool) -> Self {
        Self {
            name: name.to_string(),
            command: command.to_string(),
            list_command: command.to_string(),
            available: util::is_command_available(command),
            needs_root,
        }
    }

    /// Lists installed packages only (no upgradable-metadata subprocesses).
    pub fn list_installed_packages(&self) -> AppResult<Vec<Package>> {
        list::list_installed_packages(self.name.as_str())
    }

    /// Lists installed packages for this backend, then merges available-update metadata when known.
    pub fn list_packages(&self) -> AppResult<Vec<Package>> {
        let mut pkgs = self.list_installed_packages()?;
        if let Ok(map) = latest::fetch_latest_version_map(
            self.name.as_str(),
            self.command.as_str(),
            self.available,
        ) {
            merge_packages_with_latest_map(self, &mut pkgs, &map);
        }
        Ok(pkgs)
    }

    /// Fetches backend-specific "latest / upgradable" version data (may shell out; can be slow).
    pub fn fetch_upgrade_versions_map(&self) -> AppResult<HashMap<String, String>> {
        latest::fetch_latest_version_map(self.name.as_str(), self.command.as_str(), self.available)
    }

    /// Runs the backend-specific remove/uninstall command for `name`.
    pub fn remove_package(&self, name: &str) -> AppResult<String> {
        commands::remove_package(self, name)
    }

    /// Runs the backend-specific upgrade/update command for `name`.
    pub fn upgrade_package(&self, name: &str) -> AppResult<String> {
        commands::upgrade_package(self, name)
    }

    /// Refreshes package databases/mirrors when supported, then retries upgrade for `name`.
    pub fn refresh_mirrors_and_upgrade_package(&self, name: &str) -> AppResult<String> {
        commands::refresh_mirrors_and_upgrade_package(self, name)
    }

    /// Returns how many packages this backend reports as updatable (best-effort; `0` on failure).
    pub fn count_pending_updates(&self) -> AppResult<usize> {
        counts::count_pending_updates(self)
    }
}

/// Merges `latest_version` / [`PackageStatus::Outdated`] from a pre-built name→version map.
#[allow(clippy::redundant_pub_crate)] // used from `lib.rs` (crate root), not a `pub` module
pub(crate) fn merge_packages_with_latest_map(
    pm: &PackageManager,
    packages: &mut [Package],
    map: &HashMap<String, String>,
) {
    if map.is_empty() {
        return;
    }
    for p in packages {
        let hit = match pm.name.as_str() {
            "pip" if pip_uses_arch_pacman_for_global() => {
                let key = p.installed_by.as_deref().unwrap_or(p.name.as_str());
                map.get(key)
                    .or_else(|| map.get(&key.to_ascii_lowercase()))
                    .cloned()
            }
            "pip" => map.get(&p.name.to_ascii_lowercase()).cloned(),
            _ => map.get(&p.name).cloned(),
        };
        if let Some(latest) = hit
            && latest != p.version
        {
            p.latest_version = Some(latest);
            p.status = PackageStatus::Outdated;
        }
    }
}
