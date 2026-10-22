//! whatsapp-rust client: QR-only pairing, messaging, bans, local history cache,
//! inbound pump (+ call event routing into `calls`).
//!
//! chat_id conventions (channel is always `whatsapp:<chat_id>`):
//!   - person:  phone digits, e.g. `15551234567`
//!   - person without a known phone (LID only): `<digits>@lid`
//!   - group:   `<id>@g.us`

use crate::calls::Calls;
use bia_mcp::Ctx;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use whatsapp_rust::media::{self, AudioOptions, DocumentOptions, ImageOptions};
use whatsapp_rust::prelude::*;
use whatsapp_rust::wacore::download::MediaType;
use whatsapp_rust::wacore::types::events::LazyHistorySync;
use whatsapp_rust::wacore_binary::JidExt;
use whatsapp_rust::{RevokeType, UploadOptions};

/// Messages kept per chat in the local history cache.
const HIST_CAP: usize = 1000;
/// Conversations in a HistorySync are trimmed to this many newest messages.
const HISTSYNC_PER_CHAT: usize = 100;
/// Don't answer a banned chat more often than this (avoids bot ping-pong).
const BAN_REPLY_GAP: Duration = Duration::from_secs(30);

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct Cfg {
    /// Own phone number digits (set once paired).
    #[serde(default)]
    pub phone: String,
    /// True once a QR scan completed; gates auto-restore at boot.
    #[serde(default)]
    pub paired: bool,
}

#[derive(Clone)]
pub struct Target {
    pub key: String,
    pub jid: Jid,
    pub title: String,
}

struct Inner {
    client: Option<Arc<Client>>,
    ctx: Option<Ctx>,
    phase: String,
    qr: Option<String>,
    qr_seq: u64,
    banned: HashSet<String>,
    /// Bumped whenever the live bot is replaced/stopped so a dying run loop or
    /// stale event handler cannot clobber the new one.
    generation: u64,
    /// After logout / unlink, do not auto-reconnect until login/restore.
    suppress_reconnect: bool,
}

pub struct Wa {
    inner: Mutex<Inner>,
    connect_lock: tokio::sync::Mutex<()>,
    reconnecting: AtomicBool,
    book: Mutex<ChatBook>,
    hist: Mutex<HistoryStore>,
    ban_replies: Mutex<HashMap<String, Instant>>,
    rt: tokio::runtime::Runtime,
}

// ---------------------------------------------------------------------------
// Paths + small helpers
// ---------------------------------------------------------------------------

pub fn files_dir() -> PathBuf {
    PathBuf::from(std::env::var("BIA_FILES").unwrap_or_else(|_| "/home/bia/files".into()))
}

fn safe_name(key: &str) -> String {
    key.chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '_' })
        .collect()
}

fn digits_only(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_digit()).collect()
}

fn looks_numeric(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit() || "+-() .".contains(c))
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Strip `whatsapp:` / `whatsapp-` prefixes and whitespace.
pub fn normalize_target(t: &str) -> String {
    let t = t.trim();
    t.strip_prefix("whatsapp:")
        .or_else(|| t.strip_prefix("whatsapp-"))
        .unwrap_or(t)
        .trim()
        .to_string()
}

/// Chat key for a JID without any network lookups.
pub fn key_of_jid(jid: &Jid) -> String {
    let j = jid.to_non_ad();
    if j.is_pn() {
        j.user.to_string()
    } else if j.is_lid() {
        format!("{}@lid", j.user)
    } else if j.is_group() {
        format!("{}@g.us", j.user)
    } else {
        j.to_string()
    }
}

/// Chat key for a JID; LIDs are mapped to the phone number when WhatsApp told us.
pub async fn key_of_jid_async(client: &Client, jid: &Jid) -> String {
    let j = jid.to_non_ad();
    if j.is_lid() {
        if let Ok(Some(e)) = client.get_lid_pn_entry(&j).await {
            return e.phone_number.to_string();
        }
    }
    key_of_jid(&j)
}

fn jid_of_key(key: &str) -> Option<Jid> {
    let key = key.trim();
    if key.contains('@') {
        key.parse::<Jid>().ok().map(|j| j.to_non_ad())
    } else {
        let d = digits_only(key);
        if d.is_empty() {
            None
        } else {
            Some(Jid::pn(d.as_str()))
        }
    }
}

/// A received voice/audio message for the model: `[voice message] [speech ru] …`, then
/// the message's own lines (file: …), then a notice if a language was just learned.
fn with_transcript(heard: Result<bia_speech::Heard, String>, text: &str, _file: &str) -> String {
    match heard {
        Ok(h) => {
            let spoken = h.text();
            let mut out = if spoken.is_empty() {
                format!("[voice message, no words found]\n{text}")
            } else {
                format!("[voice message] {spoken}\n{text}")
            };
            if let Some(n) = h.notice {
                out.push('\n');
                out.push_str(&n);
            }
            out
        }
        Err(e) => format!("[voice message, transcription failed: {e}]\n{text}"),
    }
}

fn resolve_file_path(root: &Path, f: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(f);
    let abs = if p.is_absolute() {
        p
    } else {
        root.join(f.trim_start_matches('/'))
    };
    if !abs.exists() {
        return Err(format!("file not found: {}", abs.display()));
    }
    Ok(abs)
}

fn mime_for(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("mp4") => "video/mp4",
        Some("mov") => "video/quicktime",
        Some("webm") => "video/webm",
        Some("mp3") => "audio/mpeg",
        Some("ogg") | Some("oga") | Some("opus") => "audio/ogg",
        Some("m4a") => "audio/mp4",
        Some("wav") => "audio/wav",
        Some("pdf") => "application/pdf",
        Some("txt") | Some("log") => "text/plain",
        Some("json") => "application/json",
        Some("csv") => "text/csv",
        Some("zip") => "application/zip",
        _ => "application/octet-stream",
    }
}

fn ext_for(mime: Option<&str>, default: &str) -> String {
    let m = mime.unwrap_or("").split(';').next().unwrap_or("").trim();
    match m {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "video/mp4" => "mp4",
        "video/quicktime" => "mov",
        "video/webm" => "webm",
        "audio/ogg" => "ogg",
        "audio/mpeg" => "mp3",
        "audio/mp4" | "audio/x-m4a" => "m4a",
        "audio/aac" => "aac",
        "audio/wav" | "audio/x-wav" => "wav",
        "application/pdf" => "pdf",
        "text/plain" => "txt",
        _ => default,
    }
    .to_string()
}

fn chrono_stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86400;
    let tod = secs % 86400;
    let h = tod / 3600;
    let m = (tod % 3600) / 60;
    let s = tod % 60;
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{y:04}{mo:02}{d:02}-{h:02}{m:02}{s:02}")
}

/// Terminal-friendly QR (unicode half blocks) for the pairing payload.
fn qr_ascii(code: &str) -> String {
    use qrcode::render::unicode::Dense1x2;
    match qrcode::QrCode::new(code.as_bytes()) {
        Ok(q) => q
            .render::<Dense1x2>()
            .dark_color(Dense1x2::Light)
            .light_color(Dense1x2::Dark)
            .quiet_zone(true)
            .build(),
        Err(e) => format!("(could not render QR: {e})"),
    }
}

/// Text + media kind of a (base) message. `kind` is one of image/video/voice/audio/document/sticker.
/// Locations and contacts have no file: their text says what was shared.
fn describe(base: &wa::Message) -> (String, Option<&'static str>) {
    let mut text = base
        .text_content()
        .or_else(|| base.get_caption())
        .unwrap_or("")
        .to_string();
    if let Some(shared) = shared_text(base) {
        text = if text.is_empty() { shared } else { format!("{text}\n{shared}") };
    }
    let kind = if base.image_message.is_set() {
        Some("image")
    } else if base.video_message.is_set() {
        Some("video")
    } else if let Some(a) = base.audio_message.as_option() {
        Some(if a.ptt.unwrap_or(false) { "voice" } else { "audio" })
    } else if base.document_message.is_set() {
        Some("document")
    } else if base.sticker_message.is_set() {
        Some("sticker")
    } else {
        None
    };
    (text, kind)
}

/// The context (reply, forward, mentions) of a message, on whichever kind carries it.
fn context_info(base: &wa::Message) -> Option<&wa::ContextInfo> {
    macro_rules! first {
        ($($f:ident),+) => {{
            None$(.or_else(|| base.$f.as_option().and_then(|m| m.context_info.as_option())))+
        }};
    }
    first!(
        extended_text_message,
        image_message,
        video_message,
        ptv_message,
        audio_message,
        document_message,
        sticker_message,
        location_message,
        live_location_message,
        contact_message,
        contacts_array_message,
        poll_creation_message,
        poll_creation_message_v3
    )
}

/// Live locations per chat (`files/state/whatsapp-live.json`).
fn live() -> &'static bia_mcp::share::LiveTracker {
    static T: OnceLock<bia_mcp::share::LiveTracker> = OnceLock::new();
    T.get_or_init(|| bia_mcp::share::LiveTracker::open(files_dir().join("state").join("whatsapp-live.json")))
}

/// The point of a live location message (live updates, or a location flagged live).
fn live_point(base: &wa::Message) -> Option<(f64, f64, Option<u32>)> {
    if let Some(l) = base.live_location_message.as_option() {
        return Some((l.degrees_latitude?, l.degrees_longitude?, l.accuracy_in_meters));
    }
    let l = base.location_message.as_option().filter(|l| l.is_live.unwrap_or(false))?;
    Some((l.degrees_latitude?, l.degrees_longitude?, l.accuracy_in_meters))
}

/// WhatsApp never says a share stopped: a share that goes quiet ends after
/// LIVE_QUIET_SECS. Twice a minute, tell the model about shares that ended.
fn ensure_live_sweeper() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        let _ = std::thread::Builder::new().name("wa-live".into()).spawn(|| loop {
            std::thread::sleep(Duration::from_secs(30));
            let now = now_secs().max(0) as u64;
            for (k, l) in live().sweep(now) {
                let chat = bia_mcp::share::live_chat(&k).to_string();
                Wa::global().inbound(&l.who, &chat, &format!("live-end-{now}"), &bia_mcp::share::live_ended_text(&l, now));
            }
        });
    });
}

impl Wa {
    /// Where people sharing a live location in `target` are now.
    pub fn live_location(&self, target: &str) -> Result<String, String> {
        let t = self.resolve_target(target)?;
        live()
            .status_for_chat(&t.key, now_secs().max(0) as u64)
            .ok_or_else(|| format!("no live location from {} (ask them to share one)", Self::format_chat(&t.key, &t.title)))
    }
}

