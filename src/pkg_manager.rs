//! Shell-backed listing and mutating commands for each supported package backend.
#![allow(clippy::missing_docs_in_private_items)]
#![allow(clippy::unused_self)] // list_* helpers share a signature for dispatch; most ignore `self`.

use std::collections::{HashMap, HashSet};
use std::process::Command;
use std::sync::LazyLock;

use regex::Regex;

use crate::{AppError, AppResult, Package, PackageStatus};

/// Regex for lines from `cargo install-update --list` that indicate an available update.
static CARGO_INSTALL_UPDATE_LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(\S+)\s+v(\S+)\s+Yes\s+v(\S+)")
        .expect("static regex for cargo install-update lines should compile")
});

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

/// True when `pacman` is on `PATH` (Arch and pacman-based distros: global Python modules use `python-*` packages).
#[must_use]
pub fn pip_uses_arch_pacman_for_global() -> bool {
    is_command_available("pacman")
}

fn pick_aur_helper_binary() -> Option<&'static str> {
    if is_command_available("yay") {
        Some("yay")
    } else if is_command_available("paru") {
        Some("paru")
    } else {
        None
    }
}

/// Pacman package name for `python-*` CLI calls (`name` may be the stripped module name or full `python-*`).
fn pip_pacman_cli_pkg_name(name: &str) -> String {
    if name.starts_with("python-") {
        name.to_string()
    } else {
        format!("python-{name}")
    }
}

impl PackageManager {
    /// Builds a manager record; `list_command` defaults to `command` until overridden.
    pub fn new(name: &str, command: &str, needs_root: bool) -> Self {
        Self {
            name: name.to_string(),
            command: command.to_string(),
            list_command: command.to_string(),
            available: is_command_available(command),
            needs_root,
        }
    }

    /// Lists installed packages only (no upgradable-metadata subprocesses).
    pub fn list_installed_packages(&self) -> AppResult<Vec<Package>> {
        match self.name.as_str() {
            "pip" => self.list_pip(),
            "npm" => self.list_npm(),
            "bun" => self.list_bun(),
            "cargo" => self.list_cargo(),
            "brew" => self.list_brew(),
            "apt" => self.list_apt(),
            "pacman" => self.list_pacman(),
            "aur" => self.list_aur(),
            "rpm" => self.list_rpm(),
            "flatpak" => self.list_flatpak(),
            "snap" => self.list_snap(),
            _ => Ok(Vec::new()),
        }
    }

    /// Lists installed packages for this backend, then merges available-update metadata when known.
    pub fn list_packages(&self) -> AppResult<Vec<Package>> {
        let mut pkgs = self.list_installed_packages()?;
        self.apply_update_info(&mut pkgs);
        Ok(pkgs)
    }

    /// Fetches backend-specific “latest / upgradable” version data (may shell out; can be slow).
    pub fn fetch_upgrade_versions_map(&self) -> AppResult<HashMap<String, String>> {
        fetch_latest_version_map(self)
    }

    fn apply_update_info(&self, packages: &mut [Package]) {
        let Ok(map) = fetch_latest_version_map(self) else {
            return;
        };
        merge_packages_with_latest_map(self, packages, &map);
    }

