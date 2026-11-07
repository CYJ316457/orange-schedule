use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder,
};

use crate::i18n;
use crate::state::{AppState, Bounds};

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

// ---------------------------------------------------------------- widget placement

/// 显示器工作区（逻辑坐标）及其缩放比。逻辑坐标 = 物理像素 / 该显示器 scale。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Area {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub scale: f64,
}

fn area_of(monitor: &tauri::Monitor) -> Area {
    let scale = monitor.scale_factor().max(0.1);
    let wa = monitor.work_area();
    Area {
        x: wa.position.x as f64 / scale,
        y: wa.position.y as f64 / scale,
        w: wa.size.width as f64 / scale,
        h: wa.size.height as f64 / scale,
        scale,
    }
}

/// 与 Electron 版 widget-controller.cjs `bounds()` 一致：
/// 无记录时放在主屏右上角；记录的位置若与任何显示器重叠不足 40px，则回到主屏；
/// 最终把窗口夹紧在所选显示器的工作区内。返回（逻辑矩形, 所选显示器 scale）。
pub fn place_widget(saved: Option<&Bounds>, areas: &[Area], primary: &Area) -> (Bounds, f64) {
    let r = saved.cloned().unwrap_or(Bounds {
        width: 410.0,
        height: 650.0,
        x: primary.x + primary.w - 438.0,
        y: primary.y + 44.0,
    });
    let a = areas
        .iter()
        .find(|a| {
            r.x + r.width > a.x + 40.0
                && r.x < a.x + a.w - 40.0
                && r.y + r.height > a.y + 40.0
                && r.y < a.y + a.h - 40.0
        })
        .copied()
        .unwrap_or(*primary);
    let width = r.width.min(a.w).max(340.0);
    let height = r.height.min(a.h).max(380.0);
    let x = r.x.max(a.x).min(a.x + a.w - width).round();
    let y = r.y.max(a.y).min(a.y + a.h - height).round();
    (Bounds { x, y, width, height }, a.scale)
}

fn widget_prefs(app: &AppHandle) -> crate::state::WidgetPrefs {
    app.state::<AppState>()
        .data
        .lock()
        .map(|d| d.settings.widget.clone())
        .unwrap_or_default()
}

/// 读取便签当前位置/大小（逻辑坐标），与已保存值不同才落盘。
pub fn save_widget_bounds(app: &AppHandle) -> Result<(), String> {
    let Some(window) = app.get_webview_window("widget") else {
        return Ok(());
    };
    if window.is_minimized().unwrap_or(false) {
        return Ok(());
    }
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let pos = window
        .outer_position()
        .map_err(|e| e.to_string())?
        .to_logical::<f64>(scale);
    let size = window
        .inner_size()
        .map_err(|e| e.to_string())?
        .to_logical::<f64>(scale);
    let bounds = Bounds {
        x: pos.x.round(),
        y: pos.y.round(),
        width: size.width.round().clamp(340.0, 680.0),
        height: size.height.round().clamp(380.0, 1000.0),
    };
    let current = widget_prefs(app).bounds;
    let unchanged = current.as_ref().is_some_and(|c| {
        c.x == bounds.x && c.y == bounds.y && c.width == bounds.width && c.height == bounds.height
    });
    if unchanged {
        return Ok(());
    }
    crate::commands::mutate(app, |data| {
        let mut value =
            serde_json::to_value(&data.settings.widget).map_err(|e| e.to_string())?;
        if let Some(obj) = value.as_object_mut() {
            obj.insert(
                "bounds".into(),
                serde_json::to_value(&bounds).map_err(|e| e.to_string())?,
            );
        }
        data.settings.widget = crate::state::normalize_prefs(Some(&value));
        Ok(())
    })
    .map(|_| ())
}

static BOUNDS_GENERATION: AtomicU64 = AtomicU64::new(0);

