//! Console: the agent's terminal. In system mode it writes to every kernel console
//! (screen and serial alike) and reads lines from all of them. In `--volume` dev mode it is stdin/stdout.
//! A three-line status header stays pinned at the top (scroll region below it) and is redrawn
//! by a background thread whenever its content changes.
use std::collections::VecDeque;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::io::AsRawFd;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Mint a short local message id when the bus does not supply one.
/// Format: `b` + 8 hex millis + 4 hex seq (no `|`). Stable for later reference.
pub fn mint_msg_id() -> String {
    static SEQ: AtomicU64 = AtomicU64::new(1);
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("b{:08x}{:04x}", ms as u32, n as u16)
}

/// One message arriving on a channel (typed on the console, or inbound from a bus).
#[derive(Clone, Debug)]
pub struct Inbound {
    pub identity: String,
    pub channel: String,
    pub issued: SystemTime,
    /// Message id: platform id (Telegram/WhatsApp) or a locally minted `b…` id.
    pub id: String,
    pub text: String,
}

/// Human messages from the buses are journaled; system notices, console input, job
/// results and schedules are not (they are regenerated or local).
fn journaled(ib: &Inbound) -> bool {
    ib.identity != crate::tools::ID_SYSTEM
        && ib.channel != "console"
        && !ib.channel.starts_with("job:")
        && ib.channel != "schedule"
}

/// Journal entries without a matching `done`, in order.
fn journal_pending(path: &std::path::Path) -> Vec<Inbound> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut ins: Vec<Inbound> = Vec::new();
    let mut done = std::collections::HashSet::new();
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        match v.get("op").and_then(|x| x.as_str()) {
            Some("in") => ins.push(Inbound {
                identity: s("identity"),
                channel: s("channel"),
                issued: std::time::UNIX_EPOCH
                    + std::time::Duration::from_secs(
                        v.get("issued").and_then(|x| x.as_u64()).unwrap_or(0),
                    ),
                id: s("id"),
                text: s("text"),
            }),
            Some("done") => {
                done.insert(s("id"));
            }
            _ => {}
        }
    }
    let mut seen = std::collections::HashSet::new();
    ins.into_iter()
        .filter(|ib| !done.contains(&ib.id) && seen.insert(ib.id.clone()))
        .collect()
}

impl Inbound {
    pub fn console(text: impl Into<String>) -> Self {
        Inbound {
            identity: crate::tools::ID_ROOT.into(),
            channel: crate::tools::CH_CONSOLE.into(),
            issued: SystemTime::now(),
            id: mint_msg_id(),
            text: text.into(),
        }
    }

    /// Fill `id` with a local mint when empty (keeps platform ids as-is).
    pub fn ensure_id(mut self) -> Self {
        if self.id.is_empty() {
            self.id = mint_msg_id();
        }
        self
    }
}

// ---- colors ----
pub const RESET: &str = "\x1b[0m";
pub const BOLD: &str = "\x1b[1m";
pub const GRAY: &str = "\x1b[90m";
pub const RED: &str = "\x1b[31m";
pub const GREEN: &str = "\x1b[32m";
pub const YELLOW: &str = "\x1b[33m";
pub const CYAN: &str = "\x1b[36m";
pub const BGREEN: &str = "\x1b[1;32m";
pub const BCYAN: &str = "\x1b[1;36m";
pub const HEADER_BG: &str = "\x1b[97;44m"; // white on blue

const HEADER_LINES: u16 = 3;

struct Out {
    /// 0 for the machine's own outputs; root's web console mirrors get their own id.
    id: usize,
    w: Box<dyn Write + Send>,
    /// Tty fd for `TIOCGWINSZ`. `None` when output is not a terminal, or a mirror.
    fd: Option<i32>,
    rows: u16,
    cols: u16,
    tty: bool,
}

/// Writer that forwards bytes to a console mirror client.
struct ChannelWriter {
    tx: Sender<Vec<u8>>,
}
impl Write for ChannelWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.tx
            .send(buf.to_vec())
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "mirror gone"))?;
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Recent console output replayed to a mirror when it connects.
const REPLAY_BYTES: usize = 256 * 1024;

#[derive(Default, Clone)]
pub struct StatusData {
    pub version: String,
    pub arch: String,
    pub kernel: String,
    pub model: String,
    pub model_detail: String,
    pub ctx_used: usize,
    pub ctx_total: usize,
    pub tps: f32,
    pub tools: usize,
    pub state: String,
    pub messages: usize,
    /// HTTPS port of the web MCP (0 while it is not listening), from files/state/web.json
    pub web_port: u16,
    /// its HTTPS addresses, space-separated
    pub web_login: String,
}

impl StatusData {
    /// What the web MCP shows on its status page (files/state/status.json).
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "version": self.version,
            "arch": self.arch,
            "kernel": self.kernel,
            "model": self.model,
            "model_detail": self.model_detail,
            "ctx_used": self.ctx_used,
            "ctx_total": self.ctx_total,
            "tps": self.tps,
            "tools": self.tools,
            "state": self.state,
            "messages": self.messages,
        })
    }
}

pub struct Console {
    outs: Mutex<Vec<Out>>,
    rx: Mutex<Receiver<Inbound>>,
    tx: Sender<Inbound>,
    held: Mutex<VecDeque<Inbound>>,
    /// Durable inbox (files/state/inbox.jsonl): human messages from the buses until the
    /// model has taken them, so a reboot/update/crash in between does not lose them.
    journal: Mutex<Option<std::path::PathBuf>>,
    pub devices: Vec<String>,
    pub color: bool,
    status: Mutex<StatusData>,
    last_drawn: Mutex<Vec<String>>,
    /// files/state/status.json and what was last written there
    status_file: Mutex<Option<(std::path::PathBuf, String)>>,
    next_id: Mutex<usize>,
    recent: Mutex<VecDeque<u8>>,
}

