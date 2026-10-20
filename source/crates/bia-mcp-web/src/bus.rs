//! The MCP side of the web bus: messages for the agent (MCP `inbound` notifications), log lines,
//! and the browser chat sessions (WebSocket sinks) of each client-certificate identity.
//! Nothing here touches the agent's console: web is a channel like telegram or whatsapp.
use bia_mcp::Ctx;
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

pub const ID_ROOT: &str = "root";
pub const ID_SYSTEM: &str = "system";
/// The main web chat of an identity; other chats are `web:<chat>`.
pub const CH_WEB: &str = "web";

/// A message for the agent: `identity|channel|…|id|text`.
pub struct Inbound {
    pub identity: String,
    pub channel: String,
    pub id: String,
    pub text: String,
}

struct Sink {
    id: usize,
    identity: String,
    tx: Sender<Vec<u8>>,
}

/// One person's browser sessions, for the open/close notices (Bus::web_opened/web_closed).
#[derive(Default)]
struct Seen {
    sessions: usize,
    /// bumped by every open, so a pending close notice knows a newer session came
    generation: u64,
    /// the model was told the web app is open (and not yet that it closed)
    open: bool,
    /// when the last close notice went out
    closed_at: Option<Instant>,
}

/// A close only counts after this long without a new session: phones drop and reconnect.
pub const CLOSE_GRACE: std::time::Duration = std::time::Duration::from_secs(60);

pub struct Bus {
    ctx: Mutex<Option<Ctx>>,
    presence: Mutex<HashMap<String, Seen>>,
    close_grace: std::time::Duration,
    /// Messages that arrived before the MCP session was up (or in tests), with when
    /// they came (Unix seconds) so the envelope's issued is not the later delivery.
    held: Mutex<VecDeque<(Inbound, i64)>>,
    sinks: Mutex<Vec<Sink>>,
    next: AtomicUsize,
    started: Instant,
    pub files: PathBuf,
}

impl Bus {
    pub fn new(files: PathBuf) -> Bus {
        Bus {
            ctx: Mutex::new(None),
            presence: Mutex::new(HashMap::new()),
            close_grace: CLOSE_GRACE,
            held: Mutex::new(VecDeque::new()),
            sinks: Mutex::new(vec![]),
            next: AtomicUsize::new(1),
            started: Instant::now(),
            files,
        }
    }

    /// The MCP session is up: deliver what was held, then send directly.
    pub fn attach(&self, ctx: Ctx) {
        let held: Vec<(Inbound, i64)> = self.held.lock().unwrap().drain(..).collect();
        for (ib, issued) in &held {
            ctx.inbound_at(&ib.identity, &ib.channel, &ib.id, &ib.text, *issued);
        }
        *self.ctx.lock().unwrap() = Some(ctx);
    }

    pub fn push_inbound(&self, ib: Inbound) {
        match self.ctx.lock().unwrap().as_ref() {
            Some(ctx) => ctx.inbound(&ib.identity, &ib.channel, &ib.id, &ib.text),
            None => {
                let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
                self.held.lock().unwrap().push_back((ib, now))
            }
        }
    }

