//! Where speech files live.
//!
//! - bundled (read-only, from the image): /usr/local/lib/bia/speech (or $BIA_SPEECH_DIR):
//!   sherpa lib, whisper-tiny, the English Piper voice, Silero VAD, GTCRN.
//! - store (carried by fold): /home/bia/files/speech: downloaded models, the language
//!   registry (languages.json), temp files.

use std::path::PathBuf;

pub fn bundled() -> PathBuf {
    std::env::var("BIA_SPEECH_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/usr/local/lib/bia/speech"))
}

pub fn files() -> PathBuf {
    PathBuf::from(std::env::var("BIA_FILES").unwrap_or_else(|_| "/home/bia/files".into()))
}

pub fn store() -> PathBuf {
    std::env::var("BIA_SPEECH_STORE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| files().join("speech"))
}

pub fn models() -> PathBuf {
    store().join("models")
}

pub fn registry() -> PathBuf {
    store().join("languages.json")
}

pub fn tmp() -> PathBuf {
    let d = store().join("tmp");
    let _ = std::fs::create_dir_all(&d);
    d
}

/// Unique-ish stamp for temp file names.
pub fn stamp() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    (ms << 16) | (N.fetch_add(1, Ordering::Relaxed) & 0xffff)
}
