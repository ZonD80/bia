//! The peer link protocol without I/O: frame encoding, timers, flow control, the
//! origin chain and the small decisions both ends must agree on. `peer.rs` drives it.
//!
//! One WebSocket per pair of bias (`GET /ws/peer`, mutual TLS with the peer certificates).
//! Text frames are JSON `{t, …}`; binary frames are file chunks
//! `[fid: 32 bytes sha256][offset: u64 BE][data]`.
//!
//! - `hello {name, ver, proto, instance, rx}` first, both ways (`rx`: last seq received).
//! - Sequenced (`seq`, acked with `ack {seq}`, resent after a reconnect when not acked):
//!   `msg {id, ts, text, reply_to?, files: [{fid, name, size}], origin: {id, path}}`,
//!   `edit {id, text}`, `react {id, emoji}`, `del {ids}`.
//! - Not sequenced: `read {id}`, `typing {on}`, `renew {csr}` → `renewed {cert}` /
//!   `renew_err {error}`, `bye {reason}`.
//! - Files are pulled by the receiver: `fget {fid, off}` → chunks, `fack {fid, off}` (bytes
//!   it has, the sender's flow control), `fdone {fid}`, `frefuse {fid, reason}`, and from
//!   the sender `fcancel {fid}`.
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

pub const PROTO: u64 = 1;
/// Ping cadence, and how long a silent link lives.
pub const PING_EVERY: Duration = Duration::from_secs(10);
pub const DEAD_AFTER: Duration = Duration::from_secs(30);
/// A link that drops without `bye` and is not back within this is reported lost.
pub const LOST_AFTER: Duration = Duration::from_secs(30);
/// A second machine with the same key: how long the existing link has to answer a ping.
pub const PROBE: Duration = Duration::from_secs(5);
/// Flush time for `bye` frames at shutdown.
pub const BYE_FLUSH: Duration = Duration::from_secs(2);
pub const MAX_LINKS: usize = 64;
pub const MAX_HANDSHAKES: usize = 4;
/// Handshakes from one address per minute (several bias may share one NAT address).
pub const IP_HANDSHAKES_PER_MIN: usize = 30;
/// File chunk payload.
pub const CHUNK: usize = 32 * 1024;
/// Files pulled at once: per peer and over all peers.
pub const FILES_PER_PEER: usize = 2;
pub const FILES_TOTAL: usize = 4;
pub const MAX_FILE: u64 = 4 << 30;
/// Free space kept after a file lands.
pub const DISK_MARGIN: u64 = 512 << 20;
/// Largest frame accepted (a chunk plus header, or a JSON frame).
pub const MAX_FRAME: u64 = 2 << 20;
/// Sequenced frames per minute before the reader slows down.
pub const RATE_PER_MIN: usize = 120;
/// Unfinished transfers and the sender's file list are kept this long.
pub const KEEP_PARTIAL_SECS: u64 = 3 * 86_400;
/// Renew this bia's certificate at a peer this many days before it expires.
pub const RENEW_WITHIN_DAYS: i64 = 30;

/// Reasons in `bye`.
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

/// Two links for one pair (both dialed at once): both ends keep the one dialed by the
/// lower agent name.
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

/// Lost/back notices: more than 5 within 10 minutes makes one "unstable" notice, then
/// silence until the link has stayed up for 10 minutes.
#[derive(Debug, Default)]
pub struct Flap {
    times: VecDeque<Instant>,
    pub flapping: bool,
}

pub enum Gate {
    Tell,
    /// The notice that replaces the next ones.
    Unstable(usize),
    Quiet,
}

pub const FLAP_WINDOW: Duration = Duration::from_secs(600);
pub const FLAP_MAX: usize = 5;

impl Flap {
    pub fn gate(&mut self, now: Instant) -> Gate {
        while self
            .times
            .front()
            .is_some_and(|t| now.duration_since(*t) > FLAP_WINDOW)
        {
            self.times.pop_front();
        }
        self.times.push_back(now);
        if self.flapping {
            return Gate::Quiet;
        }
        if self.times.len() > FLAP_MAX {
            self.flapping = true;
            return Gate::Unstable(self.times.len());
        }
        Gate::Tell
    }
    /// Up since `up` at `now`: true once when a flapping link has settled.
    pub fn settled(&mut self, up: Instant, now: Instant) -> bool {
        if self.flapping && now.duration_since(up) >= FLAP_WINDOW {
            self.flapping = false;
            self.times.clear();
            return true;
        }
        false
    }
}

/// A sequenced frame from the peer: new (handle it) or one already handled (ack only).
pub fn is_new_seq(rx: u64, seq: u64) -> bool {
    seq > rx
}

/// The request a message belongs to: `id` names it where it began (`<bia>/<message>`),
/// `path` is who passed it on, oldest first (a person, then bias).
#[derive(Clone, Debug, PartialEq)]
pub struct Origin {
    pub id: String,
    pub path: Vec<String>,
}

