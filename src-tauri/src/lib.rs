mod model;
mod protocol;
mod runtime;
mod settings;

use runtime::AppRuntime;
use std::sync::Arc;
use tauri::{
    menu::{CheckMenuItemBuilder, MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
    AppHandle, Manager, PhysicalPosition, WebviewWindow, WindowEvent,
};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            show_overlay(app, true);
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(|app| {
            let loaded = settings::load(app.handle());
            let mut settings = loaded.value;
            let autostart = app.autolaunch();
            if !settings.autostart_initialized {
                if let Err(error) = autostart.enable() {
                    eprintln!("Could not enable Windows startup: {error}");
                }
                settings.autostart_initialized = true;
                let _ = settings::save(&loaded.path, &settings);
            } else if autostart.is_enabled().unwrap_or(false) {
                // Re-register an enabled entry so an installed build replaces a
                // development or previously installed executable path.
                if let Err(error) = autostart.enable() {
                    eprintln!("Could not refresh Windows startup: {error}");
                }
            }
            let autostart_enabled = autostart.is_enabled().unwrap_or(false);
            let runtime = AppRuntime::new(
                app.handle().clone(),
                settings.clone(),
                loaded.path,
                autostart_enabled,
            );
            app.manage(Arc::clone(&runtime));

            let window = app
                .get_webview_window("main")
                .ok_or("main overlay window was not created")?;
            if settings.expanded {
                tauri::async_runtime::block_on(runtime.set_expanded(true))?;
            }
            place_window(&window, settings.window_x, settings.window_y);
            attach_window_events(&window, Arc::clone(&runtime));
            setup_tray(app.handle(), &runtime, autostart_enabled)?;

            tauri::async_runtime::spawn(Arc::clone(&runtime).supervise());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            runtime::get_app_state,
            runtime::refresh_usage,
            runtime::begin_chatgpt_login,
            runtime::set_codex_executable,
            runtime::set_autostart_enabled,
            runtime::set_overlay_expanded,
            runtime::set_overlay_height,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Codex Usage Overlay");

    app.run(|app, event| {
        if matches!(event, tauri::RunEvent::ExitRequested { .. }) {
            let runtime = app.state::<Arc<AppRuntime>>().inner().clone();
            tauri::async_runtime::block_on(runtime.shutdown());
        }
    });
}

fn setup_tray(
    app: &AppHandle,
    runtime: &Arc<AppRuntime>,
    autostart_enabled: bool,
) -> tauri::Result<()> {
    let show = MenuItemBuilder::with_id("show", "Show / Hide").build(app)?;
    let refresh = MenuItemBuilder::with_id("refresh", "Refresh now").build(app)?;
    let autostart = CheckMenuItemBuilder::with_id("autostart", "Start with Windows")
        .checked(autostart_enabled)
        .build(app)?;
    let quit = MenuItemBuilder::with_id("quit", "Quit").build(app)?;
    let menu = MenuBuilder::new(app)
        .items(&[&show, &refresh, &autostart, &quit])
        .build()?;

    runtime.set_tray_autostart_item(autostart.clone());
    let autostart_for_event = autostart;
    TrayIconBuilder::with_id("main")
        .icon(
            app.default_window_icon()
                .expect("application icon should be bundled")
                .clone(),
        )
        .tooltip("Codex Usage Overlay")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| {
            let runtime = app.state::<Arc<AppRuntime>>().inner().clone();
            match event.id().as_ref() {
                "show" => toggle_overlay(app),
                "refresh" => {
                    tauri::async_runtime::spawn(async move {
                        let _ = runtime.refresh().await;
                    });
                }
                "autostart" => {
                    let enabled = autostart_for_event.is_checked().unwrap_or(false);
                    let autostart_for_error = autostart_for_event.clone();
                    tauri::async_runtime::spawn(async move {
                        if runtime.set_autostart(enabled).await.is_err() {
                            let _ = autostart_for_error.set_checked(!enabled);
                        }
                    });
                }
                "quit" => {
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        runtime.shutdown().await;
                        app.exit(0);
                    });
                }
                _ => {}
            }
        })
        .build(app)?;
    Ok(())
}

fn attach_window_events(window: &WebviewWindow, runtime: Arc<AppRuntime>) {
    let overlay = window.clone();
    window.on_window_event(move |event| match event {
        WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            let _ = overlay.hide();
        }
        WindowEvent::Moved(position) => {
            let position = *position;
            let runtime = Arc::clone(&runtime);
            tauri::async_runtime::spawn(async move {
                runtime.record_window_position(position.x, position.y).await;
            });
        }
        _ => {}
    });
}

fn place_window(window: &WebviewWindow, saved_x: Option<i32>, saved_y: Option<i32>) {
    let Ok(size) = window.outer_size() else {
        return;
    };
    let monitors = window.available_monitors().unwrap_or_default();
    if monitors.is_empty() {
        return;
    }

    let margin = 20i32;
    let desired = saved_x.zip(saved_y);
    let monitor = desired
        .and_then(|(x, y)| {
            monitors.iter().find(|monitor| {
                let position = monitor.position();
                let size = monitor.size();
                x >= position.x
                    && x < position.x + size.width as i32
                    && y >= position.y
                    && y < position.y + size.height as i32
            })
        })
        .or_else(|| {
            window.primary_monitor().ok().flatten().and_then(|primary| {
                monitors
                    .iter()
                    .find(|monitor| monitor.name() == primary.name())
            })
        })
        .unwrap_or(&monitors[0]);

    let monitor_position = monitor.position();
    let monitor_size = monitor.size();
    let min_x = monitor_position.x + margin;
    let min_y = monitor_position.y + margin;
    let max_x =
        (monitor_position.x + monitor_size.width as i32 - size.width as i32 - margin).max(min_x);
    let max_y =
        (monitor_position.y + monitor_size.height as i32 - size.height as i32 - margin).max(min_y);
    let (x, y) = desired
        .map(|(x, y)| (x.clamp(min_x, max_x), y.clamp(min_y, max_y)))
        .unwrap_or((max_x, min_y));
    let _ = window.set_position(PhysicalPosition::new(x, y));
}

fn show_overlay(app: &AppHandle, focus: bool) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        if focus {
            let _ = window.set_focus();
        }
        let runtime = app.state::<Arc<AppRuntime>>().inner().clone();
        tauri::async_runtime::spawn(async move {
            let _ = runtime.refresh().await;
        });
    }
}

fn toggle_overlay(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        if window.is_visible().unwrap_or(false) {
            let _ = window.hide();
        } else {
            show_overlay(app, true);
        }
    }
}