    /// Runs the backend-specific remove/uninstall command for `name`.
    pub fn remove_package(&self, name: &str) -> AppResult<String> {
        ensure_privileges_ready(self.name.as_str())?;
        let output = match self.name.as_str() {
            "pip" if pip_uses_arch_pacman_for_global() => {
                let pkg = pip_pacman_cli_pkg_name(name);
                Command::new("sudo")
                    .args(["pacman", "-R", "--noconfirm", &pkg])
                    .output()?
            }
            "pip" => Command::new(&self.command)
                .args(["uninstall", "-y", name])
                .output()?,
            "npm" => Command::new(&self.command)
                .args(["uninstall", "-g", name])
                .output()?,
            "bun" => Command::new(&self.command)
                .args(["remove", "-g", name])
                .output()?,
            "cargo" | "brew" | "flatpak" => Command::new(&self.command)
                .args(["uninstall", name])
                .output()?,
            "apt" | "snap" => Command::new("sudo")
                .args([self.command.as_str(), "remove", name])
                .output()?,
            "pacman" => Command::new("sudo")
                .args([self.command.as_str(), "-R", name])
                .output()?,
            "aur" => Command::new(&self.command).args(["-R", name]).output()?,
            "rpm" => Command::new("sudo")
                .args([self.command.as_str(), "-e", name])
                .output()?,
            _ => return Err(AppError::from("Unknown package manager")),
        };

        if output.status.success() {
            Ok(format!("Successfully removed {name}"))
        } else {
            Err(AppError::from(
                String::from_utf8_lossy(&output.stderr).to_string(),
            ))
        }
    }

    /// Runs the backend-specific upgrade/update command for `name`.
    pub fn upgrade_package(&self, name: &str) -> AppResult<String> {
        ensure_privileges_ready(self.name.as_str())?;

        if self.name == "apt" {
            let update = Command::new("sudo")
                .args([self.command.as_str(), "update"])
                .output()?;
            if !update.status.success() {
                return Err(AppError::from(
                    String::from_utf8_lossy(&update.stderr).to_string(),
                ));
            }
            let output = Command::new("sudo")
                .args([self.command.as_str(), "upgrade", name])
                .output()?;
            return if output.status.success() {
                Ok(format!("Successfully upgraded {name}"))
            } else {
                Err(AppError::from(
                    String::from_utf8_lossy(&output.stderr).to_string(),
                ))
            };
        }

        let output = match self.name.as_str() {
            "pip" if pip_uses_arch_pacman_for_global() => {
                let pkg = pip_pacman_cli_pkg_name(name);
                if let Some(aur) = pick_aur_helper_binary() {
                    Command::new(aur)
                        .args(["-S", "--needed", "--noconfirm", &pkg])
                        .output()?
                } else {
                    Command::new("sudo")
                        .args(["pacman", "-S", "--needed", "--noconfirm", &pkg])
                        .output()?
                }
            }
            "pip" => Command::new(&self.command)
                .args(["install", "--upgrade", name])
                .output()?,
            "npm" if name == "npm" => Command::new(&self.command)
                .args(["install", "-g", "npm@latest"])
                .output()?,
            "bun" if name == "bun" => Command::new(&self.command).args(["upgrade"]).output()?,
            "npm" | "bun" => Command::new(&self.command)
                .args(["update", "-g", name])
                .output()?,
            "cargo" => Command::new(&self.command)
                .args(["install", name])
                .output()?,
            "brew" => Command::new(&self.command)
                .args(["upgrade", name])
                .output()?,
            "pacman" => Command::new("sudo")
                .args([self.command.as_str(), "-S", name])
                .output()?,
            "aur" => Command::new(&self.command)
                .args(["-S", "--needed", "--noconfirm", name])
                .output()?,
            "rpm" => Command::new("sudo")
                .args([self.command.as_str(), "-Uvh", name])
                .output()?,
            "flatpak" => Command::new(&self.command)
                .args(["update", name])
                .output()?,
            "snap" => Command::new("sudo")
                .args([self.command.as_str(), "refresh", name])
                .output()?,
            _ => return Err(AppError::from("Unknown package manager")),
        };

        if output.status.success() {
            Ok(format!("Successfully upgraded {name}"))
        } else {
            Err(AppError::from(
                String::from_utf8_lossy(&output.stderr).to_string(),
            ))
        }
    }