/// move/resize 400ms 防抖保存（与 Electron 版一致）。
fn schedule_bounds_save(app: &AppHandle) {
    let generation = BOUNDS_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        if BOUNDS_GENERATION.load(Ordering::SeqCst) != generation {
            return;
        }
        if let Err(error) = save_widget_bounds(&handle) {
            emit_to_widget(
                &handle,
                i18n::t(
                    "便签位置未保存：{error}",
                    &handle.state::<AppState>().language(),
                    &[("error", error)],
                ),
            );
        }
    });
}

fn emit_to_widget(app: &AppHandle, message: String) {
    if let Some(window) = app.get_webview_window("widget") {
        let _ = window.emit("app-error", message);
    }
}

/// 打开便签并记住「已开启」，下次启动自动恢复（托盘/第二实例入口使用）。
pub fn open_widget_enabled(app: &AppHandle) -> Result<(), String> {
    crate::commands::set_widget_enabled(app, true)?;
    open_widget(app)
}

pub fn open_widget(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let prefs = widget_prefs(app);

    if let Some(window) = app.get_webview_window("widget") {
        if window.is_minimized().unwrap_or(false) {
            let _ = window.unminimize();
        }
        let _ = window.show();
        let _ = apply_widget_layer(app, prefs.on_top);
        return Ok(());
    }

    let window = WebviewWindowBuilder::new(
        app,
        "widget",
        WebviewUrl::App("widget.html".into()),
    )
    .title(i18n::tr("橙子日程 · 桌面便签", &state.language()))
    .inner_size(410.0, 650.0)
    .min_inner_size(340.0, 380.0)
    .max_inner_size(680.0, 1000.0)
    .decorations(false)
    .transparent(true)
    .resizable(true)
    .maximizable(false)
    .skip_taskbar(true)
    .always_on_top(prefs.on_top)
    .visible(false)
    .build()
    .map_err(|e| e.to_string())?;

    // 用物理像素定位，避免多显示器不同 DPI 时逻辑坐标换算歧义
    let monitors = app.available_monitors().unwrap_or_default();
    let areas: Vec<Area> = monitors.iter().map(area_of).collect();
    let primary = app
        .primary_monitor()
        .ok()
        .flatten()
        .map(|m| area_of(&m))
        .or_else(|| areas.first().copied());
    if let Some(primary) = primary {
        let (b, scale) = place_widget(prefs.bounds.as_ref(), &areas, &primary);
        let _ = window.set_size(PhysicalSize::new(
            (b.width * scale).round() as u32,
            (b.height * scale).round() as u32,
        ));
        let _ = window.set_position(PhysicalPosition::new(
            (b.x * scale).round() as i32,
            (b.y * scale).round() as i32,
        ));
    }

    let handle = app.clone();
    let target = window.clone();
    window.on_window_event(move |event| match event {
        tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_) => {
            schedule_bounds_save(&handle);
        }
        // 失焦后若未置顶，重新沉到其他窗口下方
        tauri::WindowEvent::Focused(false) => {
            if !widget_prefs(&handle).on_top {
                let _ = send_to_bottom(&target);
            }
        }
        // Alt+F4 等系统关闭：与便签 × 按钮一致——保存位置、记为关闭、隐藏
        tauri::WindowEvent::CloseRequested { api, .. } => {
            if !QUITTING.load(Ordering::SeqCst) {
                api.prevent_close();
                let _ = close_widget(&handle);
                let _ = crate::commands::set_widget_enabled(&handle, false);
            }
        }
        _ => {}
    });
    window.show().map_err(|e| e.to_string())?;
    if !prefs.on_top {
        let _ = send_to_bottom(&window);
    }
    Ok(())
}

/// 保存位置后隐藏（不销毁，再次打开更快，与 Electron 版一致）。
pub fn close_widget(app: &AppHandle) -> Result<(), String> {
    let _ = save_widget_bounds(app);
    if let Some(window) = app.get_webview_window("widget") {
        let _ = window.hide();
    }
    Ok(())
}

