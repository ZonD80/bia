//! Shell jobs: run a command on a PTY as root or as `bia`, capture output, honour cancel.
//! After 30s with no PTY output the model gets a non-terminal `stale?` notice on `job:<id>`;
//! `job_input` writes to the PTY so interactive prompts can continue.
use crate::console::Console;
use crate::jobs;
use serde_json::Value;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::fd::{FromRawFd, RawFd};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const OUT_CAP: usize = 64 * 1024;
const STALE_SECS: u64 = 30;
const STALE_POLL: Duration = Duration::from_secs(5);

#[derive(Clone)]
struct PtySlot {
    master: Arc<Mutex<std::fs::File>>,
    stale_sent: Arc<AtomicBool>,
    alive: Arc<AtomicBool>,
}

static PTYS: Mutex<Option<HashMap<String, PtySlot>>> = Mutex::new(None);

fn ptys() -> std::sync::MutexGuard<'static, Option<HashMap<String, PtySlot>>> {
    PTYS.lock().unwrap_or_else(|e| e.into_inner())
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn arg_str<'a>(args: &'a Value, k: &str) -> Option<&'a str> {
    args.get(k).and_then(|v| v.as_str())
}

fn user_ids(name: &str) -> Result<(u32, u32), String> {
    let c = std::ffi::CString::new(name).map_err(|_| "user name contains NUL".to_string())?;
    unsafe {
        let pw = libc::getpwnam(c.as_ptr());
        if pw.is_null() {
            return Err(format!("no such user {name}"));
        }
        Ok(((*pw).pw_uid, (*pw).pw_gid))
    }
}

pub(crate) fn open_pty() -> Result<(RawFd, RawFd), String> {
    unsafe {
        #[cfg(target_os = "linux")]
        {
            let master = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY);
            if master < 0 {
                return Err(format!("posix_openpt: {}", std::io::Error::last_os_error()));
            }
            if libc::grantpt(master) != 0 {
                let e = std::io::Error::last_os_error();
                libc::close(master);
                return Err(format!("grantpt: {e}"));
            }
            if libc::unlockpt(master) != 0 {
                let e = std::io::Error::last_os_error();
                libc::close(master);
                return Err(format!("unlockpt: {e}"));
            }
            let mut buf = [0 as libc::c_char; 128];
            if libc::ptsname_r(master, buf.as_mut_ptr(), buf.len()) != 0 {
                let e = std::io::Error::last_os_error();
                libc::close(master);
                return Err(format!("ptsname_r: {e}"));
            }
            let slave = libc::open(buf.as_ptr(), libc::O_RDWR | libc::O_NOCTTY);
            if slave < 0 {
                let e = std::io::Error::last_os_error();
                libc::close(master);
                return Err(format!("open pts: {e}"));
            }
            let ws = libc::winsize {
                ws_row: 24,
                ws_col: 80,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };
            let _ = libc::ioctl(master, libc::TIOCSWINSZ, &ws);
            Ok((master, slave))
        }
        #[cfg(not(target_os = "linux"))]
        {
            let mut master = 0;
            let mut slave = 0;
            if libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            ) != 0
            {
                return Err(format!("openpty: {}", std::io::Error::last_os_error()));
            }
            let ws = libc::winsize {
                ws_row: 24,
                ws_col: 80,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };
            let _ = libc::ioctl(master, libc::TIOCSWINSZ, &ws);
            Ok((master, slave))
        }
    }
}

fn clip(s: &str, n: usize) -> String {
    if s.len() <= n {
        return s.to_string();
    }
    let mut end = n.min(s.len());
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n...[truncated, {} bytes]", &s[..end], s.len())
}

fn touch_out(last_out: &AtomicU64, stale_sent: &AtomicBool) {
    last_out.store(now_secs(), Ordering::Relaxed);
    stale_sent.store(false, Ordering::Relaxed);
}

