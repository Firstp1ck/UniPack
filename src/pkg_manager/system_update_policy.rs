//! Full-system update eligibility resolver for overlay bulk upgrades.

use std::collections::{BTreeMap, BTreeSet};

use crate::all_upgradables::UpgradableRow;

use super::PackageManager;

/// Why full-system update was denied for a selected backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FullSystemUpdateDenyReason {
    /// Overlay rows changed since open-time collection.
    StaleOverlay,
    /// Selected rows do not include all current rows for that backend.
    PartialSelection,
    /// Backend has no full-system update policy in `UniPack`.
    UnsupportedBackend,
    /// Backend currently has no upgradable rows.
    EmptyTarget,
}

impl FullSystemUpdateDenyReason {
    /// Returns a stable short reason code for user-facing status text.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::StaleOverlay => "stale_overlay",
            Self::PartialSelection => "partial_selection",
            Self::UnsupportedBackend => "unsupported_backend",
            Self::EmptyTarget => "empty_target",
        }
    }
}

/// Backend-native full-system update command spec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FullSystemCommandSpec {
    /// Backend label (`pacman`, `aur`, `apt`, ...).
    pub backend: String,
    /// Human-readable command summary.
    pub command_preview: String,
}

/// Resolved worker task after executing policy checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedUpgradeTask {
    /// Run backend-native full-system update once.
    FullSystemUpdate {
        /// Index in `App::package_managers`.
        pm_index: usize,
        /// Status label shown in progress UI.
        display_name: String,
    },
    /// Upgrade one package using existing package-level path.
    PackageLevelUpgrade {
        /// Index in `App::package_managers`.
        pm_index: usize,
        /// Argument passed to `PackageManager::upgrade_package`.
        op_arg: String,
        /// Status label shown in progress UI.
        display_name: String,
    },
}

/// Output of the policy resolver used by overlay execution.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedUpgradePlan {
    /// Ordered tasks for worker execution.
    pub tasks: Vec<ResolvedUpgradeTask>,
    /// Per-backend status lines for toast messaging.
    pub notes: Vec<String>,
}

/// Returns command mapping for backends that support full-system updates.
#[must_use]
pub fn full_system_command_spec(pm: &PackageManager) -> Option<FullSystemCommandSpec> {
    match pm.name.as_str() {
        "pacman" => Some(FullSystemCommandSpec {
            backend: pm.name.clone(),
            command_preview: "sudo pacman -Syu".to_string(),
        }),
        "aur" => Some(FullSystemCommandSpec {
            backend: pm.name.clone(),
            command_preview: format!("{} -Syu", pm.command),
        }),
        "apt" => Some(FullSystemCommandSpec {
            backend: pm.name.clone(),
            command_preview: "sudo apt update && sudo apt upgrade -y".to_string(),
        }),
        "flatpak" => Some(FullSystemCommandSpec {
            backend: pm.name.clone(),
            command_preview: "flatpak update -y".to_string(),
        }),
        "snap" => Some(FullSystemCommandSpec {
            backend: pm.name.clone(),
            command_preview: "sudo snap refresh".to_string(),
        }),
        _ => None,
    }
}

/// Resolves backend tasks from selected rows and current fresh-row snapshot.
#[must_use]
pub fn resolve_upgrade_plan(
    overlay_rows: &[UpgradableRow],
    selected: &BTreeSet<usize>,
    current_rows: &[UpgradableRow],
    managers: &[PackageManager],
) -> ResolvedUpgradePlan {
    let selected_rows = collect_selected_rows(overlay_rows, selected);
    if selected_rows.is_empty() {
        return ResolvedUpgradePlan::default();
    }
    let stale_overlay = overlay_rows != current_rows;
    let selected_counts = selected_backend_counts(overlay_rows, selected);
    let current_counts = backend_counts(current_rows);

    let mut selected_by_pm: BTreeMap<usize, Vec<&UpgradableRow>> = BTreeMap::new();
    for row in selected_rows {
        selected_by_pm.entry(row.pm_index).or_default().push(row);
    }

    let mut tasks = Vec::new();
    let mut notes = Vec::new();

    for (pm_index, rows) in selected_by_pm {
        let Some(pm) = managers.get(pm_index) else {
            continue;
        };

        let decision = backend_full_system_eligibility(
            pm,
            pm_index,
            stale_overlay,
            &selected_counts,
            &current_counts,
        );

        if let Some(spec) = decision.0 {
            tasks.push(ResolvedUpgradeTask::FullSystemUpdate {
                pm_index,
                display_name: format!("{} (system update)", pm.name),
            });
            notes.push(format!(
                "{}: full-update ({})",
                pm.name, spec.command_preview
            ));
            continue;
        }

        let deny = decision
            .1
            .unwrap_or(FullSystemUpdateDenyReason::UnsupportedBackend);
        notes.push(format!("{}: package-level ({})", pm.name, deny.code()));

        for row in rows {
            tasks.push(ResolvedUpgradeTask::PackageLevelUpgrade {
                pm_index,
                op_arg: row
                    .upgrade_package_name
                    .clone()
                    .unwrap_or_else(|| row.name.clone()),
                display_name: row.name.clone(),
            });
        }
    }

    ResolvedUpgradePlan { tasks, notes }
}