fn winsize(fd: i32) -> (u16, u16) {
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    if unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) } == 0 && ws.ws_row > 0 && ws.ws_col > 0
    {
        (ws.ws_row, ws.ws_col)
    } else {
        (24, 80)
    }
}

impl Console {
    /// No outputs, no readers, no status thread: for unit tests.
    #[cfg(test)]
    pub fn for_test() -> Arc<Console> {
        let (tx, rx) = channel::<Inbound>();
        Arc::new(Console {
            outs: Mutex::new(vec![]),
            rx: Mutex::new(rx),
            tx,
            held: Mutex::new(VecDeque::new()),
            journal: Mutex::new(None),
            devices: vec![],
            color: false,
            status: Mutex::new(StatusData::default()),
            last_drawn: Mutex::new(vec![]),
            status_file: Mutex::new(None),
            next_id: Mutex::new(0),
            recent: Mutex::new(VecDeque::new()),
        })
    }

    pub fn open(init_mode: bool) -> Arc<Console> {
        let (tx, rx) = channel::<Inbound>();
        let mut outs: Vec<Out> = vec![];
        let mut devices = vec![];
        let gui = std::env::var_os("BIA_GUI").is_some();
        if init_mode {
            for dev in console_devices() {
                if gui && is_vt(&dev) {
                    continue;
                }
                if let Ok(f) = OpenOptions::new().write(true).open(&dev) {
                    let fd = f.as_raw_fd();
                    let (rows, cols) = winsize(fd);
                    outs.push(Out {
                        id: 0,
                        w: Box::new(f),
                        fd: Some(fd),
                        rows,
                        cols,
                        tty: true,
                    });
                    if let Ok(r) = OpenOptions::new().read(true).open(&dev) {
                        let fd = r.as_raw_fd();
                        match OpenOptions::new().write(true).open(&dev) {
                            Ok(echo) => spawn_tty_reader(r, fd, echo, tx.clone(), dev.clone()),
                            Err(_) => spawn_reader(r, tx.clone(), dev.clone()),
                        }
                    }
                    devices.push(dev);
                }
            }
        }
        if gui || outs.is_empty() {
            let tty = unsafe { libc::isatty(1) } == 1;
            let (rows, cols) = if tty { winsize(1) } else { (0, 0) };
            outs.push(Out {
                id: 0,
                w: Box::new(std::io::stdout()),
                fd: tty.then_some(1),
                rows,
                cols,
                tty,
            });
            spawn_tty_reader(
                std::io::stdin(),
                0,
                std::io::stdout(),
                tx.clone(),
                "stdin".into(),
            );
            devices.push("stdio".into());
        }
        let color = outs.iter().any(|o| o.tty);
        let con = Arc::new(Console {
            outs: Mutex::new(outs),
            rx: Mutex::new(rx),
            tx,
            held: Mutex::new(VecDeque::new()),
            journal: Mutex::new(None),
            devices,
            color,
            status: Mutex::new(StatusData::default()),
            last_drawn: Mutex::new(vec![]),
            status_file: Mutex::new(None),
            next_id: Mutex::new(0),
            recent: Mutex::new(VecDeque::with_capacity(REPLAY_BYTES)),
        });
        con.init_screen();
        let c = con.clone();
        std::thread::Builder::new()
            .name("status".into())
            .spawn(move || loop {
                let resized = c.refresh_tty_size();
                c.draw_status(resized);
                std::thread::sleep(Duration::from_millis(250));
            })
            .expect("status thread");
        con
    }

    /// Clear the screen and reserve the header lines (scroll region below them).
    /// Re-read each tty. When the window changes, update the scroll region so the
    /// three-line status header is redrawn at the new width.
    fn refresh_tty_size(&self) -> bool {
        let mut changed = false;
        let mut outs = self.outs.lock().unwrap();
        for o in outs.iter_mut() {
            let Some(fd) = o.fd else { continue };
            let (rows, cols) = winsize(fd);
            if rows == 0 || cols == 0 || (rows == o.rows && cols == o.cols) {
                continue;
            }
            o.rows = rows;
            o.cols = cols;
            if std::env::var_os("BIA_GUI").is_none() {
                let _ = write!(o.w, "\x1b[{};{}r", HEADER_LINES + 1, rows);
            }
            let _ = o.w.flush();
            changed = true;
        }
        changed
    }

    fn init_screen(&self) {
        let mut outs = self.outs.lock().unwrap();
        for o in outs.iter_mut() {
            if o.tty {
                if std::env::var_os("BIA_GUI").is_some() {
                    let _ = write!(o.w, "\x1b[0m\x1b[2J\x1b[H");
                } else {
                    let _ = write!(
                        o.w,
                        "\x1b[0m\x1b[2J\x1b[{};{}r\x1b[{};1H",
                        HEADER_LINES + 1,
                        o.rows,
                        HEADER_LINES + 1
                    );
                }
                let _ = o.w.flush();
            }
        }
    }

    pub fn write(&self, s: &str) {
        // outs before recent, as in add_mirror: a new mirror sees each write exactly once
        let mut outs = self.outs.lock().unwrap();
        {
            let mut r = self.recent.lock().unwrap();
            let b = s.as_bytes();
            let b = &b[b.len().saturating_sub(REPLAY_BYTES)..];
            let extra = (r.len() + b.len()).saturating_sub(REPLAY_BYTES);
            r.drain(..extra);
            r.extend(b);
        }
        for o in outs.iter_mut() {
            let _ = o.w.write_all(s.as_bytes());
            let _ = o.w.flush();
        }
    }

    // ---- console mirrors (root's web console, via the web MCP) ----

