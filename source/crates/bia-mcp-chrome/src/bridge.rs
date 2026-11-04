//! Unix socket the native host connects to. Passkey requests from the extension wait here
//! until the model answers them (see passkey.rs).
use bia_mcp::Ctx;
use serde_json::{json, Value};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::frame;
use crate::host;
use crate::passkey::{self, Table};

struct Shared {
    writer: Mutex<Option<UnixStream>>,
    /// Generation of the live connection; 0 when none. A reader only clears its own.
    connected: Mutex<u64>,
    generation: AtomicU64,
    log: Mutex<Option<Ctx>>,
    /// Inode of the socket file this process listens on (0: not bound).
    inode: AtomicU64,
    /// Passkey requests waiting for chrome_passkey_answer.
    table: Mutex<Table>,
    /// Puts a line in front of the model; None: nobody can approve anything.
    announce: Mutex<Option<Announce>>,
}

type Announce = Arc<dyn Fn(&str) + Send + Sync>;

/// A native host that stops reading must not block the bridge (writes hold the writer lock).
const WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// How often the bridge checks that its socket file is still its own and reachable.
const WATCH_EVERY: std::time::Duration = std::time::Duration::from_secs(15);

#[derive(Clone)]
pub struct Bridge {
    shared: Arc<Shared>,
}

impl Bridge {
    pub fn start(dir: &Path) -> Self {
        let shared = Arc::new(Shared {
            writer: Mutex::new(None),
            connected: Mutex::new(0),
            generation: AtomicU64::new(0),
            log: Mutex::new(None),
            inode: AtomicU64::new(0),
            table: Mutex::new(Table::new()),
            announce: Mutex::new(None),
        });
        let sock = host::sock_path();
        if let Err(e) = listen(&shared, dir, &sock) {
            panic!("bind {}: {e}", sock.display());
        }
        // The socket file can vanish or be taken over (another MCP instance that rebinds and
        // exits leaves a dead file: "Connection refused"), and the directory can lose its
        // mode; the native host runs as the Chrome user and must reach it.
        let watch = shared.clone();
        let dir = dir.to_path_buf();
        std::thread::Builder::new()
            .name("chrome-sock-watch".into())
            .spawn(move || loop {
                std::thread::sleep(WATCH_EVERY);
                expire(&watch);
                if let Some(problem) = sock_problem(&watch, &sock) {
                    log(&watch, format!("chrome bridge: {problem}; listening again"));
                    open_parent(&sock);
                    if let Err(e) = listen(&watch, &dir, &sock) {
                        log(&watch, format!("chrome bridge: bind {}: {e}", sock.display()));
                    }
                }
            })
            .ok();
        Bridge { shared }
    }

    pub fn set_log(&self, ctx: Ctx) {
        *self.shared.log.lock().unwrap() = Some(ctx.clone());
        *self.shared.announce.lock().unwrap() = Some(Arc::new(move |line: &str| {
            ctx.inbound("system", "chrome", "", line);
        }));
    }

    /// Tool `passkey_answer`: the only way a waiting passkey request goes through.
    pub fn answer(&self, args: &Value) -> Result<String, String> {
        let request = args.get("request").and_then(|v| v.as_str()).unwrap_or("").trim();
        let answer = args.get("answer").and_then(|v| v.as_str()).unwrap_or("").trim();
        let pick = args.get("passkey").and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty());
        let approve = match answer {
            "approve" => true,
            "deny" => false,
            _ => return Err("answer must be approve or deny".into()),
        };
        let live = *self.shared.connected.lock().unwrap();
        // Out of the table first: one request can only ever be answered once.
        let open = self.shared.table.lock().unwrap().take(request, live, approve, pick)?;
        let (rp, origin) = (open.rp(), open.origin());
        if !approve {
            log(&self.shared, format!("passkey denied origin={origin} rp={rp}"));
            let _ = write_msg(&self.shared, &json!({"id": open.wire_id, "error": passkey::DENIED}));
            return Ok(format!("Denied. {origin} was told no; nothing was signed or created."));
        }
        match passkey::ceremony(&open, pick) {
            Ok(result) => {
                log(&self.shared, format!("passkey approved {} origin={origin} rp={rp}", open.method));
                write_msg(&self.shared, &json!({"id": open.wire_id, "result": result}))?;
                Ok(format!("Approved. The passkey answer went to {origin}."))
            }
            Err(e) => {
                log(&self.shared, format!("passkey {} origin={origin} rp={rp}: {e}", open.method));
                let _ = write_msg(&self.shared, &json!({"id": open.wire_id, "error": e}));
                Err(e)
            }
        }
    }

    /// Requests still waiting, for chrome_status (a notice can be lost to a restart or a cut).
    pub fn waiting(&self) -> Value {
        self.shared.table.lock().unwrap().list()
    }

    pub fn connected(&self) -> bool {
        *self.shared.connected.lock().unwrap() != 0
    }

    /// Why the add-on is not connected, for chrome_status (None when it is).
    pub fn problem(&self, dir: &Path) -> Option<String> {
        if self.connected() {
            return None;
        }
        let sock = host::sock_path();
        if let Some(p) = sock_problem(&self.shared, &sock) {
            return Some(p);
        }
        if let Some(user) = crate::install::remember_user(dir) {
            let owner = std::fs::metadata(&sock).map(|m| m.uid()).ok();
            let uid = uid_of(&user);
            if uid.is_some() && owner != uid {
                return Some(format!("{} is not owned by {user}, the user Chrome runs as", sock.display()));
            }
        }
        Some(if self.shared.generation.load(Ordering::Relaxed) == 0 {
            "the native host never connected (Chrome not running, add-on not loaded, or its native host manifest missing)".into()
        } else {
            "the native host disconnected (Chrome or the add-on stopped)".into()
        })
    }
}

