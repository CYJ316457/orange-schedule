use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tauri::Manager;

use crate::state::{now_ms, validate_state, AppData};

static DIR: OnceLock<PathBuf> = OnceLock::new();

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct FileSignature {
    modified_ns: u128,
    len: u64,
}

static KNOWN_SIGNATURE: std::sync::Mutex<Option<FileSignature>> =
    std::sync::Mutex::new(None);

fn file_signature(path: &Path) -> Option<FileSignature> {
    let meta = std::fs::metadata(path).ok()?;
    let modified_ns = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some(FileSignature {
        modified_ns,
        len: meta.len(),
    })
}

fn changed_since(known: &mut Option<FileSignature>, path: &Path) -> bool {
    let current = file_signature(path);
    let changed = current.is_some() && *known != current;
    *known = current;
    changed
}

fn read_if_changed(
    known: &mut Option<FileSignature>,
    path: &Path,
) -> Option<(AppData, Option<String>)> {
    if !changed_since(known, path) {
        return None;
    }
    Some(load_from(path))
}

pub fn reload_from_disk() -> Option<(AppData, Option<String>)> {
    let path = dir().join("plans.json");
    let mut known = KNOWN_SIGNATURE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    read_if_changed(&mut known, &path)
}

fn mark_known() {
    let sig = file_signature(&dir().join("plans.json"));
    *KNOWN_SIGNATURE
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = sig;
}

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
    mark_known();
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
    load_from(&dir().join("plans.json"))
}

fn load_from(path: &Path) -> (AppData, Option<String>) {
    if !path.exists() {
        return (AppData::default(), None);
    }
    let raw = match std::fs::read_to_string(path) {
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
                path,
                path.with_file_name(format!("plans-recovery-{}.json", now_ms())),
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
    std::fs::rename(&tmp, &file).map_err(|e| e.to_string())?;
    mark_known();
    Ok(())
}

#[cfg(test)]
mod sync_tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "weeklight-store-sync-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn state_json(tasks: &str) -> String {
        format!(r#"{{"version":1,"tasks":{tasks},"settings":{{}},"history":[]}}"#)
    }

    const TASK: &str = r#"{"id":"external-1","title":"提交代码","date":"2026-11-10","time":"23:00","duration":60,"reminder":10,"category":"life","priority":"normal","notes":"","done":false}"#;

    fn cleanup(dir: &Path) {
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn detects_an_external_write_and_returns_the_new_state() {
        let dir = temp_dir();
        let file = dir.join("plans.json");
        std::fs::write(&file, state_json("[]")).expect("write initial state");

        // init() records the startup signature; unchanged files are ignored.
        let mut known = file_signature(&file);
        assert!(file_signature(&file).is_some());
        assert!(read_if_changed(&mut known, &file).is_none());

        // An external process (for example the MCP server) writes a new task.
        std::fs::write(&file, state_json(&format!("[{TASK}]"))).expect("write external state");

        let (data, error) =
            read_if_changed(&mut known, &file).expect("external change must be detected");
        assert!(error.is_none(), "valid external state must load cleanly");
        assert_eq!(data.tasks.len(), 1);
        assert_eq!(data.tasks[0].title, "提交代码");

        // Each change is consumed exactly once.
        assert!(read_if_changed(&mut known, &file).is_none());
        cleanup(&dir);
    }

    #[test]
    fn our_own_persist_does_not_trigger_a_reload() {
        let dir = temp_dir();
        let file = dir.join("plans.json");
        std::fs::write(&file, state_json("[]")).expect("write initial state");
        let mut known = file_signature(&file);
        assert!(read_if_changed(&mut known, &file).is_none());

        // persist() refreshes the known signature after writing.
        std::fs::write(&file, state_json(&format!("[{TASK}]"))).expect("write own state");
        known = file_signature(&file);

        assert!(read_if_changed(&mut known, &file).is_none());
        cleanup(&dir);
    }

    #[test]
    fn a_broken_external_file_keeps_the_in_memory_state() {
        let dir = temp_dir();
        let file = dir.join("plans.json");
        std::fs::write(&file, state_json("[]")).expect("write initial state");
        let mut known = file_signature(&file);

        std::fs::write(&file, "not valid json").expect("write broken state");
        let (data, error) =
            read_if_changed(&mut known, &file).expect("external change must still be detected");
        assert!(error.is_some(), "a broken file must report an error");
        assert!(data.tasks.is_empty(), "the fallback state must not discard data");

        // The broken file is not retried on every tick.
        assert!(read_if_changed(&mut known, &file).is_none());
        cleanup(&dir);
    }

    #[test]
    fn a_missing_file_is_ignored() {
        let dir = temp_dir();
        let file = dir.join("plans.json");
        std::fs::write(&file, state_json("[]")).expect("write initial state");
        let mut known = file_signature(&file);

        std::fs::remove_file(&file).expect("remove state file");
        assert!(read_if_changed(&mut known, &file).is_none());
        cleanup(&dir);
    }
}
