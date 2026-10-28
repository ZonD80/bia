//! The tools of the messenger and instagram MCPs (the agent prefixes them: messenger_send,
//! instagram_send, …). Each one is a call to bia-meta.

use crate::helper::{self, LONG, NORMAL, QUICK};
use crate::inbound::{self, format_chat};
use crate::state;
use crate::Platform;
use bia_mcp::{arg_bool, arg_str, arg_str_list, arg_str_opt, arg_u32_opt, Ctx, Server, ToolResult};
use serde_json::{json, Value};
use std::time::Duration;

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

fn call(method: &str, params: Value, timeout: Duration) -> Result<Value, String> {
    helper::get().call(method, params, timeout)
}

fn platform() -> Platform {
    inbound::platform()
}

fn chat_arg(a: &Value) -> Result<String, String> {
    let c = platform().chat_id(arg_str(a, "chat_id")?);
    if c.is_empty() {
        return Err("empty chat_id".into());
    }
    Ok(c)
}

/// Message ids as strings (models sometimes send numbers or a single id).
fn id_list(a: &Value, key: &str) -> Result<Vec<String>, String> {
    let items = match a.get(key) {
        Some(Value::Array(v)) => v.clone(),
        Some(v @ (Value::String(_) | Value::Number(_))) => vec![v.clone()],
        _ => return Err(format!("missing argument: {key}")),
    };
    let ids: Vec<String> = items
        .iter()
        .map(|v| match v {
            Value::String(s) => s.trim().to_string(),
            other => other.to_string(),
        })
        .filter(|s| !s.is_empty())
        .collect();
    if ids.is_empty() {
        return Err(format!("{key} is empty"));
    }
    Ok(ids)
}

/// Paths under /home/bia/files (relative ones too).
fn file_path(f: &str) -> Result<String, String> {
    let p = std::path::Path::new(f);
    let abs = if p.is_absolute() { p.to_path_buf() } else { state::files_dir().join(f.trim_start_matches('/')) };
    if !abs.is_file() {
        return Err(format!("file not found: {}", abs.display()));
    }
    Ok(abs.to_string_lossy().into_owned())
}

fn audio_ext(path: &str) -> bool {
    let e = std::path::Path::new(path).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    matches!(e.as_str(), "ogg" | "oga" | "opus")
}

fn chat_title(chat: &str) -> String {
    inbound::title_of(chat)
}

fn require_login() -> Result<(), String> {
    let c = state::load_cfg(platform());
    if !c.logged_in {
        let p = platform();
        return Err(format!("not logged in to {}; call {}_login", p.title(), p.name()));
    }
    Ok(())
}

// --- account ---

fn login(a: &Value) -> ToolResult {
    let p = platform();
    let mut params = json!({});
    if let Some(c) = a.get("cookies") {
        match c {
            Value::Object(_) => params["cookies"] = c.clone(),
            Value::String(t) if !t.trim().is_empty() => params["cookie_text"] = json!(t),
            _ => {}
        }
    }
    if let Some(v) = arg_str_opt(a, "variant").filter(|v| !v.is_empty()) {
        params["variant"] = json!(v);
    }
    let res = call("login", params, LONG)?;
    let (id, name, username) = (s(&res, "id"), s(&res, "name"), s(&res, "username"));
    state::update_cfg(p, |c| {
        c.logged_in = true;
        c.id = id.to_string();
        c.name = name.to_string();
        c.username = username.to_string();
        if c.last_seen == 0 {
            c.last_seen = state::now_secs();
        }
    });
    let who = if username.is_empty() { name.to_string() } else { format!("{name} (@{username})") };
    let extra = match p {
        Platform::Messenger => " Encrypted chats connect in the background (a new device appears in the account's settings as bia's).",
        Platform::Instagram => "",
    };
    // Instagram reports whether the connection came up, not only the accepted cookies
    let conn = match s(&res, "state") {
        "" | "ready" => String::new(),
        st => {
            let why = if s(&res, "error").is_empty() { String::new() } else { format!(" ({})", s(&res, "error")) };
            format!(
                " The connection is not up yet: state {st}{why}. It retries by itself; check {n}_status in a minute. If it stays down, open https://www.instagram.com/direct/inbox/ in bia's Chrome (a security check may wait there), then call {n}_login again.",
                n = p.name()
            )
        }
    };
    Ok(format!("logged in to {} as {who}, id {id}. Messages arrive on {}:<chat>.{extra}{conn}", p.title(), p.name()))
}

fn status() -> ToolResult {
    let p = platform();
    let cfg = state::load_cfg(p);
    if !cfg.logged_in && !helper::get().running() {
        return Ok(format!("auth=logged_out — call {}_login", p.name()));
    }
    let st = call("status", json!({}), QUICK)?;
    let mut line = format!("auth={}", s(&st, "state"));
    if p == Platform::Messenger && !s(&st, "e2ee").is_empty() {
        line.push_str(&format!("  encrypted_chats={}", s(&st, "e2ee")));
    }
    let name = if s(&st, "name").is_empty() { cfg.name.clone() } else { s(&st, "name").to_string() };
    if !name.is_empty() {
        line.push_str(&format!("  account={name}"));
        let user = if s(&st, "username").is_empty() { cfg.username.clone() } else { s(&st, "username").to_string() };
        if !user.is_empty() {
            line.push_str(&format!(" (@{user})"));
        }
        line.push_str(&format!(" id {}", if s(&st, "id").is_empty() { &cfg.id } else { s(&st, "id") }));
    }
    if !s(&st, "error").is_empty() {
        line.push_str(&format!("  error: {}", s(&st, "error")));
    }
    let bans = state::bans(p);
    if !bans.is_empty() {
        line.push_str(&format!("  banned={}", bans.iter().map(|b| p.channel(b)).collect::<Vec<_>>().join(",")));
    }
    Ok(line)
}