fn pump_pty(
    mut master: impl Read,
    log: std::path::PathBuf,
    acc: Arc<Mutex<Vec<u8>>>,
    last_out: Arc<AtomicU64>,
    stale_sent: Arc<AtomicBool>,
) {
    let mut buf = [0u8; 4096];
    loop {
        match master.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                touch_out(&last_out, &stale_sent);
                if let Ok(mut a) = acc.lock() {
                    a.extend_from_slice(&buf[..n]);
                }
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&log)
                {
                    let _ = f.write_all(&buf[..n]);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
}

/// True if a process in `pgid` looks blocked in a read (via /proc, not ptrace).
fn pgid_waiting_on_read(pgid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        let Ok(dir) = std::fs::read_dir("/proc") else {
            return false;
        };
        for ent in dir.flatten() {
            let name = ent.file_name();
            let Some(pid) = name.to_str().and_then(|s| s.parse::<u32>().ok()) else {
                continue;
            };
            let stat = match std::fs::read_to_string(ent.path().join("stat")) {
                Ok(s) => s,
                Err(_) => continue,
            };
            if proc_pgrp(&stat) != Some(pgid) {
                continue;
            }
            if proc_waiting_on_read(pid) {
                return true;
            }
        }
        false
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pgid;
        false
    }
}

#[cfg(target_os = "linux")]
fn proc_pgrp(stat: &str) -> Option<u32> {
    let close = stat.rfind(')')?;
    let rest = stat.get(close + 2..)?;
    let mut fields = rest.split_whitespace();
    let _state = fields.next()?;
    let _ppid = fields.next()?;
    fields.next()?.parse().ok()
}

#[cfg(target_os = "linux")]
fn proc_waiting_on_read(pid: u32) -> bool {
    let base = format!("/proc/{pid}");
    if let Ok(wchan) = std::fs::read_to_string(format!("{base}/wchan")) {
        let w = wchan.trim();
        if w != "0"
            && !w.is_empty()
            && (w.contains("tty") || w.contains("n_tty") || w.contains("read"))
        {
            return true;
        }
    }
    if let Ok(sc) = std::fs::read_to_string(format!("{base}/syscall")) {
        let mut parts = sc.split_whitespace();
        if let Some(nr) = parts.next() {
            // x86_64/aarch64: read=0, readv=19, recvfrom=45, recvmsg=47
            if matches!(nr, "0" | "19" | "45" | "47") {
                return true;
            }
        }
    }
    false
}

fn stale_notice(jid: &str, waiting: bool) -> String {
    let wait = if waiting {
        "A process in the job looks blocked reading the PTY (likely a prompt)."
    } else {
        "No process looked blocked on a read; it may be quiet work, or waiting without polling."
    };
    format!(
        "stale?\nbash job:{jid} is still running — no PTY output for {STALE_SECS}s.\n{wait}\nThis is not the job result. Check job_logs, submit input with job_input(id, text) (include a trailing newline for Enter), interrupt it with Ctrl-C via job_input(id, \"\\u0003\"), or job_kill(id) to stop it hard."
    )
}

fn watch_stale(
    jid: String,
    pid: u32,
    jobs: Arc<Mutex<jobs::Store>>,
    con: Arc<Console>,
    last_out: Arc<AtomicU64>,
    stale_sent: Arc<AtomicBool>,
    alive: Arc<AtomicBool>,
) {
    let started = Instant::now();
    while alive.load(Ordering::Relaxed) {
        std::thread::sleep(STALE_POLL);
        if !alive.load(Ordering::Relaxed) {
            break;
        }
        let last = last_out.load(Ordering::Relaxed);
        let quiet = now_secs().saturating_sub(last);
        if quiet < STALE_SECS || started.elapsed() < Duration::from_secs(STALE_SECS) {
            continue;
        }
        if stale_sent.swap(true, Ordering::Relaxed) {
            continue;
        }
        {
            let st = jobs.lock().unwrap_or_else(|e| e.into_inner());
            let running = st
                .get(&jid)
                .map(|j| j.state == "running" || j.state == "killing")
                .unwrap_or(false);
            if !running {
                break;
            }
        }
        let waiting = pgid_waiting_on_read(pid);
        let body = stale_notice(&jid, waiting);
        {
            let st = jobs.lock().unwrap_or_else(|e| e.into_inner());
            st.append_log(&jid, "stale? notice issued to model");
        }
        con.push_inbound(jobs::inbound(&jid, &body));
    }
}

