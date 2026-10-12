//! Tauri IPC Command Handlers
//!
//! Provides type-safe frontend-to-backend API endpoints for:
//! - Process life-cycle management (start, stop, status)
//! - Discovered Lua strategy listing
//! - Network diagnostics probes (YouTube, Discord)
//! - High-throughput micro-batched log retrieval and buffer reset
//! - Privilege verification and UAC self-elevation dispatch
//! - Third-party DPI software conflict detection and resolution

use std::sync::Arc;
use tauri::State;

use crate::engine::probes::{DiagnosticsReport, DiagnosticsService};
use crate::engine::supervisor::{LogEntry, Supervisor, SupervisorStatus};
use crate::win32::conflicts::{
    resolve_all_conflicts_excluding as win32_resolve_all_conflicts,
    scan_conflicts_excluding as win32_scan_conflicts, ConflictItem,
};
use crate::win32::privileges::{is_elevated, relaunch_elevated as win32_relaunch_elevated};
use crate::win32::process::{
    discover_strategies, kill_stray_winws2_processes, resolve_binaries, LuaStrategy, ZapretBinaries,
};

/// Combined system status response for frontend initial render and heartbeat.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SystemStatusResponse {
    pub is_elevated: bool,
    pub supervisor: SupervisorStatus,
    pub diagnostics: DiagnosticsReport,
    pub binaries: Option<ZapretBinaries>,
    pub conflicts: Vec<ConflictItem>,
    pub autostart_enabled: bool,
}

/// Shared application state managed by Tauri runtime.
pub struct AppState {
    pub supervisor: Arc<Supervisor>,
    pub diagnostics: Arc<DiagnosticsService>,
    pub cached_binaries: Arc<parking_lot::RwLock<Option<ZapretBinaries>>>,
}

/// Retrieves the complete status snapshot of the application.
#[tauri::command]
pub async fn get_system_status(state: State<'_, AppState>) -> Result<SystemStatusResponse, String> {
    let elevated = is_elevated().unwrap_or(false);
    let supervisor_status = state.supervisor.get_status();
    let diag = state.diagnostics.get_report();

    // Cache resolved binaries in memory to eliminate repeated disk traversals
    let binaries = {
        let read_guard = state.cached_binaries.read();
        if read_guard.is_some() {
            read_guard.clone()
        } else {
            drop(read_guard);
            let resolved = resolve_binaries().ok();
            *state.cached_binaries.write() = resolved.clone();
            resolved
        }
    };

    let conflicts = win32_scan_conflicts(supervisor_status.pid);
    let autostart_enabled = crate::win32::autostart::is_autostart_enabled();

    Ok(SystemStatusResponse {
        is_elevated: elevated,
        supervisor: supervisor_status,
        diagnostics: diag,
        binaries,
        conflicts,
        autostart_enabled,
    })
}

/// Checks whether autostart on Windows logon is enabled.
#[tauri::command]
pub async fn get_autostart_status() -> Result<bool, String> {
    Ok(crate::win32::autostart::is_autostart_enabled())
}

/// Enables or disables autostart on Windows logon.
#[tauri::command]
pub async fn set_autostart_status(enable: bool) -> Result<bool, String> {
    crate::win32::autostart::set_autostart(enable)
}

/// Discovers and lists all Lua DPI bypass strategies available on the host.
#[tauri::command]
pub async fn list_strategies() -> Result<Vec<LuaStrategy>, String> {
    discover_strategies().map_err(|e| e.to_string())
}

/// Starts `winws2.exe` using the specified strategy and options.
#[tauri::command]
pub async fn start_supervisor(
    strategy_id: Option<String>,
    custom_args: Option<Vec<String>>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .supervisor
        .clone()
        .start(strategy_id, custom_args)
        .await
        .map_err(|e| e.to_string())
}

/// Gracefully stops the active `winws2.exe` instance and releases WinDivert.
#[tauri::command]
pub async fn stop_supervisor(state: State<'_, AppState>) -> Result<(), String> {
    state.supervisor.stop().await.map_err(|e| e.to_string())
}

/// Retrieves the latest network probe diagnostics (YouTube TLS 1.3 & Discord).
#[tauri::command]
pub async fn get_probe_metrics(state: State<'_, AppState>) -> Result<DiagnosticsReport, String> {
    let report = state.diagnostics.get_report();
    if report.youtube.status == crate::engine::probes::ProbeStatus::Checking {
        Ok(state.diagnostics.run_probes_now().await)
    } else {
        Ok(report)
    }
}

