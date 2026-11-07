use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
use tauri_plugin_opener::OpenerExt;

use crate::i18n;
use crate::state::{clean_task, make_series, now_ms, AppData, AppState, MAX_TASKS};
use crate::store;

fn lang(app: &AppHandle) -> String {
    app.state::<AppState>().language()
}

fn packaged() -> bool {
    !cfg!(debug_assertions)
}

pub fn snapshot_json(app: &AppHandle) -> Value {
    let state = app.state::<AppState>();
    let language = state.language();
    let data = state.data.lock().map(|d| d.clone()).unwrap_or_default();
    let update = state
        .update
        .lock()
        .map(|u| u.clone())
        .unwrap_or_default();
    let startup_error = state
        .startup_error
        .lock()
        .map(|s| s.clone())
        .unwrap_or_default();

    let (autostart, autostart_error) = if packaged() {
        match app.autolaunch().is_enabled() {
            Ok(enabled) => (enabled, String::new()),
            Err(e) => (
                false,
                i18n::t(
                    "无法读取 Windows 开机启动状态：{error}",
                    &language,
                    &[("error", e.to_string())],
                ),
            ),
        }
    } else {
        (false, String::new())
    };

    let startup_text = if startup_error.is_empty() {
        String::new()
    } else {
        i18n::t(
            "计划文件无法读取，已保留原文件。请从设置导入备份。{error}",
            &language,
            &[("error", startup_error)],
        )
    };

    json!({
        "version": data.version,
        "tasks": data.tasks,
        "settings": data.settings,
        "history": data.history,
        "update": update,
        "dataDir": store::dir().display().to_string(),
        "startupError": startup_text,
        "autostart": autostart,
        "autostartError": autostart_error,
        "packaged": packaged(),
        "reminderMessage": crate::reminder::current(&data.settings.reminder_rotation, &language),
        "reminderMessageCount": crate::reminder::count(),
    })
}

fn emit_state(app: &AppHandle) -> Value {
    let snap = snapshot_json(app);
    let _ = app.emit("state", &snap);
    snap
}

pub(crate) fn mutate<F>(app: &AppHandle, f: F) -> Result<Value, String>
where
    F: FnOnce(&mut AppData) -> Result<(), String>,
{
    {
        let state = app.state::<AppState>();
        let mut data = state.data.lock().map_err(|e| e.to_string())?;
        let mut next = data.clone();
        f(&mut next)?;
        store::persist(&next)?;
        *data = next;
    }
    Ok(emit_state(app))
}

fn refresh_titles(app: &AppHandle) {
    let language = lang(app);
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.set_title(&i18n::tr("橙子日程 · 计划你的每一周", &language));
    }
    if let Some(w) = app.get_webview_window("widget") {
        let _ = w.set_title(&i18n::tr("橙子日程 · 桌面便签", &language));
    }
    if let Some(w) = app.get_webview_window("reminder") {
        let _ = w.set_title(&i18n::tr("橙子日程 · 提醒", &language));
    }
    let _ = crate::windows::refresh_tray(app);
}

// ---------------------------------------------------------------- commands

#[tauri::command]
pub fn get(app: AppHandle) -> Result<Value, String> {
    Ok(snapshot_json(&app))
}

#[tauri::command]
pub fn save_task(app: AppHandle, task: Value, repeat: Option<Value>) -> Result<Value, String> {
    let language = lang(&app);
    let input_id = task.get("id").and_then(|v| v.as_str()).map(|s| s.to_string());
    mutate(&app, |data| {
        let existing = input_id
            .as_deref()
            .and_then(|id| data.tasks.iter().find(|t| t.id == id).cloned());
        if input_id.is_some() && existing.is_none() {
            return Err(i18n::tr("计划已被删除，无法保存旧编辑内容", &language));
        }
        let cleaned = clean_task(&task, existing.as_ref())
            .map_err(|k| i18n::tr(k, &language))?;
        if existing.is_some() {
            for item in data.tasks.iter_mut() {
                if item.id == cleaned.id {
                    *item = cleaned.clone();
                    break;
                }
            }
            if cleaned.done {
                for h in data.history.iter_mut() {
                    if h.task_id == cleaned.id {
                        h.dismissed = true;
                    }
                }
            }
        } else {
            let series = make_series(cleaned, &repeat.unwrap_or_else(|| json!(1)))
                .map_err(|k| i18n::tr(k, &language))?;
            if data.tasks.len() + series.len() > MAX_TASKS {
                return Err(i18n::tr(
                    "计划总数不能超过 10000 项，请缩短重复范围或先导出归档",
                    &language,
                ));
            }
            data.tasks.extend(series);
        }
        Ok(())
    })
}

