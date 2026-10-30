//! The web bus protocol without I/O: frame encoding, timers, flow control and the small
//! decisions both ends must agree on. `link.rs` drives it; ARCHITECTURE.md ("The web bus
//! protocol") is the full description.
//!
//! One WebSocket (`GET /ws`) per identity, for browsers, bia-bridge and other bias alike.
//! Text frames are JSON `{t, …}`; binary frames are file chunks
//! `[fid: 32 bytes sha256][offset: u64 BE][data]`.
//!
//! - `hello {proto, name, ver, instance, rx}` from the client first; the server answers
//!   `hello {…, me, rev, floor, typing}`, then the catch-up after `rx` and live changes.
//! - Server → client: `m {seq, m}` (a message's state after a change), `gone {seq, ids}`,
//!   `reset`, `typing {on}`, `ack {cid, ok, r|error}`, `bye {reason}`, call signalling.
//! - Client → server: actions with `cid` (`send`, `edit`, `delete`, `react`, `pin`,
//!   `read`, `button`, `vote`, `close_poll`, `mute`, `hist`, `search`, `call_decline`),
//!   and `active`, `typing`, `ack {seq}`, `ping`, `bye`.
//! - Files are pulled by the receiver, both ways: `fget {fid, off}` → chunks,
//!   `fack {fid, off}` (bytes it has, the sender's flow control), `fdone {fid}`,
//!   `frefuse {fid, reason}`, and from the sender `fcancel {fid}`.
use std::time::{Duration, Instant};

pub const PROTO: u64 = 2;
/// Both sides send `{"t":"ping"}` this often when idle; a session that sent nothing for
/// DEAD_AFTER is closed. WebSocket pongs do not count: a browser's network stack answers
/// pings even for a tab that is gone.
pub const PING_EVERY: Duration = Duration::from_secs(5);
pub const DEAD_AFTER: Duration = Duration::from_secs(15);
pub const PING: &[u8] = br#"{"t":"ping"}"#;
/// A second machine with the same key is reported once refusals go on this long.
pub const COPY_AFTER: Duration = Duration::from_secs(20);
/// Flush time for `bye` frames at shutdown.
pub const BYE_FLUSH: Duration = Duration::from_secs(2);
pub const MAX_LINKS: usize = 256;
/// Dials to other bias at once.
pub const MAX_HANDSHAKES: usize = 4;
/// Handshakes from one address per minute (several people and bias may share one NAT address).
pub const IP_HANDSHAKES_PER_MIN: usize = 30;
/// File chunk payload.
pub const CHUNK: usize = 32 * 1024;
/// Files pulled at once: per link and over all links.
pub const PULLS_PER_LINK: usize = 2;
pub const PULLS_TOTAL: usize = 8;
pub const MAX_FILE: u64 = 4 << 30;
/// Free space kept after a file lands.
pub const DISK_MARGIN: u64 = 512 << 20;
/// Largest frame accepted (a chunk plus header, or a JSON frame).
pub const MAX_FRAME: u64 = 2 << 20;
/// Actions per minute before the reader slows down.
pub const RATE_PER_MIN: usize = 240;
/// Unfinished transfers are kept this long.
pub const KEEP_PARTIAL_SECS: u64 = 3 * 86_400;
/// Renew this bia's certificate at a peer this many days before it expires.
pub const RENEW_WITHIN_DAYS: i64 = 30;

/// Reasons in `bye` that mean "back soon".
pub const BYE_RESTARTS: &[&str] = &["reboot", "poweroff", "update", "restart", "shutdown"];

/// A WebSocket frame. A client masks (`mask` = Some) as RFC 6455 asks.
pub fn encode_frame(opcode: u8, payload: &[u8], mask: Option<[u8; 4]>) -> Vec<u8> {
    let mut f = Vec::with_capacity(payload.len() + 14);
    f.push(0x80 | opcode);
    let m = if mask.is_some() { 0x80 } else { 0 };
    let n = payload.len();
    if n < 126 {
        f.push(m | n as u8);
    } else if n < 65536 {
        f.push(m | 126);
        f.extend_from_slice(&(n as u16).to_be_bytes());
    } else {
        f.push(m | 127);
        f.extend_from_slice(&(n as u64).to_be_bytes());
    }
    match mask {
        Some(k) => {
            f.extend_from_slice(&k);
            f.extend(payload.iter().enumerate().map(|(i, b)| b ^ k[i % 4]));
        }
        None => f.extend_from_slice(payload),
    }
    f
}

