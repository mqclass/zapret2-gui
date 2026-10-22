//! Per-application split tunneling: process enumeration and settings.
//!
//! Lets the user choose which applications go through the DPI bypass and which
//! go direct. The UI shows running applications; the selection is stored as a
//! list of lowercase executable paths in settings.
//!
//! NOTE: Traffic enforcement (WinDivert pre-filter in the svc) is not yet
//! implemented — this module currently provides enumeration + persistence only.

use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppInfo {
    pub pid: u32,
    pub name: String,
    pub path: String,
}

/// Enumerates running processes with their executable paths.
/// Returns at most 200 entries, sorted by name.
pub fn list_running_apps() -> Vec<AppInfo> {
    let mut apps = Vec::new();
    unsafe {
        let snapshot = match CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
            Ok(h) => h,
            Err(_) => return apps,
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                let pid = entry.th32ProcessID;
                // Skip idle/system processes.
                if pid > 4 {
                    let name = String::from_utf16_lossy(&entry.szExeFile)
                        .trim_matches('\0')
                        .to_string();
                    let path = get_process_path(pid).unwrap_or_default();
                    // Only include processes with a real executable path.
                    if !path.is_empty() && !name.is_empty() {
                        apps.push(AppInfo { pid, name, path });
                    }
                }
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
                if apps.len() >= 200 {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);
    }
    apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    // Deduplicate by path: one exe with N PIDs shows as a single row
    // (selection is per-path anyway).
    let mut seen = std::collections::HashSet::new();
    apps.retain(|app| seen.insert(app.path.to_lowercase()));
    apps
}

fn get_process_path(pid: u32) -> Option<String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        // QueryFullProcessImageNameW works with LIMITED_INFORMATION, unlike GetModuleFileNameExW
        // which needs PROCESS_QUERY_INFORMATION | PROCESS_VM_READ.
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = windows::Win32::System::Threading::QueryFullProcessImageNameW(
            handle,
            windows::Win32::System::Threading::PROCESS_NAME_WIN32,
            windows::core::PWSTR(buf.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(handle);
        if ok.is_err() || len == 0 {
            return None;
        }
        Some(String::from_utf16_lossy(&buf[..len as usize]))
    }
}

/// Resolves executable paths to current PIDs for the WinDivert filter.
/// Returns PIDs of running processes whose path (lowercased) is in the list.
pub fn resolve_pids_for_paths(paths: &[String]) -> Vec<u32> {
    if paths.is_empty() {
        return Vec::new();
    }
    let wanted: std::collections::HashSet<String> =
        paths.iter().map(|p| p.to_lowercase()).collect();
    list_running_apps()
        .into_iter()
        .filter(|app| wanted.contains(&app.path.to_lowercase()))
        .map(|app| app.pid)
        .collect()
}

/// Normalizes an executable path for storage (lowercase).
pub fn normalize_app_path(path: &str) -> String {
    path.trim().to_lowercase()
}