    /// Serve console mirrors on a Unix socket (protocol in `consock.rs`); root-only (0600).
    pub fn serve_mirror(self: &Arc<Self>, path: std::path::PathBuf) -> std::io::Result<()> {
        use std::os::unix::fs::PermissionsExt;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
            // only the socket's own directory (/run/bia/root), never /run/bia: user bia's
            // sockets live there (the Chrome passkey bridge broke when it was 0700)
            if dir.starts_with("/run/bia/") {
                std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
            }
        }
        let _ = std::fs::remove_file(&path);
        let listener = std::os::unix::net::UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        let con = self.clone();
        std::thread::Builder::new()
            .name("console-mirror".into())
            .spawn(move || {
                for s in listener.incoming().flatten() {
                    let c = con.clone();
                    let _ = std::thread::Builder::new()
                        .name("console-mirror-client".into())
                        .spawn(move || c.mirror_client(s));
                }
            })?;
        Ok(())
    }

    fn mirror_client(&self, s: std::os::unix::net::UnixStream) {
        let Ok(r) = s.try_clone() else { return };
        let mut lines = BufReader::new(r).lines();
        let Some(Ok(first)) = lines.next() else {
            return;
        };
        let dim = |v: &serde_json::Value, k: &str, d: u64| {
            v.get(k).and_then(|x| x.as_u64()).unwrap_or(d).min(1000) as u16
        };
        let v: serde_json::Value = serde_json::from_str(&first).unwrap_or_default();
        let (id, rx) = self.add_mirror(dim(&v, "rows", 30), dim(&v, "cols", 100));
        let mut out = s;
        let writer = std::thread::spawn(move || {
            while let Ok(bytes) = rx.recv() {
                if out.write_all(&bytes).is_err() {
                    break;
                }
            }
            let _ = out.shutdown(std::net::Shutdown::Both);
        });
        for line in lines {
            let Ok(line) = line else { break };
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
                continue;
            };
            match v.get("t").and_then(|t| t.as_str()) {
                Some("line") => {
                    let text = v.get("v").and_then(|x| x.as_str()).unwrap_or("");
                    self.mirror_line(&text.replace(['\r', '\n'], " "));
                }
                Some("resize") => self.resize_mirror(id, dim(&v, "rows", 30), dim(&v, "cols", 100)),
                _ => {}
            }
        }
        self.remove_mirror(id);
        let _ = writer.join();
    }

    /// A full console stream like a physical TTY (status header, scroll region, colors),
    /// starting with a replay of recent output.
    fn add_mirror(&self, rows: u16, cols: u16) -> (usize, Receiver<Vec<u8>>) {
        let (tx, rx) = channel::<Vec<u8>>();
        let id = {
            let mut n = self.next_id.lock().unwrap();
            *n += 1;
            *n
        };
        let rows = rows.max(HEADER_LINES + 2);
        let cols = cols.max(20);
        let mut w = ChannelWriter { tx };
        let _ = write!(
            w,
            "\x1b[0m\x1b[2J\x1b[{};{}r\x1b[{};1H",
            HEADER_LINES + 1,
            rows,
            HEADER_LINES + 1
        );
        {
            // hold outs while replaying so no write lands between the replay and the sink
            let mut outs = self.outs.lock().unwrap();
            let replay: Vec<u8> = self.recent.lock().unwrap().iter().copied().collect();
            let _ = w.write_all(&replay);
            outs.push(Out {
                id,
                w: Box::new(w),
                fd: None,
                rows,
                cols,
                tty: true,
            });
        }
        self.last_drawn.lock().unwrap().clear();
        self.draw_status(true);
        (id, rx)
    }

    fn remove_mirror(&self, id: usize) {
        self.outs.lock().unwrap().retain(|o| o.id != id);
        self.last_drawn.lock().unwrap().clear();
    }

    fn resize_mirror(&self, id: usize, rows: u16, cols: u16) {
        {
            let mut outs = self.outs.lock().unwrap();
            let Some(o) = outs.iter_mut().find(|o| o.id == id) else {
                return;
            };
            o.rows = rows.max(HEADER_LINES + 2);
            o.cols = cols.max(20);
            let _ = write!(o.w, "\x1b[{};{}r", HEADER_LINES + 1, o.rows);
        }
        self.last_drawn.lock().unwrap().clear();
        self.draw_status(true);
    }

    /// A line typed on root's web console: echoed on every output, then queued like a
    /// line typed on the machine (an empty line interrupts the model).
    fn mirror_line(&self, line: &str) {
        self.writeln(&self.c(GREEN, line));
        self.push_inbound(Inbound::console(line));
    }

    pub fn push_inbound(&self, ib: Inbound) {
        let ib = ib.ensure_id();
        if journaled(&ib) {
            self.journal_append(&serde_json::json!({
                "op": "in",
                "identity": ib.identity,
                "channel": ib.channel,
                "issued": ib.issued.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
                "id": ib.id,
                "text": ib.text,
            }));
        }
        let _ = self.tx.send(ib);
    }

    /// Start journaling to `path`; returns the messages a previous run received but the
    /// model never took (oldest first). The file is compacted to just those.
    pub fn open_journal(&self, path: std::path::PathBuf) -> Vec<Inbound> {
        let pending = journal_pending(&path);
        let body: String = pending
            .iter()
            .map(|ib| {
                serde_json::json!({
                    "op": "in", "identity": ib.identity, "channel": ib.channel,
                    "issued": ib.issued.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
                    "id": ib.id, "text": ib.text,
                })
                .to_string()
                    + "\n"
            })
            .collect();
        if let Some(d) = path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(&path, body);
        *self.journal.lock().unwrap() = Some(path);
        pending
    }

    /// The model took message `id` into its turn.
    pub fn journal_done(&self, id: &str) {
        if id.is_empty() {
            return;
        }
        self.journal_append(&serde_json::json!({"op": "done", "id": id}));
        // keep the file small: compact once it grows
        let path = self.journal.lock().unwrap().clone();
        if let Some(p) = path {
            if std::fs::metadata(&p)
                .map(|m| m.len() > 1_000_000)
                .unwrap_or(false)
            {
                let _ = self.open_journal(p);
            }
        }
    }

    fn journal_append(&self, v: &serde_json::Value) {
        use std::io::Write;
        let g = self.journal.lock().unwrap();
        let Some(p) = g.as_ref() else { return };
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(p)
        {
            let _ = writeln!(f, "{v}");
        }
    }
    pub fn hold_inbound(&self, ib: Inbound) {
        self.held.lock().unwrap().push_back(ib.ensure_id());
    }
    pub fn read_inbound(&self) -> Option<Inbound> {
        {
            let mut h = self.held.lock().unwrap();
            if let Some(ib) = h.pop_front() {
                return Some(ib);
            }
        }
        self.rx.lock().unwrap().recv().ok()
    }
    /// Like `read_inbound`, but `Err(())` when nothing arrives within `d`.
    pub fn read_inbound_timeout(&self, d: std::time::Duration) -> Result<Option<Inbound>, ()> {
        {
            let mut h = self.held.lock().unwrap();
            if let Some(ib) = h.pop_front() {
                return Ok(Some(ib));
            }
        }
        match self.rx.lock().unwrap().recv_timeout(d) {
            Ok(ib) => Ok(Some(ib)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err(()),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Ok(None),
        }
    }
    pub fn try_read_inbound(&self) -> Option<Inbound> {
        {
            let mut h = self.held.lock().unwrap();
            if let Some(ib) = h.pop_front() {
                return Some(ib);
            }
        }
        self.rx.lock().unwrap().try_recv().ok()
    }
    /// Block until a message arrives on `channel`; other inbound is held for later.
    pub fn wait_inbound(&self, channel: &str) -> Option<Inbound> {
        loop {
            let ib = self.read_inbound()?;
            if ib.channel == channel {
                return Some(ib);
            }
            self.hold_inbound(ib);
        }
    }
    pub fn writeln(&self, s: &str) {
        self.write(s);
        self.write("\n");
    }
    /// Colored text (no-op colors when the output is not a terminal).
    pub fn c(&self, color: &str, s: &str) -> String {
        if self.color {
            format!("{color}{s}{RESET}")
        } else {
            s.to_string()
        }
    }
    /// Blocking read of one console line (without the newline). Other channels are held.
    pub fn read_line(&self) -> Option<String> {
        self.wait_inbound(crate::tools::CH_CONSOLE)
            .map(|ib| ib.text)
    }
    /// Non-blocking: a console line typed since the last read, if any.
    pub fn try_read_line(&self) -> Option<String> {
        match self.try_read_inbound() {
            None => None,
            Some(ib) if ib.channel == crate::tools::CH_CONSOLE => Some(ib.text),
            Some(ib) => {
                self.hold_inbound(ib);
                None
            }
        }
    }
    pub fn drain(&self) {
        while self.try_read_line().is_some() {}
    }

    pub fn update_status(&self, f: impl FnOnce(&mut StatusData)) {
        let json = {
            let mut st = self.status.lock().unwrap();
            f(&mut st);
            st.to_json().to_string()
        };
        self.draw_status(false);
        if let Some((path, last)) = self.status_file.lock().unwrap().as_mut() {
            if *last != json {
                let tmp = path.with_extension("json.tmp");
                if std::fs::write(&tmp, &json).is_ok() && std::fs::rename(&tmp, &*path).is_ok() {
                    *last = json;
                }
            }
        }
    }

    /// Publish the status (model, context, state) to `path` for the web MCP's status page.
    pub fn set_status_file(&self, path: std::path::PathBuf) {
        *self.status_file.lock().unwrap() = Some((path, String::new()));
        self.update_status(|_| {});
    }

    /// Redraw the header on every terminal; only writes when the content changed (or `force`).
    pub fn draw_status(&self, force: bool) {
        let st = self.status.lock().unwrap().clone();
        let m = crate::sys::meminfo();
        let up = uptime_secs() as u64;
        let mut outs = self.outs.lock().unwrap();
        let mut drawn = self.last_drawn.lock().unwrap();
        drawn.resize(outs.len(), String::new());
        for (i, o) in outs.iter_mut().enumerate() {
            if !o.tty {
                continue;
            }
            let cols = o.cols as usize;
            let l1 = fit(
                &format!(
                    " bia {} · {} · {} · {} {} · {} · up {:02}:{:02}:{:02} · {} tools",
                    st.version,
                    st.arch,
                    st.kernel,
                    st.model,
                    st.model_detail,
                    crate::clock::hms_now(),
                    up / 3600,
                    (up % 3600) / 60,
                    up % 60,
                    st.tools
                ),
                cols,
            );
            let mw = if cols >= 130 {
                14
            } else if cols >= 100 {
                7
            } else {
                5
            };
            let used = m.total.saturating_sub(m.available);
            let l2 = format!(
                " {}  {}  {}  {:.1} tok/s  msgs {}  {}",
                meter2("RAM", used, m.cached, m.total, mw, |v| gb(v).to_string()),
                meter(
                    "swap",
                    m.swap_total.saturating_sub(m.swap_free),
                    m.swap_total,
                    mw,
                    |v| gb(v).to_string()
                ),
                meter(
                    "ctx",
                    st.ctx_used as u64,
                    st.ctx_total as u64,
                    mw,
                    |v| format!("{v}")
                ),
                st.tps,
                st.messages,
                st.state,
            );
            let l2 = fit_ansi(&l2, cols);
            let busy = cpu_frac();
            let l3 = fit_ansi(
                &format!(
                    " {}  load {}  {} cpu{}  {}{}",
                    cpu_meter(busy, mw),
                    crate::sys::loadavg(),
                    crate::sys::ncpu(),
                    if crate::sys::ncpu() == 1 { "" } else { "s" },
                    status_ips(st.web_port),
                    if !st.web_login.is_empty() {
                        format!("  {}", st.web_login)
                    } else {
                        String::new()
                    }
                ),
                cols,
            );
            let frame = format!("{l1}\n{l2}\n{l3}");
            if !force && drawn[i] == frame {
                continue;
            }
            drawn[i] = frame.clone();
            write_status_snapshot(&frame);
            // kitty under X has no header; the web terminal (no fd) always draws it
            if o.fd.is_none() || std::env::var_os("BIA_GUI").is_none() {
                // save cursor, draw header lines, restore cursor
                let _ = write!(
                    o.w,
                    "\x1b7\x1b[1;1H{HEADER_BG}{:<cols$}\x1b[K\x1b[2;1H{HEADER_BG}{}\x1b[K\x1b[3;1H{HEADER_BG}{}\x1b[K{RESET}\x1b8",
                    l1, l2, l3
                );
            }
            let _ = o.w.flush();
        }
    }
}

