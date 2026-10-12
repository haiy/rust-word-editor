//! Only application chrome is translated; document payloads are never rewritten.
use std::{collections::HashMap, sync::OnceLock};
pub fn text(source: &str, locale: &str) -> String {
    if locale != "en" {
        return source.to_owned();
    }
    static CATALOG: OnceLock<HashMap<String, String>> = OnceLock::new();
    let map = CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!("../locales/en.json")).expect("validated locale catalog")
    });
    map.get(source)
        .cloned()
        .unwrap_or_else(|| source.to_owned())
}
