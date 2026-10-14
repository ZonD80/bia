//! Child process: Google chrome-devtools-mcp connected to our Chrome via --browser-url.
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::proc;

pub struct ToolInfo {
    pub name: String,
    pub description: String,
    pub schema: Value,
}

struct Shared {
    stdin: Mutex<Option<ChildStdin>>,
    pending: Mutex<std::collections::HashMap<u64, Sender<Value>>>,
    next: AtomicU64,
    child: Mutex<Option<Child>>,
    dir: PathBuf,
}

#[derive(Clone)]
pub struct DevTools {
    shared: Arc<Shared>,
}

impl DevTools {
    /// Start chrome-devtools-mcp against `proc::DEBUG_URL`. Requires vendored node + package.
    pub fn start(dir: &Path) -> Result<Self, String> {
        let shared = Arc::new(Shared {
            stdin: Mutex::new(None),
            pending: Mutex::new(std::collections::HashMap::new()),
            next: AtomicU64::new(1),
            child: Mutex::new(None),
            dir: dir.to_path_buf(),
        });
        let dt = DevTools { shared };
        dt.spawn()?;
        Ok(dt)
    }

    pub fn list_tools(&self) -> Result<Vec<ToolInfo>, String> {
        let r = self.request("tools/list", json!({}), Duration::from_secs(30))?;
        let tools = r
            .get("tools")
            .and_then(|t| t.as_array())
            .ok_or("devtools tools/list: missing tools")?;
        Ok(tools
            .iter()
            .filter_map(|t| {
                Some(ToolInfo {
                    name: t.get("name")?.as_str()?.to_string(),
                    description: t
                        .get("description")
                        .and_then(|d| d.as_str())
                        .unwrap_or("")
                        .to_string(),
                    schema: t
                        .get("inputSchema")
                        .cloned()
                        .unwrap_or(json!({"type": "object", "properties": {}})),
                })
            })
            .collect())
    }

    pub fn call(&self, name: &str, args: Value) -> Result<String, String> {
        ensure_chrome()?;
        let r = self.request(
            "tools/call",
            json!({"name": name, "arguments": args}),
            Duration::from_secs(120),
        )?;
        let is_err = r.get("isError").and_then(|e| e.as_bool()).unwrap_or(false);
        let text = content_text(&r);
        if is_err {
            Err(text)
        } else {
            Ok(text)
        }
    }

    /// Kill and respawn after Chrome restart (browser-url session is dead).
    pub fn restart(&self) -> Result<(), String> {
        self.kill();
        self.spawn()
    }

