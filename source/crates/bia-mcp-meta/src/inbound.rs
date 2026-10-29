//! What the account hears, for the model: each incoming message becomes an envelope on
//! `<platform>:<chat>`, in arrival order per chat. Files are downloaded into
//! files/inbox/<platform>/<chat>/ and voice notes transcribed before their turn comes.
//! Messages that came while bia was off arrive in the catch-up (backlog) and are delivered
//! when they are newer than the last one delivered.

use crate::helper::{self, LONG, QUICK};
use crate::state;
use crate::Platform;
use bia_mcp::Ctx;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

static PLATFORM: OnceLock<Platform> = OnceLock::new();

pub fn set_platform(p: Platform) {
    let _ = PLATFORM.set(p);
}

pub fn platform() -> Platform {
    *PLATFORM.get().unwrap_or(&Platform::Messenger)
}

// --- the agent link ---

fn ctx_slot() -> &'static Mutex<Option<Ctx>> {
    static C: OnceLock<Mutex<Option<Ctx>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(None))
}

type Held = (String, String, String, String, i64);

fn held() -> &'static Mutex<Vec<Held>> {
    static H: OnceLock<Mutex<Vec<Held>>> = OnceLock::new();
    H.get_or_init(|| Mutex::new(Vec::new()))
}

/// Remember the agent link; messages that came before it are delivered now.
pub fn set_ctx(ctx: Ctx) {
    *ctx_slot().lock().unwrap() = Some(ctx.clone());
    let waiting: Vec<Held> = std::mem::take(&mut *held().lock().unwrap());
    for (identity, channel, id, text, issued) in waiting {
        ctx.inbound_at(&identity, &channel, &id, &text, issued);
    }
}

fn ctx() -> Option<Ctx> {
    ctx_slot().lock().unwrap().clone()
}

pub fn log(msg: &str) {
    match ctx() {
        Some(c) => c.log(msg),
        None => eprintln!("{msg}"),
    }
}

/// A message for the model on `<platform>:<chat>`.
/// Returns what the chat's ignore rules made of it (an ignored one stays unread).
pub fn inbound(identity: &str, chat: &str, id: &str, text: &str, issued: i64) -> bia_mcp::ignore::Verdict {
    let channel = platform().channel(chat);
    match ctx() {
        Some(c) => c.inbound_at(identity, &channel, id, text, issued),
        None => {
            let issued = if issued > 0 { issued } else { state::now_secs() };
            let verdict = if identity == "system" { bia_mcp::ignore::Verdict::Deliver } else { bia_mcp::ignore::check(&channel, text) };
            let mut h = held().lock().unwrap();
            if h.len() < 500 {
                h.push((identity.into(), channel, id.into(), text.into(), issued));
            }
            verdict
        }
    }
}

/// A system notice on the console (login problems, logouts).
pub fn console(text: &str) {
    match ctx() {
        Some(c) => c.inbound("system", "console", "", text),
        None => eprintln!("{text}"),
    }
}

// --- titles and labels ---

fn titles() -> &'static Mutex<HashMap<String, String>> {
    static T: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    T.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn note_title(chat: &str, title: &str) {
    if !title.trim().is_empty() {
        titles().lock().unwrap().insert(chat.to_string(), title.trim().to_string());
    }
}

pub fn title_of(chat: &str) -> String {
    titles().lock().unwrap().get(chat).cloned().unwrap_or_default()
}

/// `messenger:<chat>  Title`.
pub fn format_chat(chat: &str, title: &str) -> String {
    let p = platform();
    let title = if title.trim().is_empty() { title_of(chat) } else { title.trim().to_string() };
    if title.is_empty() || title == chat {
        p.channel(chat)
    } else {
        format!("{}  {title}", p.channel(chat))
    }
}

/// The envelope identity of a sender: `Name (id)`, never the channel.
pub fn label(sender: &str, name: &str) -> String {
    let name = name.trim();
    if name.is_empty() || name == sender {
        sender.to_string()
    } else {
        format!("{name} ({sender})")
    }
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

fn b(v: &Value, k: &str) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(false)
}

fn i(v: &Value, k: &str) -> i64 {
    v.get(k).and_then(Value::as_i64).unwrap_or(0)
}

// --- bans ---

