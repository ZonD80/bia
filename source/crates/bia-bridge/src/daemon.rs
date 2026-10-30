//! The background daemon (one per profile, started by any command): finishes a pending
//! certificate request, keeps the web bus link with the bia (protocol 2, `/ws`), stores what
//! bia says in the inbox with its files pulled, sends from the outbox (what was said while
//! the link was down waits there; files are served from disk when the bia pulls them) and
//! renews the certificate. Commands and MCP sessions reach it on 127.0.0.1 with a token from
//! `daemon.json` (readable by this user only).
use crate::link;
use crate::profile::{now_secs, random_hex, Profile};
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

const CHUNK: usize = 32 * 1024;

/// What bia said, as kept in `inbox.jsonl`.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Entry {
    pub seq: u64,
    pub id: String,
    /// unix seconds
    pub ts: u64,
    /// `msg`, `edit` or `note` (the bridge itself: approval, denial, a refusal)
    pub kind: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
}

#[derive(Default)]
struct Inbox {
    entries: Vec<Entry>,
    seen: HashSet<String>,
    /// consumer → last seq it was given
    cursors: HashMap<String, u64>,
}

/// One of our actions waiting to go (`outbox.json`); `files`: fid → local path.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct Out {
    action: Value,
    #[serde(default)]
    files: HashMap<String, String>,
}

/// The link's state on disk (`link.json`): the instance and the last revision seen.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
struct LinkFile {
    instance: String,
    /// None: never connected (the first link starts from now)
    rx: Option<u64>,
}

/// A live link: frames go out under one lock.
struct Conn {
    tls: crate::link::tlspipe::SharedTls,
    w: Mutex<()>,
}

impl Conn {
    fn send(&self, op: u8, payload: &[u8]) -> Result<()> {
        let f = link::frame(op, payload);
        let _g = self.w.lock().unwrap();
        let mut t = self.tls.clone();
        t.write_all(&f)?;
        t.flush()?;
        Ok(())
    }
    fn json(&self, v: &Value) -> Result<()> {
        self.send(1, v.to_string().as_bytes())
    }
}

/// A file of bia's being pulled.
struct Pull {
    part: PathBuf,
    dest: PathBuf,
    size: u64,
}

/// A message of bia's waiting for its files.
struct Waiting {
    entry: Entry,
    fids: HashSet<String>,
}

pub struct Daemon {
    p: Mutex<Profile>,
    inbox: Mutex<Inbox>,
    cv: Condvar,
    online: AtomicBool,
    stop: AtomicBool,
    last_err: Mutex<String>,
    log: Mutex<Option<fs::File>>,
    /// The bia's version (from its hello).
    bia_version: Mutex<String>,
    conn: Mutex<Option<Arc<Conn>>>,
    outbox: Mutex<Vec<(String, Out)>>,
    acks: Mutex<HashMap<String, Value>>,
    lf: Mutex<LinkFile>,
    pulls: Mutex<HashMap<String, Pull>>,
    waiting: Mutex<Vec<Waiting>>,
    /// Texts of bia's messages seen, to tell an edit (`msgs.json`).
    texts: Mutex<HashMap<String, String>>,
}

impl Daemon {
    fn log(&self, s: &str) {
        if let Some(f) = self.log.lock().unwrap().as_mut() {
            let _ = writeln!(f, "{} {s}", now_secs());
        }
    }

    fn profile(&self) -> Profile {
        self.p.lock().unwrap().clone()
    }

    fn add(&self, mut e: Entry) {
        let key = format!("{}/{}/{}", e.id, e.kind, if e.kind == "edit" { e.text.as_str() } else { "" });
        let p = self.profile();
        let mut ib = self.inbox.lock().unwrap();
        if !ib.seen.insert(key) {
            return;
        }
        e.seq = ib.entries.last().map(|x| x.seq).unwrap_or(0) + 1;
        if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(p.path("inbox.jsonl")) {
            let _ = writeln!(f, "{}", serde_json::to_string(&e).unwrap_or_default());
        }
        ib.entries.push(e);
        self.cv.notify_all();
    }