fn write_status_snapshot(frame: &str) {
    let root = std::env::var("BIA_FILES").unwrap_or_else(|_| "/home/bia/files".into());
    let path = std::path::PathBuf::from(root)
        .join("state")
        .join("status.txt");
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, strip_ansi(frame));
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            for d in chars.by_ref() {
                if d == 'm' {
                    break;
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

fn gb(v: u64) -> String {
    format!("{:.1}G", v as f64 / 1e9)
}

/// htop-like meter: `label[||||    ] used/total`
fn meter(label: &str, used: u64, total: u64, width: usize, fmt: impl Fn(u64) -> String) -> String {
    let frac = if total == 0 {
        0.0
    } else {
        used as f64 / total as f64
    };
    let filled = ((frac * width as f64).round() as usize).min(width);
    let color = if frac > 0.85 {
        RED
    } else if frac > 0.6 {
        YELLOW
    } else {
        GREEN
    };
    format!(
        "{label}[{color}{}{HEADER_BG}{}] {}/{}",
        "|".repeat(filled),
        " ".repeat(width - filled),
        fmt(used),
        fmt(total)
    )
}

/// htop-like two-segment meter: green = used (anonymous), yellow = page cache (file-backed, e.g. model weights).
/// Label: `RAM[|||||////  ] used+cache/total`
fn meter2(
    label: &str,
    used: u64,
    cache: u64,
    total: u64,
    width: usize,
    fmt: impl Fn(u64) -> String,
) -> String {
    let f = |v: u64| {
        if total == 0 {
            0.0
        } else {
            v as f64 / total as f64
        }
    };
    let a = ((f(used) * width as f64).round() as usize).min(width);
    let b = ((f(cache) * width as f64).round() as usize).min(width - a);
    let color = if f(used) > 0.85 {
        RED
    } else if f(used) > 0.6 {
        YELLOW
    } else {
        GREEN
    };
    format!(
        "{label}[{color}{}{YELLOW}{}{HEADER_BG}{}] {}+{}/{}",
        "|".repeat(a),
        "/".repeat(b),
        " ".repeat(width - a - b),
        fmt(used),
        fmt(cache),
        fmt(total)
    )
}

fn status_ips(port: u16) -> String {
    let ips: Vec<(String, String)> = crate::sys::ipv4_addrs()
        .into_iter()
        .filter(|(_, ip)| !ip.starts_with("169.254."))
        .collect();
    if ips.is_empty() {
        return "no ip".into();
    }
    let one = |ip: &str| {
        if port != 0 && port != 80 && port != 443 {
            format!("{ip}:{port}")
        } else {
            ip.to_string()
        }
    };
    if ips.len() == 1 {
        format!("ip {}", one(&ips[0].1))
    } else {
        format!(
            "ip {}",
            ips.iter()
                .map(|(n, ip)| format!("{n}:{}", one(ip)))
                .collect::<Vec<_>>()
                .join(" ")
        )
    }
}

fn cpu_meter(frac: f32, width: usize) -> String {
    let frac = frac.clamp(0.0, 1.0);
    let filled = ((frac * width as f32).round() as usize).min(width);
    let color = if frac > 0.85 {
        RED
    } else if frac > 0.6 {
        YELLOW
    } else {
        GREEN
    };
    format!(
        "CPU[{color}{}{HEADER_BG}{}] {:>3.0}%",
        "|".repeat(filled),
        " ".repeat(width - filled),
        frac * 100.0
    )
}

static CPU_SAMPLE: Mutex<(u64, u64, f32)> = Mutex::new((0, 0, 0.0));

fn cpu_frac() -> f32 {
    let (idle, total) = crate::sys::cpu_times();
    let mut g = CPU_SAMPLE.lock().unwrap_or_else(|e| e.into_inner());
    let (pi, pt, prev) = *g;
    let df = total.saturating_sub(pt);
    let di = idle.saturating_sub(pi);
    let busy = if df == 0 || pt == 0 {
        prev
    } else {
        (1.0 - di as f32 / df as f32).clamp(0.0, 1.0)
    };
    *g = (idle, total, busy);
    busy
}

/// Terminal cells a character takes: 2 for CJK and emoji, 0 for combining marks.
fn cells(c: char) -> usize {
    unicode_width::UnicodeWidthChar::width(c).unwrap_or(0)
}

fn fit(s: &str, cols: usize) -> String {
    if s.chars().map(cells).sum::<usize>() <= cols {
        return s.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let w = cells(c);
        if used + w + 1 > cols {
            break;
        }
        out.push(c);
        used += w;
    }
    out + "…"
}

/// Pad/truncate a string containing ANSI sequences to `cols` terminal cells.
fn fit_ansi(s: &str, cols: usize) -> String {
    let mut out = String::new();
    let mut visible = 0;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            out.push(c);
            for d in chars.by_ref() {
                out.push(d);
                if d == 'm' {
                    break;
                }
            }
            continue;
        }
        let w = cells(c);
        if visible + w > cols {
            break;
        }
        out.push(c);
        visible += w;
    }
    out.push_str(&" ".repeat(cols.saturating_sub(visible)));
    out
}