pub fn is_banned(chat: &str, sender: &str) -> bool {
    let bans = state::bans(platform());
    bans.contains(chat) || (!sender.is_empty() && bans.contains(sender))
}

/// Don't answer a banned chat more often than this (avoids bot ping-pong).
const BAN_REPLY_GAP: Duration = Duration::from_secs(30);

fn ban_reply(chat: &str, id: &str) {
    static LAST: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
    let due = {
        let mut m = LAST.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap();
        let due = m.get(chat).is_none_or(|t| t.elapsed() >= BAN_REPLY_GAP);
        if due {
            m.insert(chat.to_string(), Instant::now());
        }
        due
    };
    let (chat, id) = (chat.to_string(), id.to_string());
    std::thread::spawn(move || {
        mark_read(&chat, &id);
        if due {
            let h = helper::get();
            let _ = h.call("send", json!({"chat": chat, "text": "I've banned you"}), helper::NORMAL);
        }
    });
}

// --- delivery ---

/// Read receipt for one message. A failed one goes to the log; bia-meta sends it once more
/// after the next reconnect.
fn mark_read(chat: &str, id: &str) {
    if let Err(e) = helper::get().call("read", json!({"chat": chat, "id": id}), QUICK) {
        log(&format!("{} read {chat} {id}: {e}", platform().name()));
    }
}

/// Inbound messages to the model, in order per chat.
fn lanes() -> &'static bia_mcp::lanes::Lanes {
    static L: std::sync::LazyLock<bia_mcp::lanes::Lanes> = std::sync::LazyLock::new(Default::default);
    &L
}

/// Ids delivered lately (Meta sometimes repeats a message, e.g. live and in the catch-up).
fn seen(id: &str) -> bool {
    static S: OnceLock<Mutex<(HashSet<String>, VecDeque<String>)>> = OnceLock::new();
    let mut g = S.get_or_init(|| Mutex::new((HashSet::new(), VecDeque::new()))).lock().unwrap();
    if !g.0.insert(id.to_string()) {
        return true;
    }
    g.1.push_back(id.to_string());
    if g.1.len() > 2000 {
        if let Some(old) = g.1.pop_front() {
            g.0.remove(&old);
        }
    }
    false
}

fn bump_last_seen(ts: i64) {
    let p = platform();
    if ts > state::load_cfg(p).last_seen {
        state::update_cfg(p, |c| c.last_seen = c.last_seen.max(ts));
    }
}

pub fn on_event(ev: Value) {
    let p = platform();
    match s(&ev, "event") {
        "message" => deliver(&ev, true),
        "backlog" => backlog(&ev),
        "edit" => edited(&ev),
        "delete" => deleted(&ev),
        "reaction" => reacted(&ev),
        "chat" => note_title(s(&ev, "chat"), s(&ev, "title")),
        "logged_out" => {
            state::update_cfg(p, |c| c.logged_in = false);
            console(&format!(
                "[{} logged out: {}] Sign in to {} again in Chrome (chrome MCP), then call {}_login.",
                p.name(),
                s(&ev, "reason"),
                match p {
                    Platform::Messenger => "https://www.facebook.com",
                    Platform::Instagram => "https://www.instagram.com",
                },
                p.name()
            ));
        }
        "status" => {
            let st = s(&ev, "state");
            let err = s(&ev, "error");
            if st == "error" && !err.is_empty() {
                log(&format!("{}: {err}", p.name()));
            }
        }
        _ => {}
    }
}

fn backlog(ev: &Value) {
    let p = platform();
    let cfg = state::load_cfg(p);
    if cfg.last_seen == 0 {
        // the first connect of a fresh login: everything in it is old
        bump_last_seen(state::now_secs());
        return;
    }
    let mut msgs: Vec<Value> = ev.get("messages").and_then(Value::as_array).cloned().unwrap_or_default();
    msgs.sort_by_key(|m| i(m, "ts"));
    for m in &msgs {
        if i(m, "ts") > cfg.last_seen {
            deliver(m, false);
        }
    }
}

