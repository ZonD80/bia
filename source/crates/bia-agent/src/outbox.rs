//! Outgoing telegram/whatsapp/messenger/instagram messages. `say` must not hold the agent while a big upload
//! runs, so every send gets a local id (`o17`) and goes through a per-chat FIFO run by a
//! worker thread. The caller waits a few seconds: a quick send returns its platform
//! message ids as before; a slow one returns the local id and the outcome arrives later
//! (`[sent o17 …]` as a quiet note in the next turn, `[send failed o17 …]` as a
//! `system|outbox` message that wakes the model). Edits, reactions and deletes ride the
//! same queue, so they may name `o17` while it is still uploading.
use crate::console::{Console, Inbound};
use crate::mcp::{call_result, LinkSlot};
use crate::tools::ID_SYSTEM;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const CH_OUTBOX: &str = "outbox";
/// How long `say` waits before handing back the local id.
pub const WAIT: Duration = Duration::from_secs(5);
/// Attachments at least this large skip the wait: they will not finish in time anyway.
pub const BIG_BYTES: u64 = 8_000_000;
const KEEP: usize = 300;

pub enum Op {
    Send { text: String, files: Vec<String> },
    Voice { text: String, lang: String },
    Edit { id: String, text: String },
    React { id: String, emoji: String },
    Delete { ids: Vec<String>, revoke: bool },
}