    /// Returns how many packages this backend reports as updatable (best-effort; `0` on failure).
    pub fn count_pending_updates(&self) -> AppResult<usize> {
        if !self.available {
            return Ok(0);
        }
        match self.name.as_str() {
            "pip" => count_pip_updates(),
            "npm" => count_npm_updates(),
            "bun" => self.count_bun_outdated_after_merge(),
            "cargo" => count_cargo_updates(),
            "brew" => count_brew_updates(),
            "apt" => count_apt_updates(),
            "pacman" => count_pacman_updates(),
            "aur" => count_aur_updates(&self.command),
            "rpm" => count_rpm_updates(),
            "flatpak" => count_flatpak_updates(),
            "snap" => count_snap_updates(),
            _ => Ok(0),
        }
    }

    fn list_pip(&self) -> AppResult<Vec<Package>> {
        if pip_uses_arch_pacman_for_global() {
            return Self::list_arch_python_pacman_packages();
        }

        let output = Command::new("sh")
            .args([
                "-c",
                "pip list --format=json 2>/dev/null || pip3 list --format=json",
            ])
            .output()?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let packages: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap_or_default();

        let mut result = Vec::new();
        for pkg in packages {
            result.push(Package {
                name: pkg["name"].as_str().unwrap_or("").to_string(),
                version: pkg["version"].as_str().unwrap_or("").to_string(),
                latest_version: None,
                status: PackageStatus::Installed,
                size: 0,
                description: String::new(),
                repository: None,
                installed_by: None,
            });
        }

        Ok(result)
    }

    /// Installed `python-*` packages from pacman (Arch global Python modules).
    fn list_arch_python_pacman_packages() -> AppResult<Vec<Package>> {
        let output = Command::new("sh")
            .args(["-c", "pacman -Q 2>/dev/null"])
            .output()?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut result = Vec::new();

        for line in stdout.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 2 {
                continue;
            }
            let full = parts[0];
            if !full.starts_with("python-") {
                continue;
            }
            let display = full
                .strip_prefix("python-")
                .filter(|s| !s.is_empty())
                .unwrap_or(full)
                .to_string();
            result.push(Package {
                name: display,
                version: parts[1].to_string(),
                latest_version: None,
                status: PackageStatus::Installed,
                size: 0,
                description: String::new(),
                repository: Some("pacman".to_string()),
                installed_by: Some(full.to_string()),
            });
        }