fn backend_full_system_eligibility(
    pm: &PackageManager,
    pm_index: usize,
    stale_overlay: bool,
    selected_counts: &BTreeMap<usize, usize>,
    current_counts: &BTreeMap<usize, usize>,
) -> (
    Option<FullSystemCommandSpec>,
    Option<FullSystemUpdateDenyReason>,
) {
    let Some(spec) = full_system_command_spec(pm) else {
        return (None, Some(FullSystemUpdateDenyReason::UnsupportedBackend));
    };

    if stale_overlay {
        return (None, Some(FullSystemUpdateDenyReason::StaleOverlay));
    }

    let current_total = current_counts.get(&pm_index).copied().unwrap_or(0);
    if current_total == 0 {
        return (None, Some(FullSystemUpdateDenyReason::EmptyTarget));
    }

    let selected_total = selected_counts.get(&pm_index).copied().unwrap_or(0);
    if selected_total != current_total {
        return (None, Some(FullSystemUpdateDenyReason::PartialSelection));
    }

    (Some(spec), None)
}

fn collect_selected_rows<'a>(
    overlay_rows: &'a [UpgradableRow],
    selected: &BTreeSet<usize>,
) -> Vec<&'a UpgradableRow> {
    selected
        .iter()
        .filter_map(|idx| overlay_rows.get(*idx))
        .collect()
}

fn selected_backend_counts(
    overlay_rows: &[UpgradableRow],
    selected: &BTreeSet<usize>,
) -> BTreeMap<usize, usize> {
    let mut counts = BTreeMap::new();
    for row in collect_selected_rows(overlay_rows, selected) {
        *counts.entry(row.pm_index).or_insert(0) += 1;
    }
    counts
}

