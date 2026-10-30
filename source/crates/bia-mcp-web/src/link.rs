//! The web bus: one live WebSocket session per identity (a person's web app, bia-bridge,
//! another bia), whoever opened it. See ARCHITECTURE.md ("The web bus protocol") and
//! `linkproto.rs`.
//!
//! As the server (someone opened `GET /ws` here) this side sends the identity's chat as it
//! changes (`m` / `gone`, caught up from the client's `rx`) and runs the client's actions.
//! As the client (this bia dialed another bia with the certificate that bia gave it) it
//! applies the other bia's copy of the chat and sends its own changes as actions, kept in
//! an outbox until acknowledged. Files are pulled by the receiver over the same socket.
//! Nothing is lost while the other side is away: the store is the log on one side, the
//! outbox on the other.
use crate::bus::{mint_msg_id, Bus, Inbound, ID_SYSTEM};
use crate::linkproto::*;
use crate::tlspipe::SharedTls;
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

/// A change of ours not yet acknowledged by the bia we dial.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Out {
    pub cid: String,
    /// our chat revision of the change (a state the peer acked as server covers it)
    pub rev: u64,
    pub action: Value,
}

/// What this bia remembers about one identity's link (`state/links/<identity>.json`):
/// for another bia whether to dial it and what came from it; for anyone a ban.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LinkState {
    /// Keep a link up with this bia: dial it, redial after a loss.
    #[serde(default)]
    pub want: bool,
    /// "", "root" or "model": this bia refuses the identity.
    #[serde(default)]
    pub banned_by: String,
    /// The other bia refused this one (bye banned).
    #[serde(default)]
    pub banned_us: bool,
    /// This bia has dialed it successfully at least once.
    #[serde(default)]
    pub reached: bool,
    /// Why it cannot link ("old": an older bia; "notpeer": it refused our certificate),
    /// already told; "" when fine.
    #[serde(default)]
    pub cannot: String,
    #[serde(default)]
    pub last_up: u64,
    #[serde(default)]
    pub last_down: u64,
    /// The last revision of the other bia's copy applied here (as its client).
    #[serde(default)]
    pub rx: u64,
    #[serde(default)]
    pub outbox: Vec<Out>,
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
                return Item::Frame(1, PING.to_vec());
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
    /// The identity on the other side (another bia: its name in this bia's CA).
    peer: String,
    /// This bia dialed it (it is the client).
    dialed: bool,
    /// The identity's certificate has root trust (server side).
    root: bool,
    /// Its certificate's serial (server side), checked against revocation.
    serial: String,
    /// What the other bia calls this bia (client side).
    me_there: String,
    ip: String,
    instance: Mutex<String>,
    /// The tab is in front (a person's web app says so with `active`).
    active: AtomicBool,
    w: Arc<Writer>,
    tls: SharedTls,
    last_rx: Mutex<Instant>,
    their_bye: Mutex<Option<String>>,
    our_bye: Mutex<Option<String>>,
    /// Replaced by another link of the same identity: closing it says nothing.
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
    last_err: String,
    err_at: Option<Instant>,
    /// Files being pulled from this identity: fid → size.
    pulls: HashMap<String, u64>,
    rate: VecDeque<Instant>,
    /// Second sessions refused while this live one talks: since when, and whether told.
    dup_since: Option<Instant>,
    dup_told: bool,
    /// m/gone frames applied since the state was last saved (client side).
    unsaved: u32,
}

#[derive(Default)]
struct Inner {
    rt: HashMap<String, Rt>,
    handshakes: usize,
    ip_hits: HashMap<String, VecDeque<Instant>>,
    next_id: u64,
    /// Active peers (bias with trust both ways) and their addresses.
    peers: HashMap<String, String>,
}

pub struct Links {
    bus: Arc<Bus>,
    files: PathBuf,
    /// Random per process: tells a restart from a second machine with the same key.
    instance: String,
    inner: Mutex<Inner>,
    changed: Condvar,
    closing: AtomicBool,
    ticks: AtomicU64,
}

static LINKS: OnceLock<Arc<Links>> = OnceLock::new();

pub fn links() -> Option<&'static Arc<Links>> {
    LINKS.get()
}

/// Start the link manager (once): bring in old peer chats, dial peers, retire old files.
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
                ticks: AtomicU64::new(0),
            })
        })
        .clone();
    l.import_old_peer_chats();
    l.refresh_peers();
    let t = l.clone();
    let _ = std::thread::Builder::new()
        .name("link-tick".into())
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

// ---- what the chat store and the tools ask ----

/// A frame for the identity's live session (server side). Toward a bia this bia dialed,
/// only the typing indicator goes (our changes travel as actions there).
pub fn emit(identity: &str, v: Value) {
    let Some(l) = links() else { return };
    let link = l.inner.lock().unwrap().rt.get(identity).and_then(|r| r.link.clone());
    let Some(link) = link else { return };
    if !link.dialed {
        link.w.json(&v);
    } else if v["t"] == "typing" {
        link.w.json(&json!({"t": "typing", "on": v["on"]}));
    }
}

/// Something of the identity's is connected.
pub fn online(identity: &str) -> bool {
    links().is_some_and(|l| {
        l.inner
            .lock()
            .unwrap()
            .rt
            .get(identity)
            .is_some_and(|r| r.link.is_some())
    })
}

/// The identity is looking at the web app right now (no push needed).
pub fn active(identity: &str) -> bool {
    links().is_some_and(|l| {
        l.inner
            .lock()
            .unwrap()
            .rt
            .get(identity)
            .and_then(|r| r.link.as_ref())
            .is_some_and(|k| k.active.load(Ordering::Relaxed))
    })
}