        Ok(result)
    }

    fn list_npm(&self) -> AppResult<Vec<Package>> {
        let output = Command::new("sh")
            .args([
                "-c",
                "npm list -g --json --depth=0 2>/dev/null || npm list -g --json",
            ])
            .output()?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let data: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_default();

        let mut result = Vec::new();
        if let Some(dependencies) = data["dependencies"].as_object() {
            for (name, info) in dependencies {
                let version = info["version"].as_str().unwrap_or("").to_string();
                let description = info["description"].as_str().unwrap_or("").to_string();

                result.push(Package {
                    name: name.clone(),
                    version,
                    latest_version: None,
                    status: PackageStatus::Installed,
                    size: 0,
                    description,
                    repository: None,
                    installed_by: None,
                });
            }
        }

        Ok(result)
    }

    /// Updatable **global** packages: same list + merge as the UI, so the tab count matches `outdated` rows.
    fn count_bun_outdated_after_merge(&self) -> AppResult<usize> {
        let mut pkgs = self.list_installed_packages()?;
        let map = latest_map_bun()?;
        merge_packages_with_latest_map(self, &mut pkgs, &map);
        Ok(pkgs
            .iter()
            .filter(|p| p.status == PackageStatus::Outdated)
            .count())
    }

    fn list_bun(&self) -> AppResult<Vec<Package>> {
        let output = Command::new("sh")
            .args(["-c", "bun pm ls -g 2>/dev/null"])
            .output()?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut result = Vec::new();

        for line in stdout.lines() {
            if !(line.starts_with("├── ") || line.starts_with("└── ")) {
                continue;
            }

            let tail = line
                .strip_prefix("├── ")
                .or_else(|| line.strip_prefix("└── "))
                .unwrap_or("")
                .trim();
            let Some(at) = tail.rfind('@') else {
                continue;
            };
            let (name, version) = tail.split_at(at);
            if name.is_empty() {
                continue;
            }
            let version = version.trim_start_matches('@').to_string();

            result.push(Package {
                name: name.to_string(),
                version,
                latest_version: None,
                status: PackageStatus::Installed,
                size: 0,
                description: String::new(),
                repository: None,
                installed_by: None,
            });
        }

        Ok(result)
    }

    fn list_cargo(&self) -> AppResult<Vec<Package>> {
        let output = Command::new("sh")
            .args(["-c", "cargo install --list 2>/dev/null"])
            .output()?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut result = Vec::new();

        for line in stdout.lines() {
            let parts: Vec<&str> = line.splitn(2, " v").collect();
            if parts.len() == 2 {
                result.push(Package {
                    name: parts[0].trim().to_string(),
                    version: parts[1].trim().trim_end_matches(':').to_string(),
                    latest_version: None,
                    status: PackageStatus::Installed,
                    size: 0,
                    description: String::new(),
                    repository: None,
                    installed_by: None,
                });
            }
        }

        Ok(result)
    }

    fn list_brew(&self) -> AppResult<Vec<Package>> {
        let output = Command::new("sh")
            .args(["-c", "brew list --versions 2>/dev/null"])
            .output()?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut result = Vec::new();

        for line in stdout.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if !parts.is_empty() {
                let name = parts[0].to_string();
                let version = if parts.len() > 1 {
                    parts[1..].join(" ")
                } else {
                    String::new()
                };
                result.push(Package {
                    name,
                    version,
                    latest_version: None,
                    status: PackageStatus::Installed,
                    size: 0,
                    description: String::new(),
                    repository: Some("homebrew".to_string()),
                    installed_by: None,
                });
            }
        }

        Ok(result)
    }

    #[allow(clippy::literal_string_with_formatting_args)]
    fn list_apt(&self) -> AppResult<Vec<Package>> {
        let output = Command::new("sh")
            .args([
                "-c",
                "dpkg-query -W -f='${package} ${version} ${status}\\n' 2>/dev/null",
            ])
            .output()?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut result = Vec::new();

        for line in stdout.lines() {
            let parts: Vec<&str> = line.split(' ').collect();
            if parts.len() >= 3 {
                let status = if line.contains(" installed") {
                    PackageStatus::Installed
                } else {
                    PackageStatus::Available
                };

                result.push(Package {
                    name: parts[0].to_string(),
                    version: parts[1].to_string(),
                    latest_version: None,
                    status,
                    size: 0,
                    description: String::new(),
                    repository: None,
                    installed_by: None,
                });
            }
        }

        Ok(result)
    }

    fn list_pacman(&self) -> AppResult<Vec<Package>> {
        let output = Command::new("sh")
            .args(["-c", "pacman -Q 2>/dev/null"])
            .output()?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut result = Vec::new();

        for line in stdout.lines() {
            let parts: Vec<&str> = line.split(' ').collect();
            if parts.len() >= 2 {
                result.push(Package {
                    name: parts[0].to_string(),
                    version: parts[1].to_string(),
                    latest_version: None,
                    status: PackageStatus::Installed,
                    size: 0,
                    description: String::new(),
                    repository: Some("core".to_string()),
                    installed_by: None,
                });
            }
        }

        Ok(result)
    }

    fn list_aur(&self) -> AppResult<Vec<Package>> {
        let output = Command::new("sh")
            .args(["-c", "yay -Qem 2>/dev/null || paru -Qem 2>/dev/null"])
            .output()?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut result = Vec::new();

        for line in stdout.lines() {
            let parts: Vec<&str> = line.split(' ').collect();
            if parts.len() >= 2 {
                result.push(Package {
                    name: parts[0].to_string(),
                    version: parts[1].to_string(),
                    latest_version: None,
                    status: PackageStatus::Installed,
                    size: 0,
                    description: String::new(),
                    repository: Some("aur".to_string()),
                    installed_by: None,
                });
            }
        }

        Ok(result)
    }

    fn list_rpm(&self) -> AppResult<Vec<Package>> {
        let output = Command::new("sh")
            .args([
                "-c",
                "rpm -qa --queryformat '%{NAME}\\n%{EVR}\\n' 2>/dev/null",
            ])
            .output()?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = stdout.lines().collect();
        let mut result = Vec::new();

        let mut i = 0;
        while i < lines.len() - 1 {
            let name = lines[i].to_string();
            let version = lines.get(i + 1).unwrap_or(&"").to_string();
            result.push(Package {
                name,
                version,
                latest_version: None,
                status: PackageStatus::Installed,
                size: 0,
                description: String::new(),
                repository: None,
                installed_by: None,
            });
            i += 2;
        }

        Ok(result)
    }

    fn list_flatpak(&self) -> AppResult<Vec<Package>> {
        let output = Command::new("sh")
            .args([
                "-c",
                "flatpak list --app --columns=application,version 2>/dev/null",
            ])
            .output()?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut result = Vec::new();

        for line in stdout.lines().skip(1) {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() >= 2 {
                result.push(Package {
                    name: parts[0].to_string(),
                    version: parts[1].to_string(),
                    latest_version: None,
                    status: PackageStatus::Installed,
                    size: 0,
                    description: String::new(),
                    repository: Some("flathub".to_string()),
                    installed_by: None,
                });
            }
        }

        Ok(result)
    }

    fn list_snap(&self) -> AppResult<Vec<Package>> {
        let output = Command::new("sh")
            .args(["-c", "snap list 2>/dev/null"])
            .output()?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut result = Vec::new();

        for line in stdout.lines().skip(1) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                result.push(Package {
                    name: parts[0].to_string(),
                    version: parts[1].to_string(),
                    latest_version: None,
                    status: PackageStatus::Installed,
                    size: 0,
                    description: String::new(),
                    repository: None,
                    installed_by: None,
                });
            }
        }

        Ok(result)
    }
}