fn restore() -> ToolResult {
    let p = platform();
    if !state::load_cfg(p).logged_in {
        return Ok(format!("{} not logged in; call {}_login", p.title(), p.name()));
    }
    let st = call("restore", json!({}), LONG)?;
    match s(&st, "state") {
        "ready" => Ok(format!("{} connected as {}", p.title(), s(&st, "name"))),
        "logged_out" => {
            state::update_cfg(p, |c| c.logged_in = false);
            let why = if s(&st, "error").is_empty() { String::new() } else { format!(" ({})", s(&st, "error")) };
            Ok(format!("{} session ended{why}; sign in again in Chrome and call {}_login", p.title(), p.name()))
        }
        other => Ok(format!("{} starting ({other}{}); it keeps retrying in the background", p.title(),
            if s(&st, "error").is_empty() { String::new() } else { format!(": {}", s(&st, "error")) })),
    }
}

fn logout() -> ToolResult {
    let p = platform();
    let r = call("logout", json!({}), NORMAL);
    helper::get().stop();
    state::update_cfg(p, |c| {
        c.logged_in = false;
        c.id.clear();
        c.name.clear();
        c.username.clear();
        c.last_seen = 0;
    });
    r?;
    Ok(format!("logged out of {} on this computer (the account stays signed in in Chrome)", p.title()))
}

// --- messages ---

fn sent_line(res: &Value, chat: &str, what: &str) -> String {
    let ids: Vec<String> = res
        .get("ids")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    format!("sent {what}message id {} → {}", ids.join(", "), format_chat(chat, &chat_title(chat)))
}

fn send(ctx: &Ctx, a: &Value) -> ToolResult {
    require_login()?;
    let chat = chat_arg(a)?;
    let text = bia_mcp::md::plain(arg_str_opt(a, "text").unwrap_or(""));
    let files = arg_str_list(a, "files");
    if text.trim().is_empty() && files.is_empty() {
        return Err("text or files required".into());
    }
    let mut fs = Vec::new();
    for f in &files {
        let path = file_path(f)?;
        let kind = if audio_ext(&path) { "voice" } else { "" };
        let mut entry = json!({"path": path, "kind": kind});
        if kind == "voice" {
            if let Ok(x) = bia_speech::audio::decode_file(std::path::Path::new(entry["path"].as_str().unwrap_or(""))) {
                entry["seconds"] = json!(x.len() as f64 / 16_000.0);
            }
        }
        fs.push(entry);
    }
    if ctx.cancelled() {
        return Err("cancelled".into());
    }
    let mut params = json!({"chat": chat, "text": text, "files": fs});
    if let Some(r) = arg_str_opt(a, "reply_to").filter(|r| !r.trim().is_empty()) {
        params["reply_to"] = json!(r.trim());
    }
    let res = call("send", params, if files.is_empty() { NORMAL } else { LONG })?;
    let what = if files.is_empty() { String::new() } else { format!("{} file(s), ", files.len()) };
    Ok(sent_line(&res, &chat, &what))
}

fn send_voice(a: &Value) -> ToolResult {
    require_login()?;
    let p = platform();
    let chat = chat_arg(a)?;
    let text = arg_str(a, "text")?;
    let lang = bia_speech::contacts::lang_or(arg_str_opt(a, "lang").unwrap_or(""), &p.channel(&chat));
    let out = bia_speech::paths::tmp().join(format!("voice-{}.ogg", bia_speech::paths::stamp()));
    let (secs, _) = bia_speech::tts::voice_note(text, &lang, &out)?;
    let res = call(
        "send",
        json!({"chat": chat, "files": [{"path": out, "kind": "voice", "seconds": secs, "mime": "audio/ogg"}]}),
        LONG,
    );
    let _ = std::fs::remove_file(&out);
    Ok(sent_line(&res?, &chat, &format!("voice message ({secs:.1}s), ")))
}

fn edit(a: &Value) -> ToolResult {
    require_login()?;
    let chat = chat_arg(a)?;
    let id = arg_str(a, "message_id")?.trim().to_string();
    let text = bia_mcp::md::plain(arg_str(a, "text")?);
    call("edit", json!({"chat": chat, "id": id, "text": text}), NORMAL)?;
    Ok(format!("edited message {id} → {}", format_chat(&chat, "")))
}

fn react(a: &Value) -> ToolResult {
    require_login()?;
    let chat = chat_arg(a)?;
    let id = arg_str(a, "message_id")?.trim().to_string();
    let emoji = arg_str_opt(a, "emoji").unwrap_or("").trim().to_string();
    call("react", json!({"chat": chat, "id": id, "emoji": emoji}), NORMAL)?;
    Ok(if emoji.is_empty() {
        format!("removed reaction on message {id} → {}", format_chat(&chat, ""))
    } else {
        format!("reacted {emoji} on message {id} → {}", format_chat(&chat, ""))
    })
}

/// Meta's protocol as bia-meta speaks it has no join-by-link call; bia's Chrome is signed in
/// to the same account (bia-meta logs in with its cookies), and joining there is the real
/// Join button, after which the group arrives like any other chat.
fn join_group(p: Platform, link: &str) -> ToolResult {
    let l = link.trim();
    let bare = l.trim_start_matches("https://").trim_start_matches("http://").trim_start_matches("www.");
    let ok = match p {
        Platform::Instagram => bare.starts_with("ig.me/j/") || bare.starts_with("instagram.com/"),
        _ => bare.starts_with("m.me/j/") || bare.starts_with("messenger.com/") || bare.starts_with("facebook.com/messages/"),
    };
    if !ok {
        return Err(format!(
            "{l:?} is not a {} group invite link ({})",
            p.title(),
            if p == Platform::Instagram { "https://ig.me/j/…" } else { "https://m.me/j/…" }
        ));
    }
    let url = if l.starts_with("http") { l.to_string() } else { format!("https://{bare}") };
    Ok(format!(
        "{t} has no join-by-link call here; join it in bia's Chrome, which is signed in to the same {t} account: \
         chrome_new_page(url=\"{url}\"), press the Join button there (Join chat / Join group), then {n}_chats lists the group \
         within a minute ({n}_restore if it does not). If the page asks to log in, the {t} session in Chrome ended: log in there first.",
        t = p.title(),
        n = p.name()
    ))
}