/// Bind the socket (replacing a stale file) and accept native hosts on it.
fn listen(shared: &Arc<Shared>, dir: &Path, sock: &Path) -> std::io::Result<()> {
    open_parent(sock);
    let _ = std::fs::remove_file(sock);
    let listener = UnixListener::bind(sock)?;
    let _ = std::fs::set_permissions(sock, std::fs::Permissions::from_mode(0o660));
    if let Some(user) = crate::install::remember_user(dir) {
        let _ = std::process::Command::new("chown").arg(&user).arg(sock).status();
    }
    shared.inode.store(std::fs::metadata(sock).map(|m| m.ino()).unwrap_or(0), Ordering::Relaxed);
    let accept = shared.clone();
    let mine = shared.inode.load(Ordering::Relaxed);
    std::thread::Builder::new()
        .name("chrome-accept".into())
        .spawn(move || {
            for conn in listener.incoming() {
                // a newer listener took over: let this one go
                if accept.inode.load(Ordering::Relaxed) != mine {
                    break;
                }
                let Ok(stream) = conn else { continue };
                let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
                let Ok(read) = stream.try_clone() else { continue };
                let gen = accept.generation.fetch_add(1, Ordering::Relaxed) + 1;
                // A newer native host replaces the old one; stop the old reader too.
                // Lock order connected -> writer, same as reader().
                let old = {
                    let mut c = accept.connected.lock().unwrap();
                    let old = accept.writer.lock().unwrap().replace(stream);
                    *c = gen;
                    old
                };
                if let Some(old) = old {
                    let _ = old.shutdown(std::net::Shutdown::Both);
                }
                let shared = accept.clone();
                std::thread::spawn(move || reader(shared, read, gen));
            }
        })
        .map(|_| ())
}

/// The socket's directory must be traversable by the Chrome user (the native host runs
/// as that user); `/run/bia` at 0700 made every connection fail with EACCES.
fn open_parent(sock: &Path) {
    let Some(parent) = sock.parent() else { return };
    let _ = std::fs::create_dir_all(parent);
    if let Ok(m) = std::fs::metadata(parent) {
        if m.permissions().mode() & 0o001 == 0 && parent.starts_with("/run") {
            let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(m.permissions().mode() | 0o055));
        }
    }
}

/// What keeps a native host from reaching this bridge's socket, if anything.
fn sock_problem(shared: &Shared, sock: &PathBuf) -> Option<String> {
    let Some(parent) = sock.parent() else { return None };
    if let Ok(m) = std::fs::metadata(parent) {
        if m.permissions().mode() & 0o001 == 0 {
            return Some(format!("{} is mode {:o}; the Chrome user cannot reach the socket", parent.display(), m.permissions().mode() & 0o7777));
        }
    }
    match std::fs::metadata(sock) {
        Err(_) => Some(format!("{} is missing", sock.display())),
        Ok(m) if m.ino() != shared.inode.load(Ordering::Relaxed) => {
            Some(format!("{} belongs to another process now", sock.display()))
        }
        Ok(_) => None,
    }
}

