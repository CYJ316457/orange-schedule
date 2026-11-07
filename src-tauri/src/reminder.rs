use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::OnceLock;

use crate::state::Rotation;

#[derive(Serialize, Deserialize, Clone)]
pub struct Quote {
    pub id: String,
    pub text: String,
    pub source: String,
    #[serde(default)]
    pub url: String,
}

const ZH_JSON: &str = include_str!("../../src/reminder-classics.json");
const EN_JSON: &str = include_str!("../../src/reminder-classics.en.json");

fn quotes() -> &'static Vec<Quote> {
    static Q: OnceLock<Vec<Quote>> = OnceLock::new();
    Q.get_or_init(|| serde_json::from_str(ZH_JSON).unwrap_or_default())
}

fn english() -> &'static HashMap<String, Quote> {
    static E: OnceLock<HashMap<String, Quote>> = OnceLock::new();
    E.get_or_init(|| {
        let list: Vec<Quote> = serde_json::from_str(EN_JSON).unwrap_or_default();
        list.into_iter().map(|q| (q.id.clone(), q)).collect()
    })
}

pub fn count() -> usize {
    quotes().len()
}

pub fn exists(id: &str) -> bool {
    quotes().iter().any(|q| q.id == id)
}

pub fn current(rotation: &Rotation, lang: &str) -> Value {
    let list = quotes();
    if list.is_empty() {
        return json!({"id":"", "text":"", "source":"", "url":""});
    }
    let base = rotation
        .current_id
        .as_deref()
        .and_then(|id| list.iter().find(|q| q.id == id))
        .unwrap_or(&list[0]);
    let chosen = if lang == "en" {
        english().get(&base.id).cloned().unwrap_or_else(|| base.clone())
    } else {
        base.clone()
    };
    json!({
        "id": chosen.id,
        "text": chosen.text,
        "source": chosen.source,
        "url": chosen.url,
    })
}

/// 与 reminder-copy.cjs 的 advanceMessage 等价：优先未出现过的，全部用过则重置并排除当前项。
pub fn advance(rotation: &Rotation) -> (String, Rotation) {
    let list = quotes();
    if list.is_empty() {
        return (String::new(), rotation.clone());
    }
    let total = list.len();
    let mut seen: Vec<String> = rotation
        .seen
        .iter()
        .filter(|id| exists(id))
        .cloned()
        .collect();
    if seen.len() > total {
        seen = seen.split_off(seen.len() - total);
    }
    let mut available: Vec<&Quote> = list.iter().filter(|q| !seen.contains(&q.id)).collect();
    if available.is_empty() {
        seen.clear();
        available = list
            .iter()
            .filter(|q| Some(q.id.as_str()) != rotation.current_id.as_deref())
            .collect();
        if available.is_empty() {
            available = list.iter().collect();
        }
    }
    let index = fastrand::usize(..available.len());
    let picked = available[index];
    seen.push(picked.id.clone());
    if seen.len() > total {
        seen = seen.split_off(seen.len() - total);
    }
    (
        picked.id.clone(),
        Rotation {
            seen,
            current_id: Some(picked.id.clone()),
        },
    )
}