fn deliver(ev: &Value, live: bool) {
    let (chat, id) = (s(ev, "chat").to_string(), s(ev, "id").to_string());
    if chat.is_empty() || id.is_empty() || b(ev, "me") || b(ev, "admin") {
        return;
    }
    if !live && i(ev, "ts") <= state::load_cfg(platform()).last_seen {
        return;
    }
    if seen(&id) {
        return;
    }
    let sender = s(ev, "sender").to_string();
    note_title(&chat, s(ev, "chat_title"));
    if !b(ev, "group") {
        note_title(&chat, s(ev, "sender_name"));
    }
    let who = label(&sender, s(ev, "sender_name"));
    let ts = i(ev, "ts");
    if is_banned(&chat, &sender) {
        ban_reply(&chat, &id);
        return;
    }
    let turn = lanes().reserve(&chat);
    let ev = ev.clone();
    let _ = std::thread::Builder::new().name("meta-in".into()).spawn(move || {
        let mut turn = turn;
        // read as soon as it arrives: downloads and transcripts take a while; an ignored
        // chat stays unread unless the finished message wakes the model
        let ignoring = bia_mcp::ignore::ignoring(&platform().channel(&chat));
        if !ignoring {
            mark_read(&chat, &id);
        }
        let mut parts: Vec<String> = Vec::new();
        if let Some(r) = ev.get("reply").filter(|r| r.is_object()) {
            let from = if b(r, "me") { "you".to_string() } else { label(s(r, "sender"), s(r, "sender_name")) };
            parts.push(bia_mcp::share::reply_text(s(r, "id"), &from, s(r, "text"), ""));
        }
        let text = s(&ev, "text").trim().to_string();
        if !text.is_empty() {
            parts.push(text);
        }
        let mut voice: Option<PathBuf> = None;
        for a in ev.get("attachments").and_then(Value::as_array).cloned().unwrap_or_default() {
            match save_attachment(&chat, &id, &a) {
                Ok((rel, abs, kind)) => {
                    if matches!(kind.as_str(), "voice" | "audio") && voice.is_none() {
                        voice = Some(abs);
                    }
                    let what = match kind.as_str() {
                        "image" | "gif" | "sticker" | "video" | "document" => String::new(),
                        k => format!("[{k}] "),
                    };
                    parts.push(format!("{what}file: {rel}"));
                }
                Err(e) => parts.push(e),
            }
        }
        if parts.is_empty() {
            return;
        }
        let mut body = parts.join("\n");
        turn.wait_blocking();
        if let Some(abs) = voice {
            let lang = bia_speech::contacts::lang_for(&platform().channel(&chat));
            let heard = bia_speech::transcribe_note_from(&abs, lang.as_deref());
            body = with_transcript(heard, &body);
        }
        if inbound(&who, &chat, &id, &body, ts).read() && ignoring {
            mark_read(&chat, &id);
        }
        bump_last_seen(ts);
    });
}

/// `[voice message, 4.2s, whisper-base 1.3s] [speech ru] …` before the message's own lines.
fn with_transcript(heard: Result<bia_speech::Heard, String>, text: &str) -> String {
    match heard {
        Ok(h) => {
            let mut out = format!("{}\n{text}", h.voice_head());
            if let Some(n) = h.notice {
                out.push('\n');
                out.push_str(&n);
            }
            out
        }
        Err(e) => format!("[voice message, transcription failed: {e}]\n{text}"),
    }
}

fn ext_for(mime: &str, name: &str, kind: &str) -> String {
    if let Some(e) = Path::new(name).extension().and_then(|e| e.to_str()) {
        let e = state::safe_name(&e.to_ascii_lowercase());
        if !e.is_empty() && e.len() <= 8 {
            return e;
        }
    }
    let m = mime.split(';').next().unwrap_or("").trim();
    match m {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "image/heic" => "heic",
        "video/mp4" => "mp4",
        "video/quicktime" => "mov",
        "audio/ogg" => "ogg",
        "audio/mpeg" => "mp3",
        "audio/mp4" | "audio/x-m4a" | "audio/aac" => "m4a",
        "audio/wav" | "audio/x-wav" => "wav",
        "application/pdf" => "pdf",
        "text/plain" => "txt",
        _ => match kind {
            "image" | "sticker" => "jpg",
            "video" | "gif" => "mp4",
            "voice" | "audio" => "m4a",
            _ => "bin",
        },
    }
    .to_string()
}

