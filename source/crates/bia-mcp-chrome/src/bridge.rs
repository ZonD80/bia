//! Unix socket the native host connects to. Passkey ceremonies from the extension stay here.
use bia_mcp::Ctx;
use serde_json::{json, Value};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
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
}

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
        });
        let sock = host::sock_path();
        if let Some(parent) = sock.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::remove_file(&sock);
        let listener = UnixListener::bind(&sock).unwrap_or_else(|e| panic!("bind {}: {e}", sock.display()));
        let _ = std::fs::set_permissions(&sock, std::fs::Permissions::from_mode(0o660));
        if let Some(user) = crate::install::remember_user(dir) {
            let _ = std::process::Command::new("chown").arg(&user).arg(&sock).status();
        }
        let accept = shared.clone();
        std::thread::Builder::new()
            .name("chrome-accept".into())
            .spawn(move || {
                for conn in listener.incoming() {
                    let Ok(stream) = conn else { continue };
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
            .ok();
        Bridge { shared }
    }

    pub fn set_log(&self, ctx: Ctx) {
        *self.shared.log.lock().unwrap() = Some(ctx);
    }

    pub fn connected(&self) -> bool {
        *self.shared.connected.lock().unwrap() != 0
    }
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
        if method != "passkey_create" && method != "passkey_get" {
            continue;
        }
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
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
