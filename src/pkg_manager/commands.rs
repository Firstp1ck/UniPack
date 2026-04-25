//! Backend-specific remove/upgrade command execution, decomposed per family to keep
//! individual function complexity low.
#![allow(clippy::missing_docs_in_private_items)]

use std::process::{Command, Output};

use crate::{AppError, AppResult};

use super::PackageManager;
use super::util::{
    ensure_privileges_ready, pick_aur_helper_binary, pip_pacman_cli_pkg_name,
    pip_uses_arch_pacman_for_global,
};

/// Runs the backend-specific remove/uninstall command for `name`.
pub(super) fn remove_package(pm: &PackageManager, name: &str) -> AppResult<String> {
    ensure_privileges_ready(pm.name.as_str())?;
    let output = dispatch_remove(pm, name)?;
    finalize_output(&output, "removed", name)
}

/// Runs the backend-specific upgrade/update command for `name`.
pub(super) fn upgrade_package(pm: &PackageManager, name: &str) -> AppResult<String> {
    ensure_privileges_ready(pm.name.as_str())?;
    if pm.name == "apt" {
        return upgrade_apt(pm.command.as_str(), name);
    }
    let output = dispatch_upgrade(pm, name)?;
    finalize_output(&output, "upgraded", name)
}

/// Runs the backend-native full-system upgrade command.
pub(super) fn upgrade_system(pm: &PackageManager) -> AppResult<String> {
    ensure_privileges_ready(pm.name.as_str())?;
    let output = dispatch_system_upgrade(pm)?;
    if output.status.success() {
        Ok(format!(
            "Successfully completed system update for {}",
            pm.name
        ))
    } else {
        Err(AppError::from(
            String::from_utf8_lossy(&output.stderr).to_string(),
        ))
    }
}

/// Refreshes pacman package databases where relevant, then retries package upgrade.
pub(super) fn refresh_mirrors_and_upgrade_package(
    pm: &PackageManager,
    name: &str,
) -> AppResult<String> {
    ensure_privileges_ready(pm.name.as_str())?;
    let uses_pacman_db = matches!(pm.name.as_str(), "pacman" | "aur")
        || (pm.name == "pip" && pip_uses_arch_pacman_for_global());
    if !uses_pacman_db {
        return upgrade_package(pm, name);
    }

    let refresh = sudo_args(&["pacman", "-Syy", "--noconfirm"])?;
    if !refresh.status.success() {
        return Err(AppError::from(
            String::from_utf8_lossy(&refresh.stderr).to_string(),
        ));
    }

    let output = dispatch_upgrade(pm, name)?;
    finalize_output(&output, "upgraded", name)
}

/// Success `format!` / stderr-mapped error path shared by remove and upgrade.
fn finalize_output(output: &Output, action: &str, name: &str) -> AppResult<String> {
    if output.status.success() {
        Ok(format!("Successfully {action} {name}"))
    } else {
        Err(AppError::from(
            String::from_utf8_lossy(&output.stderr).to_string(),
        ))
    }
}

/// Executes `program` with `args` and collects combined `Output`.
fn run_args(program: &str, args: &[&str]) -> AppResult<Output> {
    Ok(Command::new(program).args(args).output()?)
}

/// Executes `sudo` with `args` and collects combined `Output`.
fn sudo_args(args: &[&str]) -> AppResult<Output> {
    Ok(Command::new("sudo").args(args).output()?)
}

/// Dispatch table for remove: one small arm per backend family.
fn dispatch_remove(pm: &PackageManager, name: &str) -> AppResult<Output> {
    if let Some(result) = try_pip_remove(pm, name) {
        return result;
    }
    let cmd = pm.command.as_str();
    match pm.name.as_str() {
        "npm" => run_args(cmd, &["uninstall", "-g", name]),
        "pnpm" | "bun" => run_args(cmd, &["remove", "-g", name]),
        "cargo" | "brew" | "flatpak" => run_args(cmd, &["uninstall", name]),
        "apt" | "snap" => sudo_args(&[cmd, "remove", name]),
        "pacman" => sudo_args(&[cmd, "-R", name]),
        "aur" => run_args(cmd, &["-R", name]),
        "rpm" => sudo_args(&[cmd, "-e", name]),
        _ => Err(AppError::from("Unknown package manager")),
    }
}