fn delete_messages(a: &Value) -> ToolResult {
    require_login()?;
    let chat = chat_arg(a)?;
    let ids = id_list(a, "message_ids")?;
    let everyone = arg_bool(a, "revoke", true);
    let res = call("delete", json!({"chat": chat, "ids": ids, "everyone": everyone}), NORMAL)?;
    let done = res.get("deleted").and_then(Value::as_array).map(|v| v.len()).unwrap_or(0);
    let failed: Vec<String> = res
        .get("failed")
        .and_then(Value::as_array)
        .map(|v| v.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let side = if everyone { "for everyone" } else { "on this account" };
    let mut out = format!("deleted {done} message(s) {side} → {}", format_chat(&chat, ""));
    if !failed.is_empty() {
        out.push_str(&format!("; not deleted: {}", failed.join("; ")));
    }
    Ok(out)
}

fn history_line(m: &Value) -> String {
    let who = if m.get("me").and_then(Value::as_bool).unwrap_or(false) {
        "me".to_string()
    } else {
        inbound::label(s(m, "sender"), s(m, "sender_name"))
    };
    let mut body = s(m, "text").replace('\n', " ");
    if let Some(atts) = m.get("attachments").and_then(Value::as_array) {
        for a in atts {
            body = format!("[{}] {body}", s(a, "kind")).trim().to_string();
        }
    }
    if body.is_empty() {
        body = "(no text)".into();
    }
    let ts = m.get("ts").and_then(Value::as_i64).unwrap_or(0);
    format!("{}  {}  [{who}]  {body}", s(m, "id"), bia_mcp::share::rfc3339(ts))
}

/// Newest first, each id once (local and server pages overlap).
fn merge_history(local: Vec<Value>, server: Vec<Value>, limit: usize) -> Vec<Value> {
    let mut seen = std::collections::HashSet::new();
    let mut rows: Vec<Value> = local
        .into_iter()
        .chain(server)
        .filter(|m| seen.insert(s(m, "id").to_string()))
        .collect();
    rows.sort_by_key(|m| std::cmp::Reverse(m.get("ts").and_then(Value::as_i64).unwrap_or(0)));
    rows.truncate(limit);
    rows
}

fn history(a: &Value) -> ToolResult {
    require_login()?;
    let chat = chat_arg(a)?;
    let limit = arg_u32_opt(a, "limit").unwrap_or(30).clamp(1, 100) as usize;
    let before = arg_str_opt(a, "before").unwrap_or("").trim().to_string();
    let head = format_chat(&chat, "");
    // what bia saw while logged in (encrypted chats live only there), filled up from the
    // server when that is not a whole page (older messages, from before the login)
    let local = call("recent", json!({"chat": chat, "limit": limit, "before": before}), QUICK)?
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut note = String::new();
    let mut server = Vec::new();
    if local.len() < limit {
        match call("history", json!({"chat": chat, "limit": limit, "before": before}), NORMAL) {
            Ok(Value::Array(v)) => server = v,
            Ok(_) => {}
            Err(e) if e.contains("encrypted chats have no history") => {
                note = " (encrypted chat: only messages seen while logged in; the server keeps none for linked devices)".into();
            }
            Err(e) => note = format!(" (older messages from the server not available: {e})"),
        }
    }
    let rows = merge_history(local, server, limit);
    if rows.is_empty() {
        return Ok(format!("{head}{note}\n(no messages)"));
    }
    let mut lines = vec![format!("{head}{note}")];
    lines.extend(rows.iter().map(history_line));
    if rows.len() == limit {
        lines.push(format!("(more: before={})", s(rows.last().unwrap_or(&Value::Null), "id")));
    }
    Ok(lines.join("\n"))
}

fn set_profile(a: &Value) -> ToolResult {
    require_login()?;
    let Some(bio) = arg_str_opt(a, "bio") else {
        return Err("pass bio (the name and photo are changed at https://www.facebook.com/me in bia's Chrome)".into());
    };
    call("set_profile", json!({"bio": bio}), NORMAL)?;
    Ok(if bio.trim().is_empty() { "Facebook bio cleared".into() } else { format!("Facebook bio set to {:?}", bio.trim()) })
}

// --- chats and people ---

fn chat_row(c: &Value) -> String {
    let chat = s(c, "chat");
    let title = s(c, "title");
    inbound::note_title(chat, title);
    let mut line = format_chat(chat, title);
    if let Some(u) = c.get("username").and_then(Value::as_str).filter(|u| !u.is_empty()) {
        line.push_str(&format!(" (@{u})"));
    }
    let mut flags = Vec::new();
    if c.get("group").and_then(Value::as_bool).unwrap_or(false) {
        flags.push("group");
    }
    if c.get("e2ee").and_then(Value::as_bool).unwrap_or(false) {
        flags.push("encrypted");
    }
    if c.get("request").and_then(Value::as_bool).unwrap_or(false) {
        flags.push("message request");
    }
    if c.get("cannot_message").and_then(Value::as_bool).unwrap_or(false) {
        flags.push("cannot be messaged");
    }
    if state::bans(platform()).contains(chat) {
        flags.push("banned");
    }
    if !flags.is_empty() {
        line.push_str(&format!("  [{}]", flags.join(", ")));
    }
    if let Some(about) = c.get("about").and_then(Value::as_str).filter(|x| !x.is_empty()) {
        line.push_str(&format!("  — {about}"));
    }
    line
}

fn chats(a: &Value) -> ToolResult {
    require_login()?;
    let limit = arg_u32_opt(a, "limit").unwrap_or(50).clamp(1, 500);
    let rows = call("chats", json!({"limit": limit, "refresh": arg_bool(a, "refresh", false)}), NORMAL)?;
    let rows = rows.as_array().cloned().unwrap_or_default();
    if rows.is_empty() {
        return Ok("(no chats yet; they fill in as messages arrive and after a search)".into());
    }
    Ok(rows.iter().map(chat_row).collect::<Vec<_>>().join("\n"))
}

fn search(a: &Value) -> ToolResult {
    require_login()?;
    let q = arg_str(a, "query")?;
    let limit = arg_u32_opt(a, "limit").unwrap_or(20).clamp(1, 50);
    let rows = call("search", json!({"query": q, "limit": limit}), NORMAL)?;
    let rows = rows.as_array().cloned().unwrap_or_default();
    if rows.is_empty() {
        return Ok(format!("(nobody on {} matches {q})", platform().title()));
    }
    Ok(rows.iter().map(chat_row).collect::<Vec<_>>().join("\n"))
}

fn resolve(a: &Value) -> ToolResult {
    require_login()?;
    let q = arg_str(a, "query")?;
    let r = call("resolve", json!({"query": q}), NORMAL)?;
    Ok(chat_row(&r))
}

fn group_info(a: &Value) -> ToolResult {
    require_login()?;
    let chat = chat_arg(a)?;
    let r = call("group_info", json!({"chat": chat}), NORMAL)?;
    let title = s(&r, "title");
    inbound::note_title(&chat, title);
    let mut lines = vec![format!("{} — {}", format_chat(&chat, title), if s(&r, "kind").is_empty() { "group" } else { s(&r, "kind") })];
    if !s(&r, "description").trim().is_empty() {
        lines.push(format!("description: {}", s(&r, "description").trim()));
    }
    for st in r.get("settings").and_then(Value::as_array).cloned().unwrap_or_default() {
        if let Some(t) = st.as_str() {
            lines.push(t.to_string());
        }
    }
    if let Some(n) = r.get("pending_requests").and_then(Value::as_i64) {
        lines.push(format!("pending join requests: {n} ({}_join_requests)", platform().name()));
    }
    let members = r.get("members").and_then(Value::as_array).cloned().unwrap_or_default();
    lines.push(format!("members ({}):", members.len()));
    for m in &members {
        let mut line = format!("  {}", inbound::label(s(m, "id"), s(m, "name")));
        if let Some(u) = m.get("username").and_then(Value::as_str).filter(|u| !u.is_empty()) {
            line.push_str(&format!(" @{u}"));
        }
        let role = s(m, "role");
        if !role.is_empty() && role != "member" {
            line.push_str(&format!("  {role}"));
        } else if m.get("admin").and_then(Value::as_bool).unwrap_or(false) {
            line.push_str("  admin");
        }
        lines.push(line);
    }
    if members.is_empty() {
        lines.push("  (not known yet)".into());
    }
    match r.get("you_admin").and_then(Value::as_bool) {
        Some(true) => lines.push("you are an admin".into()),
        Some(false) => lines.push("you are not an admin here (you cannot change these settings)".into()),
        None => {}
    }
    if !s(&r, "note").is_empty() {
        lines.push(s(&r, "note").to_string());
    }
    Ok(lines.join("\n"))
}

fn set_group_info(a: &Value) -> ToolResult {
    require_login()?;
    let p = platform();
    let chat = chat_arg(a)?;
    let title = arg_str_opt(a, "title").map(str::trim).filter(|t| !t.is_empty());
    let description = arg_str_opt(a, "description");
    let photo = arg_str_opt(a, "photo").filter(|x| !x.trim().is_empty());
    if title.is_none() && description.is_none() && photo.is_none() {
        return Err("pass title, description and/or photo".into());
    }
    let mut done = Vec::new();
    if let Some(t) = title {
        call("rename", json!({"chat": chat, "title": t}), NORMAL)?;
        inbound::note_title(&chat, t);
        done.push(format!("title \"{t}\""));
    }
    if let Some(d) = description {
        if p == Platform::Instagram {
            return Err(format!("Instagram groups have no description{}", if done.is_empty() { String::new() } else { format!(" (already set: {})", done.join(", ")) }));
        }
        call("group_settings", json!({"chat": chat, "description": d}), NORMAL).map_err(|e| {
            if done.is_empty() { e } else { format!("{e} (already set: {})", done.join(", ")) }
        })?;
        done.push(if d.trim().is_empty() { "description cleared".into() } else { "description".into() });
    }
    if let Some(ph) = photo {
        let path = file_path(ph)?;
        call("group_photo", json!({"chat": chat, "path": path}), LONG).map_err(|e| {
            if done.is_empty() { e } else { format!("{e} (already set: {})", done.join(", ")) }
        })?;
        done.push("photo".into());
    }
    Ok(format!("{}: set {}", format_chat(&chat, ""), done.join(", ")))
}

fn set_admin(a: &Value) -> ToolResult {
    require_login()?;
    let chat = chat_arg(a)?;
    let member = arg_str(a, "member")?;
    let admin = arg_bool(a, "admin", true);
    call("set_admin", json!({"chat": chat, "member": member, "admin": admin}), NORMAL)?;
    Ok(format!("{member} {} {}", if admin { "is now an admin of" } else { "is no longer an admin of" }, format_chat(&chat, "")))
}

fn group_permissions(a: &Value) -> ToolResult {
    require_login()?;
    let chat = chat_arg(a)?;
    let mut params = json!({"chat": chat});
    for k in ["only_admins_send", "only_admins_edit_info", "only_admins_add", "join_approval"] {
        if let Some(v) = a.get(k).and_then(Value::as_bool) {
            params[k] = json!(v);
        }
    }
    if let Some(v) = a.get("disappearing_seconds").and_then(Value::as_i64) {
        params["disappearing_seconds"] = json!(v);
    }
    let r = call("group_settings", params, NORMAL)?;
    let done: Vec<String> = r.get("done").and_then(Value::as_array).map(|v| v.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
    Ok(format!("{}: {}", format_chat(&chat, ""), done.join("; ")))
}

fn join_requests(a: &Value) -> ToolResult {
    require_login()?;
    let chat = chat_arg(a)?;
    let rows = call("join_requests", json!({"chat": chat}), NORMAL)?.as_array().cloned().unwrap_or_default();
    if rows.is_empty() {
        return Ok(format!("no pending join requests in {}", format_chat(&chat, "")));
    }
    let mut lines = vec![format!("{} join request(s) in {} (answer with {}_answer_join_request):", rows.len(), format_chat(&chat, ""), platform().name())];
    for r in &rows {
        let ts = r.get("ts").and_then(Value::as_i64).unwrap_or(0);
        lines.push(format!("{} asked {}", inbound::label(s(r, "id"), s(r, "name")), bia_mcp::share::rfc3339(ts)));
    }
    Ok(lines.join("\n"))
}

// --- groups and chat settings ---

fn create_group(a: &Value) -> ToolResult {
    require_login()?;
    let title = arg_str(a, "title")?;
    let members = arg_str_list(a, "members");
    let r = call("create_group", json!({"title": title, "members": members}), NORMAL)?;
    let chat = s(&r, "chat");
    inbound::note_title(chat, title);
    let mut out = format!("created group {} with {} member(s)", format_chat(chat, title), members.len());
    if !s(&r, "warning").is_empty() {
        out.push_str(&format!("; {}", s(&r, "warning")));
    }
    Ok(out)
}

fn add_members(a: &Value) -> ToolResult {
    require_login()?;
    let chat = chat_arg(a)?;
    let members = arg_str_list(a, "members");
    if members.is_empty() {
        return Err("members required".into());
    }
    let r = call("add_members", json!({"chat": chat, "members": members}), NORMAL)?;
    let failed: Vec<String> = r
        .get("failed")
        .and_then(Value::as_array)
        .map(|v| v.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let mut out = format!("added {} to {}", members.join(", "), format_chat(&chat, ""));
    if !failed.is_empty() {
        out.push_str(&format!("; refused: {}", failed.join(", ")));
    }
    Ok(out)
}

fn simple(method: &str, params: Value, done: String) -> ToolResult {
    require_login()?;
    call(method, params, NORMAL)?;
    Ok(done)
}

fn ban(a: &Value, on: bool) -> ToolResult {
    let chat = chat_arg(a)?;
    let changed = state::set_ban(platform(), &chat, on)?;
    let who = format_chat(&chat, "");
    Ok(match (on, changed) {
        (true, true) => format!("banned {who}. Their messages are not delivered; they are answered with I've banned you."),
        (true, false) => format!("already banned {who}"),
        (false, true) => format!("unbanned {who}. Their messages are delivered again."),
        (false, false) => format!("not banned {who}"),
    })
}

fn typing(a: &Value) -> ToolResult {
    if state::load_cfg(platform()).logged_in && helper::get().running() {
        let chat = chat_arg(a)?;
        let _ = call("typing", json!({"chat": chat, "on": arg_bool(a, "on", true)}), QUICK);
    }
    Ok(String::new())
}

// --- the server ---

fn obj(props: Value, required: &[&str]) -> Value {
    json!({"type": "object", "required": required, "properties": props})
}

pub fn serve(p: Platform) {
    inbound::set_platform(p);
    helper::init(p, inbound::on_event);
    let (n, t) = (p.name(), p.title());
    let hint = p.chat_hint();
    let chat = || json!({"type": "string", "description": hint});
    let login_desc = match p {
        Platform::Messenger => format!(
            "Log in to Messenger with the Facebook account that is signed in to bia's Chrome (bia never asks for the password here; the session is taken from Chrome). If Chrome is not signed in: start it (chrome MCP), open https://www.facebook.com/ and sign in there (passwords and 2FA codes from the secrets MCP), then call {n}_login again. variant=messenger takes the session from messenger.com instead. cookies is only for a session a person pasted (a Cookie header or a JSON object). This is an unofficial client: Meta may ask for a security check in the browser. Afterwards messages arrive on {n}:<chat>."
        ),
        Platform::Instagram => format!(
            "Log in to Instagram with the account that is signed in to bia's Chrome (no password here; the session is taken from Chrome). If Chrome is not signed in: start it (chrome MCP), open https://www.instagram.com/accounts/login/ and sign in (passwords and 2FA codes from the secrets MCP), then call {n}_login again. cookies is only for a session a person pasted (a Cookie header or a JSON object). This is an unofficial client: Instagram may ask for a security check in the browser. Afterwards messages arrive on {n}:<chat>."
        ),
    };
    let mut server = Server::new(n, env!("CARGO_PKG_VERSION"))
        .tool("login", &login_desc, json!({"type": "object", "properties": {
            "variant": {"type": "string", "description": "Messenger only: facebook (default) or messenger (messenger.com)"},
            "cookies": {"description": "optional: a session a person pasted, as a Cookie header string or a JSON object of cookie names to values"}
        }}), |ctx, a| {
            inbound::set_ctx(ctx.clone());
            login(&a)
        })
        .tool("status", &format!("Whether bia is logged in to {t}, the account, the connection state and banned chats."), json!({"type": "object", "properties": {}}), |ctx, _| {
            inbound::set_ctx(ctx.clone());
            status()
        })
        .tool("logout", &format!("Log out of {t} on this computer (forgets the session and the chat index here; Chrome stays signed in). Call {n}_login to log in again."), json!({"type": "object", "properties": {}}), |ctx, _| {
            inbound::set_ctx(ctx.clone());
            logout()
        })
        .tool("restore", &format!("Reconnect the saved {t} login. This runs by itself at boot; call it only if status shows it disconnected."), json!({"type": "object", "properties": {}}), |ctx, _| {
            inbound::set_ctx(ctx.clone());
            restore()
        })
        .tool("send", &format!("Send a {t} message. text is the message (markdown is turned into plain text; links keep their address). files are paths under /home/bia/files: pictures and videos are sent as media, .ogg/.opus as a voice message, anything else as a file. reply_to quotes a message id."), obj(json!({
            "chat_id": chat(),
            "text": {"type": "string", "description": "the message; optional with files"},
            "files": {"type": "array", "items": {"type": "string"}, "description": "file paths under /home/bia/files"},
            "reply_to": {"type": "string", "description": "id of the message this answers"}
        }), &["chat_id"]), |ctx, a| {
            inbound::set_ctx(ctx.clone());
            send(ctx, &a)
        })
        .tool("send_voice", &format!("Send a voice message: text is spoken with the voice of lang and sent as a {t} voice clip. Only when the person asks for a voice reply. lang is the language of the text (defaults to their preferred language from the addressbook); mark other-language parts inline: <lang=en>Grand Hotel</lang>. A language without a voice fails: lang_install it first (lang MCP)."), obj(json!({
            "chat_id": chat(),
            "text": {"type": "string", "description": "what to say"},
            "lang": {"type": "string", "description": "language of the text: en, ru, es-MX, …"}
        }), &["chat_id", "text"]), |ctx, a| {
            inbound::set_ctx(ctx.clone());
            send_voice(&a)
        })
        .tool("edit", &format!("Replace the text of a message this account sent on {t} (message_id from the send result or {n}_history)."), obj(json!({
            "chat_id": chat(),
            "message_id": {"type": "string", "description": "id of your message"},
            "text": {"type": "string", "description": "the new text"}
        }), &["chat_id", "message_id", "text"]), |ctx, a| {
            inbound::set_ctx(ctx.clone());
            edit(&a)
        })
        .tool("react", &format!("React to a {t} message with an emoji (message_id from an inbound envelope or {n}_history). An empty emoji removes this account's reaction."), obj(json!({
            "chat_id": chat(),
            "message_id": {"type": "string", "description": "the message to react to"},
            "emoji": {"type": "string", "description": "e.g. 👍; empty removes the reaction"}
        }), &["chat_id", "message_id"]), |ctx, a| {
            inbound::set_ctx(ctx.clone());
            react(&a)
        })
        .tool("delete_messages", &format!("Delete messages on {t}. revoke true (default) unsends this account's messages for everyone; false removes them only here{}. This cannot be undone.",
            if p == Platform::Instagram { " (Instagram has no remove-for-me: false is refused)" } else { "" }), obj(json!({
            "chat_id": chat(),
            "message_ids": {"type": "array", "items": {"type": "string"}, "description": "ids from inbound envelopes or history"},
            "revoke": {"type": "boolean", "description": "true (default) for everyone; false only on this account"}
        }), &["chat_id", "message_ids"]), |ctx, a| {
            inbound::set_ctx(ctx.clone());
            delete_messages(&a)
        })
        .tool("history", &format!("Recent messages in a {t} chat, newest first: message id, time (RFC 3339 UTC), sender and text. Pass before as the last id of the previous page to go further back."), obj(json!({
            "chat_id": chat(),
            "limit": {"type": "integer", "description": "how many (default 30, at most 100)"},
            "before": {"type": "string", "description": "message id: older than this one"}
        }), &["chat_id"]), |ctx, a| {
            inbound::set_ctx(ctx.clone());
            history(&a)
        })
        .tool("chats", &format!("{t} chats, most recent first: chat id, title, and flags (group, encrypted, message request, banned)."), json!({"type": "object", "properties": {
            "limit": {"type": "integer", "description": "how many (default 50)"},
            "refresh": {"type": "boolean", "description": "Instagram: fetch the inbox from the server first"}
        }}), |ctx, a| {
            inbound::set_ctx(ctx.clone());
            chats(&a)
        })
        .tool("search", &format!("Find people and groups on {t} by name{}. Returns lines whose chat id works with {n}_send.",
            if p == Platform::Instagram { " or @username" } else { "" }), obj(json!({
            "query": {"type": "string", "description": "a name, part of one, or a username"},
            "limit": {"type": "integer", "description": "how many (default 20)"}
        }), &["query"]), |ctx, a| {
            inbound::set_ctx(ctx.clone());
            search(&a)
        })
        .tool("resolve", &format!("Turn one name, @username, profile link or id into the {t} chat id {n}_send needs."), obj(json!({
            "query": {"type": "string", "description": "e.g. @ann, https://www.facebook.com/ann.lee, a name, an id"}
        }), &["query"]), |ctx, a| {
            inbound::set_ctx(ctx.clone());
            resolve(&a)
        })
        .tool("typing", "Show typing in a chat. You do not need to call this; it happens while you are replying.", obj(json!({
            "chat_id": chat(),
            "on": {"type": "boolean", "description": "true shows typing, false stops it (default true)"}
        }), &["chat_id"]), |ctx, a| {
            inbound::set_ctx(ctx.clone());
            typing(&a)
        })
        .tool("create_group", &format!("Create a {t} group with people (names, @usernames, profile links or ids from {n}_search). Returns its chat id.{}",
            if p == Platform::Instagram { " Instagram needs at least two other people." } else { "" }), obj(json!({
            "title": {"type": "string", "description": "the group's name"},
            "members": {"type": "array", "items": {"type": "string"}, "description": "people to add"}
        }), &["title", "members"]), |ctx, a| {
            inbound::set_ctx(ctx.clone());
            create_group(&a)
        })
        .tool("group_info", &format!("A {t} group: title, members and who is admin{}, and whether you are an admin.",
            if p == Platform::Messenger { ", and for encrypted groups the description, who may send / edit the info / add members, join approval and disappearing messages" } else { "" }), obj(json!({"chat_id": chat()}), &["chat_id"]), |ctx, a| {
            inbound::set_ctx(ctx.clone());
            group_info(&a)
        })
        .tool("set_admin", &format!("Make a member an admin of a {t} group (admin=true, default) or take it back (admin=false). {t} admins all have the same rights. You must be an admin."), obj(json!({
            "chat_id": chat(),
            "member": {"type": "string", "description": "name, @username, link or id of a member"},
            "admin": {"type": "boolean", "description": "true makes them admin (default), false takes it away"}
        }), &["chat_id", "member"]), |ctx, a| {
            inbound::set_ctx(ctx.clone());
            set_admin(&a)
        })
        .tool("set_group_info", &format!("Change a {t} group's title{}{} and/or photo (an image under /home/bia/files{}). Only what you pass changes.",
            if p == Platform::Messenger { ", description (encrypted groups; empty clears it)" } else { "" }, "",
            if p == Platform::Messenger { "; JPEG for encrypted groups" } else { "" }), obj(json!({
            "chat_id": chat(),
            "title": {"type": "string", "description": "new name"},
            "description": {"type": "string", "description": "Messenger encrypted groups: new description; empty clears it"},
            "photo": {"type": "string", "description": "image path under /home/bia/files"}
        }), &["chat_id"]), |ctx, a| {
            inbound::set_ctx(ctx.clone());
            set_group_info(&a)
        })
        .tool("add_members", &format!("Add people to a {t} group."), obj(json!({
            "chat_id": chat(),
            "members": {"type": "array", "items": {"type": "string"}, "description": "names, @usernames, links or ids"}
        }), &["chat_id", "members"]), |ctx, a| {
            inbound::set_ctx(ctx.clone());
            add_members(&a)
        })
        .tool("remove_member", &format!("Remove a person from a {t} group you administer."), obj(json!({
            "chat_id": chat(),
            "member": {"type": "string", "description": "name, @username, link or id"}
        }), &["chat_id", "member"]), |ctx, a| {
            inbound::set_ctx(ctx.clone());
            let chat = chat_arg(&a)?;
            let m = arg_str(&a, "member")?;
            simple("remove_member", json!({"chat": chat, "member": m}), format!("removed {m} from {}", format_chat(&chat, "")))
        })
        .tool("join_group", &format!("Join a {t} group by its invite link ({}): this answers how, through bia's Chrome.",
            if p == Platform::Instagram { "ig.me/j/…" } else { "m.me/j/…" }), obj(json!({
            "link": {"type": "string", "description": "the group's invite link"}
        }), &["link"]), move |_ctx, a| join_group(p, a.get("link").and_then(Value::as_str).unwrap_or(""))
        )
        .tool("leave_group", &format!("Leave a {t} group."), obj(json!({"chat_id": chat()}), &["chat_id"]), |ctx, a| {
            inbound::set_ctx(ctx.clone());
            let chat = chat_arg(&a)?;
            simple("leave", json!({"chat": chat}), format!("left {}", format_chat(&chat, "")))
        })
        .tool("ban", &format!("Ban a {t} chat or person in bia: their messages are not delivered to you and are answered with I've banned you. (block blocks them on {t} itself.)"), obj(json!({"chat_id": chat()}), &["chat_id"]), |ctx, a| {
            inbound::set_ctx(ctx.clone());
            ban(&a, true)
        })
        .tool("unban", &format!("Unban a {t} chat so its messages are delivered again."), obj(json!({"chat_id": chat()}), &["chat_id"]), |ctx, a| {
            inbound::set_ctx(ctx.clone());
            ban(&a, false)
        })
        .tool("block", &format!("Block (on=true, default) or unblock (on=false) a person on {t} itself: they can no longer message or call this account."), obj(json!({
            "user": {"type": "string", "description": "name, @username, link or id"},
            "on": {"type": "boolean", "description": "true blocks (default), false unblocks"}
        }), &["user"]), move |ctx, a| {
            inbound::set_ctx(ctx.clone());
            let user = arg_str(&a, "user")?;
            let on = arg_bool(&a, "on", true);
            simple("block", json!({"user": user, "on": on}), format!("{} {user} on {t}", if on { "blocked" } else { "unblocked" }))
        })
        .tool("accept_request", &format!("Accept a {t} message request (a chat flagged message request) so it moves to the inbox."), obj(json!({"chat_id": chat()}), &["chat_id"]), |ctx, a| {
            inbound::set_ctx(ctx.clone());
            let chat = chat_arg(&a)?;
            simple("accept", json!({"chat": chat}), format!("accepted {}", format_chat(&chat, "")))
        })
        .tool("delete_chat", &format!("Delete a whole {t} chat from this account's inbox (not for the other people). This cannot be undone."), obj(json!({"chat_id": chat()}), &["chat_id"]), |ctx, a| {
            inbound::set_ctx(ctx.clone());
            let chat = chat_arg(&a)?;
            simple("delete_chat", json!({"chat": chat}), format!("deleted chat {}", format_chat(&chat, "")))
        })
        .tool("mute", &format!("Mute a {t} chat's notifications: seconds to mute, -1 forever, 0 unmutes."), obj(json!({
            "chat_id": chat(),
            "seconds": {"type": "integer", "description": "-1 forever, 0 unmute, else seconds"}
        }), &["chat_id", "seconds"]), |ctx, a| {
            inbound::set_ctx(ctx.clone());
            let chat = chat_arg(&a)?;
            let secs = a.get("seconds").and_then(Value::as_i64).ok_or("seconds required")?;
            let what = match secs {
                0 => "unmuted".to_string(),
                x if x < 0 => "muted".to_string(),
                x => format!("muted for {x}s"),
            };
            simple("mute", json!({"chat": chat, "seconds": secs}), format!("{what} {}", format_chat(&chat, "")))
        });
    if p == Platform::Messenger {
        server = server
            .tool("group_permissions", "Change an encrypted Messenger group's settings; only what you pass changes. only_admins_send: only admins may send. only_admins_edit_info: only admins may change the name, description and photo. only_admins_add: only admins may add people. join_approval: admins approve who joins by link. disappearing_seconds: delete messages after this time (0 off, e.g. 86400 a day). Classic (unencrypted) groups only have admins. You must be an admin.", obj(json!({
                "chat_id": chat(),
                "only_admins_send": {"type": "boolean"},
                "only_admins_edit_info": {"type": "boolean"},
                "only_admins_add": {"type": "boolean"},
                "join_approval": {"type": "boolean"},
                "disappearing_seconds": {"type": "integer", "description": "0 off, else seconds"}
            }), &["chat_id"]), |ctx, a| {
                inbound::set_ctx(ctx.clone());
                group_permissions(&a)
            })
            .tool("join_requests", "Pending requests to join an encrypted Messenger group (when join approval is on). Answer each with messenger_answer_join_request.", obj(json!({"chat_id": chat()}), &["chat_id"]), |ctx, a| {
                inbound::set_ctx(ctx.clone());
                join_requests(&a)
            })
            .tool("answer_join_request", "Approve (approve=true, default) or decline a request to join an encrypted Messenger group.", obj(json!({
                "chat_id": chat(),
                "member": {"type": "string", "description": "the person's id from messenger_join_requests"},
                "approve": {"type": "boolean", "description": "true approves (default), false declines"}
            }), &["chat_id", "member"]), |ctx, a| {
                inbound::set_ctx(ctx.clone());
                let chat = chat_arg(&a)?;
                let m = arg_str(&a, "member")?;
                let approve = arg_bool(&a, "approve", true);
                simple("answer_join", json!({"chat": chat, "member": m, "approve": approve}),
                    format!("{} {m}'s request to join {}", if approve { "approved" } else { "declined" }, format_chat(&chat, "")))
            });
    }
    if p == Platform::Messenger {
        server = server.tool("set_profile", "Set this account's Facebook bio (the short intro on the profile, at most 101 characters; an empty bio clears it). Facebook reviews name changes and the profile photo is set on the profile page, so for those open https://www.facebook.com/me in bia's Chrome (chrome MCP).", obj(json!({
            "bio": {"type": "string", "description": "the new bio; empty clears it"}
        }), &["bio"]), |ctx, a| {
            inbound::set_ctx(ctx.clone());
            set_profile(&a)
        });
    }
    if p == Platform::Instagram {
        server = server.tool("pin", "Pin (on=true) or unpin an Instagram chat, or one message in it when message_id is given.", obj(json!({
            "chat_id": chat(),
            "message_id": {"type": "string", "description": "optional: pin this message instead of the chat"},
            "on": {"type": "boolean", "description": "true pins (default), false unpins"}
        }), &["chat_id"]), |ctx, a| {
            inbound::set_ctx(ctx.clone());
            let chat = chat_arg(&a)?;
            let id = arg_str_opt(&a, "message_id").unwrap_or("").trim().to_string();
            let on = arg_bool(&a, "on", true);
            let what = if id.is_empty() { "chat".to_string() } else { format!("message {id}") };
            simple("pin", json!({"chat": chat, "id": id, "on": on}), format!("{} {what} → {}", if on { "pinned" } else { "unpinned" }, format_chat(&chat, "")))
        });
    }
    server.run_with(move |ctx| {
        inbound::set_ctx(ctx);
        supervise(p);
    });
}

/// While an account is logged in, keep bia-meta running and connected.
fn supervise(p: Platform) {
    let _ = std::thread::Builder::new().name("meta-supervisor".into()).spawn(move || loop {
        std::thread::sleep(Duration::from_secs(30));
        if state::load_cfg(p).logged_in && !helper::get().running() {
            let _ = call("restore", json!({}), LONG);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_rows() {
        assert_eq!(id_list(&json!({"m": ["a", 7]}), "m").unwrap(), vec!["a", "7"]);
        assert_eq!(id_list(&json!({"m": "x"}), "m").unwrap(), vec!["x"]);
        assert!(id_list(&json!({"m": []}), "m").is_err());
        // the platform is set once per process; tests share whichever came first
        inbound::set_platform(Platform::Instagram);
        let pn = platform().name();
        let row = chat_row(&json!({"chat": "17", "title": "Ann", "username": "ann", "group": false, "request": true}));
        assert_eq!(row, format!("{pn}:17  Ann (@ann)  [message request]"));
        let res = json!({"ids": ["m1", "m2"]});
        assert_eq!(sent_line(&res, "17", "2 file(s), "), format!("sent 2 file(s), message id m1, m2 → {pn}:17  Ann"));
        let line = history_line(&json!({"id": "m1", "ts": 0, "sender": "5", "sender_name": "Bo", "text": "hi\nthere"}));
        assert_eq!(line, "m1  1970-01-01T00:00:00Z  [Bo (5)]  hi there");
        // local encrypted messages and older server ones: one list, newest first, no repeats
        let local = vec![json!({"id": "e2", "ts": 20}), json!({"id": "c1", "ts": 10})];
        let server = vec![json!({"id": "c1", "ts": 10}), json!({"id": "c0", "ts": 5})];
        let ids: Vec<String> = merge_history(local, server, 10).iter().map(|m| s(m, "id").to_string()).collect();
        assert_eq!(ids, ["e2", "c1", "c0"]);
        assert_eq!(merge_history(vec![json!({"id": "a", "ts": 1}), json!({"id": "b", "ts": 2})], vec![], 1).len(), 1);
    }
}
