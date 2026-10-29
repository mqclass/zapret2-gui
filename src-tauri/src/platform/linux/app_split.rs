//! Per-application split tunneling: process enumeration and settings (Linux).
//!
//! Enumeration is real on Linux too — running processes and their
//! executable paths come from `/proc`. Only ENFORCEMENT is unavailable:
//! the enforcing engine build (winws2-split.exe) is a Windows binary of
//! our fork, so on Linux the selection is stored in settings and honestly
//! never applied (the supervisor reports the split binary as missing and
//! runs the plain engine). See the phase plan, §2.

use serde::{Deserialize, Serialize};
use std::fs;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppInfo {
    pub pid: u32,
    pub name: String,
    pub path: String,
}

/// Enumerates running processes with their executable paths from `/proc`.
/// Returns at most 200 entries, sorted by name.
pub fn list_running_apps() -> Vec<AppInfo> {
    let mut apps = Vec::new();
    let entries = match fs::read_dir("/proc") {
        Ok(e) => e,
        Err(_) => return apps,
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else { continue };
        let Ok(pid) = name_str.parse::<u32>() else { continue };
        // Skip kernel threads / idle (they have no exe anyway) and the
        // Windows-analog system range.
        if pid <= 4 {
            continue;
        }
        let exe = match fs::read_link(entry.path().join("exe")) {
            Ok(p) => p,
            Err(_) => continue, // kernel thread or another user's process
        };
        let path = exe.to_string_lossy().to_string();
        let exe_name = exe
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .trim_end_matches(" (deleted)")
            .to_string();
        if path.is_empty() || exe_name.is_empty() {
            continue;
        }
        apps.push(AppInfo {
            pid,
            name: exe_name,
            path,
        });
        if apps.len() >= 400 {
            break;
        }
    }
    apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    // Deduplicate by path: one exe with N PIDs shows as a single row
    // (selection is per-path anyway).
    let mut seen = std::collections::HashSet::new();
    apps.retain(|app| seen.insert(app.path.clone()));
    apps.truncate(200);
    apps
}

/// Resolves executable paths to current PIDs for the split filter.
/// Returns PIDs of running processes whose path is in the list.
///
/// NOTE: on Linux the comparison is case-SENSITIVE (paths are), unlike the
/// Windows build — see [`normalize_app_path`].
pub fn resolve_pids_for_paths(paths: &[String]) -> Vec<u32> {
    if paths.is_empty() {
        return Vec::new();
    }
    let wanted: std::collections::HashSet<&str> = paths.iter().map(|p| p.as_str()).collect();
    list_running_apps()
        .into_iter()
        .filter(|app| wanted.contains(app.path.as_str()))
        .map(|app| app.pid)
        .collect()
}

/// Normalizes an executable path for storage.
///
/// Deliberately differs from the Windows build: Windows lowercases because
/// its filesystem is case-insensitive; Linux paths are case-sensitive, so
/// lowercasing would corrupt real paths. Only whitespace is trimmed.
pub fn normalize_app_path(path: &str) -> String {
    path.trim().to_string()
}
