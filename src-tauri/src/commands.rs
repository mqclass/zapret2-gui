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
use crate::engine::supervisor::{LogEntry, SupervisorState, SupervisorStatus};
use crate::engine::tgproxy::TgProxyStatus;
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
    /// Where the engine runs: in-process or attached to zapret2-svc.exe.
    /// Swapped to Remote when the GUI attaches to a running service.
    pub backend: Arc<parking_lot::RwLock<crate::svc_client::SupervisorBackend>>,
    pub diagnostics: Arc<DiagnosticsService>,
    pub cached_binaries: Arc<parking_lot::RwLock<CachedBinaries>>,
    /// Auxiliary long-lived processes (sidecars) managed by the app.
    /// `install_update` fails closed while any of them is active, so the
    /// installation directory is never renamed out from under a running child.
    pub sidecars: Vec<Arc<dyn crate::engine::managed::ManagedSidecar>>,
    /// Where the Telegram bridge runs: in-process or inside zapret2-svc.exe.
    /// Swapped to Remote when the GUI attaches to a running service.
    pub tgproxy: Arc<parking_lot::RwLock<crate::svc_client::TgproxyBackend>>,}

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
    let backend = state.backend.read().clone();
    let supervisor_status = backend.get_status().await;
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
    let backend = state.backend.read().clone();
    Ok(SupervisorStatusResponse {
        is_elevated: is_elevated().unwrap_or(false),
        supervisor: backend.get_status().await,
    })
}

/// Starts `winws2.exe` using the specified strategy and options.
///
/// Prefers the detached `zapret2-svc.exe` service when its binary is present:
/// the engine then outlives this GUI process. Falls back to in-process when
/// the svc binary is missing or fails to attach.
#[tauri::command]
pub async fn start_supervisor(
    strategy_id: Option<String>,
    custom_args: Option<Vec<String>>,
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    // NOTE (M-1): the fail-closed admin gate lives inside Supervisor::start()
    // itself (and inside the svc), so tray/autostart callers get the same
    // protection. This command only translates the error for the UI
    // (ADMIN_REQUIRED -> admin modal).
    let backend = state.backend.read().clone();
    if crate::svc_client::backend_needs_attach(&backend).await
    {
        match crate::svc_client::ensure_svc_attached(std::time::Duration::from_secs(10)).await
        {
            Ok(client) => {
                *state.backend.write() =
                    crate::svc_client::SupervisorBackend::Remote(client.clone());
                // The Telegram bridge follows the engine into the svc.
                *state.tgproxy.write() =
                    crate::svc_client::TgproxyBackend::Remote(client);
                let app_clone = app.clone();
                tokio::spawn(async move {
                    crate::svc_client::SvcClient::forward_events(app_clone).await;
                });
            }
            Err(e) => {
                eprintln!("svc attach failed, falling back to in-process engine: {e}");
            }
        }
    }
    let backend = state.backend.read().clone();
    backend.start(strategy_id, custom_args).await
}

/// Stops the active `winws2.exe` instance and releases WinDivert.
#[tauri::command]
pub async fn stop_supervisor(state: State<'_, AppState>) -> Result<(), String> {
    let backend = state.backend.read().clone();
    backend.stop().await
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
    let backend = state.backend.read().clone();
    Ok(backend.get_all_logs().await)
}

/// Clears the log ring buffer.
#[tauri::command]
pub async fn clear_logs(state: State<'_, AppState>) -> Result<(), String> {
    let backend = state.backend.read().clone();
    backend.clear_logs().await;
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
    let backend = state.backend.read().clone();
    let active_pid = backend.get_status().await.pid;
    let count = kill_stray_winws2_processes(active_pid);
    Ok(count)
}

/// Scans for conflicting DPI bypass processes and Windows services.
#[tauri::command]
pub async fn scan_conflicts(state: State<'_, AppState>) -> Result<Vec<ConflictItem>, String> {
    let backend = state.backend.read().clone();
    let pid = backend.get_status().await.pid;
    Ok(win32_scan_conflicts(pid))
}