/// A change this bia made in `identity`'s chat. For another bia it also waits in the
/// outbox until that bia has it (sent now when this bia is its client).
pub fn local_change(identity: &str, action: Value, rev: u64) {
    let Some(l) = links() else { return };
    l.queue_out(identity, action, rev);
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

    fn state_path(&self, name: &str) -> PathBuf {
        let safe: String = name
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || "-_.@".contains(c) { c } else { '_' })
            .collect();
        self.files.join("state").join("links").join(format!("{safe}.json"))
    }

    fn inbox_dir(&self, name: &str) -> PathBuf {
        self.files.join(crate::chat::inbox_rel(name))
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
        fs::read(self.state_path(name))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
    }

    fn save_state(&self, name: &str, st: &LinkState) {
        let p = self.state_path(name);
        let _ = fs::create_dir_all(p.parent().unwrap());
        let tmp = p.with_extension("json.tmp");
        if fs::write(&tmp, serde_json::to_vec_pretty(st).unwrap_or_default()).is_ok() {
            let _ = fs::rename(&tmp, &p);
            self.own(&p);
        }
    }

    /// The runtime record of `name`, loaded from disk the first time.
    fn rt<'a>(&self, inner: &'a mut Inner, name: &str) -> &'a mut Rt {
        let is_peer = inner.peers.contains_key(name);
        let rt = inner.rt.entry(name.to_string()).or_default();
        if !rt.loaded {
            rt.loaded = true;
            match self.load_state(name) {
                Some(st) => rt.st = st,
                None if is_peer => {
                    // a new peer: keep a link up from the start
                    rt.st.want = true;
                    self.save_state(name, &rt.st);
                }
                None => {}
            }
        }
        rt
    }

    /// A notice for the model about a link (system, on web).
    fn tell(&self, name: &str, text: &str) {
        if self.closing.load(Ordering::Relaxed) {
            return;
        }
        self.bus.log(&format!(
            "  link {name}: {}",
            text.lines().next().unwrap_or("")
        ));
        self.bus.push_inbound(Inbound {
            identity: ID_SYSTEM.into(),
            channel: crate::bus::CH_WEB.into(),
            id: mint_msg_id(),
            text: format!("[web link {name}] {text}"),
        });
    }

    /// Bias with trust both ways (from the trust store).
    fn refresh_peers(&self) {
        let Ok(pki) = self.pki() else { return };
        let peers: HashMap<String, String> = pki
            .peers()
            .into_iter()
            .filter(|p| p.status == "active")
            .map(|p| (p.name, p.url))
            .collect();
        self.inner.lock().unwrap().peers = peers;
    }

    fn is_peer(&self, name: &str) -> bool {
        self.inner.lock().unwrap().peers.contains_key(name)
    }

    /// Chats of the peer links before 1.9 (`state/peers/<name>/`) become those bias'
    /// web chats, once; their link state carries over.
    fn import_old_peer_chats(&self) {
        let dir = self.files.join("state").join("peers");
        let Ok(rd) = fs::read_dir(&dir) else { return };
        for e in rd.flatten() {
            let d = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if !d.is_dir() || d.join("imported").exists() {
                continue;
            }
            let n = crate::chat::import_peer_log(&name, &d.join("chat.jsonl"));
            if let Some(old) = fs::read(d.join("link.json"))
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            {
                if self.load_state(&name).is_none() {
                    let st = LinkState {
                        want: old["want"].as_bool().unwrap_or(true),
                        banned_by: old["banned_by"].as_str().unwrap_or("").into(),
                        reached: old["reached"].as_bool().unwrap_or(false),
                        ..Default::default()
                    };
                    self.save_state(&name, &st);
                }
            }
            let _ = fs::write(d.join("imported"), format!("{n} messages into the web chat\n"));
            self.own(&d.join("imported"));
            if n > 0 {
                self.bus.log(&format!("  link {name}: {n} messages of the old peer chat are now its web chat"));
            }
        }
    }

    // ---- the outbox (our changes for a bia this bia dials) ----

    fn queue_out(&self, name: &str, action: Value, rev: u64) {
        let mut inner = self.inner.lock().unwrap();
        if !inner.peers.contains_key(name) {
            return;
        }
        let rt = self.rt(&mut inner, name);
        let o = Out { cid: format!("o{}", hex(&rnd_bytes::<6>())), rev, action };
        rt.st.outbox.push(o.clone());
        self.save_state(name, &rt.st);
        if let Some(l) = rt.link.as_ref().filter(|l| l.dialed) {
            l.w.json(&out_frame(&o));
        }
    }

    // ---- admission (server side, before the WebSocket upgrade) ----

    /// Handshakes from `ip`: at most IP_HANDSHAKES_PER_MIN a minute. Err = HTTP status.
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

    /// Someone opened `/ws` here (server thread, after `101`): run the session.
    pub fn accept(
        self: &Arc<Self>,
        tls: SharedTls,
        reader: BufReader<SharedTls>,
        id: &ClientId,
        ip: &str,
        root: bool,
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
            Side::Server { root, serial: id.serial_hint.clone() },
            tls,
            Box::new(reader),
            ip.to_string(),
        );
    }

    // ---- dialing another bia ----

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
            "GET /ws HTTP/1.1\r\nHost: {hosthdr}\r\nUser-Agent: bia-link\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
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
        let mut len = 0usize;
        loop {
            let mut h = String::new();
            if reader.read_line(&mut h).map_err(|e| format!("read: {e}"))? == 0
                || h.trim().is_empty()
            {
                break;
            }
            if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
                len = v.trim().parse().unwrap_or(0);
            }
        }
        let code: u16 = status
            .split_whitespace()
            .nth(1)
            .and_then(|c| c.parse().ok())
            .unwrap_or(0);
        if code != 101 {
            let mut body = vec![0u8; len.min(4096)];
            let _ = reader.read_exact(&mut body);
            tls.shutdown();
            let body = String::from_utf8_lossy(&body);
            return Err(match code {
                // a 1.8 bia: its chat socket refuses a peer certificate
                403 if body.contains("only for the live link") => "OLD".into(),
                404 => "OLD".into(),
                403 | 401 => "NOTPEER".into(),
                other => format!("{} answered HTTP {other}", p.url),
            });
        }
        {
            let mut inner = self.inner.lock().unwrap();
            let rt = self.rt(&mut inner, name);
            if !rt.st.reached {
                rt.st.reached = true;
                self.save_state(name, &rt.st);
            }
        }
        let me_there = if p.my_identity.is_empty() { self.me() } else { p.my_identity.clone() };
        let me = self.clone();
        let name = name.to_string();
        let _ = std::thread::Builder::new()
            .name(format!("link-{name}"))
            .spawn(move || me.run_link(name, Side::Client { me_there }, tls, Box::new(reader), ip));
        Ok(())
    }

    fn start_dial(self: &Arc<Self>, name: &str) {
        let me = self.clone();
        let name = name.to_string();
        let _ = std::thread::Builder::new().name(format!("link-dial-{name}")).spawn(move || {
            let r = me.dial(&name);
            let mut inner = me.inner.lock().unwrap();
            inner.handshakes = inner.handshakes.saturating_sub(1);
            let rt = me.rt(&mut inner, &name);
            rt.dialing = false;
            let mut tell = None;
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
                    rt.last_err = format!("{name} runs an older bia (before 1.9)");
                    rt.err_at = Some(Instant::now());
                    rt.next_dial = Some(Instant::now() + Duration::from_secs(3600));
                    if rt.st.cannot.is_empty() {
                        rt.st.cannot = "old".into();
                        me.save_state(&name, &rt.st);
                        tell = Some(format!("{name} runs an older bia: it must update (to bia 1.9 or newer) before you can talk; what you send waits here meanwhile. This bia tries again every hour."));
                    }
                }
                Err(e) if e == "NOTPEER" => {
                    rt.err_at = Some(Instant::now());
                    rt.next_dial = Some(Instant::now() + backoff(rt.attempts.min(4), rnd01()));
                    rt.attempts = rt.attempts.saturating_add(1);
                    if !rt.st.reached {
                        // trust was just set up: it has not collected its certificate yet
                        rt.last_err = format!("{name} does not know this bia yet");
                    } else {
                        rt.last_err = format!("{name} does not accept this bia's certificate any more");
                        if rt.st.cannot.is_empty() {
                            rt.st.cannot = "notpeer".into();
                            rt.next_dial = Some(Instant::now() + Duration::from_secs(3600));
                            me.save_state(&name, &rt.st);
                            tell = Some(format!("{name} refused this bia's certificate (it no longer trusts this bia). Tell root; trust must be set up again to talk. This bia tries again every hour."));
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
            if let Some(t) = tell {
                me.tell(&name, &t);
            }
            me.changed.notify_all();
        });
    }

    // ---- the link itself (runs in its reader thread) ----

    fn run_link(
        self: &Arc<Self>,
        name: String,
        side: Side,
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
        let dialed = matches!(side, Side::Client { .. });
        let w = Arc::new(Writer {
            q: Mutex::new(Wq::default()),
            cv: Condvar::new(),
        });
        let (root, serial, me_there) = match &side {
            Side::Server { root, serial } => (*root, serial.clone(), String::new()),
            Side::Client { me_there } => (false, String::new(), me_there.clone()),
        };
        let link = Arc::new(Link {
            id,
            peer: name.clone(),
            dialed,
            root,
            serial,
            me_there,
            ip,
            instance: Mutex::new(String::new()),
            active: AtomicBool::new(false),
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
                .name(format!("link-w-{name}"))
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
        // the client says hello first; the server answers with hello once it took the
        // session (or bye when it refuses: banned, a second live session)
        if dialed {
            let rx = {
                let mut inner = self.inner.lock().unwrap();
                self.rt(&mut inner, &name).st.rx
            };
            w.json(&json!({"t": "hello", "proto": PROTO, "name": link.me_there, "ver": env!("CARGO_PKG_VERSION"), "instance": self.instance, "rx": rx}));
        }
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
            loop {
                match crate::server::ws_recv_max(&mut reader, MAX_FRAME) {
                    Ok((op, payload)) => {
                        if op != 10 {
                            *link.last_rx.lock().unwrap() = Instant::now();
                        } else if link.last_rx.lock().unwrap().elapsed() > DEAD_AFTER {
                            // only pongs for a while: nothing behind them is alive
                            break;
                        }
                        match op {
                            1 => {
                                if let Ok(v) = serde_json::from_slice::<Value>(&payload) {
                                    if dialed {
                                        self.on_server_frame(&link, &v);
                                    } else {
                                        self.on_client_frame(&link, &v);
                                    }
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
        // a refusal's bye goes out before the socket closes
        w.wait_done(Instant::now() + BYE_FLUSH);
        tls.shutdown();
        if registered.is_ok() {
            self.on_closed(&link);
        } else {
            self.on_rejected(&link);
        }
    }

    /// The other side's hello: take the session, or refuse it (banned, a second live
    /// session, a copy of a bia).
    fn register(self: &Arc<Self>, link: &Arc<Link>, hello: &Value) -> Result<(), ()> {
        let name = &link.peer;
        let instance = hello["instance"]
            .as_str()
            .filter(|i| !i.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| hex(&rnd_bytes::<8>()));
        let proto = hello["proto"].as_u64().unwrap_or(0);
        if proto != PROTO {
            if link.dialed {
                // the server is a bia before 1.9 (its chat socket speaks the old way)
                link.silent_close("proto");
                let mut inner = self.inner.lock().unwrap();
                let rt = self.rt(&mut inner, name);
                rt.next_dial = Some(Instant::now() + Duration::from_secs(3600));
                let first = rt.st.cannot.is_empty();
                rt.st.cannot = "old".into();
                self.save_state(name, &rt.st);
                drop(inner);
                if first {
                    self.tell(name, &format!("{name} runs an older bia: it must update (to bia 1.9 or newer) before you can talk; what you send waits here meanwhile."));
                }
            } else {
                link.close("proto");
            }
            return Err(());
        }
        *link.instance.lock().unwrap() = instance.clone();
        let me = self.me();
        let mut inner = self.inner.lock().unwrap();
        if self.closing.load(Ordering::Relaxed) {
            drop(inner);
            link.silent_close("shutdown");
            return Err(());
        }
        let rt = self.rt(&mut inner, name);
        if !rt.st.banned_by.is_empty() {
            drop(inner);
            link.close("banned");
            return Err(());
        }
        if let Some(old) = rt.link.clone() {
            let old_instance = old.instance.lock().unwrap().clone();
            if old_instance == instance {
                if old.dialed != link.dialed {
                    // two bias dialed each other at once: both keep the link the lower
                    // name dialed
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
                // otherwise the old one is stale (a reload, NAT rebinding, a half-open socket)
                old.silent_close("duplicate");
            } else if old.last_rx.lock().unwrap().elapsed() < DEAD_AFTER {
                // another installation while the live one talks: a second tab or device,
                // or a copy using the same key. It may try again once this one is gone.
                let copy = {
                    let since = *rt.dup_since.get_or_insert_with(Instant::now);
                    let report = since.elapsed() >= COPY_AFTER && !rt.dup_told;
                    rt.dup_told |= report;
                    report
                };
                drop(inner);
                link.silent_close("duplicate_key");
                if copy && self.is_peer(name) {
                    self.tell(name, &format!(
                        "A second machine (from {}) keeps trying to connect as {name} while {name} is connected from {}. It was refused. Tell root now: one of the two machines is a copy using {name}'s key.",
                        link.ip, old.ip
                    ));
                } else {
                    self.bus.log(&format!("  link {name}: a second session from {} refused (one is live from {})", link.ip, old.ip));
                }
                return Err(());
            } else {
                // the old one went silent: it is gone
                old.silent_close("duplicate");
            }
        }
        let rt = self.rt(&mut inner, name);
        rt.link = Some(link.clone());
        rt.dup_since = None;
        rt.dup_told = false;
        rt.dialing = false;
        rt.attempts = 0;
        rt.next_dial = None;
        rt.st.banned_us = false;
        rt.st.cannot.clear();
        rt.st.last_up = now_secs();
        if link.dialed {
            // a link both ends took is wanted again, whoever closed the last one
            rt.st.want = true;
            let rev = hello["rev"].as_u64().unwrap_or(0);
            if rev < rt.st.rx {
                // the other bia's copy is younger than what we saw (it was reinstalled)
                rt.st.rx = rev;
            }
            for o in &rt.st.outbox {
                link.w.json(&out_frame(o));
            }
        }
        self.save_state(name, &rt.st);
        drop(inner);
        if !link.dialed {
            let mut h = json!({"t": "hello", "proto": PROTO, "name": me, "ver": env!("CARGO_PKG_VERSION"), "instance": self.instance,
                               "me": {"identity": name, "root": link.root}});
            if let Some(o) = crate::chat::hello_info(name).as_object() {
                for (k, v) in o {
                    h[k] = v.clone();
                }
            }
            link.w.json(&h);
            match hello["rx"].as_u64() {
                Some(rx) => match crate::chat::since(name, rx) {
                    Some(frames) => {
                        for f in &frames {
                            link.w.json(f);
                        }
                    }
                    None => link.w.json(&json!({"t": "reset"})),
                },
                None => {}
            }
        }
        self.bus.web_opened(name, &link.ip);
        self.changed.notify_all();
        self.pump(name);
        if link.dialed {
            self.renew_there(name);
        }
        Ok(())
    }

    /// Our certificate at a peer: renewed over HTTPS 30 days before it expires.
    fn renew_there(&self, name: &str) {
        let Ok(pki) = self.pki() else { return };
        if !pki.peer_cert_days_left(name).is_some_and(|d| d < RENEW_WITHIN_DAYS) {
            return;
        }
        let name = name.to_string();
        let files = self.files.clone();
        let bus = self.bus.clone();
        std::thread::spawn(move || {
            let r = (|| -> Result<(), String> {
                let pki = Pki::load_or_create(&files).map_err(|e| format!("{e:#}"))?;
                let p = pki.peer(&name).ok_or("no such peer")?;
                let (ca, cert, key) = pki.peer_tls(&name).map_err(|e| format!("{e:#}"))?;
                let csr = pki.agent_renew_csr().map_err(|e| format!("{e:#}"))?;
                let r = client::request(&p.url, "POST", "/v1/pki/renew", Some(&json!({"csr": csr})), client::ServerTrust::Ca(&ca), Some((&cert, &key)))
                    .map_err(|e| format!("{e:#}"))?;
                if r.status != 200 {
                    return Err(format!("HTTP {} {}", r.status, r.text().trim()));
                }
                pki.peer_save_cert(&name, &r.text()).map_err(|e| format!("{e:#}"))
            })();
            match r {
                Ok(()) => bus.log(&format!("  link {name}: this bia's certificate there was renewed")),
                Err(e) => bus.log(&format!("  link {name}: renewal there failed: {e}")),
            }
        });
    }

    fn on_rejected(&self, link: &Arc<Link>) {
        if !link.dialed {
            return;
        }
        let bye = link.their_bye.lock().unwrap().clone();
        let name = &link.peer;
        let Some(reason) = bye else { return };
        let mut inner = self.inner.lock().unwrap();
        let rt = self.rt(&mut inner, name);
        let text = match reason.as_str() {
            "banned" => {
                rt.st.banned_us = true;
                rt.st.want = false;
                Some(format!("{name} banned this bia: it refuses the link. What you send waits here until it lifts the ban."))
            }
            "duplicate_key" => {
                // our own old session there may still count as live (a crash and a quick
                // restart): try again once it has gone quiet; only lasting refusals mean a copy
                rt.next_dial = Some(Instant::now() + DEAD_AFTER + Duration::from_secs(1));
                let since = *rt.dup_since.get_or_insert_with(Instant::now);
                if since.elapsed() >= COPY_AFTER && !rt.dup_told {
                    rt.dup_told = true;
                    Some(format!("{name} keeps refusing this connection: another machine using this bia's key is connected to it. Tell root now: one of the two machines is a copy."))
                } else {
                    None
                }
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
        let current = rt.link.as_ref().map(|l| l.id) == Some(link.id);
        if current {
            rt.link = None;
            rt.pulls.clear();
            rt.st.last_down = now_secs();
            rt.unsaved = 0;
        }
        let theirs = link.their_bye.lock().unwrap().clone();
        let ours = link.our_bye.lock().unwrap().clone();
        let mut tell = None;
        if current && !link.replaced.load(Ordering::Relaxed) && !self.closing.load(Ordering::Relaxed) {
            match (theirs.as_deref(), ours) {
                (Some(r), _) if BYE_RESTARTS.contains(&r) => {
                    rt.attempts = 0;
                    rt.next_dial = Some(Instant::now() + backoff(0, rnd01()));
                }
                // whichever side dialed: another bia closed the link for good
                (Some("banned"), _) => {
                    rt.st.want = false;
                    rt.st.banned_us = true;
                    tell = Some(format!("{name} banned this bia: it refuses the link. What you send waits here until it lifts the ban."));
                }
                (Some("revoked"), _) => {
                    rt.st.want = false;
                    tell = Some(format!("{name} no longer trusts this bia (its certificate there was revoked). Tell root; trust must be set up again (web_peer_add) to talk."));
                }
                (Some("disconnect"), _) => rt.st.want = false,
                (_, Some(_)) => {}
                _ => {
                    // network loss: redial at once (jittered)
                    rt.attempts = 0;
                    rt.next_dial =
                        Some(Instant::now() + Duration::from_millis(500 + (rnd01() * 2500.0) as u64));
                }
            }
        }
        self.save_state(&name, &rt.st);
        drop(inner);
        crate::bus::Bus::web_closed(&self.bus, &name);
        if let Some(t) = tell {
            self.tell(&name, &t);
        }
        self.changed.notify_all();
    }

    // ---- frames ----

    /// What a client sent this server: actions, acks, file transfer.
    fn on_client_frame(self: &Arc<Self>, link: &Arc<Link>, v: &Value) {
        let name = link.peer.as_str();
        match v["t"].as_str().unwrap_or("") {
            "ping" | "typing" => {}
            "active" => link.active.store(v["on"].as_bool().unwrap_or(false), Ordering::Relaxed),
            "ack" => {
                // a bia that dialed us has our changes up to seq: its outbox here is done
                let seq = v["seq"].as_u64().unwrap_or(0);
                let mut inner = self.inner.lock().unwrap();
                if inner.peers.contains_key(name) {
                    let rt = self.rt(&mut inner, name);
                    let n = rt.st.outbox.len();
                    rt.st.outbox.retain(|o| o.rev > seq);
                    if rt.st.outbox.len() != n {
                        self.save_state(name, &rt.st);
                    }
                }
            }
            "bye" => {
                // the client is leaving (a tab closing): its session ends now, so the
                // next one of this identity is not kept waiting
                *link.their_bye.lock().unwrap() = v["reason"].as_str().map(str::to_string);
                link.tls.shutdown();
            }
            "fget" | "fack" | "fdone" | "frefuse" | "fcancel" => self.on_file_frame(link, v),
            _ => {
                if self.slow_down(name) {
                    std::thread::sleep(Duration::from_secs(1));
                }
                let cid = v.get("cid").cloned().unwrap_or(Value::Null);
                let ack = match crate::chat::action(name, link.root, v) {
                    Ok(r) => json!({"t": "ack", "cid": cid, "ok": true, "r": r}),
                    Err(e) => json!({"t": "ack", "cid": cid, "ok": false, "error": e}),
                };
                link.w.json(&ack);
                if v["t"] == "send" {
                    self.pump(name);
                }
            }
        }
    }

    /// What the bia this one dialed sent: its copy of the chat, acks of our actions,
    /// file transfer.
    fn on_server_frame(self: &Arc<Self>, link: &Arc<Link>, v: &Value) {
        let name = link.peer.as_str();
        match v["t"].as_str().unwrap_or("") {
            "m" | "gone" => {
                let seq = v["seq"].as_u64().unwrap_or(0);
                if v["t"] == "m" {
                    if let Err(e) = crate::chat::apply_remote(name, &link.me_there, &v["m"]) {
                        self.bus.log(&format!("  link {name}: {e}"));
                    }
                } else {
                    let ids: Vec<String> = v["ids"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_string)).collect();
                    crate::chat::apply_remote_gone(name, &ids);
                }
                link.w.json(&json!({"t": "ack", "seq": seq}));
                {
                    let mut inner = self.inner.lock().unwrap();
                    let rt = self.rt(&mut inner, name);
                    if seq > rt.st.rx {
                        rt.st.rx = seq;
                    }
                    rt.unsaved += 1;
                    if rt.unsaved >= 20 {
                        rt.unsaved = 0;
                        self.save_state(name, &rt.st);
                    }
                }
                self.pump(name);
            }
            "reset" => {
                // we are behind its tombstones: take its latest messages as they are
                link.w.json(&json!({"t": "hist", "cid": "reset", "limit": 200}));
            }
            "ack" => {
                let cid = v["cid"].as_str().unwrap_or("");
                if cid == "reset" {
                    for m in v["r"]["messages"].as_array().into_iter().flatten() {
                        let _ = crate::chat::apply_remote(name, &link.me_there, m);
                    }
                    let rev = v["r"]["rev"].as_u64().unwrap_or(0);
                    let mut inner = self.inner.lock().unwrap();
                    let rt = self.rt(&mut inner, name);
                    rt.st.rx = rt.st.rx.max(rev);
                    self.save_state(name, &rt.st);
                    drop(inner);
                    self.pump(name);
                    return;
                }
                if v["ok"] != true {
                    self.bus.log(&format!("  link {name}: refused {cid}: {}", v["error"].as_str().unwrap_or("")));
                }
                let mut inner = self.inner.lock().unwrap();
                let rt = self.rt(&mut inner, name);
                rt.st.outbox.retain(|o| o.cid != cid);
                rt.unsaved = 0;
                self.save_state(name, &rt.st);
            }
            "bye" => *link.their_bye.lock().unwrap() = v["reason"].as_str().map(str::to_string),
            "fget" | "fack" | "fdone" | "frefuse" | "fcancel" => self.on_file_frame(link, v),
            _ => {}
        }
    }

    /// An identity sending more than RATE_PER_MIN actions a minute is slowed, not cut off.
    fn slow_down(&self, name: &str) -> bool {
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
    }

    // ---- files ----

    fn on_file_frame(self: &Arc<Self>, link: &Arc<Link>, v: &Value) {
        let name = link.peer.as_str();
        let fid = v["fid"].as_str().unwrap_or("").to_ascii_lowercase();
        match v["t"].as_str().unwrap_or("") {
            "fget" => {
                let off = v["off"].as_u64().unwrap_or(0);
                let Some(raw) = unhex32(&fid) else { return };
                match crate::chat::file_by_fid(name, &fid) {
                    Some((path, size)) => link.w.add_job(Job {
                        fid,
                        raw,
                        path,
                        off: off.min(size),
                        end: size,
                        acked: off.min(size),
                    }),
                    None => link.w.json(&json!({"t": "frefuse", "fid": fid, "reason": "no such file here"})),
                }
            }
            "fack" => link.w.acked(&fid, v["off"].as_u64().unwrap_or(0)),
            "fdone" => link.w.drop_job(&fid),
            "frefuse" => {
                link.w.drop_job(&fid);
                // a file we were pulling that the sender cannot give
                let pulling = self.inner.lock().unwrap().rt.get(name).is_some_and(|r| r.pulls.contains_key(&fid));
                if pulling {
                    let why = v["reason"].as_str().unwrap_or("the sender refused it").to_string();
                    self.settle(name, &fid, None, &why);
                }
            }
            "fcancel" => self.settle(name, &fid, None, "the sender cancelled it"),
            _ => {}
        }
    }

    /// Start pulling what the identity listed and is not here yet.
    fn pump(self: &Arc<Self>, name: &str) {
        let want = crate::chat::pending_pulls(name);
        if want.is_empty() {
            return;
        }
        let mut refuse = vec![];
        {
            let mut inner = self.inner.lock().unwrap();
            let total: usize = inner.rt.values().map(|r| r.pulls.len()).sum();
            let rt = self.rt(&mut inner, name);
            let Some(link) = rt.link.clone() else { return };
            let mut pulling = total;
            for (fid, _fname, size) in want {
                if rt.pulls.contains_key(&fid) {
                    continue;
                }
                let have = fs::metadata(self.part_path(name, &fid)).map(|m| m.len()).unwrap_or(0);
                let free = free_bytes(&self.files);
                if free < size.saturating_sub(have) + DISK_MARGIN {
                    refuse.push((fid, format!("not enough disk space here ({} MB free)", free >> 20)));
                    continue;
                }
                if rt.pulls.len() >= PULLS_PER_LINK || pulling >= PULLS_TOTAL {
                    continue;
                }
                rt.pulls.insert(fid.clone(), size);
                pulling += 1;
                if size == 0 {
                    // nothing to pull: an empty file is complete
                    let part = self.part_path(name, &fid);
                    let _ = fs::create_dir_all(part.parent().unwrap());
                    let _ = fs::write(&part, b"");
                    refuse.push((fid, String::new()));
                    continue;
                }
                link.w.json(&json!({"t": "fget", "fid": fid, "off": have}));
            }
        }
        for (fid, why) in refuse {
            if why.is_empty() {
                let part = self.part_path(name, &fid);
                self.finish_pull(name, &fid, &part);
            } else {
                if let Some(l) = self.inner.lock().unwrap().rt.get(name).and_then(|r| r.link.clone()) {
                    l.w.json(&json!({"t": "frefuse", "fid": fid, "reason": why}));
                }
                self.settle(name, &fid, None, &why);
            }
        }
    }

    fn on_chunk(self: &Arc<Self>, link: &Arc<Link>, b: &[u8]) {
        let name = link.peer.as_str();
        let Some((fid, off, data)) = parse_chunk(b) else {
            return;
        };
        let Some(size) = self.inner.lock().unwrap().rt.get(name).and_then(|r| r.pulls.get(&fid).copied()) else {
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
            link.w.json(&json!({"t": "frefuse", "fid": fid, "reason": "the receiver could not write it"}));
            self.settle(name, &fid, None, "could not write it here");
            return;
        }
        if have == 0 {
            self.own(&part);
        }
        let have = have + take as u64;
        link.w.json(&json!({"t": "fack", "fid": fid, "off": have}));
        if have >= size {
            self.finish_pull(name, &fid, &part);
        }
    }

    /// A pull is complete: check the hash, move it into the inbox.
    fn finish_pull(self: &Arc<Self>, name: &str, fid: &str, part: &Path) {
        let link = self.inner.lock().unwrap().rt.get(name).and_then(|r| r.link.clone());
        let sum = crate::chat::sha256_file(part).unwrap_or_default();
        if sum != fid {
            let _ = fs::remove_file(part);
            if let Some(l) = &link {
                l.w.json(&json!({"t": "frefuse", "fid": fid, "reason": "it arrived damaged (checksum mismatch)"}));
            }
            self.settle(name, fid, None, "it arrived damaged (checksum mismatch)");
            return;
        }
        if let Some(l) = &link {
            l.w.json(&json!({"t": "fdone", "fid": fid}));
        }
        self.settle(name, fid, Some(part), "");
    }

    /// A pull ended: the file (from `part`) or why not reaches the chat store.
    fn settle(self: &Arc<Self>, name: &str, fid: &str, part: Option<&Path>, err: &str) {
        let fname = crate::chat::pending_pulls(name)
            .into_iter()
            .find(|(f, _, _)| f == fid)
            .map(|(_, n, _)| n)
            .unwrap_or_else(|| "file".into());
        {
            let mut inner = self.inner.lock().unwrap();
            if let Some(rt) = inner.rt.get_mut(name) {
                rt.pulls.remove(fid);
            }
        }
        match part {
            Some(p) => {
                let dest = unique_path(&self.inbox_dir(name), &fname);
                if fs::rename(p, &dest).is_ok() {
                    self.own(&dest);
                    crate::chat::file_settled(name, fid, Ok(&dest));
                } else {
                    crate::chat::file_settled(name, fid, Err("could not move it into the inbox"));
                }
            }
            None => crate::chat::file_settled(name, fid, Err(err)),
        }
        self.pump(name);
    }

    // ---- the timer ----

    fn tick(self: &Arc<Self>) {
        let n = self.ticks.fetch_add(1, Ordering::Relaxed);
        // peers set up or removed (at either end), every 5 s
        if n % 5 == 0 {
            let before: HashSet<String> = self.inner.lock().unwrap().peers.keys().cloned().collect();
            self.refresh_peers();
            let mut inner = self.inner.lock().unwrap();
            let now: Vec<String> = inner.peers.keys().cloned().collect();
            for name in &now {
                let fresh = !before.contains(name) && !inner.rt.contains_key(name) && self.load_state(name).is_none();
                let rt = self.rt(&mut inner, name);
                if fresh {
                    rt.next_dial = Some(Instant::now());
                }
            }
            // a removed peer's links go (it may still come back as an ordinary client)
            let gone: Vec<Arc<Link>> = inner
                .rt
                .iter_mut()
                .filter(|(k, _)| before.contains(*k) && !now.contains(k))
                .filter_map(|(_, r)| r.link.take())
                .collect();
            drop(inner);
            for l in gone {
                l.close("revoked");
            }
        }
        let now = Instant::now();
        let mut dial = vec![];
        {
            let mut inner = self.inner.lock().unwrap();
            let mut hs = inner.handshakes;
            let names: Vec<String> = inner.peers.keys().cloned().collect();
            for name in names {
                let has_url = inner.peers.get(&name).is_some_and(|u| !u.is_empty());
                let rt = self.rt(&mut inner, &name);
                if rt.link.is_some() || !has_url {
                    continue;
                }
                let can = rt.st.want && rt.st.banned_by.is_empty() && !rt.dialing;
                if can && rt.next_dial.is_none_or(|t| now >= t) && hs < MAX_HANDSHAKES {
                    // wanted but never scheduled (start-up): dial now
                    rt.dialing = true;
                    rt.next_dial = None;
                    hs += 1;
                    dial.push(name.clone());
                }
            }
            inner.handshakes = hs;
        }
        for name in dial {
            self.start_dial(&name);
        }
        // sessions silent for DEAD_AFTER (only pongs, or nothing) are gone
        let silent: Vec<Arc<Link>> = self
            .inner
            .lock()
            .unwrap()
            .rt
            .values()
            .filter_map(|r| r.link.clone())
            .filter(|l| l.last_rx.lock().unwrap().elapsed() > DEAD_AFTER)
            .collect();
        for l in silent {
            l.tls.shutdown();
        }
        // sessions whose certificate here was revoked or expired go
        if n % 30 == 0 {
            if let Ok(pki) = self.pki() {
                let links: Vec<Arc<Link>> = self
                    .inner
                    .lock()
                    .unwrap()
                    .rt
                    .values()
                    .filter_map(|r| r.link.clone())
                    .filter(|l| !l.dialed && !l.serial.is_empty())
                    .collect();
                for l in links {
                    let ok = pki.issued_meta(&l.serial).is_none_or(|m| {
                        !m.revoked && (m.not_after == 0 || m.not_after > now_secs())
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

    /// Unfinished transfers older than 3 days.
    fn cleanup(&self) {
        let now = now_secs();
        let Ok(rd) = fs::read_dir(self.files.join("inbox").join("web")) else { return };
        for d in rd.flatten() {
            if let Ok(parts) = fs::read_dir(d.path().join(".part")) {
                for e in parts.flatten() {
                    if e.metadata()
                        .is_ok_and(|m| now.saturating_sub(mtime(&m)) > KEEP_PARTIAL_SECS)
                    {
                        let _ = fs::remove_file(e.path());
                    }
                }
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
                "  web: said bye ({reason}) to {}",
                links
                    .iter()
                    .map(|l| l.peer.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }

    // ---- what the tools do ----

    fn require_peer(&self, name: &str) -> Result<(), String> {
        let pki = self.pki()?;
        match pki.peer(name) {
            Some(p) if p.status == "active" => Ok(()),
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

    /// Open (or keep) the link with a peer; waits up to 15 s for it.
    pub fn connect(self: &Arc<Self>, name: &str) -> Result<String, String> {
        self.require_peer(name)?;
        self.refresh_peers();
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
                return Err(format!("{name} banned this bia; what you send waits here until it lifts the ban"));
            }
            if rt.err_at.is_some_and(|t| t > start) && !rt.dialing {
                let e = rt.last_err.clone();
                let nat = if rt.st.reached {
                    ""
                } else {
                    " This bia has never reached it directly: if it is behind NAT, only it can open the link."
                };
                return Ok(format!("not connected yet: {e}. This bia keeps trying (backing off up to 10 min); what you send waits until the link is up.{nat}"));
            }
            let left = Duration::from_secs(15).saturating_sub(start.elapsed());
            if left.is_zero() {
                return Ok(format!("{name} has not answered yet; this bia keeps trying, and what you send waits until the link is up"));
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
                format!("disconnected from {name}. This bia does not dial it again by itself; {name} may connect again (web_peer_ban keeps it out). What you send meanwhile waits")
            }
            None => format!("{name} was not connected; this bia stopped trying to reach it"),
        })
    }

    /// Refuse an identity's sessions (anyone: a person, an agent, a bia).
    pub fn ban(&self, name: &str, by: &str) -> Result<String, String> {
        if name.is_empty() || name == crate::bus::ID_ROOT {
            return Err("root cannot be banned".into());
        }
        let link = {
            let mut inner = self.inner.lock().unwrap();
            let rt = self.rt(&mut inner, name);
            if rt.st.banned_by == "root" && by != "root" {
                return Ok(format!("{name} is already banned by root"));
            }
            rt.st.banned_by = by.to_string();
            rt.st.want = false;
            self.save_state(name, &rt.st);
            rt.link.clone()
        };
        if let Some(l) = link {
            l.close("banned");
        }
        Ok(format!("banned {name}: its sessions are refused until web_peer_unban; it is told when it tries"))
    }

    pub fn unban(self: &Arc<Self>, name: &str, by: &str) -> Result<String, String> {
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
        if self.require_peer(name).is_ok() {
            let r = self.connect(name)?;
            return Ok(format!("unbanned {name}; {r}"));
        }
        Ok(format!("unbanned {name}; it can connect again"))
    }

    /// Stop the files of one of our messages from going to a peer.
    pub fn cancel(&self, name: &str, message_id: &str) -> Result<String, String> {
        let link = self.inner.lock().unwrap().rt.get(name).and_then(|r| r.link.clone());
        let sending: Vec<String> = link.as_ref().map(|l| l.w.sending().into_iter().map(|(f, _, _)| f).collect()).unwrap_or_default();
        let fids = crate::chat::message_fids(name, message_id);
        let hit: Vec<&String> = fids.iter().filter(|f| sending.contains(f)).collect();
        if hit.is_empty() {
            return Err(format!("no file of message {message_id} is going to {name} now"));
        }
        if let Some(l) = &link {
            for fid in &hit {
                l.w.drop_job(fid);
                l.w.json(&json!({"t": "fcancel", "fid": fid}));
            }
        }
        Ok(format!("stopped {} file(s) of {message_id}; {name} gets the message without them", hit.len()))
    }

    /// Forget a peer's link state (the trust part is done by the caller).
    pub fn forget(&self, name: &str) {
        let link = self
            .inner
            .lock()
            .unwrap()
            .rt
            .remove(name)
            .and_then(|r| r.link);
        if let Some(l) = link {
            l.close("revoked");
        }
        let _ = fs::remove_file(self.state_path(name));
        self.refresh_peers();
    }

    /// The link of one identity (+ transfers), for web_peer_list and root's views.
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
            "runs an older bia (before 1.9)".into()
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
        for (fid, size) in &rt.pulls {
            let have = fs::metadata(self.part_path(name, fid)).map(|m| m.len()).unwrap_or(0);
            transfers.push(format!("from {name}: {} {}% ({}/{})", &fid[..12.min(fid.len())], pct(have, *size), human_bytes(have), human_bytes(*size)));
        }
        if let Some(l) = &link {
            for (fid, acked, end) in l.w.sending() {
                transfers.push(format!(
                    "to {name}: {} {}% ({}/{})",
                    &fid[..12.min(fid.len())],
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
            "waiting": st.outbox.len(),
            "last_error": rt.last_err,
            "instance": link.as_ref().map(|l| l.instance.lock().unwrap().clone()).unwrap_or_default(),
            "transfers": transfers,
        })
    }
}

/// Which end of the link this bia is.
enum Side {
    /// Someone opened `/ws` here.
    Server { root: bool, serial: String },
    /// This bia dialed another bia, which knows it as `me_there`.
    Client { me_there: String },
}

fn out_frame(o: &Out) -> Value {
    let mut f = o.action.clone();
    f["cid"] = json!(o.cid);
    f
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
    links().ok_or_else(|| "web links are not running".to_string())
}

fn peer_arg(args: &Value) -> Result<String, String> {
    let p = bia_mcp::arg_str_opt(args, "peer")
        .unwrap_or_default()
        .trim()
        .trim_start_matches("peer:")
        .to_string();
    if p.is_empty() {
        return Err("peer is required (a name from web_peer_list)".into());
    }
    Ok(p)
}

pub fn tool_connect(_: &bia_mcp::Ctx, args: Value) -> Result<String, String> {
    l()?.connect(&peer_arg(&args)?)
}

pub fn tool_disconnect(_: &bia_mcp::Ctx, args: Value) -> Result<String, String> {
    l()?.disconnect(&peer_arg(&args)?)
}

pub fn tool_ban(_: &bia_mcp::Ctx, args: Value) -> Result<String, String> {
    l()?.ban(&peer_arg(&args)?, "model")
}

pub fn tool_unban(_: &bia_mcp::Ctx, args: Value) -> Result<String, String> {
    l()?.unban(&peer_arg(&args)?, "model")
}

pub fn tool_cancel(_: &bia_mcp::Ctx, args: Value) -> Result<String, String> {
    let id = bia_mcp::arg_str_opt(&args, "message_id")
        .unwrap_or_default()
        .trim()
        .to_string();
    l()?.cancel(&peer_arg(&args)?, &id)
}

/// Root's controls (console /peer, web Settings): bans by root outrank the model's.
pub fn tool_root(_: &bia_mcp::Ctx, args: Value) -> Result<String, String> {
    let action = bia_mcp::arg_str_opt(&args, "action")
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
        let d = std::env::temp_dir().join(format!("bia-link-u-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("a.txt"), "x").unwrap();
        assert_eq!(unique_path(&d, "a.txt"), d.join("a (2).txt"));
        assert_eq!(human_bytes(1536), "1 KB");
        assert_eq!(pct(1, 4), 25);
        let o = Out { cid: "o1".into(), rev: 3, action: json!({"t": "edit", "id": "b1", "text": "x"}) };
        assert_eq!(out_frame(&o), json!({"t": "edit", "id": "b1", "text": "x", "cid": "o1"}));
        let _ = fs::remove_dir_all(&d);
    }
}
