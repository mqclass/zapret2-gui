// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod engine;
mod win32;

use std::sync::Arc;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::Manager;

use commands::AppState;
use engine::supervisor::Supervisor;
use win32::privileges::{ensure_elevated, is_elevated};

#[tokio::main]
async fn main() {
    // 1. Enforce Administrator elevation for WinDivert driver registration
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

    // 3. Build Tauri v2 application
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(AppState {
            supervisor: supervisor_clone,
            diagnostics: diagnostics_clone,
            cached_binaries: Arc::new(parking_lot::RwLock::new(None)),
        })
        .setup(move |app| {
            let handle = app.handle().clone();
            supervisor.set_app_handle(handle.clone());

            // Build System Tray Menu
            let toggle_item = MenuItem::with_id(app, "toggle", "Показать / Скрыть (Show/Hide)", true, None::<&str>)?;
            let start_item = MenuItem::with_id(app, "start", "Включить обход (Start Bypass)", true, None::<&str>)?;
            let stop_item = MenuItem::with_id(app, "stop", "Выключить обход (Stop Bypass)", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Выход (Exit)", true, None::<&str>)?;

            let tray_menu = Menu::with_items(
                app,
                &[
                    &toggle_item,
                    &start_item,
                    &stop_item,
                    &quit_item,
                ],
            )?;

            let supervisor_tray = supervisor.clone();

            let mut tray_builder = TrayIconBuilder::new()
                .menu(&tray_menu)
                .show_menu_on_left_click(false);

            if let Some(icon) = app.default_window_icon().cloned() {
                tray_builder = tray_builder.icon(icon);
            }

            let _tray = tray_builder
                .on_menu_event(move |app_h, event| {
                    match event.id.as_ref() {
                        "toggle" => {
                            if let Some(window) = app_h.get_webview_window("main") {
                                if window.is_visible().unwrap_or(false) {
                                    let _ = window.hide();
                                } else {
                                    let _ = window.show();
                                    let _ = window.set_focus();
                                }
                            }
                        }
                        "start" => {
                            let sup = supervisor_tray.clone();
                            tokio::spawn(async move {
                                let _ = sup.start(None, None).await;
                            });
                        }
                        "stop" => {
                            let sup = supervisor_tray.clone();
                            tokio::spawn(async move {
                                let _ = sup.stop().await;
                            });
                        }
                        "quit" => {
                            let sup = supervisor_tray.clone();
                            let h = app_h.clone();
                            tokio::spawn(async move {
                                let _ = sup.stop().await;
                                h.exit(0);
                            });
                        }
                        _ => {}
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            if window.is_visible().unwrap_or(false) {
                                let _ = window.hide();
                            } else {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                    }
                })
                .build(app)?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_system_status,
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
        ])
        .run(tauri::generate_context!())
        .expect("Error while running zapret2 Tauri supervisor application");
}