fn uid_of(user: &str) -> Option<u32> {
    let out = std::process::Command::new("id").args(["-u", user]).output().ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

fn reader(shared: Arc<Shared>, mut stream: UnixStream, gen: u64) {
    loop {
        let msg = match frame::read_frame(&mut stream) {
            Ok(v) => v,
            Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
                log(&shared, format!("chrome bridge: dropped frame: {e}"));
                continue;
            }
            Err(_) => {
                {
                    let mut c = shared.connected.lock().unwrap();
                    if *c == gen {
                        *c = 0;
                        *shared.writer.lock().unwrap() = None;
                    }
                }
                // Whoever asked is gone: nothing may be approved on its behalf any more.
                shared.table.lock().unwrap().drop_generation(gen);
                break;
            }
        };
        let Some(method) = msg.get("method").and_then(|m| m.as_str()) else {
            continue;
        };
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        // every request gets an answer, so a probe never looks like a hung bridge
        if method == "ping" {
            let _ = write_msg(&shared, &json!({"id": id, "result": "pong"}));
            continue;
        }
        let params = msg.get("params").cloned().unwrap_or(json!({}));
        if method == "passkey_cancel" {
            // the page gave up, went away or timed out
            let wire = params.get("id").cloned().unwrap_or(Value::Null);
            shared.table.lock().unwrap().cancel(&wire, gen);
            continue;
        }
        if method != "passkey_create" && method != "passkey_get" {
            if !id.is_null() {
                let _ = write_msg(&shared, &json!({"id": id, "error": format!("unknown method {method}")}));
            }
            continue;
        }
        if let Err(e) = ask(&shared, method, params, id.clone(), gen) {
            let _ = write_msg(&shared, &json!({"id": id, "error": e}));
        }
    }
}

/// Queue a passkey request and tell the model. Err: refused at once, nothing to approve.
fn ask(shared: &Shared, method: &str, params: Value, wire_id: Value, gen: u64) -> Result<(), String> {
    let names = passkey::check(method, &params).inspect_err(|e| {
        log(shared, format!("passkey {method} refused: {e}"));
    })?;
    let announce = shared.announce.lock().unwrap().clone().ok_or(passkey::NOBODY)?;
    let line = shared.table.lock().unwrap().open(method, params, wire_id, gen, names).inspect_err(|e| {
        log(shared, format!("passkey {method} refused: {e}"));
    })?;
    announce(&line);
    Ok(())
}

/// Requests nobody answered in time are refused, so a page is never left holding one for good.
fn expire(shared: &Shared) {
    let gone = shared.table.lock().unwrap().expired();
    for open in gone {
        log(shared, format!("passkey {} origin={} expired unanswered", open.method, open.origin()));
        let _ = write_msg(shared, &json!({"id": open.wire_id, "error": passkey::EXPIRED}));
    }
}

fn log(shared: &Shared, line: String) {
    if let Some(ctx) = shared.log.lock().unwrap().clone() {
        ctx.log(line);
    }
}