    fn take(&self, consumer: &str, peek: bool, wait: Option<Duration>) -> Vec<Entry> {
        let end = wait.map(|w| Instant::now() + w);
        let mut ib = self.inbox.lock().unwrap();
        loop {
            let cur = ib.cursors.get(consumer).copied().unwrap_or(0);
            let out: Vec<Entry> = ib.entries.iter().filter(|e| e.seq > cur).cloned().collect();
            if !out.is_empty() || end.is_none_or(|t| Instant::now() >= t) || self.stop.load(Ordering::Relaxed) {
                if !peek {
                    let upto = ib.entries.last().map(|e| e.seq).unwrap_or(0);
                    if upto > cur {
                        ib.cursors.insert(consumer.to_string(), upto);
                        let _ = fs::write(self.profile().path("cursors.json"), serde_json::to_vec(&ib.cursors).unwrap_or_default());
                    }
                }
                return out;
            }
            let left = end.unwrap().saturating_duration_since(Instant::now()).min(Duration::from_secs(1));
            ib = self.cv.wait_timeout(ib, left).unwrap().0;
        }
    }

    fn unread(&self, consumer: &str) -> usize {
        let ib = self.inbox.lock().unwrap();
        let cur = ib.cursors.get(consumer).copied().unwrap_or(0);
        ib.entries.iter().filter(|e| e.seq > cur).count()
    }

    fn save_outbox(&self) {
        let ob = self.outbox.lock().unwrap();
        let v: Vec<Value> = ob.iter().map(|(cid, o)| json!({"cid": cid, "action": o.action, "files": o.files})).collect();
        let _ = fs::write(self.profile().path("outbox.json"), serde_json::to_vec_pretty(&v).unwrap_or_default());
    }

    fn save_link(&self) {
        let lf = self.lf.lock().unwrap().clone();
        let _ = fs::write(self.profile().path("link.json"), serde_json::to_vec_pretty(&lf).unwrap_or_default());
    }

    fn conn(&self) -> Option<Arc<Conn>> {
        self.conn.lock().unwrap().clone()
    }

