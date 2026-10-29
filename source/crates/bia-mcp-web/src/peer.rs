//! Live links with other bias (peers): one WebSocket per peer over mutual TLS, used like a
//! chat bus. A peer's messages arrive as `<peer>|peer:<peer>|…` inbound envelopes; the agent
//! answers with say on that channel (the web MCP's send & co.). Nothing is sent while the
//! link is down: a live connection is required.
//!
//! Either side dials (`GET /ws/peer` with its peer certificate) and keeps redialing after a
//! network loss or the other side's restart; a deliberate disconnect or a ban stops that.
//! The model hears about every link change. Files ride the same link (pulled by the
//! receiver, resumable, delivered with their message once complete). See `peerproto.rs`.
use crate::bus::{mint_msg_id, Bus, Inbound, ID_SYSTEM};
use crate::peerproto::*;
use crate::tlspipe::SharedTls;
use bia_mcp::{arg_str_list, arg_str_opt, Ctx};
use bia_pki::{client, ClientId, Pki};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const CH_PREFIX: &str = "peer:";

pub fn is_peer(channel: &str) -> bool {
    channel.starts_with(CH_PREFIX)
}

pub fn channel(name: &str) -> String {
    format!("{CH_PREFIX}{name}")
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn rnd_bytes<const N: usize>() -> [u8; N] {
    use ring::rand::SecureRandom;
    let mut b = [0u8; N];
    let _ = ring::rand::SystemRandom::new().fill(&mut b);
    b
}

fn rnd01() -> f64 {
    u32::from_be_bytes(rnd_bytes::<4>()) as f64 / (u32::MAX as f64 + 1.0)
}

/// What a link to one peer remembers across restarts (`state/peers/<name>/link.json`).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LinkState {
    /// Keep the link up: redial after a network loss or the peer's restart.
    #[serde(default)]
    pub want: bool,
    /// "", "root" or "model": this bia refuses the peer.
    #[serde(default)]
    pub banned_by: String,
    /// The peer refused this bia (bye banned).
    #[serde(default)]
    pub banned_us: bool,
    /// This bia has dialed the peer successfully at least once.
    #[serde(default)]
    pub reached: bool,
    /// The model was told the link came up for the first time.
    #[serde(default)]
    pub greeted: bool,
    /// Why the peer cannot link (an older bia), already told; "" when fine.
    #[serde(default)]
    pub cannot: String,
    #[serde(default)]
    pub last_up: u64,
    #[serde(default)]
    pub last_down: u64,
    #[serde(default)]
    pub next_seq: u64,
    /// Highest sequence number received from the peer.
    #[serde(default)]
    pub rx: u64,
    /// Sent, not acknowledged yet (resent after a quick reconnect).
    #[serde(default)]
    pub unacked: Vec<Value>,
    /// Received and acknowledged, waiting for files or for an earlier message.
    #[serde(default)]
    pub inbox: Vec<Pending>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pending {
    pub frame: Value,
    #[serde(default)]
    pub files: Vec<RxFile>,
    #[serde(default)]
    pub at: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RxFile {
    pub fid: String,
    pub name: String,
    pub size: u64,
    /// Where it landed (set when complete).
    #[serde(default)]
    pub path: String,
    /// Why it did not arrive.
    #[serde(default)]
    pub err: String,
}

impl RxFile {
    fn settled(&self) -> bool {
        !self.path.is_empty() || !self.err.is_empty()
    }
}

/// A file this bia offered to a peer (`state/peers/<name>/sent.json`).
#[derive(Clone, Debug, Serialize, Deserialize)]
struct OutFile {
    path: String,
    name: String,
    size: u64,
    mtime: u64,
    at: u64,
    #[serde(default)]
    done: bool,
    /// The message it went with.
    #[serde(default)]
    msg: String,
}

/// One line of `state/peers/<name>/chat.jsonl`.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct ChatLine {
    id: String,
    /// The peer's name, or "me".
    from: String,
    ts: u64,
    #[serde(default)]
    text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    files: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reply_to: Option<String>,
    /// "msg", "edit", "react", "del"
    #[serde(default = "kind_msg")]
    kind: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    ids: Vec<String>,
}

fn kind_msg() -> String {
    "msg".into()
}

// ---------------------------------------------------------------- the writer

enum Item {
    Frame(u8, Vec<u8>),
    Stop,
}

struct Job {
    fid: String,
    raw: [u8; 32],
    path: PathBuf,
    off: u64,
    end: u64,
    acked: u64,
}

#[derive(Default)]
struct Wq {
    hi: VecDeque<(u8, Vec<u8>)>,
    jobs: VecDeque<Job>,
    rate: Rate,
    closing: bool,
    done: bool,
}

/// One link's outgoing side: JSON and control frames first, file chunks while fewer than
/// a window's worth are unacknowledged, a ping when idle.
struct Writer {
    q: Mutex<Wq>,
    cv: Condvar,
}

impl Writer {
    fn push(&self, op: u8, payload: Vec<u8>) {
        let mut q = self.q.lock().unwrap();
        if !q.closing {
            q.hi.push_back((op, payload));
        }
        self.cv.notify_all();
    }
    fn json(&self, v: &Value) {
        self.push(1, v.to_string().into_bytes());
    }
    /// Send `bye`, then stop after what is queued.
    fn close_with(&self, v: Option<&Value>) {
        let mut q = self.q.lock().unwrap();
        if let Some(v) = v {
            if !q.closing {
                q.hi.push_back((1, v.to_string().into_bytes()));
            }
        }
        q.closing = true;
        q.jobs.clear();
        self.cv.notify_all();
    }
    fn wait_done(&self, until: Instant) -> bool {
        let mut q = self.q.lock().unwrap();
        while !q.done {
            let now = Instant::now();
            if now >= until {
                return false;
            }
            q = self.cv.wait_timeout(q, until - now).unwrap().0;
        }
        true
    }

    fn next(&self, last_write: Instant) -> Item {
        let mut q = self.q.lock().unwrap();
        loop {
            if let Some((op, f)) = q.hi.pop_front() {
                return Item::Frame(op, f);
            }
            if q.closing {
                return Item::Stop;
            }
            let window = window_for(q.rate.bps);
            if let Some(i) = q
                .jobs
                .iter()
                .position(|j| j.off < j.end && j.off - j.acked < window)
            {
                let mut j = q.jobs.remove(i).unwrap();
                let n = (CHUNK as u64).min(j.end - j.off) as usize;
                let mut buf = vec![0u8; n];
                let ok = fs::File::open(&j.path)
                    .and_then(|mut f| {
                        f.seek(SeekFrom::Start(j.off))?;
                        f.read_exact(&mut buf)
                    })
                    .is_ok();
                if !ok {
                    // the file changed or went away: the receiver checks the hash anyway
                    let fid = j.fid.clone();
                    q.hi.push_back((
                        1,
                        json!({"t": "frefuse", "fid": fid, "reason": "the file is gone here"})
                            .to_string()
                            .into_bytes(),
                    ));
                    continue;
                }
                let frame = chunk_frame(&j.raw, j.off, &buf);
                j.off += n as u64;
                q.jobs.push_back(j);
                return Item::Frame(2, frame);
            }
            let idle = last_write.elapsed();
            if idle >= PING_EVERY {
                return Item::Frame(9, b"p".to_vec());
            }
            q = self.cv.wait_timeout(q, PING_EVERY - idle).unwrap().0;
        }
    }

    fn acked(&self, fid: &str, off: u64) {
        let mut q = self.q.lock().unwrap();
        let mut total = 0;
        let mut finished = false;
        if let Some(j) = q.jobs.iter_mut().find(|j| j.fid == fid) {
            j.acked = off.min(j.end);
            total = j.acked;
            finished = j.acked >= j.end;
        }
        if total > 0 {
            q.rate.acked(Instant::now(), total);
        }
        if finished {
            q.jobs.retain(|j| j.fid != fid);
        }
        self.cv.notify_all();
    }

    fn drop_job(&self, fid: &str) {
        let mut q = self.q.lock().unwrap();
        q.jobs.retain(|j| j.fid != fid);
        self.cv.notify_all();
    }

    fn add_job(&self, j: Job) {
        let mut q = self.q.lock().unwrap();
        q.jobs.retain(|x| x.fid != j.fid);
        q.rate.restart();
        q.jobs.push_back(j);
        self.cv.notify_all();
    }

    fn sending(&self) -> Vec<(String, u64, u64)> {
        self.q
            .lock()
            .unwrap()
            .jobs
            .iter()
            .map(|j| (j.fid.clone(), j.acked, j.end))
            .collect()
    }
}

// ---------------------------------------------------------------- one link

pub struct Link {
    id: u64,
    peer: String,
    /// This bia dialed it.
    dialed: bool,
    ip: String,
    up: Instant,
    instance: Mutex<String>,
    w: Arc<Writer>,
    tls: SharedTls,
    last_rx: Mutex<Instant>,
    their_bye: Mutex<Option<String>>,
    our_bye: Mutex<Option<String>>,
    /// Replaced by another link to the same peer: closing it says nothing.
    replaced: AtomicBool,
}

impl Link {
    fn close(&self, reason: &str) {
        *self.our_bye.lock().unwrap() = Some(reason.to_string());
        self.w
            .close_with(Some(&json!({"t": "bye", "reason": reason})));
        let tls = self.tls.clone();
        let w = self.w.clone();
        std::thread::spawn(move || {
            w.wait_done(Instant::now() + BYE_FLUSH);
            tls.shutdown();
        });
    }
    fn silent_close(&self, reason: &str) {
        self.replaced.store(true, Ordering::Relaxed);
        self.close(reason);
    }
}

// ---------------------------------------------------------------- the registry

#[derive(Default)]
struct Rt {
    st: LinkState,
    loaded: bool,
    link: Option<Arc<Link>>,
    dialing: bool,
    attempts: u32,
    next_dial: Option<Instant>,
    /// Down without bye, not reported yet.
    down_since: Option<Instant>,
    /// The model knows the link is down: tell it when it is back.
    told_down: bool,
    /// It went down by network loss (counts towards flapping), not by a goodbye.
    net_down: bool,
    flap: Flap,
    last_err: String,
    err_at: Option<Instant>,
    /// Files being pulled from this peer.
    pulls: HashSet<String>,
    rate: VecDeque<Instant>,
}

#[derive(Default)]
struct Inner {
    rt: HashMap<String, Rt>,
    handshakes: usize,
    ip_hits: HashMap<String, VecDeque<Instant>>,
    next_id: u64,
}

pub struct Links {
    bus: Arc<Bus>,
    files: PathBuf,
    /// Random per process: tells a restart from a second machine with the same key.
    instance: String,
    inner: Mutex<Inner>,
    changed: Condvar,
    closing: AtomicBool,
    started: Instant,
    ticks: AtomicU64,
    /// Origins seen (`state/peers/origins.json`).
    origins: Mutex<()>,
}

static LINKS: OnceLock<Arc<Links>> = OnceLock::new();

pub fn links() -> Option<&'static Arc<Links>> {
    LINKS.get()
}