fn spawn_reader<R: std::io::Read + Send + 'static>(r: R, tx: Sender<Inbound>, name: String) {
    std::thread::Builder::new()
        .name(format!("console-{name}"))
        .spawn(move || {
            let mut br = BufReader::new(r);
            loop {
                // Bytes, not read_line: one invalid UTF-8 sequence (a Latin-1 serial client)
                // must not drop the whole line.
                let mut buf = Vec::new();
                match br.read_until(b'\n', &mut buf) {
                    Ok(0) => {
                        if name == "stdin" {
                            let _ = tx.send(Inbound::console("/exit"));
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(500));
                    }
                    Ok(_) => {
                        let line = String::from_utf8_lossy(&buf);
                        let l = line.trim_end_matches(['\n', '\r']).to_string();
                        if tx.send(Inbound::console(l)).is_err() {
                            return;
                        }
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(500)),
                }
            }
        })
        .expect("spawn console reader");
}

/// Terminal modes to put back at exit, for the ttys `raw_input` changed.
static SAVED_TERMIOS: Mutex<Vec<(i32, libc::termios)>> = Mutex::new(vec![]);

extern "C" fn restore_termios() {
    if let Ok(v) = SAVED_TERMIOS.lock() {
        for (fd, t) in v.iter() {
            unsafe { libc::tcsetattr(*fd, libc::TCSANOW, t) };
        }
    }
}

