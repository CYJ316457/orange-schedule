use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::i18n;
use crate::state::{at_ms, due_tasks, now_ms, AppState, HistoryEntry, Task};

pub fn spawn(app: &AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            if let Err(message) = run_tick(&handle) {
                let _ = handle.emit("app-error", message);
            }
        }
    });
}

fn run_tick(app: &AppHandle) -> Result<(), String> {
    if let Err(message) = crate::commands::reload_from_disk(app) {
        let _ = app.emit("app-error", message);
    }

    let state = app.state::<AppState>();
    let lang = state.language();
    let now = now_ms();

    let due: Vec<Task> = {
        let mut data = state.data.lock().map_err(|e| e.to_string())?;
        let due = due_tasks(&data.tasks, now);
        if due.is_empty() {
            return Ok(());
        }
        let mut next = data.clone();
        let (_, rotation) = crate::reminder::advance(&next.settings.reminder_rotation);
        next.settings.reminder_rotation = rotation;

        for task in &due {
            if let Some(t) = next.tasks.iter_mut().find(|t| t.id == task.id) {
                t.notified_at = Some(now);
                t.snooze_until = None;
            }
            next.history
                .retain(|h| h.task_id != task.id || h.dismissed);
            next.history.push(HistoryEntry {
                id: uuid::Uuid::new_v4().to_string(),
                task_id: task.id.clone(),
                at: now,
                dismissed: false,
            });
        }
        if next.history.len() > 300 {
            next.history = next.history.split_off(next.history.len() - 300);
        }
        crate::store::persist(&next)?;
        *data = next;
        due
    };

    let snap = crate::commands::snapshot_json(app);
    let _ = app.emit("state", &snap);

    let (sound, popup) = {
        let data = state.data.lock().map_err(|e| e.to_string())?;
        (data.settings.sound, data.settings.popup)
    };

    let short = |text: &str| -> String {
        let chars: Vec<char> = text.chars().collect();
        if chars.len() > 24 {
            format!("{}…", chars[..24].iter().collect::<String>())
        } else {
            text.to_string()
        }
    };

    let (title, details) = if due.len() > 1 {
        let names: Vec<String> = due.iter().take(3).map(|t| short(&t.title)).collect();
        (
            i18n::t(
                "{count} 项计划等待处理",
                &lang,
                &[("count", due.len().to_string())],
            ),
            names.join(" · "),
        )
    } else {
        let task = &due[0];
        let key = if at_ms(task).map(|at| at < now).unwrap_or(false) {
            "计划时间已到"
        } else {
            "即将开始"
        };
        (
            task.title.clone(),
            format!("{} {} · {}", task.date, task.time, i18n::tr(key, &lang)),
        )
    };

    let quote = {
        let data = state.data.lock().map_err(|e| e.to_string())?;
        crate::reminder::current(&data.settings.reminder_rotation, &lang)
    };
    let quote_text = quote.get("text").and_then(|v| v.as_str()).unwrap_or("");
    let quote_source = quote.get("source").and_then(|v| v.as_str()).unwrap_or("");
    let body = if quote_text.is_empty() {
        details
    } else {
        format!("{}\n{} — {}", details, quote_text, quote_source)
    };

    let mut builder = app.notification().builder().title(title).body(body);
    if sound {
        builder = builder.sound("default");
    }
    let _ = builder.show();

    if popup {
        let _ = crate::windows::show_reminder(app, &lang);
    }
    Ok(())
}