fn fetch_latest_version_map(pm: &PackageManager) -> AppResult<HashMap<String, String>> {
    if !pm.available {
        return Ok(HashMap::new());
    }
    match pm.name.as_str() {
        "pip" => latest_map_pip(),
        "npm" => latest_map_npm(),
        "bun" => latest_map_bun(),
        "cargo" => latest_map_cargo(),
        "brew" => latest_map_brew(),
        "apt" => latest_map_apt(),
        "pacman" => latest_map_pacman(),
        "aur" => latest_map_aur(&pm.command),
        "rpm" => latest_map_rpm(),
        "flatpak" => latest_map_flatpak(),
        "snap" => latest_map_snap(),
        _ => Ok(HashMap::new()),
    }
}

fn latest_map_pip() -> AppResult<HashMap<String, String>> {
    if pip_uses_arch_pacman_for_global() {
        let mut m = if let Some(aur) = pick_aur_helper_binary() {
            latest_map_from_qu_output(&format!("{aur} -Qu 2>/dev/null"))?
        } else if is_command_available("checkupdates") {
            latest_map_from_qu_output("checkupdates 2>/dev/null")?
        } else {
            latest_map_from_qu_output("pacman -Qu 2>/dev/null")?
        };
        m.retain(|name, _| name.starts_with("python-"));
        return Ok(m);
    }

    let output = run_shell(
        "pip list --outdated --format=json 2>/dev/null || pip3 list --outdated --format=json 2>/dev/null",
    )?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap_or_default();
    let mut m = HashMap::new();
    for v in parsed {
        let name = v["name"].as_str().unwrap_or("").to_ascii_lowercase();
        let latest = v["latest_version"].as_str().unwrap_or("").to_string();
        if !name.is_empty() && !latest.is_empty() {
            m.insert(name, latest);
        }
    }
    Ok(m)
}

