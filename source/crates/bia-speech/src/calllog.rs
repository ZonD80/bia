//! Call transcripts for the telegram/whatsapp MCPs: both sides are recorded while a
//! call is up; `finish` returns the summary that is sent to the chat afterwards.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

/// Who said a line: the other side (named when the summary is written) or bia.
pub const PEER: &str = "\u{1}peer";

struct Log {
    started: Instant,
    lines: Vec<(String, String)>,
}

fn logs() -> &'static Mutex<HashMap<String, Log>> {
    static L: OnceLock<Mutex<HashMap<String, Log>>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(HashMap::new()))
}

/// A call on `key` (e.g. `telegram:123`) is up.
pub fn start(key: &str) {
    if let Ok(mut l) = logs().lock() {
        l.entry(key.to_string()).or_insert_with(|| Log { started: Instant::now(), lines: Vec::new() });
    }
}

/// `who`: a speaker name, `PEER`, or `"bia"`. Ignored when no call is up on `key`.
pub fn add(key: &str, who: &str, text: &str) {
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    if let Ok(mut l) = logs().lock() {
        if let Some(log) = l.get_mut(key) {
            log.lines.push((who.to_string(), text.to_string()));
        }
    }
}

/// The call ended: plain-text summary (`📞 Call · 3:12` + lines), `PEER` shown as
/// `peer_name`. None when no call was recorded on `key` or nothing was said.
pub fn finish(key: &str, peer_name: &str) -> Option<String> {
    let log = logs().lock().ok()?.remove(key)?;
    if log.lines.is_empty() {
        return None;
    }
    let s = log.started.elapsed().as_secs();
    let mut out = format!("📞 Call · {}:{:02}\n", s / 60, s % 60);
    for (who, text) in log.lines {
        let who = if who == PEER { peer_name } else { who.as_str() };
        out.push_str(&format!("\n{who}: {text}"));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_and_summarises() {
        add("t:1", "bia", "ignored: no call yet");
        start("t:1");
        add("t:1", PEER, "Привет, ты меня слышишь?");
        add("t:1", "bia", "Да, слышу.");
        add("t:1", "bia", "  ");
        let s = finish("t:1", "ZonD").unwrap();
        assert!(s.starts_with("📞 Call · 0:00"));
        assert!(s.contains("\nZonD: Привет, ты меня слышишь?\nbia: Да, слышу."));
        assert!(!s.contains("ignored"));
        assert!(finish("t:1", "ZonD").is_none(), "finished once");
        start("t:2");
        assert!(finish("t:2", "x").is_none(), "nothing said");
    }
}
