//! Cross-backend collection of packages that report an available upgrade.

use std::cmp::Ordering;

use crate::pkg_manager::{PackageManager, pip_uses_arch_pacman_for_global};
use crate::{Package, PackageStatus};

/// One upgradable row for the bulk-update overlay.
///
/// **What:** Binds a package name and version transition to a backend index/label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpgradableRow {
    /// Index into the app-level [`crate::App::package_managers`] slice.
    pub pm_index: usize,
    /// Short backend label (for example `pip` or `pacman`).
    pub pm_name: String,
    /// Package name shown in the UI (e.g. stripped `python-` on pacman-based pip mode).
    pub name: String,
    /// When set, passed to the package manager for upgrades instead of [`Self::name`].
    pub upgrade_package_name: Option<String>,
    /// Installed or current version string.
    pub old_version: String,
    /// Reported target version string.
    pub new_version: String,
}

impl UpgradableRow {
    /// Builds a [`Package`] so shared version-diff rendering can be reused.
    #[must_use]
    pub fn as_package_for_display(&self) -> Package {
        Package {
            name: self.name.clone(),
            version: self.old_version.clone(),
            latest_version: Some(self.new_version.clone()),
            status: PackageStatus::Outdated,
            size: 0,
            description: String::new(),
            repository: None,
            installed_by: None,
        }
    }
}

fn push_upgradable_rows_from_packages(
    rows: &mut Vec<UpgradableRow>,
    pm_index: usize,
    pm_name: &str,
    pkgs: &[Package],
) {
    for p in pkgs {
        let Some(new_version) = p.latest_version.clone() else {
            continue;
        };
        if new_version == p.version {
            continue;
        }
        let upgrade_package_name = if pm_name == "pip" && pip_uses_arch_pacman_for_global() {
            p.installed_by.clone()
        } else {
            None
        };
        rows.push(UpgradableRow {
            pm_index,
            pm_name: pm_name.to_string(),
            name: p.name.clone(),
            upgrade_package_name,
            old_version: p.version.clone(),
            new_version,
        });
    }
}

fn sort_upgradable_rows(rows: &mut [UpgradableRow]) {
    rows.sort_by(|a, b| match a.pm_name.cmp(&b.pm_name) {
        Ordering::Equal => a.name.cmp(&b.name),
        other => other,
    });
}

/// Collects upgradable rows from in-memory package lists (no subprocess rescan).
///
/// **What:** Uses each backend’s cached [`crate::Package`] slice when present.
///
/// **Output:** Rows sorted by backend label, then package name.
///
/// **Details:** Skips backends with no cached list yet or no reported `latest_version`.
#[must_use]
pub fn collect_upgradables_from_cached_lists(
    managers: &[PackageManager],
    per_pm: &[Option<Vec<Package>>],
) -> Vec<UpgradableRow> {
    let mut rows = Vec::new();
    for (pm_index, pm) in managers.iter().enumerate() {
        if !pm.available {
            continue;
        }
        let Some(pkgs) = per_pm.get(pm_index).and_then(|s| s.as_deref()) else {
            continue;
        };
        push_upgradable_rows_from_packages(&mut rows, pm_index, pm.name.as_str(), pkgs);
    }
    sort_upgradable_rows(&mut rows);
    rows
}

/// Collects upgradable rows from every available backend via a full list + upgrade scan.
///
/// **What:** Lists each backend’s packages and keeps entries whose latest version differs.
///
/// **Output:** Rows sorted by backend label, then package name.
///
/// **Details:** Backends whose list fails are skipped so one broken tool does not abort the scan.
#[must_use]
pub fn collect_all_upgradables(managers: &[PackageManager]) -> Vec<UpgradableRow> {
    let mut rows = Vec::new();
    for (pm_index, pm) in managers.iter().enumerate() {
        if !pm.available {
            continue;
        }
        let Ok(pkgs) = pm.list_packages() else {
            continue;
        };
        push_upgradable_rows_from_packages(&mut rows, pm_index, pm.name.as_str(), &pkgs);
    }
    sort_upgradable_rows(&mut rows);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_empty_managers_yields_empty() {
        assert!(collect_all_upgradables(&[]).is_empty());
    }

    #[test]
    fn collect_from_cache_respects_latest_version() {
        let pm = PackageManager {
            name: "pip".to_string(),
            command: "pip3".to_string(),
            list_command: "pip3".to_string(),
            available: true,
            needs_root: false,
        };
        let managers = [pm];
        let pkgs = vec![Package {
            name: "wheel".to_string(),
            version: "0.41".to_string(),
            latest_version: Some("0.50".to_string()),
            status: PackageStatus::Outdated,
            size: 0,
            description: String::new(),
            repository: None,
            installed_by: None,
        }];
        let per_pm = vec![Some(pkgs)];
        let rows = collect_upgradables_from_cached_lists(&managers, &per_pm);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "wheel");
        assert_eq!(rows[0].new_version, "0.50");
    }

    #[test]
    fn collect_from_cache_skips_missing_slot() {
        let pm = PackageManager {
            name: "pip".to_string(),
            command: "pip3".to_string(),
            list_command: "pip3".to_string(),
            available: true,
            needs_root: false,
        };
        let managers = [pm];
        let per_pm = vec![None];
        assert!(collect_upgradables_from_cached_lists(&managers, &per_pm).is_empty());
    }

    #[test]
    fn sort_orders_by_pm_then_name() {
        let mut rows = [
            UpgradableRow {
                pm_index: 1,
                pm_name: "npm".to_string(),
                name: "alpha".to_string(),
                upgrade_package_name: None,
                old_version: "1".to_string(),
                new_version: "2".to_string(),
            },
            UpgradableRow {
                pm_index: 0,
                pm_name: "apt".to_string(),
                name: "zzz".to_string(),
                upgrade_package_name: None,
                old_version: "1".to_string(),
                new_version: "2".to_string(),
            },
            UpgradableRow {
                pm_index: 0,
                pm_name: "apt".to_string(),
                name: "aaa".to_string(),
                upgrade_package_name: None,
                old_version: "1".to_string(),
                new_version: "2".to_string(),
            },
        ];
        rows.as_mut_slice()
            .sort_by(|a, b| match a.pm_name.cmp(&b.pm_name) {
                Ordering::Equal => a.name.cmp(&b.name),
                other => other,
            });
        assert_eq!(rows[0].name, "aaa");
        assert_eq!(rows[1].name, "zzz");
        assert_eq!(rows[2].name, "alpha");
    }
}