fn latest_map_npm() -> AppResult<HashMap<String, String>> {
    let output = run_shell("npm outdated -g --json 2>/dev/null; true")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Ok(HashMap::new());
    }
    let parsed: serde_json::Value = serde_json::from_str(trimmed).unwrap_or_default();
    let Some(obj) = parsed.as_object() else {
        return Ok(HashMap::new());
    };
    let mut m = HashMap::new();
    for (name, info) in obj {
        let Some(info) = info.as_object() else {
            continue;
        };
        let latest = info
            .get("latest")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let current = info
            .get("current")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        if !latest.is_empty() && latest != current {
            m.insert(name.clone(), latest.to_string());
        }
    }
    Ok(m)
}

fn latest_map_bun() -> AppResult<HashMap<String, String>> {
    let output = run_shell("bun outdated -g 2>/dev/null; true")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut m = HashMap::new();
    for line in stdout.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('|') {
            continue;
        }
        if trimmed
            .chars()
            .all(|c| c == '|' || c == '-' || c.is_whitespace())
        {
            continue;
        }
        let cells: Vec<&str> = trimmed
            .split('|')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        if cells.len() < 3 {
            continue;
        }
        let headerish = cells[0].to_lowercase();
        if headerish.contains("package") && headerish.contains("current") {
            continue;
        }
        let name = cells[0];
        let cur = cells[1];
        let latest = cells[2];
        if !name.is_empty() && !latest.is_empty() && latest != cur {
            m.insert(name.to_string(), latest.to_string());
        }
    }
    Ok(m)
}

fn latest_map_cargo() -> AppResult<HashMap<String, String>> {
    if !is_command_available("cargo-install-update") {
        return Ok(HashMap::new());
    }
    let output = run_shell("cargo install-update --list 2>/dev/null; true")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut m = HashMap::new();
    for line in stdout.lines() {
        if let Some(c) = CARGO_INSTALL_UPDATE_LINE.captures(line.trim()) {
            let name = c.get(1).map_or("", |x| x.as_str()).to_string();
            let latest_raw = c.get(3).map_or("", |x| x.as_str());
            if !name.is_empty() && !latest_raw.is_empty() {
                m.insert(name, latest_raw.to_string());
            }
        }
    }
    Ok(m)
}

fn latest_map_brew() -> AppResult<HashMap<String, String>> {
    let output = run_shell("brew outdated --formula --json=v2 2>/dev/null")?;
    if !output.status.success() {
        return Ok(HashMap::new());
    }
    let data: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap_or_default();
    let mut m = HashMap::new();
    if let Some(formulae) = data["formulae"].as_array() {
        for f in formulae {
            let name = f["name"].as_str().unwrap_or("");
            let latest = f["current_version"].as_str().unwrap_or("");
            if !name.is_empty() && !latest.is_empty() {
                m.insert(name.to_string(), latest.to_string());
            }
        }
    }
    Ok(m)
}

fn parse_apt_upgradable_line(line: &str) -> Option<(String, String)> {
    if !line.contains("[upgradable from:") {
        return None;
    }
    let t0 = line.split_whitespace().next()?;
    let name = t0.split('/').next()?.to_string();
    let new_ver = line.split_whitespace().nth(1)?.to_string();
    Some((name, new_ver))
}

fn latest_map_apt() -> AppResult<HashMap<String, String>> {
    let output = run_shell("apt list --upgradable 2>/dev/null")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut m = HashMap::new();
    for line in stdout.lines() {
        if let Some((n, latest)) = parse_apt_upgradable_line(line) {
            m.insert(n, latest);
        }
    }
    Ok(m)
}

