//! Pending-update counts per backend (best-effort, `0` on failure).
#![allow(clippy::missing_docs_in_private_items)]

use crate::{AppResult, PackageStatus};

use super::latest::{latest_map_aur, latest_map_bun, latest_map_pip, latest_map_pnpm};
use super::list::list_installed_packages;
use super::util::{is_command_available, pip_uses_arch_pacman_for_global, run_shell};
use super::{PackageManager, merge_packages_with_latest_map};

/// Dispatches to the backend-specific updatable-count helper.
pub(super) fn count_pending_updates(pm: &PackageManager) -> AppResult<usize> {
    if !pm.available {
        return Ok(0);
    }
    match pm.name.as_str() {
        "pip" => count_pip_updates(),
        "npm" => count_npm_updates(),
        "pnpm" => count_pnpm_updates(),
        "bun" => count_bun_outdated_after_merge(pm),
        "cargo" => count_cargo_updates(),
        "brew" => count_brew_updates(),
        "apt" => count_apt_updates(),
        "pacman" => count_pacman_updates(),
        "aur" => count_aur_updates(&pm.command),
        "rpm" => count_rpm_updates(),
        "flatpak" => count_flatpak_updates(),
        "snap" => count_snap_updates(),
        _ => Ok(0),
    }
}

fn count_pip_updates() -> AppResult<usize> {
    if pip_uses_arch_pacman_for_global() {
        return Ok(latest_map_pip()?.len());
    }

    let output = run_shell(
        "pip list --outdated --format=json 2>/dev/null || pip3 list --outdated --format=json 2>/dev/null",
    )?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap_or_default();
    Ok(parsed.len())
}

fn count_npm_updates() -> AppResult<usize> {
    let output = run_shell("npm outdated -g --json 2>/dev/null; true")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Ok(0);
    }
    let parsed: serde_json::Value =
        serde_json::from_str(trimmed).unwrap_or(serde_json::Value::Null);
    let count = parsed.as_object().map_or(0, |o| {
        o.values()
            .filter(|info| {
                let current = info.get("current").and_then(|v| v.as_str()).unwrap_or("");
                let latest = info.get("latest").and_then(|v| v.as_str()).unwrap_or("");
                !latest.is_empty() && current != latest
            })
            .count()
    });
    Ok(count)
}

fn count_pnpm_updates() -> AppResult<usize> {
    Ok(latest_map_pnpm()?.len())
}

/// Updatable **global** bun packages: same list + merge as the UI so tab counts match `outdated` rows.
fn count_bun_outdated_after_merge(pm: &PackageManager) -> AppResult<usize> {
    let mut pkgs = list_installed_packages(pm.name.as_str())?;
    let map = latest_map_bun()?;
    merge_packages_with_latest_map(pm, &mut pkgs, &map);
    Ok(pkgs
        .iter()
        .filter(|p| p.status == PackageStatus::Outdated)
        .count())
}

fn count_cargo_updates() -> AppResult<usize> {
    if !is_command_available("cargo-install-update") {
        return Ok(0);
    }
    let output = run_shell("cargo install-update --list 2>/dev/null; true")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let count = stdout.lines().filter(|l| l.contains("Yes")).count();
    Ok(count)
}

fn count_brew_updates() -> AppResult<usize> {
    let output = run_shell("brew outdated --formula --quiet 2>/dev/null")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.lines().filter(|l| !l.trim().is_empty()).count())
}

fn count_apt_updates() -> AppResult<usize> {
    let output = run_shell("apt list --upgradable 2>/dev/null")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.lines().filter(|l| l.contains("[upgradable")).count())
}

fn count_pacman_updates() -> AppResult<usize> {
    if is_command_available("checkupdates") {
        let output = run_shell("checkupdates 2>/dev/null")?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Ok(stdout.lines().filter(|l| !l.trim().is_empty()).count());
    }
    let output = run_shell("pacman -Qu 2>/dev/null")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.lines().filter(|l| !l.trim().is_empty()).count())
}

fn count_aur_updates(cmd: &str) -> AppResult<usize> {
    Ok(latest_map_aur(cmd)?.len())
}

fn count_rpm_updates() -> AppResult<usize> {
    let shell_cmd = if is_command_available("dnf") {
        "dnf check-update -q 2>/dev/null; true"
    } else if is_command_available("yum") {
        "yum check-update -q 2>/dev/null; true"
    } else {
        return Ok(0);
    };
    let output = run_shell(shell_cmd)?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let count = stdout
        .lines()
        .filter(|l| {
            let t = l.trim();
            !t.is_empty()
                && !t.starts_with("Obsoleting")
                && !t.starts_with("Last metadata")
                && t.split_whitespace().count() >= 3
        })
        .count();
    Ok(count)
}

fn count_flatpak_updates() -> AppResult<usize> {
    let output = run_shell("flatpak remote-ls --updates --columns=application 2>/dev/null")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.lines().filter(|l| !l.trim().is_empty()).count())
}

fn count_snap_updates() -> AppResult<usize> {
    let output = run_shell("snap refresh --list 2>/dev/null")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.len() <= 1 {
        let has_all_up_to_date = lines
            .iter()
            .any(|l| l.to_lowercase().contains("all snaps up to date"));
        if has_all_up_to_date {
            return Ok(0);
        }
        return Ok(lines.len());
    }
    let first_lower = lines[0].to_lowercase();
    let skip = usize::from(first_lower.contains("name") && first_lower.contains("version"));
    Ok(lines.len().saturating_sub(skip))
}