pub fn chunk_frame(fid: &[u8; 32], off: u64, data: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(40 + data.len());
    b.extend_from_slice(fid);
    b.extend_from_slice(&off.to_be_bytes());
    b.extend_from_slice(data);
    b
}

pub fn parse_chunk(b: &[u8]) -> Option<(String, u64, &[u8])> {
    if b.len() < 40 {
        return None;
    }
    let fid = hex(&b[..32]);
    let off = u64::from_be_bytes(b[32..40].try_into().ok()?);
    Some((fid, off, &b[40..]))
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn unhex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

/// Delay before redial attempt `attempt` (0-based): 5–10 s, 10–20 s, 20–40 s, … at most
/// 10 min. `rnd` in [0, 1) spreads bias that all lost the same peer at once.
pub fn backoff(attempt: u32, rnd: f64) -> Duration {
    let base = 5f64 * 2f64.powi(attempt.min(10) as i32);
    let d = (base * (1.0 + rnd.clamp(0.0, 1.0))).min(600.0);
    Duration::from_millis((d * 1000.0) as u64)
}

/// Two links for one pair of bias (both dialed at once): both ends keep the one dialed by
/// the lower agent name.
pub fn keep_dialed_by<'a>(me: &'a str, peer: &'a str) -> &'a str {
    if me <= peer {
        me
    } else {
        peer
    }
}

/// File bytes in flight: about half a second of what the link moved lately, so a message
/// queued behind a transfer waits at most that long.
pub fn window_for(bytes_per_sec: f64) -> u64 {
    ((bytes_per_sec * 0.5) as u64).clamp(64 * 1024, 4 << 20)
}

/// Measured file throughput (acked bytes), smoothed.
#[derive(Debug)]
pub struct Rate {
    pub bps: f64,
    last: Option<(Instant, u64)>,
}

impl Default for Rate {
    fn default() -> Self {
        // start at 512 KiB/s: a 256 KiB window
        Rate {
            bps: 512.0 * 1024.0,
            last: None,
        }
    }
}

impl Rate {
    /// `total` bytes acked so far for a transfer, at `now`.
    pub fn acked(&mut self, now: Instant, total: u64) {
        match self.last {
            Some((t, n)) if total > n => {
                let dt = now.duration_since(t).as_secs_f64();
                if dt >= 0.05 {
                    let r = (total - n) as f64 / dt;
                    self.bps = self.bps * 0.7 + r * 0.3;
                    self.last = Some((now, total));
                }
            }
            Some(_) => {}
            None => self.last = Some((now, total)),
        }
    }
    pub fn restart(&mut self) {
        self.last = None;
    }
}

/// A name in the other bia's chat store, as this bia keeps it: `bia` there is the peer,
/// and the peer's name for this bia (`me_there`) is `bia` here.
pub fn name_here<'a>(name: &'a str, peer: &'a str, me_there: &str) -> &'a str {
    if name == "bia" {
        peer
    } else if name == me_there {
        "bia"
    } else {
        name
    }
}

/// `systemctl list-jobs` output → why the machine is going down (None: it is not).
pub fn reason_from_jobs(jobs: &str) -> Option<&'static str> {
    if jobs.contains("reboot.target") || jobs.contains("kexec.target") {
        Some("reboot")
    } else if jobs.contains("poweroff.target") || jobs.contains("halt.target") {
        Some("poweroff")
    } else {
        None
    }
}