fn backend_counts(rows: &[UpgradableRow]) -> BTreeMap<usize, usize> {
    let mut counts = BTreeMap::new();
    for row in rows {
        *counts.entry(row.pm_index).or_insert(0) += 1;
    }
    counts
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    fn pm(name: &str, command: &str) -> PackageManager {
        PackageManager {
            name: name.to_string(),
            command: command.to_string(),
            list_command: command.to_string(),
            available: true,
            needs_root: false,
        }
    }

    fn row(pm_index: usize, pm_name: &str, name: &str) -> UpgradableRow {
        UpgradableRow {
            pm_index,
            pm_name: pm_name.to_string(),
            name: name.to_string(),
            upgrade_package_name: None,
            old_version: "1".to_string(),
            new_version: "2".to_string(),
        }
    }

    #[test]
    fn command_mapping_includes_supported_backends() {
        let pacman = pm("pacman", "pacman");
        let aur = pm("aur", "paru");
        let apt = pm("apt", "apt");
        let flatpak = pm("flatpak", "flatpak");
        let snap = pm("snap", "snap");
        let npm = pm("npm", "npm");

        assert_eq!(
            full_system_command_spec(&pacman)
                .expect("pacman should be supported")
                .command_preview,
            "sudo pacman -Syu"
        );
        assert_eq!(
            full_system_command_spec(&aur)
                .expect("aur should be supported")
                .command_preview,
            "paru -Syu"
        );
        assert_eq!(
            full_system_command_spec(&apt)
                .expect("apt should be supported")
                .command_preview,
            "sudo apt update && sudo apt upgrade -y"
        );
        assert_eq!(
            full_system_command_spec(&flatpak)
                .expect("flatpak should be supported")
                .command_preview,
            "flatpak update -y"
        );
        assert_eq!(
            full_system_command_spec(&snap)
                .expect("snap should be supported")
                .command_preview,
            "sudo snap refresh"
        );
        assert!(full_system_command_spec(&npm).is_none());
    }

    #[test]
    fn full_selection_and_fresh_rows_yield_full_system_update() {
        let managers = vec![pm("pacman", "pacman")];
        let overlay_rows = vec![row(0, "pacman", "vim"), row(0, "pacman", "git")];
        let current_rows = overlay_rows.clone();
        let selected = BTreeSet::from([0usize, 1usize]);

        let plan = resolve_upgrade_plan(&overlay_rows, &selected, &current_rows, &managers);
        assert_eq!(plan.tasks.len(), 1);
        assert!(matches!(
            plan.tasks[0],
            ResolvedUpgradeTask::FullSystemUpdate { pm_index: 0, .. }
        ));
    }

    #[test]
    fn partial_selection_falls_back_to_package_level() {
        let managers = vec![pm("pacman", "pacman")];
        let overlay_rows = vec![row(0, "pacman", "vim"), row(0, "pacman", "git")];
        let current_rows = overlay_rows.clone();
        let selected = BTreeSet::from([0usize]);

        let plan = resolve_upgrade_plan(&overlay_rows, &selected, &current_rows, &managers);
        assert_eq!(plan.tasks.len(), 1);
        assert!(matches!(
            plan.tasks[0],
            ResolvedUpgradeTask::PackageLevelUpgrade { .. }
        ));
        assert!(
            plan.notes
                .iter()
                .any(|line| line.contains(FullSystemUpdateDenyReason::PartialSelection.code()))
        );
    }

    #[test]
    fn stale_overlay_denies_full_system_update() {
        let managers = vec![pm("apt", "apt")];
        let overlay_rows = vec![row(0, "apt", "curl")];
        let current_rows = vec![row(0, "apt", "curl"), row(0, "apt", "git")];
        let selected = BTreeSet::from([0usize]);

        let plan = resolve_upgrade_plan(&overlay_rows, &selected, &current_rows, &managers);
        assert!(matches!(
            plan.tasks[0],
            ResolvedUpgradeTask::PackageLevelUpgrade { .. }
        ));
        assert!(
            plan.notes
                .iter()
                .any(|line| line.contains(FullSystemUpdateDenyReason::StaleOverlay.code()))
        );
    }

    #[test]
    fn unsupported_backend_stays_package_level() {
        let managers = vec![pm("npm", "npm")];
        let overlay_rows = vec![row(0, "npm", "typescript")];
        let current_rows = overlay_rows.clone();
        let selected = BTreeSet::from([0usize]);

        let plan = resolve_upgrade_plan(&overlay_rows, &selected, &current_rows, &managers);
        assert!(matches!(
            plan.tasks[0],
            ResolvedUpgradeTask::PackageLevelUpgrade { .. }
        ));
        assert!(
            plan.notes
                .iter()
                .any(|line| line.contains(FullSystemUpdateDenyReason::UnsupportedBackend.code()))
        );
    }

    #[test]
    fn mixed_backend_selection_yields_mixed_plan() {
        let managers = vec![pm("pacman", "pacman"), pm("npm", "npm")];
        let overlay_rows = vec![
            row(0, "pacman", "vim"),
            row(0, "pacman", "git"),
            row(1, "npm", "typescript"),
        ];
        let current_rows = overlay_rows.clone();
        let selected = BTreeSet::from([0usize, 1usize, 2usize]);

        let plan = resolve_upgrade_plan(&overlay_rows, &selected, &current_rows, &managers);
        assert_eq!(plan.tasks.len(), 2);
        assert!(plan.tasks.iter().any(|task| matches!(
            task,
            ResolvedUpgradeTask::FullSystemUpdate { pm_index: 0, .. }
        )));
        assert!(plan.tasks.iter().any(|task| matches!(
            task,
            ResolvedUpgradeTask::PackageLevelUpgrade { pm_index: 1, .. }
        )));
    }
}
