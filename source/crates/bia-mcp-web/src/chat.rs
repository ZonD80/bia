//! Web chats: one Telegram-style chat per client-certificate identity (channel `web`):
//! people, other bias and outside agents alike. Messages carry replies, attachments,
//! reactions, edits, pins, read marks, inline buttons and polls.
//!
//! The store is also the web bus log: every change gives the message the chat's next
//! revision (`rev`), deletions leave tombstones, so a client that last saw revision `rx`
//! catches up with what changed after it (`since`). Clients act with the actions of the
//! web bus protocol (`action`); their messages reach the model as inbound envelopes once
//! their files have arrived. Another bia's own copy of a chat comes back here through
//! `apply_remote` when this bia is the one that dialed.
use crate::bus::{mint_msg_id, Bus, Inbound};
use crate::linkproto::{name_here, unhex32, valid_msg_id, MAX_FILE};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

pub const MAIN: &str = "main";
pub const BIA: &str = "bia";
const MAX_MSGS: usize = 5000;
const MAX_TEXT: usize = 64 * 1024;
/// Tombstones are kept this long; a client further behind reloads (`reset`).
const KEEP_GONE_MS: u64 = 30 * 86_400 * 1000;
/// Client action ids remembered, so an action sent twice is applied once.
const KEEP_CIDS: usize = 500;

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct Att {
    /// Relative to the files dir, or absolute (agent/root attachments outside it). Empty
    /// while the file is still on its way.
    pub path: String,
    pub name: String,
    pub size: u64,
    pub mime: String,
    /// photo | video | audio | voice | file
    pub kind: String,
    /// voice/audio: what was said (transcript of a user's voice message, or the text
    /// bia spoke)
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub transcript: String,
    /// sha256 of the content: how the other side pulls it over the web bus
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub fid: String,
    /// still being pulled from the side that sent it
    #[serde(default, skip_serializing_if = "is_false")]
    pub pending: bool,
    /// why it did not arrive
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub err: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct Btn {
    pub text: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub data: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct PollOpt {
    pub text: String,
    #[serde(default)]
    pub voters: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct Poll {
    pub question: String,
    pub options: Vec<PollOpt>,
    #[serde(default)]
    pub multi: bool,
    #[serde(default)]
    pub closed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct Fwd {
    pub from: String,
    pub ts: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct Msg {
    pub id: String,
    /// `bia` or the client identity
    pub from: String,
    /// unix millis
    pub ts: u64,
    #[serde(default)]
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    /// forwarded (messages from before 1.9; there is one chat, nothing to forward to)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fwd: Option<Fwd>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<Att>,
    /// emoji → who reacted (identity or `bia`)
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub reactions: BTreeMap<String, Vec<String>>,
    /// unix millis of the last edit, 0 = never
    #[serde(default, skip_serializing_if = "is_zero")]
    pub edited: u64,
    #[serde(default, skip_serializing_if = "is_false")]
    pub pinned: bool,
    /// user message: the model has read it; bia message: the user has seen it
    #[serde(default, skip_serializing_if = "is_false")]
    pub read: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub buttons: Vec<Vec<Btn>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll: Option<Poll>,
    /// centered service line
    #[serde(default, skip_serializing_if = "is_false")]
    pub service: bool,
    /// the chat's revision of this message's last change
    #[serde(default)]
    pub rev: u64,
    /// stored, waiting for its files before it reaches the model
    #[serde(default, skip_serializing_if = "is_false")]
    pub wait: bool,
}

fn is_zero(v: &u64) -> bool {
    *v == 0
}
fn is_false(v: &bool) -> bool {
    !*v
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
struct Gone {
    id: String,
    rev: u64,
    /// unix millis
    at: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
struct Meta {
    /// the chat's last revision
    #[serde(default)]
    rev: u64,
    /// clients that saw less than this must reload (tombstones before it are gone)
    #[serde(default)]
    floor: u64,
    #[serde(default)]
    gone: Vec<Gone>,
    #[serde(default)]
    muted: bool,
    #[serde(default)]
    cids: VecDeque<String>,
}

struct User {
    dir: PathBuf,
    meta: Meta,
    msgs: Vec<Msg>,
}

struct Hub {
    files: PathBuf,
    con: Arc<Bus>,
    users: Mutex<HashMap<String, User>>,
    typing: Mutex<HashSet<String>>,
}

static HUB: OnceLock<Hub> = OnceLock::new();

pub fn init(files: &Path, con: Arc<Bus>) {
    let _ = HUB.set(Hub {
        files: files.to_path_buf(),
        con,
        users: Mutex::new(HashMap::new()),
        typing: Mutex::new(HashSet::new()),
    });
}

fn hub() -> Result<&'static Hub, String> {
    HUB.get()
        .ok_or_else(|| "web chats are not started".to_string())
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ---- channels ----

/// The web channel. `web:<anything>` (chats before 1.9) is the same one chat.
pub fn is_web(channel: &str) -> bool {
    channel == crate::bus::CH_WEB || channel.starts_with("web:")
}

/// Every web channel is the one chat (`main`).
pub fn chat_of(channel: &str) -> Option<&'static str> {
    is_web(channel).then_some(MAIN)
}

pub fn channel_of(_chat: &str) -> String {
    crate::bus::CH_WEB.into()
}

fn safe_seg(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .take(80)
        .collect();
    let t = t.trim_start_matches('.').to_string();
    if t.is_empty() {
        "_".into()
    } else {
        t
    }
}

/// Inbox for one identity's files, relative to the files dir.
pub fn inbox_rel(identity: &str) -> String {
    format!("inbox/web/{}", safe_seg(identity))
}

// ---- files ----

pub fn mime_for(name: &str) -> &'static str {
    let ext = Path::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "txt" | "log" | "md" | "rs" | "toml" | "yaml" | "yml" | "sh" | "py" | "c" | "h" | "cpp"
        | "go" | "ts" | "conf" | "cfg" | "ini" | "csv" | "xml" => "text/plain; charset=utf-8",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "avif" => "image/avif",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "mkv" => "video/x-matroska",
        "ogv" => "video/ogg",
        "mp3" => "audio/mpeg",
        "m4a" | "aac" => "audio/mp4",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        "ogg" | "oga" | "opus" => "audio/ogg",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "tar" => "application/x-tar",
        "gz" | "tgz" => "application/gzip",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

fn kind_for(mime: &str) -> &'static str {
    if mime.starts_with("image/") {
        "photo"
    } else if mime.starts_with("video/") {
        "video"
    } else if mime.starts_with("audio/") {
        "audio"
    } else {
        "file"
    }
}

fn under(root: &Path, rel: &str) -> Option<PathBuf> {
    if rel.is_empty() {
        return None;
    }
    let mut p = root.to_path_buf();
    for c in Path::new(rel.trim_start_matches('/')).components() {
        match c {
            Component::Normal(s) => p.push(s),
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(p)
}

/// Absolute path of an attachment (relative paths are under the files dir).
pub fn att_abs(files: &Path, path: &str) -> Option<PathBuf> {
    if path.starts_with('/') {
        let p = PathBuf::from(path);
        if p.components().any(|c| matches!(c, Component::ParentDir)) {
            return None;
        }
        Some(p)
    } else {
        under(files, path)
    }
}

pub fn sha256_file(p: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(p)?;
    let mut ctx = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        ctx.update(&buf[..n]);
    }
    Ok(crate::linkproto::hex(ctx.finish().as_ref()))
}

/// Build an attachment record (with its sha256). `rel_or_abs` may be `/home/bia/files/x`
/// (stored relative).
fn make_att(files: &Path, rel_or_abs: &str, kind_hint: Option<&str>) -> Result<Att, String> {
    let raw = rel_or_abs.trim();
    let stored = match Path::new(raw).strip_prefix(files) {
        Ok(rel) => rel.to_string_lossy().to_string(),
        Err(_) => raw.to_string(),
    };
    let abs = att_abs(files, &stored).ok_or_else(|| format!("bad path {raw}"))?;
    let md = std::fs::metadata(&abs).map_err(|e| format!("{raw}: {e}"))?;
    if !md.is_file() {
        return Err(format!("{raw} is not a file"));
    }
    let name = abs
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("file")
        .to_string();
    // uploads are stamped `YYYYMMDD-HHMMSS-name`; show the original name
    let shown = strip_stamp(&name).to_string();
    let mime = mime_for(&name).to_string();
    let kind = match kind_hint {
        Some("voice") => "voice",
        Some("file") => "file",
        _ => kind_for(&mime),
    };
    Ok(Att {
        path: stored,
        name: shown,
        size: md.len(),
        mime,
        kind: kind.into(),
        fid: sha256_file(&abs).map_err(|e| format!("{raw}: {e}"))?,
        ..Default::default()
    })
}

/// An attachment the other side lists (`{fid, name, size, kind?, transcript?}`): it is
/// pulled from there.
fn remote_att(v: &Value) -> Result<Att, String> {
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
    let fid = s("fid").to_ascii_lowercase();
    if unhex32(&fid).is_none() {
        return Err("a file needs fid (its sha256 in hex)".into());
    }
    let size = v.get("size").and_then(|x| x.as_u64()).unwrap_or(0);
    if size > MAX_FILE {
        return Err(format!("file too big ({} MB; the limit is {} MB)", size >> 20, MAX_FILE >> 20));
    }
    let name = safe_file_name(&s("name"));
    let mime = mime_for(&name).to_string();
    let kind = match s("kind").as_str() {
        k @ ("voice" | "photo" | "video" | "audio" | "file") => k.to_string(),
        _ => kind_for(&mime).to_string(),
    };
    Ok(Att {
        path: String::new(),
        name,
        size,
        mime,
        kind,
        transcript: s("transcript"),
        fid,
        pending: true,
        err: String::new(),
    })
}

/// What a sender lists for one of its files (`send` action).
fn att_offer(a: &Att) -> Value {
    let mut v = json!({"fid": a.fid, "name": a.name, "size": a.size, "kind": a.kind});
    if !a.transcript.is_empty() {
        v["transcript"] = json!(a.transcript);
    }
    v
}

pub fn safe_file_name(n: &str) -> String {
    let base = n.rsplit(['/', '\\']).next().unwrap_or("file");
    let s: String = base.chars().filter(|c| !c.is_control()).take(120).collect();
    let s = s.trim().trim_start_matches('.').to_string();
    if s.is_empty() {
        "file".into()
    } else {
        s
    }
}

fn strip_stamp(name: &str) -> &str {
    let b = name.as_bytes();
    if b.len() > 16
        && b[..8].iter().all(|c| c.is_ascii_digit())
        && b[8] == b'-'
        && b[9..15].iter().all(|c| c.is_ascii_digit())
        && b[15] == b'-'
    {
        &name[16..]
    } else {
        name
    }
}

// ---- persistence ----

fn read_msgs(p: &Path) -> Vec<Msg> {
    std::fs::read(p)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

impl User {
    fn load(dir: PathBuf) -> User {
        let mut msgs = read_msgs(&dir.join("main.json"));
        let mut changed = false;
        // before 1.9 a person could have several chats: fold them into the one chat,
        // in time order (nothing a person said is dropped)
        let chats_json = dir.join("chats.json");
        let mut muted = false;
        if let Some(chats) = std::fs::read(&chats_json)
            .ok()
            .and_then(|b| serde_json::from_slice::<Vec<Value>>(&b).ok())
        {
            for c in &chats {
                let id = c["id"].as_str().unwrap_or("");
                if id == MAIN {
                    muted = c["muted"].as_bool().unwrap_or(false);
                    continue;
                }
                if id.is_empty() || !id.chars().all(|x| x.is_ascii_alphanumeric()) {
                    continue;
                }
                let f = dir.join(format!("{id}.json"));
                for m in read_msgs(&f) {
                    if !msgs.iter().any(|x| x.id == m.id) {
                        msgs.push(m);
                    }
                }
                let _ = std::fs::rename(&f, dir.join(format!("{id}.json.migrated")));
            }
            msgs.sort_by_key(|m| m.ts);
            let _ = std::fs::rename(&chats_json, dir.join("chats.json.migrated"));
            changed = true;
        }
        let meta_path = dir.join("meta.json");
        let meta = match std::fs::read(&meta_path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Meta>(&b).ok())
        {
            Some(m) if !changed => m,
            old => {
                // the first start with revisions: number what is there, oldest first
                let mut m = old.unwrap_or_default();
                m.muted |= muted;
                for x in msgs.iter_mut() {
                    m.rev += 1;
                    x.rev = m.rev;
                }
                changed = true;
                m
            }
        };
        let u = User { dir, meta, msgs };
        if changed {
            u.save();
        }
        u
    }

    fn save(&self) {
        let _ = std::fs::create_dir_all(&self.dir);
        if let Ok(b) = serde_json::to_vec(&self.msgs) {
            atomic_write(&self.dir.join("main.json"), &b);
        }
        if let Ok(b) = serde_json::to_vec(&self.meta) {
            atomic_write(&self.dir.join("meta.json"), &b);
        }
    }

    /// Keep the chat bounded and the tombstones young; returns nothing to tell anyone
    /// (trimmed history is not a deletion).
    fn prune(&mut self) {
        if self.msgs.len() > MAX_MSGS {
            let n = self.msgs.len() - MAX_MSGS;
            self.msgs.drain(0..n);
        }
        let cut = now_ms().saturating_sub(KEEP_GONE_MS);
        let mut floor = self.meta.floor;
        self.meta.gone.retain(|g| {
            if g.at < cut {
                floor = floor.max(g.rev);
                false
            } else {
                true
            }
        });
        self.meta.floor = floor;
    }

    fn pos(&self, id: &str) -> Option<usize> {
        self.msgs.iter().position(|m| m.id == id)
    }

    fn find(&self, id: &str) -> Option<&Msg> {
        self.msgs.iter().find(|m| m.id == id)
    }

    /// Message `i` changed: it gets the next revision. Returns its new state.
    fn touch(&mut self, i: usize) -> Msg {
        self.meta.rev += 1;
        self.msgs[i].rev = self.meta.rev;
        self.msgs[i].clone()
    }

    /// Add a message (in time order: a late one from a reconnect slots in) and number it.
    fn insert(&mut self, m: Msg) -> Msg {
        let at = self
            .msgs
            .iter()
            .rposition(|x| x.ts <= m.ts)
            .map(|i| i + 1)
            .unwrap_or(0);
        self.msgs.insert(at, m);
        self.touch(at)
    }

    /// Remove messages; returns the ids that were there and the tombstones' revision.
    fn remove(&mut self, ids: &[String]) -> (Vec<String>, u64) {
        let gone: Vec<String> = self
            .msgs
            .iter()
            .filter(|m| ids.contains(&m.id))
            .map(|m| m.id.clone())
            .collect();
        if gone.is_empty() {
            return (gone, 0);
        }
        self.msgs.retain(|m| !gone.contains(&m.id));
        self.meta.rev += 1;
        let (rev, at) = (self.meta.rev, now_ms());
        for id in &gone {
            self.meta.gone.push(Gone { id: id.clone(), rev, at });
        }
        (gone, rev)
    }

    /// Fill in missing sha256 of stored files (messages from before 1.9), so the other
    /// side can pull them. True when something changed.
    fn ensure_fids(&mut self, files: &Path, range: std::ops::Range<usize>) -> bool {
        let mut changed = false;
        for m in &mut self.msgs[range] {
            for a in m.files.iter_mut().filter(|a| a.fid.is_empty() && !a.pending && !a.path.is_empty()) {
                if let Some(sum) = att_abs(files, &a.path).and_then(|p| sha256_file(&p).ok()) {
                    a.fid = sum;
                    changed = true;
                }
            }
        }
        changed
    }
}

fn atomic_write(p: &Path, b: &[u8]) {
    let tmp = p.with_extension("tmp");
    if std::fs::write(&tmp, b).is_ok() {
        let _ = std::fs::rename(&tmp, p);
    }
}

impl Hub {
    fn with_user<R>(&self, identity: &str, f: impl FnOnce(&mut User) -> R) -> R {
        let mut users = self.users.lock().unwrap_or_else(|e| e.into_inner());
        let u = users
            .entry(identity.to_string())
            .or_insert_with(|| User::load(self.user_dir(identity)));
        f(u)
    }

    fn user_dir(&self, identity: &str) -> PathBuf {
        self.files.join("state").join("web").join(safe_seg(identity))
    }

    /// A message's new state to the identity's live session.
    fn emit_m(&self, identity: &str, m: &Msg) {
        crate::link::emit(identity, json!({"t": "m", "seq": m.rev, "m": m}));
    }

    fn emit_gone(&self, identity: &str, rev: u64, ids: &[String]) {
        if !ids.is_empty() {
            crate::link::emit(identity, json!({"t": "gone", "seq": rev, "ids": ids}));
        }
    }

    /// Show a voice message's transcript under it (stored and sent to the client).
    fn set_transcript(&self, identity: &str, id: &str, path: &str, text: &str) {
        let m = self.with_user(identity, |u| -> Option<Msg> {
            let i = u.pos(id)?;
            let a = u.msgs[i].files.iter_mut().find(|a| a.path == path)?;
            a.transcript = text.to_string();
            let m = u.touch(i);
            u.save();
            Some(m)
        });
        if let Some(m) = m {
            self.emit_m(identity, &m);
        }
    }

    /// Hand something the identity did to the model as an inbound envelope.
    fn to_agent(&self, identity: &str, id: &str, text: String) {
        self.con.push_inbound(Inbound {
            identity: identity.to_string(),
            channel: crate::bus::CH_WEB.into(),
            id: id.to_string(),
            text,
        });
    }
}

fn snippet(m: &Msg) -> String {
    let mut s = m.text.replace('\n', " ");
    if s.trim().is_empty() {
        s = match m.files.first() {
            Some(a) => format!("[{}] {}", a.kind, a.name),
            None if m.poll.is_some() => format!(
                "[poll] {}",
                m.poll.as_ref().map(|p| p.question.as_str()).unwrap_or("")
            ),
            None => String::new(),
        };
    }
    if s.chars().count() > 80 {
        s = s.chars().take(80).collect::<String>() + "…";
    }
    s
}

/// A new message from bia: wake the person's devices that are not in front (not when muted).
fn push_msg(h: &Hub, identity: &str, m: &Msg) {
    if h.with_user(identity, |u| u.meta.muted) {
        return;
    }
    let mut body: String = m.text.split_whitespace().collect::<Vec<_>>().join(" ");
    if body.is_empty() {
        body = match (m.files.first(), &m.poll) {
            (Some(a), _) => match a.kind.as_str() {
                "photo" => "🖼 Photo".into(),
                "video" => "📹 Video".into(),
                "voice" => "🎤 Voice message".into(),
                "audio" => format!("🎵 {}", a.name),
                _ => format!("📄 {}", a.name),
            },
            (None, Some(p)) => format!("📊 {}", p.question),
            _ => return,
        };
    }
    if body.chars().count() > 300 {
        body = body.chars().take(300).collect::<String>() + "…";
    }
    crate::push::notify(
        identity,
        crate::push::Note {
            title: BIA.into(),
            body,
            tag: "bia-chat".into(),
            url: "chat/main".into(),
            ttl: 24 * 3600,
            urgent: false,
        },
    );
}

fn files_block(files: &[Att]) -> String {
    let mut s = String::from("files:\n");
    for a in files.iter().filter(|a| !a.path.is_empty()) {
        let tag = match a.kind.as_str() {
            "voice" => " (voice message)",
            "photo" => " (photo)",
            "video" => " (video)",
            "audio" => " (audio)",
            _ => "",
        };
        s.push_str(&format!("- {}{tag}\n", a.path));
    }
    s
}

/// What the model sees for a message from the identity (Telegram-like markers + the
/// text + files).
fn agent_text(u: &User, m: &Msg) -> String {
    let mut parts: Vec<String> = vec![];
    if let Some(r) = &m.reply_to {
        match u.find(r) {
            Some(o) => parts.push(format!(
                "[reply to {r} from {}: \"{}\"]",
                if o.from == BIA { "you" } else { o.from.as_str() },
                snippet(o)
            )),
            None => parts.push(format!("[reply to {r}]")),
        }
    }
    if let Some(f) = &m.fwd {
        parts.push(format!(
            "[forwarded from {} {}]",
            f.from,
            crate::bus::rfc3339_unix(f.ts / 1000)
        ));
    }
    if !m.text.trim().is_empty() {
        parts.push(m.text.clone());
    }
    if m.files.iter().any(|a| !a.path.is_empty()) {
        parts.push(files_block(&m.files).trim_end().to_string());
    }
    for a in m.files.iter().filter(|a| a.path.is_empty()) {
        parts.push(format!(
            "[file {} not received: {}]",
            a.name,
            if a.err.is_empty() { "it did not arrive" } else { &a.err }
        ));
    }
    parts.join("\n")
}

fn parse_buttons(v: Option<&Value>) -> Vec<Vec<Btn>> {
    let Some(Value::Array(rows)) = v else {
        return vec![];
    };
    let one = |b: &Value| -> Option<Btn> {
        match b {
            Value::String(s) if !s.trim().is_empty() => Some(Btn {
                text: s.trim().into(),
                ..Default::default()
            }),
            Value::Object(o) => {
                let text = o.get("text")?.as_str()?.trim().to_string();
                if text.is_empty() {
                    return None;
                }
                let url = o
                    .get("url")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                Some(Btn {
                    text,
                    data: o
                        .get("data")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string(),
                    url: if url.starts_with("https://")
                        || url.starts_with("http://")
                        || url.starts_with('/')
                    {
                        url
                    } else {
                        String::new()
                    },
                })
            }
            _ => None,
        }
    };
    let mut out = vec![];
    for r in rows {
        match r {
            Value::Array(cells) => {
                let row: Vec<Btn> = cells.iter().filter_map(one).collect();
                if !row.is_empty() {
                    out.push(row);
                }
            }
            other => {
                if let Some(b) = one(other) {
                    out.push(vec![b]);
                }
            }
        }
    }
    out
}

fn parse_poll(v: Option<&Value>) -> Option<Poll> {
    let o = v?.as_object()?;
    let question = o.get("question")?.as_str()?.trim().to_string();
    let options: Vec<PollOpt> = o
        .get("options")?
        .as_array()?
        .iter()
        .filter_map(|x| x.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .take(10)
        .map(|s| PollOpt {
            text: s.into(),
            voters: vec![],
        })
        .collect();
    if question.is_empty() || options.len() < 2 {
        return None;
    }
    Some(Poll {
        question,
        options,
        multi: o.get("multi").and_then(|x| x.as_bool()).unwrap_or(false),
        closed: false,
    })
}

fn clip_text(s: &str) -> String {
    if s.len() <= MAX_TEXT {
        return s.to_string();
    }
    let mut end = MAX_TEXT;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

// ---- the web bus: what a client sees ----

/// `hello` facts for a new session: the chat's revision, its floor, whether bia types.
pub fn hello_info(identity: &str) -> Value {
    let Ok(h) = hub() else {
        return json!({"rev": 0, "floor": 0, "typing": false});
    };
    let (rev, floor, muted) = h.with_user(identity, |u| (u.meta.rev, u.meta.floor, u.meta.muted));
    json!({"rev": rev, "floor": floor, "muted": muted, "typing": h.typing.lock().unwrap().contains(identity)})
}

/// The catch-up for a client that last saw revision `rx`: `m` and `gone` frames in
/// revision order, or None when it is behind the floor (it must reload).
pub fn since(identity: &str, rx: u64) -> Option<Vec<Value>> {
    let h = hub().ok()?;
    h.with_user(identity, |u| {
        if rx < u.meta.floor {
            return None;
        }
        let n = u.msgs.len();
        if u.ensure_fids(&h.files, 0..n) {
            u.save();
        }
        let mut out: Vec<(u64, Value)> = u
            .msgs
            .iter()
            .filter(|m| m.rev > rx)
            .map(|m| (m.rev, json!({"t": "m", "seq": m.rev, "m": m})))
            .collect();
        let mut by_rev: BTreeMap<u64, Vec<String>> = BTreeMap::new();
        for g in u.meta.gone.iter().filter(|g| g.rev > rx) {
            by_rev.entry(g.rev).or_default().push(g.id.clone());
        }
        for (rev, ids) in by_rev {
            out.push((rev, json!({"t": "gone", "seq": rev, "ids": ids})));
        }
        out.sort_by_key(|(r, _)| *r);
        Some(out.into_iter().map(|(_, v)| v).collect())
    })
}

/// Page of messages: newest `limit` before `before`, after `after`, or centered on `around`.
fn page(
    identity: &str,
    before: Option<&str>,
    after: Option<&str>,
    around: Option<&str>,
    limit: usize,
) -> Result<Value, String> {
    let h = hub()?;
    let limit = limit.clamp(1, 200);
    h.with_user(identity, |u| {
        let len = u.msgs.len();
        let pos = |id: &str| u.msgs.iter().position(|m| m.id == id);
        let (start, end) = if let Some(a) = around.and_then(pos) {
            let s = a.saturating_sub(limit / 2);
            (s, (s + limit).min(len))
        } else if let Some(b) = before {
            let e = pos(b).unwrap_or(0);
            (e.saturating_sub(limit), e)
        } else if let Some(a) = after {
            let s = pos(a).map(|i| i + 1).unwrap_or(len);
            (s, (s + limit).min(len))
        } else {
            (len.saturating_sub(limit), len)
        };
        if u.ensure_fids(&h.files, start..end) {
            u.save();
        }
        Ok(json!({
            "messages": &u.msgs[start..end],
            "more_before": start > 0,
            "more_after": end < len,
            "rev": u.meta.rev,
        }))
    })
}

fn search(identity: &str, q: &str) -> Result<Value, String> {
    let h = hub()?;
    let q = q.trim().to_lowercase();
    if q.is_empty() {
        return Ok(json!({"results": []}));
    }
    Ok(h.with_user(identity, |u| {
        let mut out = vec![];
        for m in u.msgs.iter().rev() {
            if m.service {
                continue;
            }
            let hit = m.text.to_lowercase().contains(&q)
                || m.files.iter().any(|a| a.name.to_lowercase().contains(&q));
            if hit {
                out.push(json!({"m": m}));
                if out.len() >= 200 {
                    break;
                }
            }
        }
        json!({"results": out})
    }))
}

/// A file of this chat, by its sha256: where it is (for the other side to pull it).
pub fn file_by_fid(identity: &str, fid: &str) -> Option<(PathBuf, u64)> {
    let h = hub().ok()?;
    let rel = h.with_user(identity, |u| {
        u.msgs
            .iter()
            .flat_map(|m| m.files.iter())
            .find(|a| a.fid == fid && !a.pending && !a.path.is_empty())
            .map(|a| a.path.clone())
    })?;
    let abs = att_abs(&h.files, &rel)?;
    let size = std::fs::metadata(&abs).ok()?.len();
    Some((abs, size))
}

/// The file ids of one message.
pub fn message_fids(identity: &str, id: &str) -> Vec<String> {
    let Ok(h) = hub() else { return vec![] };
    h.with_user(identity, |u| {
        u.find(id)
            .map(|m| m.files.iter().map(|a| a.fid.clone()).filter(|f| !f.is_empty()).collect())
            .unwrap_or_default()
    })
}

/// Files the other side listed that are not here yet: `(fid, name, size)`, once each.
pub fn pending_pulls(identity: &str) -> Vec<(String, String, u64)> {
    let Ok(h) = hub() else { return vec![] };
    h.with_user(identity, |u| {
        let mut seen = HashSet::new();
        u.msgs
            .iter()
            .filter(|m| m.from != BIA)
            .flat_map(|m| m.files.iter())
            .filter(|a| a.pending && seen.insert(a.fid.clone()))
            .map(|a| (a.fid.clone(), a.name.clone(), a.size))
            .collect()
    })
}

/// A pulled file is complete at `abs` (`Ok`) or did not come (`Err(why)`): the messages
/// listing it are updated, and those now complete reach the model.
pub fn file_settled(identity: &str, fid: &str, got: Result<&Path, &str>) {
    let Ok(h) = hub() else { return };
    let rel = match got {
        Ok(p) => Some(match p.strip_prefix(&h.files) {
            Ok(r) => r.to_string_lossy().to_string(),
            Err(_) => p.to_string_lossy().to_string(),
        }),
        Err(_) => None,
    };
    let (changed, ready) = h.with_user(identity, |u| {
        let mut changed = vec![];
        let mut ready = vec![];
        for i in 0..u.msgs.len() {
            let mut hit = false;
            for a in u.msgs[i].files.iter_mut().filter(|a| a.fid == fid && a.pending) {
                a.pending = false;
                match (&rel, got) {
                    (Some(r), _) => a.path = r.clone(),
                    (None, Err(why)) => a.err = why.to_string(),
                    _ => {}
                }
                hit = true;
            }
            if !hit {
                continue;
            }
            if u.msgs[i].wait && u.msgs[i].files.iter().all(|a| !a.pending) {
                u.msgs[i].wait = false;
                ready.push(u.msgs[i].id.clone());
            }
            changed.push(u.touch(i));
        }
        if !changed.is_empty() {
            u.save();
        }
        (changed, ready)
    });
    for m in &changed {
        h.emit_m(identity, m);
    }
    for id in ready {
        deliver_stored(h, identity, &id);
    }
}

/// One action from a client of the web bus (`t` names it). Returns the `ack` payload.
/// `cid` (when given) makes it apply once even if it is sent again.
pub fn action(identity: &str, root: bool, v: &Value) -> Result<Value, String> {
    let h = hub()?;
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let ids = |k: &str| -> Vec<String> {
        v.get(k)
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default()
    };
    let t = s("t");
    let cid = s("cid");
    let once = !cid.is_empty() && !matches!(t.as_str(), "hist" | "search");
    if once && h.with_user(identity, |u| u.meta.cids.contains(&cid)) {
        return Ok(json!({"dup": true}));
    }
    let r = match t.as_str() {
        "call_decline" => {
            crate::call::decline(identity, MAIN);
            Ok(json!({}))
        }
        "hist" => {
            let o = |k: &str| v.get(k).and_then(|x| x.as_str()).filter(|x| !x.is_empty());
            let limit = v.get("limit").and_then(|x| x.as_u64()).unwrap_or(60) as usize;
            page(identity, o("before"), o("after"), o("around"), limit)
        }
        "search" => search(identity, &s("q")),
        "send" => {
            let m = client_send(h, identity, root, v)?;
            Ok(json!({"id": m.id, "rev": m.rev}))
        }
        "edit" => {
            let id = s("id");
            let text = clip_text(&s("text"));
            let r = h.with_user(identity, |u| -> Result<Option<Msg>, String> {
                let i = u.pos(&id).ok_or("no such message")?;
                if u.msgs[i].from != identity {
                    return Err("you can only edit your own messages".into());
                }
                if text.trim().is_empty() && u.msgs[i].files.is_empty() {
                    return Err("message would be empty".into());
                }
                if u.msgs[i].text == text {
                    return Ok(None);
                }
                u.msgs[i].text = text.clone();
                u.msgs[i].edited = now_ms();
                let m = u.touch(i);
                u.save();
                Ok(Some(m))
            })?;
            if let Some(m) = r {
                h.emit_m(identity, &m);
                h.to_agent(identity, &mint_msg_id(), format!("[edited {id}]\n{text}"));
            }
            Ok(json!({"ok": true}))
        }
        "delete" => {
            let list = ids("ids");
            let (gone, rev) = h.with_user(identity, |u| {
                let r = u.remove(&list);
                if !r.0.is_empty() {
                    u.save();
                }
                r
            });
            if !gone.is_empty() {
                h.emit_gone(identity, rev, &gone);
                h.to_agent(identity, &mint_msg_id(), format!("[deleted] {}", gone.join(", ")));
            }
            Ok(json!({"deleted": gone}))
        }
        "react" => {
            let id = s("id");
            let emoji = s("emoji");
            if emoji.is_empty() || emoji.chars().count() > 8 {
                return Err("bad emoji".into());
            }
            let on = v.get("on").and_then(|x| x.as_bool());
            let r = h.with_user(identity, |u| -> Result<Option<(Msg, bool)>, String> {
                let i = u.pos(&id).ok_or("no such message")?;
                let who = u.msgs[i].reactions.entry(emoji.clone()).or_default();
                let has = who.iter().any(|w| w == identity);
                let want = on.unwrap_or(!has);
                if want == has {
                    u.msgs[i].reactions.retain(|_, w| !w.is_empty());
                    return Ok(None);
                }
                if want {
                    who.push(identity.to_string());
                } else {
                    who.retain(|w| w != identity);
                }
                u.msgs[i].reactions.retain(|_, w| !w.is_empty());
                let m = u.touch(i);
                u.save();
                Ok(Some((m, want)))
            })?;
            if let Some((m, added)) = r {
                h.emit_m(identity, &m);
                let note = if added {
                    format!("[reaction] {emoji} on {id} (\"{}\")", snippet(&m))
                } else {
                    format!("[reaction] removed {emoji} on {id}")
                };
                h.to_agent(identity, &mint_msg_id(), note);
            }
            Ok(json!({"ok": true}))
        }
        "pin" => {
            let on = v.get("on").and_then(|x| x.as_bool()).unwrap_or(true);
            if let Some(m) = set_pin(h, identity, &s("id"), on)? {
                h.emit_m(identity, &m);
            }
            Ok(json!({"ok": true}))
        }
        "read" => {
            let upto = s("upto");
            let changed = h.with_user(identity, |u| {
                let end = u.pos(&upto).map(|i| i + 1).unwrap_or(u.msgs.len());
                let mut changed = vec![];
                for i in 0..end {
                    if u.msgs[i].from == BIA && !u.msgs[i].read {
                        u.msgs[i].read = true;
                        changed.push(u.touch(i));
                    }
                }
                if !changed.is_empty() {
                    u.save();
                }
                changed
            });
            for m in &changed {
                h.emit_m(identity, m);
            }
            Ok(json!({"ok": true}))
        }
        "mute" => {
            let on = v.get("on").and_then(|x| x.as_bool()).unwrap_or(true);
            h.with_user(identity, |u| {
                u.meta.muted = on;
                u.save();
            });
            Ok(json!({"muted": on}))
        }
        "button" => {
            let id = s("id");
            let row = v.get("row").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
            let col = v.get("col").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
            let (b, snip) = h.with_user(identity, |u| -> Result<(Btn, String), String> {
                let m = u.find(&id).ok_or("no such message")?;
                let b = m
                    .buttons
                    .get(row)
                    .and_then(|r| r.get(col))
                    .cloned()
                    .ok_or("no such button")?;
                Ok((b, snippet(m)))
            })?;
            let data = if b.data.is_empty() {
                String::new()
            } else {
                format!(" data={}", b.data)
            };
            h.to_agent(
                identity,
                &mint_msg_id(),
                format!("[button] \"{}\"{data} on {id} (\"{snip}\")", b.text),
            );
            Ok(json!({"ok": true}))
        }
        "vote" => {
            let id = s("id");
            let picks: Vec<usize> = v
                .get("options")
                .and_then(|x| x.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_u64())
                        .map(|x| x as usize)
                        .collect()
                })
                .unwrap_or_default();
            let (m, note) = h.with_user(identity, |u| -> Result<(Msg, String), String> {
                let i = u.pos(&id).ok_or("no such message")?;
                let p = u.msgs[i].poll.as_mut().ok_or("not a poll")?;
                if p.closed {
                    return Err("poll is closed".into());
                }
                if !p.multi && picks.len() > 1 {
                    return Err("pick one option".into());
                }
                for o in p.options.iter_mut() {
                    o.voters.retain(|w| w != identity);
                }
                let mut chosen = vec![];
                for &k in &picks {
                    if let Some(o) = p.options.get_mut(k) {
                        o.voters.push(identity.to_string());
                        chosen.push(format!("\"{}\"", o.text));
                    }
                }
                let note = if chosen.is_empty() {
                    format!("[vote] retracted in poll {id} \"{}\"", p.question)
                } else {
                    format!(
                        "[vote] {} in poll {id} \"{}\"",
                        chosen.join(", "),
                        p.question
                    )
                };
                let m = u.touch(i);
                u.save();
                Ok((m, note))
            })?;
            h.emit_m(identity, &m);
            h.to_agent(identity, &mint_msg_id(), note);
            Ok(json!({"ok": true}))
        }
        "close_poll" => {
            let id = s("id");
            let m = h.with_user(identity, |u| -> Result<Msg, String> {
                let i = u.pos(&id).ok_or("no such message")?;
                if u.msgs[i].from != identity {
                    return Err("only the one who sent a poll closes it".into());
                }
                u.msgs[i].poll.as_mut().ok_or("not a poll")?.closed = true;
                let m = u.touch(i);
                u.save();
                Ok(m)
            })?;
            h.emit_m(identity, &m);
            Ok(json!({"ok": true}))
        }
        other => Err(format!("unknown action {other}")),
    };
    if once && r.is_ok() {
        h.with_user(identity, |u| {
            u.meta.cids.push_back(cid);
            while u.meta.cids.len() > KEEP_CIDS {
                u.meta.cids.pop_front();
            }
            u.save();
        });
    }
    r
}

fn set_pin(h: &Hub, identity: &str, id: &str, on: bool) -> Result<Option<Msg>, String> {
    h.with_user(identity, |u| -> Result<Option<Msg>, String> {
        let i = u.pos(id).ok_or("no such message")?;
        if u.msgs[i].pinned == on {
            return Ok(None);
        }
        u.msgs[i].pinned = on;
        let m = u.touch(i);
        u.save();
        Ok(Some(m))
    })
}

/// `send` from a client: its own message id (so a resend is recognised), its files
/// listed to be pulled from it, root's files already on this machine (`paths`).
fn client_send(h: &Hub, identity: &str, root: bool, v: &Value) -> Result<Msg, String> {
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let id = Some(s("id")).filter(|i| !i.is_empty());
    if let Some(i) = &id {
        if !valid_msg_id(i) {
            return Err(format!("bad message id {i}"));
        }
        if let Some(m) = h.with_user(identity, |u| u.find(i).cloned()) {
            return Ok(m);
        }
    }
    let mut atts = vec![];
    for f in v.get("files").and_then(|x| x.as_array()).into_iter().flatten().take(32) {
        atts.push(remote_att(f)?);
    }
    let kind = Some(s("kind")).filter(|x| !x.is_empty());
    for p in v.get("paths").and_then(|x| x.as_array()).into_iter().flatten() {
        let p = p.as_str().unwrap_or("").trim();
        if !root {
            return Err(format!("{p}: only root sends files that are already on this machine"));
        }
        atts.push(make_att(&h.files, p, kind.as_deref())?);
    }
    let text = clip_text(&s("text"));
    let poll = match v.get("poll") {
        Some(p) if !p.is_null() => Some(parse_poll(Some(p)).ok_or("poll needs {question, options: [at least 2 strings], multi?}")?),
        _ => None,
    };
    if text.trim().is_empty() && atts.is_empty() && poll.is_none() {
        return Err("empty message".into());
    }
    let wait = atts.iter().any(|a| a.pending);
    let m = Msg {
        id: id.unwrap_or_else(mint_msg_id),
        from: identity.to_string(),
        ts: now_ms(),
        text,
        reply_to: Some(s("reply_to")).filter(|x| !x.is_empty()),
        files: atts,
        buttons: parse_buttons(v.get("buttons")),
        poll,
        wait,
        ..Default::default()
    };
    let m = h.with_user(identity, |u| {
        let m = u.insert(m);
        u.prune();
        u.save();
        m
    });
    h.emit_m(identity, &m);
    if !m.wait {
        deliver_stored(h, identity, &m.id);
    }
    Ok(m)
}

/// A stored message from the identity goes to the model (voice transcribed first), in
/// order with the identity's other messages.
fn deliver_stored(h: &Hub, identity: &str, id: &str) {
    let Some((note, voice)) = h.with_user(identity, |u| {
        let m = u.find(id)?;
        let voice: Vec<(String, PathBuf)> = m
            .files
            .iter()
            .filter(|a| a.kind == "voice" || a.kind == "audio")
            .filter_map(|a| att_abs(&h.files, &a.path).filter(|_| !a.path.is_empty()).map(|abs| (a.path.clone(), abs)))
            .collect();
        Some((agent_text(u, m), voice))
    }) else {
        return;
    };
    deliver_later(Delivery {
        turn: lanes().reserve(identity),
        identity: identity.to_string(),
        id: id.to_string(),
        note,
        voice,
    });
}

/// A message on its way to the model. Voice and audio files are transcribed first, on a
/// thread of the message's own; `turn` keeps an identity's messages in order without
/// others waiting for them.
struct Delivery {
    turn: bia_mcp::lanes::Turn,
    identity: String,
    id: String,
    note: String,
    /// (stored path, absolute path) of voice/audio attachments
    voice: Vec<(String, PathBuf)>,
}

fn deliver_later(d: Delivery) {
    let d = Arc::new(Mutex::new(Some(d)));
    let mine = d.clone();
    let spawned = std::thread::Builder::new()
        .name("web-deliver".into())
        .spawn(move || mine.lock().unwrap().take().map(deliver));
    if spawned.is_err() {
        if let Some(d) = d.lock().unwrap().take() {
            deliver(d);
        }
    }
}

/// Web messages to the model, in order per identity (see [`bia_mcp::lanes`]).
fn lanes() -> &'static bia_mcp::lanes::Lanes {
    static L: std::sync::LazyLock<bia_mcp::lanes::Lanes> = std::sync::LazyLock::new(Default::default);
    &L
}

fn deliver(mut d: Delivery) {
    let Ok(h) = hub() else { return };
    let mut note = d.note;
    if !d.voice.is_empty() {
        let (mut heads, mut tails) = (Vec::new(), Vec::new());
        for (rel, abs) in &d.voice {
            match bia_speech::transcribe_note_from(
                abs,
                bia_speech::contacts::lang_for_identity(&d.identity).as_deref(),
            ) {
                Ok(heard) => {
                    if !heard.runs.is_empty() {
                        // under the bubble: just the words, not the model's language tags
                        let shown: Vec<&str> = heard.runs.iter().map(|r| r.text.as_str()).collect();
                        h.set_transcript(&d.identity, &d.id, rel, &shown.join(" "));
                    }
                    heads.push(heard.voice_head());
                    if let Some(n) = heard.notice {
                        tails.push(n);
                    }
                }
                Err(e) => heads.push(format!("[voice message, transcription failed: {e}]")),
            }
        }
        note = format!("{}\n{note}", heads.join("\n"));
        for t in tails {
            note.push('\n');
            note.push_str(&t);
        }
    }
    d.turn.wait_blocking();
    h.to_agent(&d.identity, &d.id, note);
}

// ---- another bia's copy of the chat (this bia dialed it) ----

/// Apply a message state `m` from the bia `peer` (its store's view: `bia` is the peer,
/// `me_there` is this bia). Fields the peer owns are taken: its own messages' text,
/// files and edits, its reactions and votes, read marks on our messages. Something new
/// from the peer reaches the model (a message once its files are here). Pins stay local.
pub fn apply_remote(peer: &str, me_there: &str, m: &Value) -> Result<(), String> {
    let h = hub()?;
    let mut r: Msg = serde_json::from_value(m.clone()).map_err(|e| format!("bad message state: {e}"))?;
    if !valid_msg_id(&r.id) {
        return Err(format!("bad message id {}", r.id));
    }
    let here = |n: &str| name_here(n, peer, me_there).to_string();
    r.from = here(&r.from);
    for who in r.reactions.values_mut() {
        for w in who.iter_mut() {
            *w = here(w);
        }
    }
    if let Some(p) = r.poll.as_mut() {
        for o in p.options.iter_mut() {
            for w in o.voters.iter_mut() {
                *w = here(w);
            }
        }
    }
    let theirs = r.from != BIA;
    let notes = h.with_user(peer, |u| -> Vec<(String, String)> {
        let mut notes = vec![];
        match u.pos(&r.id) {
            None => {
                let mut m = Msg {
                    id: r.id.clone(),
                    from: r.from.clone(),
                    ts: r.ts,
                    text: clip_text(&r.text),
                    reply_to: r.reply_to.clone(),
                    fwd: r.fwd.clone(),
                    reactions: r.reactions.clone(),
                    edited: r.edited,
                    read: if theirs { false } else { r.read },
                    buttons: r.buttons.clone(),
                    poll: r.poll.clone(),
                    service: r.service,
                    ..Default::default()
                };
                for a in &r.files {
                    if unhex32(&a.fid).is_none() || a.size > MAX_FILE {
                        m.files.push(Att { name: safe_file_name(&a.name), size: a.size, mime: a.mime.clone(), kind: a.kind.clone(), err: "no usable file id".into(), ..Default::default() });
                        continue;
                    }
                    m.files.push(Att {
                        path: String::new(),
                        name: safe_file_name(&a.name),
                        size: a.size,
                        mime: mime_for(&a.name).to_string(),
                        kind: a.kind.clone(),
                        transcript: a.transcript.clone(),
                        fid: a.fid.to_ascii_lowercase(),
                        pending: true,
                        err: String::new(),
                    });
                }
                m.wait = theirs && m.files.iter().any(|a| a.pending);
                // our own message we no longer had (a reinstall): keep it, nothing to tell
                let stored = u.insert(m);
                if theirs && !stored.wait {
                    notes.push(("deliver".into(), stored.id.clone()));
                }
            }
            Some(i) => {
                let l = &mut u.msgs[i];
                let before = l.clone();
                if l.from != BIA {
                    if l.text != r.text && r.edited > l.edited {
                        notes.push(("note".into(), format!("[edited {}]\n{}", l.id, r.text)));
                    }
                    l.text = clip_text(&r.text);
                    l.edited = r.edited;
                    l.buttons = r.buttons.clone();
                    if let (Some(lp), Some(rp)) = (l.poll.as_mut(), r.poll.as_ref()) {
                        lp.closed = rp.closed;
                    }
                    for (la, ra) in l.files.iter_mut().zip(r.files.iter()) {
                        if la.fid == ra.fid && !ra.transcript.is_empty() {
                            la.transcript = ra.transcript.clone();
                        }
                    }
                } else if r.read {
                    l.read = true;
                }
                // the peer's reactions and votes
                let mut added = vec![];
                for who in l.reactions.values_mut() {
                    who.retain(|w| w != peer);
                }
                for (e, who) in &r.reactions {
                    if who.iter().any(|w| w == peer) {
                        if !before.reactions.get(e).is_some_and(|b| b.iter().any(|w| w == peer)) {
                            added.push(e.clone());
                        }
                        l.reactions.entry(e.clone()).or_default().push(peer.to_string());
                    }
                }
                l.reactions.retain(|_, w| !w.is_empty());
                if let (Some(lp), Some(rp)) = (l.poll.as_mut(), r.poll.as_ref()) {
                    for (lo, ro) in lp.options.iter_mut().zip(rp.options.iter()) {
                        lo.voters.retain(|w| w != peer);
                        if ro.voters.iter().any(|w| w == peer) {
                            lo.voters.push(peer.to_string());
                        }
                    }
                }
                for e in added {
                    let snip = snippet(l);
                    notes.push(("note".into(), format!("[reaction] {e} on {} (\"{snip}\")", l.id)));
                }
                if *l != before {
                    u.touch(i);
                }
            }
        }
        u.prune();
        u.save();
        notes
    });
    for (kind, x) in notes {
        if kind == "deliver" {
            deliver_stored(h, peer, &x);
        } else {
            h.to_agent(peer, &mint_msg_id(), x);
        }
    }
    Ok(())
}

/// The peer deleted messages in its copy of the chat (this bia dialed it).
pub fn apply_remote_gone(peer: &str, ids: &[String]) {
    let Ok(h) = hub() else { return };
    let gone = h.with_user(peer, |u| {
        let (gone, _) = u.remove(ids);
        if !gone.is_empty() {
            u.save();
        }
        gone
    });
    if !gone.is_empty() {
        h.to_agent(peer, &mint_msg_id(), format!("[deleted] {}", gone.join(", ")));
    }
}

/// Bring in the chat log of a peer link from before 1.9 (`state/peers/<name>/chat.jsonl`)
/// as that peer's web chat, once.
pub fn import_peer_log(peer: &str, log: &Path) -> usize {
    let Ok(h) = hub() else { return 0 };
    let mut view: Vec<Msg> = vec![];
    for l in std::fs::read_to_string(log).unwrap_or_default().lines() {
        let Ok(v) = serde_json::from_str::<Value>(l) else { continue };
        let id = v["id"].as_str().unwrap_or("").to_string();
        let from = match v["from"].as_str().unwrap_or("") {
            "me" => BIA.to_string(),
            f => f.to_string(),
        };
        let text = v["text"].as_str().unwrap_or("").to_string();
        match v["kind"].as_str().unwrap_or("msg") {
            "msg" if !id.is_empty() => {
                let files = v["files"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|p| p.as_str())
                    .filter_map(|p| make_att(&h.files, p, None).ok())
                    .collect();
                view.push(Msg {
                    id,
                    from,
                    ts: v["ts"].as_u64().unwrap_or(0) * 1000,
                    text,
                    reply_to: v["reply_to"].as_str().map(str::to_string),
                    files,
                    read: true,
                    ..Default::default()
                });
            }
            "edit" => {
                if let Some(m) = view.iter_mut().find(|m| m.id == id) {
                    m.text = text;
                    m.edited = v["ts"].as_u64().unwrap_or(0) * 1000;
                }
            }
            "del" => {
                let gone: Vec<&str> = v["ids"].as_array().into_iter().flatten().filter_map(|x| x.as_str()).collect();
                view.retain(|m| !gone.contains(&m.id.as_str()));
            }
            _ => {}
        }
    }
    h.with_user(peer, |u| {
        let mut n = 0;
        for m in view {
            if u.pos(&m.id).is_none() {
                u.insert(m);
                n += 1;
            }
        }
        if n > 0 {
            u.prune();
            u.save();
        }
        n
    })
}

// ---- model side ----

/// Someone who can have a web chat: root, an identity with an active client certificate
/// (a person, another bia, an agent), or one that already has a chat. Never bia itself —
/// bia is the other side of every web chat.
pub fn known_identity(identity: &str) -> bool {
    let id = identity.trim();
    if id.is_empty() || id == BIA {
        return false;
    }
    if id == crate::bus::ID_ROOT {
        return true;
    }
    let Ok(h) = hub() else { return false };
    if h.users.lock().map(|u| u.contains_key(id)).unwrap_or(false) || h.user_dir(id).is_dir() {
        return true;
    }
    bia_pki::Pki::load_or_create(&h.files)
        .map(|p| p.has_active_identity(id))
        .unwrap_or(false)
}

/// Identities that have web chats on disk (or in memory), sorted.
pub fn users() -> Vec<String> {
    let Ok(h) = hub() else { return vec![] };
    let mut out: Vec<String> = h
        .users
        .lock()
        .map(|u| u.keys().cloned().collect())
        .unwrap_or_default();
    if let Ok(rd) = std::fs::read_dir(h.files.join("state").join("web")) {
        for e in rd.flatten() {
            // a chat folder (state/web also holds the push store)
            if e.path().join("main.json").exists() || e.path().join("meta.json").exists() {
                let n = e.file_name().to_string_lossy().to_string();
                if n != BIA && !out.contains(&n) {
                    out.push(n);
                }
            }
        }
    }
    out.sort();
    out
}

/// Messages from the identity the model has not read yet.
pub fn unread(identity: &str) -> usize {
    let Ok(h) = hub() else { return 0 };
    h.with_user(identity, |u| {
        u.msgs
            .iter()
            .filter(|m| m.from != BIA && !m.read && !m.service)
            .count()
    })
}

/// A change this bia made in `identity`'s chat, for an identity this bia dials (another
/// bia): it also goes out as an action when this bia is the client.
fn outgoing(identity: &str, action: Value, rev: u64) {
    crate::link::local_change(identity, action, rev);
}

/// The model reads a message: mark it (and older ones) read (✓✓ for the other side).
pub fn agent_read(identity: &str, id: &str) {
    let Ok(h) = hub() else { return };
    let changed = h.with_user(identity, |u| {
        let Some(end) = u.pos(id) else {
            return vec![];
        };
        let mut changed = vec![];
        for i in 0..=end {
            if u.msgs[i].from != BIA && !u.msgs[i].read {
                u.msgs[i].read = true;
                changed.push(u.touch(i));
            }
        }
        if !changed.is_empty() {
            u.save();
        }
        changed
    });
    for m in &changed {
        h.emit_m(identity, m);
    }
    if let Some(last) = changed.last() {
        outgoing(identity, json!({"t": "read", "upto": id}), last.rev);
    }
}

/// Bia's typing indicator in the identity's chat.
pub fn typing(identity: &str, on: bool) {
    let Ok(h) = hub() else { return };
    {
        let mut t = h.typing.lock().unwrap();
        if on {
            t.insert(identity.to_string());
        } else {
            t.remove(identity);
        }
    }
    crate::link::emit(identity, json!({"t": "typing", "on": on}));
}

/// Store a message from bia, send it to the identity, and wake them.
fn post(h: &Hub, identity: &str, m: Msg) -> Msg {
    let m = h.with_user(identity, |u| {
        let m = u.insert(m);
        u.prune();
        u.save();
        m
    });
    h.emit_m(identity, &m);
    push_msg(h, identity, &m);
    let mut a = json!({"t": "send", "id": m.id, "text": m.text, "files": m.files.iter().map(att_offer).collect::<Vec<_>>()});
    if let Some(r) = &m.reply_to {
        a["reply_to"] = json!(r);
    }
    if !m.buttons.is_empty() {
        a["buttons"] = json!(m.buttons);
    }
    if let Some(p) = &m.poll {
        a["poll"] = json!({"question": p.question, "options": p.options.iter().map(|o| o.text.clone()).collect::<Vec<_>>(), "multi": p.multi});
    }
    outgoing(identity, a, m.rev);
    m
}

/// `say` to an identity's web chat. Returns the new message id.
pub fn say(
    identity: &str,
    text: &str,
    reply_to: Option<&str>,
    files: &[String],
    buttons: Option<&Value>,
    poll: Option<&Value>,
) -> Result<String, String> {
    let h = hub()?;
    let mut atts = vec![];
    for p in files {
        atts.push(make_att(&h.files, p, None)?);
    }
    let poll = match poll {
        Some(p) if !p.is_null() => Some(
            parse_poll(Some(p))
                .ok_or("poll needs {question, options: [at least 2 strings], multi?}")?,
        ),
        _ => None,
    };
    let text = clip_text(text);
    if text.trim().is_empty() && atts.is_empty() && poll.is_none() {
        return Err("message, files or poll required".into());
    }
    let reply_to = reply_to
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    if let Some(r) = &reply_to {
        if !h.with_user(identity, |u| u.find(r).is_some()) {
            return Err(format!("reply_to {r}: no such message in {identity}'s web chat"));
        }
    }
    let m = post(
        h,
        identity,
        Msg {
            id: mint_msg_id(),
            from: BIA.into(),
            ts: now_ms(),
            text,
            reply_to,
            files: atts,
            buttons: parse_buttons(buttons),
            poll,
            ..Default::default()
        },
    );
    Ok(m.id)
}

// ---- for web calls (call.rs) ----

pub fn emit_event(identity: &str, v: Value) {
    crate::link::emit(identity, v);
}

pub fn has_client(identity: &str) -> bool {
    crate::link::online(identity)
}

/// The one chat (there is nothing to create).
pub fn ensure_chat(identity: &str, _channel: &str) -> Result<String, String> {
    if known_identity(identity) {
        Ok(MAIN.into())
    } else {
        Err(format!("no web user {identity}"))
    }
}

/// A call event for the model (`system` identity on web).
pub fn agent_notice(identity: &str, _chat: &str, text: &str) {
    let Ok(h) = hub() else { return };
    h.con.push_inbound(Inbound {
        identity: crate::bus::ID_SYSTEM.into(),
        channel: crate::bus::CH_WEB.into(),
        id: mint_msg_id(),
        text: format!("{text} (web call with {identity})"),
    });
}

/// What the person said on a web call, as their message.
pub fn agent_speech(identity: &str, _chat: &str, text: &str) {
    if let Ok(h) = hub() {
        h.to_agent(identity, &mint_msg_id(), text.to_string());
    }
}

/// All message texts of a chat (tests).
#[cfg(test)]
pub fn chat_text(identity: &str) -> String {
    hub()
        .map(|h| {
            h.with_user(identity, |u| {
                u.msgs
                    .iter()
                    .map(|m| m.text.clone())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
        })
        .unwrap_or_default()
}

/// Post a text message from bia (the call summary).
pub fn post_bia(identity: &str, _chat: &str, text: &str) -> Result<String, String> {
    say(identity, text, None, &[], None, None)
}

/// Speak `text` (in `lang`) and post it as a voice message from bia.
pub fn say_voice(
    identity: &str,
    text: &str,
    lang: &str,
    reply_to: Option<&str>,
) -> Result<String, String> {
    let h = hub()?;
    let text = text.trim();
    if text.is_empty() {
        return Err("message is the text to speak".into());
    }
    let reply_to = reply_to
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    if let Some(r) = &reply_to {
        if !h.with_user(identity, |u| u.find(r).is_some()) {
            return Err(format!("reply_to {r}: no such message in {identity}'s web chat"));
        }
    }
    let dir = h.files.join("speech/out");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let out = dir.join(format!("voice-{}.ogg", bia_speech::paths::stamp()));
    let preferred = bia_speech::contacts::lang_for_identity(identity).unwrap_or_default();
    let lang = if lang.trim().is_empty() {
        preferred.as_str()
    } else {
        lang
    };
    bia_speech::tts::voice_note(text, lang, &out)?;
    let mut att = make_att(&h.files, &out.to_string_lossy(), Some("voice"))?;
    att.transcript = text.to_string();
    let m = post(
        h,
        identity,
        Msg {
            id: mint_msg_id(),
            from: BIA.into(),
            ts: now_ms(),
            reply_to,
            files: vec![att],
            ..Default::default()
        },
    );
    Ok(m.id)
}

pub fn agent_edit(identity: &str, id: &str, text: &str) -> Result<(), String> {
    let h = hub()?;
    let text = clip_text(text);
    let m = h.with_user(identity, |u| -> Result<Msg, String> {
        let i = u
            .pos(id)
            .ok_or_else(|| format!("no message {id} in {identity}'s web chat"))?;
        if u.msgs[i].from != BIA {
            return Err("you can only edit your own messages".into());
        }
        u.msgs[i].text = text.clone();
        u.msgs[i].edited = now_ms();
        let m = u.touch(i);
        u.save();
        Ok(m)
    })?;
    h.emit_m(identity, &m);
    outgoing(identity, json!({"t": "edit", "id": id, "text": text}), m.rev);
    Ok(())
}

pub fn agent_delete(identity: &str, ids: &[String]) -> Result<usize, String> {
    let h = hub()?;
    let (gone, rev) = h.with_user(identity, |u| {
        let r = u.remove(ids);
        if !r.0.is_empty() {
            u.save();
        }
        r
    });
    if !gone.is_empty() {
        h.emit_gone(identity, rev, &gone);
        outgoing(identity, json!({"t": "delete", "ids": gone}), rev);
    }
    Ok(gone.len())
}

/// Set bia's reaction on a message (one at a time, like a regular account). Empty removes it.
pub fn agent_react(identity: &str, id: &str, emoji: &str) -> Result<(), String> {
    let h = hub()?;
    let emoji = emoji.trim().to_string();
    let (m, old) = h.with_user(identity, |u| -> Result<(Msg, Vec<String>), String> {
        let i = u
            .pos(id)
            .ok_or_else(|| format!("no message {id} in {identity}'s web chat"))?;
        let m = &mut u.msgs[i];
        let old: Vec<String> = m
            .reactions
            .iter()
            .filter(|(e, w)| **e != emoji && w.iter().any(|x| x == BIA))
            .map(|(e, _)| e.clone())
            .collect();
        for w in m.reactions.values_mut() {
            w.retain(|x| x != BIA);
        }
        if !emoji.is_empty() {
            m.reactions.entry(emoji.clone()).or_default().push(BIA.into());
        }
        m.reactions.retain(|_, w| !w.is_empty());
        let m = u.touch(i);
        u.save();
        Ok((m, old))
    })?;
    h.emit_m(identity, &m);
    for e in old {
        outgoing(identity, json!({"t": "react", "id": id, "emoji": e, "on": false}), m.rev);
    }
    if !emoji.is_empty() {
        outgoing(identity, json!({"t": "react", "id": id, "emoji": emoji, "on": true}), m.rev);
    }
    Ok(())
}

/// Pins are this bia's own (they stay here).
pub fn agent_pin(identity: &str, id: &str, on: bool) -> Result<(), String> {
    let h = hub()?;
    if let Some(m) = set_pin(h, identity, id, on)? {
        h.emit_m(identity, &m);
    }
    Ok(())
}

/// Close a poll the model sent (no more votes); returns the final tally.
pub fn agent_close_poll(identity: &str, id: &str) -> Result<String, String> {
    let h = hub()?;
    let m = h.with_user(identity, |u| -> Result<Msg, String> {
        let i = u
            .pos(id)
            .ok_or_else(|| format!("no message {id} in {identity}'s web chat"))?;
        u.msgs[i].poll.as_mut().ok_or("not a poll")?.closed = true;
        let m = u.touch(i);
        u.save();
        Ok(m)
    })?;
    h.emit_m(identity, &m);
    outgoing(identity, json!({"t": "close_poll", "id": id}), m.rev);
    let p = m.poll.unwrap_or_default();
    Ok(p.options
        .iter()
        .map(|o| format!("{}: {}", o.text, o.voters.len()))
        .collect::<Vec<_>>()
        .join(", "))
}

/// Recent messages of an identity's web chat for the model (`web_history`).
pub fn agent_history(identity: &str, limit: usize) -> Result<String, String> {
    let h = hub()?;
    h.with_user(identity, |u| {
        let v = &u.msgs;
        let start = v.len().saturating_sub(limit.clamp(1, 100));
        let lines: Vec<String> = v[start..]
            .iter()
            .rev()
            .map(|m| {
                let mut extra = vec![];
                if let Some(r) = &m.reply_to {
                    extra.push(format!("reply to {r}"));
                }
                if m.edited > 0 {
                    extra.push("edited".into());
                }
                if m.pinned {
                    extra.push("pinned".into());
                }
                for (e, w) in &m.reactions {
                    extra.push(format!("{e} {}", w.join(",")));
                }
                for a in &m.files {
                    if a.pending {
                        extra.push(format!("{} {} (arriving)", a.kind, a.name));
                    } else {
                        extra.push(format!("{} {}", a.kind, if a.path.is_empty() { &a.name } else { &a.path }));
                    }
                }
                if let Some(p) = &m.poll {
                    let tally: Vec<String> = p
                        .options
                        .iter()
                        .map(|o| format!("{}={}", o.text, o.voters.len()))
                        .collect();
                    extra.push(format!(
                        "poll \"{}\" {}{}",
                        p.question,
                        tally.join(" "),
                        if p.closed { " closed" } else { "" }
                    ));
                }
                let extra = if extra.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", extra.join("; "))
                };
                format!(
                    "{} {} {}:{extra} {}",
                    m.id,
                    crate::bus::rfc3339_unix(m.ts / 1000),
                    m.from,
                    snippet(m)
                )
            })
            .collect();
        if lines.is_empty() {
            Ok(format!("({identity}'s web chat is empty)"))
        } else {
            Ok(lines.join("\n"))
        }
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::Once;
    use std::time::Duration;

    /// One chat store per test process (it is a global, like in the server).
    pub(crate) fn setup() -> (Arc<Bus>, PathBuf) {
        static INIT: Once = Once::new();
        static mut BUS: Option<(Arc<Bus>, PathBuf)> = None;
        INIT.call_once(|| {
            let dir = std::env::temp_dir().join(format!("bia-mcp-web-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let bus = Arc::new(Bus::new(dir.clone()));
            init(&dir, bus.clone());
            unsafe { BUS = Some((bus, dir)) };
        });
        #[allow(static_mut_refs)]
        unsafe {
            BUS.clone().unwrap()
        }
    }

    /// The next inbound for `identity` (others are skipped).
    pub(crate) fn next_for(bus: &Bus, identity: &str) -> Inbound {
        bus.next_held_for(identity, Duration::from_secs(10)).expect("delivered")
    }

    #[test]
    fn old_chats_fold_into_one_and_get_revisions() {
        let (_, dir) = setup();
        let d = dir.join("state/web/olga");
        std::fs::create_dir_all(&d).unwrap();
        let m = |id: &str, ts: u64, text: &str| json!({"id": id, "from": "olga", "ts": ts, "text": text});
        std::fs::write(d.join("chats.json"), json!([{"id": "main", "title": "bia", "created": 1, "muted": true}, {"id": "c1", "title": "trip", "created": 2}]).to_string()).unwrap();
        std::fs::write(d.join("main.json"), json!([m("b1", 10, "one"), m("b3", 30, "three")]).to_string()).unwrap();
        std::fs::write(d.join("c1.json"), json!([m("b2", 20, "two")]).to_string()).unwrap();
        let h = hub().unwrap();
        let (texts, revs, muted) = h.with_user("olga", |u| {
            (u.msgs.iter().map(|m| m.text.clone()).collect::<Vec<_>>(), u.msgs.iter().map(|m| m.rev).collect::<Vec<_>>(), u.meta.muted)
        });
        assert_eq!(texts, ["one", "two", "three"]);
        assert_eq!(revs, [1, 2, 3]);
        assert!(muted, "the main chat's mute carries over");
        assert!(d.join("chats.json.migrated").exists() && d.join("c1.json.migrated").exists());
        assert!(!d.join("chats.json").exists());
    }

    #[test]
    fn catch_up_follows_revisions_and_tombstones() {
        let (bus, dir) = setup();
        std::fs::create_dir_all(dir.join("state/web/pat")).unwrap();
        let a = say("pat", "first", None, &[], None, None).unwrap();
        let b = say("pat", "second", None, &[], None, None).unwrap();
        let rev = hello_info("pat")["rev"].as_u64().unwrap();
        agent_edit("pat", &a, "first, edited").unwrap();
        agent_delete("pat", std::slice::from_ref(&b)).unwrap();
        let f = since("pat", rev).unwrap();
        assert_eq!(f.len(), 2, "{f:?}");
        assert_eq!((f[0]["t"].as_str(), f[0]["m"]["text"].as_str()), (Some("m"), Some("first, edited")));
        assert_eq!((f[1]["t"].as_str(), f[1]["ids"][0].as_str()), (Some("gone"), Some(b.as_str())));
        assert!(since("pat", 0).unwrap().len() >= 2);
        // a client behind the floor reloads
        hub().unwrap().with_user("pat", |u| u.meta.floor = rev + 5);
        assert!(since("pat", rev).is_none());
        let _ = bus;
    }

    #[test]
    fn client_actions_apply_once_and_wait_for_files() {
        let (bus, dir) = setup();
        std::fs::create_dir_all(dir.join("state/web/quinn")).unwrap();
        let fid = "ab".repeat(32);
        let send = json!({"t": "send", "cid": "c-1", "id": "wq-0001", "text": "see file", "files": [{"fid": fid, "name": "../x.txt", "size": 3}]});
        let r = action("quinn", false, &send).unwrap();
        assert_eq!(r["id"], "wq-0001");
        assert_eq!(action("quinn", false, &send).unwrap()["dup"], true, "the same cid applies once");
        // the same message id under another cid is recognised too
        let again = action("quinn", false, &json!({"t": "send", "cid": "c-2", "id": "wq-0001", "text": "see file"})).unwrap();
        assert_eq!(again["id"], "wq-0001");
        assert_eq!(pending_pulls("quinn"), [(fid.clone(), "x.txt".to_string(), 3)]);
        // nothing reaches the model before the file is here
        assert!(bus.next_held_for("quinn", Duration::from_millis(300)).is_none());
        let got = dir.join("inbox/web/quinn/x.txt");
        std::fs::create_dir_all(got.parent().unwrap()).unwrap();
        std::fs::write(&got, b"abc").unwrap();
        file_settled("quinn", &fid, Ok(&got));
        let ib = next_for(&bus, "quinn");
        assert!(ib.text.contains("see file") && ib.text.contains("inbox/web/quinn/x.txt"), "{}", ib.text);
        assert_eq!(ib.id, "wq-0001");
        assert!(pending_pulls("quinn").is_empty());
        assert_eq!(file_by_fid("quinn", &fid).map(|(p, n)| (p, n)), Some((got, 3)));
        // a root path is refused to a normal client
        let e = action("quinn", false, &json!({"t": "send", "paths": ["/etc/passwd"]})).unwrap_err();
        assert!(e.contains("only root"), "{e}");
        // set-style reactions: on twice is one reaction
        for _ in 0..2 {
            action("quinn", false, &json!({"t": "react", "id": "wq-0001", "emoji": "👍", "on": true})).unwrap();
        }
        let n = hub().unwrap().with_user("quinn", |u| u.find("wq-0001").unwrap().reactions["👍"].len());
        assert_eq!(n, 1);
    }

    #[test]
    fn a_peers_copy_merges_by_ownership() {
        let (bus, dir) = setup();
        std::fs::create_dir_all(dir.join("state/web/fred")).unwrap();
        // fred's store: its own message ("bia" there) and ours ("alex" there)
        let theirs = json!({"id": "bf-0001", "from": "bia", "ts": 1000, "text": "hi alex", "rev": 7});
        apply_remote("fred", "alex", &theirs).unwrap();
        let ib = next_for(&bus, "fred");
        assert_eq!((ib.channel.as_str(), ib.text.as_str()), ("web", "hi alex"));
        // the same state again: nothing new for the model
        apply_remote("fred", "alex", &theirs).unwrap();
        // fred edits and reacts on its own message
        apply_remote("fred", "alex", &json!({"id": "bf-0001", "from": "bia", "ts": 1000, "text": "hi alex!", "edited": 2000, "reactions": {"👍": ["bia"]}})).unwrap();
        let t: Vec<String> = (0..2).map(|_| next_for(&bus, "fred").text).collect();
        assert!(t.iter().any(|x| x.starts_with("[edited bf-0001]")) && t.iter().any(|x| x.starts_with("[reaction] 👍")), "{t:?}");
        // our message as fred holds it: only fred's read mark and reactions count
        let mine = say("fred", "hello fred", None, &[], None, None).unwrap();
        apply_remote("fred", "alex", &json!({"id": mine, "from": "alex", "ts": 1, "text": "tampered", "read": true, "reactions": {"❤️": ["bia", "alex"]}})).unwrap();
        let m = hub().unwrap().with_user("fred", |u| u.find(&mine).cloned().unwrap());
        assert_eq!(m.text, "hello fred", "our own text stays ours");
        assert!(m.read, "fred saw it");
        assert_eq!(m.reactions["❤️"], ["fred"], "fred's reaction only");
        let _ = next_for(&bus, "fred");
        apply_remote_gone("fred", &["bf-0001".to_string()]);
        assert!(next_for(&bus, "fred").text.starts_with("[deleted] bf-0001"));
    }

    #[test]
    fn peer_log_from_before_is_imported() {
        let (_, dir) = setup();
        let log = dir.join("peerlog.jsonl");
        std::fs::write(
            &log,
            [
                json!({"id": "p1", "from": "me", "ts": 5, "text": "hello", "kind": "msg"}),
                json!({"id": "p2", "from": "gus", "ts": 6, "text": "hi", "kind": "msg"}),
                json!({"id": "p1", "from": "me", "ts": 7, "text": "hello!", "kind": "edit"}),
            ]
            .map(|v| v.to_string())
            .join("\n"),
        )
        .unwrap();
        assert_eq!(import_peer_log("gus", &log), 2);
        assert_eq!(import_peer_log("gus", &log), 0, "once");
        let t = chat_text("gus");
        assert_eq!(t, "hello!\nhi");
    }
}

#[cfg(test)]
mod voice_tests {
    use super::*;
    use std::time::Duration;

    /// Real models: BIA_SPEECH_DIR=<bundled speech tree> cargo test -p bia-mcp-web -- --ignored voice
    #[test]
    #[ignore = "needs the speech models (BIA_SPEECH_DIR)"]
    fn voice_both_ways() {
        let (con, dir) = tests::setup();
        std::env::set_var("BIA_SPEECH_STORE", dir.join("speech"));
        let h = hub().unwrap();

        // bia answers with a voice message
        let said = "Good morning. I will call you back at eight tonight.";
        assert!(say_voice("root", said, "", None)
            .unwrap_err()
            .contains("lang is required"));
        let id = say_voice("root", said, "en", None).unwrap();
        let att = h
            .with_user("root", |u| u.find(&id).map(|m| m.files[0].clone()))
            .unwrap();
        assert_eq!(
            (att.kind.as_str(), att.transcript.as_str()),
            ("voice", said)
        );
        let abs = att_abs(&dir, &att.path).unwrap();
        assert!(std::fs::metadata(&abs).unwrap().len() > 1000, "ogg written");

        // the user sends that audio back as their own voice message
        let rel = format!("{}/voice-test.ogg", inbox_rel("root"));
        std::fs::create_dir_all(dir.join(&rel).parent().unwrap()).unwrap();
        std::fs::copy(&abs, dir.join(&rel)).unwrap();
        let r = action("root", true, &json!({"t": "send", "paths": [rel], "kind": "voice"})).unwrap();
        let mid = r["id"].as_str().unwrap().to_string();
        let ib = loop {
            let ib = con
                .next_held(Duration::from_secs(60))
                .expect("delivered to the model");
            if ib.identity == "root" && is_web(&ib.channel) {
                break ib;
            }
        };
        eprintln!("model sees on {}:\n{}", ib.channel, ib.text);
        assert!(
            ib.text.starts_with("[voice message, ") && ib.text.contains("] [speech en]"),
            "{}",
            ib.text
        );
        assert!(
            ib.text.to_lowercase().contains("call you back"),
            "{}",
            ib.text
        );
        assert!(ib.text.contains("(voice message)"));
        let stored = h
            .with_user("root", |u| {
                u.find(&mid).map(|m| m.files[0].transcript.clone())
            })
            .unwrap();
        assert!(
            stored.to_lowercase().contains("call you back") && !stored.contains("[speech"),
            "transcript under the bubble: {stored}"
        );
    }
}