#[tauri::command]
pub fn preview_task_action(app: AppHandle, id: String, action: String) -> Result<Value, String> {
    let language = lang(&app);
    let state = app.state::<AppState>();
    let data = state.data.lock().map_err(|e| e.to_string())?;
    crate::series::preview_json(&data.tasks, &id, &action)
        .map_err(|k| i18n::tr(k, &language))
}

#[tauri::command]
pub fn apply_task_action(app: AppHandle, request: Value) -> Result<Value, String> {
    let language = lang(&app);
    mutate(&app, |data| {
        crate::series::apply_task_action(data, &request)
            .map_err(|k| i18n::tr(k, &language))
    })
}

#[tauri::command]
pub fn toggle_task(app: AppHandle, id: String) -> Result<Value, String> {
    let language = lang(&app);
    mutate(&app, |data| {
        let task = data
            .tasks
            .iter_mut()
            .find(|t| t.id == id)
            .ok_or_else(|| i18n::tr("计划不存在", &language))?;
        task.done = !task.done;
        if task.done {
            task.snooze_until = None;
            for h in data.history.iter_mut() {
                if h.task_id == id {
                    h.dismissed = true;
                }
            }
        }
        Ok(())
    })
}

#[tauri::command]
pub fn delete_task(app: AppHandle, id: String) -> Result<Value, String> {
    mutate(&app, |data| {
        data.tasks.retain(|t| t.id != id);
        data.history.retain(|h| h.task_id != id);
        Ok(())
    })
}

#[tauri::command]
pub fn move_task(
    app: AppHandle,
    id: String,
    date: String,
    time: Option<String>,
) -> Result<Value, String> {
    let language = lang(&app);
    mutate(&app, |data| {
        let old = data
            .tasks
            .iter()
            .find(|t| t.id == id)
            .cloned()
            .ok_or_else(|| i18n::tr("计划不存在", &language))?;
        let mut input = serde_json::to_value(&old).map_err(|e| e.to_string())?;
        if let Some(obj) = input.as_object_mut() {
            obj.insert("date".into(), json!(date));
            obj.insert(
                "time".into(),
                json!(time.clone().unwrap_or_else(|| old.time.clone())),
            );
            obj.insert("done".into(), json!(old.done));
        }
        let cleaned = clean_task(&input, Some(&old)).map_err(|k| i18n::tr(k, &language))?;
        for item in data.tasks.iter_mut() {
            if item.id == id {
                *item = cleaned.clone();
            }
        }
        for h in data.history.iter_mut() {
            if h.task_id == id {
                h.dismissed = true;
            }
        }
        Ok(())
    })
}

#[tauri::command]
pub fn reminder_action(app: AppHandle, id: String, action: String) -> Result<Value, String> {
    let language = lang(&app);
    mutate(&app, |data| {
        let Some(task) = data.tasks.iter_mut().find(|t| t.id == id) else {
            return Ok(());
        };
        match action.as_str() {
            "snooze" => task.snooze_until = Some(now_ms() + 600000),
            "done" => {
                task.done = true;
                task.snooze_until = None;
            }
            "dismiss" => {}
            _ => return Err(i18n::tr("操作无效", &language)),
        }
        for h in data.history.iter_mut() {
            if h.task_id == id {
                h.dismissed = true;
            }
        }
        Ok(())
    })
}