/// SIGKILL a bash job: its process group, then anything left in its session
/// (setsid gives the job pgid = sid = pid; children that started their own
/// process group would otherwise survive and hold the PTY open).
pub fn kill_tree(pid: u32) {
    if pid == 0 {
        return;
    }
    let _ = unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
    let Ok(rd) = std::fs::read_dir("/proc") else {
        return;
    };
    for e in rd.flatten() {
        let Some(p) = e.file_name().to_str().and_then(|s| s.parse::<i32>().ok()) else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{p}/stat")) else {
            continue;
        };
        // fields after "(comm)": state ppid pgrp session …
        let rest = stat.rsplit_once(')').map(|(_, r)| r).unwrap_or("");
        let session = rest
            .split_whitespace()
            .nth(3)
            .and_then(|s| s.parse::<u32>().ok());
        if session == Some(pid) {
            let _ = unsafe { libc::kill(p, libc::SIGKILL) };
        }
    }
}

/// Write bytes to a running bash job's PTY. `text` is sent as-is (include `\n` for Enter).
pub fn job_input(id: &str, text: &str) -> Result<String, String> {
    let id = id.trim().trim_start_matches(jobs::PREFIX);
    if id.is_empty() {
        return Err("id required".into());
    }
    if text.is_empty() {
        return Err("text required".into());
    }
    let slot = {
        let guard = ptys();
        guard
            .as_ref()
            .and_then(|m| m.get(id).cloned())
            .ok_or_else(|| format!("job {id} has no PTY (not a running bash job)"))?
    };
    let mut f = slot
        .master
        .lock()
        .map_err(|_| "PTY lock poisoned".to_string())?;
    f.write_all(text.as_bytes())
        .map_err(|e| format!("write PTY: {e}"))?;
    let _ = f.flush();
    // Allow another stale notice if the job goes quiet again after this input.
    slot.stale_sent.store(false, Ordering::Relaxed);
    Ok(format!("ok: wrote {} byte(s) to job:{id} PTY", text.len()))
}

fn register_pty(id: &str, slot: PtySlot) {
    let mut guard = ptys();
    let map = guard.get_or_insert_with(HashMap::new);
    map.insert(id.to_string(), slot);
}

fn unregister_pty(id: &str) {
    let mut guard = ptys();
    if let Some(map) = guard.as_mut() {
        if let Some(slot) = map.remove(id) {
            slot.alive.store(false, Ordering::Relaxed);
        }
    }
}