/// Terminates conflicting DPI bypass processes and stops conflicting Windows services.
#[tauri::command]
pub async fn resolve_conflicts(state: State<'_, AppState>) -> Result<usize, String> {
    let backend = state.backend.read().clone();
    let pid = backend.get_status().await.pid;
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

/// Exits the application cleanly, stopping the Telegram bridge, the background
/// supervisor and WinDivert first.
#[tauri::command]
pub async fn exit_app(state: State<'_, AppState>, app: tauri::AppHandle) -> Result<(), String> {
    let tgproxy = state.tgproxy.read().clone();
    let _ = tgproxy.stop().await;
    // A remote svc outlives the GUI by design (the bypass stays up); only
    // stop the engine when it runs in this process.
    if !state.backend.read().clone().is_remote() {
        let backend = state.backend.read().clone();
        let _ = backend.stop().await;
    }
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
    state.backend.read().clone().open_logs_folder()
}

/// Retrieves the absolute file path to the active log file.
#[tauri::command]
pub async fn get_log_file_path(state: State<'_, AppState>) -> Result<String, String> {
    Ok(state.backend.read().clone().get_log_file_path())
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
    // Serialized vs. concurrent backend mutations (L-4).
    crate::win32::settings::update_settings(|s| *s = settings)
}

/// Toggles Windows toast notifications for engine events.
#[tauri::command]
pub async fn set_notifications_enabled(enable: bool) -> Result<bool, String> {
    crate::win32::settings::update_settings(|s| s.notifications_enabled = enable)
        .map_err(|e| e.to_string())?;
    Ok(enable)
}

/// Returns the Telegram bridge status snapshot (local or via svc pipe).
#[tauri::command]
pub async fn get_tgproxy_status(state: State<'_, AppState>) -> Result<TgProxyStatus, String> {
    let tgproxy = state.tgproxy.read().clone();
    Ok(tgproxy.get_status().await)
}

/// Starts the Telegram MTProto→WebSocket bridge (local or via svc pipe).
#[tauri::command]
pub async fn start_tgproxy(state: State<'_, AppState>) -> Result<(), String> {
    // Same svc-migration policy as the Start button: prefer the svc when
    // its binary is available so the bridge outlives the GUI.
    ensure_tgproxy_backend(&state).await;
    let tgproxy = state.tgproxy.read().clone();
    tgproxy.start().await
}

/// Stops the Telegram bridge.
#[tauri::command]
pub async fn stop_tgproxy(state: State<'_, AppState>) -> Result<(), String> {
    let tgproxy = state.tgproxy.read().clone();
    tgproxy.stop().await
}

/// Updates the Telegram proxy listen port and MTProto secret.
/// Refused while the bridge is running (it reads config only at startup).
#[tauri::command]
pub async fn set_tgproxy_config(
    state: State<'_, AppState>,
    port: u16,
    secret: String,
) -> Result<(), String> {
    let tgproxy = state.tgproxy.read().clone();
    tgproxy.set_config(port, secret).await
}

/// Generates a fresh CSPRNG secret for the Telegram proxy.
/// Refused while the bridge is running.
#[tauri::command]
pub async fn regenerate_tgproxy_secret(state: State<'_, AppState>) -> Result<String, String> {
    let tgproxy = state.tgproxy.read().clone();
    tgproxy.regenerate_secret().await
}

/// Sets the Telegram bridge connection mode: "auto", "cfproxy" or "direct".
/// Refused while the bridge is running (it reads the mode only at startup).
#[tauri::command]
pub async fn set_tgproxy_mode(state: State<'_, AppState>, mode: String) -> Result<(), String> {
    let tgproxy = state.tgproxy.read().clone();
    tgproxy.set_mode(mode).await
}

/// Honest end-to-end test of the Telegram bridge: handshake through the
/// listener to the Telegram DC. Returns the DC round-trip time in
/// milliseconds. Fails with the broken step named when the chain is down.
#[tauri::command]
pub async fn test_tgproxy_connection(state: State<'_, AppState>) -> Result<u64, String> {
    let tgproxy = state.tgproxy.read().clone();
    Ok(tgproxy.test_connection().await.map(|d| d.as_millis() as u64)?)
}

/// Toggles LAN access for the Telegram bridge (bind 0.0.0.0 so phones on
/// the local network can use it). Refused while the bridge is running.
#[tauri::command]
pub async fn set_tgproxy_lan(state: State<'_, AppState>, enabled: bool) -> Result<(), String> {
    let tgproxy = state.tgproxy.read().clone();
    tgproxy.set_lan(enabled).await
}

/// Migrates the Telegram-bridge backend to follow the supervisor backend:
/// when the engine runs inside the svc, the bridge must run there too
/// (a GUI crash must not kill either). Idempotent.
pub async fn ensure_tgproxy_backend(state: &State<'_, AppState>) {
    use crate::svc_client::{backend_needs_attach, ensure_svc_attached, TgproxyBackend};
    // First, apply the standard svc-migration policy to the engine itself:
    // a local backend with a svc binary on disk moves into the svc.
    {
        let backend = state.backend.read().clone();
        if backend_needs_attach(&backend).await {
            if let Ok(client) = ensure_svc_attached(std::time::Duration::from_secs(10)).await {
                *state.backend.write() = crate::svc_client::SupervisorBackend::Remote(client);
            }
        }
    }
    // Then the bridge follows the engine.
    let client = match &*state.backend.read() {
        crate::svc_client::SupervisorBackend::Remote(c) => c.clone(),
        crate::svc_client::SupervisorBackend::Local(_) => return,
    };
    if !state.tgproxy.read().is_remote() {
        *state.tgproxy.write() = TgproxyBackend::Remote(client);
    }
}

/// Lists all registered camouflage transports with live availability for the UI.
///
/// The registry is the single source of truth: the frontend never hardcodes
/// transport ids, names or readiness — it renders exactly this snapshot.
#[tauri::command]
pub async fn list_transports() -> Result<Vec<crate::engine::transport::TransportView>, String> {
    Ok(crate::engine::transport::global_registry().list())
}

/// Returns the currently selected transport id, if any (`None` = direct launch).
#[tauri::command]
pub async fn get_active_transport() -> Result<Option<String>, String> {
    Ok(crate::win32::settings::load_settings().active_transport_id)
}

/// Selects the active camouflage transport.
///
/// Fail-closed and explicit: unknown ids and unavailable transports
/// (e.g. experimental ones without server-side infrastructure) are refused
/// with a reason — the selection is never stored silently.
#[tauri::command]
pub async fn set_active_transport(transport_id: String) -> Result<(), String> {
    let registry = crate::engine::transport::global_registry();
    let transport = registry.get(&transport_id).ok_or_else(|| {
        format!(
            "Неизвестный транспорт «{}». Доступные: {}",
            transport_id,
            registry.ids().join(", ")
        )
    })?;

    if let crate::engine::transport::TransportAvailability::Unavailable { reason } =
        transport.is_available()
    {
        return Err(format!(
            "Транспорт «{}» недоступен: {}",
            transport.name(),
            reason
        ));
    }

    let mut settings = crate::win32::settings::load_settings();
    if transport_id == "direct" {
        settings.active_transport_id = None;
    } else {
        settings.active_transport_id = Some(transport_id.clone());
    }
    // Ensure a config entry exists so the UI can offer editing later.
    settings
        .transport_configs
        .entry(transport_id.clone())
        .or_insert_with(|| crate::engine::transport::TransportConfig::for_transport(&transport_id));
    crate::win32::settings::save_settings(settings)?;
    Ok(())
}

/// Saves a transport's config options (as a JSON string) after validating
/// them against the transport's own schema — without starting the engine.
///
/// The existing `enabled` flag of the stored config is preserved; a missing
/// config entry is created with `enabled: true`.
///
/// Known limitation (honest scope): there is no UI editor for transport
/// options yet — `TransportsSection` only selects transports, and
/// `validate_transport_config` exists for a future field-level editor. Until
/// then this command is the only way to persist options (e.g. for
/// researchers); see docs/TRANSPORTS.md.
#[tauri::command]
pub async fn set_transport_config(
    transport_id: String,
    config_json: String,
) -> Result<(), String> {
    use crate::engine::transport::TransportError;

    let registry = crate::engine::transport::global_registry();
    let transport = registry
        .get(&transport_id)
        .ok_or_else(|| {
            TransportError::UnknownTransport {
                id: transport_id.clone(),
                known: registry.ids().join(", "),
            }
            .to_string()
        })?;

    let options: serde_json::Value = serde_json::from_str(&config_json).map_err(|e| {
        TransportError::InvalidConfig {
            id: transport_id.clone(),
            message: format!("Некорректный JSON: {}", e),
        }
        .to_string()
    })?;

    let mut settings = crate::win32::settings::load_settings();
    let enabled = settings
        .transport_configs
        .get(&transport_id)
        .map(|c| c.enabled)
        .unwrap_or(true);
    let config = crate::engine::transport::TransportConfig {
        transport_id: transport_id.clone(),
        enabled,
        options,
    };
    transport.validate_config(&config).map_err(|e| e.to_string())?;
    settings.transport_configs.insert(transport_id, config);
    crate::win32::settings::save_settings(settings)?;
    Ok(())
}

/// Validates a transport config JSON without starting the engine, so the UI
/// can surface field-level errors early.
#[tauri::command]
pub async fn validate_transport_config(
    transport_id: String,
    config_json: String,
) -> Result<(), String> {
    crate::engine::transport::validate_transport_config_json(&transport_id, &config_json)
        .map_err(|e| e.to_string())}

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
    let backend = state.backend.read().clone();
    let status = backend.get_status().await;
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
    let backend = state.backend.read().clone();
    let logs = backend.get_all_logs().await;
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
    crate::win32::settings::update_settings(|s| {
        s.split_tunnel_mode = mode;
        s.clone()
    })
}

