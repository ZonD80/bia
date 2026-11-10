//! When the agent panics: keep what happened (message, place, stack trace), restart by itself,
//! and tell the model on the next start. Crash loops (CRASH_LIMIT within CRASH_WINDOW) stop at
//! the recovery prompt instead, so a bia that can't start doesn't restart forever.
//!
//! files/state/crash.json   the last crash, read (and removed) by the next start → boot notice
//! files/state/crashes.json  when recent crashes happened (the loop guard)
//! files/state/crash.log    every crash with its full trace, newest last (kept to CRASH_LOG_MAX)

use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;

pub const CRASH_LIMIT: usize = 3;
pub const CRASH_WINDOW_SECS: u64 = 10 * 60;
const CRASH_LOG_MAX: usize = 256 * 1024;
const TRACE_LINES: usize = 24;

/// Filled by the panic hook (it runs before the unwind reaches main): where and the trace.
static LAST: Mutex<Option<(String, String)>> = Mutex::new(None);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Crash {
    pub at: u64,
    pub version: String,
    pub message: String,
    pub location: String,
    /// bia's own frames of the stack trace, for the model and the console
    pub trace: String,
}

/// Keep the default "thread 'main' panicked at …" line, and remember the place and full trace.
pub fn install_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info.location().map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column())).unwrap_or_default();
        let trace = std::backtrace::Backtrace::force_capture().to_string();
        if let Ok(mut last) = LAST.lock() {
            *last = Some((location, trace));
        }
        default(info);
    }));
}

/// The panic that reached main, with what the hook kept. → (crash, full trace)
pub fn take(message: String, version: &str, now: u64) -> (Crash, String) {
    let (location, full) = LAST.lock().ok().and_then(|mut l| l.take()).unwrap_or_default();
    let crash = Crash { at: now, version: version.into(), message, location, trace: own_frames(&full) };
    (crash, full)
}

/// The frames of bia's own code (function and file:line), at most TRACE_LINES lines.
pub fn own_frames(full: &str) -> String {
    let lines: Vec<&str> = full.lines().collect();
    let mut out = vec![];
    let mut i = 0;
    while i < lines.len() && out.len() < TRACE_LINES {
        let l = lines[i].trim();
        // "12: bia_agent::tools::Tools::builtin" then "at ./source/crates/bia-agent/src/tools.rs:3330:23"
        let is_frame = l.split_once(": ").is_some_and(|(n, _)| n.chars().all(|c| c.is_ascii_digit()));
        if is_frame && l.contains("bia_") {
            out.push(l.split_once(": ").map(|x| x.1).unwrap_or(l).to_string());
            if let Some(next) = lines.get(i + 1).map(|s| s.trim()) {
                if let Some(at) = next.strip_prefix("at ") {
                    let at = at.rsplit_once("/source/").map(|x| format!("source/{}", x.1)).unwrap_or_else(|| at.to_string());
                    out.push(format!("  at {at}"));
                    i += 1;
                }
            }
        }
        i += 1;
    }
    out.join("\n")
}