/// Pip-family remove (Arch pacman vs user pip) isolated to keep the main match simple.
fn try_pip_remove(pm: &PackageManager, name: &str) -> Option<AppResult<Output>> {
    if pm.name != "pip" {
        return None;
    }
    Some(if pip_uses_arch_pacman_for_global() {
        let pkg = pip_pacman_cli_pkg_name(name);
        sudo_args(&["pacman", "-R", "--noconfirm", &pkg])
    } else {
        run_args(pm.command.as_str(), &["uninstall", "-y", name])
    })
}

/// Dispatch table for upgrade (non-apt); apt has an extra `update` step and is handled separately.
fn dispatch_upgrade(pm: &PackageManager, name: &str) -> AppResult<Output> {
    if let Some(result) = try_pip_upgrade(pm, name) {
        return result;
    }
    if let Some(result) = try_node_upgrade(pm, name) {
        return result;
    }
    let cmd = pm.command.as_str();
    match pm.name.as_str() {
        "cargo" => run_args(cmd, &["install", name]),
        "brew" => run_args(cmd, &["upgrade", name]),
        "pacman" => sudo_args(&[cmd, "-S", "--needed", "--noconfirm", name]),
        "aur" => run_args(cmd, &["-S", "--needed", "--noconfirm", name]),
        "rpm" => sudo_args(&[cmd, "-Uvh", name]),
        "flatpak" => run_args(cmd, &["update", name]),
        "snap" => sudo_args(&[cmd, "refresh", name]),
        _ => Err(AppError::from("Unknown package manager")),
    }
}

/// Dispatch table for backend-native full-system updates.
fn dispatch_system_upgrade(pm: &PackageManager) -> AppResult<Output> {
    let cmd = pm.command.as_str();
    match pm.name.as_str() {
        "pacman" => sudo_args(&[cmd, "-Syu"]),
        "aur" => run_args(cmd, &["-Syu"]),
        "apt" => {
            let update = sudo_args(&[cmd, "update"])?;
            if !update.status.success() {
                return Ok(update);
            }
            sudo_args(&[cmd, "upgrade", "-y"])
        }
        "flatpak" => run_args(cmd, &["update", "-y"]),
        "snap" => sudo_args(&[cmd, "refresh"]),
        _ => Err(AppError::from(
            "Full system update is unsupported for this backend",
        )),
    }
}

/// Pip-family upgrade (Arch pacman via AUR helper or sudo, vs user `pip install --upgrade`).
fn try_pip_upgrade(pm: &PackageManager, name: &str) -> Option<AppResult<Output>> {
    if pm.name != "pip" {
        return None;
    }
    Some(if pip_uses_arch_pacman_for_global() {
        upgrade_pip_arch(name)
    } else {
        run_args(pm.command.as_str(), &["install", "--upgrade", name])
    })
}

/// Arch pip upgrade: AUR helper if available, otherwise `sudo pacman -S --needed`.
fn upgrade_pip_arch(name: &str) -> AppResult<Output> {
    let pkg = pip_pacman_cli_pkg_name(name);
    pick_aur_helper_binary().map_or_else(
        || sudo_args(&["pacman", "-S", "--needed", "--noconfirm", &pkg]),
        |aur| run_args(aur, &["-S", "--needed", "--noconfirm", &pkg]),
    )
}

/// Node-family upgrade (pnpm/npm/bun), including self-upgrade variants for `npm` and `bun`.
fn try_node_upgrade(pm: &PackageManager, name: &str) -> Option<AppResult<Output>> {
    let cmd = pm.command.as_str();
    match pm.name.as_str() {
        "npm" if name == "npm" => Some(run_args(cmd, &["install", "-g", "npm@latest"])),
        "bun" if name == "bun" => Some(run_args(cmd, &["upgrade"])),
        "pnpm" | "npm" | "bun" => Some(run_args(cmd, &["update", "-g", name])),
        _ => None,
    }
}

/// Apt `update` followed by `upgrade <name>`; surfaces the first failing stderr.
fn upgrade_apt(cmd: &str, name: &str) -> AppResult<String> {
    let update = sudo_args(&[cmd, "update"])?;
    if !update.status.success() {
        return Err(AppError::from(
            String::from_utf8_lossy(&update.stderr).to_string(),
        ));
    }
    let output = sudo_args(&[cmd, "upgrade", name])?;
    finalize_output(&output, "upgraded", name)
}
