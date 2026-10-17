mod audio_out;
mod calls;
mod client;
mod groups;
mod speech;
mod tts;

use bia_mcp::{arg_bool, arg_str, arg_str_list, arg_str_opt, arg_u32_opt, Ctx, Server};
use calls::Calls;
use client::{Cfg, Tg, API_ID};
use serde_json::json;

fn parse_i64(v: &serde_json::Value) -> Result<i64, String> {
    if let Some(s) = v.as_str() {
        let s = s.trim();
        if s.is_empty() {
            return Err("empty message id".into());
        }
        return s
            .parse::<i64>()
            .map_err(|_| format!("bad message id: {s}"));
    }
    if let Some(n) = v.as_i64() {
        return Ok(n);
    }
    if let Some(n) = v.as_u64() {
        return i64::try_from(n).map_err(|_| format!("bad message id: {n}"));
    }
    Err(format!("bad message id: {v}"))
}

fn arg_i64_list(args: &serde_json::Value, key: &str) -> Result<Vec<i64>, String> {
    let Some(v) = args.get(key) else {
        return Err(format!("missing argument: {key}"));
    };
    let items = match v {
        serde_json::Value::Array(a) => a.clone(),
        other => vec![other.clone()],
    };
    let mut out = Vec::new();
    for item in &items {
        let n = parse_i64(item)?;
        if n == 0 {
            return Err("message id 0 is not valid".into());
        }
        out.push(n);
    }
    if out.is_empty() {
        return Err("message_ids is empty".into());
    }
    Ok(out)
}

fn arg_i64_opt(args: &serde_json::Value, key: &str) -> Result<i64, String> {
    match args.get(key) {
        None | Some(serde_json::Value::Null) => Ok(0),
        Some(v) if v.as_str().is_some_and(|s| s.trim().is_empty()) => Ok(0),
        Some(v) => parse_i64(v),
    }
}

fn side(revoke: bool) -> &'static str {
    if revoke {
        "for everyone"
    } else {
        "on this account"
    }
}

fn require_login(ctx: &Ctx) -> Result<(), String> {
    Tg::global().set_ctx(ctx.clone());
    Tg::global().require_ready()?;
    Ok(())
}

struct NtgLog;

impl log::Log for NtgLog {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.target().starts_with("ntgcalls") && metadata.level() <= log::Level::Info
    }
    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        // Cap volume: keep channel/ssrc/incoming lines, drop noisy webrtc chatter.
        let msg = record.args().to_string();
        let interesting = msg.contains("incoming")
            || msg.contains("Incoming")
            || msg.contains("ssrc")
            || msg.contains("SSRC")
            || msg.contains("PLAYBACK")
            || msg.contains("Playback")
            || msg.contains("CAPTURE")
            || msg.contains("P2P")
            || msg.contains("audio")
            || msg.contains("Audio")
            || msg.contains("Reconfigur")
            || msg.contains("External")
            || record.level() <= log::Level::Warn;
        if interesting {
            Tg::global().log(format!("ntg: {}", msg));
        }
    }
    fn flush(&self) {}
}

