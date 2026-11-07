use crate::i18n_map::CORE;

/// 中文原文即 key；英文查表。查不到时回退到 key 本身（与前端 i18n.t 一致）。
pub fn translate<'a>(key: &'a str, lang: &str) -> &'a str {
    if lang == "en" {
        for (zh, en) in CORE {
            if *zh == key {
                return en;
            }
        }
    }
    key
}

pub fn t(key: &str, lang: &str, values: &[(&str, String)]) -> String {
    let mut out = translate(key, lang).to_string();
    for (name, value) in values {
        out = out.replace(&format!("{{{}}}", name), value);
    }
    out
}

pub fn tr(key: &str, lang: &str) -> String {
    translate(key, lang).to_string()
}
