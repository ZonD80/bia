//! MCP client over stdio + discovery of MCP folders in /home/bia/MCPs.
use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::ffi::CString;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use crate::console::{Console, Inbound, GRAY, RESET};
use crate::jobs;

/// Guest install hint when the WhatsApp MCP cannot load its native Opus dependency.
const WHATSAPP_OPUS_HINT: &str = "WhatsApp MCP needs libopus0 (libopus.so.0). On the guest install it with: apt-get update && apt-get install -y --no-install-recommends libopus0";

fn shared_lib_loadable(soname: &str) -> bool {
    let Ok(c) = CString::new(soname) else {
        return false;
    };
    unsafe {
        let h = libc::dlopen(c.as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL);
        if h.is_null() {
            return false;
        }
        libc::dlclose(h);
        true
    }
}

fn whatsapp_opus_missing_notice(log_dir: &Path) -> Option<String> {
    if !shared_lib_loadable("libopus.so.0") {
        return Some(WHATSAPP_OPUS_HINT.into());
    }
    let log = log_dir.join("mcp-whatsapp.log");
    let Ok(text) = std::fs::read_to_string(&log) else {
        return None;
    };
    if text.contains("libopus.so.0") || text.contains("libopus.so") {
        Some(WHATSAPP_OPUS_HINT.into())
    } else {
        None
    }
}

#[derive(Deserialize, Debug, Clone)]
pub struct Manifest {
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    /// either a string or {"aarch64": "...", "x86_64": "..."}; relative to the MCP folder
    pub command: Value,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// tool to call once at boot (e.g. "restore" to re-apply a saved configuration)
    #[serde(default)]
    pub boot_tool: Option<String>,
    /// short usage examples added to the system prompt, e.g. "configure eth0 with dhcp -> dhcp(interface='eth0')"
    #[serde(default)]
    pub examples: Vec<String>,
    /// words that make the agent load this MCP's functions automatically for a request
    #[serde(default)]
    pub keywords: Vec<String>,
}

