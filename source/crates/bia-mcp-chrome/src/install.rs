//! Force-install the Chrome-packed CRX. webAuthenticationProxy exists only then.
//! The update server keeps serving that file for the life of this process.
use serde_json::{json, Value};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::crx::{self, Packed};

pub const PORT: u16 = 4773;
const HOST_NAME: &str = "com.bia.chrome";

#[derive(Clone)]
pub struct Updates {
    inner: Arc<Mutex<Option<Packed>>>,
}

impl Updates {
    pub fn new() -> Self {
        Updates { inner: Arc::new(Mutex::new(None)) }
    }
    pub fn set(&self, packed: Packed) {
        *self.inner.lock().unwrap() = Some(packed);
    }
    fn get(&self) -> Option<Packed> {
        self.inner.lock().unwrap().clone()
    }
}

impl Default for Updates {
    fn default() -> Self {
        Self::new()
    }
}

pub fn serve(updates: Updates) {
    let listener = match TcpListener::bind(("127.0.0.1", PORT)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("chrome update server: {e}");
            return;
        }
    };
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut stream) = conn else { continue };
            let updates = updates.clone();
            let _ = std::thread::spawn(move || {
                let _ = handle(&mut stream, &updates);
            });
        }
    });
}

fn handle(stream: &mut TcpStream, updates: &Updates) {
    let mut buf = [0u8; 1024];
    let n = stream.read(&mut buf).unwrap_or(0);
    let req = String::from_utf8_lossy(&buf[..n]);
    let path = req.split_whitespace().nth(1).unwrap_or("/");
    let packed = updates.get();
    let (status, ctype, body) = if path.starts_with("/update.xml") {
        match &packed {
            Some(p) => {
                let xml = format!(
                    "<?xml version='1.0' encoding='UTF-8'?>\n<gupdate xmlns='http://www.google.com/update2/response' protocol='2.0'>\n  <app appid='{id}'>\n    <updatecheck codebase='http://127.0.0.1:{PORT}/bia-chrome.crx' version='{ver}' />\n  </app>\n</gupdate>\n",
                    id = p.extension_id,
                    ver = p.version,
                );
                ("200 OK", "application/xml", xml.into_bytes())
            }
            None => ("404 Not Found", "text/plain", b"not installed".to_vec()),
        }
    } else if path.starts_with("/bia-chrome.crx") {
        match &packed {
            Some(p) => ("200 OK", "application/x-chrome-extension", p.crx.clone()),
            None => ("404 Not Found", "text/plain", b"not installed".to_vec()),
        }
    } else {
        ("404 Not Found", "text/plain", b"not found".to_vec())
    };
    let head = format!("HTTP/1.0 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&body);
}

pub fn install(dir: &Path, updates: &Updates) -> Result<Value, String> {
    let packed = publish(dir, updates)?;
    let id = match &packed {
        Some(p) => p.extension_id.clone(),
        None => {
            let key = crx::load_or_create_key(&dir.join("extension.pem"))?;
            crx::extension_id(&key)?
        }
    };
    let version = packed.as_ref().map(|p| p.version.clone()).unwrap_or_else(|| manifest_version(dir));
    remove_external(&id)?;
    write_host(dir, &id)?;
    write_policies(&id)?;
    fs::write(dir.join("chrome-user"), "bia\n").map_err(|e| e.to_string())?;
    chown_sock("bia");
    Ok(json!({
        "extension_id": id,
        "version": version,
        "crx": if packed.is_some() { "packed" } else { "pending" },
        "extension": "extension/",
        "key": "extension.pem",
        "update_url": format!("http://127.0.0.1:{PORT}/update.xml"),
        "install": "force_installed",
        "native_host": HOST_NAME,
        "next": if packed.is_some() {
            "chrome_install restarts Chrome itself"
        } else {
            "install Google Chrome, then run install to pack extension/ and load it"
        },
    }))
}

/// Serve an already packed CRX, if one is on disk. Does not run Chrome.
/// `install` packs a new CRX when the source changed.
pub fn warm(dir: &Path, updates: &Updates) {
    if let Ok(packed) = load_crx(dir) {
        updates.set(packed);
    }
}

/// Pack `extension/` with `extension.pem` when this machine has Chrome, then serve the CRX.
/// Image builds have no Chrome: the extension id still comes from the shipped key, and the
/// CRX is packed later on the machine that runs Chrome.
pub fn publish(dir: &Path, updates: &Updates) -> Result<Option<Packed>, String> {
    let manifest = dir.join("extension").join("manifest.json");
    if !manifest.is_file() {
        return Err(format!("missing {} — the extension source ships next to this MCP", manifest.display()));
    }
    let _ = crx::load_or_create_key(&dir.join("extension.pem"))?;
    if crate::pack::chrome_bin().is_some() {
        crate::pack::ensure(dir)?;
    }
    match load_crx(dir) {
        Ok(packed) => {
            updates.set(packed.clone());
            Ok(Some(packed))
        }
        Err(e) if crate::pack::chrome_bin().is_some() => Err(e),
        Err(_) => Ok(None),
    }
}

fn load_crx(dir: &Path) -> Result<Packed, String> {
    let crx_path = dir.join("extension.crx");
    let crx = fs::read(&crx_path).map_err(|e| format!("read {}: {e}", crx_path.display()))?;
    if !(crx.starts_with(b"Cr24") && crx.len() > 16) {
        return Err(format!("{} is not a CRX", crx_path.display()));
    }
    let version = manifest_version(dir);
    let key = crx::load_or_create_key(&dir.join("extension.pem"))?;
    let extension_id = crx::extension_id(&key)?;
    Ok(Packed { extension_id, crx, version })
}

fn manifest_version(dir: &Path) -> String {
    let path = dir.join("extension").join("manifest.json");
    let Ok(text) = fs::read_to_string(path) else { return "0.0.0".into() };
    serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|v| v.get("version").and_then(|x| x.as_str()).map(|s| s.to_string()))
        .unwrap_or_else(|| "0.0.0".into())
}