/// Record a crash: crash.json for the next start, crashes.json for the loop guard, the full
/// trace appended to crash.log. → whether to restart by itself (false: a crash loop).
pub fn record(state: &Path, crash: &Crash, full_trace: &str) -> bool {
    let _ = std::fs::create_dir_all(state);
    let _ = std::fs::write(state.join("crash.json"), serde_json::to_vec_pretty(crash).unwrap_or_default());
    let recent_path = state.join("crashes.json");
    let mut recent: Vec<u64> = std::fs::read(&recent_path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    recent.retain(|t| crash.at.saturating_sub(*t) < CRASH_WINDOW_SECS);
    recent.push(crash.at);
    let _ = std::fs::write(&recent_path, serde_json::to_vec(&recent).unwrap_or_default());
    let entry = format!(
        "=== {} bia {} — panic: {} at {}\n{}\n\n",
        crate::clock::rfc3339_unix(crash.at),
        crash.version,
        crash.message,
        crash.location,
        if full_trace.trim().is_empty() { "(no trace)" } else { full_trace.trim_end() }
    );
    let log = state.join("crash.log");
    let mut text = std::fs::read_to_string(&log).unwrap_or_default();
    text.push_str(&entry);
    if text.len() > CRASH_LOG_MAX {
        let cut = text.len() - CRASH_LOG_MAX;
        let start = text[cut..].find("=== ").map(|p| cut + p).unwrap_or(cut);
        text = text[start..].to_string();
    }
    let _ = std::fs::write(&log, text);
    recent.len() < CRASH_LIMIT
}

/// The crash the previous run left, once (the file is removed).
pub fn take_last(state: &Path) -> Option<Crash> {
    let p = state.join("crash.json");
    let c = std::fs::read(&p).ok().and_then(|b| serde_json::from_slice(&b).ok());
    let _ = std::fs::remove_file(&p);
    c
}

/// The boot notice's first part after a crash (the rest says what was resumed).
pub fn notice(c: &Crash, now: u64) -> String {
    let mut s = format!(
        "the agent crashed {} ago and restarted by itself (bia {}): panic: {}",
        crate::clock::ago(c.at.min(now)),
        c.version,
        c.message
    );
    if !c.location.is_empty() {
        s.push_str(&format!(" at {}", c.location));
    }
    if !c.trace.is_empty() {
        s.push_str(&format!("\nTrace (bia's frames, newest first):\n{}", c.trace));
    }
    s.push_str(
        "\nTell root on the channel root last wrote on: that you crashed and restarted, the panic \
         line, and that the full trace is in files/state/crash.log (ask before sending it anywhere). \
         Then go on with what you were doing; if a tool call led to the crash, do it another way.",
    );
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRACE: &str = "   0: std::backtrace_rs::backtrace::libunwind::trace
             at /rustc/abc/library/std/src/../../backtrace/src/backtrace/libunwind.rs:117:9
   1: std::panicking::begin_panic_handler
   2: bia_agent::tools::Tools::builtin
             at ./source/crates/bia-agent/src/tools.rs:3330:23
   3: bia_agent::tools::Tools::call_inner
             at ./source/crates/bia-agent/src/tools.rs:2593:23
   4: core::ops::function::FnOnce::call_once
";

    #[test]
    fn keeps_only_bias_frames() {
        let f = own_frames(TRACE);
        assert_eq!(
            f,
            "bia_agent::tools::Tools::builtin\n  at source/crates/bia-agent/src/tools.rs:3330:23\nbia_agent::tools::Tools::call_inner\n  at source/crates/bia-agent/src/tools.rs:2593:23"
        );
    }

    #[test]
    fn a_real_trace_keeps_bias_frames() {
        let full = std::backtrace::Backtrace::force_capture().to_string();
        let f = own_frames(&full);
        assert!(f.contains("bia_agent::crash::tests::a_real_trace_keeps_bias_frames"), "{f}\n---\n{full}");
        assert!(f.contains("src/crash.rs:"), "{f}");
    }

    #[test]
    fn restarts_until_a_crash_loop() {
        let dir = std::env::temp_dir().join(format!("bia-crash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let c = |at| Crash { at, version: "2.1.7".into(), message: "boom".into(), location: "x.rs:1:1".into(), trace: String::new() };
        assert!(record(&dir, &c(1000), TRACE));
        assert!(record(&dir, &c(1100), TRACE));
        assert!(!record(&dir, &c(1200), TRACE)); // the third within ten minutes: stop
        assert!(record(&dir, &c(1200 + CRASH_WINDOW_SECS + 1), TRACE)); // the old ones aged out
        let log = std::fs::read_to_string(dir.join("crash.log")).unwrap();
        assert_eq!(log.matches("=== ").count(), 4);
        assert!(log.contains("tools.rs:3330:23"));
        let last = take_last(&dir).unwrap();
        assert_eq!(last.message, "boom");
        assert!(take_last(&dir).is_none()); // once
        let n = notice(&last, last.at + 5);
        assert!(n.contains("restarted by itself") && n.contains("panic: boom at x.rs:1:1"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
