//! Unix socket the native host connects to. Passkey ceremonies from the extension stay here.
use bia_mcp::Ctx;
use serde_json::{json, Value};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::frame;
use crate::host;

struct Shared {
    writer: Mutex<Option<UnixStream>>,
    /// Generation of the live connection; 0 when none. A reader only clears its own.
    connected: Mutex<u64>,
    generation: AtomicU64,
    log: Mutex<Option<Ctx>>,
    /// Inode of the socket file this process listens on (0: not bound).
    inode: AtomicU64,
}

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
        *self.shared.log.lock().unwrap() = Some(ctx);
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
                let mut c = shared.connected.lock().unwrap();
                if *c == gen {
                    *c = 0;
                    *shared.writer.lock().unwrap() = None;
                }
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
        if method != "passkey_create" && method != "passkey_get" {
            if !id.is_null() {
                let _ = write_msg(&shared, &json!({"id": id, "error": format!("unknown method {method}")}));
            }
            continue;
        }
        let params = msg.get("params").cloned().unwrap_or(json!({}));
        let rp = params.get("rpId").and_then(|v| v.as_str()).unwrap_or("");
        let origin = params.get("origin").and_then(|v| v.as_str()).unwrap_or("");
        let reply = match passkey(method, &params) {
            Ok(result) => {
                log(&shared, format!("passkey {method} origin={origin} rp={rp}"));
                json!({"id": id, "result": result})
            }
            Err(e) => {
                log(&shared, format!("passkey {method} origin={origin} rp={rp}: {e}"));
                json!({"id": id, "error": e})
            }
        };
        let _ = write_msg(&shared, &reply);
    }
}

fn log(shared: &Shared, line: String) {
    if let Some(ctx) = shared.log.lock().unwrap().clone() {
        ctx.log(line);
    }
}

fn strings(params: &Value, key: &str) -> Vec<String> {
    params
        .get(key)
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default()
}

fn passkey(method: &str, params: &Value) -> Result<Value, String> {
    let rp = params.get("rpId").and_then(|v| v.as_str()).unwrap_or("");
    let origin = params.get("origin").and_then(|v| v.as_str()).unwrap_or("");
    let challenge = params.get("challenge").and_then(|v| v.as_str()).unwrap_or("");
    let text = if method == "passkey_create" {
        let user = params.get("userName").and_then(|v| v.as_str()).unwrap_or("bia");
        let uid = params.get("userId").and_then(|v| v.as_str());
        bia_mcp_secrets::store::passkey_ceremony_create(
            rp,
            origin,
            challenge,
            user,
            uid,
            &strings(params, "excludeCredentials"),
        )?
    } else {
        bia_mcp_secrets::store::passkey_ceremony_get(
            rp,
            origin,
            challenge,
            &strings(params, "allowCredentials"),
        )?
    };
    serde_json::from_str(&text).map_err(|e| e.to_string())
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
}
