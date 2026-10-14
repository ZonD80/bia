//! Speech diagnostics: one shared log, /home/bia/files/speech/logs/speech.log (all
//! speech processes append; lines are written with O_APPEND, kept under ~2 MB).
//! `tail` feeds status tools so a failed call can be diagnosed from the chat.

use std::io::Write;
use std::sync::Mutex;

const MAX_BYTES: u64 = 2_000_000;

fn path() -> std::path::PathBuf {
    crate::paths::store().join("logs/speech.log")
}

pub fn log(who: &str, msg: impl AsRef<str>) {
    static GUARD: Mutex<()> = Mutex::new(());
    let _g = GUARD.lock();
    let p = path();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if std::fs::metadata(&p).map(|m| m.len() > MAX_BYTES).unwrap_or(false) {
        let _ = std::fs::rename(&p, p.with_extension("log.1"));
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let (s, ms) = (now.as_secs(), now.subsec_millis());
    let hms = format!("{:02}:{:02}:{:02}.{ms:03}", (s / 3600) % 24, (s / 60) % 60, s % 60);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        let _ = writeln!(f, "{hms}Z {who}: {}", msg.as_ref().replace('\n', " "));
    }
}

/// Last `n` lines of the log.
pub fn tail(n: usize) -> String {
    let text = std::fs::read_to_string(path()).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}