/// Turn off the kernel's line editing and echo on a tty (signals and output processing stay).
/// The kernel erases one column per Backspace whatever the character's width, so a CJK
/// character or emoji left half of itself on screen; `LineEd` erases by display width.
fn raw_input(fd: i32) -> bool {
    unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        if libc::isatty(fd) != 1 || libc::tcgetattr(fd, &mut t) != 0 {
            return false;
        }
        let saved = t;
        t.c_iflag |= libc::IUTF8 | libc::ICRNL;
        t.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ECHOE | libc::ECHOK | libc::ECHONL);
        t.c_cc[libc::VMIN] = 1;
        t.c_cc[libc::VTIME] = 0;
        if libc::tcsetattr(fd, libc::TCSANOW, &t) != 0 {
            return false;
        }
        let mut v = SAVED_TERMIOS.lock().unwrap_or_else(|e| e.into_inner());
        if v.is_empty() {
            libc::atexit(restore_termios);
        }
        v.push((fd, saved));
        true
    }
}

/// Read a tty in raw mode, edit the line with `LineEd` and echo to `echo`.
/// Falls back to plain line reading when `fd` is not a tty (a pipe, a file).
fn spawn_tty_reader<R, W>(r: R, fd: i32, mut echo: W, tx: Sender<Inbound>, name: String)
where
    R: std::io::Read + Send + 'static,
    W: Write + Send + 'static,
{
    if !raw_input(fd) {
        return spawn_reader(r, tx, name);
    }
    std::thread::Builder::new()
        .name(format!("console-{name}"))
        .spawn(move || {
            let mut r = r;
            let mut ed = LineEd::default();
            let mut buf = [0u8; 256];
            loop {
                let n = match r.read(&mut buf) {
                    Ok(0) | Err(_) => {
                        std::thread::sleep(Duration::from_millis(500));
                        continue;
                    }
                    Ok(n) => n,
                };
                let mut out = Vec::new();
                for &b in &buf[..n] {
                    let line = match ed.feed(b, &mut out) {
                        Some(Edit::Line(l)) => l,
                        Some(Edit::Eof) if name == "stdin" => "/exit".to_string(),
                        Some(Edit::Eof) | None => continue,
                    };
                    let _ = echo.write_all(&out);
                    let _ = echo.flush();
                    out.clear();
                    if tx.send(Inbound::console(line)).is_err() {
                        return;
                    }
                }
                let _ = echo.write_all(&out);
                let _ = echo.flush();
            }
        })
        .expect("spawn console reader");
}

#[derive(Debug, PartialEq)]
enum Edit {
    Line(String),
    /// Ctrl+D on an empty line
    Eof,
}

/// Line editor for a raw tty: appends typed text and erases by display width, so
/// Backspace takes a whole CJK character or emoji (two cells) and a letter with its
/// combining marks. Ctrl+U clears the line, Ctrl+W the last word. Arrow and function
/// keys are ignored rather than typed as escape sequences.
#[derive(Default)]
struct LineEd {
    /// what was typed, one entry per erasable unit, with the cells it was echoed as
    units: Vec<(String, usize)>,
    /// bytes of a UTF-8 character still being read
    pending: Vec<u8>,
    esc: Esc,
}

#[derive(Default, PartialEq)]
enum Esc {
    #[default]
    None,
    Start,
    /// inside `ESC [` or `ESC O`, until the final byte
    Seq,
}