/// Text for a shared location, live location, contact or several contacts.
fn shared_text(base: &wa::Message) -> Option<String> {
    use bia_mcp::share;
    let s = |o: &Option<String>| o.clone().unwrap_or_default();
    if let Some(l) = base.location_message.as_option() {
        let (lat, lon) = (l.degrees_latitude?, l.degrees_longitude?);
        let live = l.is_live.unwrap_or(false).then_some(0);
        return Some(share::location_text(lat, lon, &s(&l.name), &s(&l.address), l.accuracy_in_meters, live, &s(&l.comment)));
    }
    if let Some(l) = base.live_location_message.as_option() {
        let (lat, lon) = (l.degrees_latitude?, l.degrees_longitude?);
        return Some(share::location_text(lat, lon, "", "", l.accuracy_in_meters, Some(0), &s(&l.caption)));
    }
    let card = |c: &wa::message::ContactMessage| {
        let mut card = share::parse_vcard(&s(&c.vcard));
        if let Some(n) = c.display_name.clone().filter(|n| !n.trim().is_empty()) {
            card.name = n;
        }
        share::contact_text(&card, "")
    };
    if let Some(c) = base.contact_message.as_option() {
        return Some(card(c));
    }
    if let Some(a) = base.contacts_array_message.as_option() {
        let all: Vec<String> = a.contacts.iter().map(card).collect();
        return (!all.is_empty()).then(|| all.join("\n"));
    }
    None
}

fn hist_text(text: &str, kind: Option<&str>) -> String {
    match kind {
        Some(k) => format!("[{k}] {text}").trim().to_string(),
        None => text.to_string(),
    }
}

/// Profile photos must be JPEG; ffmpeg (when present) also crops to 640x640.
fn profile_jpeg(path: &Path) -> Result<Vec<u8>, String> {
    if bia_speech::audio::has_ffmpeg() {
        let out = bia_speech::paths::tmp().join(format!("photo-{}.jpg", bia_speech::paths::stamp()));
        let r = std::process::Command::new("ffmpeg")
            .args(["-y", "-i"])
            .arg(path)
            .args([
                "-vf",
                "scale=640:640:force_original_aspect_ratio=increase,crop=640:640",
                "-frames:v",
                "1",
                "-q:v",
                "3",
            ])
            .arg(&out)
            .output();
        if let Ok(o) = r {
            if o.status.success() {
                let bytes = std::fs::read(&out);
                let _ = std::fs::remove_file(&out);
                if let Ok(b) = bytes {
                    if !b.is_empty() {
                        return Ok(b);
                    }
                }
            }
        }
    }
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    if bytes.starts_with(&[0xFF, 0xD8]) {
        Ok(bytes)
    } else {
        Err("profile photo must be a JPEG (ffmpeg is needed to convert other formats)".into())
    }
}

// ---------------------------------------------------------------------------
// Chat book (key -> jid/title) and local message history
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Default)]
struct ChatInfo {
    jid: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    last_ts: i64,
}

#[derive(Default)]
struct ChatBook {
    chats: HashMap<String, ChatInfo>,
}

impl ChatBook {
    fn path() -> PathBuf {
        Wa::wa_dir().join("chats.json")
    }

    fn load() -> Self {
        let chats = std::fs::read(Self::path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Self { chats }
    }

    fn save(&self) {
        let p = Self::path();
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(b) = serde_json::to_vec(&self.chats) {
            let _ = std::fs::write(p, b);
        }
    }

    /// Returns true when something worth persisting changed.
    fn note(&mut self, key: &str, jid: &Jid, title: Option<&str>, ts: i64) -> bool {
        let e = self.chats.entry(key.to_string()).or_insert_with(|| ChatInfo {
            jid: String::new(),
            title: String::new(),
            last_ts: 0,
        });
        let mut changed = false;
        if e.jid.is_empty() {
            e.jid = jid.to_non_ad().to_string();
            changed = true;
        }
        if let Some(t) = title.map(str::trim).filter(|t| !t.is_empty()) {
            if e.title != t {
                e.title = t.to_string();
                changed = true;
            }
        }
        if ts > e.last_ts {
            e.last_ts = ts;
        }
        changed
    }

    fn title(&self, key: &str) -> Option<String> {
        self.chats
            .get(key)
            .map(|c| c.title.clone())
            .filter(|t| !t.is_empty())
    }

    fn jid(&self, key: &str) -> Option<Jid> {
        self.chats.get(key).and_then(|c| c.jid.parse::<Jid>().ok())
    }

    fn sorted(&self) -> Vec<(String, String, i64)> {
        let mut v: Vec<_> = self
            .chats
            .iter()
            .map(|(k, c)| (k.clone(), c.title.clone(), c.last_ts))
            .collect();
        v.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
        v
    }

    /// Case-insensitive: exact title matches first, then substring (title or key).
    fn find(&self, q: &str) -> Vec<(String, String)> {
        let q = q.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        let mut exact = Vec::new();
        let mut partial = Vec::new();
        for (k, t, _) in self.sorted() {
            let tl = t.to_lowercase();
            if !tl.is_empty() && tl == q {
                exact.push((k, t));
            } else if tl.contains(&q) || k.to_lowercase().contains(&q) {
                partial.push((k, t));
            }
        }
        exact.extend(partial);
        exact
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct HistLine {
    pub id: String,
    pub ts: i64,
    #[serde(default)]
    pub me: bool,
    /// Sender label (display name / phone).
    #[serde(default)]
    pub from: String,
    /// Sender JID (needed for admin revokes in groups).
    #[serde(default)]
    pub from_jid: String,
    #[serde(default)]
    pub text: String,
}

fn read_jsonl(path: &Path) -> Vec<HistLine> {
    let Ok(s) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    s.lines()
        .filter_map(|l| serde_json::from_str::<HistLine>(l).ok())
        .collect()
}

fn write_jsonl(path: &Path, lines: &[HistLine]) {
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let mut out = String::new();
    for l in lines {
        if let Ok(s) = serde_json::to_string(l) {
            out.push_str(&s);
            out.push('\n');
        }
    }
    let _ = std::fs::write(path, out);
}

fn append_jsonl(path: &Path, line: &HistLine) {
    use std::io::Write;
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Ok(s) = serde_json::to_string(line) {
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "{s}");
        }
    }
}

/// Per-chat JSONL cache under `whatsapp/history/`, lazily mirrored in memory.
/// WhatsApp has no server-side "fetch history" for linked devices, so this is
/// what the `history` tool reads.
#[derive(Default)]
struct HistoryStore {
    chats: HashMap<String, Vec<HistLine>>,
}

impl HistoryStore {
    fn path(key: &str) -> PathBuf {
        Wa::wa_dir().join("history").join(format!("{}.jsonl", safe_name(key)))
    }

    fn load(&mut self, key: &str) -> &mut Vec<HistLine> {
        let path = Self::path(key);
        self.chats
            .entry(key.to_string())
            .or_insert_with(|| read_jsonl(&path))
    }

    /// False when the id was already cached.
    fn append(&mut self, key: &str, line: HistLine) -> bool {
        let path = Self::path(key);
        let v = self.load(key);
        if v.iter().any(|l| l.id == line.id) {
            return false;
        }
        v.push(line.clone());
        if v.len() > HIST_CAP {
            let extra = v.len() - HIST_CAP;
            v.drain(..extra);
            write_jsonl(&path, v);
        } else {
            append_jsonl(&path, &line);
        }
        true
    }

    /// Merge older/imported lines; keeps the chat time-ordered and capped.
    fn merge(&mut self, key: &str, lines: Vec<HistLine>) -> usize {
        let path = Self::path(key);
        let v = self.load(key);
        let mut added = 0;
        for l in lines {
            if !v.iter().any(|x| x.id == l.id) {
                v.push(l);
                added += 1;
            }
        }
        if added > 0 {
            v.sort_by_key(|l| l.ts);
            if v.len() > HIST_CAP {
                let extra = v.len() - HIST_CAP;
                v.drain(..extra);
            }
            write_jsonl(&path, v);
        }
        added
    }

    /// Newest first. `before` = id of the last line of the previous page.
    fn page(&mut self, key: &str, limit: usize, before: Option<&str>) -> Result<Vec<HistLine>, String> {
        let v = self.load(key);
        let end = match before {
            None => v.len(),
            Some(id) => v
                .iter()
                .position(|l| l.id == id)
                .ok_or_else(|| format!("unknown message id for before: {id}"))?,
        };
        Ok(v[..end].iter().rev().take(limit).cloned().collect())
    }

    fn get(&mut self, key: &str, id: &str) -> Option<HistLine> {
        self.load(key).iter().find(|l| l.id == id).cloned()
    }

    /// Replace the text of a cached line (after an edit). False when the id is unknown.
    fn set_text(&mut self, key: &str, id: &str, text: &str) -> bool {
        let path = Self::path(key);
        let v = self.load(key);
        let Some(l) = v.iter_mut().find(|l| l.id == id) else {
            return false;
        };
        l.text = text.to_string();
        write_jsonl(&path, v);
        true
    }

    fn own_ids(&mut self, key: &str) -> Vec<String> {
        self.load(key)
            .iter()
            .rev()
            .filter(|l| l.me)
            .take(200)
            .map(|l| l.id.clone())
            .collect()
    }

    fn delete_ids(&mut self, key: &str, ids: &[String]) -> usize {
        let path = Self::path(key);
        let v = self.load(key);
        let before = v.len();
        v.retain(|l| !ids.contains(&l.id));
        let removed = before - v.len();
        if removed > 0 {
            write_jsonl(&path, v);
        }
        removed
    }

    fn clear(&mut self, key: &str) {
        self.chats.insert(key.to_string(), Vec::new());
        let _ = std::fs::remove_file(Self::path(key));
    }
}

struct ParsedConv {
    jid: Jid,
    name: Option<String>,
    lines: Vec<HistLine>,
}

/// Decode a HistorySync blob into per-chat cache lines (CPU heavy; run off the runtime).
fn parse_history(h: &LazyHistorySync) -> Vec<ParsedConv> {
    let Some(hs) = h.get() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for conv in &hs.conversations {
        let Ok(jid) = conv.id.parse::<Jid>() else {
            continue;
        };
        if jid.is_status_broadcast() || jid.is_newsletter() || jid.is_broadcast_list() {
            continue;
        }
        let mut lines = Vec::new();
        for hm in &conv.messages {
            let Some(wmi) = hm.message.as_option() else {
                continue;
            };
            let Some(mk) = wmi.key.as_option() else {
                continue;
            };
            let Some(id) = mk.id.clone() else {
                continue;
            };
            let Some(msg) = wmi.message.as_option() else {
                continue;
            };
            let base = msg.get_base_message();
            if base.protocol_message.is_set() || base.reaction_message.is_set() {
                continue;
            }
            let (text, kind) = describe(base);
            if text.is_empty() && kind.is_none() {
                continue;
            }
            let me = mk.from_me.unwrap_or(false);
            let from_jid = mk.participant.clone().unwrap_or_default();
            let from = if me {
                "me".to_string()
            } else {
                wmi.push_name.clone().unwrap_or_default()
            };
            lines.push(HistLine {
                id,
                ts: wmi.message_timestamp.unwrap_or(0) as i64,
                me,
                from,
                from_jid,
                text: hist_text(&text, kind),
            });
        }
        lines.sort_by_key(|l| l.ts);
        if lines.len() > HISTSYNC_PER_CHAT {
            let extra = lines.len() - HISTSYNC_PER_CHAT;
            lines.drain(..extra);
        }
        out.push(ParsedConv {
            jid,
            name: conv.name.clone(),
            lines,
        });
    }
    out
}

// ---------------------------------------------------------------------------
// Wa: lifecycle
// ---------------------------------------------------------------------------

impl Wa {
    pub fn global() -> &'static Wa {
        static G: OnceLock<Wa> = OnceLock::new();
        G.get_or_init(|| {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_name("wa-tokio")
                .build()
                .expect("tokio runtime");
            let wa = Wa {
                inner: Mutex::new(Inner {
                    client: None,
                    ctx: None,
                    phase: "disconnected".into(),
                    qr: None,
                    qr_seq: 0,
                    banned: read_bans(),
                    generation: 0,
                    suppress_reconnect: false,
                }),
                connect_lock: tokio::sync::Mutex::new(()),
                reconnecting: AtomicBool::new(false),
                book: Mutex::new(ChatBook::load()),
                hist: Mutex::new(HistoryStore::default()),
                ban_replies: Mutex::new(HashMap::new()),
                rt,
            };
            wa.ensure_health_supervisor();
            wa
        })
    }

