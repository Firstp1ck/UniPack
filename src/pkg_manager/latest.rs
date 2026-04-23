//! Backend-specific fetch/parse of "latest-version" maps for upgrade metadata.
#![allow(clippy::missing_docs_in_private_items)]

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;

use crate::AppResult;

use super::util::{
    is_command_available, pick_aur_helper_binary, pip_uses_arch_pacman_for_global, run_shell,
};

/// Regex for lines from `cargo install-update --list` that indicate an available update.
static CARGO_INSTALL_UPDATE_LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(\S+)\s+v(\S+)\s+Yes\s+v(\S+)")
        .expect("static regex for cargo install-update lines should compile")
});

/// Dispatches to the backend-specific "latest version" helper.
pub(super) fn fetch_latest_version_map(
    pm_name: &str,
    pm_command: &str,
    pm_available: bool,
) -> AppResult<HashMap<String, String>> {
    if !pm_available {
        return Ok(HashMap::new());
    }
    match pm_name {
        "pip" => latest_map_pip(),
        "npm" => latest_map_npm(),
        "pnpm" => latest_map_pnpm(),
        "bun" => latest_map_bun(),
        "cargo" => latest_map_cargo(),
        "brew" => latest_map_brew(),
        "apt" => latest_map_apt(),
        "pacman" => latest_map_pacman(),
        "aur" => latest_map_aur(pm_command),
        "rpm" => latest_map_rpm(),
        "flatpak" => latest_map_flatpak(),
        "snap" => latest_map_snap(),
        _ => Ok(HashMap::new()),
    }
}

pub(super) fn latest_map_pip() -> AppResult<HashMap<String, String>> {
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

pub(super) fn parse_pnpm_outdated_json(json: &str) -> HashMap<String, String> {
    let trimmed = json.trim();
    if trimmed.is_empty() {
        return HashMap::new();
    }
    let parsed: serde_json::Value = serde_json::from_str(trimmed).unwrap_or_default();
    let mut m = HashMap::new();
    if let Some(obj) = parsed.as_object() {
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
        return m;
    }
    if let Some(arr) = parsed.as_array() {
        for entry in arr {
            let Some(obj) = entry.as_object() else {
                continue;
            };
            let name = obj
                .get("packageName")
                .or_else(|| obj.get("name"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let latest = obj
                .get("latest")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let current = obj
                .get("current")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            if !name.is_empty() && !latest.is_empty() && latest != current {
                m.insert(name.to_string(), latest.to_string());
            }
        }
    }
    m
}

pub(super) fn latest_map_pnpm() -> AppResult<HashMap<String, String>> {
    let output = run_shell("pnpm outdated -g --format json 2>/dev/null; true")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_pnpm_outdated_json(&stdout))
}

pub(super) fn latest_map_bun() -> AppResult<HashMap<String, String>> {
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

/// Names of packages explicitly installed from the AUR (`-Qem`), matching `list_aur` output.
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

pub(super) fn latest_map_aur(cmd: &str) -> AppResult<HashMap<String, String>> {
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

#[cfg(test)]
mod tests {
    use super::parse_pnpm_outdated_json;

    #[test]
    fn parse_pnpm_outdated_object_shape() {
        let json = r#"{
            "eslint": {"current":"9.1.0","latest":"9.2.0"},
            "typescript": {"current":"5.6.2","latest":"5.6.2"}
        }"#;
        let map = parse_pnpm_outdated_json(json);
        assert_eq!(map.get("eslint"), Some(&"9.2.0".to_string()));
        assert!(!map.contains_key("typescript"));
    }

    #[test]
    fn parse_pnpm_outdated_array_shape() {
        let json = r#"[
            {"packageName":"prettier","current":"3.4.0","latest":"3.5.0"},
            {"name":"vite","current":"6.0.0","latest":"6.0.1"}
        ]"#;
        let map = parse_pnpm_outdated_json(json);
        assert_eq!(map.get("prettier"), Some(&"3.5.0".to_string()));
        assert_eq!(map.get("vite"), Some(&"6.0.1".to_string()));
    }
}