/// Download one attachment; Ok((path under files, absolute path, kind)) or the note for the model.
fn save_attachment(chat: &str, id: &str, a: &Value) -> Result<(String, PathBuf, String), String> {
    let kind = s(a, "kind").to_string();
    let what = match kind.as_str() {
        "voice" => "voice message",
        "audio" => "audio file",
        "image" => "photo",
        "document" => "file",
        k => k,
    }
    .to_string();
    let note = s(a, "note");
    if !note.is_empty() {
        return Err(format!("[{what} not received: {note}]"));
    }
    let r = s(a, "ref");
    if r.is_empty() {
        return Err(format!("[{what} not received]"));
    }
    let p = platform();
    let dir = state::inbox_dir(p).join(state::safe_name(chat));
    let short: String = id.chars().filter(|c| c.is_ascii_alphanumeric()).rev().take(12).collect::<String>().chars().rev().collect();
    let name = format!("{}-{short}.{}", state::stamp(), ext_for(s(a, "mime"), s(a, "name"), &kind));
    let dest = dir.join(&name);
    match helper::get().call("download", json!({"ref": r, "path": dest}), LONG) {
        Ok(res) => {
            // the real type is known only now: fix the extension if it was guessed
            let mime = s(&res, "mime");
            let mut dest = dest;
            let mut name = name;
            if !mime.is_empty() && s(a, "name").is_empty() {
                let want = ext_for(mime, "", &kind);
                if dest.extension().and_then(|e| e.to_str()) != Some(want.as_str()) {
                    let fixed = dest.with_extension(&want);
                    if std::fs::rename(&dest, &fixed).is_ok() {
                        name = fixed.file_name().and_then(|n| n.to_str()).unwrap_or(&name).to_string();
                        dest = fixed;
                    }
                }
            }
            Ok((format!("inbox/{}/{}/{name}", p.name(), state::safe_name(chat)), dest, kind))
        }
        Err(e) => {
            log(&format!("{} media {chat}: {e}", p.name()));
            Err(format!("[{what} not received: {e}; ask them to send it again if you need it]"))
        }
    }
}

fn edited(ev: &Value) {
    let (chat, id) = (s(ev, "chat"), s(ev, "id"));
    let sender = s(ev, "sender");
    if b(ev, "me") || chat.is_empty() || id.is_empty() || is_banned(chat, sender) {
        return;
    }
    let new = s(ev, "text");
    let old = s(ev, "old");
    if !old.is_empty() && old == new {
        return;
    }
    let sent = i(ev, "sent_ts");
    let edited = i(ev, "ts").max(1);
    let text = bia_mcp::share::edit_text(id, (sent > 0).then_some(sent), edited, (!old.is_empty()).then_some(old), new);
    let who = if sender.is_empty() { "someone".to_string() } else { label(sender, s(ev, "sender_name")) };
    inbound(&who, chat, id, &text, edited);
}

fn deleted(ev: &Value) {
    let (chat, id) = (s(ev, "chat"), s(ev, "id"));
    if b(ev, "me") || chat.is_empty() || id.is_empty() || is_banned(chat, s(ev, "sender")) {
        return;
    }
    inbound("system", chat, id, &format!("[deleted] {id}"), 0);
}

fn reacted(ev: &Value) {
    let (chat, id) = (s(ev, "chat"), s(ev, "id"));
    let sender = s(ev, "sender");
    if b(ev, "me") || chat.is_empty() || id.is_empty() || is_banned(chat, sender) {
        return;
    }
    let emoji = s(ev, "emoji").trim();
    let body = if emoji.is_empty() { "[reaction] removed".to_string() } else { format!("[reaction] {emoji}") };
    // the envelope id is the message reacted to (what react/delete need)
    inbound(&label(sender, s(ev, "sender_name")), chat, id, &body, 0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_and_names() {
        assert_eq!(label("100", "Ann"), "Ann (100)");
        assert_eq!(label("100", ""), "100");
        assert_eq!(label("100", "100"), "100");
        assert_eq!(ext_for("audio/mp4", "", "voice"), "m4a");
        assert_eq!(ext_for("", "Report.PDF", "document"), "pdf");
        assert_eq!(ext_for("", "", "image"), "jpg");
        assert!(!seen("x1"));
        assert!(seen("x1"), "second delivery of the same id is dropped");
    }

    #[test]
    fn chat_lines() {
        set_platform(Platform::Messenger);
        note_title("t5", "Family");
        assert_eq!(format_chat("t5", ""), "messenger:t5  Family");
        assert_eq!(format_chat("9", "9"), "messenger:9");
    }
}
