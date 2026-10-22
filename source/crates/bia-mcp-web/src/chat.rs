//! Web chats: Telegram-style message store behind the HTTPS console.
//! Each client-cert identity owns its chats (`main` ↔ channel `web`, others ↔ `web:<chat>`).
//! Messages carry replies, forwards, attachments, reactions, edits, pins, read marks,
//! inline buttons and polls. Browsers get JSON events through `Bus::web_event`;
//! user actions reach the model as normal inbound envelopes.
use crate::bus::{mint_msg_id, Bus, Inbound};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

pub const MAIN: &str = "main";
pub const BIA: &str = "bia";
const MAX_MSGS: usize = 5000;
const MAX_TEXT: usize = 64 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Att {
    /// Relative to the files dir, or absolute (agent/root attachments outside it).
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
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Btn {
    pub text: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub data: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct PollOpt {
    pub text: String,
    #[serde(default)]
    pub voters: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Poll {
    pub question: String,
    pub options: Vec<PollOpt>,
    #[serde(default)]
    pub multi: bool,
    #[serde(default)]
    pub closed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Fwd {
    pub from: String,
    pub ts: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
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
    /// centered service line (chat created, renamed, …)
    #[serde(default, skip_serializing_if = "is_false")]
    pub service: bool,
}

fn is_zero(v: &u64) -> bool {
    *v == 0
}
fn is_false(v: &bool) -> bool {
    !*v
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatMeta {
    pub id: String,
    pub title: String,
    pub created: u64,
    #[serde(default)]
    pub muted: bool,
}

struct User {
    dir: PathBuf,
    chats: Vec<ChatMeta>,
    msgs: HashMap<String, Vec<Msg>>,
}

struct Hub {
    files: PathBuf,
    con: Arc<Bus>,
    users: Mutex<HashMap<String, User>>,
    typing: Mutex<HashSet<(String, String)>>,
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

pub fn is_web(channel: &str) -> bool {
    channel == crate::bus::CH_WEB || channel.starts_with("web:")
}

/// `web` → `main`, `web:<chat>` → `<chat>`.
pub fn chat_of(channel: &str) -> Option<&str> {
    if channel == crate::bus::CH_WEB {
        Some(MAIN)
    } else {
        channel.strip_prefix("web:").filter(|c| !c.is_empty())
    }
}

pub fn channel_of(chat: &str) -> String {
    if chat == MAIN {
        crate::bus::CH_WEB.into()
    } else {
        format!("web:{chat}")
    }
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

fn valid_chat_id(c: &str) -> bool {
    !c.is_empty() && c.len() <= 32 && c.chars().all(|c| c.is_ascii_alphanumeric())
}

fn new_chat_id() -> String {
    static SEQ: AtomicU64 = AtomicU64::new(1);
    format!(
        "c{:08x}{:02x}",
        now_ms() as u32,
        SEQ.fetch_add(1, Ordering::Relaxed) as u8
    )
}

/// Inbox for one identity's uploads, relative to the files dir.
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

/// Build an attachment record. `rel_or_abs` may be `/home/bia/files/x` (stored relative).
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
        transcript: String::new(),
    })
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

impl User {
    fn load(dir: PathBuf) -> User {
        let mut chats: Vec<ChatMeta> = std::fs::read(dir.join("chats.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        if !chats.iter().any(|c| c.id == MAIN) {
            chats.insert(
                0,
                ChatMeta {
                    id: MAIN.into(),
                    title: BIA.into(),
                    created: now_ms(),
                    muted: false,
                },
            );
        }
        let mut msgs = HashMap::new();
        for c in &chats {
            let v: Vec<Msg> = std::fs::read(dir.join(format!("{}.json", c.id)))
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default();
            msgs.insert(c.id.clone(), v);
        }
        User { dir, chats, msgs }
    }

    fn save_meta(&self) {
        let _ = std::fs::create_dir_all(&self.dir);
        if let Ok(b) = serde_json::to_vec_pretty(&self.chats) {
            atomic_write(&self.dir.join("chats.json"), &b);
        }
    }

    fn save_chat(&mut self, chat: &str) {
        let _ = std::fs::create_dir_all(&self.dir);
        if let Some(v) = self.msgs.get_mut(chat) {
            if v.len() > MAX_MSGS {
                let n = v.len() - MAX_MSGS;
                v.drain(0..n);
            }
            if let Ok(b) = serde_json::to_vec(v) {
                atomic_write(&self.dir.join(format!("{chat}.json")), &b);
            }
        }
    }

    fn chat(&self, chat: &str) -> Result<&Vec<Msg>, String> {
        self.msgs
            .get(chat)
            .ok_or_else(|| format!("no such chat {chat}"))
    }

    fn chat_mut(&mut self, chat: &str) -> Result<&mut Vec<Msg>, String> {
        self.msgs
            .get_mut(chat)
            .ok_or_else(|| format!("no such chat {chat}"))
    }

    fn find(&self, chat: &str, id: &str) -> Option<&Msg> {
        self.msgs.get(chat)?.iter().find(|m| m.id == id)
    }

    fn summary(&self, c: &ChatMeta) -> Value {
        let v = self.msgs.get(&c.id);
        let last = v.and_then(|v| v.last());
        let unread = v
            .map(|v| {
                v.iter()
                    .filter(|m| m.from == BIA && !m.read && !m.service)
                    .count()
            })
            .unwrap_or(0);
        let pinned: Vec<&str> = v
            .map(|v| {
                v.iter()
                    .filter(|m| m.pinned)
                    .map(|m| m.id.as_str())
                    .collect()
            })
            .unwrap_or_default();
        json!({
            "id": c.id,
            "title": c.title,
            "created": c.created,
            "muted": c.muted,
            "channel": channel_of(&c.id),
            "last": last,
            "unread": unread,
            "pinned": pinned,
        })
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
        let u = users.entry(identity.to_string()).or_insert_with(|| {
            User::load(
                self.files
                    .join("state")
                    .join("web")
                    .join(safe_seg(identity)),
            )
        });
        f(u)
    }

    fn emit(&self, identity: &str, v: Value) {
        self.con.web_event(identity, &v.to_string());
    }

    fn emit_chat(&self, identity: &str, chat: &str) {
        let s = self.with_user(identity, |u| {
            u.chats.iter().find(|c| c.id == chat).map(|c| u.summary(c))
        });
        if let Some(s) = s {
            self.emit(identity, json!({"t": "chat", "c": s}));
        }
    }

    /// Show a voice message's transcript under it (stored and pushed to browsers).
    fn set_transcript(&self, identity: &str, chat: &str, id: &str, path: &str, text: &str) {
        let m = self.with_user(identity, |u| -> Option<Msg> {
            let m = u.chat_mut(chat).ok()?.iter_mut().find(|m| m.id == id)?;
            let a = m.files.iter_mut().find(|a| a.path == path)?;
            a.transcript = text.to_string();
            let m = m.clone();
            u.save_chat(chat);
            Some(m)
        });
        if let Some(m) = m {
            self.emit(identity, json!({"t": "upd", "chat": chat, "m": m}));
        }
    }

    /// Hand a user action to the model as an inbound envelope on the chat's channel.
    fn to_agent(&self, identity: &str, chat: &str, id: &str, text: String) {
        self.con.push_inbound(Inbound {
            identity: identity.to_string(),
            channel: channel_of(chat),
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

/// A new message from bia: wake the person's browsers that are not in front (not for muted chats).
fn push_msg(h: &Hub, identity: &str, chat: &str, m: &Msg) {
    let Some(c) = h.with_user(identity, |u| u.chats.iter().find(|c| c.id == chat).cloned()) else {
        return;
    };
    if c.muted {
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
            title: c.title,
            body,
            tag: format!("bia-{chat}"),
            url: format!("chat/{chat}"),
            ttl: 24 * 3600,
            urgent: false,
        },
    );
}

fn files_block(files: &[Att]) -> String {
    let mut s = String::from("files:\n");
    for a in files {
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

/// What the model sees for a user message (Telegram-like markers + the text + files).
fn agent_text(u: &User, chat: &str, m: &Msg) -> String {
    let mut parts: Vec<String> = vec![];
    if let Some(r) = &m.reply_to {
        match u.find(chat, r) {
            Some(o) => parts.push(format!(
                "[reply to {r} from {}: \"{}\"]",
                o.from,
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
    if !m.files.is_empty() {
        parts.push(files_block(&m.files).trim_end().to_string());
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

// ---- browser API ----

/// First event on a chat WebSocket: identity, chats and typing.
pub fn hello(identity: &str, root: bool) -> Value {
    let Ok(h) = hub() else {
        return json!({"t": "hello"});
    };
    let chats = h.with_user(identity, |u| {
        u.chats.iter().map(|c| u.summary(c)).collect::<Vec<_>>()
    });
    let typing: Vec<String> = h
        .typing
        .lock()
        .unwrap()
        .iter()
        .filter(|(i, _)| i == identity)
        .map(|(_, c)| c.clone())
        .collect();
    json!({
        "t": "hello",
        "me": {"identity": identity, "root": root},
        "chats": chats,
        "typing": typing,
    })
}

/// Page of messages: newest `limit` before `before`, after `after`, or centered on `around`.
pub fn messages(
    identity: &str,
    chat: &str,
    before: Option<&str>,
    after: Option<&str>,
    around: Option<&str>,
    limit: usize,
) -> Result<Value, String> {
    let h = hub()?;
    let limit = limit.clamp(1, 200);
    h.with_user(identity, |u| {
        let v = u.chat(chat)?;
        let pos = |id: &str| v.iter().position(|m| m.id == id);
        let (start, end) = if let Some(a) = around.and_then(pos) {
            let s = a.saturating_sub(limit / 2);
            (s, (s + limit).min(v.len()))
        } else if let Some(b) = before {
            let e = pos(b).unwrap_or(0);
            (e.saturating_sub(limit), e)
        } else if let Some(a) = after {
            let s = pos(a).map(|i| i + 1).unwrap_or(v.len());
            (s, (s + limit).min(v.len()))
        } else {
            (v.len().saturating_sub(limit), v.len())
        };
        Ok(json!({
            "chat": chat,
            "messages": &v[start..end],
            "more_before": start > 0,
            "more_after": end < v.len(),
        }))
    })
}

pub fn search(identity: &str, chat: Option<&str>, q: &str) -> Result<Value, String> {
    let h = hub()?;
    let q = q.trim().to_lowercase();
    if q.is_empty() {
        return Ok(json!({"results": []}));
    }
    Ok(h.with_user(identity, |u| {
        let mut out = vec![];
        for c in u.chats.iter().rev() {
            if chat.is_some_and(|x| x != c.id) {
                continue;
            }
            let Some(v) = u.msgs.get(&c.id) else { continue };
            for m in v.iter().rev() {
                if m.service {
                    continue;
                }
                let hit = m.text.to_lowercase().contains(&q)
                    || m.files.iter().any(|a| a.name.to_lowercase().contains(&q));
                if hit {
                    out.push(json!({"chat": c.id, "title": c.title, "m": m}));
                    if out.len() >= 200 {
                        break;
                    }
                }
            }
        }
        json!({"results": out})
    }))
}

/// Resolve an attachment the identity may download: `(abs path, display name, mime)`.
pub fn attachment(
    identity: &str,
    chat: &str,
    id: &str,
    i: usize,
) -> Option<(PathBuf, String, String)> {
    let h = hub().ok()?;
    let a = h.with_user(identity, |u| {
        u.find(chat, id).and_then(|m| m.files.get(i)).cloned()
    })?;
    let abs = att_abs(&h.files, &a.path)?;
    Some((abs, a.name, a.mime))
}

/// One action from a browser (`t` names it). Returns the reply payload for the caller.
pub fn action(identity: &str, root: bool, v: &Value) -> Result<Value, String> {
    let h = hub()?;
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let chat = {
        let c = s("chat");
        if c.is_empty() {
            MAIN.to_string()
        } else {
            c
        }
    };
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
    match s("t").as_str() {
        // legacy `{t:"line", v, files}` from older pages and peer agents
        "call_decline" => {
            crate::call::decline(identity, &chat);
            Ok(json!({}))
        }
        "line" | "send" => {
            let text = if s("t") == "line" { s("v") } else { s("text") };
            let reply = Some(s("reply_to")).filter(|x| !x.is_empty());
            let kind = Some(s("kind")).filter(|x| !x.is_empty());
            let files = ids("files");
            let m = user_send(
                h,
                identity,
                root,
                &chat,
                &text,
                reply,
                &files,
                kind.as_deref(),
            )?;
            Ok(json!({"id": m.id, "m": m}))
        }
        "edit" => {
            let id = s("id");
            let text = clip_text(&s("text"));
            let (m, note) = h.with_user(identity, |u| -> Result<(Msg, String), String> {
                let m = u
                    .chat_mut(&chat)?
                    .iter_mut()
                    .find(|m| m.id == id)
                    .ok_or("no such message")?;
                if m.from != identity {
                    return Err("you can only edit your own messages".into());
                }
                if text.trim().is_empty() && m.files.is_empty() {
                    return Err("message would be empty".into());
                }
                m.text = text.clone();
                m.edited = now_ms();
                let m = m.clone();
                u.save_chat(&chat);
                Ok((m, format!("[edited {id}]\n{text}")))
            })?;
            h.emit(identity, json!({"t": "upd", "chat": chat, "m": m}));
            h.to_agent(identity, &chat, &mint_msg_id(), note);
            Ok(json!({"ok": true}))
        }
        "delete" => {
            let list = ids("ids");
            let gone = h.with_user(identity, |u| -> Result<Vec<String>, String> {
                let v = u.chat_mut(&chat)?;
                let before: Vec<String> = v.iter().map(|m| m.id.clone()).collect();
                v.retain(|m| !list.contains(&m.id));
                let gone: Vec<String> = before.into_iter().filter(|id| list.contains(id)).collect();
                u.save_chat(&chat);
                Ok(gone)
            })?;
            if !gone.is_empty() {
                h.emit(identity, json!({"t": "del", "chat": chat, "ids": gone}));
                h.emit_chat(identity, &chat);
                h.to_agent(
                    identity,
                    &chat,
                    &mint_msg_id(),
                    format!("[deleted] {}", gone.join(", ")),
                );
            }
            Ok(json!({"deleted": gone}))
        }
        "react" => {
            let id = s("id");
            let emoji = s("emoji");
            if emoji.is_empty() || emoji.chars().count() > 8 {
                return Err("bad emoji".into());
            }
            let (m, added) = h.with_user(identity, |u| -> Result<(Msg, bool), String> {
                let m = u
                    .chat_mut(&chat)?
                    .iter_mut()
                    .find(|m| m.id == id)
                    .ok_or("no such message")?;
                let who = m.reactions.entry(emoji.clone()).or_default();
                let added = if let Some(i) = who.iter().position(|w| w == identity) {
                    who.remove(i);
                    false
                } else {
                    who.push(identity.to_string());
                    true
                };
                m.reactions.retain(|_, w| !w.is_empty());
                let m = m.clone();
                u.save_chat(&chat);
                Ok((m, added))
            })?;
            h.emit(identity, json!({"t": "upd", "chat": chat, "m": m}));
            let note = if added {
                format!("[reaction] {emoji} on {id} (\"{}\")", snippet(&m))
            } else {
                format!("[reaction] removed {emoji} on {id}")
            };
            h.to_agent(identity, &chat, &mint_msg_id(), note);
            Ok(json!({"ok": true}))
        }
        "pin" => {
            let id = s("id");
            let on = v.get("on").and_then(|x| x.as_bool()).unwrap_or(true);
            let m = set_pin(h, identity, &chat, &id, on)?;
            h.emit(identity, json!({"t": "upd", "chat": chat, "m": m}));
            h.emit_chat(identity, &chat);
            Ok(json!({"ok": true}))
        }
        "read" => {
            let upto = s("upto");
            let changed = h.with_user(identity, |u| -> Result<Vec<Msg>, String> {
                let v = u.chat_mut(&chat)?;
                let end = v
                    .iter()
                    .position(|m| m.id == upto)
                    .map(|i| i + 1)
                    .unwrap_or(v.len());
                let mut changed = vec![];
                for m in v[..end].iter_mut() {
                    if m.from == BIA && !m.read {
                        m.read = true;
                        changed.push(m.clone());
                    }
                }
                if !changed.is_empty() {
                    u.save_chat(&chat);
                }
                Ok(changed)
            })?;
            if !changed.is_empty() {
                h.emit_chat(identity, &chat);
            }
            Ok(json!({"ok": true}))
        }
        "button" => {
            let id = s("id");
            let row = v.get("row").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
            let col = v.get("col").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
            let (b, snip) = h.with_user(identity, |u| -> Result<(Btn, String), String> {
                let m = u.find(&chat, &id).ok_or("no such message")?;
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
                &chat,
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
                let m = u
                    .chat_mut(&chat)?
                    .iter_mut()
                    .find(|m| m.id == id)
                    .ok_or("no such message")?;
                let p = m.poll.as_mut().ok_or("not a poll")?;
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
                for &i in &picks {
                    if let Some(o) = p.options.get_mut(i) {
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
                let m = m.clone();
                u.save_chat(&chat);
                Ok((m, note))
            })?;
            h.emit(identity, json!({"t": "upd", "chat": chat, "m": m}));
            h.to_agent(identity, &chat, &mint_msg_id(), note);
            Ok(json!({"ok": true}))
        }
        "forward" => {
            let to = s("to");
            let list = ids("ids");
            let to = if to.is_empty() { chat.clone() } else { to };
            let src: Vec<Msg> = h.with_user(identity, |u| -> Result<Vec<Msg>, String> {
                u.chat(&to)?;
                let v = u.chat(&chat)?;
                Ok(v.iter()
                    .filter(|m| list.contains(&m.id) && !m.service)
                    .cloned()
                    .collect())
            })?;
            let mut out = vec![];
            for o in src {
                let m = Msg {
                    id: mint_msg_id(),
                    from: identity.to_string(),
                    ts: now_ms(),
                    text: o.text.clone(),
                    fwd: Some(o.fwd.clone().unwrap_or(Fwd {
                        from: o.from.clone(),
                        ts: o.ts,
                    })),
                    files: o.files.clone(),
                    ..Default::default()
                };
                out.push(store_user_msg(h, identity, &to, m)?.id);
            }
            Ok(json!({"ids": out}))
        }
        "chat_new" => {
            let title = s("title");
            let title = if title.trim().is_empty() {
                "New chat".to_string()
            } else {
                title.trim().chars().take(64).collect()
            };
            let id = new_chat_id();
            h.with_user(identity, |u| {
                u.chats.push(ChatMeta {
                    id: id.clone(),
                    title: title.clone(),
                    created: now_ms(),
                    muted: false,
                });
                u.msgs.insert(id.clone(), vec![]);
                u.save_meta();
                u.save_chat(&id);
            });
            h.emit_chat(identity, &id);
            Ok(json!({"id": id, "channel": channel_of(&id)}))
        }
        "chat_rename" => {
            let title: String = s("title").trim().chars().take(64).collect();
            if title.is_empty() {
                return Err("title required".into());
            }
            h.with_user(identity, |u| -> Result<(), String> {
                let c = u
                    .chats
                    .iter_mut()
                    .find(|c| c.id == chat)
                    .ok_or("no such chat")?;
                c.title = title.clone();
                u.save_meta();
                Ok(())
            })?;
            h.emit_chat(identity, &chat);
            Ok(json!({"ok": true}))
        }
        "chat_mute" => {
            let on = v.get("on").and_then(|x| x.as_bool()).unwrap_or(true);
            h.with_user(identity, |u| -> Result<(), String> {
                let c = u
                    .chats
                    .iter_mut()
                    .find(|c| c.id == chat)
                    .ok_or("no such chat")?;
                c.muted = on;
                u.save_meta();
                Ok(())
            })?;
            h.emit_chat(identity, &chat);
            Ok(json!({"ok": true}))
        }
        "chat_clear" => {
            h.with_user(identity, |u| -> Result<(), String> {
                u.chat_mut(&chat)?.clear();
                u.save_chat(&chat);
                Ok(())
            })?;
            h.emit(identity, json!({"t": "clear", "chat": chat}));
            h.emit_chat(identity, &chat);
            Ok(json!({"ok": true}))
        }
        "chat_delete" => {
            if chat == MAIN {
                return Err("the main chat cannot be deleted (clear it instead)".into());
            }
            h.with_user(identity, |u| -> Result<(), String> {
                let n = u.chats.len();
                u.chats.retain(|c| c.id != chat);
                if u.chats.len() == n {
                    return Err("no such chat".into());
                }
                u.msgs.remove(&chat);
                let _ = std::fs::remove_file(u.dir.join(format!("{chat}.json")));
                u.save_meta();
                Ok(())
            })?;
            h.emit(identity, json!({"t": "chat_del", "id": chat}));
            Ok(json!({"ok": true}))
        }
        other => Err(format!("unknown action {other}")),
    }
}

fn set_pin(h: &Hub, identity: &str, chat: &str, id: &str, on: bool) -> Result<Msg, String> {
    h.with_user(identity, |u| -> Result<Msg, String> {
        let m = u
            .chat_mut(chat)?
            .iter_mut()
            .find(|m| m.id == id)
            .ok_or("no such message")?;
        m.pinned = on;
        let m = m.clone();
        u.save_chat(chat);
        Ok(m)
    })
}

#[allow(clippy::too_many_arguments)]
fn user_send(
    h: &Hub,
    identity: &str,
    root: bool,
    chat: &str,
    text: &str,
    reply_to: Option<String>,
    files: &[String],
    kind: Option<&str>,
) -> Result<Msg, String> {
    let text = clip_text(text);
    let inbox = format!("{}/", inbox_rel(identity));
    let mut atts = vec![];
    for p in files {
        let rel = p.trim();
        if !root && (rel.starts_with('/') || !rel.starts_with(&inbox) || rel.contains("..")) {
            return Err(format!("{rel}: attachments must be your own uploads"));
        }
        atts.push(make_att(&h.files, rel, kind)?);
    }
    if text.trim().is_empty() && atts.is_empty() {
        return Err("empty message".into());
    }
    let m = Msg {
        id: mint_msg_id(),
        from: identity.to_string(),
        ts: now_ms(),
        text,
        reply_to,
        files: atts,
        ..Default::default()
    };
    store_user_msg(h, identity, chat, m)
}

/// Store, broadcast and hand to the model.
fn store_user_msg(h: &Hub, identity: &str, chat: &str, m: Msg) -> Result<Msg, String> {
    let note = h.with_user(identity, |u| -> Result<String, String> {
        u.chat(chat)?;
        let note = agent_text(u, chat, &m);
        u.chat_mut(chat)?.push(m.clone());
        u.save_chat(chat);
        Ok(note)
    })?;
    h.emit(identity, json!({"t": "msg", "chat": chat, "m": m}));
    h.emit_chat(identity, chat);
    let voice = m
        .files
        .iter()
        .filter(|a| a.kind == "voice" || a.kind == "audio")
        .filter_map(|a| att_abs(&h.files, &a.path).map(|abs| (a.path.clone(), abs)))
        .collect();
    deliver_later(Delivery {
        turn: lanes().reserve(&format!("{identity}/{chat}")),
        identity: identity.to_string(),
        chat: chat.to_string(),
        id: m.id.clone(),
        note,
        voice,
    });
    Ok(m)
}

/// A user message on its way to the model. Voice and audio files are transcribed
/// first, on a thread of the message's own; `turn` keeps a chat's messages in order
/// without other chats waiting for them.
struct Delivery {
    turn: bia_mcp::lanes::Turn,
    identity: String,
    chat: String,
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

/// Web chats' messages to the model, in order per chat (see [`bia_mcp::lanes`]).
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
                    let t = heard.text();
                    if t.is_empty() {
                        heads.push("[voice message, no words found]".to_string());
                    } else {
                        // under the bubble: just the words, not the model's language tags
                        let shown: Vec<&str> = heard.runs.iter().map(|r| r.text.as_str()).collect();
                        h.set_transcript(&d.identity, &d.chat, &d.id, rel, &shown.join(" "));
                        heads.push(format!("[voice message] {t}"));
                    }
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
    h.to_agent(&d.identity, &d.chat, &d.id, note);
}

// ---- model side ----

/// A person who can have web chats: root, an identity with an active client certificate, or
/// one that already has chats. Never bia itself — bia is the other side of every web chat.
pub fn known_identity(identity: &str) -> bool {
    let id = identity.trim();
    if id.is_empty() || id == BIA {
        return false;
    }
    if id == crate::bus::ID_ROOT {
        return true;
    }
    let Ok(h) = hub() else { return false };
    if h.users.lock().map(|u| u.contains_key(id)).unwrap_or(false)
        || h.files
            .join("state")
            .join("web")
            .join(safe_seg(id))
            .is_dir()
    {
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
            if e.path().is_dir() {
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

/// One line per chat of `identity`: `channel  title  (n unread)`.
/// Whether `identity` already has the chat on `channel` (their main chat always counts).
pub fn has_chat(identity: &str, channel: &str) -> bool {
    let Some(chat) = chat_of(channel) else {
        return false;
    };
    if chat == MAIN {
        return true;
    }
    let Ok(h) = hub() else { return false };
    h.with_user(identity, |u| {
        u.msgs.contains_key(chat) || u.chats.iter().any(|c| c.id == chat)
    })
}

pub fn chat_lines(identity: &str) -> Vec<String> {
    let Ok(h) = hub() else { return vec![] };
    h.with_user(identity, |u| {
        u.chats
            .iter()
            .map(|c| {
                let unread = u
                    .msgs
                    .get(&c.id)
                    .map(|v| {
                        v.iter()
                            .filter(|m| m.from != BIA && !m.read && !m.service)
                            .count()
                    })
                    .unwrap_or(0);
                let unread = if unread > 0 {
                    format!("  ({unread} unread)")
                } else {
                    String::new()
                };
                format!("{}  \"{}\"{unread}", channel_of(&c.id), c.title)
            })
            .collect()
    })
}

fn chat_for(h: &Hub, identity: &str, channel: &str) -> Result<String, String> {
    let chat = chat_of(channel)
        .ok_or_else(|| format!("{channel} is not a web channel"))?
        .to_string();
    if chat != MAIN && !valid_chat_id(&chat) {
        return Err(format!("bad web chat {chat}"));
    }
    h.with_user(identity, |u| {
        if !u.msgs.contains_key(&chat) {
            // the model may open a new chat by naming it
            u.chats.push(ChatMeta {
                id: chat.clone(),
                title: chat.clone(),
                created: now_ms(),
                muted: false,
            });
            u.msgs.insert(chat.clone(), vec![]);
            u.save_meta();
        }
    });
    Ok(chat)
}

/// The model reads a user message: mark it (and older ones) read (✓✓ in the browser).
pub fn agent_read(identity: &str, channel: &str, id: &str) {
    let Ok(h) = hub() else { return };
    let Some(chat) = chat_of(channel).map(|c| c.to_string()) else {
        return;
    };
    let changed = h.with_user(identity, |u| {
        let Some(v) = u.msgs.get_mut(&chat) else {
            return vec![];
        };
        let Some(end) = v.iter().position(|m| m.id == id) else {
            return vec![];
        };
        let mut changed = vec![];
        for m in v[..=end].iter_mut() {
            if m.from != BIA && !m.read {
                m.read = true;
                changed.push(m.id.clone());
            }
        }
        if !changed.is_empty() {
            u.save_chat(&chat);
        }
        changed
    });
    if !changed.is_empty() {
        h.emit(identity, json!({"t": "read", "chat": chat, "ids": changed}));
    }
}

/// Bia typing/thinking indicator for a web chat.
pub fn typing(identity: &str, channel: &str, on: bool) {
    let Ok(h) = hub() else { return };
    let Some(chat) = chat_of(channel).map(|c| c.to_string()) else {
        return;
    };
    {
        let mut t = h.typing.lock().unwrap();
        let key = (identity.to_string(), chat.clone());
        if on {
            t.insert(key);
        } else {
            t.remove(&key);
        }
    }
    h.emit(identity, json!({"t": "typing", "chat": chat, "on": on}));
}

/// `say` on a web channel. Returns the new message id.
#[allow(clippy::too_many_arguments)]
pub fn say(
    identity: &str,
    channel: &str,
    text: &str,
    reply_to: Option<&str>,
    files: &[String],
    buttons: Option<&Value>,
    poll: Option<&Value>,
) -> Result<String, String> {
    let h = hub()?;
    let chat = chat_for(h, identity, channel)?;
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
        let exists = h.with_user(identity, |u| u.find(&chat, r).is_some());
        if !exists {
            return Err(format!("reply_to {r}: no such message in {channel}"));
        }
    }
    let m = Msg {
        id: mint_msg_id(),
        from: BIA.into(),
        ts: now_ms(),
        text,
        reply_to,
        files: atts,
        buttons: parse_buttons(buttons),
        poll,
        ..Default::default()
    };
    h.with_user(identity, |u| -> Result<(), String> {
        u.chat_mut(&chat)?.push(m.clone());
        u.save_chat(&chat);
        Ok(())
    })?;
    h.emit(identity, json!({"t": "msg", "chat": chat, "m": m}));
    h.emit_chat(identity, &chat);
    push_msg(h, identity, &chat, &m);
    Ok(m.id)
}

// ---- for web calls (webcall.rs) ----

pub fn emit_event(identity: &str, v: Value) {
    if let Ok(h) = hub() {
        h.emit(identity, v);
    }
}

pub fn has_client(identity: &str) -> bool {
    hub().map(|h| h.con.has_web(identity)).unwrap_or(false)
}

/// The chat behind a web channel (created when the model names a new one).
pub fn ensure_chat(identity: &str, channel: &str) -> Result<String, String> {
    chat_for(hub()?, identity, channel)
}

/// A call event for the model (`system` identity on the chat's channel).
pub fn agent_notice(identity: &str, chat: &str, text: &str) {
    let Ok(h) = hub() else { return };
    let channel = channel_of(chat);
    h.con.push_inbound(Inbound {
        identity: crate::bus::ID_SYSTEM.into(),
        channel,
        id: mint_msg_id(),
        text: format!("{text} (web call with {identity})"),
    });
}

/// What the person said on a web call, as their message.
pub fn agent_speech(identity: &str, chat: &str, text: &str) {
    if let Ok(h) = hub() {
        h.to_agent(identity, chat, &mint_msg_id(), text.to_string());
    }
}

/// All message texts of a chat (tests).
#[cfg(test)]
pub fn chat_text(identity: &str, chat: &str) -> String {
    hub()
        .map(|h| {
            h.with_user(identity, |u| {
                u.chat(chat)
                    .map(|c| {
                        c.iter()
                            .map(|m| m.text.clone())
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_default()
            })
        })
        .unwrap_or_default()
}

/// Post a text message from bia (the call summary).
pub fn post_bia(identity: &str, chat: &str, text: &str) -> Result<String, String> {
    say(identity, &channel_of(chat), text, None, &[], None, None)
}

/// Speak `text` (in `lang`) and post it as a voice message from bia.
pub fn say_voice(
    identity: &str,
    channel: &str,
    text: &str,
    lang: &str,
    reply_to: Option<&str>,
) -> Result<String, String> {
    let h = hub()?;
    let chat = chat_for(h, identity, channel)?;
    let text = text.trim();
    if text.is_empty() {
        return Err("message is the text to speak".into());
    }
    let reply_to = reply_to
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    if let Some(r) = &reply_to {
        if !h.with_user(identity, |u| u.find(&chat, r).is_some()) {
            return Err(format!("reply_to {r}: no such message in {channel}"));
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
    let m = Msg {
        id: mint_msg_id(),
        from: BIA.into(),
        ts: now_ms(),
        reply_to,
        files: vec![att],
        ..Default::default()
    };
    h.with_user(identity, |u| -> Result<(), String> {
        u.chat_mut(&chat)?.push(m.clone());
        u.save_chat(&chat);
        Ok(())
    })?;
    h.emit(identity, json!({"t": "msg", "chat": chat, "m": m}));
    h.emit_chat(identity, &chat);
    push_msg(h, identity, &chat, &m);
    Ok(m.id)
}

pub fn agent_edit(identity: &str, channel: &str, id: &str, text: &str) -> Result<(), String> {
    let h = hub()?;
    let chat = chat_for(h, identity, channel)?;
    let text = clip_text(text);
    let m = h.with_user(identity, |u| -> Result<Msg, String> {
        let m = u
            .chat_mut(&chat)?
            .iter_mut()
            .find(|m| m.id == id)
            .ok_or_else(|| format!("no message {id} in {channel}"))?;
        if m.from != BIA {
            return Err("you can only edit your own messages".into());
        }
        m.text = text;
        m.edited = now_ms();
        let m = m.clone();
        u.save_chat(&chat);
        Ok(m)
    })?;
    h.emit(identity, json!({"t": "upd", "chat": chat, "m": m}));
    h.emit_chat(identity, &chat);
    Ok(())
}

pub fn agent_delete(identity: &str, channel: &str, ids: &[String]) -> Result<usize, String> {
    let h = hub()?;
    let chat = chat_for(h, identity, channel)?;
    let gone = h.with_user(identity, |u| -> Result<Vec<String>, String> {
        let v = u.chat_mut(&chat)?;
        let gone: Vec<String> = v
            .iter()
            .filter(|m| ids.contains(&m.id))
            .map(|m| m.id.clone())
            .collect();
        v.retain(|m| !ids.contains(&m.id));
        u.save_chat(&chat);
        Ok(gone)
    })?;
    if !gone.is_empty() {
        h.emit(identity, json!({"t": "del", "chat": chat, "ids": gone}));
        h.emit_chat(identity, &chat);
    }
    Ok(gone.len())
}

/// Set bia's reaction on a message (one at a time, like a regular account). Empty removes it.
pub fn agent_react(identity: &str, channel: &str, id: &str, emoji: &str) -> Result<(), String> {
    let h = hub()?;
    let chat = chat_for(h, identity, channel)?;
    let emoji = emoji.trim().to_string();
    let m = h.with_user(identity, |u| -> Result<Msg, String> {
        let m = u
            .chat_mut(&chat)?
            .iter_mut()
            .find(|m| m.id == id)
            .ok_or_else(|| format!("no message {id} in {channel}"))?;
        for w in m.reactions.values_mut() {
            w.retain(|x| x != BIA);
        }
        if !emoji.is_empty() {
            m.reactions.entry(emoji).or_default().push(BIA.into());
        }
        m.reactions.retain(|_, w| !w.is_empty());
        let m = m.clone();
        u.save_chat(&chat);
        Ok(m)
    })?;
    h.emit(identity, json!({"t": "upd", "chat": chat, "m": m}));
    Ok(())
}

pub fn agent_pin(identity: &str, channel: &str, id: &str, on: bool) -> Result<(), String> {
    let h = hub()?;
    let chat = chat_for(h, identity, channel)?;
    let m = set_pin(h, identity, &chat, id, on)?;
    h.emit(identity, json!({"t": "upd", "chat": chat, "m": m}));
    h.emit_chat(identity, &chat);
    Ok(())
}

/// Close a poll the model sent (no more votes); returns the final tally.
pub fn agent_close_poll(identity: &str, channel: &str, id: &str) -> Result<String, String> {
    let h = hub()?;
    let chat = chat_for(h, identity, channel)?;
    let m = h.with_user(identity, |u| -> Result<Msg, String> {
        let m = u
            .chat_mut(&chat)?
            .iter_mut()
            .find(|m| m.id == id)
            .ok_or_else(|| format!("no message {id} in {channel}"))?;
        m.poll.as_mut().ok_or("not a poll")?.closed = true;
        let m = m.clone();
        u.save_chat(&chat);
        Ok(m)
    })?;
    h.emit(identity, json!({"t": "upd", "chat": chat, "m": m}));
    let p = m.poll.unwrap_or_default();
    Ok(p.options
        .iter()
        .map(|o| format!("{}: {}", o.text, o.voters.len()))
        .collect::<Vec<_>>()
        .join(", "))
}

/// Recent messages of a web chat for the model (`web_history`).
pub fn agent_history(identity: &str, channel: &str, limit: usize) -> Result<String, String> {
    let h = hub()?;
    let chat = chat_for(h, identity, channel)?;
    h.with_user(identity, |u| {
        let v = u.chat(&chat)?;
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
                    extra.push(format!("{} {}", a.kind, a.path));
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
            Ok(format!("({channel} is empty)"))
        } else {
            Ok(lines.join("\n"))
        }
    })
}

/// Legacy `/v1/history`: the main chat in the old `{role, who, content}` shape.
pub fn legacy_history(identity: &str) -> Value {
    let Ok(h) = hub() else {
        return json!({"messages": []});
    };
    let msgs = h.with_user(identity, |u| {
        let v = u.msgs.get(MAIN).cloned().unwrap_or_default();
        let start = v.len().saturating_sub(80);
        v[start..]
            .iter()
            .filter(|m| !m.service)
            .map(|m| {
                let mut content = m.text.clone();
                if !m.files.is_empty() {
                    content.push_str(&format!("\n{}", files_block(&m.files)));
                }
                if m.from == BIA {
                    json!({"role": "assistant", "who": format!("bia → {identity}@web"), "content": content, "id": m.id})
                } else {
                    json!({"role": "user", "who": format!("{identity}@web"), "content": content, "id": m.id})
                }
            })
            .collect::<Vec<_>>()
    });
    json!({"messages": msgs})
}

#[cfg(test)]
mod voice_tests {
    use super::*;
    use std::time::Duration;

    /// Real models: BIA_SPEECH_DIR=<bundled speech tree> cargo test -p bia-mcp-web -- --ignored voice
    #[test]
    #[ignore = "needs the speech models (BIA_SPEECH_DIR)"]
    fn voice_both_ways() {
        let dir = std::env::temp_dir().join(format!("bia-webvoice-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("BIA_SPEECH_STORE", dir.join("speech"));
        let con = Arc::new(Bus::new(dir.clone()));
        init(&dir, con.clone());
        let h = hub().unwrap();

        // bia answers with a voice message in a custom chat
        let said = "Good morning. I will call you back at eight tonight.";
        assert!(say_voice("root", "web:test", said, "", None)
            .unwrap_err()
            .contains("lang is required"));
        let id = say_voice("root", "web:test", said, "en", None).unwrap();
        let att = h
            .with_user("root", |u| u.find("test", &id).map(|m| m.files[0].clone()))
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
        let m = user_send(
            h,
            "root",
            true,
            "test",
            "",
            None,
            std::slice::from_ref(&rel),
            Some("voice"),
        )
        .unwrap();
        let ib = loop {
            let ib = con
                .next_held(Duration::from_secs(60))
                .expect("delivered to the model");
            if is_web(&ib.channel) {
                break ib;
            }
        };
        eprintln!("model sees on {}:\n{}", ib.channel, ib.text);
        assert_eq!(ib.channel, "web:test");
        assert!(
            ib.text.starts_with("[voice message] [speech en]"),
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
                u.find("test", &m.id).map(|m| m.files[0].transcript.clone())
            })
            .unwrap();
        assert!(
            stored.to_lowercase().contains("call you back") && !stored.contains("[speech"),
            "transcript under the bubble: {stored}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
