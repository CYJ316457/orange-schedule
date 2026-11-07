use serde::Serialize;
use serde_json::Value;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

use crate::i18n;
use crate::state::AppState;

pub const REPOSITORY_URL: &str = "https://github.com/OrangeCatzhang/orange-schedule";
const MANIFEST_URL: &str =
    "https://raw.githubusercontent.com/OrangeCatzhang/orange-schedule/main/docs/updates/latest.json";
const RELEASES_API: &str =
    "https://api.github.com/repos/OrangeCatzhang/orange-schedule/releases/latest";
const MAX_NOTES: usize = 10000;
const MANUAL_SUFFIX: &str = "请稍后重试，或打开官方下载页手动下载。";

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    pub version: String,
    pub url: String,
    pub release_url: String,
    pub notes: String,
    pub published_at: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSnapshot {
    pub phase: String,
    pub current_version: String,
    pub release: Option<Release>,
    pub checked_at: Option<i64>,
    pub error: String,
    pub dismissed: bool,
}

impl Default for UpdateSnapshot {
    fn default() -> Self {
        Self {
            phase: "idle".into(),
            current_version: env!("CARGO_PKG_VERSION").to_string(),
            release: None,
            checked_at: None,
            error: String::new(),
            dismissed: false,
        }
    }
}

fn parse_version(value: &str) -> Option<[u64; 3]> {
    if value.len() > 64 {
        return None;
    }
    let parts: Vec<&str> = value.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let mut out = [0u64; 3];
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() || (part.len() > 1 && part.starts_with('0')) {
            return None;
        }
        if !part.bytes().all(|b| b.is_ascii_digit()) || part.len() > 9 {
            return None;
        }
        out[i] = part.parse().ok()?;
    }
    Some(out)
}

pub fn compare_versions(left: &str, right: &str) -> Option<i32> {
    let a = parse_version(left)?;
    let b = parse_version(right)?;
    for i in 0..3 {
        if a[i] != b[i] {
            return Some(if a[i] > b[i] { 1 } else { -1 });
        }
    }
    Some(0)
}

fn parse_release(release: &Value, current: &str, lang: &str) -> Result<Option<Release>, String> {
    if parse_version(current).is_none() {
        return Err(i18n::tr("当前应用版本号格式不正确，无法检查更新。", lang));
    }
    let obj = release
        .as_object()
        .ok_or_else(|| i18n::tr("更新信息格式不正确，请稍后重试，或打开官方下载页手动下载。", lang))?;
    if obj.get("draft").and_then(|v| v.as_bool()) != Some(false)
        || obj.get("prerelease").and_then(|v| v.as_bool()) != Some(false)
    {
        return Err(i18n::tr("更新信息不是已发布的稳定版本，请稍后重试。", lang));
    }
    let tag = obj.get("tag_name").and_then(|v| v.as_str()).unwrap_or("");
    let version = tag.strip_prefix('v').unwrap_or("");
    if parse_version(version).is_none() {
        return Err(i18n::tr("更新版本号格式不正确，无法确认可用更新。", lang));
    }
    let filename = format!("OrangeSchedule-Setup-{}.exe", version);
    let url = format!("{}/releases/download/{}/{}", REPOSITORY_URL, tag, filename);
    let installer = obj
        .get("assets")
        .and_then(|v| v.as_array())
        .and_then(|arr| {
            arr.iter().find(|a| {
                a.get("name").and_then(|v| v.as_str()) == Some(filename.as_str())
            })
        });
    let Some(installer) = installer else {
        return Err(i18n::tr("该版本的 Windows 安装包尚未准备好，请稍后重试。", lang));
    };
    let state_ok = installer.get("state").and_then(|v| v.as_str()) == Some("uploaded");
    let size_ok = installer
        .get("size")
        .and_then(|v| v.as_i64())
        .map(|s| s > 0)
        .unwrap_or(false);
    if !state_ok || !size_ok {
        return Err(i18n::tr("该版本的 Windows 安装包尚未准备好，请稍后重试。", lang));
    }
    if installer.get("browser_download_url").and_then(|v| v.as_str()) != Some(url.as_str()) {
        return Err(i18n::tr(
            "更新安装包地址与官方发布地址不一致，已停止提供下载。",
            lang,
        ));
    }
    if compare_versions(version, current).unwrap_or(0) <= 0 {
        return Ok(None);
    }
    let notes: String = obj
        .get("body")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .chars()
        .take(MAX_NOTES)
        .collect();
    let published = obj
        .get("published_at")
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| {
            d.with_timezone(&chrono::Utc)
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        });
    Ok(Some(Release {
        version: version.to_string(),
        url,
        release_url: format!("{}/releases/tag/{}", REPOSITORY_URL, tag),
        notes,
        published_at: published,
    }))
}

enum FetchError {
    Retryable(String),
    Fatal(String),
}