    /// Wait up to `timeout` for the next held message (tests).
    pub fn next_held(&self, timeout: std::time::Duration) -> Option<Inbound> {
        let end = Instant::now() + timeout;
        loop {
            if let Some((ib, _)) = self.held.lock().unwrap().pop_front() {
                return Some(ib);
            }
            if Instant::now() >= end {
                return None;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    /// Messages not delivered to an agent yet (tests read them here).
    pub fn take_held(&self) -> Vec<Inbound> {
        self.held.lock().unwrap().drain(..).map(|(ib, _)| ib).collect()
    }

    /// A log line for the agent's MCP log.
    pub fn log(&self, s: &str) {
        let s = s.trim();
        if s.is_empty() {
            return;
        }
        match self.ctx.lock().unwrap().as_ref() {
            Some(ctx) => ctx.log(s),
            None => {
                let _ = writeln!(std::io::stderr(), "{s}");
            }
        }
    }

    // ---- browser chat sessions ----

    /// A browser chat session: it receives `web_event` JSON (0x1d-prefixed).
    pub fn add_web_sink(&self, identity: &str) -> (usize, Receiver<Vec<u8>>) {
        let (tx, rx) = channel();
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        self.sinks.lock().unwrap().push(Sink {
            id,
            identity: identity.to_string(),
            tx,
        });
        (id, rx)
    }

    pub fn remove_sink(&self, id: usize) {
        self.sinks.lock().unwrap().retain(|s| s.id != id);
    }

    pub fn send_to_sink(&self, id: usize, bytes: &[u8]) -> bool {
        let sinks = self.sinks.lock().unwrap();
        match sinks.iter().find(|s| s.id == id) {
            Some(s) => s.tx.send(bytes.to_vec()).is_ok(),
            None => false,
        }
    }

    /// A browser chat session of this identity is connected.
    pub fn has_web(&self, identity: &str) -> bool {
        self.sinks
            .lock()
            .unwrap()
            .iter()
            .any(|s| s.identity == identity)
    }

    /// JSON event for every browser session of one identity.
    pub fn web_event(&self, identity: &str, payload: &str) {
        let mut pkt = Vec::with_capacity(1 + payload.len());
        pkt.push(0x1d);
        pkt.extend_from_slice(payload.as_bytes());
        let mut sinks = self.sinks.lock().unwrap();
        sinks.retain(|s| s.identity != identity || s.tx.send(pkt.clone()).is_ok());
    }

    /// Tests: a short grace before a close notice.
    pub fn with_close_grace(mut self, d: std::time::Duration) -> Bus {
        self.close_grace = d;
        self
    }

    /// A browser chat session of `identity` connected (from `ip`). The person's first
    /// session tells the model their web app is open; more tabs, and a reconnect within
    /// the grace after a drop, say nothing.
    pub fn web_opened(&self, identity: &str, ip: &str) {
        let away = {
            let mut p = self.presence.lock().unwrap();
            let seen = p.entry(identity.to_string()).or_default();
            seen.sessions += 1;
            seen.generation += 1;
            if seen.open {
                return;
            }
            seen.open = true;
            seen.closed_at.map(|t| t.elapsed())
        };
        let since = match away {
            Some(d) => format!("back after {}", human_duration(d)),
            None => "first time since bia started".into(),
        };
        self.push_inbound(Inbound {
            identity: ID_SYSTEM.into(),
            channel: CH_WEB.into(),
            id: mint_msg_id(),
            text: format!(
                "[web open] {identity} opened the web app (from {ip}; {since}). Their chats there are web and web:<chat>. \
                 Greet them with say(identity=\"{identity}\", channel=\"web\") only if it fits: you have news or an answer for them, or they were away long. \
                 Otherwise do nothing; no reply is needed."
            ),
        });
    }

    /// A browser chat session of `identity` ended. When it was their last one and none
    /// comes back within the grace, the model hears that the web app is closed.
    pub fn web_closed(bus: &Arc<Bus>, identity: &str) {
        let generation = {
            let mut p = bus.presence.lock().unwrap();
            let Some(seen) = p.get_mut(identity) else { return };
            seen.sessions = seen.sessions.saturating_sub(1);
            if seen.sessions > 0 || !seen.open {
                return;
            }
            seen.generation
        };
        let (bus, identity) = (bus.clone(), identity.to_string());
        std::thread::spawn(move || {
            std::thread::sleep(bus.close_grace);
            {
                let mut p = bus.presence.lock().unwrap();
                let Some(seen) = p.get_mut(&identity) else { return };
                if seen.sessions > 0 || seen.generation != generation || !seen.open {
                    return;
                }
                seen.open = false;
                seen.closed_at = Some(Instant::now());
            }
            bus.push_inbound(Inbound {
                identity: ID_SYSTEM.into(),
                channel: CH_WEB.into(),
                id: mint_msg_id(),
                text: format!(
                    "[web closed] {identity} closed the web app (no browser of theirs is open now). \
                     What you send to their web chat waits there for them. No reply is needed."
                ),
            });
        });
    }

    // ---- machine status (written by the agent to files/state/status.json) ----

    pub fn status(&self) -> Value {
        std::fs::read(self.files.join("state").join("status.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_else(|| serde_json::json!({}))
    }

    /// Machine uptime (seconds).
    pub fn uptime_secs(&self) -> u64 {
        std::fs::read_to_string("/proc/uptime")
            .ok()
            .and_then(|s| {
                s.split_whitespace()
                    .next()
                    .and_then(|v| v.parse::<f64>().ok())
            })
            .map(|v| v as u64)
            .unwrap_or_else(|| self.started.elapsed().as_secs())
    }
}

/// 45 s, 12 min, 3 h, 2 days: how long someone was away.
fn human_duration(d: std::time::Duration) -> String {
    let s = d.as_secs();
    match s {
        0..=89 => format!("{s} s"),
        90..=5399 => format!("{} min", (s + 30) / 60),
        5400..=172_799 => format!("{} h", (s + 1800) / 3600),
        _ => format!("{} days", (s + 43_200) / 86_400),
    }
}

/// RFC 3339 UTC timestamp (seconds).
pub fn rfc3339(t: SystemTime) -> String {
    let secs = t
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem / 60 % 60,
        rem % 60
    )
}

/// Days since 1970-01-01 → (year, month, day) (Howard Hinnant's algorithm).
fn civil(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    (y, m, d)
}

/// Local message id for web messages (`b…`, like the agent's other local buses).
pub fn mint_msg_id() -> String {
    static SEQ: AtomicUsize = AtomicUsize::new(1);
    let ms = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("b{:08x}{:04x}", ms as u32, n as u16)
}

/// `files/inbox/<bus>/` folders exist (uploads land there).
pub fn ensure_inbox(files: &std::path::Path) {
    let _ = std::fs::create_dir_all(files.join("inbox").join("web"));
}

pub fn rfc3339_unix(secs: u64) -> String {
    rfc3339(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(secs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_open_and_close_notices_skip_reconnects_and_extra_tabs() {
        let bus = Arc::new(Bus::new(std::env::temp_dir()).with_close_grace(std::time::Duration::from_millis(200)));
        let texts = |bus: &Bus| bus.take_held().into_iter().map(|ib| (ib.identity, ib.text)).collect::<Vec<_>>();
        bus.web_opened("alex", "192.0.2.7");
        let t = texts(&bus);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].0, ID_SYSTEM);
        assert!(t[0].1.starts_with("[web open] alex") && t[0].1.contains("192.0.2.7") && t[0].1.contains("first time"), "{}", t[0].1);
        // a second tab, then one of two closes: nothing
        bus.web_opened("alex", "192.0.2.7");
        Bus::web_closed(&bus, "alex");
        std::thread::sleep(std::time::Duration::from_millis(400));
        assert!(texts(&bus).is_empty());
        // the last tab drops and comes back within the grace: nothing
        Bus::web_closed(&bus, "alex");
        std::thread::sleep(std::time::Duration::from_millis(50));
        bus.web_opened("alex", "192.0.2.7");
        std::thread::sleep(std::time::Duration::from_millis(400));
        assert!(texts(&bus).is_empty());
        // closed for good: one close notice, and the next open says they are back
        Bus::web_closed(&bus, "alex");
        std::thread::sleep(std::time::Duration::from_millis(400));
        let t = texts(&bus);
        assert_eq!(t.len(), 1);
        assert!(t[0].1.starts_with("[web closed] alex"), "{}", t[0].1);
        bus.web_opened("alex", "192.0.2.8");
        let t = texts(&bus);
        assert!(t.len() == 1 && t[0].1.contains("back after"), "{t:?}");
        assert_eq!(human_duration(std::time::Duration::from_secs(3 * 3600)), "3 h");
    }

    #[test]
    fn rfc3339_known_dates() {
        let t = |s: u64| SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(s);
        assert_eq!(rfc3339(t(0)), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(t(951_782_400)), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339(t(1_792_002_818)), "2026-10-14T18:33:38Z");
    }

    #[test]
    fn inbound_is_held_until_attached_and_sinks_are_per_identity() {
        let bus = Bus::new(std::env::temp_dir());
        bus.push_inbound(Inbound {
            identity: "alex".into(),
            channel: "web".into(),
            id: "b1".into(),
            text: "hi".into(),
        });
        let held = bus.take_held();
        assert_eq!(held.len(), 1);
        assert_eq!(
            (held[0].identity.as_str(), held[0].channel.as_str()),
            ("alex", "web")
        );

        let (a, rx_a) = bus.add_web_sink("alex");
        let (_b, rx_b) = bus.add_web_sink("root");
        assert!(bus.has_web("alex") && !bus.has_web("bob"));
        bus.web_event("alex", "{\"t\":\"x\"}");
        assert_eq!(rx_a.try_recv().unwrap(), b"\x1d{\"t\":\"x\"}".to_vec());
        assert!(rx_b.try_recv().is_err());
        bus.remove_sink(a);
        assert!(!bus.has_web("alex"));
    }
}