    pub fn block_on<F: std::future::Future>(&self, f: F) -> F::Output {
        self.rt.block_on(f)
    }

    pub fn spawn<F>(&self, f: F)
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        self.rt.spawn(f);
    }

    pub fn set_ctx(&self, ctx: Ctx) {
        self.inner.lock().unwrap().ctx = Some(ctx.clone());
        ensure_live_sweeper();
        // messages that arrived before the agent link existed
        let held: Vec<_> = held_inbound().lock().map(|mut h| std::mem::take(&mut *h)).unwrap_or_default();
        for (identity, channel, id, text, issued) in held {
            ctx.inbound_at(&identity, &channel, &id, &text, issued);
        }
    }

    pub fn ctx_clone(&self) -> Option<Ctx> {
        self.inner.lock().unwrap().ctx.clone()
    }

    pub fn wa_dir() -> PathBuf {
        files_dir().join("whatsapp")
    }

    pub fn db_path() -> PathBuf {
        Self::wa_dir().join("whatsapp.db")
    }

    pub fn cfg_path() -> PathBuf {
        files_dir().join("state").join("whatsapp.json")
    }

    pub fn load_cfg() -> Cfg {
        std::fs::read(Self::cfg_path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save_cfg(c: &Cfg) -> Result<(), String> {
        if let Some(p) = Self::cfg_path().parent() {
            std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        std::fs::write(
            Self::cfg_path(),
            serde_json::to_vec_pretty(c).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        unsafe { libc::sync() };
        Ok(())
    }

    fn remove_db_files() {
        let db = Self::db_path();
        for suffix in ["", "-wal", "-shm", "-journal"] {
            let mut p = db.clone().into_os_string();
            p.push(suffix);
            let _ = std::fs::remove_file(PathBuf::from(p));
        }
        unsafe { libc::sync() };
    }

    pub fn phase(&self) -> String {
        let (phase, client) = {
            let g = self.inner.lock().unwrap();
            (g.phase.clone(), g.client.clone())
        };
        if client.as_ref().is_some_and(|c| c.is_logged_in()) {
            "ready".into()
        } else {
            phase
        }
    }

    fn set_phase(&self, p: &str) {
        self.inner.lock().unwrap().phase = p.into();
    }

    pub fn client(&self) -> Result<Arc<Client>, String> {
        self.inner
            .lock()
            .unwrap()
            .client
            .clone()
            .ok_or_else(|| "whatsapp client not running; call restore or login".into())
    }

    pub fn log(&self, msg: impl Into<String>) {
        if let Some(ctx) = self.ctx_clone() {
            ctx.log(msg.into());
        } else {
            eprintln!("whatsapp: {}", msg.into());
        }
    }

    /// A message for the model on channel `whatsapp:<key>`.
    pub fn inbound(&self, identity: &str, key: &str, id: &str, text: &str) {
        self.inbound_at(identity, key, id, text, 0);
    }

    /// [`Wa::inbound`] with when it was written (WhatsApp's timestamp, Unix seconds; 0 = now).
    pub fn inbound_at(&self, identity: &str, key: &str, id: &str, text: &str, issued: i64) {
        let channel = format!("whatsapp:{key}");
        match self.ctx_clone() {
            Some(ctx) => ctx.inbound_at(identity, &channel, id, text, issued),
            None => hold_inbound(identity, &channel, id, text, issued),
        }
    }

    /// A system notice (QR codes, link status). Must be identity=system so the
    /// model does not treat it as a physical-console user and reply on console.
    pub fn console(&self, text: &str) {
        if let Some(ctx) = self.ctx_clone() {
            ctx.inbound("system", "console", "", text);
        } else {
            eprintln!("whatsapp: {text}");
        }
    }

    pub fn format_chat(key: &str, title: &str) -> String {
        if title.is_empty() || title == key || title == format!("+{key}") {
            format!("whatsapp:{key}")
        } else {
            format!("whatsapp:{key}  {title}")
        }
    }

    fn allow_auto_reconnect(&self) -> bool {
        let suppressed = self.inner.lock().unwrap().suppress_reconnect;
        !suppressed && Self::load_cfg().paired && Self::db_path().exists()
    }

    fn ensure_health_supervisor(&self) {
        static STARTED: OnceLock<()> = OnceLock::new();
        STARTED.get_or_init(|| {
            let _ = std::thread::Builder::new()
                .name("wa-health".into())
                .spawn(|| loop {
                    std::thread::sleep(Duration::from_secs(20));
                    Wa::global().health_tick();
                });
        });
    }

    /// whatsapp-rust reconnects dropped sockets itself; this only revives a bot
    /// whose run loop ended (or never started) while we still hold a paired session.
    fn health_tick(&self) {
        if self.allow_auto_reconnect() && self.client().is_err() {
            self.kick_reconnect();
        }
    }

    /// Called by status/restore: heal without requiring the model to reconnect manually.
    pub fn heal_if_needed(&self) {
        self.ensure_health_supervisor();
        if self.allow_auto_reconnect() && self.client().is_err() {
            self.kick_reconnect();
        }
    }

    fn kick_reconnect(&self) {
        if !self.allow_auto_reconnect() {
            return;
        }
        if self.reconnecting.swap(true, Ordering::SeqCst) {
            return;
        }
        let _ = std::thread::Builder::new()
            .name("wa-reconnect".into())
            .spawn(|| {
                let wa = Wa::global();
                let mut delay = Duration::from_secs(2);
                for attempt in 1u32..=30 {
                    if !wa.allow_auto_reconnect() || wa.client().is_ok() {
                        break;
                    }
                    match wa.ensure_connected() {
                        Ok(_) => {
                            wa.log(format!("reconnected (attempt {attempt})"));
                            break;
                        }
                        Err(e) => wa.log(format!("reconnect attempt {attempt} failed: {e}")),
                    }
                    std::thread::sleep(delay);
                    delay = (delay.saturating_mul(2)).min(Duration::from_secs(60));
                }
                wa.reconnecting.store(false, Ordering::SeqCst);
            });
    }

    pub fn ensure_connected(&self) -> Result<Arc<Client>, String> {
        if let Ok(c) = self.client() {
            return Ok(c);
        }
        self.block_on(self.connect_async())
    }

    async fn connect_async(&self) -> Result<Arc<Client>, String> {
        let _guard = self.connect_lock.lock().await;
        if let Ok(c) = self.client() {
            return Ok(c);
        }
        std::fs::create_dir_all(Self::wa_dir()).map_err(|e| e.to_string())?;
        let store = SqliteStore::new(&Self::db_path().to_string_lossy())
            .await
            .map_err(|e| format!("open session db: {e}"))?;
        let generation = {
            let mut g = self.inner.lock().unwrap();
            g.generation = g.generation.wrapping_add(1);
            g.suppress_reconnect = false;
            g.qr = None;
            g.phase = "connecting".into();
            g.generation
        };

        let bot = Bot::builder()
            .with_backend(store)
            .with_device_props(
                whatsapp_rust::wacore::store::DevicePropsOverride::new().with_os("bia"),
            )
            .on_qr_code(move |code, timeout| async move {
                Wa::global().on_qr(generation, &code, timeout);
            })
            .on_connected(move |client| async move {
                Wa::global().on_connected(generation, client);
            })
            .on_logged_out(move |info| async move {
                let reason = format!("{:?}", info.reason);
                Wa::global().on_logged_out(generation, reason).await;
            })
            .on_message(|ctx| async move {
                Wa::global().on_message(ctx).await;
            })
            .on_event_for(
                &[
                    EventKind::PairSuccess,
                    EventKind::PairError,
                    EventKind::PairingQrCodesExhausted,
                    EventKind::QrScannedWithoutMultidevice,
                    EventKind::ClientOutdated,
                    EventKind::TemporaryBan,
                    EventKind::ConnectFailure,
                    EventKind::Disconnected,
                    EventKind::IncomingCall,
                    EventKind::MissedCall,
                    EventKind::CallEndedElsewhere,
                    EventKind::HistorySync,
                    EventKind::PushNameUpdate,
                ],
                move |event, client| async move {
                    Wa::global().on_event(generation, event, client).await;
                },
            )
            .build()
            .await
            .map_err(|e| format!("build bot: {e}"))?;

        let handle = bot.spawn();
        let client = handle.client();
        self.inner.lock().unwrap().client = Some(client.clone());
        // The handle is a Future that resolves when the run loop exits; dropping
        // it would abort the bot, so the watcher owns it.
        self.rt.spawn(async move {
            handle.await;
            Wa::global().on_bot_ended(generation);
        });
        Ok(client)
    }

    fn on_bot_ended(&self, generation: u64) {
        let reconnect = {
            let mut g = self.inner.lock().unwrap();
            if g.generation != generation {
                return;
            }
            g.client = None;
            if g.phase == "ready" || g.phase == "connecting" {
                g.phase = "disconnected".into();
            }
            !g.suppress_reconnect
        };
        self.log("bot run loop ended");
        if reconnect {
            self.kick_reconnect();
        }
    }

    /// Stop the live bot (keeps the session db). Safe to await from handlers.
    async fn stop_async(&self, reason: &str) {
        let client = {
            let mut g = self.inner.lock().unwrap();
            g.generation = g.generation.wrapping_add(1);
            g.qr = None;
            if g.phase != "logged_out" && g.phase != "qr_expired" {
                g.phase = "disconnected".into();
            }
            g.client.take()
        };
        crate::qr_win::close();
        if let Some(c) = client {
            let _ = tokio::time::timeout(Duration::from_secs(10), c.disconnect()).await;
        }
        self.log(format!("connection stopped: {reason}"));
    }

    fn drop_connection(&self, reason: &str) {
        self.block_on(self.stop_async(reason));
    }

    // -- bot callbacks ------------------------------------------------------

    fn current_generation_is(&self, generation: u64) -> bool {
        self.inner.lock().unwrap().generation == generation
    }

    fn on_qr(&self, generation: u64, code: &str, timeout: Duration) {
        let seq = {
            let mut g = self.inner.lock().unwrap();
            if g.generation != generation {
                return;
            }
            g.qr = Some(code.to_string());
            g.qr_seq += 1;
            g.phase = "waiting_qr".into();
            g.qr_seq
        };
        let art = qr_ascii(code);
        let secs = timeout.as_secs();
        match crate::qr_win::show(code, &art, seq, secs) {
            Ok(notice) => self.console(&notice),
            Err(e) => {
                // Still write the PNG for the model / web attach even without a GUI window.
                let png = crate::qr_win::png_path();
                let png_note = if png.is_file() {
                    format!(" PNG at {} (temporary; deleted when linked).", png.display())
                } else {
                    String::new()
                };
                self.console(&format!(
                    "[whatsapp qr #{seq}] Scan with WhatsApp > Settings > Linked devices > Link a device (valid ~{secs}s; GUI window unavailable: {e}).{png_note}\n{art}\nqr-string: {code}"
                ));
            }
        }
    }

    fn on_connected(&self, generation: u64, client: Arc<Client>) {
        if !self.current_generation_is(generation) {
            return;
        }
        {
            let mut g = self.inner.lock().unwrap();
            g.phase = "ready".into();
            g.qr = None;
        }
        crate::qr_win::close();
        // Re-apply branding on every connect (also covers sessions paired before "bia").
        let client_brand = client.clone();
        self.spawn(async move {
            client_brand
                .set_device_props(
                    whatsapp_rust::wacore::store::DevicePropsOverride::new().with_os("bia"),
                )
                .await;
        });
        let phone = client.pn().map(|j| j.user.to_string()).unwrap_or_default();
        let mut cfg = Self::load_cfg();
        let mut changed = false;
        if !phone.is_empty() && cfg.phone != phone {
            cfg.phone = phone;
            changed = true;
        }
        if !cfg.paired {
            cfg.paired = true;
            changed = true;
        }
        if changed {
            let _ = Self::save_cfg(&cfg);
        }
        self.log("connected");
    }

    async fn on_logged_out(&self, generation: u64, reason: String) {
        {
            let mut g = self.inner.lock().unwrap();
            // Our own logout()/stop bumps the generation first, so only a real
            // server-side unlink gets through here.
            if g.generation != generation || g.suppress_reconnect {
                return;
            }
            g.suppress_reconnect = true;
            g.phase = "logged_out".into();
        }
        crate::qr_win::close();
        self.stop_async("logged out by WhatsApp").await;
        Self::remove_db_files();
        let mut cfg = Self::load_cfg();
        cfg.paired = false;
        let _ = Self::save_cfg(&cfg);
        self.console(&format!(
            "[whatsapp logged out ({reason})] The linked device was removed. Call whatsapp_login to link again."
        ));
    }

    async fn on_event(&self, generation: u64, event: Arc<Event>, client: Arc<Client>) {
        if !self.current_generation_is(generation) {
            return;
        }
        match &*event {
            Event::PairSuccess(p) => {
                let phone = p.id.user.to_string();
                let mut cfg = Self::load_cfg();
                cfg.phone = phone.clone();
                cfg.paired = true;
                let _ = Self::save_cfg(&cfg);
                self.set_phase("pairing");
                self.console(&format!(
                    "[whatsapp linked as +{phone}] Finishing setup; whatsapp_status will show auth=ready."
                ));
            }
            Event::PairError(e) => {
                self.console(&format!("[whatsapp pairing failed: {}]", e.error));
            }
            Event::PairingQrCodesExhausted(_) => {
                if !Self::load_cfg().paired {
                    self.set_phase("qr_expired");
                    self.console("[whatsapp qr expired] Nobody scanned the QR. Call whatsapp_login again.");
                    let wa = Wa::global();
                    self.rt.spawn(async move {
                        wa.stop_async("qr exhausted").await;
                    });
                }
            }
            Event::QrScannedWithoutMultidevice(_) => {
                self.console("[whatsapp] The scanning phone does not support linked devices; update WhatsApp.");
            }
            Event::ClientOutdated(_) => {
                self.console("[whatsapp] WhatsApp rejected this client version (outdated). Update whatsapp-rust.");
            }
            Event::TemporaryBan(b) => {
                self.console(&format!(
                    "[whatsapp temporary ban] code={:?} expires in {}s{}",
                    b.code,
                    b.expire.num_seconds(),
                    b.message.as_deref().map(|m| format!(": {m}")).unwrap_or_default()
                ));
            }
            Event::ConnectFailure(f) => {
                self.log(format!(
                    "connect failure: {:?}{}",
                    f.reason,
                    f.message.as_deref().map(|m| format!(" ({m})")).unwrap_or_default()
                ));
            }
            Event::Disconnected(_) => {
                let mut g = self.inner.lock().unwrap();
                if g.phase == "ready" {
                    g.phase = "reconnecting".into();
                }
            }
            Event::IncomingCall(ic) => {
                Calls::global().on_incoming(client, ic.clone()).await;
            }
            Event::MissedCall(m) => {
                Calls::global()
                    .on_missed(&client, m.from.clone(), m.call_id.clone())
                    .await;
            }
            Event::CallEndedElsewhere(c) => {
                Calls::global().on_answered_elsewhere(&c.call_id);
            }
            Event::PushNameUpdate(p) => {
                let key = key_of_jid_async(&client, &p.jid).await;
                if !p.new_push_name.trim().is_empty() {
                    self.note_chat(&key, &p.jid, Some(&p.new_push_name), 0);
                }
            }
            Event::HistorySync(h) => {
                let h = (**h).clone();
                let parsed = tokio::task::spawn_blocking(move || parse_history(&h))
                    .await
                    .unwrap_or_default();
                let mut total = 0usize;
                for conv in parsed {
                    let key = key_of_jid_async(&client, &conv.jid).await;
                    let last_ts = conv.lines.last().map(|l| l.ts).unwrap_or(0);
                    self.note_chat(&key, &conv.jid, conv.name.as_deref(), last_ts);
                    total += self.hist.lock().unwrap().merge(&key, conv.lines);
                }
                if total > 0 {
                    self.log(format!("history sync: cached {total} message(s)"));
                }
            }
            _ => {}
        }
    }

    // -- chat book ------------------------------------------------------------

    fn note_chat(&self, key: &str, jid: &Jid, title: Option<&str>, ts: i64) {
        let mut b = self.book.lock().unwrap();
        if b.note(key, jid, title, ts) {
            b.save();
        }
    }

    pub fn title_for(&self, key: &str) -> String {
        self.book
            .lock()
            .unwrap()
            .title(key)
            .unwrap_or_else(|| {
                if key.contains('@') {
                    key.to_string()
                } else {
                    format!("+{key}")
                }
            })
    }

    async fn group_title(&self, client: &Client, key: &str, jid: &Jid) -> Option<String> {
        let known = self.book.lock().unwrap().title(key);
        if known.is_some() {
            return known;
        }
        match tokio::time::timeout(Duration::from_secs(5), client.groups().get_metadata(jid)).await {
            Ok(Ok(meta)) if !meta.subject.trim().is_empty() => Some(meta.subject),
            _ => None,
        }
    }

    // -- bans -----------------------------------------------------------------

    pub fn is_banned(&self, key: &str) -> bool {
        self.inner.lock().unwrap().banned.contains(key)
    }

    pub fn banned_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.inner.lock().unwrap().banned.iter().cloned().collect();
        ids.sort_unstable();
        ids
    }

    pub fn set_banned(&self, key: &str, on: bool) -> Result<bool, String> {
        let (changed, ids) = {
            let mut g = self.inner.lock().unwrap();
            let changed = if on {
                g.banned.insert(key.to_string())
            } else {
                g.banned.remove(key)
            };
            (changed, g.banned.iter().cloned().collect::<Vec<_>>())
        };
        write_bans(&ids)?;
        if on {
            Calls::global().reject_banned(key);
        }
        Ok(changed)
    }

    // -- auth -------------------------------------------------------------------

    fn current_qr(&self) -> Option<String> {
        self.inner.lock().unwrap().qr.clone()
    }

    fn own_phone(&self) -> String {
        if let Ok(c) = self.client() {
            if let Some(j) = c.pn() {
                return j.user.to_string();
            }
        }
        Self::load_cfg().phone
    }

    /// Start (or continue) QR pairing. Returns a short status; the scannable QR
    /// opens in a dedicated window on display :0 (console gets a short notice).
    pub fn login(&self) -> Result<String, String> {
        if let Ok(c) = self.client() {
            if c.is_logged_in() {
                return Ok(format!("already logged in as +{}", self.own_phone()));
            }
        }
        let paired = Self::load_cfg().paired && Self::db_path().exists();
        if !paired {
            let have_fresh_qr = self.client().is_ok()
                && self.phase() == "waiting_qr"
                && self.current_qr().is_some();
            if !have_fresh_qr {
                // Fresh pairing: drop any stale half-linked session.
                self.drop_connection("login (fresh pairing)");
                Self::remove_db_files();
                let mut cfg = Self::load_cfg();
                cfg.paired = false;
                cfg.phone.clear();
                Self::save_cfg(&cfg)?;
            }
        }
        self.inner.lock().unwrap().suppress_reconnect = false;
        let client = self.ensure_connected()?;

        let deadline = Instant::now() + Duration::from_secs(if paired { 25 } else { 30 });
        loop {
            if client.is_logged_in() {
                return Ok(format!("auth: ready — logged in as +{}", self.own_phone()));
            }
            if !paired {
                if let Some(qr) = self.current_qr() {
                    let png = crate::qr_win::png_path();
                    return Ok(format!(
                        "auth: waiting_qr — scannable PNG at {} (temporary; removed when auth completes). Also shown in the WhatsApp QR window on display :0. Scan with WhatsApp > Settings > Linked devices > Link a device. It rotates about every 20s. Then call whatsapp_status until auth=ready.\nqr-string: {qr}",
                        png.display()
                    ));
                }
            }
            if Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        if paired {
            Ok(format!(
                "auth: {} — saved session is still connecting; check whatsapp_status shortly",
                self.phase()
            ))
        } else {
            Err("timed out waiting for the WhatsApp QR; check the network and call login again".into())
        }
    }

    pub fn require_ready(&self) -> Result<Arc<Client>, String> {
        let paired = Self::load_cfg().paired && Self::db_path().exists();
        let client = match self.client() {
            Ok(c) => c,
            Err(_) if paired => {
                self.inner.lock().unwrap().suppress_reconnect = false;
                self.ensure_connected()?
            }
            Err(_) => {
                return Err(format!(
                    "not logged in ({}); call whatsapp_login and scan the QR",
                    self.phase()
                ))
            }
        };
        if client.is_logged_in() {
            return Ok(client);
        }
        if client.pn().is_none() && !paired {
            return Err(format!(
                "not logged in ({}); call whatsapp_login and scan the QR",
                self.phase()
            ));
        }
        match self.block_on(client.wait_for_connected(Duration::from_secs(15))) {
            Ok(()) => Ok(client),
            Err(e) => Err(format!("whatsapp not connected yet ({e}); retry shortly")),
        }
    }

    pub fn logout(&self) -> Result<String, String> {
        let client = {
            let mut g = self.inner.lock().unwrap();
            g.suppress_reconnect = true;
            g.generation = g.generation.wrapping_add(1);
            g.qr = None;
            g.phase = "disconnected".into();
            g.client.take()
        };
        crate::qr_win::close();
        if let Some(c) = client {
            self.block_on(async {
                let _ = tokio::time::timeout(Duration::from_secs(15), c.logout()).await;
            });
        }
        Calls::global().reset();
        Self::remove_db_files();
        let mut cfg = Self::load_cfg();
        cfg.paired = false;
        cfg.phone.clear();
        let _ = Self::save_cfg(&cfg);
        Ok("logged out; call login to link WhatsApp again".into())
    }

    pub fn restore(&self) -> Result<String, String> {
        self.ensure_health_supervisor();
        let cfg = Self::load_cfg();
        if !(cfg.paired && Self::db_path().exists()) {
            self.set_phase("disconnected");
            return Ok("whatsapp not logged in; call login and scan the QR".into());
        }
        self.inner.lock().unwrap().suppress_reconnect = false;
        let client = match self.ensure_connected() {
            Ok(c) => c,
            Err(e) => {
                self.kick_reconnect();
                return Err(e);
            }
        };
        match self.block_on(client.wait_for_connected(Duration::from_secs(20))) {
            Ok(()) => Ok(format!("whatsapp connected as +{}", self.own_phone())),
            Err(e) => Ok(format!(
                "whatsapp starting ({e}); it keeps retrying in the background (phase={})",
                self.phase()
            )),
        }
    }

    // -- targets ----------------------------------------------------------------

    fn target_for(&self, jid: Jid) -> Target {
        let jid = jid.to_non_ad();
        let key = key_of_jid(&jid);
        let title = self.title_for(&key);
        Target { key, jid, title }
    }

    /// Offline resolution of a chat_id: `15551234567`, `whatsapp:15551234567`,
    /// `+1 555 123 4567`, `<id>@g.us`, `<digits>@lid`, or a known chat name.
    pub fn resolve_target(&self, target: &str) -> Result<Target, String> {
        let t = normalize_target(target);
        if t.is_empty() {
            return Err("empty chat_id".into());
        }
        if t.contains('@') {
            let jid: Jid = t.parse().map_err(|e| format!("bad WhatsApp id {t}: {e}"))?;
            return Ok(self.target_for(jid));
        }
        if looks_numeric(&t) {
            let digits = digits_only(&t);
            if digits.len() < 5 {
                return Err(format!("chat_id too short: {t}"));
            }
            return Ok(self.target_for(Jid::pn(digits.as_str())));
        }
        let hits = self.book.lock().unwrap().find(&t);
        match hits.len() {
            0 => Err(format!(
                "no WhatsApp chat matching {t:?}; use a phone number with country code or whatsapp_search"
            )),
            1 => {
                let (key, _) = &hits[0];
                self.target_from_key(key)
            }
            _ => {
                if hits[0].1.to_lowercase() == t.to_lowercase()
                    && hits.get(1).map(|h| h.1.to_lowercase()) != Some(t.to_lowercase())
                {
                    return self.target_from_key(&hits[0].0);
                }
                let list = hits
                    .iter()
                    .take(5)
                    .map(|(k, n)| Self::format_chat(k, n))
                    .collect::<Vec<_>>()
                    .join("; ");
                Err(format!("ambiguous chat {t:?}: {list}"))
            }
        }
    }

    fn target_from_key(&self, key: &str) -> Result<Target, String> {
        let jid = self
            .book
            .lock()
            .unwrap()
            .jid(key)
            .or_else(|| jid_of_key(key))
            .ok_or_else(|| format!("cannot address chat {key}"))?;
        Ok(Target {
            key: key.to_string(),
            jid: jid.to_non_ad(),
            title: self.title_for(key),
        })
    }

    /// Like `resolve_target` but confirms a bare phone number is registered on WhatsApp.
    pub fn resolve_verified(&self, target: &str) -> Result<Target, String> {
        let t = self.resolve_target(target)?;
        if !t.jid.is_pn() {
            return Ok(t);
        }
        let client = self.require_ready()?;
        let jid = t.jid.clone();
        let res = self
            .block_on(async { client.contacts().is_on_whatsapp(std::slice::from_ref(&jid)).await })
            .map_err(|e| format!("resolve: {e}"))?;
        if res.iter().any(|r| r.is_registered) {
            Ok(t)
        } else {
            Err(format!("+{} is not on WhatsApp", t.key))
        }
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<(String, String)>, String> {
        let client = self.require_ready()?;
        let q = query.trim().to_string();
        let limit = limit.max(1);
        self.block_on(async {
            let mut rows: Vec<(String, String)> = Vec::new();
            let mut seen: HashSet<String> = HashSet::new();
            let mut push = |rows: &mut Vec<(String, String)>, key: String, title: String| {
                if seen.insert(key.clone()) {
                    rows.push((key, title));
                }
            };

            if looks_numeric(&q) && digits_only(&q).len() >= 5 {
                let digits = digits_only(&q);
                let jid = Jid::pn(digits.as_str());
                let res = client
                    .contacts()
                    .is_on_whatsapp(std::slice::from_ref(&jid))
                    .await
                    .map_err(|e| format!("search: {e}"))?;
                if res.iter().any(|r| r.is_registered) {
                    let title = self.title_for(&digits);
                    push(&mut rows, digits, title);
                }
            }

            let known = self.book.lock().unwrap().find(&q);
            for (k, t) in known {
                let title = if t.is_empty() { self.title_for(&k) } else { t };
                push(&mut rows, k, title);
            }

            if let Ok(groups) = client.groups().get_participating().await {
                let ql = q.to_lowercase();
                for (jid, meta) in groups {
                    if ql.is_empty() || meta.subject.to_lowercase().contains(&ql) {
                        let key = key_of_jid(&jid);
                        self.note_chat(&key, &jid, Some(&meta.subject), 0);
                        push(&mut rows, key, meta.subject.clone());
                    }
                }
            }
            rows.truncate(limit);
            Ok(rows)
        })
    }

    pub fn chats(&self) -> Result<Vec<(String, String, bool)>, String> {
        let rows = self.book.lock().unwrap().sorted();
        Ok(rows
            .into_iter()
            .take(50)
            .map(|(k, t, _)| {
                let title = if t.is_empty() { self.title_for(&k) } else { t };
                let banned = self.is_banned(&k);
                (k, title, banned)
            })
            .collect())
    }

    // -- sending ----------------------------------------------------------------

    fn record_outgoing(&self, key: &str, id: &str, text: &str) {
        self.hist.lock().unwrap().append(
            key,
            HistLine {
                id: id.to_string(),
                ts: now_secs(),
                me: true,
                from: "me".into(),
                from_jid: String::new(),
                text: text.to_string(),
            },
        );
    }

    pub fn send(&self, target: &str, text: &str, files: &[String]) -> Result<String, String> {
        self.send_cancellable(target, text, files, &|| false)
    }

    /// `send` that stops (dropping an upload in progress) once `cancelled` turns true.
    pub fn send_cancellable(&self, target: &str, text: &str, files: &[String], cancelled: &dyn Fn() -> bool) -> Result<String, String> {
        let client = self.require_ready()?;
        let t = self.resolve_target(target)?;
        self.note_chat(&t.key, &t.jid, None, now_secs());
        // the model writes markdown; WhatsApp has its own *bold* syntax
        let text = &bia_mcp::md::to_whatsapp(text);
        self.block_on(async {
            tokio::select! {
                r = self.send_async(&client, &t, text, files) => r,
                _ = async {
                    while !cancelled() {
                        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                    }
                } => Err("cancelled; messages already sent stay".into()),
            }
        })
    }

    /// Send a map pin (with name/address it shows as a place).
    pub fn send_location(&self, target: &str, lat: f64, lon: f64, name: &str, address: &str) -> Result<String, String> {
        bia_mcp::share::check_coords(lat, lon)?;
        let client = self.require_ready()?;
        let t = self.resolve_target(target)?;
        self.note_chat(&t.key, &t.jid, None, now_secs());
        let opt = |v: &str| Some(v.trim().to_string()).filter(|v| !v.is_empty());
        let msg = wa::Message {
            location_message: buffa::MessageField::some(wa::message::LocationMessage {
                degrees_latitude: Some(lat),
                degrees_longitude: Some(lon),
                name: opt(name),
                address: opt(address),
                ..Default::default()
            }),
            ..Default::default()
        };
        let shown = bia_mcp::share::location_text(lat, lon, name, address, None, None, "");
        self.block_on(async {
            let r = client.send_message(t.jid.clone(), msg).await.map_err(|e| format!("send location: {e}"))?;
            self.record_outgoing(&t.key, &r.message_id, &shown);
            Ok(format!("sent location message id {} → {}", r.message_id, Self::format_chat(&t.key, &t.title)))
        })
    }

    /// Send a contact card; its phone carries `waid`, so WhatsApp offers "Message".
    pub fn send_contact(&self, target: &str, name: &str, phone: &str, email: &str) -> Result<String, String> {
        if name.trim().is_empty() || phone.trim().is_empty() {
            return Err("a contact needs a name and a phone number".into());
        }
        let client = self.require_ready()?;
        let t = self.resolve_target(target)?;
        self.note_chat(&t.key, &t.jid, None, now_secs());
        let vcard = bia_mcp::share::build_vcard(name, phone, email, true);
        let msg = wa::Message {
            contact_message: buffa::MessageField::some(wa::message::ContactMessage {
                display_name: Some(name.trim().to_string()),
                vcard: Some(vcard.clone()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let shown = bia_mcp::share::contact_text(&bia_mcp::share::parse_vcard(&vcard), "");
        self.block_on(async {
            let r = client.send_message(t.jid.clone(), msg).await.map_err(|e| format!("send contact: {e}"))?;
            self.record_outgoing(&t.key, &r.message_id, &shown);
            Ok(format!("sent contact {} message id {} → {}", name.trim(), r.message_id, Self::format_chat(&t.key, &t.title)))
        })
    }

    async fn send_async(
        &self,
        client: &Arc<Client>,
        t: &Target,
        text: &str,
        files: &[String],
    ) -> Result<String, String> {
        if files.is_empty() {
            let r = client
                .send_text(t.jid.clone(), text)
                .await
                .map_err(|e| format!("send: {e}"))?;
            self.record_outgoing(&t.key, &r.message_id, text);
            return Ok(format!("sent message id {} → {}", r.message_id, Self::format_chat(&t.key, &t.title)));
        }
        let root = files_dir();
        let mut n = 0usize;
        let mut ids: Vec<String> = Vec::new();
        for f in files {
            let path = resolve_file_path(&root, f)?;
            let data = tokio::fs::read(&path)
                .await
                .map_err(|e| format!("read {}: {e}", path.display()))?;
            let mime = mime_for(&path);
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("file")
                .to_string();
            let caption = if n == 0 && !text.is_empty() {
                Some(text.to_string())
            } else {
                None
            };
            let as_image = matches!(mime, "image/jpeg" | "image/png" | "image/webp");
            let as_voice = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "ogg" | "oga" | "opus"));
            let (msg, label) = if as_voice {
                // OGG/Opus goes out as a voice note (push-to-talk), not as a document.
                let (secs, wave) = match bia_speech::audio::decode_file(&path) {
                    Ok(x) if !x.is_empty() => (x.len() as f32 / 16_000.0, bia_speech::audio::whatsapp_waveform(&x)),
                    _ => (0.0, vec![]),
                };
                let up = client
                    .upload(data, MediaType::Audio, UploadOptions::default())
                    .await
                    .map_err(|e| format!("upload {}: {e}", path.display()))?;
                (
                    media::audio_message(
                        up,
                        AudioOptions {
                            mimetype: Some("audio/ogg; codecs=opus".into()),
                            duration_seconds: (secs > 0.0).then(|| secs.ceil() as u32),
                            ptt: Some(true),
                            waveform: (!wave.is_empty()).then_some(wave),
                            ..Default::default()
                        },
                    ),
                    "voice",
                )
            } else if as_image {
                let up = client
                    .upload(data, MediaType::Image, UploadOptions::default())
                    .await
                    .map_err(|e| format!("upload {}: {e}", path.display()))?;
                (
                    media::image_message(
                        up,
                        ImageOptions {
                            caption: caption.clone(),
                            mimetype: Some(mime.to_string()),
                            ..Default::default()
                        },
                    ),
                    "image",
                )
            } else {
                let up = client
                    .upload(data, MediaType::Document, UploadOptions::default())
                    .await
                    .map_err(|e| format!("upload {}: {e}", path.display()))?;
                (
                    media::document_message(
                        up,
                        DocumentOptions {
                            mimetype: Some(mime.to_string()),
                            file_name: Some(name.clone()),
                            caption: caption.clone(),
                            ..Default::default()
                        },
                    ),
                    "document",
                )
            };
            let r = client
                .send_message(t.jid.clone(), msg)
                .await
                .map_err(|e| format!("send_file: {e}"))?;
            let shown = match &caption {
                Some(c) => format!("[{label}] {c}"),
                None => format!("[{label}] {name}"),
            };
            self.record_outgoing(&t.key, &r.message_id, &shown);
            ids.push(r.message_id.clone());
            n += 1;
        }
        Ok(format!(
            "sent {n} file(s), message id {} → {}",
            ids.join(", "),
            Self::format_chat(&t.key, &t.title)
        ))
    }

    /// Speak `text` (in `lang`) and send it as a voice note (push-to-talk bubble).
    pub fn send_voice(&self, target: &str, text: &str, lang: &str) -> Result<String, String> {
        let client = self.require_ready()?;
        let t = self.resolve_target(target)?;
        self.note_chat(&t.key, &t.jid, None, now_secs());
        let lang = &bia_speech::contacts::lang_or(lang, &format!("whatsapp:{}", t.key));
        let out = bia_speech::paths::tmp().join(format!("voice-{}.ogg", bia_speech::paths::stamp()));
        let (secs, samples) = bia_speech::tts::voice_note(text, lang, &out)?;
        let data = std::fs::read(&out).map_err(|e| format!("read voice: {e}"));
        let _ = std::fs::remove_file(&out);
        let data = data?;
        let waveform = bia_speech::audio::whatsapp_waveform(&samples);
        self.block_on(async {
            let up = client
                .upload(data, MediaType::Audio, UploadOptions::default())
                .await
                .map_err(|e| format!("upload voice: {e}"))?;
            let msg = media::audio_message(
                up,
                AudioOptions {
                    mimetype: Some("audio/ogg; codecs=opus".into()),
                    duration_seconds: Some(secs.ceil() as u32),
                    ptt: Some(true),
                    waveform: Some(waveform),
                    ..Default::default()
                },
            );
            let r = client
                .send_message(t.jid.clone(), msg)
                .await
                .map_err(|e| format!("send voice: {e}"))?;
            self.record_outgoing(&t.key, &r.message_id, &format!("[voice] {text}"));
            Ok(format!(
                "sent voice message ({secs:.1}s), message id {} → {}",
                r.message_id,
                Self::format_chat(&t.key, &t.title)
            ))
        })
    }

    pub fn set_typing(&self, key: &str, on: bool) {
        let Ok(client) = self.client() else {
            return;
        };
        let Some(jid) = jid_of_key(&normalize_target(key)) else {
            return;
        };
        self.rt.spawn(async move {
            let cs = client.chatstate();
            let _ = if on {
                cs.send_composing(&jid).await
            } else {
                cs.send_paused(&jid).await
            };
        });
    }

    // -- profile ----------------------------------------------------------------

    pub fn set_profile(
        &self,
        first_name: Option<&str>,
        last_name: Option<&str>,
        bio: Option<&str>,
    ) -> Result<String, String> {
        if first_name.is_none() && last_name.is_none() && bio.is_none() {
            return Err("pass first_name, last_name, and/or bio".into());
        }
        let client = self.require_ready()?;
        self.block_on(async {
            let mut parts = Vec::new();
            if first_name.is_some() || last_name.is_some() {
                let current = client.push_name();
                let mut words = current.split_whitespace();
                let cur_first = words.next().unwrap_or("").to_string();
                let cur_last = words.collect::<Vec<_>>().join(" ");
                let f = first_name.unwrap_or(&cur_first).trim().to_string();
                let l = last_name.unwrap_or(&cur_last).trim().to_string();
                let name = [f, l]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ");
                if name.is_empty() {
                    return Err("display name cannot be empty".to_string());
                }
                client
                    .profile()
                    .set_push_name(&name)
                    .await
                    .map_err(|e| format!("set_profile name: {e}"))?;
                parts.push(format!("name={name:?}"));
            }
            if let Some(b) = bio {
                client
                    .profile()
                    .set_status_text(b)
                    .await
                    .map_err(|e| format!("set_profile bio: {e}"))?;
                parts.push(format!("bio={b:?}"));
            }
            Ok(format!("profile updated ({})", parts.join(", ")))
        })
    }

    pub fn set_photo(&self, path: &str) -> Result<String, String> {
        let client = self.require_ready()?;
        let abs = resolve_file_path(&files_dir(), path)?;
        let bytes = profile_jpeg(&abs)?;
        self.block_on(async {
            client
                .profile()
                .set_profile_picture(bytes)
                .await
                .map_err(|e| format!("set_photo: {e}"))?;
            Ok(format!("profile photo set from {}", abs.display()))
        })
    }

    // -- history / deletion --------------------------------------------------------

    pub fn history(
        &self,
        target: &str,
        limit: usize,
        before: &str,
    ) -> Result<(String, String, Vec<String>), String> {
        let t = self.resolve_target(target)?;
        let limit = limit.clamp(1, 100);
        let before = before.trim();
        let page = self.hist.lock().unwrap().page(
            &t.key,
            limit,
            if before.is_empty() { None } else { Some(before) },
        )?;
        let lines = page
            .into_iter()
            .map(|l| {
                let who = if l.me {
                    "me".to_string()
                } else if l.from.is_empty() {
                    t.key.clone()
                } else {
                    l.from.clone()
                };
                let body = if l.text.is_empty() {
                    "(no text)".to_string()
                } else {
                    l.text.replace('\n', " ")
                };
                format!("{}  {}  [{who}]  {body}", l.id, bia_mcp::share::rfc3339(l.ts))
            })
            .collect();
        Ok((t.key, t.title, lines))
    }

    /// React to a cached message. Empty `emoji` removes this account's reaction.
    /// Replace the text of a message this account sent (WhatsApp allows ~15 minutes).
    pub fn edit(&self, target: &str, message_id: &str, text: &str) -> Result<String, String> {
        let id = message_id.trim();
        if id.is_empty() {
            return Err("empty message id".into());
        }
        if text.trim().is_empty() {
            return Err("new text is empty".into());
        }
        let client = self.require_ready()?;
        let t = self.resolve_target(target)?;
        if let Some(l) = self.hist.lock().unwrap().get(&t.key, id) {
            if !l.me {
                return Err("you can only edit your own messages".into());
            }
        }
        let text = &bia_mcp::md::to_whatsapp(text);
        let content = wa::Message {
            conversation: Some(text.to_string()),
            ..Default::default()
        };
        self.block_on(async {
            client
                .edit_message(t.jid.clone(), id.to_string(), content)
                .await
                .map_err(|e| format!("edit: {e}"))
        })?;
        self.hist.lock().unwrap().set_text(&t.key, id, text);
        Ok(format!("edited message {id} → {}", Self::format_chat(&t.key, &t.title)))
    }

    pub fn react(
        &self,
        target: &str,
        message_id: &str,
        emoji: &str,
    ) -> Result<(String, String), String> {
        let id = message_id.trim();
        if id.is_empty() {
            return Err("empty message id".into());
        }
        let client = self.require_ready()?;
        let t = self.resolve_target(target)?;
        let line = self.hist.lock().unwrap().get(&t.key, id);
        let (from_me, participant) = match &line {
            Some(l) => (
                l.me,
                if t.jid.is_group() && !l.me && !l.from_jid.is_empty() {
                    Some(l.from_jid.clone())
                } else {
                    None
                },
            ),
            None => (false, None),
        };
        let target_key = wa::MessageKey {
            remote_jid: Some(t.jid.to_string()),
            from_me: Some(from_me),
            id: Some(id.to_string()),
            participant,
        };
        let emoji = emoji.trim();
        self.block_on(async {
            client
                .send_reaction(t.jid.clone(), target_key, emoji)
                .await
                .map_err(|e| format!("react: {e}"))?;
            Ok((t.key.clone(), t.title.clone()))
        })
    }

    pub fn delete_messages(
        &self,
        target: &str,
        ids: &[String],
        revoke: bool,
    ) -> Result<(usize, String, String), String> {
        let client = self.require_ready()?;
        let t = self.resolve_target(target)?;
        let mut done: Vec<String> = Vec::new();
        let mut errs: Vec<String> = Vec::new();
        self.block_on(async {
            for id in ids {
                match self.delete_one(&client, &t, id, revoke).await {
                    Ok(()) => done.push(id.clone()),
                    Err(e) => errs.push(format!("{id}: {e}")),
                }
            }
        });
        self.hist.lock().unwrap().delete_ids(&t.key, &done);
        if done.is_empty() && !errs.is_empty() {
            return Err(errs.join("; "));
        }
        if !errs.is_empty() {
            self.log(format!("delete_messages partial failure: {}", errs.join("; ")));
        }
        Ok((done.len(), t.key, t.title))
    }

    async fn delete_one(
        &self,
        client: &Arc<Client>,
        t: &Target,
        id: &str,
        revoke: bool,
    ) -> Result<(), String> {
        let line = self.hist.lock().unwrap().get(&t.key, id);
        if revoke {
            let kind = match &line {
                Some(l) if !l.me => {
                    if !t.jid.is_group() {
                        return Err(
                            "WhatsApp only lets you delete your own messages for everyone".into(),
                        );
                    }
                    let sender: Jid = l
                        .from_jid
                        .parse()
                        .map_err(|_| "sender unknown; cannot admin-revoke".to_string())?;
                    RevokeType::Admin {
                        original_sender: sender,
                    }
                }
                _ => RevokeType::Sender,
            };
            client
                .revoke_message(t.jid.clone(), id.to_string(), kind)
                .await
                .map_err(|e| format!("revoke: {e}"))?;
            return Ok(());
        }
        // Local delete: drop from the cache and best-effort sync "delete for me"
        // to the phone through app state.
        let (from_me, participant, ts) = match &line {
            Some(l) => (
                l.me,
                if t.jid.is_group() && !l.me {
                    l.from_jid.parse::<Jid>().ok()
                } else {
                    None
                },
                Some(l.ts),
            ),
            None => (false, None, None),
        };
        if let Err(e) = client
            .chat_actions()
            .delete_message_for_me(&t.jid, participant.as_ref(), id, from_me, false, ts)
            .await
        {
            self.log(format!("delete_for_me sync failed for {id}: {e}"));
        }
        Ok(())
    }

    pub fn delete_history(
        &self,
        target: &str,
        revoke: bool,
        remove: bool,
    ) -> Result<(String, String), String> {
        let client = self.require_ready()?;
        let t = self.resolve_target(target)?;
        let own = if revoke {
            self.hist.lock().unwrap().own_ids(&t.key)
        } else {
            Vec::new()
        };
        self.block_on(async {
            // WhatsApp cannot wipe the other side's copy; the closest thing is
            // revoking our own recent messages (older ones are refused by the server).
            for id in own {
                let _ = client
                    .revoke_message(t.jid.clone(), id, RevokeType::Sender)
                    .await;
            }
            if remove {
                client
                    .chat_actions()
                    .delete_chat(&t.jid, true, None)
                    .await
                    .map_err(|e| format!("delete_history: {e}"))
            } else {
                client
                    .chat_actions()
                    .clear_chat(&t.jid, false, true, None)
                    .await
                    .map_err(|e| format!("delete_history: {e}"))
            }
        })?;
        self.hist.lock().unwrap().clear(&t.key);
        Ok((t.key, t.title))
    }

    // -- inbound messages -----------------------------------------------------------

    async fn on_message(&self, ctx: MessageContext) {
        // Its place among this chat's messages to the model, taken before anything slow
        // (messages are handled concurrently; the lane keeps them in order).
        let turn = lanes().reserve(&ctx.info.source.chat.to_non_ad().to_string());
        if let Err(e) = self.handle_inbound(&ctx, turn).await {
            self.log(format!("inbound error: {e}"));
        }
    }

    /// Revokes (and other protocol messages). Notify the model on REVOKE from others.
    async fn handle_inbound_protocol(
        &self,
        ctx: &MessageContext,
        pm: &wa::message::ProtocolMessage,
    ) -> Result<(), String> {
        use wa::message::protocol_message::Type as ProtoType;
        if pm.r#type == Some(ProtoType::MESSAGE_EDIT) {
            return self.handle_inbound_edit(ctx, pm).await;
        }
        if pm.r#type != Some(ProtoType::REVOKE) {
            return Ok(());
        }
        let info = &ctx.info;
        // Our own revoke (tool / phone) — tool result already covers it.
        if info.source.is_from_me {
            return Ok(());
        }
        let client = ctx.client.clone();
        let chat = info.source.chat.to_non_ad();
        if chat.is_status_broadcast() || chat.is_newsletter() || chat.is_broadcast_list() {
            return Ok(());
        }
        let key = key_of_jid_async(&client, &chat).await;
        let sender = info.source.sender.to_non_ad();
        let sender_key = key_of_jid_async(&client, &sender).await;
        if self.is_banned(&key) || (key != sender_key && self.is_banned(&sender_key)) {
            return Ok(());
        }
        let deleted_id = pm
            .key
            .as_option()
            .and_then(|k| k.id.clone())
            .unwrap_or_else(|| "?".into());
        let ts = info.timestamp.timestamp();
        let push = info.push_name.trim().to_string();
        let title = if chat.is_group() {
            self.group_title(&client, &key, &chat).await
        } else if !push.is_empty() {
            Some(push)
        } else {
            None
        };
        self.note_chat(&key, &chat, title.as_deref(), ts);
        self.hist.lock().unwrap().delete_ids(&key, &[deleted_id.clone()]);
        self.inbound_at("system", &key, &deleted_id, &format!("[deleted] {deleted_id}"), ts);
        let read_sender = if chat.is_group() {
            Some(&info.source.sender)
        } else {
            None
        };
        if let Err(e) = client
            .mark_as_read(&info.source.chat, read_sender, &[info.id.as_str()])
            .await
        {
            self.log(format!("mark_read delete chat={key}: {e}"));
        }
        Ok(())
    }

    /// A message was edited. Ours (phone / other device): update the cache only.
    /// Someone else's: the cached line takes the new text, the history gets an edit line,
    /// and the model hears when it was sent and edited, the old text and the new one.
    async fn handle_inbound_edit(&self, ctx: &MessageContext, pm: &wa::message::ProtocolMessage) -> Result<(), String> {
        let info = &ctx.info;
        let client = ctx.client.clone();
        let chat = info.source.chat.to_non_ad();
        if chat.is_status_broadcast() || chat.is_newsletter() || chat.is_broadcast_list() {
            return Ok(());
        }
        let Some(id) = pm.key.as_option().and_then(|k| k.id.clone()) else { return Ok(()) };
        let Some(edited_msg) = pm.edited_message.as_option() else { return Ok(()) };
        let (new, _) = describe(edited_msg.get_base_message());
        let key = key_of_jid_async(&client, &chat).await;
        let old = self.hist.lock().unwrap().get(&key, &id);
        if info.source.is_from_me {
            self.hist.lock().unwrap().set_text(&key, &id, &new);
            return Ok(());
        }
        let sender = info.source.sender.to_non_ad();
        let sender_key = key_of_jid_async(&client, &sender).await;
        if self.is_banned(&key) || (key != sender_key && self.is_banned(&sender_key)) {
            return Ok(());
        }
        if old.as_ref().is_some_and(|l| l.text == new) {
            return Ok(());
        }
        let edited = pm.timestamp_ms.map(|ms| ms / 1000).unwrap_or_else(|| info.timestamp.timestamp());
        let text = bia_mcp::share::edit_text(&id, old.as_ref().map(|l| l.ts), edited, old.as_ref().map(|l| l.text.as_str()), &new);
        let label = old
            .as_ref()
            .map(|l| l.from.clone())
            .filter(|f| !f.is_empty())
            .unwrap_or_else(|| {
                let push = info.push_name.trim();
                if push.is_empty() { format!("+{sender_key}") } else { format!("{push} ({sender_key})") }
            });
        {
            let mut h = self.hist.lock().unwrap();
            h.set_text(&key, &id, &new);
            h.append(
                &key,
                HistLine {
                    id: info.id.clone(),
                    ts: edited,
                    me: false,
                    from: label.clone(),
                    from_jid: sender.to_string(),
                    text: text.clone(),
                },
            );
        }
        self.inbound_at(&label, &key, &info.id, &text, edited);
        let read_sender = if chat.is_group() { Some(&info.source.sender) } else { None };
        let _ = client.mark_as_read(&info.source.chat, read_sender, &[info.id.as_str()]).await;
        Ok(())
    }

    async fn handle_inbound_reaction(
        &self,
        ctx: &MessageContext,
        rm: &wa::message::ReactionMessage,
    ) -> Result<(), String> {
        let info = &ctx.info;
        let client = ctx.client.clone();
        let chat = info.source.chat.to_non_ad();
        if chat.is_status_broadcast() || chat.is_newsletter() || chat.is_broadcast_list() {
            return Ok(());
        }
        let from_me = info.source.is_from_me;
        if from_me {
            return Ok(());
        }
        let key = key_of_jid_async(&client, &chat).await;
        let sender = info.source.sender.to_non_ad();
        let sender_key = key_of_jid_async(&client, &sender).await;
        let push = info.push_name.trim().to_string();
        let label = {
            let known = self.book.lock().unwrap().title(&sender_key).unwrap_or_default();
            let name = if !push.is_empty() {
                push.clone()
            } else {
                known
            };
            if name.is_empty() || name == sender_key || name == format!("+{sender_key}") {
                format!("+{sender_key}")
            } else {
                format!("{name} ({sender_key})")
            }
        };
        let ts = info.timestamp.timestamp();
        let title = if chat.is_group() {
            self.group_title(&client, &key, &chat).await
        } else if !push.is_empty() {
            Some(push)
        } else {
            None
        };
        self.note_chat(&key, &chat, title.as_deref(), ts);

        if self.is_banned(&key) || (key != sender_key && self.is_banned(&sender_key)) {
            return Ok(());
        }

        let target_id = rm
            .key
            .as_option()
            .and_then(|k| k.id.clone())
            .unwrap_or_else(|| "?".into());
        let emoji = rm.text.as_deref().unwrap_or("").trim();
        let body = if emoji.is_empty() {
            "[reaction] removed".to_string()
        } else {
            format!("[reaction] {emoji}")
        };
        // Envelope id is the reacted-to message (what react/delete tools need).
        self.inbound_at(&label, &key, &target_id, &body, ts);

        let read_sender = if chat.is_group() {
            Some(&info.source.sender)
        } else {
            None
        };
        if let Err(e) = client
            .mark_as_read(&info.source.chat, read_sender, &[info.id.as_str()])
            .await
        {
            self.log(format!("mark_read reaction chat={key}: {e}"));
        }
        Ok(())
    }

    async fn handle_inbound(&self, ctx: &MessageContext, mut turn: bia_mcp::lanes::Turn) -> Result<(), String> {
        let info = &ctx.info;
        let client = ctx.client.clone();
        let chat = info.source.chat.to_non_ad();
        if chat.is_status_broadcast() || chat.is_newsletter() || chat.is_broadcast_list() {
            return Ok(());
        }
        let base = ctx.message.get_base_message();
        if let Some(pm) = base.protocol_message.as_option() {
            return self.handle_inbound_protocol(ctx, pm).await;
        }
        if let Some(rm) = base.reaction_message.as_option() {
            return self.handle_inbound_reaction(ctx, rm).await;
        }
        let (text, kind) = describe(base);
        if text.is_empty() && kind.is_none() {
            return Ok(());
        }

        let key = key_of_jid_async(&client, &chat).await;
        let from_me = info.source.is_from_me;
        let sender = info.source.sender.to_non_ad();
        let push = info.push_name.trim().to_string();
        let sender_key = if from_me {
            "me".to_string()
        } else {
            key_of_jid_async(&client, &sender).await
        };
        // Stable identity: push name, else known contact title, else +phone.
        // Never use "whatsapp:<id>" as the identity (that duplicates the channel).
        let label = if from_me {
            "me".to_string()
        } else {
            let known = self.book.lock().unwrap().title(&sender_key).unwrap_or_default();
            let name = if !push.is_empty() {
                push.clone()
            } else {
                known
            };
            if name.is_empty() || name == sender_key || name == format!("+{sender_key}") {
                format!("+{sender_key}")
            } else {
                format!("{name} ({sender_key})")
            }
        };

        let ts = info.timestamp.timestamp();
        let title = if chat.is_group() {
            self.group_title(&client, &key, &chat).await
        } else if !from_me && !push.is_empty() {
            Some(push.clone())
        } else {
            None
        };
        self.note_chat(&key, &chat, title.as_deref(), ts);
        // Live location: WhatsApp sends one message per move. The first point of a share
        // reaches the model; later points only update the tracker (no history line either).
        let mut text = text;
        if !from_me {
            if let Some((lat, lon, acc)) = live_point(base) {
                ensure_live_sweeper();
                let lk = bia_mcp::share::live_key(&key, &sender_key, chat.is_group());
                if live().seen(&lk, &label, lat, lon, acc, None, false, now_secs().max(0) as u64)
                    != bia_mcp::share::LiveEvent::Started
                {
                    return Ok(());
                }
                text.push_str("\n(the position updates by itself; whatsapp_live_location gives where they are now)");
            }
        }
        // Cache + dedupe: WhatsApp sometimes redelivers the same id (e.g. once
        // without push name, then again with it). Only the first delivery reaches the model.
        let is_new = self.hist.lock().unwrap().append(
            &key,
            HistLine {
                id: info.id.clone(),
                ts,
                me: from_me,
                from: if from_me { "me".into() } else { label.clone() },
                from_jid: sender.to_string(),
                text: hist_text(&text, kind),
            },
        );
        // Sent from the phone or another linked device: cache only.
        if from_me {
            return Ok(());
        }
        if !is_new {
            return Ok(());
        }

        let read_sender = if chat.is_group() {
            Some(&info.source.sender)
        } else {
            None
        };

        if self.is_banned(&key) || (key != sender_key && self.is_banned(&sender_key)) {
            let reply = {
                let mut m = self.ban_replies.lock().unwrap();
                let due = m.get(&key).is_none_or(|t| t.elapsed() >= BAN_REPLY_GAP);
                if due {
                    m.insert(key.clone(), Instant::now());
                }
                due
            };
            if reply {
                let _ = client.send_text(info.source.chat.clone(), "I've banned you").await;
            }
            // Clear the unread badge so banned chats don't pile up.
            let _ = client
                .mark_as_read(&info.source.chat, read_sender, &[info.id.as_str()])
                .await;
            return Ok(());
        }

        // Read as soon as it arrives: downloading media and transcribing a voice note
        // take a while, and the sender should not wait for them to see it was received.
        if let Err(e) = client
            .mark_as_read(&info.source.chat, read_sender, &[info.id.as_str()])
            .await
        {
            self.log(format!("mark_read chat={key}: {e}"));
        }
        let mut deliver = text.clone();
        if let Some(r) = self.reply_note(&client, &key, base).await {
            deliver = if deliver.is_empty() { r } else { format!("{r}\n{deliver}") };
        }
        let mut voice = None;
        // During an active voice call, ignore chat media (live STT covers speech).
        let in_call = Calls::global().active_key().as_deref() == Some(key.as_str());
        if let Some(k) = kind.filter(|_| !in_call) {
            let note = match self.save_media(&client, &key, base, &info.id).await {
                Ok((rel, abs)) => {
                    if matches!(k, "voice" | "audio") {
                        voice = Some((rel.clone(), abs));
                    }
                    format!("file: {rel}")
                }
                Err(e) => {
                    self.log(format!("media {key}: {e}"));
                    let what = match k {
                        "voice" => "voice message",
                        "audio" => "audio file",
                        "image" => "photo",
                        "document" => "file",
                        k => k,
                    };
                    format!("[{what} not received: {e}; ask them to send it again if you need it]")
                }
            };
            deliver = if deliver.is_empty() { note } else { format!("{deliver}\n{note}") };
        }
        if deliver.is_empty() {
            return Ok(());
        }
        // Downloaded alongside the chat's other messages; transcribed and delivered in
        // turn, so what this chat sent next reaches the model after the transcript.
        turn.wait().await;
        if let Some((rel, abs)) = voice {
            let lang = bia_speech::contacts::lang_for(&format!("whatsapp:{key}"));
            let heard = tokio::task::spawn_blocking(move || bia_speech::transcribe_note_from(&abs, lang.as_deref()))
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r);
            deliver = with_transcript(heard, &deliver, &format!("whatsapp/{rel}"));
        }
        self.inbound_at(&label, &key, &info.id, &deliver, ts);
        Ok(())
    }

    /// `[reply to <id> from <who>: "…"]` for a message that quotes another one: author and
    /// text from the chat cache ("you" for bia's own), else from the quoted copy it carries.
    async fn reply_note(&self, client: &Client, key: &str, base: &wa::Message) -> Option<String> {
        let ctx = context_info(base)?;
        let id = ctx.stanza_id.as_deref().filter(|s| !s.is_empty())?;
        let cached = self.hist.lock().unwrap().get(key, id);
        let (from, text) = match cached {
            Some(l) => (if l.me { "you".to_string() } else { l.from }, l.text),
            None => {
                let from = match ctx.participant.as_deref().and_then(|p| p.parse::<Jid>().ok()) {
                    Some(j) => format!("+{}", key_of_jid_async(client, &j).await),
                    None => String::new(),
                };
                let text = ctx.quoted_message.as_option().map(|q| {
                    let (t, kind) = describe(q);
                    hist_text(&t, kind)
                });
                (from, text.unwrap_or_default())
            }
        };
        Some(bia_mcp::share::reply_text(id, &from, &text, ""))
    }

    async fn save_media(
        &self,
        client: &Client,
        key: &str,
        base: &wa::Message,
        msg_id: &str,
    ) -> Result<(String, PathBuf), String> {
        let dl = |e: anyhow::Error| format!("download: {e}");
        let (data, ext) = if let Some(m) = base.image_message.as_option() {
            (
                client.download(m).await.map_err(dl)?,
                ext_for(m.mimetype.as_deref(), "jpg"),
            )
        } else if let Some(m) = base.video_message.as_option() {
            (
                client.download(m).await.map_err(dl)?,
                ext_for(m.mimetype.as_deref(), "mp4"),
            )
        } else if let Some(m) = base.audio_message.as_option() {
            (
                client.download(m).await.map_err(dl)?,
                ext_for(m.mimetype.as_deref(), "ogg"),
            )
        } else if let Some(m) = base.document_message.as_option() {
            let ext = m
                .file_name
                .as_deref()
                .and_then(|n| Path::new(n).extension())
                .and_then(|e| e.to_str())
                .map(|e| safe_name(&e.to_ascii_lowercase()))
                .filter(|e| !e.is_empty())
                .unwrap_or_else(|| ext_for(m.mimetype.as_deref(), "bin"));
            (client.download(m).await.map_err(dl)?, ext)
        } else if let Some(m) = base.sticker_message.as_option() {
            (
                client.download(m).await.map_err(dl)?,
                ext_for(m.mimetype.as_deref(), "webp"),
            )
        } else {
            return Err("no downloadable media".into());
        };

        let dir = files_dir().join("inbox").join("whatsapp").join(safe_name(key));
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| e.to_string())?;
        let short: String = msg_id
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .take(12)
            .collect();
        let name = format!("{}-{short}.{ext}", chrono_stamp());
        let dest = dir.join(&name);
        tokio::fs::write(&dest, &data)
            .await
            .map_err(|e| format!("write {}: {e}", dest.display()))?;
        Ok((format!("inbox/whatsapp/{}/{name}", safe_name(key)), dest))
    }
}

// ---------------------------------------------------------------------------
// Bans persistence
// ---------------------------------------------------------------------------

fn bans_path() -> PathBuf {
    files_dir().join("state").join("whatsapp-bans.json")
}

fn read_bans() -> HashSet<String> {
    let Ok(b) = std::fs::read(bans_path()) else {
        return HashSet::new();
    };
    let v: serde_json::Value = serde_json::from_slice(&b).unwrap_or(serde_json::json!({}));
    v.get("chats")
        .and_then(|c| c.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default()
}

fn write_bans(ids: &[String]) -> Result<(), String> {
    if let Some(p) = bans_path().parent() {
        std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
    }
    let v = serde_json::json!({ "chats": ids });
    std::fs::write(bans_path(), serde_json::to_vec_pretty(&v).unwrap()).map_err(|e| e.to_string())?;
    unsafe { libc::sync() };
    Ok(())
}

/// Inbound messages to the model, in order per chat (see [`bia_mcp::lanes`]).
fn lanes() -> &'static bia_mcp::lanes::Lanes {
    static L: std::sync::LazyLock<bia_mcp::lanes::Lanes> = std::sync::LazyLock::new(Default::default);
    &L
}

/// Inbound messages that arrived before the agent link (`set_ctx`) existed; delivered
/// when it is set instead of being lost (at most 500 kept).
type Held = (String, String, String, String, i64);

fn held_inbound() -> &'static Mutex<Vec<Held>> {
    static H: OnceLock<Mutex<Vec<Held>>> = OnceLock::new();
    H.get_or_init(|| Mutex::new(Vec::new()))
}

fn hold_inbound(identity: &str, channel: &str, id: &str, text: &str, issued: i64) {
    // held ones reach the agent later: keep when they came even without WhatsApp's timestamp
    let issued = if issued > 0 { issued } else { now_secs() };
    if let Ok(mut h) = held_inbound().lock() {
        if h.len() < 500 {
            h.push((identity.to_string(), channel.to_string(), id.to_string(), text.to_string(), issued));
        }
    }
}

#[cfg(test)]
mod shared_tests {
    use super::*;

    #[test]
    fn locations_and_contacts_become_text() {
        let loc = wa::Message {
            location_message: buffa::MessageField::some(wa::message::LocationMessage {
                degrees_latitude: Some(52.520008),
                degrees_longitude: Some(13.404954),
                name: Some("Grand Hotel".into()),
                address: Some("Main St 1".into()),
                comment: Some("see you here".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let (text, kind) = describe(&loc);
        assert_eq!(kind, None);
        assert_eq!(
            text,
            "[location] Grand Hotel, Main St 1 — 52.520008, 13.404954 https://maps.google.com/?q=52.520008,13.404954\nsee you here"
        );
        let live = wa::Message {
            live_location_message: buffa::MessageField::some(wa::message::LiveLocationMessage {
                degrees_latitude: Some(1.5),
                degrees_longitude: Some(-2.5),
                accuracy_in_meters: Some(30),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(describe(&live).0.starts_with("[live location] 1.500000, -2.500000 (±30 m) (sharing live)"));
        let vcard = bia_mcp::share::build_vcard("Jane Doe", "+49 170 1234567", "", true);
        let contact = wa::Message {
            contact_message: buffa::MessageField::some(wa::message::ContactMessage {
                display_name: Some("Jane".into()),
                vcard: Some(vcard.clone()),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(describe(&contact).0, "[contact] Jane, +49 170 1234567");
        let many = wa::Message {
            contacts_array_message: buffa::MessageField::some(wa::message::ContactsArrayMessage {
                display_name: Some("2 contacts".into()),
                contacts: vec![
                    wa::message::ContactMessage { display_name: Some("A".into()), vcard: Some(vcard.clone()), ..Default::default() },
                    wa::message::ContactMessage { display_name: Some("B".into()), vcard: Some(String::new()), ..Default::default() },
                ],
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(describe(&many).0, "[contact] A, +49 170 1234567\n[contact] B");
        let plain = wa::Message { conversation: Some("hi".into()), ..Default::default() };
        assert_eq!(describe(&plain).0, "hi");
    }
}