#[tauri::command]
pub fn settings(app: AppHandle, settings: Value) -> Result<Value, String> {
    let language = lang(&app);
    let obj = settings
        .as_object()
        .ok_or_else(|| i18n::tr("计划格式不正确", &language))?;

    if let Some(value) = obj.get("language").and_then(|v| v.as_str()) {
        if value != "zh-CN" && value != "en" {
            return Err(i18n::tr("不支持的界面语言", &language));
        }
    }
    if let Some(enabled) = obj.get("autostart").and_then(|v| v.as_bool()) {
        if !packaged() {
            return Err(i18n::tr("开机自动启动仅支持打包后的 Windows 应用", &language));
        }
        let manager = app.autolaunch();
        let result = if enabled {
            manager.enable()
        } else {
            manager.disable()
        };
        if let Err(e) = result {
            return Err(i18n::t(
                "Windows 未保存开机自动启动设置。请检查当前账户权限或安全软件是否拦截后重试。",
                &language,
                &[("error", e.to_string())],
            ));
        }
        if manager.is_enabled().unwrap_or(false) != enabled {
            return Err(i18n::tr(
                "Windows 未保存开机自动启动设置。请检查当前账户权限或安全软件是否拦截后重试。",
                &language,
            ));
        }
    }

    let changed_language = obj
        .get("language")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let snap = mutate(&app, |data| {
        if let Some(value) = obj.get("language").and_then(|v| v.as_str()) {
            data.settings.language = value.to_string();
        }
        if let Some(value) = obj.get("sound").and_then(|v| v.as_bool()) {
            data.settings.sound = value;
        }
        if let Some(value) = obj.get("popup").and_then(|v| v.as_bool()) {
            data.settings.popup = value;
        }
        if let Some(value) = obj.get("checkUpdates").and_then(|v| v.as_bool()) {
            data.settings.check_updates = value;
        }
        Ok(())
    })?;

    if changed_language.is_some() {
        refresh_titles(&app);
    }
    Ok(snap)
}

#[tauri::command]
pub async fn check_updates(app: AppHandle) -> Result<Value, String> {
    crate::update::run_check(&app, true).await
}

#[tauri::command]
pub fn dismiss_update(app: AppHandle) -> Result<Value, String> {
    let state = app.state::<AppState>();
    let snapshot = {
        let mut update = state.update.lock().map_err(|e| e.to_string())?;
        update.dismissed = true;
        update.clone()
    };
    let _ = app.emit("state", snapshot_json(&app));
    serde_json::to_value(snapshot).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn download_update(app: AppHandle) -> Result<bool, String> {
    let language = lang(&app);
    let state = app.state::<AppState>();
    let url = state
        .update
        .lock()
        .map_err(|e| e.to_string())?
        .release
        .as_ref()
        .map(|r| r.url.clone())
        .ok_or_else(|| i18n::tr("请先检查是否有可用更新", &language))?;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| e.to_string())?;
    Ok(true)
}

#[tauri::command]
pub fn open_update_page(app: AppHandle) -> Result<bool, String> {
    app.opener()
        .open_url(
            format!("{}/releases/latest", crate::update::REPOSITORY_URL),
            None::<&str>,
        )
        .map_err(|e| e.to_string())?;
    Ok(true)
}

#[tauri::command]
pub async fn test_reminder(app: AppHandle) -> Result<bool, String> {
    let language = lang(&app);
    let snap = mutate(&app, |data| {
        let (_, rotation) = crate::reminder::advance(&data.settings.reminder_rotation);
        data.settings.reminder_rotation = rotation;
        Ok(())
    })?;
    let quote = snap
        .get("reminderMessage")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let text = quote.get("text").and_then(|v| v.as_str()).unwrap_or("");
    let source = quote.get("source").and_then(|v| v.as_str()).unwrap_or("");
    let (sound, popup) = {
        let state = app.state::<AppState>();
        let data = state.data.lock().map_err(|e| e.to_string())?;
        (data.settings.sound, data.settings.popup)
    };

    use tauri_plugin_notification::NotificationExt;
    let body = format!(
        "{}\n{} — {}",
        i18n::tr("这是一条测试通知。", &language),
        text,
        source
    );
    let mut builder = app
        .notification()
        .builder()
        .title(i18n::tr("橙子日程提醒已准备好", &language))
        .body(body);
    if sound {
        builder = builder.sound("default");
    }
    let _ = builder.show();
    // 与 Electron 版一致：关闭「独立提醒窗口」时只发系统通知
    if popup {
        let _ = crate::windows::show_reminder(&app, &language);
    }
    Ok(true)
}

