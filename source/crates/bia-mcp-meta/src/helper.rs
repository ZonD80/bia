//! The bia-meta process: requests go in as JSON lines and are answered by id; events come
//! out unasked and are handled in order on one thread. When it dies it is started again on
//! the next call (and by the supervisor while an account is logged in).

use crate::state;
use crate::Platform;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Mutex, OnceLock};
use std::time::{Duration, Instant};

type Reply = Result<Value, String>;

struct Proc {
    child: Child,
    stdin: ChildStdin,
    gen: u64,
}

pub struct Helper {
    p: Platform,
    proc_: Mutex<Option<Proc>>,
    pending: Mutex<HashMap<u64, mpsc::Sender<Reply>>>,
    next: AtomicU64,
    gen: AtomicU64,
    events: mpsc::Sender<Value>,
    /// handles each event; set once at start
    on_event: OnceLock<fn(Value)>,
}

static HELPER: OnceLock<Helper> = OnceLock::new();

/// The process for this MCP's platform (set up by [`init`]).
pub fn get() -> &'static Helper {
    HELPER.get().expect("helper::init not called")
}

/// Set up the helper; `on_event` runs for every event, in order.
pub fn init(p: Platform, on_event: fn(Value)) -> &'static Helper {
    let (tx, rx) = mpsc::channel::<Value>();
    let h = HELPER.get_or_init(|| Helper {
        p,
        proc_: Mutex::new(None),
        pending: Mutex::new(HashMap::new()),
        next: AtomicU64::new(1),
        gen: AtomicU64::new(0),
        events: tx,
        on_event: OnceLock::new(),
    });
    let _ = h.on_event.set(on_event);
    let _ = std::thread::Builder::new().name("meta-events".into()).spawn(move || {
        while let Ok(ev) = rx.recv() {
            if let Some(f) = get().on_event.get() {
                f(ev);
            }
        }
    });
    h
}