/// The reason file the agent writes before an update (`{reason, at}`), when fresh.
pub fn reason_from_file(v: &serde_json::Value, now: u64) -> Option<String> {
    let r = v.get("reason")?.as_str()?.trim();
    let at = v.get("at")?.as_u64()?;
    (BYE_RESTARTS.contains(&r) && now.saturating_sub(at) <= 120).then(|| r.to_string())
}

/// Ids a client may choose for its own messages: short, safe in file names and URLs.
pub fn valid_msg_id(id: &str) -> bool {
    (4..=48).contains(&id.len()) && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn frames_round_trip_masked_and_not() {
        for n in [0usize, 5, 125, 126, 300, 65535, 65536, 70000] {
            let p: Vec<u8> = (0..n).map(|i| (i * 7) as u8).collect();
            for mask in [None, Some([1, 2, 3, 4])] {
                let f = encode_frame(2, &p, mask);
                let (op, got) = crate::server::ws_recv_max(&mut &f[..], 1 << 20).unwrap();
                assert_eq!((op, got.len()), (2, n));
                assert_eq!(got, p);
            }
        }
    }

    #[test]
    fn chunks_and_hex() {
        let fid = [0xab; 32];
        let f = chunk_frame(&fid, 70_000, b"data");
        let (h, off, d) = parse_chunk(&f).unwrap();
        assert_eq!(
            (h.as_str(), off, d),
            (hex(&fid).as_str(), 70_000, &b"data"[..])
        );
        assert_eq!(unhex32(&h), Some(fid));
        assert!(parse_chunk(&f[..39]).is_none());
        assert!(unhex32("zz").is_none());
    }

    #[test]
    fn backoff_grows_with_jitter_and_caps() {
        assert_eq!(backoff(0, 0.0), Duration::from_secs(5));
        assert_eq!(backoff(0, 0.999).as_secs(), 9);
        assert_eq!(backoff(1, 0.5), Duration::from_secs(15));
        assert_eq!(backoff(3, 0.0), Duration::from_secs(40));
        assert_eq!(backoff(30, 0.9), Duration::from_secs(600));
    }

    #[test]
    fn both_ends_pick_the_same_link() {
        assert_eq!(keep_dialed_by("alex", "fred"), "alex");
        assert_eq!(keep_dialed_by("fred", "alex"), "alex");
    }

    #[test]
    fn window_is_half_a_second_within_limits() {
        assert_eq!(window_for(1_000.0), 64 * 1024);
        assert_eq!(window_for(1_000_000.0), 500_000);
        assert_eq!(window_for(1e12), 4 << 20);
        let mut r = Rate::default();
        let t = Instant::now();
        r.acked(t, 0);
        r.acked(t + Duration::from_secs(1), 10_000_000);
        assert!(r.bps > 3_000_000.0, "{}", r.bps);
    }

    #[test]
    fn names_translate_between_the_two_stores() {
        // fred's store says "bia" for fred and "alex" for us; here fred is "fred", we are "bia"
        assert_eq!(name_here("bia", "fred", "alex"), "fred");
        assert_eq!(name_here("alex", "fred", "alex"), "bia");
        assert_eq!(name_here("root", "fred", "alex"), "root");
    }

    #[test]
    fn message_ids() {
        assert!(valid_msg_id("b1234abcd0001") && valid_msg_id("w-9f_x"));
        assert!(!valid_msg_id("abc") && !valid_msg_id("a/b/c/d") && !valid_msg_id(&"x".repeat(49)));
    }

    #[test]
    fn shutdown_reasons() {
        assert_eq!(
            reason_from_jobs("123 reboot.target start waiting\n"),
            Some("reboot")
        );
        assert_eq!(
            reason_from_jobs("9 poweroff.target start running"),
            Some("poweroff")
        );
        assert_eq!(reason_from_jobs("No jobs running."), None);
        assert_eq!(
            reason_from_file(&json!({"reason": "update", "at": 1000}), 1100),
            Some("update".into())
        );
        assert_eq!(
            reason_from_file(&json!({"reason": "update", "at": 1000}), 1200),
            None
        );
        assert_eq!(
            reason_from_file(&json!({"reason": "evil", "at": 1000}), 1000),
            None
        );
    }
}
