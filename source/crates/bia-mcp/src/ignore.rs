//! Ignored chats: busy groups whose messages the model does not need to see. The agent
//! writes the rules (`files/state/ignore.json`, from its chat_ignore tool); the chat MCPs
//! check every message of such a chat after it is complete (a voice note transcribed),
//! so a mention spoken in a voice message wakes the model too. An ignored message still
//! goes to the agent, marked, to be kept out of the model's context; it is not marked
//! read, the way a person leaves a chat they ignore unread.

use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::SystemTime;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Rules {
    /// channel (telegram:-100…, whatsapp:…@g.us, messenger:…, instagram:…) → rule
    #[serde(default)]
    pub chats: BTreeMap<String, Rule>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Rule {
    /// Unix seconds when ignoring began
    #[serde(default)]
    pub since: u64,
    /// regular expressions (case-insensitive) that wake the model
    #[serde(default)]
    pub wake: Vec<String>,
    /// why the model ignores it, in its own words
    #[serde(default)]
    pub note: String,
    /// the chat's name when ignoring began
    #[serde(default)]
    pub title: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Verdict {
    /// not an ignored chat: the model gets it as usual
    Deliver,
    /// kept from the model
    Ignored,
    /// an ignored chat, but this message wakes the model: what woke it
    Woke(String),
}

impl Verdict {
    /// The chat's MCP sends a read receipt for it (an ignored one stays unread).
    pub fn read(&self) -> bool {
        !matches!(self, Verdict::Ignored)
    }
}

/// A reply to one of bia's own messages (`bia_mcp::share::reply_text` names bia "you").
const REPLY_TO_YOU: &str = r"\[reply to \S+ from you[:,]";

pub fn path() -> PathBuf {
    let files = std::env::var("BIA_FILES").unwrap_or_else(|_| "/home/bia/files".into());
    PathBuf::from(files).join("state").join("ignore.json")
}

/// A wake pattern as matched: case-insensitive, size-bounded (the text comes from
/// strangers; the regex crate also runs in linear time, so no pattern can hang).
pub fn compile(pattern: &str) -> Result<Regex, String> {
    RegexBuilder::new(pattern)
        .case_insensitive(true)
        .size_limit(1 << 20)
        .build()
        .map_err(|e| format!("wake pattern {pattern:?}: {e}"))
}

pub fn load() -> Rules {
    std::fs::read(path()).ok().and_then(|d| serde_json::from_slice(&d).ok()).unwrap_or_default()
}

/// Written by the agent; the MCPs see the change at their next message.
pub fn save(r: &Rules) -> std::io::Result<()> {
    let p = path();
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(r)?)?;
    std::fs::rename(&tmp, &p)
}

struct Cache {
    stamp: Option<SystemTime>,
    rules: Rules,
    wake: BTreeMap<String, Vec<(String, Regex)>>,
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

fn with_rules<T>(f: impl FnOnce(&Cache) -> T) -> T {
    let stamp = std::fs::metadata(path()).and_then(|m| m.modified()).ok();
    let mut g = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if g.as_ref().is_none_or(|c| c.stamp != stamp) {
        let rules = if stamp.is_some() { load() } else { Rules::default() };
        let wake = rules
            .chats
            .iter()
            .map(|(ch, r)| {
                // a bad pattern is skipped here; chat_ignore refuses it before it is saved
                let res = r.wake.iter().filter_map(|p| compile(p).ok().map(|re| (p.clone(), re))).collect();
                (ch.clone(), res)
            })
            .collect();
        *g = Some(Cache { stamp, rules, wake });
    }
    f(g.as_ref().unwrap())
}

/// `channel` is ignored now (decide before the message is complete, e.g. whether to
/// send the read receipt at once).
pub fn ignoring(channel: &str) -> bool {
    with_rules(|c| c.rules.chats.contains_key(channel))
}

/// What happens to a finished message (`text` as the model would get it, transcript
/// included) on `channel`.
pub fn check(channel: &str, text: &str) -> Verdict {
    with_rules(|c| {
        if !c.rules.chats.contains_key(channel) {
            return Verdict::Deliver;
        }
        static REPLY: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| Regex::new(REPLY_TO_YOU).unwrap());
        if REPLY.is_match(text) {
            return Verdict::Woke("a reply to you".into());
        }
        match c.wake.get(channel).and_then(|res| res.iter().find(|(_, re)| re.is_match(text))) {
            Some((p, _)) => Verdict::Woke(format!("wake pattern {p}")),
            None => Verdict::Ignored,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_wake_and_ignore() {
        let dir = std::env::temp_dir().join(format!("bia-ignore-{}", std::process::id()));
        std::env::set_var("BIA_FILES", &dir);
        assert_eq!(check("telegram:-1001", "hi"), Verdict::Deliver);
        let mut r = Rules::default();
        r.chats.insert("telegram:-1001".into(), Rule { since: 1, wake: vec![r"\bfred\b".into(), "@fred_bot".into()], ..Default::default() });
        save(&r).unwrap();
        assert!(ignoring("telegram:-1001") && !ignoring("telegram:5"));
        assert_eq!(check("telegram:-1001", "price is up"), Verdict::Ignored);
        assert!(!check("telegram:-1001", "price is up").read());
        // in a voice transcript, any case
        assert_eq!(check("telegram:-1001", "[voice message, 3.1s, en] [speech en] Hey FRED, what do you think?"), Verdict::Woke(r"wake pattern \bfred\b".into()));
        assert_eq!(check("telegram:-1001", "[reply to 77 from you: \"ok\"]\nreally?"), Verdict::Woke("a reply to you".into()));
        assert_eq!(check("telegram:-1001", "[reply to 77 from Anna: \"ok\"]\nreally?"), Verdict::Ignored);
        assert_eq!(check("telegram:5", "anything"), Verdict::Deliver);
        assert!(compile("(unclosed").is_err());
        // the rules file changing is seen at once
        r.chats.clear();
        std::thread::sleep(std::time::Duration::from_millis(20));
        save(&r).unwrap();
        let _ = std::fs::OpenOptions::new().append(true).open(path()).map(|f| f.set_modified(SystemTime::now() + std::time::Duration::from_secs(1)));
        assert_eq!(check("telegram:-1001", "price is up"), Verdict::Deliver);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
