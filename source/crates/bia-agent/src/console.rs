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
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        match v.get("op").and_then(|x| x.as_str()) {
            Some("in") => ins.push(Inbound {
                identity: s("identity"),
                channel: s("channel"),
                issued: std::time::UNIX_EPOCH
                    + std::time::Duration::from_secs(v.get("issued").and_then(|x| x.as_u64()).unwrap_or(0)),
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
    ins.into_iter().filter(|ib| !done.contains(&ib.id) && seen.insert(ib.id.clone())).collect()
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
    id: usize,
    w: Box<dyn Write + Send>,
    /// Tty fd for `TIOCGWINSZ`. `None` for the web sink.
    fd: Option<i32>,
    rows: u16,
    cols: u16,
    tty: bool,
    /// browser chat/logs: status is a JSON overlay, not DECSC/CUP
    web: bool,
    /// Client-cert identity for web sinks (`None` on physical consoles).
    web_identity: Option<String>,
}

/// Writer that forwards bytes to a web console client.
struct ChannelWriter {
    tx: Sender<Vec<u8>>,
}
impl Write for ChannelWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.tx
            .send(buf.to_vec())
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "web client gone"))?;
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

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
    /// web console login code (empty if the web server is off)
    pub web_code: String,
    pub web_port: u16,
    /// one-click login URLs (`http://ip/?code=`), space-separated
    pub web_login: String,
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
    /// recent output, replayed to a web client when it connects
    recent: Mutex<VecDeque<u8>>,
    next_id: Mutex<usize>,
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
                        id: outs.len(),
                        w: Box::new(f),
                        fd: Some(fd),
                        rows,
                        cols,
                        tty: true,
                        web: false,
                        web_identity: None,
                    });
                    if let Ok(r) = OpenOptions::new().read(true).open(&dev) {
                        spawn_reader(r, tx.clone(), dev.clone());
                    }
                    devices.push(dev);
                }
            }
        }
        if gui || outs.is_empty() {
            let tty = unsafe { libc::isatty(1) } == 1;
            let (rows, cols) = if tty { winsize(1) } else { (0, 0) };
            outs.push(Out {
                id: outs.len(),
                w: Box::new(std::io::stdout()),
                fd: tty.then_some(1),
                rows,
                cols,
                tty,
                web: false,
                web_identity: None,
            });
            spawn_reader(std::io::stdin(), tx.clone(), "stdin".into());
            devices.push("stdio".into());
        }
        let color = outs.iter().any(|o| o.tty);
        let n = outs.len();
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
            recent: Mutex::new(VecDeque::with_capacity(REPLAY_BYTES)),
            next_id: Mutex::new(n),
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
            if o.tty && !o.web {
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

    /// A browser chat session of this identity is connected.
    pub fn has_web(&self, identity: &str) -> bool {
        self.outs
            .lock()
            .map(|o| o.iter().any(|o| o.web && o.web_identity.as_deref() == Some(identity)))
            .unwrap_or(false)
    }

    /// JSON event for the browser chat sinks of one client-cert identity (see `webchat`).
    pub fn web_event(&self, identity: &str, payload: &str) {
        let mut pkt = Vec::with_capacity(1 + payload.len());
        pkt.push(0x1d);
        pkt.extend_from_slice(payload.as_bytes());
        let mut outs = self.outs.lock().unwrap();
        for o in outs.iter_mut() {
            if o.web && o.web_identity.as_deref() == Some(identity) {
                let _ = o.w.write_all(&pkt);
                let _ = o.w.flush();
            }
        }
    }

    pub fn write(&self, s: &str) {
        {
            let mut r = self.recent.lock().unwrap();
            for b in s.bytes() {
                if r.len() >= REPLAY_BYTES {
                    r.pop_front();
                }
                r.push_back(b);
            }
        }
        let mut outs = self.outs.lock().unwrap();
        for o in outs.iter_mut() {
            // Chat sinks get JSON events only; the console stream goes to TTYs and root's web terminal.
            if o.web {
                continue;
            }
            let _ = o.w.write_all(s.as_bytes());
            let _ = o.w.flush();
        }
    }

    // ---- web console sinks ----

    fn next_sink_id(&self) -> usize {
        let mut n = self.next_id.lock().unwrap();
        *n += 1;
        *n
    }

    /// Register a browser chat session; it receives `web_event` JSON (0x1d-prefixed) only.
    pub fn add_web_sink(&self, identity: &str) -> (usize, Receiver<Vec<u8>>) {
        let (tx, rx) = channel::<Vec<u8>>();
        let id = self.next_sink_id();
        self.outs.lock().unwrap().push(Out {
            id,
            w: Box::new(ChannelWriter { tx }),
            fd: None,
            rows: 0,
            cols: 0,
            tty: false,
            web: true,
            web_identity: Some(identity.to_string()),
        });
        (id, rx)
    }

    /// Register root's browser terminal: a full console stream like a physical TTY
    /// (status header, scroll region, colors). Recent output is replayed first.
    pub fn add_term_sink(&self, rows: u16, cols: u16) -> (usize, Receiver<Vec<u8>>) {
        let (tx, rx) = channel::<Vec<u8>>();
        let id = self.next_sink_id();
        let rows = rows.max(HEADER_LINES + 2);
        let cols = cols.max(20);
        let mut w = ChannelWriter { tx };
        let replay: Vec<u8> = self.recent.lock().unwrap().iter().copied().collect();
        let _ = write!(
            w,
            "\x1b[0m\x1b[2J\x1b[{};{}r\x1b[{};1H",
            HEADER_LINES + 1,
            rows,
            HEADER_LINES + 1
        );
        let _ = w.write_all(&replay);
        self.outs.lock().unwrap().push(Out {
            id,
            w: Box::new(w),
            fd: None,
            rows,
            cols,
            tty: true,
            web: false,
            web_identity: Some(crate::tools::ID_ROOT.into()),
        });
        self.last_drawn.lock().unwrap().clear();
        self.draw_status(true);
        (id, rx)
    }
    pub fn remove_sink(&self, id: usize) {
        self.outs.lock().unwrap().retain(|o| o.id != id);
        self.last_drawn.lock().unwrap().clear();
    }
    pub fn resize_sink(&self, id: usize, rows: u16, cols: u16) {
        let mut outs = self.outs.lock().unwrap();
        if let Some(o) = outs.iter_mut().find(|o| o.id == id) {
            o.rows = rows.max(if o.web { 2 } else { HEADER_LINES + 2 });
            o.cols = cols.max(20);
            if !o.web {
                let _ = write!(o.w, "\x1b[{};{}r", HEADER_LINES + 1, o.rows);
            }
        }
        drop(outs);
        self.last_drawn.lock().unwrap().clear();
        self.draw_status(true);
    }
    pub fn send_to_sink(&self, id: usize, bytes: &[u8]) -> bool {
        let mut outs = self.outs.lock().unwrap();
        match outs.iter_mut().find(|o| o.id == id) {
            Some(o) => o.w.write_all(bytes).is_ok(),
            None => false,
        }
    }
    /// A line typed on a client terminal (root's web terminal → channel `console`).
    /// Echoed to every other console output, then queued like a physical keystroke line.
    pub fn push_input_from(
        &self,
        identity: String,
        channel: String,
        line: String,
        origin: Option<usize>,
    ) {
        let echo = format!(
            "{}\n",
            if self.color {
                format!("{GREEN}{line}{RESET}")
            } else {
                line.clone()
            }
        );
        {
            let mut outs = self.outs.lock().unwrap();
            for o in outs.iter_mut() {
                if Some(o.id) != origin && !o.web {
                    let _ = o.w.write_all(echo.as_bytes());
                    let _ = o.w.flush();
                }
            }
        }
        let _ = self.tx.send(
            Inbound {
                identity,
                channel,
                issued: SystemTime::now(),
                id: String::new(),
                text: line,
            }
            .ensure_id(),
        );
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
            if std::fs::metadata(&p).map(|m| m.len() > 1_000_000).unwrap_or(false) {
                let _ = self.open_journal(p);
            }
        }
    }

    fn journal_append(&self, v: &serde_json::Value) {
        use std::io::Write;
        let g = self.journal.lock().unwrap();
        let Some(p) = g.as_ref() else { return };
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
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

    pub fn status(&self) -> StatusData {
        self.status.lock().unwrap().clone()
    }

    pub fn update_status(&self, f: impl FnOnce(&mut StatusData)) {
        f(&mut self.status.lock().unwrap());
        self.draw_status(false);
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
            if o.web {
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
                meter2("RAM", used, m.cached, m.total, mw, |v| format!("{}", gb(v))),
                meter(
                    "swap",
                    m.swap_total.saturating_sub(m.swap_free),
                    m.swap_total,
                    mw,
                    |v| format!("{}", gb(v))
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

fn fit(s: &str, cols: usize) -> String {
    let n = s.chars().count();
    if n > cols {
        s.chars().take(cols.saturating_sub(1)).collect::<String>() + "…"
    } else {
        s.to_string()
    }
}

/// Pad/truncate a string containing ANSI sequences to `cols` visible characters.
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
        if visible >= cols {
            break;
        }
        out.push(c);
        visible += 1;
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
                let mut line = String::new();
                match br.read_line(&mut line) {
                    Ok(0) => {
                        if name == "stdin" {
                            let _ = tx.send(Inbound::console("/exit"));
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(500));
                    }
                    Ok(_) => {
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
        assert_eq!(left.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["2"]);
        assert_eq!(left[0].channel, "web:c1");
        con2.journal_done("2");
        assert!(Console::open(false).open_journal(path).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
