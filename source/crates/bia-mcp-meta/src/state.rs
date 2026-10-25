//! What this MCP keeps beside bia-meta's own store: whether an account is logged in, how
//! far inbound messages were delivered (catch-up after a restart), and banned chats.

use crate::Platform;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::PathBuf;

pub fn files_dir() -> PathBuf {
    PathBuf::from(std::env::var("BIA_FILES").unwrap_or_else(|_| "/home/bia/files".into()))
}

/// bia-meta's state directory (session, keys, chat index): files/<platform>.
pub fn account_dir(p: Platform) -> PathBuf {
    files_dir().join(p.name())
}

/// Received files land here, one folder per chat.
pub fn inbox_dir(p: Platform) -> PathBuf {
    files_dir().join("inbox").join(p.name())
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct Cfg {
    /// True once a login worked; gates the reconnect at boot.
    #[serde(default)]
    pub logged_in: bool,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub username: String,
    /// Newest delivered inbound message (Unix seconds): catch-up messages older than this
    /// were already seen.
    #[serde(default)]
    pub last_seen: i64,
}

fn cfg_path(p: Platform) -> PathBuf {
    files_dir().join("state").join(format!("{}.json", p.name()))
}

pub fn load_cfg(p: Platform) -> Cfg {
    std::fs::read(cfg_path(p))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_cfg(p: Platform, c: &Cfg) -> Result<(), String> {
    let path = cfg_path(p);
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(c).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

/// Update the config in place (read-modify-write).
pub fn update_cfg(p: Platform, f: impl FnOnce(&mut Cfg)) {
    let mut c = load_cfg(p);
    f(&mut c);
    let _ = save_cfg(p, &c);
}

fn bans_path(p: Platform) -> PathBuf {
    files_dir().join("state").join(format!("{}-bans.json", p.name()))
}

#[derive(Serialize, Deserialize, Default)]
struct BanFile {
    #[serde(default)]
    chats: BTreeSet<String>,
}

pub fn bans(p: Platform) -> BTreeSet<String> {
    std::fs::read(bans_path(p))
        .ok()
        .and_then(|b| serde_json::from_slice::<BanFile>(&b).ok())
        .map(|f| f.chats)
        .unwrap_or_default()
}

/// Ban or unban; true when it changed.
pub fn set_ban(p: Platform, chat: &str, on: bool) -> Result<bool, String> {
    let mut chats = bans(p);
    let changed = if on { chats.insert(chat.to_string()) } else { chats.remove(chat) };
    if changed {
        let path = bans_path(p);
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, serde_json::to_vec_pretty(&BanFile { chats }).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    }
    Ok(changed)
}

pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// File-name safe version of a chat id.
pub fn safe_name(key: &str) -> String {
    key.chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '_' })
        .collect()
}

/// YYYYMMDD-HHMMSS (UTC) for received file names.
pub fn stamp() -> String {
    let secs = now_secs().max(0) as u64;
    let (days, tod) = (secs / 86400, secs % 86400);
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + u64::from(mo <= 2);
    format!("{y:04}{mo:02}{d:02}-{:02}{:02}{:02}", tod / 3600, (tod % 3600) / 60, tod % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cfg_and_bans_round_trip() {
        let dir = std::env::temp_dir().join(format!("bia-meta-state-{}", std::process::id()));
        std::env::set_var("BIA_FILES", &dir);
        let p = Platform::Instagram;
        assert!(!load_cfg(p).logged_in);
        update_cfg(p, |c| {
            c.logged_in = true;
            c.last_seen = 42;
        });
        let c = load_cfg(p);
        assert!(c.logged_in && c.last_seen == 42);
        assert!(set_ban(p, "123", true).unwrap());
        assert!(!set_ban(p, "123", true).unwrap(), "already banned");
        assert!(bans(p).contains("123"));
        assert!(set_ban(p, "123", false).unwrap());
        assert!(bans(p).is_empty());
        assert_eq!(safe_name("5@g.us"), "5_g.us");
        assert_eq!(stamp().len(), 15);
        let _ = std::fs::remove_dir_all(dir);
    }
}
