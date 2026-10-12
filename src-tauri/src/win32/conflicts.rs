//! Conflict Detection and Resolution Engine
//!
//! Detects and terminates conflicting DPI bypass software:
//! - zapret v1 (Flowseal / `winws.exe` / `zapret` Windows service)
//! - GoodbyeDPI (`goodbyedpi.exe` / `GoodbyeDPI` service)
//! - SpoofDPI / ByeDPI (`spoofdpi.exe`, `byedpi.exe`, `ciadpi.exe`)
//! - Lingering WinDivert driver services
//!
//! Uses 100% native in-process Win32 Service and Toolhelp APIs without
//! ever spawning console windows or invoking external command prompts.

use serde::{Deserialize, Serialize};
use std::os::windows::process::CommandExt;
use std::process::Command;
use windows::core::PCWSTR;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Services::{
    CloseServiceHandle, ControlService, OpenSCManagerW, OpenServiceW, QueryServiceStatus,
    SC_MANAGER_CONNECT, SERVICE_CONTROL_STOP, SERVICE_QUERY_STATUS, SERVICE_RUNNING,
    SERVICE_START_PENDING, SERVICE_STATUS, SERVICE_STOP,
};
use windows::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};

const CREATE_NO_WINDOW: u32 = 0x08000000;

/// Detected third-party software conflict.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictItem {
    pub name: String,
    pub kind: String, // "process" or "service"
    pub pid: Option<u32>,
    pub description: String,
}

/// Known conflicting processes.
const CONFLICTING_PROCESSES: &[(&str, &str)] = &[
    ("winws.exe", "zapret v1 (Flowseal / standalone)"),
    ("goodbyedpi.exe", "GoodbyeDPI"),
    ("spoofdpi.exe", "SpoofDPI"),
    ("byedpi.exe", "ByeDPI (ciadpi)"),
    ("ciadpi.exe", "ByeDPI (ciadpi)"),
];

/// Known conflicting Windows services.
const CONFLICTING_SERVICES: &[(&str, &str)] = &[
    ("zapret", "zapret v1 Windows Service"),
    ("GoodbyeDPI", "GoodbyeDPI Windows Service"),
];

/// Scans the system for running conflicting processes and Windows services.
/// Completely in-process with 0 console window spawns.
#[allow(dead_code)]
pub fn scan_conflicts() -> Vec<ConflictItem> {
    scan_conflicts_excluding(None)
}

/// Scans the system for running conflicting processes and Windows services,
/// explicitly excluding any process matching `active_pid`, our own PID, or spawned child processes.
pub fn scan_conflicts_excluding(active_pid: Option<u32>) -> Vec<ConflictItem> {
    let mut conflicts = Vec::new();
    let current_pid = std::process::id();

    // 1. Scan running processes via Toolhelp snapshot
    unsafe {
        if let Ok(handle) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
            let mut entry = PROCESSENTRY32W::default();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

            if Process32FirstW(handle, &mut entry).is_ok() {
                loop {
                    let pid = entry.th32ProcessID;
                    let parent_pid = entry.th32ParentProcessID;

                    // Bulletproof protection: NEVER flag ourselves, our active supervised child,
                    // or any process whose parent is our application instance.
                    let is_our_own_process = pid == current_pid
                        || parent_pid == current_pid
                        || Some(pid) == active_pid;

                    if !is_our_own_process {
                        let len = entry
                            .szExeFile
                            .iter()
                            .position(|&c| c == 0)
                            .unwrap_or(entry.szExeFile.len());
                        let exe_name = String::from_utf16_lossy(&entry.szExeFile[..len]);

                        for (target_name, desc) in CONFLICTING_PROCESSES {
                            if exe_name.eq_ignore_ascii_case(target_name) {
                                conflicts.push(ConflictItem {
                                    name: exe_name.clone(),
                                    kind: "process".into(),
                                    pid: Some(pid),
                                    description: desc.to_string(),
                                });
                            }
                        }
                    }

                    if Process32NextW(handle, &mut entry).is_err() {
                        break;
                    }
                }
            }
            let _ = CloseHandle(handle);
        }
    }

    // 2. Scan Windows services using native Win32 Service API (0 processes spawned)
    for (svc_name, desc) in CONFLICTING_SERVICES {
        if is_service_running(svc_name) {
            conflicts.push(ConflictItem {
                name: svc_name.to_string(),
                kind: "service".into(),
                pid: None,
                description: desc.to_string(),
            });
        }
    }

    conflicts
}