impl Op {
    fn delivers(&self) -> bool {
        matches!(self, Op::Send { .. } | Op::Voice { .. })
    }
    fn verb(&self) -> &'static str {
        match self {
            Op::Send { .. } => "send",
            Op::Voice { .. } => "voice message",
            Op::Edit { .. } => "edit",
            Op::React { .. } => "reaction",
            Op::Delete { .. } => "delete",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub channel: String,
    /// queued | sending | sent | failed | cancelled | lost
    pub state: String,
    #[serde(default)]
    pub native: Vec<String>,
    #[serde(default)]
    pub what: String,
    pub issued: u64,
    #[serde(default)]
    pub ended: u64,
    #[serde(default)]
    pub error: String,
}

#[derive(Serialize, Deserialize, Default)]
struct File {
    #[serde(default)]
    next: u32,
    #[serde(default)]
    entries: Vec<Entry>,
}

enum Wait {
    Waiting,
    Done(Result<String, String>),
    Gone,
}

struct Task {
    /// local id for sends; empty for edits/reactions/deletes
    local: String,
    channel: String,
    chat: String,
    op: Op,
    link: LinkSlot,
    bytes: u64,
    cancel: Arc<AtomicBool>,
    wait: Arc<(Mutex<Wait>, Condvar)>,
}

struct Inner {
    path: PathBuf,
    next: u32,
    entries: Vec<Entry>,
    queues: HashMap<String, VecDeque<Task>>,
    /// channel → (local id, cancel flag) of the task its worker is running
    running: HashMap<String, (String, Arc<AtomicBool>)>,
    /// quiet outcomes for the next turn
    notes: Vec<String>,
}

impl Inner {
    fn save(&self) {
        let f = File {
            next: self.next,
            entries: self.entries.clone(),
        };
        if let Ok(b) = serde_json::to_vec_pretty(&f) {
            let tmp = self.path.with_extension("json.tmp");
            if std::fs::write(&tmp, b).is_ok() {
                let _ = std::fs::rename(&tmp, &self.path);
            }
        }
    }
    fn entry(&mut self, id: &str) -> Option<&mut Entry> {
        self.entries.iter_mut().find(|e| e.id == id)
    }
    fn trim(&mut self) {
        while self.entries.len() > KEEP {
            match self
                .entries
                .iter()
                .position(|e| !matches!(e.state.as_str(), "queued" | "sending"))
            {
                Some(i) => {
                    self.entries.remove(i);
                }
                None => break,
            }
        }
    }
}

pub enum Outcome {
    /// finished within the wait: (local id, MCP result text or error)
    Done(String, Result<String, String>),
    /// still queued or uploading: (local id, tasks ahead of it in this chat)
    Pending(String, usize),
    /// held until its MCP, starting in the background, is up: (the start's job id)
    Held(String),
}

#[derive(Clone)]
pub struct Outbox {
    inner: Arc<Mutex<Inner>>,
    con: Arc<Console>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `o17` (or `o17.2` for the second message of a multi-file send).
pub fn is_local(id: &str) -> bool {
    let id = id.trim();
    id.strip_prefix('o')
        .map(|r| {
            !r.is_empty()
                && r.split('.')
                    .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        })
        .unwrap_or(false)
}

/// Platform ids from an MCP send result: "…, message id 12, 13 → Chat".
pub fn native_ids(text: &str) -> Vec<String> {
    let Some(i) = text.find("message id ") else {
        return vec![];
    };
    let rest = &text[i + "message id ".len()..];
    let rest = rest.split(" →").next().unwrap_or(rest);
    rest.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Run time allowed for one task: two minutes plus 100 KB/s for the attachments, at most 6 h.
fn deadline_for(bytes: u64) -> Duration {
    Duration::from_secs((120 + bytes / 100_000).min(6 * 3600))
}

impl Outbox {
    /// Load files/state/outbox.json. Sends that were in flight when the agent stopped are
    /// marked lost and returned as notices for the model.
    pub fn load(state_dir: &Path, con: Arc<Console>) -> (Outbox, Vec<Inbound>) {
        let path = state_dir.join("outbox.json");
        let f: File = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let mut inner = Inner {
            path,
            next: f.next.max(1),
            entries: f.entries,
            queues: HashMap::new(),
            running: HashMap::new(),
            notes: vec![],
        };
        let mut lost = vec![];
        for e in inner
            .entries
            .iter_mut()
            .filter(|e| matches!(e.state.as_str(), "queued" | "sending"))
        {
            let was = std::mem::replace(&mut e.state, "lost".into());
            e.ended = now();
            let text = if was == "queued" {
                format!(
                    "[send lost {} → {}] {}: the agent restarted before it was sent; not delivered",
                    e.id, e.channel, e.what
                )
            } else {
                format!("[send lost {} → {}] {}: the agent restarted during the upload; it may or may not have been delivered", e.id, e.channel, e.what)
            };
            lost.push(notice(&text));
        }
        if !lost.is_empty() {
            inner.save();
        }
        (
            Outbox {
                inner: Arc::new(Mutex::new(inner)),
                con,
            },
            lost,
        )
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Queue `op` for `channel` (`telegram:<chat>`) and wait up to `wait` for it.
    /// `what` describes a send for notices ("2 files, 48 MB").
    pub fn submit(
        &self,
        channel: &str,
        chat: &str,
        op: Op,
        link: LinkSlot,
        bytes: u64,
        what: &str,
        wait: Duration,
    ) -> Outcome {
        let cancel = Arc::new(AtomicBool::new(false));
        let waiter = Arc::new((Mutex::new(Wait::Waiting), Condvar::new()));
        let (local, ahead, spawn) = {
            let mut g = self.lock();
            let local = if op.delivers() {
                let id = format!("o{}", g.next);
                g.next += 1;
                g.entries.push(Entry {
                    id: id.clone(),
                    channel: channel.into(),
                    state: "queued".into(),
                    native: vec![],
                    what: what.into(),
                    issued: now(),
                    ended: 0,
                    error: String::new(),
                });
                g.trim();
                g.save();
                id
            } else {
                String::new()
            };
            let busy = g.running.contains_key(channel);
            let q = g.queues.entry(channel.into()).or_default();
            let ahead = q.len() + busy as usize;
            q.push_back(Task {
                local: local.clone(),
                channel: channel.into(),
                chat: chat.into(),
                op,
                link,
                bytes,
                cancel,
                wait: waiter.clone(),
            });
            if !busy {
                g.running.insert(
                    channel.into(),
                    (String::new(), Arc::new(AtomicBool::new(false))),
                );
            }
            (local, ahead, !busy)
        };
        if spawn {
            let me = self.clone();
            let ch = channel.to_string();
            let again = me.clone();
            if std::thread::Builder::new()
                .name(format!("outbox-{ch}"))
                .spawn(move || me.worker(&ch))
                .is_err()
            {
                // No thread: drain the queue here rather than strand it.
                again.worker(channel);
            }
        }
        let (m, cv) = &*waiter;
        let mut st = m.lock().unwrap_or_else(|e| e.into_inner());
        let until = Instant::now() + wait;
        loop {
            if let Wait::Done(_) = &*st {
                // An error in hand goes back as the say result; no notice follows.
                let Wait::Done(r) = std::mem::replace(&mut *st, Wait::Gone) else {
                    unreachable!()
                };
                return Outcome::Done(local, r);
            }
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                *st = Wait::Gone;
                return Outcome::Pending(local, ahead);
            }
            st = cv
                .wait_timeout(st, left)
                .map(|(g, _)| g)
                .unwrap_or_else(|e| e.into_inner().0);
        }
    }

    fn worker(&self, channel: &str) {
        loop {
            let task = {
                let mut g = self.lock();
                match g.queues.get_mut(channel).and_then(|q| q.pop_front()) {
                    Some(t) => {
                        g.running
                            .insert(channel.into(), (t.local.clone(), t.cancel.clone()));
                        t
                    }
                    None => {
                        g.queues.remove(channel);
                        g.running.remove(channel);
                        return;
                    }
                }
            };
            let started = Instant::now();
            let r = self.run(&task);
            self.finish(task, r, started.elapsed());
        }
    }

    /// Execute one task: resolve local ids, call the MCP, wait for the result.
    fn run(&self, t: &Task) -> Result<String, String> {
        if t.cancel.load(Ordering::Relaxed) {
            return Err("cancelled before it started".into());
        }
        let (tool, args) = match &t.op {
            Op::Send { text, files } => {
                let mut a = json!({"chat_id": t.chat, "text": text});
                if !files.is_empty() {
                    a["files"] = json!(files);
                }
                ("send", a)
            }
            Op::Voice { text, lang } => (
                "send_voice",
                json!({"chat_id": t.chat, "text": text, "lang": lang}),
            ),
            Op::Edit { id, text } => (
                "edit",
                json!({"chat_id": t.chat, "message_id": self.resolve(id)?, "text": text}),
            ),
            Op::React { id, emoji } => (
                "react",
                json!({"chat_id": t.chat, "message_id": self.resolve(id)?, "emoji": emoji}),
            ),
            Op::Delete { ids, revoke } => {
                let ids = ids
                    .iter()
                    .map(|i| self.resolve(i))
                    .collect::<Result<Vec<_>, _>>()?;
                (
                    "delete_messages",
                    json!({"chat_id": t.chat, "message_ids": ids, "revoke": revoke}),
                )
            }
        };
        if !t.local.is_empty() {
            let mut g = self.lock();
            if let Some(e) = g.entry(&t.local) {
                e.state = "sending".into();
            }
            g.save();
        }
        let link = t.link.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let (rpc, rx) = link.start(tool, args).map_err(|e| format!("{e:#}"))?;
        let deadline = Instant::now() + deadline_for(t.bytes);
        loop {
            match rx.recv_timeout(Duration::from_millis(300)) {
                Ok(v) => {
                    let (text, err) = call_result(&v);
                    return if err { Err(text) } else { Ok(text) };
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(
                        "the MCP exited during the call; it may or may not have been delivered"
                            .into(),
                    );
                }
                Err(RecvTimeoutError::Timeout) => {
                    if t.cancel.load(Ordering::Relaxed) {
                        link.abandon(rpc);
                        return Err(
                            "cancelled; files already uploaded may still have been delivered"
                                .into(),
                        );
                    }
                    if Instant::now() >= deadline {
                        link.abandon(rpc);
                        return Err(format!(
                            "timed out after {}s",
                            deadline_for(t.bytes).as_secs()
                        ));
                    }
                }
            }
        }
    }

    /// `o17` → its first platform id, `o17.2` → the second; platform ids pass through.
    fn resolve(&self, id: &str) -> Result<String, String> {
        let id = id.trim();
        if !is_local(id) {
            return Ok(id.to_string());
        }
        let (base, nth) = match id.split_once('.') {
            Some((b, n)) => (b, n.parse::<usize>().unwrap_or(1).max(1)),
            None => (id, 1),
        };
        let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let e = g
            .entries
            .iter()
            .find(|e| e.id == base)
            .ok_or_else(|| format!("unknown message {id}"))?;
        if e.state != "sent" {
            return Err(format!("{base} was not delivered ({})", e.state));
        }
        e.native
            .get(nth - 1)
            .cloned()
            .ok_or_else(|| format!("{base} has {} message id(s), no #{nth}", e.native.len()))
    }

    fn finish(&self, t: Task, r: Result<String, String>, took: Duration) {
        let cancelled = t.cancel.load(Ordering::Relaxed);
        {
            let mut g = self.lock();
            if let Some(e) = g.entry(&t.local) {
                e.ended = now();
                match &r {
                    Ok(text) => {
                        e.state = "sent".into();
                        e.native = native_ids(text);
                    }
                    Err(err) => {
                        e.state = if cancelled { "cancelled" } else { "failed" }.into();
                        e.error = err.clone();
                    }
                }
            }
            if !t.local.is_empty() {
                g.save();
            }
        }
        let (m, cv) = &*t.wait;
        let mut st = m.lock().unwrap_or_else(|e| e.into_inner());
        if let Wait::Waiting = &*st {
            *st = Wait::Done(r);
            cv.notify_all();
            return;
        }
        drop(st);
        // say already returned: report the outcome.
        let label = if t.local.is_empty() {
            String::new()
        } else {
            format!(" {}", t.local)
        };
        let secs = took.as_secs();
        match r {
            Ok(text) => {
                let line = if t.op.delivers() {
                    format!("[sent{label} → {}] {text} ({secs}s)", t.channel)
                } else {
                    format!("[{} done on {}] {text}", t.op.verb(), t.channel)
                };
                self.con
                    .writeln(&self.con.c(crate::console::GRAY, &format!("  {line}")));
                self.lock().notes.push(line);
            }
            Err(e) if cancelled && t.op.delivers() => {
                let line = format!("[send cancelled{label} → {}] {e}", t.channel);
                self.con
                    .writeln(&self.con.c(crate::console::GRAY, &format!("  {line}")));
                self.lock().notes.push(line);
            }
            Err(e) => {
                let line = format!("[{} failed{label} → {}] {e}", t.op.verb(), t.channel);
                self.con
                    .writeln(&self.con.c(crate::console::RED, &format!("  {line}")));
                self.con.push_inbound(notice(&line));
            }
        }
    }

    /// A send that could not even be queued (its MCP did not start): wakes the model, as
    /// a failed send does.
    pub fn report_failed(&self, line: &str) {
        self.con.writeln(&self.con.c(crate::console::RED, &format!("  {line}")));
        self.con.push_inbound(notice(line));
    }

    /// Quiet outcomes since the last call, for the next turn's history.
    pub fn take_notes(&self) -> Vec<String> {
        std::mem::take(&mut self.lock().notes)
    }

    /// Stop a queued or running send.
    pub fn cancel(&self, id: &str) -> Result<String, String> {
        let id = id.trim();
        let mut g = self.lock();
        if let Some((_, flag)) = g.running.values().find(|(l, _)| l == id) {
            flag.store(true, Ordering::Relaxed);
            return Ok(format!("cancelling {id}; [send cancelled {id} …] follows. Files already uploaded may still arrive"));
        }
        for q in g.queues.values_mut() {
            if let Some(t) = q.iter().find(|t| t.local == id) {
                t.cancel.store(true, Ordering::Relaxed);
                return Ok(format!("cancelled {id} before it started"));
            }
        }
        match g.entries.iter().find(|e| e.id == id) {
            Some(e) => Err(format!("{id} is already {}", e.state)),
            None => Err(format!("unknown message {id}")),
        }
    }

    /// One line per send still in flight, for list_jobs.
    pub fn brief(&self) -> String {
        let g = self.lock();
        let t = now();
        g.entries
            .iter()
            .filter(|e| matches!(e.state.as_str(), "queued" | "sending"))
            .map(|e| {
                format!(
                    "{} → {} {} ({}, {}s)",
                    e.id,
                    e.channel,
                    e.state,
                    e.what,
                    t.saturating_sub(e.issued)
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn notice(text: &str) -> Inbound {
    Inbound {
        identity: ID_SYSTEM.into(),
        channel: CH_OUTBOX.into(),
        issued: SystemTime::now(),
        id: String::new(),
        text: text.into(),
    }
    .ensure_id()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::{Manifest, McpServer};

    /// A stand-in messaging MCP: `send` answers "message id 9<rpc>" (after 2 s when the text
    /// says slow, 30 s for stuck, exits for die); `edit` echoes the message_id it got.
    const FAKE: &str = r#"#!/bin/sh
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  ok() { printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[{"type":"text","text":"%s"}]}}\n' "$id" "$1"; }
  case "$line" in
    *'"method":"initialize"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{}}\n' "$id" ;;
    *'"name":"send"'*)
      case "$line" in
        *die*) exit 0 ;;
        *stuck*) (sleep 30; ok "sent, message id 9$id → X") & ;;
        *slow*) (sleep 2; ok "sent 2 file(s), message id 9$id, 8$id → X") & ;;
        *) ok "sent, message id 9$id → X" ;;
      esac ;;
    *'"name":"edit"'*) ok "edited $(printf '%s' "$line" | sed -n 's/.*"message_id":"\([^"]*\)".*/\1/p')" ;;
  esac
done
"#;

    fn setup(tag: &str) -> (PathBuf, Arc<Console>, McpServer) {
        let dir = std::env::temp_dir().join(format!("bia-outbox-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("fake.sh");
        std::fs::write(&exe, FAKE).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        let m: Manifest =
            serde_json::from_value(json!({"name": "telegram", "command": exe.to_str().unwrap()}))
                .unwrap();
        let con = Console::for_test();
        let jobs = Arc::new(Mutex::new(crate::jobs::Store::load(&dir)));
        let srv = McpServer::spawn(
            &dir,
            &m,
            std::env::consts::ARCH,
            &[],
            &dir,
            con.clone(),
            jobs,
        )
        .unwrap();
        (dir, con, srv)
    }

    fn send(text: &str) -> Op {
        Op::Send {
            text: text.into(),
            files: vec![],
        }
    }

    fn wait_for(mut f: impl FnMut() -> bool) {
        let until = Instant::now() + Duration::from_secs(10);
        while !f() {
            assert!(Instant::now() < until, "timed out");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    #[test]
    fn quick_slow_queued_edit_and_cancel() {
        let (dir, con, srv) = setup("flow");
        let (ob, lost) = Outbox::load(&dir, con.clone());
        assert!(lost.is_empty());
        let w = Duration::from_millis(500);

        // Quick: platform ids come back inline.
        match ob.submit("telegram:5", "5", send("hi"), srv.link(), 0, "text", w) {
            Outcome::Done(id, Ok(t)) => {
                assert_eq!(id, "o1");
                assert!(t.contains("message id 9"), "{t}");
            }
            _ => panic!("expected a quick send"),
        }

        // Slow: the local id comes back; an edit naming it waits behind it and gets the real id.
        let Outcome::Pending(id, 0) =
            ob.submit("telegram:5", "5", send("slow"), srv.link(), 0, "2 files", w)
        else {
            panic!("expected pending")
        };
        assert_eq!(id, "o2");
        assert!(ob.brief().contains("o2 → telegram:5"));
        let edit = Op::Edit {
            id: "o2.2".into(),
            text: "new".into(),
        };
        let Outcome::Pending(_, 1) = ob.submit("telegram:5", "5", edit, srv.link(), 0, "", w)
        else {
            panic!("edit should wait behind o2")
        };
        // Another chat is not held up by chat 5.
        assert!(matches!(
            ob.submit("telegram:6", "6", send("hi"), srv.link(), 0, "text", w),
            Outcome::Done(_, Ok(_))
        ));

        wait_for(|| ob.lock().notes.len() >= 2);
        let notes = ob.take_notes();
        assert!(notes[0].starts_with("[sent o2 → telegram:5]"), "{notes:?}");
        let e = ob
            .lock()
            .entries
            .iter()
            .find(|e| e.id == "o2")
            .cloned()
            .unwrap();
        assert_eq!(e.state, "sent");
        assert_eq!(e.native.len(), 2);
        assert!(
            notes[1].contains(&format!("edited {}", e.native[1])),
            "{notes:?}"
        );
        assert!(ob.brief().is_empty());

        // Cancel during the upload: quiet note, entry cancelled, edits on it fail loudly.
        let Outcome::Pending(id, 0) =
            ob.submit("telegram:5", "5", send("stuck"), srv.link(), 0, "big", w)
        else {
            panic!("expected pending")
        };
        assert!(ob.cancel(&id).unwrap().starts_with("cancelling"));
        wait_for(|| !ob.lock().notes.is_empty());
        assert!(ob.take_notes()[0].starts_with(&format!("[send cancelled {id}")));
        assert!(ob.cancel(&id).unwrap_err().contains("already cancelled"));
        let edit = Op::Edit {
            id: id.clone(),
            text: "x".into(),
        };
        match ob.submit("telegram:5", "5", edit, srv.link(), 0, "", w) {
            Outcome::Done(_, Err(e)) => assert!(e.contains("not delivered"), "{e}"),
            _ => panic!("edit of a cancelled send must fail"),
        }

        // A send in flight across a restart is reported lost.
        let Outcome::Pending(id, 0) =
            ob.submit("telegram:7", "7", send("stuck"), srv.link(), 0, "big", w)
        else {
            panic!("expected pending")
        };
        wait_for(|| {
            ob.lock()
                .entries
                .iter()
                .any(|e| e.id == id && e.state == "sending")
        });
        let (_, lost) = Outbox::load(&dir, con.clone());
        assert_eq!(lost.len(), 1);
        assert!(lost[0]
            .text
            .contains(&format!("[send lost {id} → telegram:7]")));
        assert!(con.try_read_inbound().is_none());
        drop(srv);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn server_exit_fails_loudly() {
        let (dir, con, srv) = setup("exit");
        let (ob, _) = Outbox::load(&dir, con.clone());
        let Outcome::Pending(id, 0) = ob.submit(
            "telegram:5",
            "5",
            send("slow"),
            srv.link(),
            0,
            "x",
            Duration::from_millis(200),
        ) else {
            panic!("expected pending")
        };
        let _ = ob.submit(
            "telegram:6",
            "6",
            send("die"),
            srv.link(),
            0,
            "x",
            Duration::from_millis(200),
        );
        let mut got = None;
        wait_for(|| {
            got = con.try_read_inbound();
            got.is_some()
        });
        let ib = got.unwrap();
        assert_eq!(ib.channel, CH_OUTBOX);
        assert!(ib.text.contains("[send failed"), "{}", ib.text);
        assert!(ib.text.contains("exited"), "{}", ib.text);
        let _ = id;
        drop(srv);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn local_ids() {
        assert!(is_local("o1"));
        assert!(is_local("o17.2"));
        assert!(!is_local("o"));
        assert!(!is_local("o1."));
        assert!(!is_local("5512"));
        assert!(!is_local("oops"));
        assert!(!is_local("b0a1b2c3d0001"));
    }

    #[test]
    fn parses_native_ids() {
        assert_eq!(
            native_ids("sent, message id 5512 → Ann (@ann)"),
            vec!["5512"]
        );
        assert_eq!(
            native_ids("sent 2 file(s), message id 1, 2 → Chat"),
            vec!["1", "2"]
        );
        assert_eq!(
            native_ids("sent message id 3EB0ABC → +1555"),
            vec!["3EB0ABC"]
        );
        assert_eq!(
            native_ids("sent voice message (3.2s), message id 77 → X"),
            vec!["77"]
        );
        assert!(native_ids("sent → X").is_empty());
    }
}
