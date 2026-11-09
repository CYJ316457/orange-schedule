use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tauri::Manager;

use crate::state::{now_ms, validate_state, AppData};

static DIR: OnceLock<PathBuf> = OnceLock::new();

pub fn init(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    // 1. 优先使用 WEEKLIGHT_DATA_DIR 环境变量
    let override_dir = std::env::var("WEEKLIGHT_DATA_DIR")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from);

    // 2. 检查程序所在目录下是否存在 data/ 目录或 .portable 标记文件（便携/免安装绿色模式）
    let portable_dir = if override_dir.is_none() {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .and_then(|dir| {
                let data = dir.join("data");
                if data.is_dir() || dir.join(".portable").is_file() {
                    Some(data)
                } else {
                    None
                }
            })
    } else {
        None
    };

    let is_custom = override_dir.is_some() || portable_dir.is_some();
    let dir = match override_dir.or(portable_dir) {
        Some(p) if p.is_absolute() => p,
        Some(p) => std::env::current_dir().map_err(|e| e.to_string())?.join(p),
        None => app.path().app_data_dir().map_err(|e| e.to_string())?,
    };
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let _ = DIR.set(dir.clone());
    if !is_custom {
        migrate_from_electron(&dir);
    }
    Ok(dir)
}

pub fn dir() -> PathBuf {
    DIR.get().cloned().unwrap_or_else(|| PathBuf::from("."))
}

/// Electron 版的 userData 目录名取决于 app.getName()，这里按候选逐个探测。
fn migrate_from_electron(target_dir: &Path) {
    let target = target_dir.join("plans.json");
    if target.exists() {
        return;
    }
    let Ok(appdata) = std::env::var("APPDATA") else {
        return;
    };
    let base = PathBuf::from(appdata);
    for name in ["weeklight", "橙子日程", "OrangeSchedule", "orange-schedule"] {
        let legacy = base.join(name).join("plans.json");
        if legacy.exists() {
            let _ = std::fs::copy(&legacy, &target);
            return;
        }
    }
}

pub fn load() -> (AppData, Option<String>) {
    let file = dir().join("plans.json");
    if !file.exists() {
        return (AppData::default(), None);
    }
    let raw = match std::fs::read_to_string(&file) {
        Ok(s) => s,
        Err(e) => return (AppData::default(), Some(e.to_string())),
    };
    let parsed: Result<AppData, String> = serde_json::from_str::<Value>(&raw)
        .map_err(|e| e.to_string())
        .and_then(|v| validate_state(&v).map_err(|k| k.to_string()));
    match parsed {
        Ok(data) => (data, None),
        Err(err) => {
            // 与 Electron 版一致：保留原文件，另存恢复副本
            let _ = std::fs::copy(
                &file,
                dir().join(format!("plans-recovery-{}.json", now_ms())),
            );
            (AppData::default(), Some(err))
        }
    }
}

pub fn persist(state: &AppData) -> Result<(), String> {
    let file = dir().join("plans.json");
    let tmp = dir().join("plans.json.tmp");
    let json = serde_json::to_string_pretty(state).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
    if file.exists() {
        let _ = std::fs::copy(&file, dir().join("plans.json.bak"));
    }
    std::fs::rename(&tmp, &file).map_err(|e| e.to_string())
}
