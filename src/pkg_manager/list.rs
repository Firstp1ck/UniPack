//! Per-backend listing of installed packages (no upgrade-metadata subprocesses).
#![allow(clippy::missing_docs_in_private_items)]

use std::process::Command;

use crate::{AppResult, Package, PackageStatus};

use super::util::pip_uses_arch_pacman_for_global;

/// Dispatches to the backend-specific list helper for `pm_name`.
pub(super) fn list_installed_packages(pm_name: &str) -> AppResult<Vec<Package>> {
    match pm_name {
        "pip" => list_pip(),
        "npm" => list_npm(),
        "pnpm" => list_pnpm(),
        "bun" => list_bun(),
        "cargo" => list_cargo(),
        "brew" => list_brew(),
        "apt" => list_apt(),
        "pacman" => list_pacman(),
        "aur" => list_aur(),
        "rpm" => list_rpm(),
        "flatpak" => list_flatpak(),
        "snap" => list_snap(),
        _ => Ok(Vec::new()),
    }
}

/// Installed Python packages: pacman `python-*` rows on Arch, otherwise `pip list --format=json`.
pub(super) fn list_pip() -> AppResult<Vec<Package>> {
    if pip_uses_arch_pacman_for_global() {
        return list_arch_python_pacman_packages();
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

fn list_npm() -> AppResult<Vec<Package>> {
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

fn list_pnpm() -> AppResult<Vec<Package>> {
    let output = Command::new("sh")
        .args([
            "-c",
            "pnpm list -g --json --depth=0 2>/dev/null || pnpm list -g --json 2>/dev/null",
        ])
        .output()?;

    if !output.status.success() {
        return Ok(Vec::new());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let data: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_default();
    let mut result = Vec::new();

    let mut roots: Vec<&serde_json::Value> = Vec::new();
    if let Some(arr) = data.as_array() {
        roots.extend(arr.iter());
    } else {
        roots.push(&data);
    }
    for root in roots {
        let Some(dependencies) = root
            .get("dependencies")
            .and_then(serde_json::Value::as_object)
        else {
            continue;
        };
        for (name, info) in dependencies {
            let version = info
                .get("version")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_string();
            let description = info
                .get("description")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_string();
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

fn list_bun() -> AppResult<Vec<Package>> {
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

fn list_cargo() -> AppResult<Vec<Package>> {
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

fn list_brew() -> AppResult<Vec<Package>> {
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
fn list_apt() -> AppResult<Vec<Package>> {
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

fn list_pacman() -> AppResult<Vec<Package>> {
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

fn list_aur() -> AppResult<Vec<Package>> {
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

fn list_rpm() -> AppResult<Vec<Package>> {
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

fn list_flatpak() -> AppResult<Vec<Package>> {
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

fn list_snap() -> AppResult<Vec<Package>> {
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
