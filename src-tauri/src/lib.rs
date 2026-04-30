mod audio;
mod config;
mod display;
mod monitor;

use audio::AudioController;
use config::{ConfigManager, DisplayConfig};
use display::DisplayController;
use monitor::VdMonitor;

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Manager,
};

struct AppState {
    config: ConfigManager,
    monitor: VdMonitor,
    display: DisplayController,
    audio: AudioController,
    auto_mute_was_muted: Mutex<bool>,
}

static APP_STATE: Lazy<Arc<AppState>> = Lazy::new(|| {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    let config_dir = exe_dir.join("config");

    Arc::new(AppState {
        config: ConfigManager::new(config_dir),
        monitor: VdMonitor::new(),
        display: DisplayController::new(),
        audio: AudioController::new(),
        auto_mute_was_muted: Mutex::new(false),
    })
});

#[tauri::command]
fn get_status() -> serde_json::Value {
    let state = &*APP_STATE;
    json!({
        "vd_running": state.monitor.is_vd_running(),
        "connected": state.monitor.is_connected(),
        "vdd_installed": state.display.is_vdd_installed(),
        "monitoring": true,
        "baseline_ready": state.display.has_initial_state(),
        "switched": state.display.is_switched()
    })
}

#[tauri::command]
fn restore_display() -> serde_json::Value {
    let success = APP_STATE.display.restore_display();
    json!({"success": success})
}

#[tauri::command]
fn get_monitors() -> serde_json::Value {
    let state = &*APP_STATE;
    let monitors = state.display.get_all_monitors();
    let active_id = monitors
        .iter()
        .find(|monitor| monitor.primary)
        .or_else(|| monitors.iter().find(|monitor| monitor.active))
        .map(|monitor| monitor.id.clone())
        .unwrap_or_default();
    let config = state.config.display_config.lock();

    json!({
        "monitors": monitors,
        "active_id": active_id,
        "target_id": config.target_id,
        "auto_switch": config.auto_switch,
        "auto_mute": config.auto_mute,
        "vdd_installed": state.display.is_vdd_installed(),
        "baseline_ready": state.display.has_initial_state(),
        "switched": state.display.is_switched()
    })
}

#[tauri::command]
fn switch_display(data: serde_json::Value) -> serde_json::Value {
    let display_id = data["display_id"].as_str().unwrap_or("");
    let success = APP_STATE.display.switch_to_display(display_id);
    json!({"success": success})
}

#[tauri::command]
fn set_display_settings(data: serde_json::Value) -> serde_json::Value {
    let state = &*APP_STATE;
    let mut config = state.config.display_config.lock();

    if let Some(v) = data["auto_switch"].as_bool() {
        config.auto_switch = v;
    }
    if let Some(v) = data["target_id"].as_str() {
        config.target_id = v.to_string();
    }
    if let Some(v) = data["auto_mute"].as_bool() {
        config.auto_mute = v;
    }

    let cloned = config.clone();
    drop(config);
    state.config.save_display_config(&cloned);

    if !state.monitor.is_connected() {
        let _ = state.display.capture_initial_state_if_needed();
    }

    json!({"success": true})
}

#[tauri::command]
fn toggle_mute() -> serde_json::Value {
    let (success, muted) = APP_STATE.audio.toggle_mute();
    json!({
        "success": success,
        "muted": muted
    })
}

#[tauri::command]
fn get_mute_state() -> serde_json::Value {
    json!({
        "success": true,
        "muted": APP_STATE.audio.get_mute_state()
    })
}

#[tauri::command]
fn hide_to_tray(app: tauri::AppHandle) -> serde_json::Value {
    let success = app
        .get_webview_window("main")
        .map(|window| window.hide().is_ok())
        .unwrap_or(false);
    json!({ "success": success })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let state = APP_STATE.clone();
    let initial_connected = state.monitor.refresh_now();
    if !initial_connected {
        let _ = state.display.capture_initial_state();
    }

    let closure_state = APP_STATE.clone();
    APP_STATE.monitor.start(move |connected| {
        let config = closure_state.config.display_config.lock().clone();

        if !connected {
            let _ = closure_state.display.capture_initial_state_if_needed();
        }

        if config.auto_mute {
            handle_auto_mute(&closure_state, connected);
        }

        if config.auto_switch {
            handle_display_switch(&closure_state, connected, &config);
        }
    });

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            get_status,
            restore_display,
            get_monitors,
            switch_display,
            set_display_settings,
            toggle_mute,
            get_mute_state,
            hide_to_tray,
        ])
        .setup(|app| {
            setup_tray(app)?;
            restore_window(app);
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| match event {
            tauri::RunEvent::WindowEvent { label, event, .. } => match event {
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    api.prevent_close();
                    if let Some(window) = app_handle.get_webview_window(label.as_str()) {
                        save_window_position(&window);
                        let _ = window.hide();
                    }
                }
                tauri::WindowEvent::Moved { .. } => {
                    save_window_position_later();
                }
                _ => {}
            },
            tauri::RunEvent::ExitRequested { api, .. } => {
                api.prevent_exit();
            }
            _ => {}
        });
}

