//! The background daemon (one per profile, started by any command): finishes a pending
//! certificate request, keeps the chat link with the bia open, stores what bia says in
//! the inbox (attachments downloaded) and catches up after a drop, renews the
//! certificate. Commands and MCP sessions reach it on 127.0.0.1 with a token from
//! `daemon.json` (readable by this user only).
use crate::link::{self, Ws};
use crate::profile::{now_secs, random_hex, Profile};
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// What bia said, as kept in `inbox.jsonl`.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Entry {
    pub seq: u64,
    pub chat: String,
    pub id: String,
    /// unix seconds
    pub ts: u64,
    /// `msg`, `edit` or `note` (the bridge itself: approval, denial)
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

pub struct Daemon {
    p: Mutex<Profile>,
    inbox: Mutex<Inbox>,
    cv: Condvar,
    online: AtomicBool,
    stop: AtomicBool,
    last_err: Mutex<String>,
    log: Mutex<Option<fs::File>>,
    /// The bia's version (`/v1/bridge`), learned when the link comes up.
    bia_version: Mutex<String>,
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
        let key = format!("{}/{}/{}/{}", e.chat, e.id, e.kind, if e.kind == "edit" { e.text.as_str() } else { "" });
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

    fn take(&self, consumer: &str, chat: Option<&str>, peek: bool, wait: Option<Duration>) -> Vec<Entry> {
        let end = wait.map(|w| Instant::now() + w);
        let mut ib = self.inbox.lock().unwrap();
        loop {
            let cur = ib.cursors.get(consumer).copied().unwrap_or(0);
            let out: Vec<Entry> =
                ib.entries.iter().filter(|e| e.seq > cur && chat.is_none_or_eq(&e.chat)).cloned().collect();
            if !out.is_empty() || end.map_or(true, |t| Instant::now() >= t) || self.stop.load(Ordering::Relaxed) {
                if !peek {
                    // reading one chat moves the cursor past what was shown only
                    let upto = if chat.is_some() { out.last().map(|e| e.seq).unwrap_or(cur) } else { ib.entries.last().map(|e| e.seq).unwrap_or(0) };
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
}

trait ChatFilter {
    fn is_none_or_eq(&self, c: &str) -> bool;
}
impl ChatFilter for Option<&str> {
    fn is_none_or_eq(&self, c: &str) -> bool {
        self.map_or(true, |x| x == c)
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
            ib.seen.insert(format!("{}/{}/{}/{}", e.chat, e.id, e.kind, if e.kind == "edit" { e.text.as_str() } else { "" }));
            ib.entries.push(e);
        }
    }
    ib.cursors = fs::read(p.path("cursors.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
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
    });
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
        let _ = fs::remove_file(d.profile().path("daemon.json"));
        std::thread::sleep(Duration::from_millis(100));
        std::process::exit(0);
    }
}

fn v_is_stop(line: &str) -> bool {
    serde_json::from_str::<Value>(line).map(|v| v["op"] == "stop").unwrap_or(false)
}

fn entries_json(v: &[Entry]) -> Value {
    json!(v)
}

fn handle(d: &Arc<Daemon>, v: &Value) -> Result<Value> {
    let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
    let consumer = Some(s("consumer")).filter(|c| !c.is_empty()).unwrap_or_else(|| "default".into());
    let chat = Some(s("chat")).filter(|c| !c.is_empty());
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
                "cert_days_left": p.days_left(),
                "bia_version": d.bia_version.lock().unwrap().clone(),
                "bridge_version": env!("CARGO_PKG_VERSION"),
            }))
        }
        "read" => Ok(json!({"ok": true, "messages": entries_json(&d.take(&consumer, chat.as_deref(), v["peek"] == true, None))})),
        "wait" => {
            let t = v["timeout"].as_u64().unwrap_or(300).min(3600);
            Ok(json!({"ok": true, "messages": entries_json(&d.take(&consumer, chat.as_deref(), false, Some(Duration::from_secs(t))))}))
        }
        "send" => {
            let p = d.profile();
            let files: Vec<String> = v["files"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_string)).collect();
            let id = link::send(&p, chat.as_deref().unwrap_or("main"), &s("text"), &files, Some(&s("reply_to")))?;
            Ok(json!({"ok": true, "id": id}))
        }
        "chats" => {
            let r = link::call(&d.profile(), "GET", "/v1/chats", None)?;
            Ok(json!({"ok": true, "chats": r["chats"]}))
        }
        "history" => {
            let limit = v["limit"].as_u64().unwrap_or(20).clamp(1, 200);
            let c = chat.unwrap_or_else(|| "main".into());
            let r = link::call(&d.profile(), "GET", &format!("/v1/chats/{}/messages?limit={limit}", link::urlenc(&c)), None)?;
            Ok(json!({"ok": true, "chat": c, "messages": r["messages"]}))
        }
        other => Err(anyhow!("unknown op {other}")),
    }
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
                    d.add(Entry { seq: 0, chat: String::new(), id: format!("request-{}", p.request), ts: now_secs(), kind: "note".into(), text, files: vec![], reply_to: None });
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
                if let Err(e) = run_ws(&d, &p) {
                    set_err(&d, &format!("chat link: {e:#}"));
                }
                d.online.store(false, Ordering::Relaxed);
                if started.elapsed() > Duration::from_secs(60) {
                    backoff = 2;
                }
                let jitter = (u32::from_str_radix(&random_hex(2), 16).unwrap_or(0) % 1000) as u64;
                sleep_unless_stopped(&d, Duration::from_millis(backoff * 1000 + jitter));
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

