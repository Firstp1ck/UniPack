//! Shared subprocess, availability, and privilege helpers used by all backend modules.
#![allow(clippy::missing_docs_in_private_items)]

use std::process::Command;

use crate::{AppError, AppResult};

/// True when `cmd` can be located on `PATH` via `sh -c command -v`.
pub(super) fn is_command_available(cmd: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("command -v {cmd}")])
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Runs a shell snippet wrapped in `timeout 25` to bound long-running commands.
pub(super) fn run_shell(cmd: &str) -> AppResult<std::process::Output> {
    Ok(Command::new("sh")
        .args(["-c", &format!("timeout 25 {cmd}")])
        .output()?)
}

/// Preferred AUR helper binary (`yay`, then `paru`) or `None` when neither is installed.
pub(super) fn pick_aur_helper_binary() -> Option<&'static str> {
    if is_command_available("yay") {
        Some("yay")
    } else if is_command_available("paru") {
        Some("paru")
    } else {
        None
    }
}

/// Pacman package name for `python-*` CLI calls (`name` may be stripped module or full `python-*`).
pub(super) fn pip_pacman_cli_pkg_name(name: &str) -> String {
    if name.starts_with("python-") {
        name.to_string()
    } else {
        format!("python-{name}")
    }
}

/// True when `pacman` is on `PATH` (Arch / pacman-based: global Python modules use `python-*`).
#[must_use]
pub fn pip_uses_arch_pacman_for_global() -> bool {
    is_command_available("pacman")
}

/// Returns an error when the backend needs passwordless sudo and none is cached.
pub(super) fn ensure_privileges_ready(pm_name: &str) -> AppResult<()> {
    let needs_sudo = matches!(pm_name, "apt" | "snap" | "pacman" | "rpm" | "aur")
        || (pm_name == "pip" && pip_uses_arch_pacman_for_global());
    if !needs_sudo {
        return Ok(());
    }

    let sudo_ready = Command::new("sh")
        .args(["-c", "sudo -n true"])
        .output()
        .is_ok_and(|o| o.status.success());

    if sudo_ready {
        Ok(())
    } else {
        Err(AppError::from("Run sudo -v in terminal, then retry."))
    }
}
