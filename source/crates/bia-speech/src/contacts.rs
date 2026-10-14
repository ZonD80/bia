//! Preferred language per channel and per identity, from the agent's addressbook
//! (mirrored to files/speech/contacts.json). Calls and voice notes start in it, and
//! speech uses it when the model leaves `lang` out.

use std::sync::Mutex;
use std::time::SystemTime;

#[derive(Default)]
struct Map {
    mtime: Option<SystemTime>,
    channels: std::collections::HashMap<String, String>,
    identities: std::collections::HashMap<String, String>,
}

fn path() -> std::path::PathBuf {
    crate::paths::store().join("contacts.json")
}

fn with<T>(f: impl FnOnce(&Map) -> T) -> T {
    static M: Mutex<Option<Map>> = Mutex::new(None);
    let mtime = std::fs::metadata(path()).and_then(|m| m.modified()).ok();
    let mut g = M.lock().unwrap_or_else(|e| e.into_inner());
    if g.as_ref().map(|m| m.mtime) != Some(mtime) {
        let v: serde_json::Value = std::fs::read(path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let read = |k: &str| {
            v.get(k)
                .and_then(|o| o.as_object())
                .map(|o| o.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect())
                .unwrap_or_default()
        };
        *g = Some(Map { mtime, channels: read("channels"), identities: read("identities") });
    }
    f(g.as_ref().expect("set"))
}

/// The preferred language of whoever is on `channel` (telegram:123, whatsapp:…).
pub fn lang_for(channel: &str) -> Option<String> {
    with(|m| m.channels.get(channel).cloned())
}

/// The preferred language of a web/console identity (contact id).
pub fn lang_for_identity(identity: &str) -> Option<String> {
    with(|m| m.identities.get(identity).cloned())
}

/// `lang` if given, else the preferred language of `channel`.
pub fn lang_or(lang: &str, channel: &str) -> String {
    if lang.trim().is_empty() {
        lang_for(channel).unwrap_or_default()
    } else {
        lang.to_string()
    }
}