fn fetch_json(url: &str, api: bool, lang: &str) -> Result<Value, FetchError> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(5))
        .timeout(Duration::from_secs(6))
        .build();
    let ua = format!("OrangeSchedule/{}", env!("CARGO_PKG_VERSION"));
    let req = agent
        .get(url)
        .set("Accept", if api { "application/vnd.github+json" } else { "application/json" })
        .set("User-Agent", &ua);
    let req = if api {
        req.set("X-GitHub-Api-Version", "2026-03-10")
    } else {
        req
    };
    match req.call() {
        Ok(resp) => resp.into_json::<Value>().map_err(|_| {
            FetchError::Fatal(i18n::tr(
                "更新信息无法解析，请稍后重试，或打开官方下载页手动下载。",
                lang,
            ))
        }),
        Err(ureq::Error::Status(404, _)) => Err(FetchError::Retryable(i18n::tr(
            "暂未找到可用的公开版本（404），请稍后重试，或打开官方下载页手动下载。",
            lang,
        ))),
        Err(ureq::Error::Status(429, resp)) => {
            let remaining = resp.header("x-ratelimit-remaining").unwrap_or("");
            let _ = remaining;
            Err(FetchError::Retryable(i18n::tr(
                "更新服务暂时限制了当前网络的请求，请稍后重试，或打开官方下载页手动下载。",
                lang,
            )))
        }
        Err(ureq::Error::Status(403, resp)) => {
            let limited = resp.header("x-ratelimit-remaining") == Some("0")
                || resp.header("retry-after").is_some();
            Err(FetchError::Retryable(if limited {
                i18n::tr(
                    "更新服务暂时限制了当前网络的请求，请稍后重试，或打开官方下载页手动下载。",
                    lang,
                )
            } else {
                i18n::tr(
                    "更新服务暂时拒绝访问（403），请稍后重试，或打开官方下载页手动下载。",
                    lang,
                )
            }))
        }
        Err(ureq::Error::Status(code, _)) => Err(FetchError::Retryable(i18n::t(
            "更新服务暂时不可用（HTTP {status}），{detail}",
            lang,
            &[
                ("status", code.to_string()),
                ("detail", MANUAL_SUFFIX.to_string()),
            ],
        ))),
        Err(ureq::Error::Transport(t)) => {
            let text = t.to_string();
            if text.to_lowercase().contains("timed out") || text.to_lowercase().contains("timeout") {
                Err(FetchError::Retryable(i18n::tr(
                    "检查更新超时，请检查网络后重试，或打开官方下载页手动下载。",
                    lang,
                )))
            } else {
                Err(FetchError::Retryable(i18n::tr(
                    "无法连接更新服务，请检查网络后重试，或打开官方下载页手动下载。",
                    lang,
                )))
            }
        }
    }
}

fn check_blocking(current: &str, lang: &str) -> Result<Option<Release>, String> {
    match fetch_json(MANIFEST_URL, false, lang) {
        Ok(value) => parse_release(&value, current, lang),
        Err(FetchError::Fatal(msg)) => Err(msg),
        Err(FetchError::Retryable(first)) => match fetch_json(RELEASES_API, true, lang) {
            Ok(value) => parse_release(&value, current, lang),
            Err(FetchError::Fatal(msg)) => Err(msg),
            Err(FetchError::Retryable(_)) => Err(first),
        },
    }
}

pub async fn run_check(app: &AppHandle, manual: bool) -> Result<Value, String> {
    let state = app.state::<AppState>();
    let lang = state.language();
    let current = env!("CARGO_PKG_VERSION").to_string();

    if !manual {
        let enabled = state
            .data
            .lock()
            .map(|d| d.settings.check_updates)
            .unwrap_or(false);
        if !enabled {
            return Ok(crate::commands::snapshot_json(app));
        }
    }

    {
        let mut update = state.update.lock().map_err(|e| e.to_string())?;
        update.phase = "checking".into();
        update.error.clear();
    }
    let _ = app.emit("state", crate::commands::snapshot_json(app));

    let result = tauri::async_runtime::spawn_blocking(move || check_blocking(&current, &lang))
        .await
        .map_err(|e| e.to_string())?;

    {
        let mut update = state.update.lock().map_err(|e| e.to_string())?;
        update.checked_at = Some(crate::state::now_ms());
        match result {
            Ok(Some(release)) => {
                update.phase = "available".into();
                update.release = Some(release);
                update.error.clear();
                update.dismissed = false;
            }
            Ok(None) => {
                update.phase = "current".into();
                update.release = None;
                update.error.clear();
            }
            Err(message) => {
                update.phase = "error".into();
                update.error = message;
            }
        }
    }
    let snap = crate::commands::snapshot_json(app);
    let _ = app.emit("state", &snap);
    Ok(snap)
}

pub fn schedule(app: &AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(7)).await;
        loop {
            let _ = run_check(&handle, false).await;
            tokio::time::sleep(Duration::from_secs(6 * 60 * 60)).await;
        }
    });
}