/// Start the link manager (once): dial wanted peers, report changes, retire old files.
pub fn init(files: &Path, bus: Arc<Bus>) -> Arc<Links> {
    let l = LINKS
        .get_or_init(|| {
            Arc::new(Links {
                bus,
                files: files.to_path_buf(),
                instance: hex(&rnd_bytes::<8>()),
                inner: Mutex::new(Inner::default()),
                changed: Condvar::new(),
                closing: AtomicBool::new(false),
                started: Instant::now(),
                ticks: AtomicU64::new(0),
                origins: Mutex::new(()),
            })
        })
        .clone();
    let t = l.clone();
    let _ = std::thread::Builder::new()
        .name("peer-tick".into())
        .spawn(move || loop {
            std::thread::sleep(Duration::from_secs(1));
            if t.closing.load(Ordering::Relaxed) {
                return;
            }
            t.tick();
        });
    l
}

/// Shut every link with `bye {reason}` and wait (2 s at most) for those to go out.
pub fn shutdown_all(reason: &str) {
    if let Some(l) = links() {
        l.shutdown(reason);
    }
}

/// Why this process is stopping: the agent's reason file (update), else systemd's queued
/// job (reboot, poweroff), else the agent or this MCP restarting.
pub fn shutdown_reason(files: &Path) -> String {
    // bia-update writes /run/bia/shutdown-reason (files may be folded away by then)
    for f in [
        PathBuf::from("/run/bia/shutdown-reason"),
        files.join("state").join("shutdown-reason"),
    ] {
        if let Some(r) = fs::read(&f)
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| reason_from_file(&v, now_secs()))
        {
            return r;
        }
    }
    let jobs = std::process::Command::new("systemctl")
        .args(["list-jobs", "--no-legend", "--no-pager"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    reason_from_jobs(&jobs).unwrap_or("restart").to_string()
}

impl Links {
    fn pki(&self) -> Result<Pki, String> {
        Pki::load_or_create(&self.files).map_err(|e| format!("pki: {e:#}"))
    }

    fn me(&self) -> String {
        self.pki()
            .map(|p| p.agent_name())
            .unwrap_or_else(|_| "bia".into())
    }

    fn dir(&self, name: &str) -> PathBuf {
        self.files.join("state").join("peers").join(name)
    }

    fn inbox_dir(&self, name: &str) -> PathBuf {
        self.files.join("inbox").join("peer").join(name)
    }

    fn part_path(&self, name: &str, fid: &str) -> PathBuf {
        self.inbox_dir(name).join(".part").join(fid)
    }

    /// This process runs as root; what it writes under files belongs to the files owner
    /// (bia's backups and tools run as that user). `p` and its folders up to files.
    fn own(&self, p: &Path) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let Ok(m) = fs::metadata(&self.files) else {
                return;
            };
            if unsafe { libc::geteuid() } != 0 || m.uid() == 0 {
                return;
            }
            let mut cur = Some(p);
            while let Some(c) = cur {
                if c == self.files || !c.starts_with(&self.files) {
                    break;
                }
                let _ = std::os::unix::fs::chown(c, Some(m.uid()), Some(m.gid()));
                cur = c.parent();
            }
        }
    }

    fn load_state(&self, name: &str) -> Option<LinkState> {
        fs::read(self.dir(name).join("link.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
    }

    fn save_state(&self, name: &str, st: &LinkState) {
        let d = self.dir(name);
        let _ = fs::create_dir_all(&d);
        let tmp = d.join("link.json.tmp");
        if fs::write(&tmp, serde_json::to_vec_pretty(st).unwrap_or_default()).is_ok() {
            let _ = fs::rename(&tmp, d.join("link.json"));
            self.own(&d.join("link.json"));
        }
    }

    /// The runtime record of `name`, loaded from disk the first time.
    fn rt<'a>(&self, inner: &'a mut Inner, name: &str) -> &'a mut Rt {
        let rt = inner.rt.entry(name.to_string()).or_default();
        if !rt.loaded {
            rt.loaded = true;
            match self.load_state(name) {
                Some(st) => rt.st = st,
                None => {
                    // a new peer: keep a link up from the start
                    rt.st.want = true;
                    self.save_state(name, &rt.st);
                }
            }
        }
        rt
    }

    /// A notice for the model on the peer's channel (wakes it).
    fn tell(&self, peer: &str, text: &str) {
        if self.closing.load(Ordering::Relaxed) {
            return;
        }
        self.bus.log(&format!(
            "  peer {peer}: {}",
            text.lines().next().unwrap_or("")
        ));
        self.bus.push_inbound(Inbound {
            identity: ID_SYSTEM.into(),
            channel: channel(peer),
            id: mint_msg_id(),
            text: format!("[peer {peer}] {text}"),
        });
    }

    /// Lost/back notices go through the flap gate.
    fn tell_gated(&self, rt: &mut Rt, peer: &str, text: &str) {
        match rt.flap.gate(Instant::now()) {
            Gate::Tell => self.tell(peer, text),
            Gate::Unstable(n) => self.tell(
                peer,
                &format!("The link with {peer} is unstable ({n} drops and returns within 10 min). This bia keeps reconnecting by itself and stays quiet about it until the link has held for 10 min."),
            ),
            Gate::Quiet => {}
        }
    }

    fn fail_unacked(&self, rt: &mut Rt, peer: &str, why: &str) {
        if rt.st.unacked.is_empty() {
            return;
        }
        let lines: Vec<String> = rt
            .st
            .unacked
            .drain(..)
            .map(|f| {
                let what = match f["t"].as_str().unwrap_or("") {
                    "msg" => {
                        let t: String = f["text"].as_str().unwrap_or("").chars().take(60).collect();
                        format!("message {} \"{t}\"", f["id"].as_str().unwrap_or(""))
                    }
                    "edit" => format!("edit of {}", f["id"].as_str().unwrap_or("")),
                    "react" => format!("reaction on {}", f["id"].as_str().unwrap_or("")),
                    other => other.to_string(),
                };
                format!("- {what}")
            })
            .collect();
        self.tell(
            peer,
            &format!(
                "Not delivered to {peer} ({why}); send again once the link is up:\n{}",
                lines.join("\n")
            ),
        );
    }

    // ---- admission (server side, before the WebSocket upgrade) ----

    /// Handshakes from `ip`: at most 10 a minute. Err = HTTP status.
    pub fn admit(&self, ip: &str) -> Result<(), u16> {
        let mut inner = self.inner.lock().unwrap();
        let now = Instant::now();
        let hits = inner.ip_hits.entry(ip.to_string()).or_default();
        while hits
            .front()
            .is_some_and(|t| now.duration_since(*t) > Duration::from_secs(60))
        {
            hits.pop_front();
        }
        if hits.len() >= IP_HANDSHAKES_PER_MIN {
            return Err(429);
        }
        hits.push_back(now);
        Ok(())
    }

    /// A peer dialed this bia (server thread, after `101`): run the link here.
    pub fn accept(
        self: &Arc<Self>,
        tls: SharedTls,
        reader: BufReader<SharedTls>,
        id: &ClientId,
        ip: &str,
    ) {
        let busy = {
            let inner = self.inner.lock().unwrap();
            inner.rt.values().filter(|r| r.link.is_some()).count() >= MAX_LINKS
                && !inner.rt.get(&id.identity).is_some_and(|r| r.link.is_some())
        };
        if busy || self.closing.load(Ordering::Relaxed) {
            let mut t = tls.clone();
            let _ = t.write_all(&encode_frame(
                1,
                json!({"t": "bye", "reason": "busy"}).to_string().as_bytes(),
                None,
            ));
            let _ = t.flush();
            tls.shutdown();
            return;
        }
        self.run_link(
            id.identity.clone(),
            false,
            tls,
            Box::new(reader),
            ip.to_string(),
        );
    }

    // ---- dialing ----

    fn dial(self: &Arc<Self>, name: &str) -> Result<(), String> {
        let pki = self.pki()?;
        let p = pki.peer(name).ok_or_else(|| format!("no peer {name}"))?;
        let (ca, cert, key) = pki.peer_tls(name).map_err(|e| format!("{e:#}"))?;
        let (conn, tcp) =
            client::connect(&p.url, client::ServerTrust::Ca(&ca), Some((&cert, &key)))
                .map_err(|e| format!("{e:#}"))?;
        set_sndbuf(&tcp);
        let ip = tcp
            .peer_addr()
            .map(|a| a.ip().to_string())
            .unwrap_or_default();
        let (host, port) = client::split_url(&p.url).map_err(|e| format!("{e:#}"))?;
        let tls = SharedTls::new(conn, tcp).map_err(|e| e.to_string())?;
        let key = base64_std(&rnd_bytes::<16>());
        let hosthdr = if host.contains(':') {
            format!("[{host}]:{port}")
        } else {
            format!("{host}:{port}")
        };
        let req = format!(
            "GET /ws/peer HTTP/1.1\r\nHost: {hosthdr}\r\nUser-Agent: bia-peer\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
        );
        let mut w = tls.clone();
        w.write_all(req.as_bytes())
            .map_err(|e| format!("send: {e}"))?;
        w.flush().map_err(|e| format!("send: {e}"))?;
        let mut reader = BufReader::new(tls.clone());
        let mut status = String::new();
        reader
            .read_line(&mut status)
            .map_err(|e| format!("read: {e}"))?;
        loop {
            let mut h = String::new();
            if reader.read_line(&mut h).map_err(|e| format!("read: {e}"))? == 0
                || h.trim().is_empty()
            {
                break;
            }
        }
        let code: u16 = status
            .split_whitespace()
            .nth(1)
            .and_then(|c| c.parse().ok())
            .unwrap_or(0);
        match code {
            101 => {}
            404 => {
                tls.shutdown();
                return Err("OLD".into());
            }
            403 | 401 => {
                tls.shutdown();
                return Err("NOTPEER".into());
            }
            other => {
                tls.shutdown();
                return Err(format!("{} answered HTTP {other}", p.url));
            }
        }
        {
            let mut inner = self.inner.lock().unwrap();
            let rt = self.rt(&mut inner, name);
            if !rt.st.reached {
                rt.st.reached = true;
                self.save_state(name, &rt.st);
            }
        }
        let me = self.clone();
        let name = name.to_string();
        let _ = std::thread::Builder::new()
            .name(format!("peer-{name}"))
            .spawn(move || me.run_link(name, true, tls, Box::new(reader), ip));
        Ok(())
    }

    fn start_dial(self: &Arc<Self>, name: &str) {
        let me = self.clone();
        let name = name.to_string();
        let _ = std::thread::Builder::new().name(format!("peer-dial-{name}")).spawn(move || {
            let r = me.dial(&name);
            let mut inner = me.inner.lock().unwrap();
            inner.handshakes = inner.handshakes.saturating_sub(1);
            let old_msg = |n: &str| format!("{n} runs an older bia without peer links (no /ws/peer). It must update before you can talk; this bia tries again every hour.");
            let rt = me.rt(&mut inner, &name);
            rt.dialing = false;
            match r {
                Ok(()) => {
                    // the hello is on its way; do not dial again meanwhile
                    rt.next_dial = Some(Instant::now() + backoff(0, rnd01()));
                    if !rt.st.cannot.is_empty() {
                        rt.st.cannot.clear();
                        me.save_state(&name, &rt.st);
                    }
                }
                Err(e) if e == "OLD" => {
                    rt.last_err = format!("{name} runs an older bia without peer links");
                    rt.err_at = Some(Instant::now());
                    rt.next_dial = Some(Instant::now() + Duration::from_secs(3600));
                    if rt.st.cannot.is_empty() {
                        rt.st.cannot = "old".into();
                        me.save_state(&name, &rt.st);
                        let t = old_msg(&name);
                        drop(inner);
                        me.tell(&name, &t);
                        me.changed.notify_all();
                        return;
                    }
                }
                Err(e) if e == "NOTPEER" => {
                    rt.err_at = Some(Instant::now());
                    rt.next_dial = Some(Instant::now() + backoff(rt.attempts.min(4), rnd01()));
                    rt.attempts = rt.attempts.saturating_add(1);
                    if !rt.st.greeted {
                        // trust was just set up: it has not collected its certificate yet
                        rt.last_err = format!("{name} does not know this bia as a peer yet");
                    } else {
                        rt.last_err = format!("{name} does not accept this bia's certificate any more");
                        if rt.st.cannot.is_empty() {
                            rt.st.cannot = "notpeer".into();
                            rt.next_dial = Some(Instant::now() + Duration::from_secs(3600));
                            me.save_state(&name, &rt.st);
                            drop(inner);
                            me.tell(&name, &format!("{name} refused this bia's certificate (it no longer has this bia as a peer). Tell root; trust must be set up again to talk. This bia tries again every hour."));
                            me.changed.notify_all();
                            return;
                        }
                    }
                }
                Err(e) => {
                    rt.last_err = e;
                    rt.err_at = Some(Instant::now());
                    rt.next_dial = Some(Instant::now() + backoff(rt.attempts, rnd01()));
                    rt.attempts = rt.attempts.saturating_add(1);
                }
            }
            drop(inner);
            me.changed.notify_all();
        });
    }

    // ---- the link itself (runs in its reader thread) ----

    fn run_link(
        self: &Arc<Self>,
        name: String,
        dialed: bool,
        tls: SharedTls,
        mut reader: Box<dyn Read + Send>,
        ip: String,
    ) {
        let _ = tls.set_read_timeout(Some(DEAD_AFTER));
        let _ = tls.set_nodelay(true);
        let id = {
            let mut inner = self.inner.lock().unwrap();
            inner.next_id += 1;
            inner.next_id
        };
        let w = Arc::new(Writer {
            q: Mutex::new(Wq::default()),
            cv: Condvar::new(),
        });
        let link = Arc::new(Link {
            id,
            peer: name.clone(),
            dialed,
            ip,
            up: Instant::now(),
            instance: Mutex::new(String::new()),
            w: w.clone(),
            tls: tls.clone(),
            last_rx: Mutex::new(Instant::now()),
            their_bye: Mutex::new(None),
            our_bye: Mutex::new(None),
            replaced: AtomicBool::new(false),
        });
        // writer thread
        {
            let mut out = tls.clone();
            let w = w.clone();
            let _ = std::thread::Builder::new()
                .name(format!("peer-w-{name}"))
                .spawn(move || {
                    let mut last = Instant::now();
                    loop {
                        match w.next(last) {
                            Item::Frame(op, payload) => {
                                let mask = if dialed { Some(rnd_bytes::<4>()) } else { None };
                                let f = encode_frame(op, &payload, mask);
                                if out.write_all(&f).and_then(|_| out.flush()).is_err() {
                                    break;
                                }
                                last = Instant::now();
                            }
                            Item::Stop => break,
                        }
                    }
                    let mut q = w.q.lock().unwrap();
                    q.done = true;
                    q.closing = true;
                    w.cv.notify_all();
                });
        }
        // the dialer says hello first; the other side answers with hello once it took the
        // link (or bye when it refuses: banned, a duplicate, a second machine)
        if dialed {
            let rx = {
                let mut inner = self.inner.lock().unwrap();
                self.rt(&mut inner, &name).st.rx
            };
            w.json(&self.hello(rx));
        }
        // the first frame must be hello
        let registered = match crate::server::ws_recv_max(&mut reader, MAX_FRAME) {
            Ok((1, b)) => match serde_json::from_slice::<Value>(&b) {
                Ok(v) if v["t"] == "hello" => self.register(&link, &v),
                Ok(v) if v["t"] == "bye" => {
                    *link.their_bye.lock().unwrap() = v["reason"].as_str().map(str::to_string);
                    Err(())
                }
                _ => Err(()),
            },
            _ => Err(()),
        };
        if registered.is_ok() {
            self.after_up(&name);
            loop {
                match crate::server::ws_recv_max(&mut reader, MAX_FRAME) {
                    Ok((op, payload)) => {
                        *link.last_rx.lock().unwrap() = Instant::now();
                        match op {
                            1 => {
                                if let Ok(v) = serde_json::from_slice::<Value>(&payload) {
                                    self.on_text(&link, &v);
                                }
                            }
                            2 => self.on_chunk(&link, &payload),
                            9 => w.push(10, payload),
                            10 => {}
                            8 => break,
                            _ => {}
                        }
                    }
                    Err(_) => break,
                }
            }
        }
        w.close_with(None);
        tls.shutdown();
        if registered.is_ok() {
            self.on_closed(&link);
        } else {
            self.on_rejected(&link);
        }
    }

    fn hello(&self, rx: u64) -> Value {
        json!({"t": "hello", "name": self.me(), "ver": env!("CARGO_PKG_VERSION"), "proto": PROTO, "instance": self.instance, "rx": rx})
    }

    /// The peer's hello: take the link, or refuse it (banned, a duplicate, a second machine).
    fn register(self: &Arc<Self>, link: &Arc<Link>, hello: &Value) -> Result<(), ()> {
        let name = &link.peer;
        let their_name = hello["name"].as_str().unwrap_or("");
        let instance = hello["instance"].as_str().unwrap_or("").to_string();
        let proto = hello["proto"].as_u64().unwrap_or(0);
        let me = self.me();
        if their_name != name {
            link.silent_close("wrong_name");
            return Err(());
        }
        if proto != PROTO {
            link.silent_close("proto");
            self.tell(
                name,
                &format!(
                    "{name} speaks peer protocol {proto}, this bia {PROTO}. {}",
                    if proto > PROTO {
                        "Update this bia (tell root) to talk with it."
                    } else {
                        "It must update before you can talk."
                    }
                ),
            );
            return Err(());
        }
        *link.instance.lock().unwrap() = instance.clone();
        let mut inner = self.inner.lock().unwrap();
        if self.closing.load(Ordering::Relaxed) {
            drop(inner);
            link.silent_close("shutdown");
            return Err(());
        }
        let rt = self.rt(&mut inner, name);
        if !rt.st.banned_by.is_empty() {
            drop(inner);
            link.silent_close("banned");
            return Err(());
        }
        if let Some(old) = rt.link.clone() {
            let old_instance = old.instance.lock().unwrap().clone();
            if old_instance == instance {
                if old.dialed != link.dialed {
                    // both dialed at once: both ends keep the link the lower name dialed
                    let winner = keep_dialed_by(&me, name);
                    let new_wins = if link.dialed {
                        winner == me
                    } else {
                        winner == name.as_str()
                    };
                    if !new_wins {
                        drop(inner);
                        link.silent_close("duplicate");
                        return Err(());
                    }
                }
                // otherwise the old one is stale (NAT rebinding, a half-open socket)
                old.silent_close("duplicate");
            } else {
                // another instance: a restart whose old socket is not dead yet, or a
                // second machine with the same key — ask the old link
                drop(inner);
                let sent = Instant::now();
                old.w.push(9, b"probe".to_vec());
                let deadline = sent + PROBE;
                let alive = loop {
                    if *old.last_rx.lock().unwrap() > sent {
                        break true;
                    }
                    if Instant::now() >= deadline || old.w.q.lock().unwrap().done {
                        break false;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                };
                if alive {
                    link.silent_close("duplicate_key");
                    self.tell(name, &format!(
                        "A second machine (from {}) tried to connect as {name} while {name} is connected from {}. It was refused. Tell root now: one of the two machines is a copy using {name}'s key.",
                        link.ip, old.ip
                    ));
                    return Err(());
                }
                old.silent_close("duplicate");
                inner = self.inner.lock().unwrap();
            }
        }
        let rt = self.rt(&mut inner, name);
        let rx_them = hello["rx"].as_u64().unwrap_or(0);
        rt.link = Some(link.clone());
        // a link both ends took is wanted again, whoever closed the last one
        rt.st.want = true;
        rt.dialing = false;
        rt.attempts = 0;
        rt.next_dial = None;
        rt.st.banned_us = false;
        rt.st.cannot.clear();
        rt.st.last_up = now_secs();
        if !link.dialed {
            link.w.json(&self.hello(rt.st.rx));
        }
        // resend what they did not get; forget what they did
        rt.st
            .unacked
            .retain(|f| f["seq"].as_u64().unwrap_or(0) > rx_them);
        for f in &rt.st.unacked {
            link.w.json(f);
        }
        let first = !rt.st.greeted;
        rt.st.greeted = true;
        let back = rt.told_down;
        let net = rt.net_down;
        rt.net_down = false;
        let away = now_secs().saturating_sub(rt.st.last_down);
        rt.told_down = false;
        rt.down_since = None;
        self.save_state(name, &rt.st);
        if first {
            self.tell(name, &format!(
                "Connected: the link with {name} is up (from {}). Talk with say(channel=\"peer:{name}\"); {name} is another agent, not a person.",
                link.ip
            ));
        } else if back {
            let text = format!(
                "Back online: the link with {name} is up again after {}.",
                human_secs(away)
            );
            if net {
                self.tell_gated(rt, name, &text);
            } else {
                self.tell(name, &text);
            }
        }
        drop(inner);
        self.changed.notify_all();
        Ok(())
    }

    /// The link is up: fetch pending files, renew this bia's certificate there when due.
    fn after_up(self: &Arc<Self>, name: &str) {
        self.pump(name);
        if let Ok(pki) = self.pki() {
            if pki
                .peer_cert_days_left(name)
                .is_some_and(|d| d < RENEW_WITHIN_DAYS)
            {
                if let Ok(csr) = pki.agent_renew_csr() {
                    self.send_plain(name, &json!({"t": "renew", "csr": csr}));
                }
            }
        }
    }

    fn on_rejected(&self, link: &Arc<Link>) {
        let bye = link.their_bye.lock().unwrap().clone();
        let name = &link.peer;
        let Some(reason) = bye else { return };
        let mut inner = self.inner.lock().unwrap();
        let rt = self.rt(&mut inner, name);
        let text = match reason.as_str() {
            "banned" => {
                rt.st.banned_us = true;
                rt.st.want = false;
                Some(bye_text(name, "banned"))
            }
            "duplicate_key" => {
                rt.st.want = false;
                Some(bye_text(name, "duplicate_key"))
            }
            "busy" => {
                rt.next_dial = Some(Instant::now() + backoff(rt.attempts.max(3), rnd01()));
                None
            }
            _ => None,
        };
        self.save_state(name, &rt.st);
        drop(inner);
        if let Some(t) = text {
            self.tell(name, &t);
        }
        self.changed.notify_all();
    }

    fn on_closed(self: &Arc<Self>, link: &Arc<Link>) {
        let name = link.peer.clone();
        let mut inner = self.inner.lock().unwrap();
        let rt = self.rt(&mut inner, &name);
        if rt.link.as_ref().map(|l| l.id) != Some(link.id) {
            return;
        }
        rt.link = None;
        rt.pulls.clear();
        rt.st.last_down = now_secs();
        let theirs = link.their_bye.lock().unwrap().clone();
        let ours = link.our_bye.lock().unwrap().clone();
        if link.replaced.load(Ordering::Relaxed) || self.closing.load(Ordering::Relaxed) {
            self.save_state(&name, &rt.st);
            return;
        }
        match (theirs.as_deref(), ours) {
            (Some(r), _) if BYE_RESTARTS.contains(&r) => {
                self.tell(&name, &bye_text(&name, r));
                rt.told_down = true;
                self.fail_unacked(rt, &name, &format!("{name} went down"));
                rt.attempts = 0;
                rt.next_dial = Some(Instant::now() + backoff(0, rnd01()));
            }
            (Some(r @ ("disconnect" | "banned" | "revoked" | "duplicate_key")), _) => {
                rt.st.want = false;
                if r == "banned" {
                    rt.st.banned_us = true;
                }
                rt.told_down = true;
                self.tell(&name, &bye_text(&name, r));
                self.fail_unacked(rt, &name, &format!("the link was closed: {r}"));
            }
            (_, Some(_)) => {
                // this bia closed it (disconnect, ban, remove): the tool said so
                rt.told_down = true;
                rt.st.unacked.clear();
            }
            _ => {
                // network loss: redial at once (jittered); reported LOST_AFTER after the
                // last frame (a dead link already waited DEAD_AFTER for it)
                rt.down_since = Some(*link.last_rx.lock().unwrap());
                rt.net_down = true;
                rt.attempts = 0;
                rt.next_dial =
                    Some(Instant::now() + Duration::from_millis(500 + (rnd01() * 2500.0) as u64));
            }
        }
        self.save_state(&name, &rt.st);
        drop(inner);
        self.changed.notify_all();
    }

    // ---- frames from the peer ----

    fn send_plain(&self, name: &str, v: &Value) -> bool {
        let inner = self.inner.lock().unwrap();
        match inner.rt.get(name).and_then(|r| r.link.clone()) {
            Some(l) => {
                l.w.json(v);
                true
            }
            None => false,
        }
    }

    fn on_text(self: &Arc<Self>, link: &Arc<Link>, v: &Value) {
        let name = link.peer.as_str();
        match v["t"].as_str().unwrap_or("") {
            "ack" => {
                let seq = v["seq"].as_u64().unwrap_or(0);
                let mut inner = self.inner.lock().unwrap();
                let rt = self.rt(&mut inner, name);
                let n = rt.st.unacked.len();
                rt.st.unacked.retain(|f| f["seq"].as_u64() != Some(seq));
                if rt.st.unacked.len() != n {
                    self.save_state(name, &rt.st);
                }
            }
            "msg" | "edit" | "react" | "del" => self.on_seq(link, v),
            "read" | "typing" => {}
            "fget" => self.on_fget(link, v),
            "fack" => link.w.acked(
                v["fid"].as_str().unwrap_or(""),
                v["off"].as_u64().unwrap_or(0),
            ),
            "fdone" => {
                let fid = v["fid"].as_str().unwrap_or("");
                link.w.drop_job(fid);
                self.out_files_update(name, |m| {
                    if let Some(f) = m.get_mut(fid) {
                        f.done = true;
                    }
                });
            }
            "frefuse" => {
                let fid = v["fid"].as_str().unwrap_or("").to_string();
                link.w.drop_job(&fid);
                let file = self
                    .out_files(name)
                    .get(&fid)
                    .map(|f| f.name.clone())
                    .unwrap_or_else(|| fid.clone());
                self.tell(
                    name,
                    &format!(
                        "{name} did not take the file {file}: {}.",
                        v["reason"].as_str().unwrap_or("refused")
                    ),
                );
            }
            "fcancel" => {
                let fid = v["fid"].as_str().unwrap_or("").to_string();
                self.settle_rx(name, &fid, None, "the sender cancelled it");
            }
            "renew" => {
                let r = self.pki().and_then(|p| {
                    p.peer_renew(name, v["csr"].as_str().unwrap_or(""))
                        .map_err(|e| format!("{e:#}"))
                });
                match r {
                    Ok(cert) => {
                        self.bus
                            .log(&format!("  peer {name}: renewed its certificate here"));
                        link.w.json(&json!({"t": "renewed", "cert": cert}));
                    }
                    Err(e) => link.w.json(&json!({"t": "renew_err", "error": e})),
                }
            }
            "renewed" => match self.pki().and_then(|p| {
                p.peer_save_cert(name, v["cert"].as_str().unwrap_or(""))
                    .map_err(|e| format!("{e:#}"))
            }) {
                Ok(()) => self.bus.log(&format!(
                    "  peer {name}: this bia's certificate there was renewed"
                )),
                Err(e) => self
                    .bus
                    .log(&format!("  peer {name}: renewal answer not usable: {e}")),
            },
            "renew_err" => self.bus.log(&format!(
                "  peer {name}: renewal refused: {}",
                v["error"].as_str().unwrap_or("")
            )),
            "bye" => *link.their_bye.lock().unwrap() = v["reason"].as_str().map(str::to_string),
            _ => {}
        }
    }

    /// A sequenced frame: acknowledge it once it is on disk, deliver it in order.
    fn on_seq(self: &Arc<Self>, link: &Arc<Link>, v: &Value) {
        let name = link.peer.as_str();
        let seq = v["seq"].as_u64().unwrap_or(0);
        // a peer sending more than RATE_PER_MIN a minute is slowed down, not cut off
        let slow = {
            let mut inner = self.inner.lock().unwrap();
            let rt = self.rt(&mut inner, name);
            let now = Instant::now();
            while rt
                .rate
                .front()
                .is_some_and(|t| now.duration_since(*t) > Duration::from_secs(60))
            {
                rt.rate.pop_front();
            }
            rt.rate.push_back(now);
            rt.rate.len() > RATE_PER_MIN
        };
        if slow {
            std::thread::sleep(Duration::from_secs(1));
        }
        {
            let mut inner = self.inner.lock().unwrap();
            let rt = self.rt(&mut inner, name);
            if is_new_seq(rt.st.rx, seq) {
                rt.st.rx = seq;
                let mut files = vec![];
                if v["t"] == "msg" {
                    for f in v["files"].as_array().into_iter().flatten().take(32) {
                        let fid = f["fid"].as_str().unwrap_or("").to_ascii_lowercase();
                        let fname = safe_file_name(f["name"].as_str().unwrap_or("file"));
                        let size = f["size"].as_u64().unwrap_or(0);
                        let mut rf = RxFile {
                            fid: fid.clone(),
                            name: fname,
                            size,
                            ..Default::default()
                        };
                        if unhex32(&fid).is_none() {
                            rf.err = "bad file id".into();
                        }
                        files.push(rf);
                    }
                }
                rt.st.inbox.push(Pending {
                    frame: v.clone(),
                    files,
                    at: now_secs(),
                });
                self.save_state(name, &rt.st);
            }
        }
        link.w.json(&json!({"t": "ack", "seq": seq}));
        self.pump(name);
    }

    /// Deliver what is complete at the head of the peer's inbox; start pulling files.
    fn pump(self: &Arc<Self>, name: &str) {
        let mut deliver = vec![];
        let mut refuse = vec![];
        {
            let mut inner = self.inner.lock().unwrap();
            let total: usize = inner.rt.values().map(|r| r.pulls.len()).sum();
            let rt = self.rt(&mut inner, name);
            let mut pulling = total;
            let link = rt.link.clone();
            loop {
                if let Some(link) = &link {
                    for p in rt.st.inbox.iter_mut() {
                        for f in p.files.iter_mut().filter(|f| !f.settled()) {
                            if rt.pulls.contains(&f.fid) {
                                continue;
                            }
                            if f.size > MAX_FILE {
                                f.err = format!(
                                    "too big ({} MB; the limit is {} MB)",
                                    f.size >> 20,
                                    MAX_FILE >> 20
                                );
                                refuse.push((f.fid.clone(), f.err.clone()));
                                continue;
                            }
                            let have = fs::metadata(self.part_path(name, &f.fid))
                                .map(|m| m.len())
                                .unwrap_or(0);
                            let free = free_bytes(&self.files);
                            if free < f.size.saturating_sub(have) + DISK_MARGIN {
                                f.err =
                                    format!("not enough disk space here ({} MB free)", free >> 20);
                                refuse.push((f.fid.clone(), f.err.clone()));
                                continue;
                            }
                            if rt.pulls.len() >= FILES_PER_PEER || pulling >= FILES_TOTAL {
                                continue;
                            }
                            if f.size == 0 {
                                // nothing to pull: settle below
                            }
                            rt.pulls.insert(f.fid.clone());
                            pulling += 1;
                            link.w
                                .json(&json!({"t": "fget", "fid": f.fid, "off": have}));
                        }
                    }
                }
                let ready = rt
                    .st
                    .inbox
                    .first()
                    .is_some_and(|p| p.files.iter().all(|f| f.settled()));
                if !ready {
                    break;
                }
                deliver.push(rt.st.inbox.remove(0));
            }
            if !deliver.is_empty() || !refuse.is_empty() {
                self.save_state(name, &rt.st);
            }
            if let Some(link) = &link {
                for (fid, why) in &refuse {
                    link.w
                        .json(&json!({"t": "frefuse", "fid": fid, "reason": why}));
                }
            }
        }
        for p in deliver {
            self.deliver(name, p);
        }
    }

    fn deliver(&self, name: &str, p: Pending) {
        let v = &p.frame;
        let id = v["id"].as_str().unwrap_or("").to_string();
        let ts = v["ts"].as_u64().unwrap_or_else(now_secs);
        let mut log = ChatLine {
            id: id.clone(),
            from: name.to_string(),
            ts,
            text: String::new(),
            files: vec![],
            reply_to: None,
            kind: v["t"].as_str().unwrap_or("msg").to_string(),
            ids: vec![],
        };
        let text = match v["t"].as_str().unwrap_or("") {
            "msg" => {
                let mut parts = vec![];
                if let Some(o) = Origin::from_json(&v["origin"]) {
                    parts.push(o.header());
                    if let Some(again) = self.origin_seen(&o, name) {
                        parts.push(again);
                    }
                }
                if let Some(r) = v["reply_to"].as_str().filter(|r| !r.is_empty()) {
                    log.reply_to = Some(r.to_string());
                    parts.push(match self.chat_find(name, r) {
                        Some(o) => format!(
                            "[reply to {r} from {}: \"{}\"]",
                            if o.from == "me" {
                                "you".to_string()
                            } else {
                                o.from.clone()
                            },
                            snippet(&o.text)
                        ),
                        None => format!("[reply to {r}]"),
                    });
                }
                let body = v["text"].as_str().unwrap_or("").to_string();
                if !body.trim().is_empty() {
                    parts.push(body.clone());
                }
                log.text = body;
                let done: Vec<&RxFile> = p.files.iter().filter(|f| !f.path.is_empty()).collect();
                if !done.is_empty() {
                    let mut s = String::from("files:");
                    for f in &done {
                        s.push_str(&format!("\n- {}", f.path));
                        log.files.push(f.path.clone());
                    }
                    parts.push(s);
                }
                for f in p.files.iter().filter(|f| f.path.is_empty()) {
                    parts.push(format!("[file {} not received: {}]", f.name, f.err));
                }
                parts.join("\n")
            }
            "edit" => {
                let t = v["text"].as_str().unwrap_or("").to_string();
                log.text = t.clone();
                format!("[edited {id}] {t}")
            }
            "react" => {
                let e = v["emoji"].as_str().unwrap_or("").to_string();
                log.text = e.clone();
                if e.is_empty() {
                    format!("[reaction] cleared on {id}")
                } else {
                    format!("[reaction] {e} on {id}")
                }
            }
            "del" => {
                log.ids = v["ids"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .take(100)
                    .collect();
                format!("[deleted] {}", log.ids.join(", "))
            }
            _ => return,
        };
        self.chat_append(name, &log);
        self.bus.push_inbound(Inbound {
            identity: name.to_string(),
            channel: channel(name),
            id: if log.kind == "msg" { id } else { mint_msg_id() },
            text,
        });
    }

    // ---- files ----

    fn out_files(&self, name: &str) -> HashMap<String, OutFile> {
        fs::read(self.dir(name).join("sent.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn out_files_update(&self, name: &str, f: impl FnOnce(&mut HashMap<String, OutFile>)) {
        let _g = self.origins.lock().unwrap();
        let mut m = self.out_files(name);
        f(&mut m);
        let d = self.dir(name);
        let _ = fs::create_dir_all(&d);
        let _ = fs::write(
            d.join("sent.json"),
            serde_json::to_vec(&m).unwrap_or_default(),
        );
        self.own(&d.join("sent.json"));
    }

    fn on_fget(&self, link: &Arc<Link>, v: &Value) {
        let name = link.peer.as_str();
        let fid = v["fid"].as_str().unwrap_or("").to_string();
        let off = v["off"].as_u64().unwrap_or(0);
        let Some(raw) = unhex32(&fid) else { return };
        let files = self.out_files(name);
        let Some(f) = files.get(&fid) else {
            link.w.json(
                &json!({"t": "frefuse", "fid": fid, "reason": "this bia no longer has that file"}),
            );
            return;
        };
        let ok = fs::metadata(&f.path).is_ok_and(|m| m.len() == f.size && mtime(&m) == f.mtime);
        if !ok {
            link.w.json(&json!({"t": "frefuse", "fid": fid, "reason": "the file changed or is gone at the sender"}));
            return;
        }
        link.w.add_job(Job {
            fid,
            raw,
            path: PathBuf::from(&f.path),
            off: off.min(f.size),
            end: f.size,
            acked: off.min(f.size),
        });
    }

    fn on_chunk(self: &Arc<Self>, link: &Arc<Link>, b: &[u8]) {
        let name = link.peer.as_str();
        let Some((fid, off, data)) = parse_chunk(b) else {
            return;
        };
        let (size, pulling) = {
            let mut inner = self.inner.lock().unwrap();
            let rt = self.rt(&mut inner, name);
            let size = rt
                .st
                .inbox
                .iter()
                .flat_map(|p| p.files.iter())
                .find(|f| f.fid == fid && !f.settled())
                .map(|f| f.size);
            (size, rt.pulls.contains(&fid))
        };
        let (Some(size), true) = (size, pulling) else {
            return;
        };
        let part = self.part_path(name, &fid);
        let _ = fs::create_dir_all(part.parent().unwrap());
        let have = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        if off != have {
            // out of step (after a reconnect): ask again from what is here
            if off > have {
                link.w.json(&json!({"t": "fget", "fid": fid, "off": have}));
            }
            return;
        }
        let take = (data.len() as u64).min(size.saturating_sub(have)) as usize;
        let ok = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&part)
            .and_then(|mut f| f.write_all(&data[..take]))
            .is_ok();
        if !ok {
            self.settle_rx(name, &fid, None, "could not write it here");
            link.w.json(
                &json!({"t": "frefuse", "fid": fid, "reason": "the receiver could not write it"}),
            );
            return;
        }
        if have == 0 {
            self.own(&part);
        }
        let have = have + take as u64;
        link.w.json(&json!({"t": "fack", "fid": fid, "off": have}));
        if have >= size {
            let sum = sha256_file(&part).unwrap_or_default();
            if sum == fid {
                link.w.json(&json!({"t": "fdone", "fid": fid}));
                self.settle_rx(name, &fid, Some(&part), "");
            } else {
                let _ = fs::remove_file(&part);
                link.w.json(&json!({"t": "frefuse", "fid": fid, "reason": "it arrived damaged (checksum mismatch)"}));
                self.settle_rx(name, &fid, None, "it arrived damaged (checksum mismatch)");
            }
        }
    }

    /// A pulled file is complete (`part`) or failed (`err`): move it into the inbox and
    /// let the inbox go on.
    fn settle_rx(self: &Arc<Self>, name: &str, fid: &str, part: Option<&Path>, err: &str) {
        {
            let mut inner = self.inner.lock().unwrap();
            let inbox = self.inbox_dir(name);
            let rt = self.rt(&mut inner, name);
            rt.pulls.remove(fid);
            let mut final_path = None;
            for f in rt
                .st
                .inbox
                .iter_mut()
                .flat_map(|p| p.files.iter_mut())
                .filter(|f| f.fid == fid && !f.settled())
            {
                match (part, &final_path) {
                    (Some(p), None) => {
                        let dest = unique_path(&inbox, &f.name);
                        if fs::rename(p, &dest).is_ok() {
                            self.own(&dest);
                            f.path = dest.display().to_string();
                            final_path = Some(dest);
                        } else {
                            f.err = "could not move it into the inbox".into();
                        }
                    }
                    // the same file in another message: a copy of the first
                    (Some(_), Some(first)) => {
                        let dest = unique_path(&inbox, &f.name);
                        match fs::copy(first, &dest) {
                            Ok(_) => {
                                self.own(&dest);
                                f.path = dest.display().to_string()
                            }
                            Err(_) => f.err = "could not copy it".into(),
                        }
                    }
                    (None, _) => f.err = err.to_string(),
                }
            }
            self.save_state(name, &rt.st);
        }
        self.pump(name);
    }

    // ---- the chat log and origins ----

    fn chat_append(&self, name: &str, l: &ChatLine) {
        let d = self.dir(name);
        let _ = fs::create_dir_all(&d);
        if let Ok(mut f) = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(d.join("chat.jsonl"))
        {
            let _ = writeln!(f, "{}", serde_json::to_string(l).unwrap_or_default());
        }
        self.own(&d.join("chat.jsonl"));
    }

    fn chat_lines(&self, name: &str) -> Vec<ChatLine> {
        fs::read_to_string(self.dir(name).join("chat.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }

    /// Messages with edits and deletions applied, oldest first.
    fn chat_view(&self, name: &str) -> Vec<ChatLine> {
        let mut out: Vec<ChatLine> = vec![];
        for l in self.chat_lines(name) {
            match l.kind.as_str() {
                "msg" => out.push(l),
                "edit" => {
                    if let Some(m) = out.iter_mut().find(|m| m.id == l.id) {
                        m.text = l.text;
                    }
                }
                "del" => out.retain(|m| !l.ids.contains(&m.id)),
                _ => {}
            }
        }
        out
    }

    fn chat_find(&self, name: &str, id: &str) -> Option<ChatLine> {
        self.chat_view(name).into_iter().rev().find(|m| m.id == id)
    }

    fn origins_path(&self) -> PathBuf {
        self.files.join("state").join("peers").join("origins.json")
    }

    fn origins_load(&self) -> HashMap<String, Value> {
        fs::read(self.origins_path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn origins_save(&self, m: &HashMap<String, Value>) {
        let p = self.origins_path();
        let _ = fs::create_dir_all(p.parent().unwrap());
        let _ = fs::write(&p, serde_json::to_vec(m).unwrap_or_default());
        self.own(&p);
    }

    /// A message of request `o` came from `from`: "[asked again: …]" when it came before.
    fn origin_seen(&self, o: &Origin, from: &str) -> Option<String> {
        let _g = self.origins.lock().unwrap();
        let mut m = self.origins_load();
        let now = now_secs();
        let line = m.get(&o.id).map(|e| {
            let first = e["from"].as_str().unwrap_or("?");
            if first == "me" {
                return format!(
                    "[asked again: this request started with you; you passed it on at {}]",
                    crate::bus::rfc3339_unix(e["at"].as_u64().unwrap_or(0))
                );
            }
            let at = crate::bus::rfc3339_unix(e["at"].as_u64().unwrap_or(0));
            match e["answered"].as_u64().filter(|a| *a > 0) {
                Some(a) => format!("[asked again: you got this request from {first} at {at} and replied at {}]", crate::bus::rfc3339_unix(a)),
                None => format!("[asked again: you got this request from {first} at {at} and have not replied yet]"),
            }
        });
        if line.is_none() {
            m.insert(
                o.id.clone(),
                json!({"from": from, "at": now, "answered": 0}),
            );
            m.retain(|_, e| now.saturating_sub(e["at"].as_u64().unwrap_or(0)) < 7 * 86_400);
            self.origins_save(&m);
        }
        line
    }

    /// This bia sent a message of request `id`: replied (or, for its own request, started).
    fn origin_answered(&self, id: &str) {
        let _g = self.origins.lock().unwrap();
        let mut m = self.origins_load();
        let now = now_secs();
        let e = m
            .entry(id.to_string())
            .or_insert_with(|| json!({"from": "me", "at": now, "answered": 0}));
        if e["answered"].as_u64().unwrap_or(0) == 0 {
            e["answered"] = json!(now);
            self.origins_save(&m);
        }
    }

    // ---- the timer ----

    fn tick(self: &Arc<Self>) {
        let n = self.ticks.fetch_add(1, Ordering::Relaxed);
        // new peers (trust just set up, at either end) and removed ones, every 5 s
        if n % 5 == 0 {
            if let Ok(pki) = self.pki() {
                let active: Vec<String> = pki
                    .peers()
                    .into_iter()
                    .filter(|p| p.status == "active")
                    .map(|p| p.name)
                    .collect();
                let mut inner = self.inner.lock().unwrap();
                for name in &active {
                    let fresh = !inner.rt.contains_key(name) && self.load_state(name).is_none();
                    let rt = self.rt(&mut inner, name);
                    if fresh {
                        rt.next_dial = Some(Instant::now());
                    }
                }
                let gone: Vec<Arc<Link>> = inner
                    .rt
                    .iter_mut()
                    .filter(|(k, _)| !active.contains(k))
                    .filter_map(|(_, r)| r.link.take())
                    .collect();
                drop(inner);
                for l in gone {
                    l.silent_close("revoked");
                }
            }
        }
        let now = Instant::now();
        let mut dial = vec![];
        let mut lost = vec![];
        let mut settled = vec![];
        {
            let mut inner = self.inner.lock().unwrap();
            let mut hs = inner.handshakes;
            let names: Vec<String> = inner.rt.keys().cloned().collect();
            for name in names {
                let rt = self.rt(&mut inner, &name);
                if let Some(l) = &rt.link {
                    if rt.flap.settled(l.up, now) {
                        settled.push(name.clone());
                    }
                    continue;
                }
                if rt
                    .down_since
                    .is_some_and(|t| now.duration_since(t) >= LOST_AFTER)
                    && !rt.told_down
                {
                    rt.down_since = None;
                    rt.told_down = true;
                    lost.push(name.clone());
                }
                let can = rt.st.want && rt.st.banned_by.is_empty() && !rt.dialing;
                if can && rt.next_dial.map_or(true, |t| now >= t) && hs < MAX_HANDSHAKES {
                    // wanted but never scheduled (start-up): dial now
                    rt.dialing = true;
                    rt.next_dial = None;
                    hs += 1;
                    dial.push(name.clone());
                }
            }
            inner.handshakes = hs;
        }
        for name in lost {
            let mut inner = self.inner.lock().unwrap();
            let rt = self.rt(&mut inner, &name);
            let reached = rt.st.reached;
            let text = format!(
                "Lost: the link with {name} dropped without a goodbye and is not back after 30 s. This bia keeps trying{}; a notice comes when it is back. Messages need a live link until then.",
                if reached { "" } else { " (it has never reached it directly, so it may be up to them)" }
            );
            self.tell_gated(rt, &name, &text);
            self.fail_unacked(rt, &name, "the link dropped");
            self.save_state(&name, &rt.st);
        }
        for name in settled {
            self.tell(
                &name,
                &format!("The link with {name} has held for 10 min: stable again, online."),
            );
        }
        for name in dial {
            self.start_dial(&name);
        }
        // links whose certificate here was revoked or expired go
        if n % 30 == 0 {
            if let Ok(pki) = self.pki() {
                let links: Vec<Arc<Link>> = self
                    .inner
                    .lock()
                    .unwrap()
                    .rt
                    .values()
                    .filter_map(|r| r.link.clone())
                    .collect();
                for l in links {
                    let ok = pki.peer(&l.peer).is_some_and(|p| {
                        p.status == "active"
                            && pki.issued_meta(&p.issued_serial).is_some_and(|m| {
                                !m.revoked && (m.not_after == 0 || m.not_after > now_secs())
                            })
                    });
                    if !ok {
                        l.close("revoked");
                    }
                }
            }
        }
        if n % 3600 == 60 {
            self.cleanup();
        }
    }

    /// Unfinished transfers, old sent-file records and waiting messages older than 3 days.
    fn cleanup(self: &Arc<Self>) {
        let now = now_secs();
        let names: Vec<String> = self.inner.lock().unwrap().rt.keys().cloned().collect();
        for name in names {
            if let Ok(rd) = fs::read_dir(self.inbox_dir(&name).join(".part")) {
                for e in rd.flatten() {
                    if e.metadata()
                        .is_ok_and(|m| now.saturating_sub(mtime(&m)) > KEEP_PARTIAL_SECS)
                    {
                        let _ = fs::remove_file(e.path());
                    }
                }
            }
            self.out_files_update(&name, |m| {
                m.retain(|_, f| now.saturating_sub(f.at) < KEEP_PARTIAL_SECS)
            });
            let stale = {
                let mut inner = self.inner.lock().unwrap();
                let rt = self.rt(&mut inner, &name);
                let mut any = false;
                for p in rt
                    .st
                    .inbox
                    .iter_mut()
                    .filter(|p| now.saturating_sub(p.at) > KEEP_PARTIAL_SECS)
                {
                    for f in p.files.iter_mut().filter(|f| !f.settled()) {
                        f.err = "the link was gone for days".into();
                        any = true;
                    }
                }
                any
            };
            if stale {
                self.pump(&name);
            }
        }
    }

    fn shutdown(&self, reason: &str) {
        if self.closing.swap(true, Ordering::SeqCst) {
            return;
        }
        let links: Vec<Arc<Link>> = self
            .inner
            .lock()
            .unwrap()
            .rt
            .values()
            .filter_map(|r| r.link.clone())
            .collect();
        for l in &links {
            *l.our_bye.lock().unwrap() = Some(reason.to_string());
            l.w.close_with(Some(&json!({"t": "bye", "reason": reason})));
        }
        let until = Instant::now() + BYE_FLUSH;
        for l in &links {
            l.w.wait_done(until);
            l.tls.shutdown();
        }
        if !links.is_empty() {
            self.bus.log(&format!(
                "  peers: said bye ({reason}) to {}",
                links
                    .iter()
                    .map(|l| l.peer.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }

    // ---- what the tools do ----

    fn require_peer(&self, name: &str) -> Result<bia_pki::Peer, String> {
        let pki = self.pki()?;
        match pki.peer(name) {
            Some(p) if p.status == "active" => Ok(p),
            Some(p) => Err(format!(
                "{name} is not an active peer ({}); see web_peer_list",
                p.status
            )),
            None => Err(format!("no peer {name}; peers: {}", {
                let n: Vec<String> = pki.peers().into_iter().map(|p| p.name).collect();
                if n.is_empty() {
                    "(none)".to_string()
                } else {
                    n.join(", ")
                }
            })),
        }
    }

    /// The live link to `name`, or why there is none.
    fn live(&self, name: &str) -> Result<Arc<Link>, String> {
        self.require_peer(name)?;
        let mut inner = self.inner.lock().unwrap();
        let rt = self.rt(&mut inner, name);
        if let Some(l) = &rt.link {
            return Ok(l.clone());
        }
        let since = if rt.st.last_down > 0 {
            format!(" since {}", crate::bus::rfc3339_unix(rt.st.last_down))
        } else {
            String::new()
        };
        Err(if !rt.st.banned_by.is_empty() {
            format!(
                "{name} is banned here (by {}); web_peer_unban first",
                rt.st.banned_by
            )
        } else if rt.st.banned_us {
            format!("{name} banned this bia; nothing can be sent there until it unbans this bia")
        } else if rt.st.cannot == "old" {
            format!("{name} runs an older bia without peer links; it must update first")
        } else if rt.st.cannot == "notpeer" {
            format!("{name} does not accept this bia's certificate any more; tell root")
        } else if !rt.st.reached && rt.st.last_up == 0 {
            format!("{name} is offline. A live connection is required. This bia has never reached {name} directly ({}): if it is behind NAT only it can open the link; wait for its connect notice, or try web_peer_connect(peer=\"{name}\")", if rt.last_err.is_empty() { "not tried yet".into() } else { rt.last_err.clone() })
        } else {
            format!("{name} is offline{since}. A live connection is required: web_peer_connect(peer=\"{name}\") first")
        })
    }

    fn send_seq(&self, name: &str, mut frame: Value) -> Result<(), String> {
        let link = self.live(name)?;
        let mut inner = self.inner.lock().unwrap();
        let rt = self.rt(&mut inner, name);
        if rt.link.as_ref().map(|l| l.id) != Some(link.id) {
            return Err(format!(
                "{name} just went offline; try again when it is back"
            ));
        }
        rt.st.next_seq += 1;
        frame["seq"] = json!(rt.st.next_seq);
        rt.st.unacked.push(frame.clone());
        self.save_state(name, &rt.st);
        link.w.json(&frame);
        Ok(())
    }

    pub fn send(&self, name: &str, args: &Value) -> Result<String, String> {
        if args.get("voice").and_then(|v| v.as_bool()).unwrap_or(false) {
            return Err("voice messages do not go over peer links; send text".into());
        }
        for k in ["buttons", "poll"] {
            if args.get(k).is_some_and(|v| !v.is_null()) {
                return Err(format!("{k} do not go over peer links; send text"));
            }
        }
        self.live(name)?;
        let text = arg_str_opt(args, "text").unwrap_or_default().to_string();
        let paths = arg_str_list(args, "files");
        let mut offered = vec![];
        let mut total = 0u64;
        let id = format!("p{}", &mint_msg_id()[1..]);
        for p in &paths {
            let abs = crate::chat::att_abs(&self.files, p)
                .ok_or_else(|| format!("{p}: not a usable path"))?;
            let m = fs::metadata(&abs).map_err(|e| format!("{}: {e}", abs.display()))?;
            if !m.is_file() {
                return Err(format!("{}: not a file", abs.display()));
            }
            if m.len() > MAX_FILE {
                return Err(format!(
                    "{}: too big for a peer link ({} MB; the limit is {} MB)",
                    abs.display(),
                    m.len() >> 20,
                    MAX_FILE >> 20
                ));
            }
            let path_s = abs.display().to_string();
            let cached = self
                .out_files(name)
                .into_iter()
                .find(|(_, f)| f.path == path_s && f.size == m.len() && f.mtime == mtime(&m))
                .map(|(k, _)| k);
            let fid = match cached {
                Some(f) => f,
                None => sha256_file(&abs).map_err(|e| format!("{}: {e}", abs.display()))?,
            };
            let fname = safe_file_name(
                &abs.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "file".into()),
            );
            self.out_files_update(name, |mm| {
                mm.insert(
                    fid.clone(),
                    OutFile {
                        path: path_s.clone(),
                        name: fname.clone(),
                        size: m.len(),
                        mtime: mtime(&m),
                        at: now_secs(),
                        done: false,
                        msg: id.clone(),
                    },
                );
            });
            total += m.len();
            offered.push(json!({"fid": fid, "name": fname, "size": m.len()}));
        }
        if text.trim().is_empty() && offered.is_empty() {
            return Err("nothing to send: text or files".into());
        }
        let origin = origin_out(args.get("origin").filter(|o| !o.is_null()), &self.me(), &id);
        let reply_to = arg_str_opt(args, "reply_to")
            .map(str::trim)
            .filter(|r| !r.is_empty())
            .map(str::to_string);
        let mut frame = json!({"t": "msg", "id": id, "ts": now_secs(), "text": text, "files": offered, "origin": origin.to_json()});
        if let Some(r) = &reply_to {
            frame["reply_to"] = json!(r);
        }
        self.send_seq(name, frame)?;
        self.origin_answered(&origin.id);
        self.chat_append(
            name,
            &ChatLine {
                id: id.clone(),
                from: "me".into(),
                ts: now_secs(),
                text,
                files: paths.clone(),
                reply_to,
                kind: "msg".into(),
                ids: vec![],
            },
        );
        let mut out = format!("sent to {name}@peer:{name} (message id {id})");
        if !paths.is_empty() {
            out.push_str(&format!(
                "; {name} pulls {} file(s), {} now and gets the message when they have arrived (web_peer_list shows progress; web_peer_cancel stops it)",
                paths.len(),
                human_bytes(total)
            ));
        }
        Ok(out)
    }

    pub fn edit(&self, name: &str, id: &str, text: &str) -> Result<String, String> {
        match self.chat_find(name, id) {
            Some(m) if m.from == "me" => {}
            Some(_) => {
                return Err(format!(
                    "{id} is {name}'s message; only this bia's own messages can be edited"
                ))
            }
            None => return Err(format!("no message {id} on peer:{name}")),
        }
        self.send_seq(name, json!({"t": "edit", "id": id, "text": text}))?;
        self.chat_append(
            name,
            &ChatLine {
                id: id.into(),
                from: "me".into(),
                ts: now_secs(),
                text: text.into(),
                files: vec![],
                reply_to: None,
                kind: "edit".into(),
                ids: vec![],
            },
        );
        Ok(format!("edited {id} on peer:{name}"))
    }

    pub fn react(&self, name: &str, id: &str, emoji: &str) -> Result<String, String> {
        self.send_seq(name, json!({"t": "react", "id": id, "emoji": emoji}))?;
        Ok(if emoji.is_empty() {
            format!("removed the reaction on {id}")
        } else {
            format!("reacted {emoji} on {id}")
        })
    }

    pub fn delete(&self, name: &str, ids: &[String]) -> Result<String, String> {
        let view = self.chat_view(name);
        let own: Vec<String> = ids
            .iter()
            .filter(|i| view.iter().any(|m| &m.id == *i && m.from == "me"))
            .cloned()
            .collect();
        if own.is_empty() {
            return Err("only this bia's own messages can be deleted on a peer chat".into());
        }
        self.send_seq(name, json!({"t": "del", "ids": own}))?;
        self.chat_append(
            name,
            &ChatLine {
                id: String::new(),
                from: "me".into(),
                ts: now_secs(),
                text: String::new(),
                files: vec![],
                reply_to: None,
                kind: "del".into(),
                ids: own.clone(),
            },
        );
        Ok(format!("deleted {} message(s) on both sides", own.len()))
    }

    pub fn history(&self, name: &str, limit: usize) -> Result<String, String> {
        self.require_peer(name)?;
        let v = self.chat_view(name);
        if v.is_empty() {
            return Ok(format!("(no messages with {name} yet)"));
        }
        let start = v.len().saturating_sub(limit.max(1));
        Ok(v[start..]
            .iter()
            .map(|m| {
                let who = if m.from == "me" {
                    "you".to_string()
                } else {
                    m.from.clone()
                };
                let mut s = format!(
                    "{} {} {who}: {}",
                    m.id,
                    crate::bus::rfc3339_unix(m.ts),
                    m.text
                );
                if let Some(r) = &m.reply_to {
                    s.push_str(&format!(" [reply to {r}]"));
                }
                for f in &m.files {
                    s.push_str(&format!(" [file {f}]"));
                }
                s
            })
            .collect::<Vec<_>>()
            .join("\n"))
    }

    pub fn read(&self, name: &str, id: &str) {
        self.send_plain(name, &json!({"t": "read", "id": id}));
    }

    pub fn typing(&self, name: &str, on: bool) {
        self.send_plain(name, &json!({"t": "typing", "on": on}));
    }

    /// Open (or keep) the link; waits up to 15 s for it.
    pub fn connect(self: &Arc<Self>, name: &str) -> Result<String, String> {
        self.require_peer(name)?;
        let start = Instant::now();
        {
            let mut inner = self.inner.lock().unwrap();
            let rt = self.rt(&mut inner, name);
            if !rt.st.banned_by.is_empty() {
                return Err(format!(
                    "{name} is banned here (by {}); web_peer_unban first",
                    rt.st.banned_by
                ));
            }
            if rt.link.is_some() {
                return Ok(format!("{name} is already connected"));
            }
            rt.st.want = true;
            rt.st.banned_us = false;
            rt.attempts = 0;
            rt.next_dial = Some(Instant::now());
            self.save_state(name, &rt.st);
        }
        let mut inner = self.inner.lock().unwrap();
        loop {
            let rt = self.rt(&mut inner, name);
            if rt.link.is_some() {
                return Ok(format!("connected to {name}"));
            }
            if rt.st.banned_us {
                return Err(bye_text(name, "banned"));
            }
            if !rt.st.want {
                return Err(format!("{name} refused the link: {}", rt.last_err));
            }
            if rt.err_at.is_some_and(|t| t > start) && !rt.dialing {
                let e = rt.last_err.clone();
                let nat = if rt.st.reached {
                    ""
                } else {
                    " This bia has never reached it directly: if it is behind NAT, only it can open the link."
                };
                return Ok(format!("not connected yet: {e}. This bia keeps trying (backing off up to 10 min) and tells you when the link is up; web_peer_disconnect stops trying.{nat}"));
            }
            let left = Duration::from_secs(15).saturating_sub(start.elapsed());
            if left.is_zero() {
                return Ok(format!("{name} has not answered yet; this bia keeps trying and tells you when the link is up"));
            }
            inner = self
                .changed
                .wait_timeout(inner, left.min(Duration::from_millis(500)))
                .unwrap()
                .0;
        }
    }

    pub fn disconnect(&self, name: &str) -> Result<String, String> {
        self.require_peer(name)?;
        let link = {
            let mut inner = self.inner.lock().unwrap();
            let rt = self.rt(&mut inner, name);
            rt.st.want = false;
            rt.next_dial = None;
            self.save_state(name, &rt.st);
            rt.link.clone()
        };
        Ok(match link {
            Some(l) => {
                l.close("disconnect");
                format!("disconnected from {name}. Nothing reconnects by itself; {name} is told and may connect again (web_peer_ban keeps it out)")
            }
            None => format!("{name} was not connected; this bia stopped trying to reach it"),
        })
    }

    pub fn ban(&self, name: &str, by: &str) -> Result<String, String> {
        self.require_peer(name)?;
        let link = {
            let mut inner = self.inner.lock().unwrap();
            let rt = self.rt(&mut inner, name);
            if rt.st.banned_by == "root" && by != "root" {
                return Ok(format!("{name} is already banned by root"));
            }
            rt.st.banned_by = by.to_string();
            rt.st.want = false;
            rt.st.inbox.clear();
            rt.st.unacked.clear();
            self.save_state(name, &rt.st);
            rt.link.clone()
        };
        if let Some(l) = link {
            l.close("banned");
        }
        Ok(format!("banned {name}: its links are refused and nothing goes to or comes from it. {name} is told. web_peer_unban lifts it"))
    }

    pub fn unban(self: &Arc<Self>, name: &str, by: &str) -> Result<String, String> {
        self.require_peer(name)?;
        {
            let mut inner = self.inner.lock().unwrap();
            let rt = self.rt(&mut inner, name);
            if rt.st.banned_by.is_empty() {
                return Ok(format!("{name} is not banned"));
            }
            if rt.st.banned_by == "root" && by != "root" {
                return Err(format!("root banned {name}; only root can unban it"));
            }
            rt.st.banned_by.clear();
            self.save_state(name, &rt.st);
        }
        let r = self.connect(name)?;
        Ok(format!("unbanned {name}; {r}"))
    }

    pub fn cancel(&self, name: &str, message_id: &str) -> Result<String, String> {
        let fids: Vec<String> = self
            .out_files(name)
            .into_iter()
            .filter(|(_, f)| f.msg == message_id && !f.done)
            .map(|(k, _)| k)
            .collect();
        if fids.is_empty() {
            return Err(format!(
                "no file of message {message_id} is still going to {name}"
            ));
        }
        let link = self
            .inner
            .lock()
            .unwrap()
            .rt
            .get(name)
            .and_then(|r| r.link.clone());
        for fid in &fids {
            if let Some(l) = &link {
                l.w.drop_job(fid);
                l.w.json(&json!({"t": "fcancel", "fid": fid}));
            }
        }
        self.out_files_update(name, |m| m.retain(|k, _| !fids.contains(k)));
        Ok(format!(
            "stopped {} file(s) of {message_id}; {name} gets the message without them",
            fids.len()
        ))
    }

    /// Forget the peer here (the trust part is done by the caller).
    pub fn forget(&self, name: &str) {
        let link = self
            .inner
            .lock()
            .unwrap()
            .rt
            .remove(name)
            .and_then(|r| r.link);
        if let Some(l) = link {
            l.silent_close("revoked");
        }
        let _ = fs::remove_dir_all(self.dir(name));
    }

    /// One line per peer (+ transfers), for web_peer_list and root's views.
    pub fn describe(&self, name: &str) -> Value {
        let mut inner = self.inner.lock().unwrap();
        let rt = self.rt(&mut inner, name);
        let st = &rt.st;
        let link = rt.link.clone();
        let state = if !st.banned_by.is_empty() {
            format!("banned by {}", st.banned_by)
        } else if st.banned_us {
            "banned this bia".into()
        } else if let Some(l) = &link {
            format!(
                "online since {} ({} {})",
                crate::bus::rfc3339_unix(st.last_up),
                if l.dialed { "dialed to" } else { "dialed from" },
                l.ip
            )
        } else if st.cannot == "old" {
            "runs an older bia without peer links".into()
        } else if st.cannot == "notpeer" {
            "does not accept this bia's certificate".into()
        } else if rt.dialing {
            "connecting".into()
        } else if st.last_down > 0 {
            format!(
                "offline since {}{}",
                crate::bus::rfc3339_unix(st.last_down),
                if st.want { " (reconnecting)" } else { "" }
            )
        } else {
            format!("offline{}", if st.want { " (connecting)" } else { "" })
        };
        let mut transfers = vec![];
        for p in &st.inbox {
            for f in p.files.iter().filter(|f| !f.settled()) {
                let have = fs::metadata(self.part_path(name, &f.fid))
                    .map(|m| m.len())
                    .unwrap_or(0);
                transfers.push(format!(
                    "from {name}: {} {}% ({}/{})",
                    f.name,
                    pct(have, f.size),
                    human_bytes(have),
                    human_bytes(f.size)
                ));
            }
        }
        if let Some(l) = &link {
            let files = self.out_files(name);
            for (fid, acked, end) in l.w.sending() {
                let n = files.get(&fid).map(|f| f.name.clone()).unwrap_or(fid);
                transfers.push(format!(
                    "to {name}: {n} {}% ({}/{})",
                    pct(acked, end),
                    human_bytes(acked),
                    human_bytes(end)
                ));
            }
        }
        json!({
            "name": name,
            "online": link.is_some(),
            "state": state,
            "want": st.want,
            "banned_by": st.banned_by,
            "banned_us": st.banned_us,
            "reached": st.reached,
            "last_error": rt.last_err,
            "instance": link.as_ref().map(|l| l.instance.lock().unwrap().clone()).unwrap_or_default(),
            "transfers": transfers,
        })
    }

    /// Seconds since this process started (tests).
    #[allow(dead_code)]
    pub fn uptime(&self) -> u64 {
        self.started.elapsed().as_secs()
    }
}

// ---------------------------------------------------------------- helpers

fn set_sndbuf(tcp: &std::net::TcpStream) {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        // a small kernel buffer keeps messages from waiting behind megabytes of file
        let v: libc::c_int = 64 * 1024;
        unsafe {
            libc::setsockopt(
                tcp.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_SNDBUF,
                &v as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            );
        }
    }
}

pub fn set_sndbuf_shared(tls: &SharedTls) {
    tls.with_tcp(set_sndbuf);
}

fn free_bytes(p: &Path) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let c = std::ffi::CString::new(p.as_os_str().as_bytes()).unwrap_or_default();
        let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(c.as_ptr(), &mut s) } == 0 {
            return (s.f_bavail as u64).saturating_mul(s.f_frsize as u64);
        }
    }
    u64::MAX
}

fn mtime(m: &fs::Metadata) -> u64 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn sha256_file(p: &Path) -> std::io::Result<String> {
    let mut f = fs::File::open(p)?;
    let mut ctx = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        ctx.update(&buf[..n]);
    }
    Ok(hex(ctx.finish().as_ref()))
}

fn safe_file_name(n: &str) -> String {
    let base = n.rsplit(['/', '\\']).next().unwrap_or("file");
    let s: String = base.chars().filter(|c| !c.is_control()).take(120).collect();
    let s = s.trim().trim_start_matches('.').to_string();
    if s.is_empty() {
        "file".into()
    } else {
        s
    }
}

fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let _ = fs::create_dir_all(dir);
    let p = dir.join(name);
    if !p.exists() {
        return p;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.to_string(), String::new()),
    };
    (2..)
        .map(|i| dir.join(format!("{stem} ({i}){ext}")))
        .find(|p| !p.exists())
        .unwrap()
}

fn snippet(t: &str) -> String {
    let s = t.replace('\n', " ");
    if s.chars().count() > 80 {
        s.chars().take(80).collect::<String>() + "…"
    } else {
        s
    }
}

fn pct(a: u64, b: u64) -> u64 {
    if b == 0 {
        100
    } else {
        a.saturating_mul(100) / b
    }
}

fn human_bytes(n: u64) -> String {
    match n {
        0..=1023 => format!("{n} B"),
        1024..=1_048_575 => format!("{} KB", n >> 10),
        1_048_576..=1_073_741_823 => format!("{:.1} MB", n as f64 / 1_048_576.0),
        _ => format!("{:.2} GB", n as f64 / 1_073_741_824.0),
    }
}

fn base64_std(b: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(b)
}

// ---------------------------------------------------------------- tool handlers

fn l() -> Result<&'static Arc<Links>, String> {
    links().ok_or_else(|| "peer links are not running".to_string())
}

fn peer_arg(args: &Value) -> Result<String, String> {
    let p = arg_str_opt(args, "peer")
        .unwrap_or_default()
        .trim()
        .trim_start_matches(CH_PREFIX)
        .to_string();
    if p.is_empty() {
        return Err("peer is required (a name from web_peer_list)".into());
    }
    Ok(p)
}

/// The peer a `peer:<name>` channel names (agent-only send & co.).
pub fn channel_peer(args: &Value) -> Result<String, String> {
    let ch = arg_str_opt(args, "channel").unwrap_or_default().trim();
    ch.strip_prefix(CH_PREFIX)
        .filter(|n| !n.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("{ch} is not a peer channel"))
}

pub fn tool_connect(_: &Ctx, args: Value) -> Result<String, String> {
    l()?.connect(&peer_arg(&args)?)
}

pub fn tool_disconnect(_: &Ctx, args: Value) -> Result<String, String> {
    l()?.disconnect(&peer_arg(&args)?)
}

pub fn tool_ban(_: &Ctx, args: Value) -> Result<String, String> {
    l()?.ban(&peer_arg(&args)?, "model")
}

pub fn tool_unban(_: &Ctx, args: Value) -> Result<String, String> {
    l()?.unban(&peer_arg(&args)?, "model")
}

pub fn tool_cancel(_: &Ctx, args: Value) -> Result<String, String> {
    let id = arg_str_opt(&args, "message_id")
        .unwrap_or_default()
        .trim()
        .to_string();
    l()?.cancel(&peer_arg(&args)?, &id)
}

/// Root's controls (console /peer, web Settings): bans by root outrank the model's.
pub fn tool_root(_: &Ctx, args: Value) -> Result<String, String> {
    let action = arg_str_opt(&args, "action")
        .unwrap_or_default()
        .trim()
        .to_string();
    let links = l()?;
    if action == "list" {
        return crate::pki::peer_list_text();
    }
    let name = peer_arg(&args)?;
    match action.as_str() {
        "connect" => links.connect(&name),
        "disconnect" => links.disconnect(&name),
        "ban" => links.ban(&name, "root"),
        "unban" => links.unban(&name, "root"),
        other => Err(format!(
            "unknown action {other}: list, connect, disconnect, ban, unban"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_paths() {
        assert_eq!(safe_file_name("../../etc/passwd"), "passwd");
        assert_eq!(safe_file_name(".hidden"), "hidden");
        assert_eq!(safe_file_name(""), "file");
        let d = std::env::temp_dir().join(format!("bia-peer-u-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("a.txt"), "x").unwrap();
        assert_eq!(unique_path(&d, "a.txt"), d.join("a (2).txt"));
        assert_eq!(unique_path(&d, "b.txt"), d.join("b.txt"));
        assert_eq!(
            sha256_file(&d.join("a.txt")).unwrap(),
            "2d711642b726b04401627ca9fbac32f5c8530fb1903cc4db02258717921a4881"
        );
        let _ = fs::remove_dir_all(&d);
        assert_eq!(human_bytes(3 << 20), "3.0 MB");
        assert_eq!(pct(1, 4), 25);
        assert!(is_peer("peer:fred") && !is_peer("web:x"));
    }
}