    /// Send an action now (when the link is up) and wait for its ack.
    fn ask(&self, mut action: Value, wait: Duration) -> Result<Value> {
        let conn = self.conn().ok_or_else(|| anyhow!("not connected to the bia now (bia-bridge status)"))?;
        let cid = format!("q{}", random_hex(6));
        action["cid"] = json!(cid);
        conn.json(&action)?;
        let end = Instant::now() + wait;
        loop {
            if let Some(a) = self.acks.lock().unwrap().remove(&cid) {
                return if a["ok"] == true { Ok(a["r"].clone()) } else { Err(anyhow!("{}", a["error"].as_str().unwrap_or("refused"))) };
            }
            if Instant::now() >= end {
                bail!("the bia did not answer in time");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

/// Run the daemon for `name` in the foreground (`bia-bridge daemon`).
pub fn run(name: &str) -> Result<()> {
    let p = Profile::load(name)?;
    if crate::ipc::ping(&p).is_ok() {
        return Ok(()); // one is running already
    }
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    let token = random_hex(24);
    let mut ib = Inbox::default();
    for l in fs::read_to_string(p.path("inbox.jsonl")).unwrap_or_default().lines() {
        if let Ok(e) = serde_json::from_str::<Entry>(l) {
            ib.seen.insert(format!("{}/{}/{}", e.id, e.kind, if e.kind == "edit" { e.text.as_str() } else { "" }));
            ib.entries.push(e);
        }
    }
    ib.cursors = fs::read(p.path("cursors.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let outbox: Vec<(String, Out)> = fs::read(p.path("outbox.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Vec<Value>>(&b).ok())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|v| Some((v["cid"].as_str()?.to_string(), serde_json::from_value(v).ok()?)))
        .collect();
    let mut lf: LinkFile = fs::read(p.path("link.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    if lf.instance.is_empty() {
        lf.instance = random_hex(8);
    }
    let texts = fs::read(p.path("msgs.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let log = fs::OpenOptions::new().create(true).append(true).open(p.path("daemon.log")).ok();
    let d = Arc::new(Daemon {
        p: Mutex::new(p.clone()),
        inbox: Mutex::new(ib),
        cv: Condvar::new(),
        online: AtomicBool::new(false),
        stop: AtomicBool::new(false),
        last_err: Mutex::new(String::new()),
        log: Mutex::new(log),
        bia_version: Mutex::new(String::new()),
        conn: Mutex::new(None),
        outbox: Mutex::new(outbox),
        acks: Mutex::new(HashMap::new()),
        lf: Mutex::new(lf),
        pulls: Mutex::new(HashMap::new()),
        waiting: Mutex::new(vec![]),
        texts: Mutex::new(texts),
    });
    d.save_link();
    bia_pki::write_private(
        &p.path("daemon.json"),
        serde_json::to_vec(&json!({"port": port, "token": token, "pid": std::process::id(), "started": now_secs(), "version": env!("CARGO_PKG_VERSION")}))?.as_slice(),
    )?;
    d.log(&format!("daemon up on 127.0.0.1:{port}"));
    let dl = d.clone();
    std::thread::spawn(move || link_loop(dl));
    for conn in listener.incoming().flatten() {
        if d.stop.load(Ordering::Relaxed) {
            break;
        }
        let d = d.clone();
        let token = token.clone();
        std::thread::spawn(move || serve(d, conn, &token));
    }
    Ok(())
}

fn serve(d: Arc<Daemon>, conn: TcpStream, token: &str) {
    let mut line = String::new();
    let mut r = BufReader::new(match conn.try_clone() {
        Ok(c) => c,
        Err(_) => return,
    });
    if r.read_line(&mut line).is_err() {
        return;
    }
    let reply = match serde_json::from_str::<Value>(&line) {
        Ok(v) if v["token"].as_str() == Some(token) => match handle(&d, &v) {
            Ok(r) => r,
            Err(e) => json!({"ok": false, "error": format!("{e:#}")}),
        },
        _ => json!({"ok": false, "error": "bad token"}),
    };
    let mut w = conn;
    let _ = writeln!(w, "{reply}");
    if v_is_stop(&line) && reply["ok"] == true {
        d.stop.store(true, Ordering::Relaxed);
        d.cv.notify_all();
        if let Some(c) = d.conn() {
            let _ = c.json(&json!({"t": "bye", "reason": "shutdown"}));
            c.tls.shutdown();
        }
        let _ = fs::remove_file(d.profile().path("daemon.json"));
        std::thread::sleep(Duration::from_millis(100));
        std::process::exit(0);
    }
}

fn v_is_stop(line: &str) -> bool {
    serde_json::from_str::<Value>(line).map(|v| v["op"] == "stop").unwrap_or(false)
}

fn handle(d: &Arc<Daemon>, v: &Value) -> Result<Value> {
    let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
    let consumer = Some(s("consumer")).filter(|c| !c.is_empty()).unwrap_or_else(|| "default".into());
    match v["op"].as_str().unwrap_or("") {
        "ping" => Ok(json!({"ok": true, "pid": std::process::id()})),
        "stop" => Ok(json!({"ok": true})),
        "status" => {
            let p = d.profile();
            Ok(json!({
                "ok": true,
                "profile": p.name, "url": p.url, "identity": p.identity, "state": p.state,
                "message": p.message, "request": p.request, "ca_sha256": p.ca_sha256,
                "online": d.online.load(Ordering::Relaxed),
                "last_error": d.last_err.lock().unwrap().clone(),
                "unread": d.unread(&consumer),
                "waiting": d.outbox.lock().unwrap().len(),
                "cert_days_left": p.days_left(),
                "bia_version": d.bia_version.lock().unwrap().clone(),
                "bridge_version": env!("CARGO_PKG_VERSION"),
            }))
        }
        "read" => Ok(json!({"ok": true, "messages": d.take(&consumer, v["peek"] == true, None)})),
        "wait" => {
            let t = v["timeout"].as_u64().unwrap_or(300).min(3600);
            Ok(json!({"ok": true, "messages": d.take(&consumer, false, Some(Duration::from_secs(t)))}))
        }
        "send" => {
            let files: Vec<String> = v["files"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_string)).collect();
            let (id, sent) = send(d, &s("text"), &files, &s("reply_to"))?;
            Ok(json!({"ok": true, "id": id, "sent": sent}))
        }
        "history" => {
            let limit = v["limit"].as_u64().unwrap_or(20).clamp(1, 200);
            let r = d.ask(json!({"t": "hist", "limit": limit}), Duration::from_secs(30))?;
            Ok(json!({"ok": true, "messages": r["messages"]}))
        }
        other => Err(anyhow!("unknown op {other}")),
    }
}

/// Queue a message (files listed by their sha256, served from here when the bia pulls them)
/// and send it now when the link is up. Returns its id and whether the bia has it yet.
fn send(d: &Arc<Daemon>, text: &str, files: &[String], reply_to: &str) -> Result<(String, bool)> {
    let id = format!("g{}", random_hex(8));
    let mut offered = vec![];
    let mut map = HashMap::new();
    for f in files {
        let path = PathBuf::from(f);
        let data_len = fs::metadata(&path).map_err(|e| anyhow!("{f}: {e}"))?.len();
        let fid = sha256_file(&path).map_err(|e| anyhow!("{f}: {e}"))?;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
        offered.push(json!({"fid": fid, "name": name, "size": data_len}));
        // the bia pulls it after it acknowledged the message: remember where it is
        remember_sent(d, &fid, f);
        map.insert(fid, f.clone());
    }
    if text.trim().is_empty() && offered.is_empty() {
        bail!("nothing to send");
    }
    let mut action = json!({"t": "send", "id": id, "text": text, "files": offered});
    if !reply_to.is_empty() {
        action["reply_to"] = json!(reply_to);
    }
    let cid = format!("o{}", random_hex(6));
    d.outbox.lock().unwrap().push((cid.clone(), Out { action: action.clone(), files: map }));
    d.save_outbox();
    let Some(conn) = d.conn() else {
        return Ok((id, false));
    };
    action["cid"] = json!(cid);
    conn.json(&action)?;
    // the ack comes back on the link's reader, which drops it from the outbox
    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        if !d.outbox.lock().unwrap().iter().any(|(c, _)| *c == cid) {
            return Ok((id, true));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok((id, false))
}

fn sha256_file(p: &std::path::Path) -> std::io::Result<String> {
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
    Ok(ctx.finish().as_ref().iter().map(|x| format!("{x:02x}")).collect())
}

// ---- the link with the bia ----

fn set_err(d: &Daemon, e: &str) {
    *d.last_err.lock().unwrap() = e.to_string();
    d.log(e);
}

fn link_loop(d: Arc<Daemon>) {
    let mut backoff = 2u64;
    let mut renew_checked = Instant::now() - Duration::from_secs(86_400);
    while !d.stop.load(Ordering::Relaxed) {
        let mut p = d.profile();
        match p.state.as_str() {
            "pending" => {
                let wait = match link::poll_request(&mut p) {
                    Ok(w) => {
                        set_err(&d, "");
                        w
                    }
                    Err(e) => {
                        set_err(&d, &format!("checking the request: {e:#}"));
                        30
                    }
                };
                if p.state != "pending" {
                    let text = match p.state.as_str() {
                        "active" => format!("[bridge] {} approved this client as {}.{}", p.name, p.identity, if p.message.is_empty() { String::new() } else { format!(" Its owner's message: {}", p.message) }),
                        "denied" => format!("[bridge] {} denied the request.{}", p.name, if p.message.is_empty() { String::new() } else { format!(" Reason: {}", p.message) }),
                        other => format!("[bridge] the request at {} is {other}; connect again to ask anew.", p.name),
                    };
                    d.add(Entry { seq: 0, id: format!("request-{}", p.request), ts: now_secs(), kind: "note".into(), text, files: vec![], reply_to: None });
                }
                *d.p.lock().unwrap() = p;
                sleep_unless_stopped(&d, Duration::from_secs(wait.max(1)));
            }
            "active" => {
                if renew_checked.elapsed() > Duration::from_secs(6 * 3600) {
                    renew_checked = Instant::now();
                    if p.days_left().is_some_and(|n| n < 30) {
                        match link::renew(&p) {
                            Ok(()) => d.log("certificate renewed"),
                            Err(e) => set_err(&d, &format!("renewal: {e:#}")),
                        }
                    }
                }
                let started = Instant::now();
                let r = run_link(&d, &p);
                d.online.store(false, Ordering::Relaxed);
                *d.conn.lock().unwrap() = None;
                d.pulls.lock().unwrap().clear();
                let mut wait = backoff;
                match r {
                    Ok(Some(reason)) if reason == "duplicate_key" => {
                        set_err(&d, "another bia-bridge with this certificate is connected to the bia; this one waits");
                        wait = 30;
                    }
                    Ok(Some(reason)) if reason == "banned" => {
                        set_err(&d, "the bia banned this client; it tries again every 10 minutes");
                        wait = 600;
                    }
                    Ok(_) => {}
                    Err(e) => set_err(&d, &format!("link: {e:#}")),
                }
                if started.elapsed() > Duration::from_secs(60) {
                    backoff = 2;
                }
                let jitter = (u32::from_str_radix(&random_hex(2), 16).unwrap_or(0) % 1000) as u64;
                sleep_unless_stopped(&d, Duration::from_millis(wait * 1000 + jitter));
                backoff = (backoff * 2).min(60);
            }
            _ => sleep_unless_stopped(&d, Duration::from_secs(60)),
        }
    }
}

fn sleep_unless_stopped(d: &Daemon, t: Duration) {
    let end = Instant::now() + t;
    while Instant::now() < end && !d.stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// One link: hello, catch-up, then frames until it drops. Returns the bia's bye reason.
fn run_link(d: &Arc<Daemon>, p: &Profile) -> Result<Option<String>> {
    let mut ws = link::ws_connect(p)?;
    // the bia pings every 5 s when idle: 15 s of silence is a dead link
    let _ = ws.tls.set_read_timeout(Some(Duration::from_secs(15)));
    let conn = Arc::new(Conn { tls: ws.tls.clone(), w: Mutex::new(()) });
    let lf = d.lf.lock().unwrap().clone();
    conn.json(&json!({"t": "hello", "proto": 2, "name": p.identity, "ver": env!("CARGO_PKG_VERSION"), "instance": lf.instance, "rx": lf.rx}))?;
    let (op, b) = ws.recv()?;
    let hello: Value = if op == 1 { serde_json::from_slice(&b)? } else { bail!("no hello") };
    if hello["t"] == "bye" {
        return Ok(hello["reason"].as_str().map(str::to_string));
    }
    if hello["t"] != "hello" || hello["proto"] != 2 {
        bail!("the bia speaks an older protocol: it must update (to bia 1.9 or newer), or use an older bia-bridge");
    }
    *d.bia_version.lock().unwrap() = hello["ver"].as_str().unwrap_or("").to_string();
    if lf.rx.is_none() {
        // the first link starts from now
        d.lf.lock().unwrap().rx = Some(hello["rev"].as_u64().unwrap_or(0));
        d.save_link();
    }
    *d.conn.lock().unwrap() = Some(conn.clone());
    d.online.store(true, Ordering::Relaxed);
    set_err(d, "");
    d.log("web bus link up");
    // what was said while the link was down
    let queued: Vec<(String, Out)> = d.outbox.lock().unwrap().clone();
    for (cid, o) in queued {
        let mut a = o.action.clone();
        a["cid"] = json!(cid);
        conn.json(&a)?;
    }
    // a ping every 5 s (the bia closes a session silent for 15 s)
    let alive = Arc::new(AtomicBool::new(true));
    {
        let (c, alive) = (conn.clone(), alive.clone());
        std::thread::spawn(move || {
            while alive.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_secs(5));
                if !alive.load(Ordering::Relaxed) || c.send(1, br#"{"t":"ping"}"#).is_err() {
                    break;
                }
            }
        });
    }
    let mut bye = None;
    let mut unsaved = 0;
    let r = (|| -> Result<()> {
        loop {
            if d.stop.load(Ordering::Relaxed) {
                return Ok(());
            }
            let (op, b) = ws.recv()?;
            match op {
                1 => {
                    let Ok(v) = serde_json::from_slice::<Value>(&b) else { continue };
                    match v["t"].as_str().unwrap_or("") {
                        "m" => {
                            take_msg(d, &conn, p, &v["m"]);
                            let seq = v["seq"].as_u64().unwrap_or(0);
                            let mut lf = d.lf.lock().unwrap();
                            if Some(seq) > lf.rx {
                                lf.rx = Some(seq);
                            }
                            drop(lf);
                            unsaved += 1;
                            if unsaved >= 10 {
                                unsaved = 0;
                                d.save_link();
                            }
                            let _ = conn.json(&json!({"t": "ack", "seq": seq}));
                        }
                        "gone" => {
                            let seq = v["seq"].as_u64().unwrap_or(0);
                            d.lf.lock().unwrap().rx = Some(seq);
                            d.save_link();
                        }
                        "reset" => {
                            // too far behind: start again from now
                            d.lf.lock().unwrap().rx = Some(hello["rev"].as_u64().unwrap_or(0));
                            d.save_link();
                        }
                        "ack" => {
                            let cid = v["cid"].as_str().unwrap_or("").to_string();
                            if v["ok"] != true && cid.starts_with('o') {
                                d.add(Entry { seq: 0, id: format!("refused-{cid}"), ts: now_secs(), kind: "note".into(), text: format!("[bridge] the bia refused a message of yours: {}", v["error"].as_str().unwrap_or("")), files: vec![], reply_to: None });
                            }
                            let n = d.outbox.lock().unwrap().len();
                            d.outbox.lock().unwrap().retain(|(c, _)| *c != cid);
                            if d.outbox.lock().unwrap().len() != n {
                                d.save_outbox();
                            }
                            d.acks.lock().unwrap().insert(cid, v);
                        }
                        "fget" => serve_file(d, &conn, &v),
                        "frefuse" | "fcancel" => {
                            let fid = v["fid"].as_str().unwrap_or("").to_string();
                            if d.pulls.lock().unwrap().remove(&fid).is_some() {
                                settle(d, &fid, None);
                            }
                        }
                        "bye" => bye = v["reason"].as_str().map(str::to_string),
                        _ => {}
                    }
                }
                2 => on_chunk(d, &conn, &b),
                8 => return Ok(()),
                9 => {
                    let _ = conn.send(10, &b);
                }
                _ => {}
            }
        }
    })();
    alive.store(false, Ordering::Relaxed);
    d.save_link();
    ws.tls.shutdown();
    match r {
        Ok(()) => Ok(bye),
        Err(_) if bye.is_some() => Ok(bye),
        Err(e) => Err(e),
    }
}

/// A message state from the bia: bia's new or edited messages go to the inbox (their
/// files pulled first). Our own and the rest are already known here.
fn take_msg(d: &Arc<Daemon>, conn: &Arc<Conn>, p: &Profile, m: &Value) {
    let id = m["id"].as_str().unwrap_or("").to_string();
    if id.is_empty() || m["from"] != "bia" || m["service"] == true {
        return;
    }
    let text = m["text"].as_str().unwrap_or("").to_string();
    let old = d.texts.lock().unwrap().insert(id.clone(), text.clone());
    let _ = fs::write(p.path("msgs.json"), serde_json::to_vec(&*d.texts.lock().unwrap()).unwrap_or_default());
    let kind = match old {
        None => "msg",
        Some(o) if o != text => "edit",
        Some(_) => return,
    };
    let mut e = Entry {
        seq: 0,
        id: id.clone(),
        ts: m["ts"].as_u64().unwrap_or(0) / 1000,
        kind: kind.into(),
        text,
        files: vec![],
        reply_to: m["reply_to"].as_str().map(str::to_string),
    };
    if kind == "edit" {
        d.add(e);
        return;
    }
    let dir = p.dir().join("files");
    let mut fids = HashSet::new();
    for a in m["files"].as_array().into_iter().flatten() {
        let name = a["name"].as_str().unwrap_or("file").rsplit(['/', '\\']).next().unwrap_or("file").to_string();
        let dest = dir.join(format!("{id}-{name}"));
        e.files.push(dest.display().to_string());
        let fid = a["fid"].as_str().unwrap_or("").to_string();
        let size = a["size"].as_u64().unwrap_or(0);
        if dest.exists() || link::unhex32(&fid).is_none() {
            continue;
        }
        let part = dir.join(".part").join(&fid);
        let _ = fs::create_dir_all(part.parent().unwrap());
        let have = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        d.pulls.lock().unwrap().insert(fid.clone(), Pull { part, dest, size });
        fids.insert(fid.clone());
        if size == 0 {
            continue;
        }
        let _ = conn.json(&json!({"t": "fget", "fid": fid, "off": have}));
    }
    if fids.is_empty() {
        d.add(e);
        return;
    }
    d.waiting.lock().unwrap().push(Waiting { entry: e, fids: fids.clone() });
    for fid in fids {
        let empty = d.pulls.lock().unwrap().get(&fid).is_some_and(|p| p.size == 0);
        if empty {
            let part = d.pulls.lock().unwrap().get(&fid).map(|p| p.part.clone()).unwrap();
            let _ = fs::write(&part, b"");
            finish(d, conn, &fid);
        }
    }
}

fn on_chunk(d: &Arc<Daemon>, conn: &Arc<Conn>, b: &[u8]) {
    let Some((fid, off, data)) = link::parse_chunk(b) else { return };
    let Some((part, size)) = d.pulls.lock().unwrap().get(&fid).map(|p| (p.part.clone(), p.size)) else { return };
    let have = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    if off != have {
        if off > have {
            let _ = conn.json(&json!({"t": "fget", "fid": fid, "off": have}));
        }
        return;
    }
    let take = (data.len() as u64).min(size.saturating_sub(have)) as usize;
    if fs::OpenOptions::new().create(true).append(true).open(&part).and_then(|mut f| f.write_all(&data[..take])).is_err() {
        let _ = conn.json(&json!({"t": "frefuse", "fid": fid, "reason": "the receiver could not write it"}));
        d.pulls.lock().unwrap().remove(&fid);
        settle(d, &fid, None);
        return;
    }
    let have = have + take as u64;
    let _ = conn.json(&json!({"t": "fack", "fid": fid, "off": have}));
    if have >= size {
        finish(d, conn, &fid);
    }
}

/// A pull is complete: check the hash, keep it under its name.
fn finish(d: &Arc<Daemon>, conn: &Arc<Conn>, fid: &str) {
    let Some(pull) = d.pulls.lock().unwrap().remove(fid) else { return };
    let ok = sha256_file(&pull.part).map(|s| s == fid).unwrap_or(false);
    if ok && fs::rename(&pull.part, &pull.dest).is_ok() {
        let _ = conn.json(&json!({"t": "fdone", "fid": fid}));
        settle(d, fid, Some(&pull.dest));
    } else {
        let _ = fs::remove_file(&pull.part);
        let _ = conn.json(&json!({"t": "frefuse", "fid": fid, "reason": "it arrived damaged (checksum mismatch)"}));
        settle(d, fid, None);
    }
}

/// A file arrived (or not): messages that have all theirs go to the inbox.
fn settle(d: &Arc<Daemon>, fid: &str, got: Option<&std::path::Path>) {
    let ready: Vec<Entry> = {
        let mut w = d.waiting.lock().unwrap();
        for x in w.iter_mut() {
            if x.fids.remove(fid) && got.is_none() {
                // say which one did not come
                let missing = x.entry.files.iter().position(|f| f.contains(&format!("{}-", x.entry.id)) && !std::path::Path::new(f).exists());
                if let Some(i) = missing {
                    let f = x.entry.files.remove(i);
                    x.entry.text.push_str(&format!("\n[file {} did not arrive]", f.rsplit(['/', '\\']).next().unwrap_or("")));
                }
            }
        }
        let (done, rest): (Vec<Waiting>, Vec<Waiting>) = w.drain(..).partition(|x| x.fids.is_empty());
        *w = rest;
        done.into_iter().map(|x| x.entry).collect()
    };
    for e in ready {
        d.add(e);
    }
}

/// The bia pulls a file of ours (listed in a send).
fn serve_file(d: &Arc<Daemon>, conn: &Arc<Conn>, v: &Value) {
    let fid = v["fid"].as_str().unwrap_or("").to_string();
    let off = v["off"].as_u64().unwrap_or(0);
    let path = d
        .outbox
        .lock()
        .unwrap()
        .iter()
        .find_map(|(_, o)| o.files.get(&fid).cloned())
        .or_else(|| sent_file(d, &fid));
    let (Some(path), Some(raw)) = (path, link::unhex32(&fid)) else {
        let _ = conn.json(&json!({"t": "frefuse", "fid": fid, "reason": "no such file here"}));
        return;
    };
    let conn = conn.clone();
    std::thread::spawn(move || {
        let Ok(mut f) = fs::File::open(&path) else {
            let _ = conn.json(&json!({"t": "frefuse", "fid": fid, "reason": "the file is gone here"}));
            return;
        };
        let _ = f.seek(SeekFrom::Start(off));
        let mut pos = off;
        let mut buf = vec![0u8; CHUNK];
        loop {
            let n = match f.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            if conn.send(2, &link::chunk(&raw, pos, &buf[..n])).is_err() {
                break;
            }
            pos += n as u64;
        }
    });
}

/// Files of ours the bia may pull (`sent.json`: fid → path), also after the send was
/// acknowledged.
fn sent_file(d: &Daemon, fid: &str) -> Option<String> {
    let m: HashMap<String, String> = fs::read(d.profile().path("sent.json")).ok().and_then(|b| serde_json::from_slice(&b).ok())?;
    m.get(fid).cloned()
}

fn remember_sent(d: &Daemon, fid: &str, path: &str) {
    let p = d.profile().path("sent.json");
    let mut m: HashMap<String, String> = fs::read(&p).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    if m.insert(fid.to_string(), path.to_string()).is_none() {
        let _ = fs::write(&p, serde_json::to_vec(&m).unwrap_or_default());
    }
}