fn run_ws(d: &Arc<Daemon>, p: &Profile) -> Result<()> {
    let mut ws = link::ws_connect(p)?;
    let _ = ws.tls.set_read_timeout(Some(Duration::from_secs(90)));
    // the server drops a client silent for 2 minutes
    let alive = Arc::new(AtomicBool::new(true));
    {
        let (tls, alive) = (ws.tls.clone(), alive.clone());
        std::thread::spawn(move || {
            while alive.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_secs(30));
                if !alive.load(Ordering::Relaxed) || Ws::send_text(&tls, r#"{"t":"ping"}"#).is_err() {
                    break;
                }
            }
        });
    }
    let r = (|| -> Result<()> {
        let (op, b) = ws.recv()?;
        let hello: Value = if op == 1 { serde_json::from_slice(&b)? } else { bail!("no hello") };
        if hello["t"] != "hello" {
            bail!("no hello");
        }
        catch_up(d, p, &hello);
        if let Ok(v) = link::bridge_list(p) {
            *d.bia_version.lock().unwrap() = v["version"].as_str().unwrap_or("").to_string();
        }
        d.online.store(true, Ordering::Relaxed);
        set_err(d, "");
        d.log("chat link up");
        loop {
            if d.stop.load(Ordering::Relaxed) {
                return Ok(());
            }
            let (op, b) = ws.recv()?;
            match op {
                1 => {
                    let Ok(v) = serde_json::from_slice::<Value>(&b) else { continue };
                    let chat = v["chat"].as_str().unwrap_or("").to_string();
                    match v["t"].as_str().unwrap_or("") {
                        "msg" => take_msg(d, p, &chat, &v["m"], "msg"),
                        "upd" if v["m"]["from"] == "bia" && v["m"]["edited"].as_u64().unwrap_or(0) > 0 => take_msg(d, p, &chat, &v["m"], "edit"),
                        _ => {}
                    }
                }
                8 => bail!("closed by the bia"),
                9 => {} // the server does not ping
                _ => {}
            }
        }
    })();
    alive.store(false, Ordering::Relaxed);
    ws.tls.shutdown();
    r
}

/// After (re)connecting: messages bia wrote while the link was down.
fn catch_up(d: &Arc<Daemon>, p: &Profile, hello: &Value) {
    let path = p.path("seen.json");
    let first = !path.exists();
    let mut seen: HashMap<String, String> = fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    for c in hello["chats"].as_array().into_iter().flatten() {
        let chat = c["id"].as_str().unwrap_or("").to_string();
        if chat.is_empty() {
            continue;
        }
        let q = match seen.get(&chat) {
            Some(last) => format!("/v1/chats/{}/messages?after={}&limit=200", link::urlenc(&chat), link::urlenc(last)),
            // a new client starts from now; a chat made while the link was down is read whole
            None if first => {
                if let Some(id) = c["last"]["id"].as_str() {
                    seen.insert(chat.clone(), id.to_string());
                }
                continue;
            }
            None => format!("/v1/chats/{}/messages?limit=200", link::urlenc(&chat)),
        };
        if let Ok(v) = link::call(p, "GET", &q, None) {
            for m in v["messages"].as_array().into_iter().flatten() {
                take_msg(d, p, &chat, m, "msg");
                if let Some(id) = m["id"].as_str() {
                    seen.insert(chat.clone(), id.to_string());
                }
            }
        }
    }
    let _ = fs::write(&path, serde_json::to_vec(&seen).unwrap_or_default());
}

fn take_msg(d: &Arc<Daemon>, p: &Profile, chat: &str, m: &Value, kind: &str) {
    let id = m["id"].as_str().unwrap_or("").to_string();
    if id.is_empty() {
        return;
    }
    if kind == "msg" {
        let path = p.path("seen.json");
        let mut seen: HashMap<String, String> = fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        seen.insert(chat.to_string(), id.clone());
        let _ = fs::write(&path, serde_json::to_vec(&seen).unwrap_or_default());
    }
    if m["from"] != "bia" || m["service"] == true {
        return;
    }
    let mut files = vec![];
    for (i, a) in m["files"].as_array().into_iter().flatten().enumerate() {
        let name = a["name"].as_str().unwrap_or("file").rsplit(['/', '\\']).next().unwrap_or("file").to_string();
        let dir = p.dir().join("files").join(chat);
        let dest = dir.join(format!("{id}-{name}"));
        if !dest.exists() {
            match link::download(p, chat, &id, i) {
                Ok(b) => {
                    let _ = fs::create_dir_all(&dir);
                    let _ = fs::write(&dest, b);
                }
                Err(e) => {
                    set_err(d, &format!("attachment {name}: {e:#}"));
                    continue;
                }
            }
        }
        files.push(dest.display().to_string());
    }
    d.add(Entry {
        seq: 0,
        chat: chat.to_string(),
        id,
        ts: m["ts"].as_u64().unwrap_or(0) / 1000,
        kind: kind.into(),
        text: m["text"].as_str().unwrap_or("").to_string(),
        files,
        reply_to: m["reply_to"].as_str().map(str::to_string),
    });
}