impl Manifest {
    pub fn command_for(&self, arch: &str) -> Option<String> {
        match &self.command {
            Value::String(s) => Some(s.clone()),
            Value::Object(m) => m.get(arch).and_then(|v| v.as_str()).map(|s| s.to_string()),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub schema: Value,
}

#[derive(Clone)]
struct SpawnSpec {
    dir: PathBuf,
    manifest: Manifest,
    arch: String,
    env: Vec<(String, String)>,
    log_dir: PathBuf,
    con: Arc<Console>,
    jobs: Arc<Mutex<jobs::Store>>,
}

enum Pending {
    Sync(std::sync::mpsc::Sender<Value>),
    Job(String),
}

struct Shared {
    pending: Mutex<HashMap<u64, Pending>>,
    jobs: Arc<Mutex<jobs::Store>>,
    name: String,
    con: Arc<Console>,
}

/// A handle other threads use to call the server (the outbox). Cheap to clone.
#[derive(Clone)]
pub struct McpLink {
    stdin: Arc<Mutex<ChildStdin>>,
    next_id: Arc<AtomicU64>,
    shared: Arc<Shared>,
}

/// The current link of a server; respawn swaps the link inside, so queued work reaches the new process.
pub type LinkSlot = Arc<Mutex<McpLink>>;

impl McpLink {
    fn send(&self, v: &Value) -> Result<()> {
        let mut w = self.stdin.lock().unwrap_or_else(|e| e.into_inner());
        writeln!(w, "{v}").context("mcp write")?;
        w.flush().context("mcp flush")
    }

    /// Send tools/call. The response arrives on the receiver, which disconnects when the server exits.
    pub fn start(&self, tool: &str, args: Value) -> Result<(u64, Receiver<Value>)> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = channel();
        self.shared
            .pending
            .lock()
            .unwrap()
            .insert(id, Pending::Sync(tx));
        if let Err(e) = self.send(&json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": tool, "arguments": args}})) {
            self.shared.pending.lock().unwrap().remove(&id);
            return Err(e);
        }
        Ok((id, rx))
    }

    /// Stop waiting for a call and ask the server to cancel it (a worker thread may wait
    /// briefly for another write to finish).
    pub fn abandon(&self, jsonrpc_id: u64) -> bool {
        self.shared.pending.lock().unwrap().remove(&jsonrpc_id);
        for _ in 0..20 {
            if let Some(ok) = try_cancel_on(&self.stdin, jsonrpc_id) {
                return ok;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }
}

/// (text, is_error) of a tools/call response line.
pub fn call_result(v: &Value) -> (String, bool) {
    if let Some(e) = v.get("error") {
        return (
            e.get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("error")
                .to_string(),
            true,
        );
    }
    parse_tool_result(v.get("result").unwrap_or(&Value::Null))
}

/// Ask the server to cancel a call without ever blocking: a stuck server that stopped
/// reading stdin must not freeze the agent. The line is under PIPE_BUF, so it is
/// written whole or not at all. None while another thread is writing: O_NONBLOCK on the
/// shared fd must not leak into its write.
fn try_cancel_on(stdin: &Mutex<ChildStdin>, jsonrpc_id: u64) -> Option<bool> {
    let w = match stdin.try_lock() {
        Ok(w) => w,
        Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => return None,
    };
    let fd = w.as_raw_fd();
    let line = format!(
        "{}\n",
        json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": jsonrpc_id}})
    );
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags < 0 {
            return Some(false);
        }
        libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        let n = libc::write(fd, line.as_ptr() as *const libc::c_void, line.len());
        libc::fcntl(fd, libc::F_SETFL, flags);
        Some(n == line.len() as isize)
    }
}

pub struct McpServer {
    pub name: String,
    pub version: String,
    pub dir: PathBuf,
    child: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    shared: Arc<Shared>,
    next_id: Arc<AtomicU64>,
    slot: LinkSlot,
    spec: SpawnSpec,
    started: SystemTime,
}

impl McpServer {
    pub fn spawn(
        dir: &Path,
        m: &Manifest,
        arch: &str,
        env: &[(String, String)],
        log_dir: &Path,
        con: Arc<Console>,
        jobs: Arc<Mutex<jobs::Store>>,
    ) -> Result<McpServer> {
        let cmd = m
            .command_for(arch)
            .ok_or_else(|| anyhow!("no command for {arch}"))?;
        let exe = if Path::new(&cmd).is_absolute() {
            PathBuf::from(&cmd)
        } else {
            dir.join(&cmd)
        };
        if !exe.exists() {
            bail!("{} missing", exe.display());
        }
        // Preflight: without libopus the dynamic linker exits before main.
        if m.name == "whatsapp" && !shared_lib_loadable("libopus.so.0") {
            bail!("{WHATSAPP_OPUS_HINT}");
        }
        let args = &m.args;
        std::fs::create_dir_all(log_dir).ok();
        let stderr = std::fs::File::create(log_dir.join(format!("mcp-{}.log", m.name)))
            .map(Stdio::from)
            .unwrap_or_else(|_| Stdio::null());
        let mut c = Command::new(&exe);
        c.args(args)
            .current_dir(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(stderr);
        for (k, v) in env {
            c.env(k, v);
        }
        for (k, v) in &m.env {
            c.env(k, v);
        }
        let mut child = c
            .spawn()
            .with_context(|| format!("spawn {}", exe.display()))
            .map_err(|e| {
                if m.name == "whatsapp" {
                    if let Some(hint) = whatsapp_opus_missing_notice(log_dir) {
                        return anyhow!("{e:#}\n{hint}");
                    }
                }
                e
            })?;
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let shared = Arc::new(Shared {
            pending: Mutex::new(HashMap::new()),
            jobs: jobs.clone(),
            name: m.name.clone(),
            con: con.clone(),
        });
        let sh = shared.clone();
        std::thread::Builder::new()
            .name(format!("mcp-{}", m.name))
            .spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    let Ok(v) = serde_json::from_str::<Value>(&line) else {
                        continue;
                    };
                    dispatch(&sh, v);
                }
                // Server gone: waiters see a disconnect instead of waiting out their timeout.
                sh.pending
                    .lock()
                    .unwrap()
                    .retain(|_, p| !matches!(p, Pending::Sync(_)));
            })?;
        let spec = SpawnSpec {
            dir: dir.into(),
            manifest: m.clone(),
            arch: arch.into(),
            env: env.to_vec(),
            log_dir: log_dir.into(),
            con: con.clone(),
            jobs,
        };
        let stdin = Arc::new(Mutex::new(stdin));
        let next_id = Arc::new(AtomicU64::new(1));
        let slot = Arc::new(Mutex::new(McpLink {
            stdin: stdin.clone(),
            next_id: next_id.clone(),
            shared: shared.clone(),
        }));
        let mut s = McpServer {
            name: m.name.clone(),
            version: m.version.clone(),
            dir: spec.dir.clone(),
            child,
            stdin,
            shared,
            next_id,
            slot,
            spec,
            started: SystemTime::now(),
        };
        let init = match s.request(
            "initialize",
            json!({"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "bia", "version": env!("CARGO_PKG_VERSION")}}),
            Duration::from_secs(60),
        ) {
            Ok(v) => v,
            Err(e) => {
                if m.name == "whatsapp" {
                    if let Some(hint) = whatsapp_opus_missing_notice(log_dir) {
                        bail!("MCP whatsapp failed to start: {e:#}\n{hint}");
                    }
                }
                return Err(e);
            }
        };
        if let Some(v) = init.pointer("/serverInfo/version").and_then(|v| v.as_str()) {
            if s.version.is_empty() {
                s.version = v.to_string();
            }
        }
        s.notify("notifications/initialized", json!({}))?;
        Ok(s)
    }

    fn send(&mut self, v: &Value) -> Result<()> {
        let mut w = self.stdin.lock().unwrap_or_else(|e| e.into_inner());
        writeln!(w, "{v}").context("mcp write")?;
        w.flush().context("mcp flush")
    }

    /// Link for calls made from other threads; stays valid across respawn.
    pub fn link(&self) -> LinkSlot {
        self.slot.clone()
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        self.send(&json!({"jsonrpc": "2.0", "method": method, "params": params}))
    }

    fn request(&mut self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = channel();
        self.shared
            .pending
            .lock()
            .unwrap()
            .insert(id, Pending::Sync(tx));
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))?;
        match rx.recv_timeout(timeout) {
            Ok(v) => {
                if let Some(e) = v.get("error") {
                    bail!(
                        "{}: {}",
                        self.name,
                        e.get("message").and_then(|m| m.as_str()).unwrap_or("error")
                    );
                }
                Ok(v.get("result").cloned().unwrap_or(Value::Null))
            }
            Err(RecvTimeoutError::Timeout) => {
                self.shared.pending.lock().unwrap().remove(&id);
                bail!("{}: {method} timed out", self.name)
            }
            Err(RecvTimeoutError::Disconnected) => bail!("{}: server exited", self.name),
        }
    }