fn write_msg(shared: &Shared, msg: &Value) -> Result<(), String> {
    let mut guard = shared.writer.lock().unwrap();
    let w = guard.as_mut().ok_or("extension disconnected")?;
    frame::write_frame(w, msg).map_err(|e| format!("extension write: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_probes_and_names_a_lost_socket() {
        let dir = std::env::temp_dir().join(format!("bia-bridge-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("chrome.sock");
        std::env::set_var("BIA_CHROME_SOCK", &sock);
        let b = Bridge::start(&dir);
        assert!(b.problem(&dir).unwrap().contains("never connected"), "{:?}", b.problem(&dir));

        let mut c = UnixStream::connect(&sock).unwrap();
        c.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
        frame::write_frame(&mut c, &json!({"id": "p1", "method": "ping"})).unwrap();
        assert_eq!(frame::read_frame(&mut c).unwrap(), json!({"id": "p1", "result": "pong"}));
        frame::write_frame(&mut c, &json!({"id": "p2", "method": "nope"})).unwrap();
        assert_eq!(frame::read_frame(&mut c).unwrap()["error"], json!("unknown method nope"));
        assert!(b.connected() && b.problem(&dir).is_none());
        drop(c);

        // another instance took the path over and went away
        std::thread::sleep(std::time::Duration::from_millis(200));
        std::fs::remove_file(&sock).unwrap();
        std::os::unix::net::UnixListener::bind(&sock).unwrap();
        assert!(sock_problem(&b.shared, &sock).unwrap().contains("another process"));
        listen(&b.shared, &dir, &sock).unwrap();
        assert_eq!(sock_problem(&b.shared, &sock), None);
        let mut c = UnixStream::connect(&sock).unwrap();
        c.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
        frame::write_frame(&mut c, &json!({"id": "p3", "method": "ping"})).unwrap();
        assert_eq!(frame::read_frame(&mut c).unwrap()["result"], json!("pong"));

        // a directory others cannot traverse is named (outside /run it is not changed)
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(sock_problem(&b.shared, &sock).unwrap().contains("cannot reach"));
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_passkey_request_waits_for_the_answer() {
        let dir = std::env::temp_dir().join(format!("bia-bridge-pk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("BIA_FILES", &dir);
        let shared = Arc::new(Shared {
            writer: Mutex::new(None),
            connected: Mutex::new(0),
            generation: AtomicU64::new(0),
            log: Mutex::new(None),
            inode: AtomicU64::new(0),
            table: Mutex::new(Table::new()),
            announce: Mutex::new(None),
        });
        let sock = dir.join("pk.sock");
        listen(&shared, &dir, &sock).unwrap();
        let b = Bridge { shared: shared.clone() };
        let mut c = UnixStream::connect(&sock).unwrap();
        c.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
        let create = |id: &str, challenge: &str| {
            json!({"id": id, "method": "passkey_create", "params": {
                "rpId": "example.com", "origin": "https://example.com", "challenge": challenge, "userName": "ann", "known": true
            }})
        };

        // nobody to ask: refused, never signed
        frame::write_frame(&mut c, &create("e1", "AAEC")).unwrap();
        assert_eq!(frame::read_frame(&mut c).unwrap()["error"], json!(passkey::NOBODY));

        let lines: Arc<Mutex<Vec<String>>> = Arc::default();
        let sink = lines.clone();
        *shared.announce.lock().unwrap() = Some(Arc::new(move |l: &str| sink.lock().unwrap().push(l.to_string())));
        let wait_line = |n: usize| -> String {
            for _ in 0..100 {
                if let Some(l) = lines.lock().unwrap().get(n) {
                    return l.clone();
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            panic!("no notice {n}");
        };
        let request_of = |line: &str| line.split(['[', ']']).nth(1).unwrap().split(' ').next_back().unwrap().to_string();

        // denied: the page hears no, and nothing reached the store
        frame::write_frame(&mut c, &create("e2", "AAEC")).unwrap();
        let line = wait_line(0);
        assert!(line.contains("https://example.com"), "{line}");
        let r = request_of(&line);
        assert_eq!(b.waiting().as_array().unwrap().len(), 1);
        b.answer(&json!({"request": r, "answer": "deny"})).unwrap();
        let reply = frame::read_frame(&mut c).unwrap();
        assert_eq!(reply["id"], json!("e2"));
        assert_eq!(reply["error"], json!(passkey::DENIED));
        assert!(bia_mcp_secrets::store::passkey_ceremony_names("example.com", "https://example.com", "AAEC", &[]).is_err());
        assert!(b.answer(&json!({"request": r, "answer": "approve"})).unwrap_err().contains("no longer open"), "answered twice");

        // approved: only now does the passkey exist
        frame::write_frame(&mut c, &create("e3", "AAEC")).unwrap();
        let r = request_of(&wait_line(1));
        assert!(b.answer(&json!({"request": r, "answer": "maybe"})).is_err());
        b.answer(&json!({"request": r, "answer": "approve"})).unwrap();
        let reply = frame::read_frame(&mut c).unwrap();
        assert_eq!(reply["id"], json!("e3"));
        assert!(reply["result"]["id"].is_string() || reply["result"]["credential_id"].is_string(), "{reply}");
        assert_eq!(bia_mcp_secrets::store::passkey_ceremony_names("example.com", "https://example.com", "AAEC", &[]).unwrap().len(), 1);

        // the page gave up: the old id approves nothing
        frame::write_frame(&mut c, &json!({"id": "e4", "method": "passkey_get", "params": {
            "rpId": "example.com", "origin": "https://example.com", "challenge": "AAEC", "known": true
        }})).unwrap();
        let r = request_of(&wait_line(2));
        frame::write_frame(&mut c, &json!({"method": "passkey_cancel", "params": {"id": "e4"}})).unwrap();
        frame::write_frame(&mut c, &json!({"id": "p", "method": "ping"})).unwrap();
        assert_eq!(frame::read_frame(&mut c).unwrap()["id"], json!("p"));
        assert!(b.answer(&json!({"request": r, "answer": "approve"})).is_err());

        // the extension went away: its request cannot be approved either
        frame::write_frame(&mut c, &json!({"id": "e5", "method": "passkey_get", "params": {
            "rpId": "example.com", "origin": "https://example.com", "challenge": "AAEC", "known": true
        }})).unwrap();
        let r = request_of(&wait_line(3));
        drop(c);
        for _ in 0..100 {
            if b.waiting().as_array().unwrap().is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(b.answer(&json!({"request": r, "answer": "approve"})).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
