// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod engine;
mod svc_client;
mod svc_ipc;
mod platform;

use std::sync::Arc;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::Manager;

use commands::AppState;
use commands::CachedBinaries;
use engine::supervisor::Supervisor;
use crate::platform::privileges::{ensure_elevated, is_elevated};

#[tokio::main]
async fn main() {
    // 0. WebView2 pre-flight: the UI cannot render without the system WebView2
    //    runtime. Show a plain-language dialog instead of a raw framework error.
    crate::platform::webview_check::ensure_webview2();

    // 1. Enforce Administrator elevation for WinDivert driver registration.
    //    If the user declines the UAC prompt, the app still starts but the UI shows
    //    an admin gate and engine start is refused until elevated (see start_supervisor).
    let elevated = is_elevated().unwrap_or(false);
    if !elevated {
        // Attempt automatic UAC elevation
        if let Err(e) = ensure_elevated() {
            eprintln!("Warning: Failed to auto-elevate: {}", e);
        }
    }

    // 2. Initialize supervisor engine with Windows Job Object isolation
    let supervisor = Arc::new(
        Supervisor::new().expect("Failed to initialize Windows Job Object and supervisor"),
    );
    let diagnostics = supervisor.diagnostics();

    let supervisor_clone = supervisor.clone();
    let diagnostics_clone = diagnostics.clone();

    // 2b. Initialize the native Telegram bridge manager. The MTProto→WebSocket
    // bridge runs as a Tokio task (no external executable), sharing the
    // supervisor only as a funnel into the common log buffer. It starts
    // in-process, but migrates into zapret2-svc.exe as soon as we attach
    // to the svc — so a GUI crash never kills the bridge either.
    let tgproxy = Arc::new(engine::tgproxy::TgProxyManager::new(supervisor.clone()));
    let tgproxy_backend =
        Arc::new(parking_lot::RwLock::new(crate::svc_client::TgproxyBackend::Local(
            tgproxy.clone(),
        )));

    // 3. Build Tauri v2 application
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // Intercept close button and hide to system tray instead of terminating
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .manage(AppState {
            backend: Arc::new(parking_lot::RwLock::new(
                crate::svc_client::SupervisorBackend::Local(supervisor_clone),
            )),
            diagnostics: diagnostics_clone,
            cached_binaries: Arc::new(parking_lot::RwLock::new(CachedBinaries {
                resolved: None,
                resolved_at: None,
            })),
            // No external sidecars: the Telegram bridge runs in-process,
            // so there is nothing holding files for install_update to gate on.
            sidecars: Vec::new(),
            tgproxy: tgproxy_backend,        })
        .setup(move |app| {
            let handle = app.handle().clone();
            // If a svc instance is already running (e.g. the GUI was restarted
            // while the bypass stayed up), attach to it instead of the local
            // engine. The Telegram bridge follows: it runs inside the svc.
            {
                let app_state = handle.state::<crate::commands::AppState>();
                let backend = app_state.backend.clone();
                let tgproxy_backend = app_state.tgproxy.clone();
                let handle_clone = handle.clone();
                tauri::async_runtime::spawn(async move {
                    if let Ok(client) = crate::svc_client::SvcClient::connect().await {
                        if client.hello().await.is_ok() {
                            let client = std::sync::Arc::new(client);
                            *backend.write() =
                                crate::svc_client::SupervisorBackend::Remote(client.clone());
                            // The bridge follows into the svc; stop any
                            // in-process copy first so two never coexist.
                            let old_tgproxy = tgproxy_backend.read().clone();
                            old_tgproxy.stop_local_for_migration().await;
                            *tgproxy_backend.write() =
                                crate::svc_client::TgproxyBackend::Remote(client);
                            tauri::async_runtime::spawn(
                                crate::svc_client::SvcClient::forward_events(handle_clone),
                            );
                        }
                    }
                });
            }
            let sink: Arc<dyn crate::engine::EventSink> =
                Arc::new(crate::engine::TauriSink::new(handle.clone()));
            supervisor.set_event_sink(sink.clone());
            tgproxy.set_event_sink(sink);

            // If the Telegram proxy was left enabled, restore it (best-effort).
            // Runs after the attach probe above, so it lands in the svc when
            // one is running.
            let tgproxy_auto = handle
                .state::<crate::commands::AppState>()
                .tgproxy
                .clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                let enabled = crate::platform::settings::load_settings().tgproxy_enabled;
                if enabled {
                    // Drop the parking_lot guard before .await: the guard is
                    // not Send, and spawn requires a Send future.
                    let tgproxy = tgproxy_auto.read().clone();
                    if let Err(e) = tgproxy.start().await {
                        eprintln!("Warning: Failed to auto-start Telegram proxy: {}", e);
                    }
                }
            });

            // If started via Windows autostart with --minimized or --autostart
            let is_autostart = std::env::args().any(|a| a == "--minimized" || a == "--autostart");
            if is_autostart {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }

                // Automatically start bypass engine on Windows logon
                let backend_auto = app.state::<crate::commands::AppState>().backend.clone();
                let tgproxy_auto =
                    app.state::<crate::commands::AppState>().tgproxy.clone();
                let app_auto = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    // Small delay to allow Windows network adapters to finish DHCP/gateway connection
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

                    // Migrate to the detached svc when its binary is present.
                    // Bind the clone first: parking_lot guard is not Send.
                    // A locally running bridge is stopped first (transiently):
                    // the svc autostarts its own copy and must find the port
                    // free; the local copy is re-checked after the swap too,
                    // in case the 3s bridge autostart fired mid-attach.
                    let backend_snapshot = backend_auto.read().clone();
                    if crate::svc_client::backend_needs_attach(&backend_snapshot).await
                    {
                        let old_tgproxy = tgproxy_auto.read().clone();
                        let bridge_was_running =
                            old_tgproxy.stop_local_for_migration().await;
                        if let Ok(client) = crate::svc_client::ensure_svc_attached(
                            std::time::Duration::from_secs(10),
                        )
                        .await
                        {
                            *backend_auto.write() =
                                crate::svc_client::SupervisorBackend::Remote(client.clone());
                            // The bridge follows the engine into the svc.
                            *tgproxy_auto.write() =
                                crate::svc_client::TgproxyBackend::Remote(client);
                            old_tgproxy.stop_local_for_migration().await;
                            let h = app_auto.clone();
                            tokio::spawn(async move {
                                crate::svc_client::SvcClient::forward_events(h).await;
                            });
                        } else if bridge_was_running {
                            old_tgproxy.restart_local().await;
                        }
                    }
                    let b = backend_auto.read().clone();
                    if let Err(e) = b.start(None, None).await {
                        eprintln!("Warning: Failed to auto-start bypass on boot: {}", e);
                    }
                });
            }

            // Build System Tray Menu
            let toggle_item = MenuItem::with_id(app, "toggle", "Открыть окно zapret2", true, None::<&str>)?;
            let start_item = MenuItem::with_id(app, "start", "Включить обход", true, None::<&str>)?;
            let stop_item = MenuItem::with_id(app, "stop", "Выключить обход", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Выход из программы", true, None::<&str>)?;

            let tray_menu = Menu::with_items(
                app,
                &[
                    &toggle_item,
                    &start_item,
                    &stop_item,
                    &quit_item,
                ],
            )?;

            let tgproxy_tray = app
                .state::<crate::commands::AppState>()
                .tgproxy
                .clone();

            let icon = match app.default_window_icon().cloned() {
                Some(i) => i,
                None => tauri::image::Image::from_bytes(include_bytes!("../icons/32x32.png")).expect("Failed to load embedded tray icon"),
            };

            let _tray = TrayIconBuilder::new()
                .menu(&tray_menu)
                .icon(icon)
                .tooltip("zapret2 — Обход блокировок активен")
                .show_menu_on_left_click(false)
                .on_menu_event(move |app_h, event| {
                    match event.id.as_ref() {
                        "toggle" => {
                            if let Some(window) = app_h.get_webview_window("main") {
                                if window.is_visible().unwrap_or(false) {
                                    let _ = window.hide();
                                } else {
                                    let _ = window.show();
                                    let _ = window.unminimize();
                                    let _ = window.set_focus();
                                }
                            }
                        }
                        "start" => {
                            let backend = app_h.state::<crate::commands::AppState>().backend.clone();
                            let tgproxy_backend =
                                app_h.state::<crate::commands::AppState>().tgproxy.clone();
                            let h = app_h.clone();
                            tokio::spawn(async move {
                                // Same svc-migration policy as the Start button.
                                let backend_snapshot = backend.read().clone();
                                if crate::svc_client::backend_needs_attach(&backend_snapshot).await
                                {
                                    let old_tgproxy = tgproxy_backend.read().clone();
                                    let bridge_was_running =
                                        old_tgproxy.stop_local_for_migration().await;
                                    if let Ok(client) =
                                        crate::svc_client::ensure_svc_attached(
                                            std::time::Duration::from_secs(10),
                                        )
                                        .await
                                    {
                                        *backend.write() =
                                            crate::svc_client::SupervisorBackend::Remote(client.clone());
                                        // The bridge follows the engine into the svc.
                                        *tgproxy_backend.write() =
                                            crate::svc_client::TgproxyBackend::Remote(client);
                                        old_tgproxy.stop_local_for_migration().await;
                                        tokio::spawn(
                                            crate::svc_client::SvcClient::forward_events(h),
                                        );
                                    } else if bridge_was_running {
                                        old_tgproxy.restart_local().await;
                                    }
                                }
                                let b = backend.read().clone();
                                let _ = b.start(None, None).await;
                            });
                        }
                        "stop" => {
                            let backend = app_h.state::<crate::commands::AppState>().backend.clone();
                            tokio::spawn(async move {
                                let b = backend.read().clone();
                                let _ = b.stop().await;
                            });
                        }
                        "quit" => {
                            let backend = app_h.state::<crate::commands::AppState>().backend.clone();
                            let tgp = tgproxy_tray.clone();
                            let h = app_h.clone();
                            tokio::spawn(async move {
                                // A remote svc outlives the GUI by design (the bypass
                                // and the bridge stay up); only stop what runs
                                // in this process.
                                let t = tgp.read().clone();
                                if !t.is_remote() {
                                    let _ = t.stop().await;
                                }
                                let b = backend.read().clone();
                                if !b.is_remote() {
                                    let _ = b.stop().await;
                                }
                                h.exit(0);
                            });
                        }
                        _ => {}
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    match event {
                        TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        }
                        | TrayIconEvent::DoubleClick {
                            button: MouseButton::Left,
                            ..
                        } => {
                            let app = tray.app_handle();
                            if let Some(window) = app.get_webview_window("main") {
                                if window.is_visible().unwrap_or(false) {
                                    let _ = window.hide();
                                } else {
                                    let _ = window.show();
                                    let _ = window.unminimize();
                                    let _ = window.set_focus();
                                }
                            }
                        }
                        _ => {}
                    }
                })
                .build(app)?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_system_status,
            commands::get_supervisor_status,
            commands::list_strategies,
            commands::start_supervisor,
            commands::stop_supervisor,
            commands::get_probe_metrics,
            commands::refresh_probes,
            commands::get_logs,
            commands::clear_logs,
            commands::relaunch_elevated,
            commands::kill_stray_processes,
            commands::scan_conflicts,
            commands::resolve_conflicts,
            commands::save_custom_profile,
            commands::delete_custom_profile,
            commands::parse_args_string,
            commands::get_autostart_status,
            commands::set_autostart_status,
            commands::exit_app,
            commands::list_domain_files,
            commands::read_domain_list,
            commands::add_domain_to_list,
            commands::remove_domain_from_list,
            commands::flush_dns_cache,
            commands::open_lists_folder,
            commands::reveal_lists_folder,
            commands::open_logs_folder,
            commands::get_log_file_path,
            commands::ping_custom_target,
            commands::probe_strategy,
            commands::set_last_strategy,
            commands::get_app_settings,
            commands::save_app_settings,
            commands::set_notifications_enabled,
            commands::set_watchdog_enabled,
            commands::set_watchdog_interval,
            commands::set_accent_color,
            commands::list_transports,
            commands::get_active_transport,
            commands::set_active_transport,
            commands::set_transport_config,
            commands::validate_transport_config,
            commands::export_diagnostics_bundle,
            commands::set_split_tunnel_mode,
            commands::run_auto_doctor,
            commands::get_dns_status,
            commands::set_system_dns,
            commands::add_custom_dns,
            commands::remove_custom_dns,
            commands::list_running_apps,
            commands::get_app_split_config,
            commands::set_app_split_config,
            commands::get_split_dns_domains,
            commands::add_split_dns_domain,
            commands::remove_split_dns_domain,
            commands::get_smart_hosts_status,
            commands::enable_smart_hosts,
            commands::disable_smart_hosts,
            commands::refresh_smart_hosts,
            commands::set_smart_hosts_sources,
            commands::set_smart_hosts_adblock,
            commands::maybe_refresh_smart_hosts,
            commands::get_tgproxy_status,
            commands::start_tgproxy,
            commands::stop_tgproxy,
            commands::set_tgproxy_config,
            commands::regenerate_tgproxy_secret,
            commands::set_tgproxy_mode,
            commands::test_tgproxy_connection,
            commands::set_tgproxy_lan,
            commands::benchmark_youtube_speed,
            commands::sync_remote_lists,
            commands::check_for_updates,
            commands::download_update,
            commands::cancel_update_download,
            commands::install_update,
            commands::get_sidecar_statuses,
            commands::get_update_prefs,
            commands::set_auto_update_enabled,
            commands::window_minimize,
            commands::window_toggle_maximize,
            commands::window_start_dragging,
            commands::window_close,
        ])
        .run(tauri::generate_context!())
        .expect("Error while running zapret2 Tauri supervisor application");
}
