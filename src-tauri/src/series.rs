use serde::Serialize;
use serde_json::Value;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};

use crate::state::{id_valid, AppData, Task};

const ACTIONS: [&str; 3] = ["complete", "reopen", "delete"];

pub fn validate_series_tasks(tasks: &[Task]) -> Result<(), &'static str> {
    let mut seen: HashSet<String> = HashSet::new();
    for task in tasks {
        if task.series_id.is_none() && task.series_index.is_none() {
            continue;
        }
        let series_id = task.series_id.as_deref().unwrap_or("");
        let index = task.series_index.unwrap_or(-1);
        if !id_valid(series_id) || !(0..=9999).contains(&index) {
            return Err("重复计划分组信息无效");
        }
        let key = format!("{}:{}", series_id, index);
        if !seen.insert(key) {
            return Err("重复计划组中存在相同的事项序号");
        }
    }
    Ok(())
}

#[derive(Serialize, Clone)]
struct DisplayTask {
    id: String,
    title: String,
    date: String,
    time: String,
    done: bool,
}

fn display(t: &Task) -> DisplayTask {
    DisplayTask {
        id: t.id.clone(),
        title: t.title.clone(),
        date: t.date.clone(),
        time: t.time.clone(),
        done: t.done,
    }
}

fn schedule(t: &Task) -> String {
    format!("{}T{}", t.date, t.time)
}

fn token<T: Serialize>(value: &T) -> String {
    let text = serde_json::to_string(value).unwrap_or_default();
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

#[derive(Serialize)]
struct PreviewBody {
    id: String,
    action: String,
    source: String,
    tasks: Vec<DisplayTask>,
}

#[derive(Serialize)]
struct SingleBody<'a> {
    id: &'a str,
    action: &'a str,
    task: DisplayTask,
}

pub struct Preview {
    pub source: String,
    pub items: Vec<Task>,
    pub token: String,
    pub single_token: String,
    pub task: Task,
}

pub fn preview_task_action(tasks: &[Task], id: &str, action: &str) -> Result<Preview, &'static str> {
    if !ACTIONS.contains(&action) {
        return Err("计划操作无效");
    }
    let task = tasks
        .iter()
        .find(|t| t.id == id)
        .cloned()
        .ok_or("计划已被删除，请刷新后重试")?;

    let mut source = "single".to_string();
    let mut matches: Vec<Task> = vec![task.clone()];

    if let Some(series_id) = &task.series_id {
        source = "series".into();
        let base_index = task.series_index.unwrap_or(0);
        matches = tasks
            .iter()
            .filter(|t| t.series_id.as_deref() == Some(series_id.as_str()) && t.series_index.unwrap_or(0) >= base_index)
            .cloned()
            .collect();
        matches.sort_by_key(|t| t.series_index.unwrap_or(0));
    } else if task.created_at > 0 {
        let candidates: Vec<Task> = tasks
            .iter()
            .filter(|t| {
                t.series_id.is_none()
                    && t.series_index.is_none()
                    && t.created_at == task.created_at
                    && t.title == task.title
                    && t.time == task.time
                    && t.duration == task.duration
                    && t.category == task.category
                    && t.reminder == task.reminder
                    && t.priority == task.priority
                    && t.notes == task.notes
            })
            .cloned()
            .collect();
        if candidates.len() > 1 {
            source = "legacy".into();
            let base = schedule(&task);
            matches = candidates.into_iter().filter(|t| schedule(t) >= base).collect();
            matches.sort_by(|a, b| {
                schedule(a)
                    .cmp(&schedule(b))
                    .then_with(|| a.id.cmp(&b.id))
            });
        }
    }

    let items: Vec<DisplayTask> = matches.iter().map(display).collect();
    let body = PreviewBody {
        id: id.to_string(),
        action: action.to_string(),
        source: source.clone(),
        tasks: items,
    };
    let single = SingleBody {
        id,
        action,
        task: display(&task),
    };

    Ok(Preview {
        source,
        token: token(&body),
        single_token: token(&single),
        items: matches,
        task,
    })
}

pub fn preview_json(tasks: &[Task], id: &str, action: &str) -> Result<Value, &'static str> {
    let p = preview_task_action(tasks, id, action)?;
    Ok(serde_json::json!({
        "task": display(&p.task),
        "source": p.source,
        "followingCount": p.items.len(),
        "items": p.items.iter().map(display).collect::<Vec<_>>(),
        "token": p.token,
        "singleToken": p.single_token,
    }))
}

pub fn apply_task_action(state: &mut AppData, request: &Value) -> Result<(), &'static str> {
    let obj = request.as_object().ok_or("请选择有效的操作范围")?;
    let id = obj.get("id").and_then(|v| v.as_str()).ok_or("请选择有效的操作范围")?;
    let action = obj
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or("计划操作无效")?;
    let scope = obj
        .get("scope")
        .and_then(|v| v.as_str())
        .ok_or("请选择有效的操作范围")?;
    if scope != "single" && scope != "following" {
        return Err("请选择有效的操作范围");
    }
    let req_token = obj.get("token").and_then(|v| v.as_str()).unwrap_or("");

    let preview = preview_task_action(&state.tasks, id, action)?;
    if scope == "following" && req_token != preview.token {
        return Err("事项已发生变化，请重新确认操作范围");
    }
    if scope == "single" && action == "delete" && req_token != preview.single_token {
        return Err("事项已发生变化，请重新确认删除");
    }

    let ids: HashSet<String> = if scope == "following" {
        preview.items.iter().map(|t| t.id.clone()).collect()
    } else {
        [id.to_string()].into_iter().collect()
    };

    if action == "delete" {
        state.tasks.retain(|t| !ids.contains(&t.id));
        state.history.retain(|h| !ids.contains(&h.task_id));
    } else {
        let done = action == "complete";
        let now = crate::state::now_ms();
        for task in state.tasks.iter_mut() {
            if ids.contains(&task.id) {
                task.done = done;
                task.snooze_until = None;
                task.updated_at = now;
            }
        }
        if done {
            for h in state.history.iter_mut() {
                if ids.contains(&h.task_id) {
                    h.dismissed = true;
                }
            }
        }
    }
    Ok(())
}
