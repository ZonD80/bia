//! Unloads speech models nobody has used for a while. The model caches (recognizers,
//! language identification, denoiser, speaker embedder, voices) are per process and
//! every speech MCP has its own, so models left loaded after a burst of work add up
//! to gigabytes of idle heap. Each cache lookup and each frame of call audio counts
//! as use; `BIA_SPEECH_IDLE_SECS` sets the wait (default 300, 0 keeps models loaded).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Once, OnceLock};
use std::time::{Duration, Instant};

const DEFAULT_IDLE_SECS: u64 = 300;
const CHECK_EVERY: Duration = Duration::from_secs(30);

static LAST_USE: AtomicU64 = AtomicU64::new(0);

fn now_secs() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs()
}

fn idle_secs() -> u64 {
    std::env::var("BIA_SPEECH_IDLE_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(DEFAULT_IDLE_SECS)
}

fn due(last: u64, now: u64, idle: u64) -> bool {
    idle > 0 && now.saturating_sub(last) >= idle
}

/// Speech models are in use right now.
pub fn touch() {
    LAST_USE.store(now_secs(), Ordering::Relaxed);
    static REAPER: Once = Once::new();
    REAPER.call_once(|| {
        let idle = idle_secs();
        if idle == 0 {
            return;
        }
        let _ = std::thread::Builder::new().name("speech-idle".into()).spawn(move || loop {
            std::thread::sleep(CHECK_EVERY);
            if due(LAST_USE.load(Ordering::Relaxed), now_secs(), idle) {
                let n = crate::models::unload_unused() + crate::tts::unload_unused();
                if n > 0 {
                    trim();
                    crate::diag::log("speech", format!("unloaded {n} idle model(s) after {idle}s without use"));
                }
            }
        });
    });
}

/// Hand the freed heap back to the system (glibc keeps it otherwise).
fn trim() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    unsafe {
        libc::malloc_trim(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn due_after_idle_only() {
        assert!(!due(100, 399, 300));
        assert!(due(100, 400, 300));
        assert!(!due(100, 10_000, 0), "0 keeps models loaded");
        assert!(!due(100, 50, 300), "clock never runs backwards into an unload");
    }
}