    fn spawn(&self) -> Result<(), String> {
        let (node, entry) = paths(&self.shared.dir)?;
        let mut cmd = Command::new(&node);
        cmd.arg(&entry)
            .args([
                &format!("--browser-url={}", proc::DEBUG_URL),
                "--no-usage-statistics",
                "--no-performance-crux",
                "--no-page-id-routing",
                "--no-category-performance",
                "--no-category-memory",
                "--screenshot-format=jpeg",
                "--screenshot-max-width=1280",
                "--screenshot-max-height=720",
                "--workspace=/home/bia/files",
            ])
            .current_dir(&self.shared.dir)
            .env("CHROME_DEVTOOLS_MCP_NO_USAGE_STATISTICS", "1")
            .env("CHROME_DEVTOOLS_MCP_NO_UPDATE_CHECKS", "1")
            .env("CHROME_DEVTOOLS_MCP_NO_CONFIG_DISCOVERY", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("spawn chrome-devtools-mcp: {e} (node {})", node.display()))?;
        let stdin = child.stdin.take().ok_or("devtools: no stdin")?;
        let stdout = child.stdout.take().ok_or("devtools: no stdout")?;
        *self.shared.stdin.lock().unwrap() = Some(stdin);
        *self.shared.child.lock().unwrap() = Some(child);

        let shared = self.shared.clone();
        thread::Builder::new()
            .name("chrome-devtools-mcp".into())
            .spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    let Ok(v) = serde_json::from_str::<Value>(&line) else {
                        continue;
                    };
                    if let Some(id) = json_id(v.get("id")) {
                        if let Some(tx) = shared.pending.lock().unwrap().remove(&id) {
                            let _ = tx.send(v);
                        }
                    }
                }
                let mut c = shared.child.lock().unwrap();
                if let Some(mut child) = c.take() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                *shared.stdin.lock().unwrap() = None;
            })
            .map_err(|e| e.to_string())?;

        let init = self.request(
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "bia-mcp-chrome", "version": env!("CARGO_PKG_VERSION")}
            }),
            Duration::from_secs(30),
        )?;
        if init.get("serverInfo").is_none() && init.get("error").is_some() {
            return Err(format!("devtools initialize failed: {init}"));
        }
        self.notify("notifications/initialized", json!({}))?;
        Ok(())
    }

    fn kill(&self) {
        if let Some(mut child) = self.shared.child.lock().unwrap().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        *self.shared.stdin.lock().unwrap() = None;
        self.shared.pending.lock().unwrap().clear();
    }

    fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        self.write(&json!({"jsonrpc": "2.0", "method": method, "params": params}))
    }

    fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, String> {
        let id = self.shared.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        self.shared.pending.lock().unwrap().insert(id, tx);
        if let Err(e) = self.write(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})) {
            self.shared.pending.lock().unwrap().remove(&id);
            return Err(e);
        }
        match rx.recv_timeout(timeout) {
            Ok(v) => {
                if let Some(err) = v.get("error") {
                    let msg = err
                        .get("message")
                        .and_then(|m| m.as_str())
                        .unwrap_or("devtools error");
                    return Err(msg.to_string());
                }
                Ok(v.get("result").cloned().unwrap_or(json!({})))
            }
            Err(_) => {
                self.shared.pending.lock().unwrap().remove(&id);
                Err(format!("devtools timed out on {method}"))
            }
        }
    }

    fn write(&self, v: &Value) -> Result<(), String> {
        let mut g = self.shared.stdin.lock().unwrap();
        let w = g.as_mut().ok_or("chrome-devtools-mcp is not running")?;
        writeln!(w, "{v}").map_err(|e| format!("devtools write: {e}"))?;
        w.flush().map_err(|e| format!("devtools flush: {e}"))
    }
}

fn ensure_chrome() -> Result<(), String> {
    proc::require()?;
    if !proc::running() {
        proc::start()?;
    }
    if proc::debug_ready() || proc::wait_debug(15) {
        return Ok(());
    }
    Err(format!(
        "Chrome is running but {} is not accepting connections",
        proc::DEBUG_URL
    ))
}

fn paths(dir: &Path) -> Result<(PathBuf, PathBuf), String> {
    let arch = if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else {
        return Err(format!("unsupported arch {}", std::env::consts::ARCH));
    };
    let node = dir.join("runtime").join(arch).join("node");
    let entry = dir
        .join("devtools")
        .join("build/src/bin/chrome-devtools-mcp.js");
    if !node.is_file() {
        return Err(format!(
            "missing {} — run tools/fetch-chrome-devtools.sh on the host, then patch-mcp",
            node.display()
        ));
    }
    if !entry.is_file() {
        return Err(format!(
            "missing {} — run tools/fetch-chrome-devtools.sh on the host, then patch-mcp",
            entry.display()
        ));
    }
    Ok((node, entry))
}

fn content_text(r: &Value) -> String {
    if let Some(a) = r.get("content").and_then(|c| c.as_array()) {
        let parts: Vec<String> = a
            .iter()
            .filter_map(|c| {
                if let Some(t) = c.get("text").and_then(|t| t.as_str()) {
                    return Some(t.to_string());
                }
                // Image / resource blocks: keep a short pointer so the model can find the file.
                if c.get("type").and_then(|t| t.as_str()) == Some("image") {
                    if let Some(data) = c.get("data").and_then(|d| d.as_str()) {
                        return Some(format!("[image {} bytes]", data.len()));
                    }
                    return Some("[image]".into());
                }
                None
            })
            .collect();
        if !parts.is_empty() {
            return parts.join("\n");
        }
    }
    r.to_string()
}

fn json_id(v: Option<&Value>) -> Option<u64> {
    let v = v?;
    v.as_u64()
        .or_else(|| v.as_i64().and_then(|n| u64::try_from(n).ok()))
        .or_else(|| v.as_str()?.parse().ok())
}
