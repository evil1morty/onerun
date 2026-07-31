mod commands;
mod process;
mod types;
mod util;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{
    Emitter, Manager, WebviewWindow,
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{MouseButton, TrayIconBuilder, TrayIconEvent},
};

use types::AppState;

/// Show and focus the main window.
fn show_window(win: &WebviewWindow) {
    let _ = win.show();
    let _ = win.unminimize();
    let _ = win.set_focus();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(win) = app.get_webview_window("main") {
                show_window(&win);
            }
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--minimized"]),
        ))
        .setup(|app| {
            process::init_job_object();

            let config_dir = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| PathBuf::from("."));

            app.manage(AppState {
                processes: Arc::new(Mutex::new(HashMap::new())),
                log_viewer: Arc::new(Mutex::new(None)),
                config_path: Mutex::new(config_dir.join("projects.json")),
                settings_path: Mutex::new(config_dir.join("settings.json")),
                force_close: Mutex::new(false),
            });

            // ── System tray ───────────────────────────────
            let show_item = MenuItemBuilder::with_id("show", "Show OneRun").build(app)?;
            let quit_item = MenuItemBuilder::with_id("quit", "Quit").build(app)?;
            let tray_menu = MenuBuilder::new(app)
                .items(&[&show_item, &quit_item])
                .build()?;

            TrayIconBuilder::new()
                .icon(app.default_window_icon().cloned().unwrap())
                .tooltip("OneRun")
                .menu(&tray_menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "show" => {
                        if let Some(win) = app.get_webview_window("main") {
                            show_window(&win);
                        }
                    }
                    "quit" => {
                        let has_running = app
                            .try_state::<AppState>()
                            .map(|s| s.processes.lock()
                                .map(|map| map.values().any(|ps| ps.running))
                                .unwrap_or(false))
                            .unwrap_or(false);

                        if has_running {
                            if let Some(win) = app.get_webview_window("main") {
                                show_window(&win);
                                let _ = win.emit("confirm-close", ());
                            }
                        } else {
                            // Even with no tracked running processes, a child may have
                            // been spawned milliseconds ago and not yet recorded in the
                            // state map. On Windows the global JOB_HANDLE kills it on
                            // exit; on Linux/macOS there is no equivalent, so we kill
                            // explicitly to avoid orphaned dev servers.
                            if let Some(s) = app.try_state::<AppState>() {
                                process::kill_all(&s.processes);
                            }
                            app.exit(0);
                        }
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, .. } = event {
                        if let Some(win) = tray.app_handle().get_webview_window("main") {
                            show_window(&win);
                        }
                    }
                })
                .build(app)?;

            // Hide on startup when launched via autostart
            if std::env::args().any(|a| a == "--minimized") {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.hide();
                }
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::pick_folder,
            commands::scan_project,
            commands::start_process,
            commands::stop_process,
            commands::stop_all_processes,
            commands::purge_project,
            commands::get_logs,
            commands::set_log_viewer,
            commands::get_status,
            commands::get_all_status,
            commands::open_in_explorer,
            commands::open_in_editor,
            commands::open_in_terminal,
            commands::open_in_claude,
            commands::open_in_browser,
            commands::load_config,
            commands::save_config,
            commands::load_settings,
            commands::save_settings,
            commands::force_close,
            commands::get_autostart,
            commands::set_autostart,
            commands::check_paths_exist,
            commands::get_default_projects_dir,
            commands::create_project_folder,
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() != "main" {
                    return;
                }

                // Force close: kill all processes and exit
                if let Some(state) = window.try_state::<AppState>() {
                    if *state.force_close.lock().unwrap() {
                        process::kill_all(&state.processes);
                        window.app_handle().exit(0);
                        return;
                    }
                }

                // Normal close: hide to system tray
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