#[tauri::command]
pub fn next_reminder_message(app: AppHandle) -> Result<Value, String> {
    mutate(&app, |data| {
        let (_, rotation) = crate::reminder::advance(&data.settings.reminder_rotation);
        data.settings.reminder_rotation = rotation;
        Ok(())
    })
}

#[tauri::command]
pub async fn export_data(app: AppHandle) -> Result<Option<String>, String> {
    let language = lang(&app);
    let dialog = app.clone();
    let lang_dialog = language.clone();
    let name = format!(
        "{}-{}.json",
        i18n::tr("橙子日程备份", &language),
        chrono::Local::now().format("%Y-%m-%d")
    );
    let path = tauri::async_runtime::spawn_blocking(move || {
        dialog
            .dialog()
            .file()
            .add_filter(i18n::tr("JSON 备份", &lang_dialog), &["json"])
            .set_file_name(name)
            .blocking_save_file()
    })
    .await
    .map_err(|e| e.to_string())?;

    let Some(file) = path else {
        return Ok(None);
    };
    let Some(path) = file.into_path().ok() else {
        return Ok(None);
    };
    let data = app
        .state::<AppState>()
        .data
        .lock()
        .map_err(|e| e.to_string())?
        .clone();
    let json = serde_json::to_string_pretty(&data).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())?;
    Ok(Some(path.display().to_string()))
}

#[tauri::command]
pub async fn import_data(app: AppHandle) -> Result<Option<Value>, String> {
    let language = lang(&app);
    let dialog = app.clone();
    let lang_pick = language.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        dialog
            .dialog()
            .file()
            .add_filter(i18n::tr("JSON 备份", &lang_pick), &["json"])
            .blocking_pick_file()
    })
    .await
    .map_err(|e| e.to_string())?;

    let Some(file) = picked else {
        return Ok(None);
    };
    let Some(path) = file.into_path().ok() else {
        return Ok(None);
    };
    let meta = std::fs::metadata(&path).map_err(|e| e.to_string())?;
    if meta.len() > 20 * 1024 * 1024 {
        return Err(i18n::tr("备份文件超过 20 MB", &language));
    }
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let text = text.trim_start_matches('\u{feff}');
    let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let incoming = crate::state::validate_state(&value)
        .map_err(|k| i18n::tr(k, &language))?;

    let confirm_app = app.clone();
    let lang_confirm = language.clone();
    let message = format!(
        "{}\n\n{}",
        i18n::t(
            "导入 {count} 项计划？",
            &language,
            &[("count", incoming.tasks.len().to_string())],
        ),
        i18n::tr(
            "相同 ID 的计划将使用备份中的内容，其余现有计划保留。导入前会自动保存完整备份。",
            &language,
        )
    );
    let confirmed = tauri::async_runtime::spawn_blocking(move || {
        confirm_app
            .dialog()
            .message(message)
            .title(i18n::tr("导入橙子日程备份", &lang_confirm))
            // OkCancelCustom(确认, 取消)：第一个按钮返回 true
            .buttons(MessageDialogButtons::OkCancelCustom(
                i18n::tr("导入并合并", &lang_confirm),
                i18n::tr("取消", &lang_confirm),
            ))
            .blocking_show()
    })
    .await
    .map_err(|e| e.to_string())?;
    if !confirmed {
        return Ok(None);
    }

    let snap = mutate(&app, |data| {
        let backup = store::dir().join(format!("before-import-{}.json", now_ms()));
        if let Ok(json) = serde_json::to_string_pretty(&*data) {
            let _ = std::fs::write(backup, json);
        }
        let mut map: std::collections::HashMap<String, crate::state::Task> = data
            .tasks
            .iter()
            .cloned()
            .map(|t| (t.id.clone(), t))
            .collect();
        for mut task in incoming.tasks.clone() {
            if let Some(old) = map.get(&task.id) {
                if old.series_id.is_some() && task.series_id.is_none() {
                    task.series_id = old.series_id.clone();
                    task.series_index = old.series_index;
                }
            }
            map.insert(task.id.clone(), task);
        }
        if map.len() > MAX_TASKS {
            return Err(i18n::tr("合并后超过 10000 项上限", &language));
        }
        let mut tasks: Vec<crate::state::Task> = map.into_values().collect();
        tasks.sort_by(|a, b| (a.date.clone() + &a.time).cmp(&(b.date.clone() + &b.time)));
        crate::series::validate_series_tasks(&tasks)
            .map_err(|k| i18n::tr(k, &language))?;
        data.tasks = tasks;
        Ok(())
    })?;
    Ok(Some(snap))
}