    pub fn list_tools(&mut self) -> Result<Vec<McpTool>> {
        let r = self.request("tools/list", json!({}), Duration::from_secs(60))?;
        Ok(r.get("tools")
            .and_then(|t| t.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|t| {
                        Some(McpTool {
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
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Returns (text, is_error). Blocking — for boot_tool and internal telegram send/typing.
    pub fn call(
        &mut self,
        tool: &str,
        args: Value,
        _on_log: &mut dyn FnMut(&str),
    ) -> Result<(String, bool)> {
        let r = self.request(
            "tools/call",
            json!({"name": tool, "arguments": args}),
            Duration::from_secs(3600),
        )?;
        Ok(parse_tool_result(&r))
    }

    /// Fire tools/call without waiting. Result completes the job on the stdout thread.
    pub fn fire(&mut self, tool: &str, args: Value, job_id: &str) -> Result<u64> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.shared
            .pending
            .lock()
            .unwrap()
            .insert(id, Pending::Job(job_id.into()));
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": tool, "arguments": args}}))?;
        Ok(id)
    }

    /// Ask the server to cancel a call without blocking (see try_cancel_fd).
    pub fn try_cancel(&mut self, jsonrpc_id: u64) -> bool {
        try_cancel_on(&self.stdin, jsonrpc_id).unwrap_or(false)
    }

    pub fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    pub fn transport_error(e: &anyhow::Error) -> bool {
        let s = format!("{e:#}");
        s.contains("mcp write") || s.contains("mcp flush") || s.contains("server exited")
    }

    /// Kill and spawn again from `spec.dir` (re-reads mcp.json). Re-runs boot_tool if set.
    pub fn respawn(&mut self) -> Result<()> {
        let mut spec = self.spec.clone();
        let mf = spec.dir.join("mcp.json");
        if let Ok(s) = std::fs::read_to_string(&mf) {
            if let Ok(m) = serde_json::from_str::<Manifest>(&s) {
                spec.manifest = m;
            }
        }
        self.kill();
        let mut new = McpServer::spawn(
            &spec.dir,
            &spec.manifest,
            &spec.arch,
            &spec.env,
            &spec.log_dir,
            spec.con,
            spec.jobs,
        )?;
        if let Some(bt) = spec.manifest.boot_tool.clone() {
            let _ = new.call(&bt, json!({}), &mut |_| {});
        }
        // Keep the slot others hold; point it at the new process.
        let fresh = new.slot.lock().unwrap().clone();
        *self.slot.lock().unwrap_or_else(|e| e.into_inner()) = fresh;
        new.slot = self.slot.clone();
        *self = new;
        Ok(())
    }

    /// Switch to another folder under /home/bia/MCPs and respawn.
    pub fn relocate(&mut self, dir: PathBuf) -> Result<()> {
        self.spec.dir = dir;
        self.respawn()
    }

    /// True if mcp.json or the launch command on disk is newer than this process.
    pub fn sources_stale(&self) -> bool {
        let newer = |p: &Path| {
            std::fs::metadata(p)
                .ok()
                .and_then(|m| m.modified().ok())
                .map(|t| t > self.started)
                .unwrap_or(false)
        };
        if newer(&self.spec.dir.join("mcp.json")) {
            return true;
        }
        let Some(cmd) = self.spec.manifest.command_for(&self.spec.arch) else {
            return false;
        };
        let exe = if Path::new(&cmd).is_absolute() {
            PathBuf::from(&cmd)
        } else {
            self.spec.dir.join(&cmd)
        };
        newer(&exe)
    }

    pub fn ensure_alive(&mut self) -> Result<()> {
        if self.alive() {
            Ok(())
        } else {
            self.respawn()
        }
    }
}

impl Drop for McpServer {
    fn drop(&mut self) {
        self.kill();
    }
}

fn dispatch(sh: &Shared, v: Value) {
    if v.get("id").is_none() {
        if let Some(ib) = inbound_from_notification(&v) {
            let tag = if sh.con.color {
                format!("{GRAY}[{}@{}]{RESET}", ib.identity, ib.channel)
            } else {
                format!("[{}@{}]", ib.identity, ib.channel)
            };
            sh.con.writeln(&format!("  {tag} {}", ib.text));
            sh.con.push_inbound(ib);
            return;
        }
        if let Some(msg) = log_from_notification(&v) {
            sh.con.writeln(&sh.con.c(GRAY, &format!("     {msg}")));
            if let Ok(st) = sh.jobs.lock() {
                if let Some(jid) = job_id_for_log(&st, &sh.name, &v) {
                    st.append_log(&jid, &msg);
                }
            }
        }
        return;
    }
    let Some(id) = json_id(v.get("id")) else {
        return;
    };
    let pending = sh.pending.lock().unwrap().remove(&id);
    match pending {
        Some(Pending::Sync(tx)) => {
            let _ = tx.send(v);
        }
        Some(Pending::Job(jid)) => {
            complete_job(&sh.jobs, &sh.con, &jid, &v);
        }
        None => {}
    }
}

fn complete_job(jobs: &Mutex<jobs::Store>, con: &Console, jid: &str, v: &Value) {
    let (text, is_err) = if let Some(e) = v.get("error") {
        (
            e.get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("error")
                .to_string(),
            true,
        )
    } else {
        parse_tool_result(v.get("result").unwrap_or(&Value::Null))
    };
    let state = if text.trim() == "cancelled" || text.trim_start().starts_with("cancelled") {
        "cancelled"
    } else if is_err {
        "error"
    } else {
        "ok"
    };
    let body = format!("{state}\n{text}");
    let mut st = jobs.lock().unwrap();
    if st.finish(jid, state, &text) {
        st.append_log(jid, &format!("done {state}"));
        let log = st.logs(jid, 80).unwrap_or_default();
        drop(st);
        let body = if log.trim().is_empty() || log == "(empty)" || log == "(no log)" {
            body
        } else {
            format!("{body}\n--- log ---\n{log}")
        };
        con.push_inbound(jobs::inbound(jid, &body));
    }
}

fn job_id_for_log(st: &jobs::Store, mcp: &str, v: &Value) -> Option<String> {
    if let Some(id) = json_id(v.pointer("/params/requestId")) {
        return st.by_rpc(mcp, id).map(|j| j.id.clone());
    }
    let running = st.running_on(mcp);
    if running.len() == 1 {
        Some(running[0].id.clone())
    } else {
        None
    }
}

fn parse_tool_result(r: &Value) -> (String, bool) {
    let is_err = r.get("isError").and_then(|e| e.as_bool()).unwrap_or(false);
    let text = r
        .get("content")
        .and_then(|c| c.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|c| c.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_else(|| r.to_string());
    (text, is_err)
}

fn json_id(v: Option<&Value>) -> Option<u64> {
    let v = v?;
    v.as_u64()
        .or_else(|| v.as_i64().and_then(|n| u64::try_from(n).ok()))
        .or_else(|| v.as_str()?.parse().ok())
}

fn inbound_from_notification(v: &Value) -> Option<Inbound> {
    if v.get("method").and_then(|m| m.as_str()) != Some("notifications/message") {
        return None;
    }
    let d = v.pointer("/params/data")?;
    let obj = d.as_object()?;
    if obj.get("t").and_then(|t| t.as_str()) != Some("inbound") {
        return None;
    }
    Some(
        Inbound {
            identity: obj
                .get("identity")
                .and_then(|x| x.as_str())
                .unwrap_or("unknown")
                .to_string(),
            channel: obj.get("channel").and_then(|x| x.as_str())?.to_string(),
            issued: std::time::SystemTime::now(),
            id: obj
                .get("id")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            text: obj
                .get("message")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
        }
        .ensure_id(),
    )
}

fn log_from_notification(v: &Value) -> Option<String> {
    if v.get("method").and_then(|m| m.as_str()) != Some("notifications/message") {
        return None;
    }
    let d = v.pointer("/params/data")?;
    if d.is_object() {
        return None;
    }
    Some(
        d.as_str()
            .map(|s| s.to_string())
            .unwrap_or_else(|| d.to_string()),
    )
}

pub fn discover(dir: &Path) -> Vec<(PathBuf, Result<Manifest>)> {
    let mut out = vec![];
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        let mf = p.join("mcp.json");
        if !mf.exists() || e.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let m = std::fs::read_to_string(&mf)
            .map_err(|e| anyhow!("{e}"))
            .and_then(|s| {
                serde_json::from_str::<Manifest>(&s).map_err(|e| anyhow!("mcp.json: {e}"))
            });
        out.push((p, m));
    }
    out
}

/// Timeout for a model-facing MCP tool.
pub fn timeout_for(_tool: &str) -> Duration {
    Duration::from_secs(3600)
}
