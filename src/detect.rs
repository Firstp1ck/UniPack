//! Host detection: distro name, available package managers, and pre-TUI sudo warm-up.

use std::io::{self, BufRead, IsTerminal, Write as IoWrite};
use std::path::Path;
use std::process::{Command, Stdio};

use crate::model::{AppResult, Package};
use crate::pkg_manager::{PackageManager, pip_uses_arch_pacman_for_global};

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

/// Pacman `python-*` package name for pip-on-pacman remove/upgrade.
///
/// `p.name` is shown without the `python-` prefix; this returns the name with that prefix when
/// applicable so the underlying pacman command receives the real package name.
pub fn pip_pacman_op_arg(pm: &PackageManager, p: &Package) -> String {
    if pm.name == "pip" && pip_uses_arch_pacman_for_global() {
        p.installed_by.clone().unwrap_or_else(|| p.name.clone())
    } else {
        p.name.clone()
    }
}

/// Detects package managers by probing for their binaries in parallel.
pub fn detect_package_managers() -> Vec<PackageManager> {
    const PM_CONFIGS: &[(&str, &str, &str, bool)] = &[
        ("pip", "pip3", "pip", false),
        ("npm", "npm", "npm", false),
        ("pnpm", "pnpm", "pnpm", false),
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

    if pip_uses_arch_pacman_for_global() {
        if let Some(pm) = managers.iter_mut().find(|m| m.name == "pip") {
            pm.available = true;
            pm.needs_root = true;
        } else {
            managers.push(PackageManager {
                name: "pip".to_string(),
                command: "pip3".to_string(),
                list_command: "pacman".to_string(),
                available: true,
                needs_root: true,
            });
        }
    }

    if !managers.iter().any(|m| m.name == "aur") && is_command_available("paru") {
        managers.push(PackageManager {
            name: "aur".to_string(),
            command: "paru".to_string(),
            list_command: "paru".to_string(),
            available: true,
            needs_root: sudo_ok,
        });
    }

    managers
}

/// Returns `true` when `command -v cmd` succeeds via a shell lookup.
pub fn is_command_available(cmd: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("command -v {cmd}")])
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Backends whose installs/upgrades go through privileged tooling (`sudo`, etc.).
#[must_use]
fn pm_benefits_from_sudo_timestamp(pm: &PackageManager) -> bool {
    pm.available
        && (matches!(pm.name.as_str(), "apt" | "pacman" | "aur" | "rpm" | "snap")
            || (pm.name == "pip" && pip_uses_arch_pacman_for_global()))
}

/// Runs `sudo -v` with inherited stdio (password prompt on the real terminal).
fn run_sudo_v_inherit_stdio() -> AppResult<()> {
    let status = Command::new("sudo")
        .args(["-v"])
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "sudo -v failed ({status}); authenticate in this terminal before starting UniPack"
        )
        .into())
    }
}

/// When privileged backends exist, asks on the CLI whether to run `sudo -v` before the TUI.
///
/// Returns `Ok(true)` only after a successful `sudo -v`. Returns `Ok(false)` when the user
/// declines, when stdin is not a TTY, or when sudo / privileged backends are not applicable.
pub fn offer_sudo_warm_before_tui(package_managers: &[PackageManager]) -> AppResult<bool> {
    if !cfg!(unix) {
        return Ok(false);
    }
    if !package_managers.iter().any(pm_benefits_from_sudo_timestamp) {
        return Ok(false);
    }
    if !is_command_available("sudo") {
        return Ok(false);
    }
    if !io::stdin().is_terminal() {
        return Ok(false);
    }
    eprint!(
        "Some package managers need elevated privileges to install or upgrade. Authenticate with sudo now? [y/N] "
    );
    let _ = io::stderr().flush();
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    let low = line.trim().to_ascii_lowercase();
    let consent = matches!(low.as_str(), "y" | "yes");
    if !consent {
        return Ok(false);
    }
    run_sudo_v_inherit_stdio()?;
    Ok(true)
}