/// Spawn `sh -lc command` on a PTY in its own session. Returns the pid.
pub fn start(
    args: &Value,
    log_path: &Path,
    jobs: Arc<Mutex<jobs::Store>>,
    jid: String,
    con: Arc<Console>,
) -> Result<u32, String> {
    let command = arg_str(args, "command").unwrap_or("").trim();
    if command.is_empty() {
        return Err("command is required".into());
    }
    let user = arg_str(args, "user").unwrap_or("root").trim().to_string();
    let cwd_arg = arg_str(args, "cwd").unwrap_or("").trim();
    let timeout = args
        .get("timeout_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(600);
    let cwd = if cwd_arg.is_empty() {
        if Path::new("/home/bia").is_dir() {
            "/home/bia".into()
        } else {
            "/".into()
        }
    } else {
        cwd_arg.to_string()
    };

    let (master_fd, slave_fd) = open_pty()?;
    let ids = if user != "root" && !user.is_empty() {
        Some(user_ids(&user)?)
    } else {
        None
    };

    let mut cmd = Command::new("/bin/sh");
    cmd.arg("-lc")
        .arg(command)
        .current_dir(&cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    unsafe {
        cmd.pre_exec(move || {
            if libc::close(master_fd) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            let _ = libc::ioctl(slave_fd, libc::TIOCSCTTY as _, 0);
            if libc::dup2(slave_fd, 0) < 0
                || libc::dup2(slave_fd, 1) < 0
                || libc::dup2(slave_fd, 2) < 0
            {
                return Err(std::io::Error::last_os_error());
            }
            if slave_fd > 2 {
                let _ = libc::close(slave_fd);
            }
            if let Some((uid, gid)) = ids {
                if libc::setgid(gid) != 0 || libc::setuid(uid) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            unsafe {
                libc::close(master_fd);
                libc::close(slave_fd);
            }
            return Err(format!("spawn: {e}"));
        }
    };
    unsafe {
        libc::close(slave_fd);
    }
    let pid = child.id();
    let master = unsafe { std::fs::File::from_raw_fd(master_fd) };
    let reader = match master.try_clone() {
        Ok(r) => r,
        Err(e) => {
            let _ = unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
            let _ = child.wait();
            return Err(format!("dup PTY master: {e}"));
        }
    };

    let last_out = Arc::new(AtomicU64::new(now_secs()));
    let stale_sent = Arc::new(AtomicBool::new(false));
    let alive = Arc::new(AtomicBool::new(true));
    let master_arc = Arc::new(Mutex::new(master));

    register_pty(
        &jid,
        PtySlot {
            master: master_arc,
            stale_sent: stale_sent.clone(),
            alive: alive.clone(),
        },
    );

    {
        let mut st = jobs.lock().unwrap_or_else(|e| e.into_inner());
        st.set_pid(&jid, pid);
        st.append_log(&jid, &format!("pid {pid} user={user} cwd={cwd} pty"));
        st.append_log(&jid, &format!("$ {command}"));
    }

    let acc = Arc::new(Mutex::new(Vec::new()));
    let log = log_path.to_path_buf();
    let (pump_tx, pump_done) = std::sync::mpsc::channel::<()>();
    {
        let acc = acc.clone();
        let last_out = last_out.clone();
        let stale_sent = stale_sent.clone();
        std::thread::spawn(move || {
            pump_pty(reader, log, acc, last_out, stale_sent);
            let _ = pump_tx.send(());
        });
    }

    {
        let jobs_t = jobs.clone();
        let jid_t = jid.clone();
        let con_t = con.clone();
        let last_out = last_out.clone();
        let stale_sent = stale_sent.clone();
        let alive_t = alive.clone();
        let _ = std::thread::Builder::new()
            .name(format!("bash-stale-{jid_t}"))
            .spawn(move || {
                watch_stale(jid_t, pid, jobs_t, con_t, last_out, stale_sent, alive_t);
            });
    }

    if timeout > 0 {
        let jobs_t = jobs.clone();
        let jid_t = jid.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(timeout));
            let st = jobs_t.lock().unwrap_or_else(|e| e.into_inner());
            if st
                .get(&jid_t)
                .map(|j| j.state == "running" || j.state == "killing")
                .unwrap_or(false)
            {
                drop(st);
                kill_tree(pid);
            }
        });
    }

    let jobs2 = jobs;
    let jid2 = jid.clone();
    std::thread::Builder::new()
        .name(format!("bash-{jid2}"))
        .spawn(move || {
            let status = child.wait();
            unregister_pty(&jid2);
            // Drain what is left, but do not wait for EOF forever: a daemon started by
            // the job (setsid, nohup … &) can keep the PTY open long after sh exited.
            let _ = pump_done.recv_timeout(Duration::from_secs(2));
            let out = acc
                .lock()
                .map(|v| String::from_utf8_lossy(&v).into_owned())
                .unwrap_or_default();
            let (state, mut body) = match status {
                Ok(s) => {
                    let code = s.code().unwrap_or(1);
                    let state = if code == 0 { "ok" } else { "error" };
                    (state, format!("exit {code}"))
                }
                Err(e) => ("error", format!("wait: {e}")),
            };
            if !out.trim().is_empty() {
                body.push('\n');
                body.push_str(&clip(&out, OUT_CAP));
            }
            let mut st = jobs2.lock().unwrap_or_else(|e| e.into_inner());
            if st.get(&jid2).is_some_and(|j| j.state == "killing") {
                body = match body.split_once('\n') {
                    Some((_, out)) => format!("killed (job_kill)\n{out}"),
                    None => "killed (job_kill)".into(),
                };
            }
            if st.finish(&jid2, state, &body) {
                drop(st);
                con.push_inbound(jobs::inbound(&jid2, &body));
            }
        })
        .map_err(|e| {
            unregister_pty(&jid);
            format!("thread: {e}")
        })?;
    Ok(pid)
}