fn restore_window(app: &tauri::App) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };

    let wc = APP_STATE.config.window_config.lock().clone();
    let mut on_screen = false;
    if let Ok(monitors) = window.available_monitors() {
        for monitor in monitors {
            let pos = monitor.position();
            let size = monitor.size();
            let mx = pos.x;
            let my = pos.y;
            let mw = size.width as i32;
            let mh = size.height as i32;
            if wc.x >= mx && wc.x < mx + mw && wc.y >= my && wc.y < my + mh {
                on_screen = true;
                break;
            }
        }
    }

    let position = if on_screen {
        tauri::PhysicalPosition::new(wc.x, wc.y)
    } else {
        tauri::PhysicalPosition::new(100, 100)
    };
    let _ = window.set_position(position);
    let _ = window.set_size(tauri::PhysicalSize::new(wc.width, wc.height));
    let _ = window.show();
}

fn save_window_position(window: &tauri::WebviewWindow) {
    let state = &*APP_STATE;
    if state.display.is_switched() {
        return;
    }

    if let (Ok(pos), Ok(size)) = (window.outer_position(), window.outer_size()) {
        let wc = config::WindowConfig {
            x: pos.x,
            y: pos.y,
            width: size.width,
            height: size.height,
        };
        *state.config.window_config.lock() = wc.clone();
        state.config.save_window_config(&wc);
    }
}

fn save_window_position_later() {
    static MOVE_TIMER: Lazy<Arc<Mutex<Option<std::time::Instant>>>> =
        Lazy::new(|| Arc::new(Mutex::new(None)));
    static WORKER_STARTED: Lazy<()> = Lazy::new(|| {
        let timer = MOVE_TIMER.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_millis(200));
            let should_resume = timer
                .lock()
                .map(|t| t.elapsed() > std::time::Duration::from_millis(500))
                .unwrap_or(false);
            if should_resume {
                *timer.lock() = None;
                APP_STATE.monitor.resume();
            }
        });
    });

    Lazy::force(&WORKER_STARTED);
    APP_STATE.monitor.pause();
    *MOVE_TIMER.lock() = Some(std::time::Instant::now());
}

fn setup_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let show = MenuItem::with_id(app, "show", "显示窗口", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    let _tray = TrayIconBuilder::new()
        .icon(app.default_window_icon().cloned().unwrap())
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("VDSleep - VR 显示器管理")
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                if let Some(window) = app.get_webview_window("main") {
                    window.show().ok();
                    window.set_focus().ok();
                }
            }
            "quit" => {
                if let Some(window) = app.get_webview_window("main") {
                    save_window_position(&window);
                }
                std::process::exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let tauri::tray::TrayIconEvent::Click {
                button: tauri::tray::MouseButton::Left,
                button_state: tauri::tray::MouseButtonState::Up,
                ..
            } = event
            {
                if let Some(window) = tray.app_handle().get_webview_window("main") {
                    window.show().ok();
                    window.unminimize().ok();
                    window.set_focus().ok();
                }
            }
        })
        .build(app)?;

    Ok(())
}

fn handle_auto_mute(state: &AppState, connected: bool) {
    if !connected {
        state.audio.set_mute(true);
        *state.auto_mute_was_muted.lock() = true;
    } else if *state.auto_mute_was_muted.lock() {
        state.audio.set_mute(false);
        *state.auto_mute_was_muted.lock() = false;
    }
}

fn handle_display_switch(state: &AppState, connected: bool, config: &DisplayConfig) {
    if connected {
        if config.target_id.trim().is_empty() || !state.display.has_initial_state() {
            return;
        }

        let _ = state.display.switch_to_display(&config.target_id);
    } else if state.display.is_switched() {
        let _ = state.display.restore_display();
    }
}