impl LineEd {
    fn feed(&mut self, b: u8, echo: &mut Vec<u8>) -> Option<Edit> {
        match self.esc {
            Esc::Start => {
                self.esc = if b == b'[' || b == b'O' {
                    Esc::Seq
                } else {
                    Esc::None
                };
                return None;
            }
            Esc::Seq => {
                if (0x40..=0x7e).contains(&b) {
                    self.esc = Esc::None;
                }
                return None;
            }
            Esc::None => {}
        }
        if b >= 0x80 || !self.pending.is_empty() {
            if b < 0x80 {
                self.pending.clear(); // a broken sequence; take this byte on its own
            } else {
                self.pending.push(b);
                match std::str::from_utf8(&self.pending) {
                    Ok(s) => {
                        let c = s.chars().next().unwrap_or('\u{fffd}');
                        self.pending.clear();
                        self.push(c, echo);
                    }
                    Err(e) if e.error_len().is_some() => self.pending.clear(),
                    Err(_) => {}
                }
                return None;
            }
        }
        match b {
            b'\r' | b'\n' => {
                echo.push(b'\n'); // the tty's ONLCR makes it \r\n, as for all console output
                let line: String = self.units.drain(..).map(|(s, _)| s).collect();
                return Some(Edit::Line(line));
            }
            0x7f | 0x08 => self.erase(1, echo),
            0x15 => self.erase(self.units.len(), echo),
            0x17 => {
                let blanks = self
                    .units
                    .iter()
                    .rev()
                    .take_while(|(s, _)| s.trim().is_empty())
                    .count();
                let word = self.units[..self.units.len() - blanks]
                    .iter()
                    .rev()
                    .take_while(|(s, _)| !s.trim().is_empty())
                    .count();
                self.erase(blanks + word, echo);
            }
            0x04 if self.units.is_empty() => return Some(Edit::Eof),
            0x1b => self.esc = Esc::Start,
            b'\t' => self.push(' ', echo),
            0x20..=0x7e => self.push(b as char, echo),
            _ => {}
        }
        None
    }

    fn push(&mut self, c: char, echo: &mut Vec<u8>) {
        let mut tmp = [0u8; 4];
        let joins = cells(c) == 0
            || self
                .units
                .last()
                .is_some_and(|(s, _)| s.ends_with('\u{200d}'));
        match self.units.last_mut() {
            // A combining mark, variation selector or the second half of a ZWJ emoji
            // belongs to the unit before it; its width is that of the whole cluster.
            Some((s, w)) if joins => {
                s.push(c);
                let now = unicode_width::UnicodeWidthStr::width(s.as_str());
                if now > *w {
                    *w = now;
                }
            }
            _ => self.units.push((c.to_string(), cells(c))),
        }
        echo.extend_from_slice(c.encode_utf8(&mut tmp).as_bytes());
    }

    fn erase(&mut self, n: usize, echo: &mut Vec<u8>) {
        for _ in 0..n {
            let Some((_, w)) = self.units.pop() else {
                break;
            };
            for _ in 0..w {
                echo.extend_from_slice(b"\x08 \x08");
            }
        }
    }
}

/// Devices behind the kernel consoles listed in /proc/consoles.
/// `tty0` (the VT system) is mapped to /dev/tty1, the first virtual terminal.
fn is_vt(dev: &str) -> bool {
    dev == "/dev/tty0" || dev == "/dev/tty1" || dev == "/dev/console"
}

fn console_devices() -> Vec<String> {
    let mut v = vec![];
    if let Ok(s) = std::fs::read_to_string("/proc/consoles") {
        for line in s.lines() {
            let Some(name) = line.split_whitespace().next() else {
                continue;
            };
            let flags = line.split_whitespace().nth(1).unwrap_or("");
            let dev = if name == "tty0" {
                "/dev/tty1".to_string()
            } else {
                format!("/dev/{name}")
            };
            if name == "tty0"
                || flags.contains('R')
                || name.starts_with("ttyS")
                || name.starts_with("ttyAMA")
            {
                v.push(dev);
            }
        }
    }
    if v.is_empty() {
        v.push("/dev/console".into());
    }
    v
}

/// Boot-progress printer: "[ 42%] message", with in-place updates for bars.
pub struct Boot {
    con: Arc<Console>,
    inline: Mutex<bool>,
}

impl Boot {
    pub fn new(con: Arc<Console>) -> Boot {
        Boot {
            con,
            inline: Mutex::new(false),
        }
    }
    pub fn step(&self, pct: u32, msg: &str) {
        self.end_inline();
        self.con.writeln(&format!(
            "{} {msg}",
            self.con.c(BOLD, &format!("[{pct:3}%]"))
        ));
        let short: String = msg.chars().take(40).collect();
        self.con
            .update_status(|s| s.state = format!("boot {pct}% {short}"));
    }
    /// Update the current line in place (progress bars).
    pub fn bar(&self, pct: u32, label: &str, frac: f32) {
        let width = 30usize;
        let filled = ((frac.clamp(0.0, 1.0) * width as f32) as usize).min(width);
        let bar: String = "=".repeat(filled) + &" ".repeat(width - filled);
        self.con.write(&format!(
            "\r{} {label} [{}] {:3}%",
            self.con.c(BOLD, &format!("[{pct:3}%]")),
            self.con.c(GREEN, &bar),
            (frac * 100.0) as u32
        ));
        *self.inline.lock().unwrap() = true;
    }
    pub fn end_inline(&self) {
        let mut i = self.inline.lock().unwrap();
        if *i {
            self.con.write("\n");
            *i = false;
        }
    }
    pub fn note(&self, msg: &str) {
        self.end_inline();
        self.con
            .writeln(&format!("       {}", self.con.c(GRAY, msg)));
    }
    pub fn warn(&self, msg: &str) {
        self.end_inline();
        self.con
            .writeln(&format!("  {}   {msg}", self.con.c(RED, "!!")));
    }
}

pub fn uptime_secs() -> f64 {
    std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|s| s.split_whitespace().next().and_then(|v| v.parse().ok()))
        .unwrap_or(0.0)
}

#[cfg(test)]
mod journal_tests {
    use super::*;