/// The origin of a message this bia sends. `arg` comes from the agent: `{id, path}` when
/// the turn handles a peer's request (passed on), `{new, from}` when a person asked here.
pub fn origin_out(arg: Option<&Value>, me: &str, mint: &str) -> Origin {
    let s = |v: &Value, k: &str| {
        v.get(k)
            .and_then(|x| x.as_str())
            .map(str::trim)
            .unwrap_or("")
            .to_string()
    };
    match arg {
        Some(a) if !s(a, "id").is_empty() => {
            let mut path: Vec<String> = a
                .get("path")
                .and_then(|p| p.as_array())
                .map(|p| {
                    p.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            if path.last().map(String::as_str) != Some(me) {
                path.push(me.to_string());
            }
            Origin {
                id: s(a, "id"),
                path,
            }
        }
        Some(a) if !s(a, "new").is_empty() => {
            let from = s(a, "from");
            let mut path = vec![];
            if !from.is_empty() && from != me {
                path.push(from);
            }
            path.push(me.to_string());
            Origin {
                id: format!("{me}/{}", s(a, "new")),
                path,
            }
        }
        _ => Origin {
            id: format!("{me}/{mint}"),
            path: vec![me.to_string()],
        },
    }
}

impl Origin {
    pub fn to_json(&self) -> Value {
        json!({"id": self.id, "path": self.path})
    }
    pub fn from_json(v: &Value) -> Option<Origin> {
        let id = v.get("id")?.as_str()?.trim();
        if id.is_empty() || id.len() > 200 {
            return None;
        }
        let path = v
            .get("path")
            .and_then(|p| p.as_array())
            .map(|p| {
                p.iter()
                    .filter_map(|x| x.as_str())
                    .map(|x| {
                        x.replace(['→', '[', ']', ':'], "")
                            .trim()
                            .chars()
                            .take(64)
                            .collect::<String>()
                    })
                    .filter(|x| !x.is_empty())
                    .take(16)
                    .collect()
            })
            .unwrap_or_default();
        Some(Origin {
            id: id.replace(['[', ']', ' '], ""),
            path,
        })
    }
    /// `[request alex/b17: root → alex → you]` — the agent reads id and path back from it.
    pub fn header(&self) -> String {
        let mut p = self.path.clone();
        p.push("you".into());
        format!("[request {}: {}]", self.id, p.join(" → "))
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
pub fn reason_from_file(v: &Value, now: u64) -> Option<String> {
    let r = v.get("reason")?.as_str()?.trim();
    let at = v.get("at")?.as_u64()?;
    (BYE_RESTARTS.contains(&r) && now.saturating_sub(at) <= 120).then(|| r.to_string())
}

/// What the model is told when a peer said bye.
pub fn bye_text(peer: &str, reason: &str) -> String {
    match reason {
        "reboot" => format!("{peer} is rebooting (it said bye). It reconnects when it is back, and this bia keeps trying too; a notice comes when the link is up."),
        "poweroff" => format!("{peer} is powering off (it said bye). This bia keeps trying to reconnect; a notice comes if it comes back."),
        "update" => format!("{peer} is updating (it said bye). It reconnects when it is back, and this bia keeps trying too."),
        "restart" | "shutdown" => format!("{peer}'s agent is restarting (it said bye). It reconnects when it is back, and this bia keeps trying too."),
        "disconnect" => format!("{peer} closed the link. Nothing reconnects by itself: web_peer_connect(peer=\"{peer}\") opens it again if you need it; web_peer_ban stops {peer} from coming back."),
        "banned" => format!("{peer} banned this bia: it refuses the link and messages. Nothing is sent there until it unbans this bia."),
        "revoked" => format!("{peer} no longer trusts this bia (its certificate here was revoked). Tell root; trust must be set up again (web_peer_add) to talk."),
        "duplicate_key" => format!("{peer} refused this connection: another machine using this bia's key is connected to it. Tell root now: one of the two machines is a copy."),
        "busy" => format!("{peer} has too many links open and refused this one; this bia tries again later."),
        other => format!("{peer} closed the link ({other})."),
    }
}

pub fn human_secs(s: u64) -> String {
    match s {
        0..=89 => format!("{s} s"),
        90..=5399 => format!("{} min", (s + 30) / 60),
        5400..=172_799 => format!("{} h", (s + 1800) / 3600),
        _ => format!("{} days", (s + 43_200) / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn flapping_gives_one_notice_then_settles() {
        let mut f = Flap::default();
        let t = Instant::now();
        for i in 0..5 {
            assert!(matches!(f.gate(t + Duration::from_secs(i)), Gate::Tell));
        }
        assert!(matches!(
            f.gate(t + Duration::from_secs(6)),
            Gate::Unstable(6)
        ));
        assert!(matches!(f.gate(t + Duration::from_secs(7)), Gate::Quiet));
        assert!(!f.settled(t + Duration::from_secs(8), t + Duration::from_secs(60)));
        assert!(f.settled(t + Duration::from_secs(8), t + Duration::from_secs(700)));
        assert!(matches!(f.gate(t + Duration::from_secs(800)), Gate::Tell));
        // spread out over more than 10 minutes: never flapping
        let mut g = Flap::default();
        for i in 0..20 {
            assert!(matches!(
                g.gate(t + Duration::from_secs(i * 200)),
                Gate::Tell
            ));
        }
    }

    #[test]
    fn origin_chain() {
        let o = origin_out(Some(&json!({"new": "b17", "from": "root"})), "alex", "x");
        assert_eq!(
            o,
            Origin {
                id: "alex/b17".into(),
                path: vec!["root".into(), "alex".into()]
            }
        );
        assert_eq!(o.header(), "[request alex/b17: root → alex → you]");
        let passed = origin_out(
            Some(&json!({"id": "alex/b17", "path": ["root", "alex"]})),
            "fred",
            "x",
        );
        assert_eq!(passed.path, ["root", "alex", "fred"]);
        assert_eq!(passed.id, "alex/b17");
        let none = origin_out(None, "fred", "b9");
        assert_eq!(
            none,
            Origin {
                id: "fred/b9".into(),
                path: vec!["fred".into()]
            }
        );
        let back =
            Origin::from_json(&json!({"id": "alex/b17", "path": ["root", "al]ex → x"]})).unwrap();
        assert_eq!(back.path, ["root", "alex  x"]);
        assert!(Origin::from_json(&json!({"id": ""})).is_none());
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
        assert!(is_new_seq(4, 5) && !is_new_seq(5, 5) && !is_new_seq(5, 3));
    }
}