#[tauri::command]
pub fn open_data(app: AppHandle) -> Result<bool, String> {
    app.opener()
        .open_path(store::dir().display().to_string(), None::<&str>)
        .map_err(|e| e.to_string())?;
    Ok(true)
}

#[tauri::command]
pub fn show_main(app: AppHandle) -> Result<bool, String> {
    crate::windows::show_main(&app)?;
    Ok(true)
}

/// 只在状态变化时落盘，避免每次打开/关闭便签都写文件。
pub(crate) fn set_widget_enabled(app: &AppHandle, enabled: bool) -> Result<Value, String> {
    let current = app
        .state::<AppState>()
        .data
        .lock()
        .map(|d| d.settings.widget.enabled)
        .map_err(|e| e.to_string())?;
    if current == enabled {
        return Ok(snapshot_json(app));
    }
    mutate(app, |data| {
        data.settings.widget.enabled = enabled;
        Ok(())
    })
}

#[tauri::command]
pub async fn open_widget(app: AppHandle) -> Result<Value, String> {
    crate::windows::open_widget_enabled(&app)?;
    Ok(snapshot_json(&app))
}

#[tauri::command]
pub fn close_widget(app: AppHandle) -> Result<Value, String> {
    crate::windows::close_widget(&app)?;
    set_widget_enabled(&app, false)
}

#[tauri::command]
pub fn widget_settings(app: AppHandle, patch: Value) -> Result<Value, String> {
    let language = lang(&app);
    let allowed = ["mode", "anchor", "category", "onTop", "theme", "showCompleted"];
    let on_top_patch = patch.get("onTop").and_then(|v| v.as_bool());

    let snap = mutate(&app, |data| {
        let mut current =
            serde_json::to_value(&data.settings.widget).map_err(|e| e.to_string())?;
        if let (Some(target), Some(source)) = (current.as_object_mut(), patch.as_object()) {
            for key in allowed {
                if let Some(value) = source.get(key) {
                    target.insert(key.to_string(), value.clone());
                }
            }
        }
        data.settings.widget = crate::state::normalize_prefs(Some(&current));
        if let Some(on_top) = on_top_patch {
            data.settings.widget.on_top = on_top;
        }
        Ok(())
    })?;

    if let Some(on_top) = on_top_patch {
        let _ = crate::windows::apply_widget_layer(&app, on_top);
    }
    let _ = language;
    Ok(snap)
}

#[tauri::command]
pub fn edit_task(app: AppHandle, id: String) -> Result<bool, String> {
    let language = lang(&app);
    {
        let state = app.state::<AppState>();
        let data = state.data.lock().map_err(|e| e.to_string())?;
        if !data.tasks.iter().any(|t| t.id == id) {
            return Err(i18n::tr("该计划已被删除", &language));
        }
    }
    crate::windows::show_main(&app)?;
    crate::windows::emit_to_main(&app, "edit-task", id);
    Ok(true)
}

#[tauri::command]
pub fn quit(app: AppHandle) {
    crate::windows::quit(&app);
}