fn sysroot() -> PathBuf {
    std::env::var_os("BIA_SYSROOT").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

fn write_host(dir: &Path, id: &str) -> Result<(), String> {
    let exe = std::env::var_os("BIA_CHROME_MCP_BIN")
        .map(PathBuf::from)
        .or_else(|| std::env::current_exe().ok())
        .ok_or_else(|| "current exe".to_string())?;
    let wrapper = dir.join("native-host");
    let script = format!("#!/bin/sh\nexec '{}' --native-host\n", exe.display());
    fs::write(&wrapper, script).map_err(|e| format!("write native-host: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).map_err(|e| format!("chmod native-host: {e}"))?;
    }
    // Image builds write the script into a staging dir. The manifest must name the
    // path Chrome will see on the guest (/home/bia/MCPs/chrome/native-host).
    let manifest_path = std::env::var("BIA_CHROME_NATIVE_HOST")
        .unwrap_or_else(|_| wrapper.display().to_string());
    let manifest = json!({
        "name": HOST_NAME,
        "description": "bia chrome bridge",
        "path": manifest_path,
        "type": "stdio",
        "allowed_origins": [format!("chrome-extension://{id}/")],
    });
    let text = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;
    for d in native_host_dirs() {
        fs::create_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
        fs::write(d.join(format!("{HOST_NAME}.json")), &text).map_err(|e| format!("host manifest: {e}"))?;
    }
    Ok(())
}

fn write_policies(id: &str) -> Result<(), String> {
    let url = format!("http://127.0.0.1:{PORT}/update.xml");
    let policy = json!({
        "ExtensionInstallSources": [format!("http://127.0.0.1:{PORT}/*")],
        "ExtensionInstallForcelist": [format!("{id};{url}")],
        "ExtensionSettings": {
            id: {
                "installation_mode": "force_installed",
                "update_url": url,
                "toolbar_pin": "force_pinned"
            }
        },
        // a site saving several files at once must not stop at "Download multiple files?
        // Block / Allow", which nobody is there to answer (1 = allow every site)
        "DefaultAutomaticDownloadsSetting": 1
    });
    let text = serde_json::to_vec_pretty(&policy).map_err(|e| e.to_string())?;
    for d in policy_dirs() {
        fs::create_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
        fs::write(d.join("bia-chrome.json"), &text).map_err(|e| format!("policy: {e}"))?;
    }
    Ok(())
}

fn native_host_dirs() -> Vec<PathBuf> {
    let root = sysroot();
    vec![
        root.join("etc/opt/chrome/native-messaging-hosts"),
        root.join("etc/opt/google/chrome/native-messaging-hosts"),
        root.join("etc/chromium/native-messaging-hosts"),
    ]
}

fn policy_dirs() -> Vec<PathBuf> {
    let root = sysroot();
    vec![
        root.join("etc/opt/chrome/policies/managed"),
        root.join("etc/opt/google/chrome/policies/managed"),
        root.join("etc/chromium/policies/managed"),
    ]
}

fn remove_external(id: &str) -> Result<(), String> {
    for d in external_dirs() {
        for name in [format!("{id}.json"), format!("{id}.crx")] {
            let p = d.join(name);
            if p.exists() {
                fs::remove_file(&p).map_err(|e| format!("remove {}: {e}", p.display()))?;
            }
        }
    }
    let pref = crate::proc::profile_dir().join("External Extensions").join(format!("{id}.json"));
    if pref.exists() {
        let _ = fs::remove_file(pref);
    }
    Ok(())
}

fn external_dirs() -> Vec<PathBuf> {
    let root = sysroot();
    vec![
        root.join("usr/share/google-chrome/extensions"),
        root.join("opt/google/chrome/extensions"),
        root.join("usr/share/chromium/extensions"),
    ]
}

fn chown_sock(user: &str) {
    let sock = crate::host::sock_path();
    if sock.exists() {
        let _ = std::process::Command::new("chown").arg(user).arg(&sock).status();
    }
}

pub fn remember_user(dir: &Path) -> Option<String> {
    fs::read_to_string(dir.join("chrome-user")).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}