/// 彻底退出前保存便签位置。
pub fn quit(app: &AppHandle) {
    QUITTING.store(true, Ordering::SeqCst);
    let _ = save_widget_bounds(app);
    app.exit(0);
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

// ---------------------------------------------------------------- tray

fn build_tray_menu(app: &AppHandle) -> Result<Menu<tauri::Wry>, String> {
    let lang = app.state::<AppState>().language();
    let open = MenuItem::with_id(
        app,
        "open",
        i18n::tr("打开橙子日程", &lang),
        true,
        None::<&str>,
    )
    .map_err(|e| e.to_string())?;
    let widget = MenuItem::with_id(
        app,
        "widget",
        i18n::tr("桌面便签", &lang),
        true,
        None::<&str>,
    )
    .map_err(|e| e.to_string())?;
    let new = MenuItem::with_id(
        app,
        "new",
        i18n::tr("新建计划", &lang),
        true,
        None::<&str>,
    )
    .map_err(|e| e.to_string())?;
    let quit = MenuItem::with_id(
        app,
        "quit",
        i18n::tr("退出橙子日程（停止提醒）", &lang),
        true,
        None::<&str>,
    )
    .map_err(|e| e.to_string())?;
    let separator = PredefinedMenuItem::separator(app).map_err(|e| e.to_string())?;
    Menu::with_items(app, &[&open, &widget, &new, &separator, &quit]).map_err(|e| e.to_string())
}

pub fn create_tray(app: &AppHandle) -> Result<(), String> {
    let menu = build_tray_menu(app)?;
    let lang = app.state::<AppState>().language();
    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or("缺少应用图标")?;

    TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .tooltip(i18n::tr("橙子日程 · 提醒在后台运行", &lang))
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => {
                let _ = show_main(app);
            }
            "widget" => {
                let _ = open_widget_enabled(app);
            }
            "new" => {
                let _ = show_main(app);
                emit_to_main(app, "new-task", serde_json::Value::Null);
            }
            "quit" => quit(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let _ = show_main(tray.app_handle());
            }
        })
        .build(app)
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn refresh_tray(app: &AppHandle) -> Result<(), String> {
    let Some(tray) = app.tray_by_id("main-tray") else {
        return Ok(());
    };
    let lang = app.state::<AppState>().language();
    let menu = build_tray_menu(app)?;
    tray.set_menu(Some(menu)).map_err(|e| e.to_string())?;
    tray.set_tooltip(Some(i18n::tr("橙子日程 · 提醒在后台运行", &lang)))
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRIMARY: Area = Area { x: 0.0, y: 0.0, w: 1920.0, h: 1040.0, scale: 1.0 };
    const RIGHT: Area = Area { x: 1920.0, y: 0.0, w: 1280.0, h: 680.0, scale: 1.5 };

    fn b(x: f64, y: f64, width: f64, height: f64) -> Bounds {
        Bounds { x, y, width, height }
    }

    #[test]
    fn default_goes_to_primary_top_right() {
        let (r, s) = place_widget(None, &[PRIMARY], &PRIMARY);
        assert_eq!((r.x, r.y, r.width, r.height, s), (1482.0, 44.0, 410.0, 650.0, 1.0));
    }

    #[test]
    fn saved_position_on_secondary_monitor_is_kept() {
        let saved = b(2200.0, 20.0, 400.0, 600.0);
        let (r, s) = place_widget(Some(&saved), &[PRIMARY, RIGHT], &PRIMARY);
        assert_eq!((r.x, r.y, r.width, r.height, s), (2200.0, 20.0, 400.0, 600.0, 1.5));
    }

    #[test]
    fn offscreen_position_falls_back_to_primary_and_is_clamped() {
        // 副屏已拔掉：位置落在不存在的区域
        let saved = b(2200.0, 20.0, 400.0, 600.0);
        let (r, s) = place_widget(Some(&saved), &[PRIMARY], &PRIMARY);
        assert_eq!((r.x, r.y, s), (1520.0, 20.0, 1.0));
    }

    #[test]
    fn size_is_clamped_to_small_work_area() {
        let small = Area { x: 0.0, y: 0.0, w: 1024.0, h: 500.0, scale: 1.0 };
        let saved = b(100.0, 100.0, 500.0, 900.0);
        let (r, _) = place_widget(Some(&saved), &[small], &small);
        assert_eq!((r.y, r.height), (0.0, 500.0));
    }
}
