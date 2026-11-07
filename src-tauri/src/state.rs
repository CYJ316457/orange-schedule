use chrono::{Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Mutex;

pub const CATEGORIES: [&str; 3] = ["work", "study", "life"];
pub const REMINDERS: [i64; 8] = [-1, 0, 5, 10, 15, 30, 60, 1440];
pub const MAX_TASKS: usize = 10000;

pub fn now_ms() -> i64 {
    Local::now().timestamp_millis()
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct WidgetPrefs {
    pub enabled: bool,
    pub mode: String,
    pub anchor: Option<String>,
    pub category: String,
    pub on_top: bool,
    pub theme: String,
    pub show_completed: bool,
    pub bounds: Option<Bounds>,
}

impl Default for WidgetPrefs {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: "day".into(),
            anchor: None,
            category: "all".into(),
            on_top: false,
            theme: "sage".into(),
            show_completed: true,
            bounds: None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Rotation {
    pub seen: Vec<String>,
    pub current_id: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub language: String,
    pub sound: bool,
    pub popup: bool,
    pub check_updates: bool,
    pub widget: WidgetPrefs,
    pub reminder_rotation: Rotation,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: "zh-CN".into(),
            sound: true,
            popup: true,
            check_updates: true,
            widget: WidgetPrefs::default(),
            reminder_rotation: Rotation::default(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub title: String,
    pub date: String,
    pub time: String,
    pub duration: i64,
    pub reminder: i64,
    pub category: String,
    pub priority: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub done: bool,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub notified_at: Option<i64>,
    #[serde(default)]
    pub snooze_until: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series_index: Option<i64>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub id: String,
    pub task_id: String,
    pub at: i64,
    #[serde(default)]
    pub dismissed: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AppData {
    pub version: u32,
    pub tasks: Vec<Task>,
    pub settings: Settings,
    pub history: Vec<HistoryEntry>,
}

impl Default for AppData {
    fn default() -> Self {
        Self {
            version: 1,
            tasks: Vec::new(),
            settings: Settings::default(),
            history: Vec::new(),
        }
    }
}

pub struct AppState {
    pub data: Mutex<AppData>,
    pub update: Mutex<crate::update::UpdateSnapshot>,
    pub startup_error: Mutex<String>,
}

impl AppState {
    pub fn new(data: AppData, startup_error: String) -> Self {
        Self {
            data: Mutex::new(data),
            update: Mutex::new(crate::update::UpdateSnapshot::default()),
            startup_error: Mutex::new(startup_error),
        }
    }

    pub fn language(&self) -> String {
        self.data
            .lock()
            .map(|d| d.settings.language.clone())
            .unwrap_or_else(|_| "zh-CN".into())
    }
}

// ---------------------------------------------------------------- date helpers

pub fn date_valid(s: &str) -> bool {
    if s.len() != 10 {
        return false;
    }
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map(|d| d.format("%Y-%m-%d").to_string() == s)
        .unwrap_or(false)
}

pub fn time_valid(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 5 || b[2] != b':' {
        return false;
    }
    let (h1, h2, m1, m2) = (b[0], b[1], b[3], b[4]);
    if !h1.is_ascii_digit() || !h2.is_ascii_digit() || !m1.is_ascii_digit() || !m2.is_ascii_digit() {
        return false;
    }
    let hour = (h1 - b'0') * 10 + (h2 - b'0');
    let minute = (m1 - b'0') * 10 + (m2 - b'0');
    hour < 24 && minute < 60
}

pub fn at_ms(t: &Task) -> Option<i64> {
    let date = NaiveDate::parse_from_str(&t.date, "%Y-%m-%d").ok()?;
    let time = NaiveTime::parse_from_str(&t.time, "%H:%M").ok()?;
    let naive = NaiveDateTime::new(date, time);
    Local
        .from_local_datetime(&naive)
        .single()
        .map(|dt| dt.timestamp_millis())
}

pub fn id_valid(id: &str) -> bool {
    if id.is_empty() || id.len() > 80 {
        return false;
    }
    id.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

// ---------------------------------------------------------------- normalize

pub fn normalize_language(value: Option<&str>) -> String {
    match value {
        Some("en") => "en".into(),
        _ => "zh-CN".into(),
    }
}

pub fn normalize_prefs(raw: Option<&Value>) -> WidgetPrefs {
    let mut p = WidgetPrefs::default();
    let Some(obj) = raw.and_then(|v| v.as_object()) else {
        return p;
    };
    p.enabled = obj.get("enabled").and_then(|v| v.as_bool()) == Some(true);
    p.mode = if obj.get("mode").and_then(|v| v.as_str()) == Some("week") {
        "week".into()
    } else {
        "day".into()
    };
    p.anchor = obj
        .get("anchor")
        .and_then(|v| v.as_str())
        .filter(|s| date_valid(s))
        .map(|s| s.to_string());
    p.category = match obj.get("category").and_then(|v| v.as_str()) {
        Some(c @ ("work" | "study" | "life")) => c.to_string(),
        _ => "all".into(),
    };
    p.on_top = obj.get("onTop").and_then(|v| v.as_bool()) == Some(true);
    p.theme = match obj.get("theme").and_then(|v| v.as_str()) {
        Some(t @ ("cream" | "lavender")) => t.to_string(),
        _ => "sage".into(),
    };
    p.show_completed = obj.get("showCompleted").and_then(|v| v.as_bool()) != Some(false);
    if let Some(b) = obj.get("bounds").and_then(|v| v.as_object()) {
        let num = |k: &str| b.get(k).and_then(|v| v.as_f64());
        if let (Some(x), Some(y), Some(w), Some(h)) = (num("x"), num("y"), num("width"), num("height")) {
            if x.abs() < 50000.0
                && y.abs() < 50000.0
                && (340.0..=680.0).contains(&w)
                && (380.0..=1000.0).contains(&h)
            {
                p.bounds = Some(Bounds {
                    x: x.round(),
                    y: y.round(),
                    width: w.round(),
                    height: h.round(),
                });
            }
        }
    }
    p
}

pub fn normalize_rotation(raw: Option<&Value>) -> Rotation {
    let mut seen: Vec<String> = Vec::new();
    let mut current: Option<String> = None;
    if let Some(obj) = raw.and_then(|v| v.as_object()) {
        if let Some(arr) = obj.get("seen").and_then(|v| v.as_array()) {
            for item in arr {
                if let Some(id) = item.as_str() {
                    if crate::reminder::exists(id) && !seen.iter().any(|x| x == id) {
                        seen.push(id.to_string());
                    }
                }
            }
        }
        if let Some(id) = obj.get("currentId").and_then(|v| v.as_str()) {
            if crate::reminder::exists(id) {
                current = Some(id.to_string());
            }
        }
    }
    if let Some(id) = &current {
        if !seen.iter().any(|x| x == id) {
            seen.push(id.clone());
        }
    }
    let total = crate::reminder::count();
    if seen.len() > total {
        seen = seen.split_off(seen.len() - total);
    }
    Rotation {
        seen,
        current_id: current,
    }
}

pub fn normalize_settings(raw: Option<&Value>) -> Settings {
    let obj = raw.and_then(|v| v.as_object());
    let get_bool = |k: &str| obj.and_then(|o| o.get(k)).and_then(|v| v.as_bool());
    Settings {
        language: normalize_language(
            obj.and_then(|o| o.get("language")).and_then(|v| v.as_str()),
        ),
        sound: get_bool("sound") != Some(false),
        popup: get_bool("popup") != Some(false),
        check_updates: get_bool("checkUpdates") != Some(false),
        widget: normalize_prefs(obj.and_then(|o| o.get("widget"))),
        reminder_rotation: normalize_rotation(obj.and_then(|o| o.get("reminderRotation"))),
    }
}

// ---------------------------------------------------------------- validation

pub fn clean_task(input: &Value, existing: Option<&Task>) -> Result<Task, &'static str> {
    let obj = input.as_object().ok_or("计划格式不正确")?;
    let title = obj
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if title.is_empty() || title.chars().count() > 160 {
        return Err("请填写 1–160 字的计划名称");
    }
    let date = obj
        .get("date")
        .and_then(|v| v.as_str())
        .ok_or("请选择有效日期")?
        .to_string();
    if !date_valid(&date) {
        return Err("请选择有效日期");
    }
    let time = obj
        .get("time")
        .and_then(|v| v.as_str())
        .ok_or("请选择有效时间")?
        .to_string();
    if !time_valid(&time) {
        return Err("请选择有效时间");
    }
    let duration = obj.get("duration").and_then(|v| v.as_i64()).unwrap_or(-1);
    if !(15..=1440).contains(&duration) {
        return Err("时长须为 15–1440 分钟");
    }
    let reminder = obj.get("reminder").and_then(|v| v.as_i64()).unwrap_or(i64::MIN);
    if !REMINDERS.contains(&reminder) {
        return Err("提醒时间不正确");
    }
    let category = obj
        .get("category")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if !CATEGORIES.contains(&category.as_str()) {
        return Err("请选择有效分类");
    }
    let priority = if obj.get("priority").and_then(|v| v.as_str()) == Some("high") {
        "high"
    } else {
        "normal"
    };
    let notes: String = obj
        .get("notes")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .chars()
        .take(5000)
        .collect();
    let done = obj.get("done").and_then(|v| v.as_bool()).unwrap_or(false);

    let mut task = Task {
        id: existing
            .map(|e| e.id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        title,
        date,
        time,
        duration,
        reminder,
        category,
        priority: priority.into(),
        notes,
        done,
        created_at: existing
            .map(|e| if e.created_at > 0 { e.created_at } else { 0 })
            .unwrap_or_else(now_ms),
        updated_at: now_ms(),
        notified_at: None,
        snooze_until: None,
        series_id: None,
        series_index: None,
    };

    if let Some(old) = existing {
        if old.date == task.date && old.time == task.time && old.reminder == task.reminder {
            task.notified_at = old.notified_at;
            task.snooze_until = old.snooze_until;
        }
        if old.series_id.is_some() || old.series_index.is_some() {
            task.series_id = old.series_id.clone();
            task.series_index = old.series_index;
            crate::series::validate_series_tasks(std::slice::from_ref(&task))?;
        }
    }
    Ok(task)
}

pub fn make_series(task: Task, repeat: &Value) -> Result<Vec<Task>, &'static str> {
    let (count, step, weekdays, single) = match repeat {
        Value::Number(n) => {
            let c = n.as_i64().ok_or("重复次数不正确")?;
            if ![1, 4, 8, 12].contains(&c) {
                return Err("重复次数不正确");
            }
            (c, 7i64, false, c == 1)
        }
        Value::Object(o) => {
            let freq = o
                .get("frequency")
                .and_then(|v| v.as_str())
                .ok_or("重复设置不正确")?;
            if freq != "daily" && freq != "weekdays" {
                return Err("重复设置不正确");
            }
            let until = o
                .get("until")
                .and_then(|v| v.as_str())
                .ok_or("请选择有效的重复结束日期")?;
            if !date_valid(until) {
                return Err("请选择有效的重复结束日期");
            }
            if until < task.date.as_str() {
                return Err("重复结束日期不能早于开始日期");
            }
            let start = NaiveDate::parse_from_str(&task.date, "%Y-%m-%d").map_err(|_| "请选择有效日期")?;
            let end = NaiveDate::parse_from_str(until, "%Y-%m-%d")
                .map_err(|_| "请选择有效的重复结束日期")?;
            let days = (end - start).num_days() + 1;
            let weekdays = freq == "weekdays";
            let mut count = days;
            if weekdays {
                use chrono::Datelike;
                let mut c = (days / 7) * 5;
                let first = start.weekday().num_days_from_monday() as i64;
                for i in 0..(days % 7) {
                    let w = (first + i) % 7;
                    if w != 5 && w != 6 {
                        c += 1;
                    }
                }
                count = c;
                if count == 0 {
                    return Err("所选日期范围内没有工作日，请调整日期");
                }
            }
            if !(1..=10000).contains(&count) {
                return Err(if weekdays {
                    "工作日重复最多生成 10000 项，请缩短日期范围"
                } else {
                    "每日重复最多生成 10000 项，请缩短日期范围"
                });
            }
            (count, 1i64, weekdays, false)
        }
        _ => return Err("重复设置不正确"),
    };

    let start = NaiveDate::parse_from_str(&task.date, "%Y-%m-%d").map_err(|_| "请选择有效日期")?;
    let series_id = if single {
        None
    } else {
        Some(uuid::Uuid::new_v4().to_string())
    };
    let mut result: Vec<Task> = Vec::new();
    let mut offset: i64 = 0;
    while (result.len() as i64) < count {
        if offset > 200000 {
            return Err("重复日期超出支持范围");
        }
        let day = start + chrono::Duration::days(offset);
        if weekdays {
            use chrono::Datelike;
            let w = day.weekday().num_days_from_monday();
            if w >= 5 {
                offset += step;
                continue;
            }
        }
        let mut t = task.clone();
        if !result.is_empty() {
            t.id = uuid::Uuid::new_v4().to_string();
        }
        t.date = day.format("%Y-%m-%d").to_string();
        if let Some(sid) = &series_id {
            t.series_id = Some(sid.clone());
            t.series_index = Some(result.len() as i64);
        }
        result.push(t);
        offset += step;
    }
    Ok(result)
}

pub fn validate_state(raw: &Value) -> Result<AppData, &'static str> {
    let obj = raw.as_object().ok_or("不是有效的橙子日程备份（最多 10000 项）")?;
    if obj.get("version").and_then(|v| v.as_u64()) != Some(1) {
        return Err("不是有效的橙子日程备份（最多 10000 项）");
    }
    let arr = obj
        .get("tasks")
        .and_then(|v| v.as_array())
        .ok_or("不是有效的橙子日程备份（最多 10000 项）")?;
    if arr.len() > MAX_TASKS {
        return Err("不是有效的橙子日程备份（最多 10000 项）");
    }
    let mut ids = std::collections::HashSet::new();
    let mut tasks = Vec::with_capacity(arr.len());
    for item in arr {
        let id = item
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or("备份包含无效或重复的计划 ID")?;
        if !id_valid(id) || !ids.insert(id.to_string()) {
            return Err("备份包含无效或重复的计划 ID");
        }
        let existing = clean_task(item, None).map_err(|_| "备份包含无效或重复的计划 ID")?;
        for key in ["notifiedAt", "snoozeUntil"] {
            if let Some(v) = item.get(key) {
                if !v.is_null() {
                    match v.as_i64() {
                        Some(n) if n >= 0 => {}
                        _ => return Err("备份中的提醒状态无效"),
                    }
                }
            }
        }
        tasks.push(existing);
    }
    crate::series::validate_series_tasks(&tasks)?;

    let mut history: Vec<HistoryEntry> = Vec::new();
    if let Some(arr) = obj.get("history").and_then(|v| v.as_array()) {
        for item in arr {
            let Some(o) = item.as_object() else { continue };
            let (Some(id), Some(task_id), Some(at)) = (
                o.get("id").and_then(|v| v.as_str()),
                o.get("taskId").and_then(|v| v.as_str()),
                o.get("at").and_then(|v| v.as_i64()),
            ) else {
                continue;
            };
            history.push(HistoryEntry {
                id: id.to_string(),
                task_id: task_id.to_string(),
                at,
                dismissed: o.get("dismissed").and_then(|v| v.as_bool()).unwrap_or(false),
            });
        }
    }
    if history.len() > 300 {
        history = history.split_off(history.len() - 300);
    }

    Ok(AppData {
        version: 1,
        tasks,
        settings: normalize_settings(obj.get("settings")),
        history,
    })
}

pub fn due_tasks(tasks: &[Task], now: i64) -> Vec<Task> {
    let mut out: Vec<Task> = tasks
        .iter()
        .filter(|t| {
            if t.done || t.reminder < 0 {
                return false;
            }
            match t.snooze_until {
                Some(s) if s > 0 => s <= now,
                _ => t.notified_at.is_none() && at_ms(t).map(|a| a - t.reminder * 60000 <= now).unwrap_or(false),
            }
        })
        .cloned()
        .collect();
    out.sort_by_key(|t| at_ms(t).unwrap_or(i64::MAX));
    out
}

pub fn widget_enabled(state: &AppState) -> bool {
    state
        .data
        .lock()
        .map(|d| d.settings.widget.enabled)
        .unwrap_or(false)
}
