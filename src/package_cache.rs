//! On-disk cache of per-backend package lists under the system temp directory.

#![allow(clippy::missing_docs_in_private_items)]

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::model::{AppError, AppResult, Package};
use crate::pkg_manager::PackageManager;

const CACHE_SCHEMA: u32 = 2;
const CACHE_SUBDIR: &str = "unipack";
const CACHE_FILENAME: &str = "package_lists.json";

#[derive(Debug, Serialize, Deserialize)]
struct PackageCacheFile {
    schema_version: u32,
    fingerprint: String,
    entries: Vec<Option<Vec<Package>>>,
}

/// Stable fingerprint of the detected backend list (invalidates cache when tools change).
pub fn managers_fingerprint(managers: &[PackageManager]) -> String {
    managers
        .iter()
        .map(|m| format!("{}:{}:{}", m.name, m.command, m.list_command))
        .collect::<Vec<_>>()
        .join("\n")
}

fn cache_path() -> PathBuf {
    std::env::temp_dir().join(CACHE_SUBDIR).join(CACHE_FILENAME)
}

/// Loads cached package rows when the file exists and matches `managers`.
pub fn load_disk_cache(managers: &[PackageManager]) -> Option<Vec<Option<Vec<Package>>>> {
    let path = cache_path();
    let raw = std::fs::read(&path).ok()?;
    let parsed: PackageCacheFile = serde_json::from_slice(&raw).ok()?;
    if parsed.schema_version != CACHE_SCHEMA {
        return None;
    }
    if parsed.fingerprint != managers_fingerprint(managers) {
        return None;
    }
    if parsed.entries.len() != managers.len() {
        return None;
    }
    Some(parsed.entries)
}

/// Writes the current in-memory lists to disk (best-effort for UI; errors are ignored by callers).
pub fn save_disk_cache(
    managers: &[PackageManager],
    entries: &[Option<Vec<Package>>],
) -> AppResult<()> {
    if entries.len() != managers.len() {
        return Err(AppError::from("package cache length mismatch"));
    }
    let dir = std::env::temp_dir().join(CACHE_SUBDIR);
    std::fs::create_dir_all(&dir)?;
    let file = PackageCacheFile {
        schema_version: CACHE_SCHEMA,
        fingerprint: managers_fingerprint(managers),
        entries: entries.to_vec(),
    };
    let path = cache_path();
    let data = serde_json::to_vec(&file)?;
    std::fs::write(path, data)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_roundtrip_cache_payload() {
        let pm = PackageManager {
            name: "pip".to_string(),
            command: "pip3".to_string(),
            list_command: "pip3".to_string(),
            available: true,
            needs_root: false,
        };
        let managers = [pm];
        let entries = vec![Some(vec![Package {
            name: "x".to_string(),
            version: "1".to_string(),
            latest_version: None,
            status: crate::PackageStatus::Installed,
            size: 0,
            description: String::new(),
            repository: None,
            installed_by: None,
        }])];
        let fp = managers_fingerprint(&managers);
        let file = PackageCacheFile {
            schema_version: CACHE_SCHEMA,
            fingerprint: fp.clone(),
            entries,
        };
        let bytes = serde_json::to_vec(&file).expect("serialize cache");
        let back: PackageCacheFile = serde_json::from_slice(&bytes).expect("deserialize cache");
        assert_eq!(back.fingerprint, fp);
        assert_eq!(back.entries[0].as_ref().expect("row 0")[0].name, "x");
    }
}