    #[test]
    fn replays_only_what_the_model_did_not_take() {
        let dir = std::env::temp_dir().join(format!("bia-journal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("inbox.jsonl");
        let con = Console::open(false);
        assert!(con.open_journal(path.clone()).is_empty());
        let msg = |id: &str, ch: &str, who: &str| Inbound {
            identity: who.into(),
            channel: ch.into(),
            issued: std::time::SystemTime::now(),
            id: id.into(),
            text: format!("text {id}"),
        };
        con.push_inbound(msg("1", "telegram:5", "alice"));
        con.push_inbound(msg("2", "web:c1", "root"));
        con.push_inbound(msg("3", "telegram:5", "system"));
        con.push_inbound(msg("4", "console", "root"));
        con.journal_done("1");
        // a restart: only 2 is left (system and console are not journaled)
        let con2 = Console::open(false);
        let left = con2.open_journal(path.clone());
        assert_eq!(
            left.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
            ["2"]
        );
        assert_eq!(left[0].channel, "web:c1");
        con2.journal_done("2");
        assert!(Console::open(false).open_journal(path).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod width_tests {
    use super::*;

    #[test]
    fn fit_counts_terminal_cells() {
        assert_eq!(fit("abc", 5), "abc");
        // 中文 is 4 cells: fits in 4, not in 3
        assert_eq!(fit("中文", 4), "中文");
        assert_eq!(fit("中文", 3), "中…");
        assert_eq!(fit("😀😀😀", 5), "😀😀…");
    }

    #[test]
    fn fit_ansi_pads_wide_chars_to_cells() {
        let s = fit_ansi("\x1b[1m中\x1b[0mx", 5);
        assert_eq!(s, "\x1b[1m中\x1b[0mx  ");
        // a wide char that would straddle the edge is dropped, the cell padded
        assert_eq!(fit_ansi("ab中", 3), "ab ");
    }
}

#[cfg(test)]
mod line_tests {
    use super::*;

    fn typed(input: impl AsRef<[u8]>) -> (Vec<Edit>, String) {
        let mut ed = LineEd::default();
        let mut echo = Vec::new();
        let edits = input
            .as_ref()
            .iter()
            .copied()
            .filter_map(|b| ed.feed(b, &mut echo))
            .collect();
        (edits, String::from_utf8_lossy(&echo).into_owned())
    }

    #[test]
    fn backspace_erases_by_width() {
        // 3 characters, 2 Backspaces: one left, and 2 cells erased for each CJK character
        let (e, echo) = typed("中文字\x7f\x7f\r");
        assert_eq!(e, [Edit::Line("中".into())]);
        assert!(echo.ends_with("字\x08 \x08\x08 \x08\x08 \x08\x08 \x08\n"));
        let (e, _) = typed("привет\x7f!\r");
        assert_eq!(e, [Edit::Line("приве!".into())]);
        // e + combining acute is one unit
        let (e, echo) = typed("cafe\u{301}\x7f\r");
        assert_eq!(e, [Edit::Line("caf".into())]);
        assert_eq!(echo.matches('\x08').count(), 2);
        // emoji with a variation selector and a ZWJ family: one Backspace each
        let (e, _) = typed("a❤\u{fe0f}👨\u{200d}👩\x7f\x7f\r");
        assert_eq!(e, [Edit::Line("a".into())]);
    }

    #[test]
    fn line_keys() {
        let (e, _) = typed("one two  \x17three\r");
        assert_eq!(e, [Edit::Line("one three".into())]);
        let (e, _) = typed("junk\x15ok\r");
        assert_eq!(e, [Edit::Line("ok".into())]);
        // arrows and F5 are not typed; a broken UTF-8 byte is dropped
        let (e, echo) = typed(b"a\x1b[A\x1b[15~b\xffc\r\x04");
        assert_eq!(e, [Edit::Line("abc".into()), Edit::Eof]);
        assert!(!echo.contains('\x1b'));
        let (e, _) = typed("x\x04\x7f\x7f\r");
        assert_eq!(e, [Edit::Line("".into())]);
    }

    #[test]
    fn edits_on_a_real_pty() {
        use std::io::Read;
        use std::os::unix::io::FromRawFd;
        let (mut m, mut sl) = (0, 0);
        let ok = unsafe {
            libc::openpty(
                &mut m,
                &mut sl,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
            )
        };
        assert_eq!(ok, 0);
        let mut master = unsafe { std::fs::File::from_raw_fd(m) };
        let slave = unsafe { std::fs::File::from_raw_fd(sl) };
        let echo = slave.try_clone().unwrap();
        let (tx, rx) = channel();
        spawn_tty_reader(slave, sl, echo, tx, "pty".into());
        master.write_all("中文\x7fok\r".as_bytes()).unwrap();
        let ib = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(ib.text, "中ok");
        let mut buf = [0u8; 64];
        let n = master.read(&mut buf).unwrap();
        // no kernel echo: only ours, two cells erased for 文
        assert_eq!(&buf[..n], "中文\x08 \x08\x08 \x08ok\r\n".as_bytes());
    }
}

#[cfg(test)]
mod mirror_tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn mirror_replays_streams_and_types() {
        let dir = std::env::temp_dir().join(format!("bia-mirror-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("console.sock");
        let con = Console::for_test();
        con.write("before connect\n");
        con.serve_mirror(path.clone()).unwrap();
        let mut s = std::os::unix::net::UnixStream::connect(&path).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        writeln!(s, r#"{{"rows":24,"cols":80}}"#).unwrap();
        let mut got = Vec::new();
        let mut buf = [0u8; 4096];
        let mut wait_for = |s: &mut std::os::unix::net::UnixStream, needle: &str| {
            while !String::from_utf8_lossy(&got).contains(needle) {
                let n = s.read(&mut buf).expect("mirror output");
                assert!(n > 0, "mirror closed");
                got.extend_from_slice(&buf[..n]);
            }
        };
        wait_for(&mut s, "before connect");
        con.write("after connect\n");
        wait_for(&mut s, "after connect");
        writeln!(s, r#"{{"t":"line","v":"/status"}}"#).unwrap();
        wait_for(&mut s, "/status");
        let ib = con
            .read_inbound_timeout(Duration::from_secs(5))
            .unwrap()
            .expect("typed line");
        assert_eq!(
            (ib.identity.as_str(), ib.channel.as_str(), ib.text.as_str()),
            (crate::tools::ID_ROOT, crate::tools::CH_CONSOLE, "/status")
        );
        drop(s);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