fn main() {
    // Forward ntgcalls/WebRTC logs into the telegram MCP console (stt:/ntg:).
    let _ = log::set_logger(&NtgLog).map(|()| log::set_max_level(log::LevelFilter::Info));
    tgcalls::init_logging();

    let server = Server::new("telegram", env!("CARGO_PKG_VERSION"))
        .tool(
            "login",
            "Start Telegram login. phone is the number with country code, for example +420123456789. Then ask the user for the code and call telegram_submit_code.",
            json!({"type": "object", "required": ["phone"], "properties": {
                "phone": {"type": "string", "description": "phone number with country code, for example +420123456789"}
            }}),
            |ctx, a| {
                Tg::global().set_ctx(ctx.clone());
                Tg::global().login(arg_str(&a, "phone")?)
            },
        )
        .tool(
            "submit_code",
            "Send the login code the user read from their phone or Telegram app.",
            json!({"type": "object", "required": ["code"], "properties": {"code": {"type": "string"}}}),
            |ctx, a| {
                Tg::global().set_ctx(ctx.clone());
                Tg::global().submit_code(arg_str(&a, "code")?)
            },
        )
        .tool(
            "submit_password",
            "Send the Telegram password. Call this only if submit_code says a password is required.",
            json!({"type": "object", "required": ["password"], "properties": {"password": {"type": "string"}}}),
            |ctx, a| {
                Tg::global().set_ctx(ctx.clone());
                Tg::global().submit_password(arg_str(&a, "password")?)
            },
        )
        .tool(
            "status",
            "Are you logged into Telegram, and which phone number is saved.",
            json!({"type": "object", "properties": {}}),
            |ctx, _| {
                Tg::global().set_ctx(ctx.clone());
                let _ = Tg::global().ensure_connected();
                Tg::global().heal_if_needed();
                let c = Tg::load_cfg();
                let phase = Tg::global().phase();
                let banned = Tg::global().banned_ids();
                let banned = if banned.is_empty() {
                    String::new()
                } else {
                    format!(
                        "  banned={}",
                        banned
                            .iter()
                            .map(|id| format!("telegram:{id}"))
                            .collect::<Vec<_>>()
                            .join(",")
                    )
                };
                Ok(format!(
                    "stack=ferogram+tgcalls  auth={}  api_id={}  phone={}{banned}",
                    phase,
                    API_ID,
                    if c.phone.is_empty() {
                        "(none)".into()
                    } else {
                        c.phone
                    }
                ))
            },
        )
        .tool(
            "logout",
            "Log out of Telegram on this computer. Call telegram_login to sign in again.",
            json!({"type": "object", "properties": {}}),
            |ctx, _| {
                Tg::global().set_ctx(ctx.clone());
                Tg::global().logout()
            },
        )
        .tool(
            "set_profile",
            "Update this Telegram account's display name and/or bio (About). Only the fields you pass are changed. first_name and last_name together set the name; bio is the profile About text (Telegram limit ~70 characters for users).",
            json!({"type": "object", "properties": {
                "first_name": {"type": "string", "description": "new first name. Pass with last_name to change the display name."},
                "last_name": {"type": "string", "description": "new last name. Empty string clears it."},
                "bio": {"type": "string", "description": "new About / bio text. Empty string clears it."}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                let first = arg_str_opt(&a, "first_name");
                let last = arg_str_opt(&a, "last_name");
                let bio = arg_str_opt(&a, "bio");
                Tg::global().set_profile(first, last, bio)
            },
        )
        .tool(
            "set_photo",
            "Set this Telegram account's profile photo. path is an image under /home/bia/files (jpg/png/webp).",
            json!({"type": "object", "required": ["path"], "properties": {
                "path": {"type": "string", "description": "image path under /home/bia/files"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                Tg::global().set_photo(arg_str(&a, "path")?)
            },
        )
        .tool(
            "send",
            "Send a Telegram message. chat_id is the number from telegram_search, or an @username. text is the message. files are paths under /home/bia/files. Pictures are sent as photos. Other files are sent as documents.",
            json!({"type": "object", "required": ["chat_id"], "properties": {
                "chat_id": {"type": "string", "description": "chat number from telegram_search, or @username"},
                "text": {"type": "string", "description": "the message. Optional if you send files."},
                "files": {"type": "array", "items": {"type": "string"}, "description": "file paths under /home/bia/files"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                let chat = arg_str(&a, "chat_id")?;
                let text = arg_str_opt(&a, "text").unwrap_or("");
                let files = arg_str_list(&a, "files");
                if text.is_empty() && files.is_empty() {
                    return Err("text or files required".into());
                }
                Tg::global().send_cancellable(chat, text, &files, &|| ctx.cancelled())
            },
        )
        .tool(
            "send_location",
            "Send a location (map pin). With name or address it is sent as a place card. Coordinates in decimal degrees. Received locations arrive as [location] lines with a map link.",
            json!({"type": "object", "required": ["chat_id", "latitude", "longitude"], "properties": {
                "chat_id": {"type": "string", "description": "chat number from telegram_search, or @username"},
                "latitude": {"type": "number", "description": "-90…90, e.g. 52.520008"},
                "longitude": {"type": "number", "description": "-180…180, e.g. 13.404954"},
                "name": {"type": "string", "description": "optional place name, e.g. Grand Hotel"},
                "address": {"type": "string", "description": "optional street address"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                let num = |k: &str| a.get(k).and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))).ok_or(format!("{k} is required (a number)"));
                Tg::global().send_location(
                    arg_str(&a, "chat_id")?,
                    num("latitude")?,
                    num("longitude")?,
                    arg_str_opt(&a, "name").unwrap_or(""),
                    arg_str_opt(&a, "address").unwrap_or(""),
                )
            },
        )
        .tool(
            "live_location",
            "Where someone sharing a live location in this chat is now: latest position, how old it is, whether they still share, and a map link. Updates arrive silently (no message per move); you hear when sharing ends.",
            json!({"type": "object", "required": ["chat_id"], "properties": {
                "chat_id": {"type": "string", "description": "chat number from telegram_search, or @username"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                Tg::global().live_location(arg_str(&a, "chat_id")?)
            },
        )
        .tool(
            "send_contact",
            "Send a contact card (name and phone number, optional e-mail). Received contacts arrive as [contact] lines.",
            json!({"type": "object", "required": ["chat_id", "name", "phone"], "properties": {
                "chat_id": {"type": "string", "description": "chat number from telegram_search, or @username"},
                "name": {"type": "string", "description": "the person's name"},
                "phone": {"type": "string", "description": "phone number in international format, e.g. +49 170 1234567"},
                "email": {"type": "string", "description": "optional e-mail address"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                Tg::global().send_contact(
                    arg_str(&a, "chat_id")?,
                    arg_str(&a, "name")?,
                    arg_str(&a, "phone")?,
                    arg_str_opt(&a, "email").unwrap_or(""),
                )
            },
        )
        .tool(
            "send_voice",
            "Send a voice message: text is spoken with the voice of lang and sent as a Telegram voice note. Only when the user asks for a voice reply (otherwise answer in text). lang is the language of the text (defaults to their preferred language from the addressbook); mark other-language parts inline: <lang=en>Grand Hotel</lang>. A language without a voice fails: lang_install it first (lang MCP).",
            json!({"type": "object", "required": ["chat_id", "text"], "properties": {
                "chat_id": {"type": "string", "description": "chat number from telegram_search, or @username"},
                "text": {"type": "string", "description": "what to say"},
                "lang": {"type": "string", "description": "language of the text: en, ru, es-MX, …; defaults to the person's preferred language (addressbook)"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                Tg::global().send_voice(arg_str(&a, "chat_id")?, arg_str(&a, "text")?, arg_str_opt(&a, "lang").unwrap_or(""))
            },
        )
        .tool(
            "edit",
            "Replace the text (or caption) of a message this account sent. message_id comes from the telegram_send result or telegram_history.",
            json!({"type": "object", "required": ["chat_id", "message_id", "text"], "properties": {
                "chat_id": {"type": "string", "description": "chat number from telegram_search, or @username"},
                "message_id": {"type": "string", "description": "id of your message to replace"},
                "text": {"type": "string", "description": "the new text"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                let msg_id = match a.get("message_id") {
                    Some(v) => parse_i64(v)?,
                    None => return Err("missing argument: message_id".into()),
                };
                Tg::global().edit(arg_str(&a, "chat_id")?, msg_id, arg_str(&a, "text")?)
            },
        )
        .tool(
            "search",
            "Find chats by name or @username. Returns lines you can pass to telegram_send as chat_id.",
            json!({"type": "object", "required": ["query"], "properties": {
                "query": {"type": "string", "description": "a name or @username"},
                "limit": {"type": "integer", "description": "how many results. Default 20."}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                let query = arg_str(&a, "query")?;
                let limit = arg_u32_opt(&a, "limit").unwrap_or(20) as usize;
                let rows = Tg::global().search(query, limit)?;
                if rows.is_empty() {
                    return Ok(format!("(no Telegram chats matching {query})"));
                }
                Ok(rows
                    .into_iter()
                    .map(|(id, title, un)| Tg::format_chat(id, &title, &un))
                    .collect::<Vec<_>>()
                    .join("\n"))
            },
        )
        .tool(
            "resolve",
            "Turn one @username or chat number into the id telegram_send needs.",
            json!({"type": "object", "required": ["username"], "properties": {
                "username": {"type": "string", "description": "@username or a chat number"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                let (id, title, un) = Tg::global().resolve_target(arg_str(&a, "username")?)?;
                Ok(Tg::format_chat(id, &title, &un))
            },
        )
        .tool(
            "typing",
            "Show typing in a chat. You do not need to call this. It happens while you are replying.",
            json!({"type": "object", "required": ["chat_id"], "properties": {
                "chat_id": {"type": "string", "description": "chat number"},
                "on": {"type": "boolean", "description": "true shows typing. false stops it. Default true."}
            }}),
            |ctx, a| {
                Tg::global().set_ctx(ctx.clone());
                let chat: i64 = arg_str(&a, "chat_id")?
                    .parse()
                    .map_err(|_| "chat_id must be numeric".to_string())?;
                Tg::global().set_typing(chat, arg_bool(&a, "on", true));
                Ok(String::new())
            },
        )
        .tool(
            "chats",
            "List chats you already know. Each line has a chat number and a title.",
            json!({"type": "object", "properties": {}}),
            |ctx, _| {
                Tg::global().set_ctx(ctx.clone());
                match Tg::global().require_ready() {
                    Err(e) => Ok(format!("(not logged in: {e})")),
                    Ok(_) => {
                        let rows = Tg::global().chats()?;
                        if rows.is_empty() {
                            return Ok("(no chats yet; login first)".into());
                        }
                        Ok(rows
                            .into_iter()
                            .map(|(id, t, banned)| {
                                let mark = if banned { "  banned" } else { "" };
                                format!("telegram:{id}  {t}{mark}")
                            })
                            .collect::<Vec<_>>()
                            .join("\n"))
                    }
                }
            },
        )
        .tool(
            "history",
            "List recent messages in a chat, newest first. Each line starts with the message id (same id field as live inbound envelopes) for telegram_delete_messages / telegram_react. chat_id is a chat number or @username. Pass before as the last id from the previous page to go further back.",
            json!({"type": "object", "required": ["chat_id"], "properties": {
                "chat_id": {"type": "string", "description": "chat number from telegram_search, or @username"},
                "limit": {"type": "integer", "description": "how many messages. Default 30, at most 100."},
                "before": {"type": "string", "description": "message id. Return messages older than this one."}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                let chat = arg_str(&a, "chat_id")?;
                let limit = arg_u32_opt(&a, "limit").unwrap_or(30) as usize;
                let before = arg_i64_opt(&a, "before")?;
                let (chat_id, title, username, lines) = Tg::global().history(chat, limit, before)?;
                let head = Tg::format_chat(chat_id, &title, &username);
                if lines.is_empty() {
                    Ok(format!("{head}\n(no messages)"))
                } else {
                    Ok(format!("{head}\n{}", lines.join("\n")))
                }
            },
        )
        .tool(
            "react",
            "React to a Telegram message with an emoji. message_id comes from telegram_history. Pass an empty emoji to remove this account's reaction.",
            json!({"type": "object", "required": ["chat_id", "message_id"], "properties": {
                "chat_id": {"type": "string", "description": "chat number from telegram_search, or @username"},
                "message_id": {"type": "string", "description": "message id from telegram_history"},
                "emoji": {"type": "string", "description": "reaction emoji, e.g. 👍. Empty string removes your reaction."}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                let chat = arg_str(&a, "chat_id")?;
                let msg_id = match a.get("message_id") {
                    Some(v) => parse_i64(v)?,
                    None => return Err("missing argument: message_id".into()),
                };
                let emoji = arg_str_opt(&a, "emoji").unwrap_or("");
                let (chat_id, title, username) = Tg::global().react(chat, msg_id, emoji)?;
                let who = Tg::format_chat(chat_id, &title, &username);
                if emoji.trim().is_empty() {
                    Ok(format!("removed reaction on message {msg_id} → {who}"))
                } else {
                    Ok(format!("reacted {emoji} on message {msg_id} → {who}"))
                }
            },
        )
        .tool(
            "delete_messages",
            "Delete specific messages. message_ids come from telegram_history. By default (revoke true) they are deleted for everyone in the chat; revoke false deletes them only on this account. This cannot be undone.",
            json!({"type": "object", "required": ["chat_id", "message_ids"], "properties": {
                "chat_id": {"type": "string", "description": "chat number from telegram_search, or @username"},
                "message_ids": {"type": "array", "items": {"type": "string"}, "description": "message ids from telegram_history"},
                "revoke": {"type": "boolean", "description": "true (default) deletes for everyone; false only on this account."}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                let chat = arg_str(&a, "chat_id")?;
                let ids = arg_i64_list(&a, "message_ids")?;
                let revoke = arg_bool(&a, "revoke", true);
                let (n, chat_id, title, username) =
                    Tg::global().delete_messages(chat, &ids, revoke)?;
                Ok(format!(
                    "deleted {n} message(s) {} → {}",
                    side(revoke),
                    Tg::format_chat(chat_id, &title, &username)
                ))
            },
        )
        .tool(
            "delete_history",
            "Delete the whole message history of a chat with a person or a group. chat_id is a chat number or @username. By default (revoke true) the history is deleted for the other person too; revoke false clears it only on this account. remove true also drops the chat from the chat list. This cannot be undone.",
            json!({"type": "object", "required": ["chat_id"], "properties": {
                "chat_id": {"type": "string", "description": "chat number from telegram_search, or @username"},
                "revoke": {"type": "boolean", "description": "true (default) deletes the history for the other person too; false only on this account."},
                "remove": {"type": "boolean", "description": "true also removes the chat from the chat list. Default false."}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                let chat = arg_str(&a, "chat_id")?;
                let revoke = arg_bool(&a, "revoke", true);
                let remove = arg_bool(&a, "remove", false);
                let (chat_id, title, username) =
                    Tg::global().delete_history(chat, revoke, remove)?;
                let listed = if remove {
                    ", and removed the chat from the list"
                } else {
                    ""
                };
                Ok(format!(
                    "cleared history {}{listed} → {}",
                    side(revoke),
                    Tg::format_chat(chat_id, &title, &username)
                ))
            },
        )
        .tool(
            "create_group",
            "Create a Telegram group (a supergroup) with this account as its owner and add people to it. members are @usernames, chat numbers from telegram_search, or phone numbers of your contacts. Returns the group's chat_id (use it with telegram_send) and an invite link; people whose privacy settings block adding them are listed — send them the link.",
            json!({"type": "object", "required": ["title"], "properties": {
                "title": {"type": "string", "description": "group name"},
                "members": {"type": "array", "items": {"type": "string"}, "description": "people to add: @username, chat number or contact phone"},
                "about": {"type": "string", "description": "optional description"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                Tg::global().create_group(
                    arg_str(&a, "title")?,
                    &arg_str_list(&a, "members"),
                    arg_str_opt(&a, "about").unwrap_or_default(),
                )
            },
        )
        .tool(
            "add_members",
            "Add people to a Telegram group (chat_id from telegram_create_group or telegram_chats). Those whose privacy settings block it are listed with an invite link to send them.",
            json!({"type": "object", "required": ["chat_id", "members"], "properties": {
                "chat_id": {"type": "string", "description": "the group's chat_id"},
                "members": {"type": "array", "items": {"type": "string"}, "description": "@username, chat number or contact phone"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                Tg::global().add_members(arg_str(&a, "chat_id")?, &arg_str_list(&a, "members"))
            },
        )
        .tool(
            "remove_member",
            "Remove a person from a Telegram group you administer. They can be added again later.",
            json!({"type": "object", "required": ["chat_id", "member"], "properties": {
                "chat_id": {"type": "string", "description": "the group's chat_id"},
                "member": {"type": "string", "description": "@username or chat number of the person"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                Tg::global().remove_member(arg_str(&a, "chat_id")?, arg_str(&a, "member")?)
            },
        )
        .tool(
            "leave_group",
            "Leave a Telegram group.",
            json!({"type": "object", "required": ["chat_id"], "properties": {
                "chat_id": {"type": "string", "description": "the group's chat_id"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                Tg::global().leave_group(arg_str(&a, "chat_id")?)
            },
        )
        .tool(
            "group_invite_link",
            "An invite link to a Telegram group you administer (anyone with it can join).",
            json!({"type": "object", "required": ["chat_id"], "properties": {
                "chat_id": {"type": "string", "description": "the group's chat_id"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                Tg::global().group_invite_link(arg_str(&a, "chat_id")?)
            },
        )
        .tool(
            "ban",
            "Ban a Telegram chat. Messages from that chat are not delivered to you. Each one is answered with I've banned you. Incoming calls from that chat are auto-rejected. chat_id is a chat number, telegram:<id>, or @username.",
            json!({"type": "object", "required": ["chat_id"], "properties": {
                "chat_id": {"type": "string", "description": "chat number, telegram:<id>, or @username"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                let (chat_id, title, username) =
                    Tg::global().resolve_target(arg_str(&a, "chat_id")?)?;
                let changed = Tg::global().set_banned(chat_id, true)?;
                let who = Tg::format_chat(chat_id, &title, &username);
                if changed {
                    Ok(format!("banned {who}. Their messages are not delivered. They are answered with I've banned you. Their calls are auto-rejected."))
                } else {
                    Ok(format!("already banned {who}"))
                }
            },
        )
        .tool(
            "unban",
            "Unban a Telegram chat so its messages are delivered again. chat_id is a chat number, telegram:<id>, or @username.",
            json!({"type": "object", "required": ["chat_id"], "properties": {
                "chat_id": {"type": "string", "description": "chat number, telegram:<id>, or @username"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                let (chat_id, title, username) =
                    Tg::global().resolve_target(arg_str(&a, "chat_id")?)?;
                let changed = Tg::global().set_banned(chat_id, false)?;
                let who = Tg::format_chat(chat_id, &title, &username);
                if changed {
                    Ok(format!("unbanned {who}. Their messages are delivered again."))
                } else {
                    Ok(format!("not banned {who}"))
                }
            },
        )
        .tool(
            "call_status",
            "Show pending/active Telegram voice calls and whether local STT/TTS binaries are available.",
            json!({"type": "object", "properties": {}}),
            |ctx, _| {
                require_login(ctx)?;
                Ok(Calls::global().status_cmd())
            },
        )
        .tool(
            "call_start",
            "Start a private Telegram voice call to a user. chat_id is a private chat number, telegram:<id>, or @username. Wait for [call ready] or [call ended] inbound messages. When the conversation is over, end it yourself with call_hangup (do not wait for the other side to disconnect).",
            json!({"type": "object", "required": ["chat_id"], "properties": {
                "chat_id": {"type": "string", "description": "private chat number, telegram:<id>, or @username"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                Calls::global().start(arg_str(&a, "chat_id")?)
            },
        )
        .tool(
            "call_accept",
            "Accept a pending incoming Telegram voice call. chat_id is the telegram:<id> from the [incoming call] message.",
            json!({"type": "object", "required": ["chat_id"], "properties": {
                "chat_id": {"type": "string", "description": "chat number from [incoming call], or telegram:<id>"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                Calls::global().accept(arg_str(&a, "chat_id")?)
            },
        )
        .tool(
            "call_hangup",
            "End the active Telegram voice call (or reject a ringing incoming call). Use this when the conversation is done, the user says bye/goodbye/hang up, or you need to leave — you can and should hang up yourself; do not wait for them to disconnect. chat_id optional if only one call exists.",
            json!({"type": "object", "properties": {
                "chat_id": {"type": "string", "description": "chat number, telegram:<id>, or @username. Optional if only one call."}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                Calls::global().hangup(arg_str_opt(&a, "chat_id"))
            },
        )
        .tool(
            "call_reject",
            "Reject a pending incoming Telegram voice call (same as hangup on a ringing call).",
            json!({"type": "object", "properties": {
                "chat_id": {"type": "string", "description": "chat number from [incoming call]. Optional if only one pending call."}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                Calls::global().hangup(arg_str_opt(&a, "chat_id"))
            },
        )
        .tool(
            "call_join_group",
            "Join or create a group voice chat in a Telegram group/channel. chat_id is the group. Speaker identity in group calls uses per-participant SSRC when media is available.",
            json!({"type": "object", "required": ["chat_id"], "properties": {
                "chat_id": {"type": "string", "description": "group or channel chat number, telegram:<id>, or @username"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                Calls::global().join_group(arg_str(&a, "chat_id")?)
            },
        )
        .tool(
            "call_say",
            "Speak text into the active Telegram call via local TTS. chat_id is the call's telegram chat. lang is the language of the text (defaults to the caller's preferred language from the addressbook); mark other-language parts inline: <lang=fr>Le Petit Bistro</lang>. A language without a voice fails at once: lang_install it (lang MCP) or speak an installed one. Keep it short. Prefer this (or say) while a call is active. Do not call again until you receive [you finished speaking] for the previous utterance — a new call_say cancels the current one ([voice cancelled]). call_hangup while you are still speaking waits until that has played. [speech <lang>] and chat messages may arrive while you are still speaking; wait for [you finished speaking] before speaking again unless you intentionally interrupt. After a farewell, wait for [you finished speaking], then call_hangup.",
            json!({"type": "object", "required": ["chat_id", "text"], "properties": {
                "chat_id": {"type": "string", "description": "chat number of the active call"},
                "text": {"type": "string", "description": "short text to speak"},
                "lang": {"type": "string", "description": "language of the text: en, ru, es-MX, …; defaults to the person's preferred language (addressbook)"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                Calls::global().say(arg_str(&a, "chat_id")?, arg_str(&a, "text")?, arg_str_opt(&a, "lang").unwrap_or(""))
            },
        )
        .tool(
            "restore",
            "Reconnect the saved Telegram login. This runs by itself at boot. Call it only if status says you are logged out but you were logged in before.",
            json!({"type": "object", "properties": {}}),
            |ctx, _| {
                Tg::global().set_ctx(ctx.clone());
                Tg::global().restore()
            },
        );

    let _ = Cfg::default(); // keep type linked for serde
    server.run_with(|ctx| {
        Tg::global().set_ctx(ctx);
    });
}

#[cfg(test)]
mod tests {
    use super::{arg_i64_list, arg_i64_opt, parse_i64};
    use serde_json::json;

    #[test]
    fn message_ids_accept_strings_and_numbers() {
        let a = json!({"message_ids": ["42", 7]});
        assert_eq!(arg_i64_list(&a, "message_ids").unwrap(), vec![42, 7]);
        assert_eq!(
            arg_i64_list(&json!({"message_ids": "9"}), "message_ids").unwrap(),
            vec![9]
        );
        assert!(arg_i64_list(&json!({"message_ids": []}), "message_ids").is_err());
        assert!(arg_i64_list(&json!({"message_ids": ["0"]}), "message_ids").is_err());
        assert!(parse_i64(&json!("nope")).is_err());
        assert_eq!(arg_i64_opt(&json!({}), "before").unwrap(), 0);
        assert_eq!(arg_i64_opt(&json!({"before": "99"}), "before").unwrap(), 99);
    }
}