fn parse_pacman_qu_line(line: &str) -> Option<(String, String)> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() >= 4 && parts[2] == "->" {
        return Some((parts[0].to_string(), parts[3].to_string()));
    }
    None
}

fn latest_map_from_qu_output(cmd: &str) -> AppResult<HashMap<String, String>> {
    let output = run_shell(cmd)?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut m = HashMap::new();
    for line in stdout.lines() {
        if let Some((n, latest)) = parse_pacman_qu_line(line) {
            m.insert(n, latest);
        }
    }
    Ok(m)
}

fn latest_map_pacman() -> AppResult<HashMap<String, String>> {
    if is_command_available("checkupdates") {
        return latest_map_from_qu_output("checkupdates 2>/dev/null");
    }
    latest_map_from_qu_output("pacman -Qu 2>/dev/null")
}

/// Names of packages explicitly installed from the AUR (`-Qem`), matching [`PackageManager::list_aur`].
fn aur_explicit_foreign_names(cmd: &str) -> AppResult<HashSet<String>> {
    let output = run_shell(&format!("{cmd} -Qem 2>/dev/null"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut names = HashSet::new();
    for line in stdout.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 {
            names.insert(parts[0].to_string());
        }
    }
    Ok(names)
}

fn latest_map_aur(cmd: &str) -> AppResult<HashMap<String, String>> {
    let mut m = latest_map_from_qu_output(&format!("{cmd} -Qu 2>/dev/null"))?;
    let foreign = aur_explicit_foreign_names(cmd)?;
    m.retain(|name, _| foreign.contains(name));
    Ok(m)
}

#[allow(clippy::literal_string_with_formatting_args)]
fn latest_map_rpm() -> AppResult<HashMap<String, String>> {
    if !is_command_available("dnf") {
        return Ok(HashMap::new());
    }
    let output = run_shell("dnf repoquery --upgrades --qf '%{name}\\t%{evr}\\n' 2>/dev/null")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut m = HashMap::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut it = line.splitn(2, '\t');
        let name = it.next().unwrap_or("").trim();
        let evr = it.next().unwrap_or("").trim();
        if name.is_empty() || evr.is_empty() {
            continue;
        }
        m.insert(name.to_string(), evr.to_string());
    }
    Ok(m)
}

fn latest_map_flatpak() -> AppResult<HashMap<String, String>> {
    let output =
        run_shell("flatpak remote-ls --updates --columns=application,version 2>/dev/null")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut m = HashMap::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let lower = line.to_lowercase();
        if lower.contains("application") && lower.contains("version") {
            continue;
        }
        if let Some((id, ver)) = line.split_once('\t') {
            let id = id.trim();
            let ver = ver.trim();
            if !id.is_empty() && !ver.is_empty() {
                m.insert(id.to_string(), ver.to_string());
            }
        }
    }
    Ok(m)
}

fn latest_map_snap() -> AppResult<HashMap<String, String>> {
    let output = run_shell("snap refresh --list 2>/dev/null")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.trim().is_empty()).collect();
    let mut m = HashMap::new();
    if lines.is_empty() {
        return Ok(m);
    }
    let first_line = lines[0].to_lowercase();
    let start = usize::from(first_line.contains("name") && first_line.contains("version"));
    for line in lines.iter().skip(start) {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() >= 2 {
            m.insert(cols[0].to_string(), cols[1].to_string());
        }
    }
    Ok(m)
}

fn is_command_available(cmd: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("command -v {cmd}")])
        .output()
        .is_ok_and(|o| o.status.success())
}

fn run_shell(cmd: &str) -> AppResult<std::process::Output> {
    Ok(Command::new("sh")
        .args(["-c", &format!("timeout 25 {cmd}")])
        .output()?)
}

fn ensure_privileges_ready(pm_name: &str) -> AppResult<()> {
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
        let _ = pm_name;
        Err(AppError::from("Run sudo -v in terminal, then retry."))
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
