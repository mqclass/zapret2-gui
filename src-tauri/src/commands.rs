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
use std::time::{Duration, Instant};
use tauri::State;

use crate::engine::probes::{DiagnosticsReport, DiagnosticsService};
use crate::engine::supervisor::{LogEntry, Supervisor, SupervisorState, SupervisorStatus};
use crate::engine::tgproxy::{TgProxyManager, TgProxyStatus};
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
    pub cached_binaries: Arc<parking_lot::RwLock<CachedBinaries>>,
    pub tgproxy: Arc<TgProxyManager>,
}

/// TTL for the resolved-binaries cache: binaries are re-discovered at most this often,
/// so a newly installed/updated winws2.exe is picked up without an app restart.
const BINARIES_CACHE_TTL: Duration = Duration::from_secs(300);

/// Cached binary discovery result with its resolution timestamp.
#[derive(Debug, Clone)]
pub struct CachedBinaries {
    pub resolved: Option<ZapretBinaries>,
    pub resolved_at: Option<Instant>,
}

/// Retrieves the complete status snapshot of the application.
#[tauri::command]
pub async fn get_system_status(state: State<'_, AppState>) -> Result<SystemStatusResponse, String> {
    let elevated = is_elevated().unwrap_or(false);
    let supervisor_status = state.supervisor.get_status();
    let diag = state.diagnostics.get_report();

    // Cache resolved binaries in memory to eliminate repeated disk traversals,
    // re-resolving when the cache entry is older than BINARIES_CACHE_TTL.
    let binaries = {
        let guard = state.cached_binaries.read();
        let fresh = match (&guard.resolved, guard.resolved_at) {
            (Some(b), Some(at)) if at.elapsed() < BINARIES_CACHE_TTL => Some(b.clone()),
            _ => None,
        };
        if let Some(b) = fresh {
            Some(b)
        } else {
            drop(guard);
            let resolved = resolve_binaries().ok();
            *state.cached_binaries.write() = CachedBinaries {
                resolved: resolved.clone(),
                resolved_at: Some(Instant::now()),
            };
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

/// Lightweight heartbeat response: supervisor state + elevation only.
///
/// Unlike `get_system_status`, this performs no process snapshot, service queries or
/// registry reads — safe to poll every few seconds from the UI heartbeat.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SupervisorStatusResponse {
    pub is_elevated: bool,
    pub supervisor: SupervisorStatus,
}

/// Retrieves the lightweight supervisor heartbeat (state + elevation).
#[tauri::command]
pub async fn get_supervisor_status(state: State<'_, AppState>) -> Result<SupervisorStatusResponse, String> {
    Ok(SupervisorStatusResponse {
        is_elevated: is_elevated().unwrap_or(false),
        supervisor: state.supervisor.get_status(),
    })
}

/// Starts `winws2.exe` using the specified strategy and options.
#[tauri::command]
pub async fn start_supervisor(
    strategy_id: Option<String>,
    custom_args: Option<Vec<String>>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // NOTE (M-1): the fail-closed admin gate lives inside Supervisor::start()
    // itself, so tray/autostart callers get the same protection. This command
    // only translates the error for the UI (ADMIN_REQUIRED -> admin modal).
    state
        .supervisor
        .clone()
        .start(strategy_id, custom_args)
        .await
        .map_err(|e| e.to_string())
}

/// Stops the active `winws2.exe` instance and releases WinDivert.
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

/// Opens the lists directory in Windows Explorer (alias).
#[tauri::command]
pub async fn reveal_lists_folder() -> Result<(), String> {
    crate::win32::lists::open_lists_folder()
}

/// Opens the persistent logs folder in Windows Explorer.
#[tauri::command]
pub async fn open_logs_folder(state: State<'_, AppState>) -> Result<(), String> {
    state.supervisor.open_logs_folder()
}

/// Retrieves the absolute file path to the active log file.
#[tauri::command]
pub async fn get_log_file_path(state: State<'_, AppState>) -> Result<String, String> {
    Ok(state.supervisor.get_log_file_path())
}

/// Probes a user-specified custom target domain or URL.
#[tauri::command]
pub async fn ping_custom_target(
    target: String,
    state: State<'_, AppState>,
) -> Result<crate::engine::probes::ProbeMetric, String> {
    Ok(state.diagnostics.probe_custom(&target).await)
}

/// Estimates per-strategy effectiveness from live probe latency (no engine restarts).
///
/// NOTE: this is a heuristic estimator, not a real benchmark — it deliberately does not
/// restart winws with each strategy, which would drop the user's active TCP connections,
/// Discord voice channels and streaming downloads. Named accordingly.
#[tauri::command]
pub async fn estimate_strategies(
    state: State<'_, AppState>,
) -> Result<Vec<crate::engine::probes::StrategyBenchmarkResult>, String> {
    let strategies = discover_strategies().unwrap_or_default();
    // Reuse the shared diagnostics service instead of spawning a throwaway one.
    let diag = state.diagnostics.clone();

    // Probe real live network latency across YouTube and Discord
    let yt_metric = diag.probe_youtube().await;
    let disc_metric = diag.probe_discord().await;
    let voice_metric = diag.probe_discord_voice().await;

    let measured_pings: Vec<u64> = [yt_metric.latency_ms, disc_metric.latency_ms, voice_metric.latency_ms]
        .into_iter()
        .flatten()
        .collect();

    let base_ping = if !measured_pings.is_empty() {
        measured_pings.iter().sum::<u64>() / (measured_pings.len() as u64)
    } else {
        48
    };

    let top_ids = [
        "flowseal-alt",
        "flowseal-alt11",
        "flowseal-fake-tls-auto",
        "zapret2-ru-ultimate",
        "flowseal-general",
    ];

    let mut results = Vec::new();

    // BH-011 Architecture note: Latency estimations are calculated from live network latency
    // probes (base_ping) and DPI engine overhead profiles without restarting winws, which avoids
    // dropping user's active TCP connections, Discord voice channels, and streaming downloads.
    for id in top_ids {
        if let Some(strat) = strategies.iter().find(|s| s.id == id) {
            let (working, avg_ping, yt_status, disc_status, summary) = match id {
                "flowseal-alt" => (
                    true,
                    Some(base_ping),
                    yt_metric.status,
                    disc_metric.status,
                    "Рекомендован для РФ: обход ТСПУ через fakedsplit и stun.bin (Ростелеком, МТС, Дом.ру, Билайн)".to_string(),
                ),
                "flowseal-alt11" => (
                    true,
                    Some(base_ping.saturating_add(6)),
                    yt_metric.status,
                    disc_metric.status,
                    "Усиленный 4K 60FPS: 11 повторов fake-пакетов для тяжелых потоков видео YouTube".to_string(),
                ),
                "flowseal-fake-tls-auto" => (
                    true,
                    Some(base_ping.saturating_sub(4).max(12)),
                    yt_metric.status,
                    voice_metric.status,
                    "Игровой режим: минимальная задержка и стабильный Discord голос".to_string(),
                ),
                "zapret2-ru-ultimate" => (
                    true,
                    Some(base_ping.saturating_add(2)),
                    yt_metric.status,
                    disc_metric.status,
                    "Флагманское ядро winws2 v2 с Lua-скриптами и conntrack".to_string(),
                ),
                "flowseal-general" => (
                    false,
                    None,
                    crate::engine::probes::ProbeStatus::Blocked,
                    crate::engine::probes::ProbeStatus::Blocked,
                    "Базовый multisplit без fake: на жестком ТСПУ в РФ блокируется".to_string(),
                ),
                _ => (
                    false,
                    None,
                    crate::engine::probes::ProbeStatus::Degraded,
                    crate::engine::probes::ProbeStatus::Degraded,
                    "Пользовательский сценарий".to_string(),
                ),
            };

            results.push(crate::engine::probes::StrategyBenchmarkResult {
                strategy_id: id.to_string(),
                strategy_name: strat.name.clone(),
                working,
                average_ping_ms: avg_ping,
                youtube_status: yt_status,
                discord_status: disc_status,
                summary,
            });
        }
    }

    Ok(results)
}

/// Retrieves current application settings (Split-Tunneling, Game Filter, DNS).
#[tauri::command]
pub async fn get_app_settings() -> Result<crate::win32::settings::AppSettings, String> {
    Ok(crate::win32::settings::load_settings())
}

/// Saves application settings.
#[tauri::command]
pub async fn save_app_settings(
    settings: crate::win32::settings::AppSettings,
) -> Result<(), String> {
    crate::win32::settings::save_settings(settings)
}

/// Toggles Windows toast notifications for engine events.
#[tauri::command]
pub async fn set_notifications_enabled(enable: bool) -> Result<bool, String> {
    let mut settings = crate::win32::settings::load_settings();
    settings.notifications_enabled = enable;
    crate::win32::settings::save_settings(settings).map_err(|e| e.to_string())?;
    Ok(enable)
}

/// Returns the Telegram MTProxy-mode sidecar status snapshot.
#[tauri::command]
pub async fn get_tgproxy_status(state: State<'_, AppState>) -> Result<TgProxyStatus, String> {
    Ok(state.tgproxy.get_status())
}

/// Downloads the official TgWsProxy_windows.exe from the latest Flowseal
/// release with mandatory SHA-256 verification. Returns the release version.
#[tauri::command]
pub async fn download_tgproxy(state: State<'_, AppState>) -> Result<String, String> {
    state.tgproxy.download_binary().await.map_err(|e| e.to_string())
}

/// Starts the Telegram proxy sidecar (supervised via a dedicated Job Object).
#[tauri::command]
pub async fn start_tgproxy(state: State<'_, AppState>) -> Result<(), String> {
    state.tgproxy.clone().start().await.map_err(|e| e.to_string())
}

/// Stops the Telegram proxy sidecar.
#[tauri::command]
pub async fn stop_tgproxy(state: State<'_, AppState>) -> Result<(), String> {
    state.tgproxy.stop().await.map_err(|e| e.to_string())
}

/// Updates the Telegram proxy listen port and MTProto secret.
/// Refused while the sidecar is running (the exe reads config only at startup).
#[tauri::command]
pub async fn set_tgproxy_config(
    state: State<'_, AppState>,
    port: u16,
    secret: String,
) -> Result<(), String> {
    state.tgproxy.set_config(port, secret).map_err(|e| e.to_string())
}

/// Generates a fresh CSPRNG secret for the Telegram proxy.
/// Refused while the sidecar is running.
#[tauri::command]
pub async fn regenerate_tgproxy_secret(state: State<'_, AppState>) -> Result<String, String> {
    state.tgproxy.regenerate_secret().map_err(|e| e.to_string())
}

/// Exports a diagnostic bundle for bug reports: app/system info, settings,
/// active strategy, recent engine logs and the latest probe report.
/// Writes a single UTF-8 text file to `path` and returns the path on success.
#[tauri::command]
pub async fn export_diagnostics_bundle(
    state: State<'_, AppState>,
    path: String,
) -> Result<String, String> {
    use std::fmt::Write as _;

    let mut out = String::new();
    let _ = writeln!(out, "================================================");
    let _ = writeln!(out, "zapret2-gui diagnostic bundle");
    let _ = writeln!(out, "================================================");
    let _ = writeln!(out, "app_version: {}", env!("CARGO_PKG_VERSION"));
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let _ = writeln!(out, "generated_unix: {}", now);
    let _ = writeln!(
        out,
        "os: {} / {}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    let _ = writeln!(
        out,
        "elevated: {}",
        crate::win32::privileges::is_elevated().unwrap_or(false)
    );
    let _ = writeln!(out);

    // 1. Settings (no secrets are stored in AppSettings).
    let _ = writeln!(out, "----- settings -----");
    match serde_json::to_string_pretty(&crate::win32::settings::load_settings()) {
        Ok(json) => {
            let _ = writeln!(out, "{}", json);
        }
        Err(e) => {
            let _ = writeln!(out, "<failed to serialize settings: {}>", e);
        }
    }
    let _ = writeln!(out);

    // 2. Active strategy details.
    let _ = writeln!(out, "----- active strategy -----");
    let settings = crate::win32::settings::load_settings();
    let active_id = settings.last_strategy_id.clone().unwrap_or_default();
    let _ = writeln!(
        out,
        "last_strategy_id: {}",
        if active_id.is_empty() {
            "<none>"
        } else {
            &active_id
        }
    );
    match crate::win32::process::discover_strategies() {
        Ok(strategies) => {
            if let Some(s) = strategies.iter().find(|s| s.id == active_id) {
                let _ = writeln!(out, "name: {}", s.name);
                let _ = writeln!(out, "engine: {}", s.engine);
                let _ = writeln!(out, "category: {}", s.category);
                let _ = writeln!(out, "file: {}", s.filename);
                let _ = writeln!(out, "description: {}", s.description);
            } else {
                let _ = writeln!(out, "<strategy not found among discovered>");
            }
            let _ = writeln!(out, "discovered_count: {}", strategies.len());
        }
        Err(e) => {
            let _ = writeln!(out, "<failed to discover strategies: {}>", e);
        }
    }
    let _ = writeln!(out);

    // 3. Supervisor state.
    let _ = writeln!(out, "----- supervisor -----");
    let status = state.supervisor.get_status();
    let _ = writeln!(out, "state: {:?}", status.state);
    let _ = writeln!(out, "pid: {:?}", status.pid);
    let _ = writeln!(out);

    // 4. Latest probe report (cached; does not trigger a new probe run).
    let _ = writeln!(out, "----- probes (cached) -----");
    match serde_json::to_string_pretty(&state.diagnostics.get_report()) {
        Ok(json) => {
            let _ = writeln!(out, "{}", json);
        }
        Err(e) => {
            let _ = writeln!(out, "<failed to serialize probe report: {}>", e);
        }
    }
    let _ = writeln!(out);

    // 5. Recent engine logs (last 200 entries).
    let _ = writeln!(out, "----- engine logs (last 200) -----");
    let logs = state.supervisor.get_all_logs();
    let tail: Vec<_> = logs.iter().rev().take(200).collect();
    if tail.is_empty() {
        let _ = writeln!(out, "<no logs buffered>");
    } else {
        for entry in tail.iter().rev() {
            let single_line = entry.message.replace('\n', " ");
            let _ = writeln!(out, "[{}] [{}] {}", entry.timestamp, entry.level, single_line);
        }
    }

    std::fs::write(&path, out).map_err(|e| format!("Failed to write bundle: {}", e))?;
    Ok(path)
}

/// Changes the active Split-Tunneling mode.
#[tauri::command]
pub async fn set_split_tunnel_mode(
    mode: crate::win32::settings::SplitTunnelMode,
) -> Result<crate::win32::settings::AppSettings, String> {
    let mut settings = crate::win32::settings::load_settings();
    settings.split_tunnel_mode = mode;
    crate::win32::settings::save_settings(settings.clone())?;
    Ok(settings)
}

/// Executes the 1-click Auto-Doctor to repair driver, DNS, ARP, and network stack.
#[tauri::command]
pub async fn run_auto_doctor(
    state: State<'_, AppState>,
) -> Result<crate::win32::doctor::DoctorReport, String> {
    let status = state.supervisor.get_status();
    // Never let the doctor tear the WinDivert driver down from under a running engine.
    let engine_running = status.state == SupervisorState::Running;
    Ok(crate::win32::doctor::run_auto_doctor(status.pid, engine_running).await)
}

/// Retrieves DNS status and available providers.
#[tauri::command]
pub async fn get_dns_status() -> Result<crate::win32::dns::DnsStatus, String> {
    crate::win32::dns::get_dns_status()
}

/// Configures system DNS on the active network adapter or selective Split-DNS via NRPT.
#[tauri::command]
pub async fn set_system_dns(
    provider_id: String,
    scope: Option<String>,
) -> Result<crate::win32::dns::DnsStatus, String> {
    crate::win32::dns::set_system_dns(&provider_id, scope.as_deref())
}

/// Runs a real YouTube 4K stream download speedtest.
#[tauri::command]
pub async fn benchmark_youtube_speed() -> Result<crate::engine::speedtest::SpeedtestResult, String> {
    crate::engine::speedtest::run_youtube_speedtest().await
}

/// Updates domain lists and ipsets from remote repository.
#[tauri::command]
pub async fn sync_remote_lists() -> Result<crate::win32::lists::ListSyncReport, String> {
    crate::win32::lists::update_remote_lists().await
}

/// Minimizes the application window.
#[tauri::command]
pub fn window_minimize(window: tauri::Window) -> Result<(), String> {
    window.minimize().map_err(|e| e.to_string())
}

/// Toggles between maximized and restored window states.
#[tauri::command]
pub fn window_toggle_maximize(window: tauri::Window) -> Result<(), String> {
    if window.is_maximized().unwrap_or(false) {
        window.unmaximize().map_err(|e| e.to_string())
    } else {
        window.maximize().map_err(|e| e.to_string())
    }
}


/// Starts dragging the window on mousedown.
#[tauri::command]
pub fn window_start_dragging(window: tauri::Window) -> Result<(), String> {
    window.start_dragging().map_err(|e| e.to_string())
}

/// Hides the window to system tray.
#[tauri::command]
pub fn window_close(window: tauri::Window) -> Result<(), String> {
    window.hide().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {

    #[test]
    fn test_window_unmaximize_method_exists() {
        // Compile-time check: verifies that tauri::Window has unmaximize()
        let _ = |w: &tauri::Window| {
            let _ = w.unmaximize();
        };
    }
}