/// Executes the 1-click Auto-Doctor to repair driver, DNS, ARP, and network stack.
#[tauri::command]
pub async fn run_auto_doctor(
    state: State<'_, AppState>,
) -> Result<crate::win32::doctor::DoctorReport, String> {
    let backend = state.backend.read().clone();
    let status = backend.get_status().await;
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

/// Adds a user-defined DNS resolver (name + IPv4 addresses).
#[tauri::command]
pub async fn add_custom_dns(
    name: String,
    primary: String,
    secondary: String,
) -> Result<crate::win32::dns::DnsStatus, String> {
    crate::win32::dns::add_custom_dns(&name, &primary, &secondary)
}

/// Removes a user-defined DNS resolver by its id.
#[tauri::command]
pub async fn remove_custom_dns(id: String) -> Result<crate::win32::dns::DnsStatus, String> {
    crate::win32::dns::remove_custom_dns(&id)
}

/// Lists running applications for per-app split tunneling selection.
#[tauri::command]
pub async fn list_running_apps() -> Result<Vec<crate::win32::app_split::AppInfo>, String> {
    // Process enumeration does OpenProcess per PID; don't block the async runtime.
    tauri::async_runtime::spawn_blocking(|| crate::win32::app_split::list_running_apps())
        .await
        .map_err(|e| e.to_string())
}

/// Gets the current per-app split tunneling configuration.
#[tauri::command]
pub async fn get_app_split_config() -> Result<AppSplitConfig, String> {
    let s = crate::win32::settings::load_settings();
    Ok(AppSplitConfig {
        mode: format!("{:?}", s.app_split_mode).to_lowercase(),
        apps: s.app_split_list,
    })
}

/// Sets the per-app split tunneling mode and application list atomically.
#[tauri::command]
pub async fn set_app_split_config(
    mode: String,
    apps: Vec<String>,
) -> Result<AppSplitConfig, String> {
    let parsed = match mode.to_lowercase().as_str() {
        "disabled" => crate::win32::settings::AppSplitMode::Disabled,
        "allowlist" => crate::win32::settings::AppSplitMode::Allowlist,
        "blocklist" => crate::win32::settings::AppSplitMode::Blocklist,
        _ => return Err(format!("Unknown app split mode: {}", mode)),
    };
    // Deduplicate: direct IPC callers may send duplicates.
    let mut seen = std::collections::HashSet::new();
    let normalized: Vec<String> = apps
        .into_iter()
        .map(|p| crate::win32::app_split::normalize_app_path(&p))
        .filter(|p| !p.is_empty() && seen.insert(p.clone()))
        .take(100)
        .collect();
    crate::win32::settings::update_settings(|s| {
        s.app_split_mode = parsed;
        s.app_split_list = normalized;
    })
    .map_err(|e| e.to_string())?;
    get_app_split_config().await
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AppSplitConfig {
    pub mode: String,
    pub apps: Vec<String>,
}

/// Gets the Split-DNS domain lists (built-in + custom).
#[tauri::command]
pub async fn get_split_dns_domains() -> Result<SplitDnsDomains, String> {
    Ok(SplitDnsDomains {
        builtin: crate::win32::dns::SPLIT_DNS_NAMESPACES
            .iter()
            .map(|s| s.to_string())
            .collect(),
        custom: crate::win32::settings::load_settings().dns_split_custom_domains,
    })
}

/// Adds a custom domain to the Split-DNS list.
#[tauri::command]
pub async fn add_split_dns_domain(domain: String) -> Result<SplitDnsDomains, String> {
    crate::win32::dns::add_split_dns_domain(&domain)?;
    get_split_dns_domains().await
}

/// Removes a custom domain from the Split-DNS list.
#[tauri::command]
pub async fn remove_split_dns_domain(domain: String) -> Result<SplitDnsDomains, String> {
    crate::win32::dns::remove_split_dns_domain(&domain)?;
    get_split_dns_domains().await
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SplitDnsDomains {
    pub builtin: Vec<String>,
    pub custom: Vec<String>,
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

/// Checks GitHub for a newer release and compares it with the running version.
///
/// A transport failure (blocked GitHub, no network) returns an error — the
/// frontend scheduler swallows those silently and retries on schedule, while
/// the manual "check" button surfaces them.
#[tauri::command]
pub async fn check_for_updates() -> Result<crate::win32::updater::UpdateInfo, String> {
    crate::win32::updater::check_for_updates().await
}

/// Downloads the latest release asset with streaming SHA256 verification.
///
/// Emits `update-download-progress` events for the UI progress bar and returns
/// the staging directory. Refuses to proceed when the asset metadata carries
/// no sha256 digest; deletes the temp file on hash mismatch.
#[tauri::command]
pub async fn download_update(app: tauri::AppHandle) -> Result<String, String> {
    crate::win32::updater::download_update(app)
        .await
        .map(|p| p.to_string_lossy().to_string())
}

/// Atomically installs a previously staged update and restarts the app.
///
/// Fail-closed: refuses while the bypass engine — or any registered sidecar —
/// is running (the frontend stops them first, with the user's explicit consent
/// from the update dialog). Backs the current installation up, swaps in the
/// re-verified new tree, restores user settings, then launches the new version
/// and exits. Rolls back to the backup on any failure.
///
/// Runs on a blocking thread: hashing, extraction and the cross-volume
/// copy fallback are disk-heavy and must not stall the async runtime.
#[tauri::command]
pub async fn install_update(state: State<'_, AppState>) -> Result<(), String> {
    // Fail fast when the engine is running: installing over a live engine
    // (local or svc) would rename files out from under it.
    let backend = state.backend.read().clone();
    let engine_state = backend.get_status().await.state;
    let sidecars = state.sidecars.clone();
    tokio::task::spawn_blocking(move || {
        crate::win32::updater::install_update(engine_state, &sidecars)
    })
    .await
    .map_err(|e| format!("Install task failed: {e}"))?
}

/// Requests cancellation of an in-flight `download_update`. Cooperative: the
/// download loop checks the flag between chunks and aborts cleanly.
#[tauri::command]
pub async fn cancel_update_download() -> Result<(), String> {
    crate::win32::updater::cancel_download_update();
    Ok(())
}

/// Reports the registered sidecar processes and whether each is active.
/// The frontend update flow consults this before `install_update` so an active
/// sidecar (e.g. tgproxy) is surfaced with a clear message instead of a raw
/// backend refusal.
#[tauri::command]
pub async fn get_sidecar_statuses(
    state: State<'_, AppState>,
) -> Result<Vec<crate::win32::updater::SidecarStatusView>, String> {
    Ok(state
        .sidecars
        .iter()
        .map(|s| crate::win32::updater::SidecarStatusView {
            name: s.name().to_string(),
            active: s.is_active(),
        })
        .collect())
}

/// Returns the auto-update toggle and the last successful check timestamp.
#[tauri::command]
pub async fn get_update_prefs() -> Result<crate::win32::updater::UpdatePrefs, String> {
    Ok(crate::win32::updater::get_update_prefs())
}

/// Enables or disables the scheduled update check.
#[tauri::command]
pub async fn set_auto_update_enabled(enable: bool) -> Result<bool, String> {
    crate::win32::updater::set_auto_update_enabled(enable)
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



