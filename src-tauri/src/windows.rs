use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::i18n;
use crate::state::AppState;

pub static QUITTING: AtomicBool = AtomicBool::new(false);

pub fn show_main(app: &AppHandle) -> Result<(), String> {
    let window = app.get_webview_window("main").ok_or("主窗口不存在")?;
    if window.is_minimized().unwrap_or(false) {
        let _ = window.unminimize();
    }
    window.show().map_err(|e| e.to_string())?;
    let _ = window.set_focus();
    Ok(())
}

pub fn create_main(app: &AppHandle) -> Result<(), String> {
    let window = app.get_webview_window("main").ok_or("主窗口不存在")?;
    let target = window.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            if !QUITTING.load(Ordering::SeqCst) {
                api.prevent_close();
                let _ = target.hide();
            }
        }
    });

    let hidden = std::env::args().any(|a| a == "--hidden" || a == "--widget");
    if !hidden {
        window.show().map_err(|e| e.to_string())?;
        let _ = window.set_focus();
    }
    Ok(())
}

pub fn open_widget(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let prefs = state
        .data
        .lock()
        .map(|d| d.settings.widget.clone())
        .unwrap_or_default();

    if let Some(window) = app.get_webview_window("widget") {
        let _ = window.show();
        let _ = window.set_always_on_top(prefs.on_top);
        if !prefs.on_top {
            let _ = send_to_bottom(&window);
        }
        return Ok(());
    }

    let (width, height, x, y) = match &prefs.bounds {
        Some(b) => (
            b.width.max(340.0),
            b.height.max(380.0),
            Some(b.x),
            Some(b.y),
        ),
        None => (410.0, 650.0, None, None),
    };

    let mut builder = WebviewWindowBuilder::new(
        app,
        "widget",
        WebviewUrl::App("widget.html".into()),
    )
    .title(i18n::tr("橙子日程 · 桌面便签", &state.language()))
    .inner_size(width, height)
    .min_inner_size(340.0, 380.0)
    .max_inner_size(680.0, 1000.0)
    .decorations(false)
    .transparent(true)
    .resizable(true)
    .skip_taskbar(true)
    .always_on_top(prefs.on_top)
    .visible(false);

    if let (Some(x), Some(y)) = (x, y) {
        builder = builder.position(x, y);
    }

    let window = builder.build().map_err(|e| e.to_string())?;
    let target = window.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = target.hide();
        }
    });
    window.show().map_err(|e| e.to_string())?;
    if !prefs.on_top {
        let _ = send_to_bottom(&window);
    }
    Ok(())
}

pub fn close_widget(app: &AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("widget") {
        let _ = window.hide();
    }
    Ok(())
}

pub fn show_reminder(app: &AppHandle, lang: &str) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("reminder") {
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }
    let window = WebviewWindowBuilder::new(
        app,
        "reminder",
        WebviewUrl::App("reminder.html".into()),
    )
    .title(i18n::tr("橙子日程 · 提醒", lang))
    .inner_size(460.0, 550.0)
    .min_inner_size(400.0, 440.0)
    .resizable(true)
    .always_on_top(true)
    .visible(false)
    .build()
    .map_err(|e| e.to_string())?;
    window.show().map_err(|e| e.to_string())?;
    let _ = window.set_focus();
    Ok(())
}

pub fn apply_widget_layer(app: &AppHandle, on_top: bool) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("widget") {
        window.set_always_on_top(on_top).map_err(|e| e.to_string())?;
        if !on_top {
            send_to_bottom(&window)?;
        }
    }
    Ok(())
}

#[cfg(windows)]
pub fn send_to_bottom(window: &tauri::WebviewWindow) -> Result<(), String> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, HWND_BOTTOM, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    };

    let raw = window.hwnd().map_err(|e| e.to_string())?;
    let hwnd = HWND(raw.0);
    unsafe {
        SetWindowPos(
            hwnd,
            HWND_BOTTOM,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn send_to_bottom(_window: &tauri::WebviewWindow) -> Result<(), String> {
    Ok(())
}

#[cfg(windows)]
pub fn set_app_user_model_id(id: &str) {
    use windows::core::HSTRING;
    use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
    unsafe {
        let value = HSTRING::from(id);
        let _ = SetCurrentProcessExplicitAppUserModelID(&value);
    }
}

#[cfg(not(windows))]
pub fn set_app_user_model_id(_id: &str) {}

pub fn emit_to_main(app: &AppHandle, event: &str, payload: impl serde::Serialize + Clone) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.emit(event, payload);
    }
}