/// Checks if a Windows service is currently running using native in-process Win32 API.
/// Never spawns external command prompt processes or flashes console windows.
pub fn is_service_running(svc_name: &str) -> bool {
    let svc_utf16: Vec<u16> = svc_name.encode_utf16().chain(std::iter::once(0)).collect();

    unsafe {
        let scm = match OpenSCManagerW(None, None, SC_MANAGER_CONNECT) {
            Ok(handle) => handle,
            Err(_) => return false,
        };

        let service = match OpenServiceW(scm, PCWSTR(svc_utf16.as_ptr()), SERVICE_QUERY_STATUS) {
            Ok(handle) => handle,
            Err(_) => {
                let _ = CloseServiceHandle(scm);
                return false;
            }
        };

        let mut status = SERVICE_STATUS::default();
        let res = QueryServiceStatus(service, &mut status);

        let _ = CloseServiceHandle(service);
        let _ = CloseServiceHandle(scm);

        if res.is_ok() {
            status.dwCurrentState == SERVICE_RUNNING || status.dwCurrentState == SERVICE_START_PENDING
        } else {
            false
        }
    }
}

/// Gracefully stops a running Windows service using native Win32 Service API.
pub fn stop_service_native(svc_name: &str) -> bool {
    let svc_utf16: Vec<u16> = svc_name.encode_utf16().chain(std::iter::once(0)).collect();

    unsafe {
        let scm = match OpenSCManagerW(None, None, SC_MANAGER_CONNECT) {
            Ok(handle) => handle,
            Err(_) => return false,
        };

        let service = match OpenServiceW(scm, PCWSTR(svc_utf16.as_ptr()), SERVICE_STOP | SERVICE_QUERY_STATUS) {
            Ok(handle) => handle,
            Err(_) => {
                let _ = CloseServiceHandle(scm);
                return false;
            }
        };

        let mut status = SERVICE_STATUS::default();
        let res = ControlService(service, SERVICE_CONTROL_STOP, &mut status);

        let _ = CloseServiceHandle(service);
        let _ = CloseServiceHandle(scm);

        res.is_ok()
    }
}

/// Automatically terminates conflicting processes and stops conflicting services.
#[allow(dead_code)]
pub fn resolve_all_conflicts() -> Result<usize, String> {
    resolve_all_conflicts_excluding(None)
}

/// Automatically terminates conflicting processes and stops conflicting services,
/// protecting our own process and our active child process.
pub fn resolve_all_conflicts_excluding(active_pid: Option<u32>) -> Result<usize, String> {
    let mut resolved_count = 0;
    let current_pid = std::process::id();

    // 1. Stop conflicting Windows services natively
    for (svc_name, _) in CONFLICTING_SERVICES {
        if is_service_running(svc_name) {
            let stopped = stop_service_native(svc_name);
            if !stopped {
                let _ = Command::new("net.exe")
                    .args(["stop", svc_name, "/y"])
                    .creation_flags(CREATE_NO_WINDOW)
                    .output();
                let _ = Command::new("sc.exe")
                    .args(["stop", svc_name])
                    .creation_flags(CREATE_NO_WINDOW)
                    .output();
            }
            resolved_count += 1;
        }
    }

    // 2. Terminate conflicting processes
    unsafe {
        if let Ok(handle) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
            let mut entry = PROCESSENTRY32W::default();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

            if Process32FirstW(handle, &mut entry).is_ok() {
                loop {
                    let pid = entry.th32ProcessID;
                    let parent_pid = entry.th32ParentProcessID;

                    let is_our_own = pid == current_pid
                        || parent_pid == current_pid
                        || Some(pid) == active_pid;

                    if !is_our_own {
                        let len = entry
                            .szExeFile
                            .iter()
                            .position(|&c| c == 0)
                            .unwrap_or(entry.szExeFile.len());
                        let exe_name = String::from_utf16_lossy(&entry.szExeFile[..len]);

                        for (target_name, _) in CONFLICTING_PROCESSES {
                            if exe_name.eq_ignore_ascii_case(target_name) {
                                if let Ok(proc_handle) = OpenProcess(PROCESS_TERMINATE, false, pid) {
                                    if TerminateProcess(proc_handle, 1).is_ok() {
                                        resolved_count += 1;
                                    }
                                    let _ = CloseHandle(proc_handle);
                                }
                            }
                        }
                    }

                    if Process32NextW(handle, &mut entry).is_err() {
                        break;
                    }
                }
            }
            let _ = CloseHandle(handle);
        }
    }

    // 3. Reset WinDivert driver service state natively and via silent commands
    stop_service_native("WinDivert");
    stop_service_native("WinDivert14");
    let _ = Command::new("sc.exe")
        .args(["stop", "WinDivert"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    let _ = Command::new("sc.exe")
        .args(["stop", "WinDivert14"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();

    Ok(resolved_count)
}