/// Forces an immediate network probe diagnostics run.
#[tauri::command]
pub async fn refresh_probes(state: State<'_, AppState>) -> Result<DiagnosticsReport, String> {
    Ok(state.diagnostics.run_probes_now().await)
}

/// Retrieves all buffered logs currently in the Ring Buffer.
#[tauri::command]
pub async fn get_logs(state: State<'_, AppState>) -> Result<Vec<LogEntry>, String> {
    Ok(state.supervisor.get_all_logs())
}

/// Clears the log ring buffer.
#[tauri::command]
pub async fn clear_logs(state: State<'_, AppState>) -> Result<(), String> {
    state.supervisor.clear_logs();
    Ok(())
}

/// Relaunches the application with UAC Administrator privileges.
#[tauri::command]
pub async fn relaunch_elevated() -> Result<(), String> {
    win32_relaunch_elevated().map_err(|e| e.to_string())
}

/// Finds and forcefully terminates any orphaned `winws2.exe` instances left running,
/// protecting the actively running process from accidental termination.
#[tauri::command]
pub async fn kill_stray_processes(state: State<'_, AppState>) -> Result<usize, String> {
    let active_pid = state.supervisor.get_status().pid;
    let count = kill_stray_winws2_processes(active_pid);
    Ok(count)
}

/// Scans for conflicting DPI bypass processes and Windows services.
#[tauri::command]
pub async fn scan_conflicts(state: State<'_, AppState>) -> Result<Vec<ConflictItem>, String> {
    let pid = state.supervisor.get_status().pid;
    Ok(win32_scan_conflicts(pid))
}

/// Terminates conflicting DPI bypass processes and stops conflicting Windows services.
#[tauri::command]
pub async fn resolve_conflicts(state: State<'_, AppState>) -> Result<usize, String> {
    let pid = state.supervisor.get_status().pid;
    win32_resolve_all_conflicts(pid)
}

/// Saves or updates a custom user profile and returns the refreshed strategy list.
#[tauri::command]
pub async fn save_custom_profile(strategy: LuaStrategy) -> Result<Vec<LuaStrategy>, String> {
    crate::win32::custom_profiles::save_custom_profile(strategy)?;
    discover_strategies().map_err(|e| e.to_string())
}

/// Deletes a custom user profile by ID and returns the refreshed strategy list.
#[tauri::command]
pub async fn delete_custom_profile(id: String) -> Result<Vec<LuaStrategy>, String> {
    crate::win32::custom_profiles::delete_custom_profile(&id)?;
    discover_strategies().map_err(|e| e.to_string())
}

/// Parses a command line string into separate arguments with variable expansion.
#[tauri::command]
pub async fn parse_args_string(raw: String) -> Result<Vec<String>, String> {
    let resolved = resolve_binaries().map_err(|e| e.to_string())?;
    let lists_dir = resolved.lists_dir.unwrap_or_else(|| resolved.bin_dir.join("../lists"));
    Ok(crate::win32::custom_profiles::parse_command_line_args(&raw, &resolved.bin_dir, &lists_dir))
}

/// Exits the application cleanly, stopping the background supervisor and WinDivert first.
#[tauri::command]
pub async fn exit_app(state: State<'_, AppState>, app: tauri::AppHandle) -> Result<(), String> {
    let _ = state.supervisor.stop().await;
    app.exit(0);
    Ok(())
}

/// Returns all available domain/hostlist files in the lists directory.
#[tauri::command]
pub async fn list_domain_files() -> Result<Vec<String>, String> {
    crate::win32::lists::list_domain_files()
}

/// Reads domain entries from a specific file in the lists directory.
#[tauri::command]
pub async fn read_domain_list(filename: String) -> Result<Vec<String>, String> {
    crate::win32::lists::read_domains(&filename)
}

/// Adds a domain to a specific list file and returns the updated domain list.
#[tauri::command]
pub async fn add_domain_to_list(filename: String, domain: String) -> Result<Vec<String>, String> {
    crate::win32::lists::add_domain_to_list(&filename, &domain)
}

/// Removes a domain from a specific list file and returns the updated domain list.
#[tauri::command]
pub async fn remove_domain_from_list(filename: String, domain: String) -> Result<Vec<String>, String> {
    crate::win32::lists::remove_domain_from_list(&filename, &domain)
}

/// Flushes Windows DNS client cache via ipconfig /flushdns.
#[tauri::command]
pub async fn flush_dns_cache() -> Result<String, String> {
    crate::win32::lists::flush_dns_cache()
}

/// Opens the lists directory in Windows Explorer.
#[tauri::command]
pub async fn open_lists_folder() -> Result<(), String> {
    crate::win32::lists::open_lists_folder()
}
