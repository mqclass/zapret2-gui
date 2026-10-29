//! Conflict Detection and Resolution Engine (Linux)
//!
//! Detects and terminates other DPI-bypass tools that would fight our
//! engine over the same traffic:
//! - zapret v1 (`nfqws`)
//! - ByeDPI (`ciadpi` / `byedpi`)
//! - SpoofDPI (`spoofdpi`)
//!
//! Implemented over `/proc`, the Linux equivalent of the Toolhelp snapshot
//! the Windows build uses. There is no Service Control Manager here, so the
//! service half of the Windows module has no counterpart in phase 1:
//! `is_service_running` / `stop_service_native` answer `false` (systemd
//! unit management arrives with the unit work in step 2), and the scan
//! covers processes only. Our own engine binary (`nfqws2`) is NOT a
//! conflict — it is us; stray copies of it are the supervisor's job
//! (`platform::process::kill_stray_winws2_processes`).

use serde::{Deserialize, Serialize};
use std::fs;

/// Detected third-party software conflict.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictItem {
    pub name: String,
    pub kind: String, // "process" or "service"
    pub pid: Option<u32>,
    pub description: String,
}

/// Known conflicting processes (Linux binary names).
const CONFLICTING_PROCESSES: &[(&str, &str)] = &[
    ("nfqws", "zapret v1 (nfqws)"),
    ("ciadpi", "ByeDPI (ciadpi)"),
    ("byedpi", "ByeDPI"),
    ("spoofdpi", "SpoofDPI"),
    ("goodbyedpi", "GoodbyeDPI"),
];

/// Parent PID of a process from `/proc/<pid>/stat` (0 when unreadable).
/// The comm field may contain spaces/parens, so parsing starts after the
/// LAST ')' — the fields after it are state, ppid, ...
fn parent_pid(pid: u32) -> u32 {
    let stat = match fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(s) => s,
        Err(_) => return 0,
    };
    let after_comm = match stat.rfind(')') {
        Some(i) => &stat[i + 1..],
        None => return 0,
    };
    // Fields after comm: state ppid ...
    after_comm
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// Executable basename of a process ("" when unreadable — kernel thread or
/// another user's process without ptrace permission).
fn exe_basename(pid: u32) -> String {
    let exe = match fs::read_link(format!("/proc/{pid}/exe")) {
        Ok(p) => p,
        Err(_) => return String::new(),
    };
    exe.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .trim_end_matches(" (deleted)")
        .to_string()
}

/// Scans the system for running conflicting processes.
#[allow(dead_code)]
pub fn scan_conflicts() -> Vec<ConflictItem> {
    scan_conflicts_excluding(None)
}

/// Scans the system for running conflicting processes, explicitly
/// excluding any process matching `active_pid`, our own PID, or processes
/// whose parent is our application instance.
pub fn scan_conflicts_excluding(active_pid: Option<u32>) -> Vec<ConflictItem> {
    let mut conflicts = Vec::new();
    let current_pid = std::process::id();

    let entries = match fs::read_dir("/proc") {
        Ok(e) => e,
        Err(_) => return conflicts,
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else { continue };
        let Ok(pid) = name_str.parse::<u32>() else { continue };

        // Bulletproof protection: NEVER flag ourselves, our active
        // supervised child, or any process whose parent is our instance.
        let is_our_own_process = pid == current_pid
            || parent_pid(pid) == current_pid
            || Some(pid) == active_pid;
        if is_our_own_process {
            continue;
        }

        let exe_name = exe_basename(pid);
        if exe_name.is_empty() {
            continue;
        }
        for (target_name, desc) in CONFLICTING_PROCESSES {
            if exe_name == *target_name {
                conflicts.push(ConflictItem {
                    name: exe_name.clone(),
                    kind: "process".into(),
                    pid: Some(pid),
                    description: desc.to_string(),
                });
            }
        }
    }

    conflicts
}

/// Windows-service check. No SCM exists on Linux and phase 1 manages no
/// systemd units, so there is nothing truthful to report but "not running
/// (as a service we can see)". Kept for surface parity with the Windows
/// module, whose callers (driver cleanup) do not exist here.
pub fn is_service_running(_svc_name: &str) -> bool {
    false
}

/// Windows-service stop. See [`is_service_running`]: always false on Linux
/// in phase 1 — nothing was stopped.
pub fn stop_service_native(_svc_name: &str) -> bool {
    false
}

/// Automatically terminates conflicting processes.
#[allow(dead_code)]
pub fn resolve_all_conflicts() -> Result<usize, String> {
    resolve_all_conflicts_excluding(None)
}

/// Automatically terminates conflicting processes, protecting our own
/// process and our active child process.
pub fn resolve_all_conflicts_excluding(active_pid: Option<u32>) -> Result<usize, String> {
    let mut resolved_count = 0;

    for item in scan_conflicts_excluding(active_pid) {
        let Some(pid) = item.pid else { continue };
        // L-6 TOCTOU guard: re-verify the image name from a fresh /proc
        // read — the PID may have been reused since the scan.
        if !crate::platform::process::process_image_matches(pid, &item.name) {
            continue;
        }
        // SIGTERM first (the Windows TerminateProcess has no graceful
        // analog need here: these tools exit on TERM); count only real
        // terminations.
        if unsafe { libc::kill(pid as i32, libc::SIGTERM) } == 0 {
            resolved_count += 1;
        }
    }

    Ok(resolved_count)
}