/// bia-meta: next to this binary, else in the messenger MCP (instagram shares it).
fn binary() -> Result<PathBuf, String> {
    if let Ok(b) = std::env::var("BIA_META_BIN") {
        return Ok(PathBuf::from(b));
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().map(PathBuf::from).unwrap_or_default();
    let here = dir.join("bia-meta");
    if here.is_file() {
        return Ok(here);
    }
    // MCPs/<name>/bin/<arch>/ → MCPs/messenger/bin/<arch>/bia-meta
    if let (Some(arch), Some(mcps)) = (dir.file_name(), dir.parent().and_then(|b| b.parent()).and_then(|m| m.parent())) {
        let shared = mcps.join("messenger").join("bin").join(arch).join("bia-meta");
        if shared.is_file() {
            return Ok(shared);
        }
    }
    Err(format!(
        "bia-meta is missing (looked in {} and in MCPs/messenger/bin): the messenger MCP is incomplete; an OS update (carry_update) puts it back",
        dir.display()
    ))
}

impl Helper {
    pub fn running(&self) -> bool {
        let mut g = self.proc_.lock().unwrap();
        match g.as_mut() {
            Some(p) => matches!(p.child.try_wait(), Ok(None)),
            None => false,
        }
    }

    fn ensure(&self) -> Result<(), String> {
        let mut g = self.proc_.lock().unwrap();
        if let Some(p) = g.as_mut() {
            if matches!(p.child.try_wait(), Ok(None)) {
                return Ok(());
            }
        }
        let bin = binary()?;
        let dir = state::account_dir(self.p);
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let mut child = Command::new(&bin)
            .args(["--platform", self.p.name(), "--dir"])
            .arg(&dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("start {}: {e}", bin.display()))?;
        let gen = self.gen.fetch_add(1, Ordering::SeqCst) + 1;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let stderr = child.stderr.take().ok_or("no stderr")?;
        let stdin = child.stdin.take().ok_or("no stdin")?;
        *g = Some(Proc { child, stdin, gen });
        drop(g);
        let _ = std::thread::Builder::new().name("meta-out".into()).spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                get().incoming(v);
            }
            get().died(gen);
        });
        let _ = std::thread::Builder::new().name("meta-err".into()).spawn(move || {
            for line in BufReader::new(stderr).lines() {
                let Ok(line) = line else { break };
                // all of it (a panic's stack too) into logs/mcp-<platform>.log
                eprintln!("bia-meta: {line}");
                warn_line(&line);
            }
        });
        Ok(())
    }

    fn incoming(&self, v: Value) {
        if v.get("event").is_some() {
            let _ = self.events.send(v);
            return;
        }
        let Some(id) = v.get("id").and_then(Value::as_u64) else { return };
        let Some(tx) = self.pending.lock().unwrap().remove(&id) else { return };
        let reply = match v.get("error").and_then(Value::as_str) {
            Some(e) => Err(e.to_string()),
            None => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
        };
        let _ = tx.send(reply);
    }

    fn died(&self, gen: u64) {
        {
            let mut g = self.proc_.lock().unwrap();
            if g.as_ref().is_some_and(|p| p.gen == gen) {
                if let Some(mut p) = g.take() {
                    let _ = p.child.wait();
                }
            }
        }
        for (_, tx) in self.pending.lock().unwrap().drain() {
            let _ = tx.send(Err("bia-meta stopped (it is restarted by itself); try again".into()));
        }
        crate::inbound::log(&format!("{}: bia-meta stopped", self.p.name()));
    }

    /// Call a bia-meta method and wait for its answer.
    pub fn call(&self, method: &str, params: Value, timeout: Duration) -> Reply {
        self.ensure()?;
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel();
        self.pending.lock().unwrap().insert(id, tx);
        let line = json!({"id": id, "method": method, "params": params}).to_string();
        let wrote = {
            let mut g = self.proc_.lock().unwrap();
            match g.as_mut() {
                Some(p) => writeln!(p.stdin, "{line}").and_then(|_| p.stdin.flush()).map_err(|e| e.to_string()),
                None => Err("bia-meta is not running".into()),
            }
        };
        if let Err(e) = wrote {
            self.pending.lock().unwrap().remove(&id);
            return Err(format!("bia-meta: {e}"));
        }
        match rx.recv_timeout(timeout) {
            Ok(r) => r,
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                Err(format!(
                    "bia-meta did not answer {method} within {}s; try again, and check {}_status if it keeps happening",
                    timeout.as_secs(),
                    self.p.name()
                ))
            }
        }
    }

    /// Stop the process (logout leaves nothing running).
    pub fn stop(&self) {
        if let Some(mut p) = self.proc_.lock().unwrap().take() {
            let _ = p.child.kill();
            let _ = p.child.wait();
        }
    }
}

/// Short calls (status, typing, read marks).
pub const QUICK: Duration = Duration::from_secs(30);
/// Calls that talk to Meta (send text, search, groups).
pub const NORMAL: Duration = Duration::from_secs(150);
/// Uploads and downloads.
pub const LONG: Duration = Duration::from_secs(16 * 60);

/// bia-meta logs on stderr (kept in the MCP's log); warnings and errors also reach the
/// agent console (rate limited).
fn warn_line(line: &str) {
    let important = [" WRN ", " ERR ", " FTL ", "panic"].iter().any(|k| line.contains(k));
    if !important {
        return;
    }
    static WINDOW: Mutex<Option<(Instant, u32)>> = Mutex::new(None);
    {
        let mut w = WINDOW.lock().unwrap();
        let now = Instant::now();
        let (start, n) = w.get_or_insert((now, 0));
        if now.duration_since(*start) >= Duration::from_secs(30) {
            *start = now;
            *n = 0;
        }
        *n += 1;
        if *n > 15 {
            return;
        }
    }
    crate::inbound::log(line);
}
