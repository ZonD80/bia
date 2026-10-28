mod audio_out;
mod calls;
mod client;
mod group_admin;
mod groups;
mod qr_win;
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
            "Start Telegram login. phone is the number with country code, for example +420123456789. The answer says where the code went (the Telegram app on another device, SMS, email, …): get the code from there and call telegram_submit_code. If Telegram first wants a login email (auth: waiting_email), call telegram_submit_email. When codes do not arrive, use telegram_login_qr instead (no code needed).",
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
            "Send the login code (from the Telegram app, SMS, a call or email), or the code that confirms a new login email after telegram_submit_email.",
            json!({"type": "object", "required": ["code"], "properties": {"code": {"type": "string"}}}),
            |ctx, a| {
                Tg::global().set_ctx(ctx.clone());
                Tg::global().submit_code(arg_str(&a, "code")?)
            },
        )
        .tool(
            "login_qr",
            "Log in to Telegram by scanning a QR code, without any code by app, SMS or email: shows the QR in a Telegram QR window on display :0 and as a PNG (/home/bia/files/telegram/qr.png). Scan it in the Telegram app of that account on a phone where it is signed in: Settings > Devices > Link Desktop Device. It refreshes until scanned (up to 5 minutes); a system|telegram line says when the login is done or needs the 2FA password (telegram_submit_password). Use it when login codes do not arrive.",
            json!({"type": "object", "properties": {}}),
            |ctx, _| {
                Tg::global().set_ctx(ctx.clone());
                Tg::global().login_qr()
            },
        )
        .tool(
            "submit_email",
            "Telegram wants a login email for this account before it sends a code (auth: waiting_email): send the email address for this account. A code goes there; pass it to telegram_submit_code, and Telegram then sends the login code itself.",
            json!({"type": "object", "required": ["email"], "properties": {"email": {"type": "string", "description": "the login email for this account"}}}),
            |ctx, a| {
                Tg::global().set_ctx(ctx.clone());
                Tg::global().submit_email(arg_str(&a, "email")?)
            },
        )
        .tool(
            "resend_code",
            "The login code did not arrive: Telegram sends it again by its next method (often SMS, then a call). The answer says where it went. If Telegram offers no other method, or it still does not arrive, use telegram_login_qr.",
            json!({"type": "object", "properties": {}}),
            |ctx, _| {
                Tg::global().set_ctx(ctx.clone());
                Tg::global().resend_code()
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
            "List recent messages in a chat, newest first. Each line is the message id (same id field as live inbound envelopes, for telegram_delete_messages / telegram_react), when it was written (RFC 3339 UTC), the sender and the text. chat_id is a chat number or @username. Pass before as the last id from the previous page to go further back.",
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
            "join_group",
            "Join a Telegram group or channel by its invite link (t.me/+…, t.me/joinchat/…) or public name (@name, t.me/name).",
            json!({"type": "object", "required": ["link"], "properties": {
                "link": {"type": "string", "description": "the invite link, or the public group's @name or t.me/name"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                Tg::global().join_group(arg_str(&a, "link")?)
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
            "group_info",
            "A Telegram group's settings: title, description, member count, what members may do, slow mode, join approval, whether new members see earlier messages, protected content, disappearing messages, pending join requests, the admins with their rights and titles, and your own rights.",
            json!({"type": "object", "required": ["chat_id"], "properties": {
                "chat_id": {"type": "string", "description": "the group's chat_id"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                Tg::global().group_info(arg_str(&a, "chat_id")?)
            },
        )
        .tool(
            "set_admin",
            "Make a member an admin of a Telegram group (admin=true, default) or take it back (admin=false). rights are what the admin may do: change_info, delete_messages, ban_users, invite_users, pin_messages, manage_call, manage_topics, post_stories, edit_stories, delete_stories, add_admins, anonymous, or all; default: everything except add_admins and anonymous. title is the admin's custom title shown in the group. Needs the add_admins right (owners have it). Basic groups only know admin yes/no.",
            json!({"type": "object", "required": ["chat_id", "member"], "properties": {
                "chat_id": {"type": "string", "description": "the group's chat_id"},
                "member": {"type": "string", "description": "@username, user id or name of a member"},
                "admin": {"type": "boolean", "description": "true makes them admin (default), false takes it away"},
                "rights": {"type": "array", "items": {"type": "string"}, "description": "admin rights by name; default the usual set"},
                "title": {"type": "string", "description": "optional custom admin title (up to 16 characters)"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                Tg::global().set_admin(
                    arg_str(&a, "chat_id")?,
                    arg_str(&a, "member")?,
                    arg_bool(&a, "admin", true),
                    &arg_str_list(&a, "rights"),
                    arg_str_opt(&a, "title").unwrap_or(""),
                )
            },
        )
        .tool(
            "group_permissions",
            "Change what every member of a Telegram group may do, and the group's switches. allow / deny take member actions: send_text, send_photos, send_videos, send_video_notes, send_voices, send_audios, send_files, send_stickers, send_gifs, send_games, send_inline, add_links, send_polls, send_reactions, invite_users, pin_messages, change_info, manage_topics, or the groups send_media, send_messages, all; the rest stays as it is (telegram_group_info shows it). slow_mode_seconds is 0, 10, 30, 60, 300, 900 or 3600; join_approval makes admins approve who joins; new_members_see_history; protect_content stops forwarding and saving; disappearing_seconds deletes messages after that time (0 off, e.g. 86400 a day). Slow mode, join approval and history work in supergroups. Needs admin rights.",
            json!({"type": "object", "required": ["chat_id"], "properties": {
                "chat_id": {"type": "string", "description": "the group's chat_id"},
                "allow": {"type": "array", "items": {"type": "string"}, "description": "member actions to allow"},
                "deny": {"type": "array", "items": {"type": "string"}, "description": "member actions to forbid"},
                "slow_mode_seconds": {"type": "integer", "description": "0 off, 10, 30, 60, 300, 900, 3600"},
                "join_approval": {"type": "boolean", "description": "true: admins approve join requests"},
                "new_members_see_history": {"type": "boolean", "description": "true: new members see earlier messages"},
                "protect_content": {"type": "boolean", "description": "true: no forwarding, copying or saving"},
                "disappearing_seconds": {"type": "integer", "description": "auto-delete messages after this many seconds; 0 off"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                let int = |k: &str| a.get(k).and_then(|v| v.as_i64()).map(|n| n as i32);
                let flag = |k: &str| a.get(k).and_then(|v| v.as_bool());
                Tg::global().group_permissions(
                    arg_str(&a, "chat_id")?,
                    &arg_str_list(&a, "allow"),
                    &arg_str_list(&a, "deny"),
                    int("slow_mode_seconds"),
                    flag("join_approval"),
                    flag("new_members_see_history"),
                    flag("protect_content"),
                    int("disappearing_seconds"),
                )
            },
        )
        .tool(
            "restrict_member",
            "Restrict one member of a Telegram supergroup: deny lists what they may no longer do (the member actions of telegram_group_permissions), deny=[\"all\"] bans them from the group, deny=[] lifts every restriction and ban. seconds limits it in time (0 = until lifted). Needs the ban_users right. (telegram_ban is different: it only stops their messages reaching you.)",
            json!({"type": "object", "required": ["chat_id", "member", "deny"], "properties": {
                "chat_id": {"type": "string", "description": "the group's chat_id"},
                "member": {"type": "string", "description": "@username, user id or name"},
                "deny": {"type": "array", "items": {"type": "string"}, "description": "actions to forbid; [\"all\"] bans; [] lifts"},
                "seconds": {"type": "integer", "description": "how long; 0 or omitted = until lifted"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                Tg::global().restrict_member(
                    arg_str(&a, "chat_id")?,
                    arg_str(&a, "member")?,
                    &arg_str_list(&a, "deny"),
                    a.get("seconds").and_then(|v| v.as_i64()).unwrap_or(0),
                )
            },
        )
        .tool(
            "set_group_info",
            "Change a Telegram group's title, description (empty clears it) and/or photo (an image under /home/bia/files). Only what you pass changes. Needs the change_info right.",
            json!({"type": "object", "required": ["chat_id"], "properties": {
                "chat_id": {"type": "string", "description": "the group's chat_id"},
                "title": {"type": "string", "description": "new title"},
                "description": {"type": "string", "description": "new description; empty clears it"},
                "photo": {"type": "string", "description": "image path under /home/bia/files"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                Tg::global().set_group_info(
                    arg_str(&a, "chat_id")?,
                    arg_str_opt(&a, "title"),
                    arg_str_opt(&a, "description"),
                    arg_str_opt(&a, "photo"),
                )
            },
        )
        .tool(
            "join_requests",
            "Pending requests to join a Telegram group (when join approval is on): who asked and when. Answer each with telegram_answer_join_request.",
            json!({"type": "object", "required": ["chat_id"], "properties": {
                "chat_id": {"type": "string", "description": "the group's chat_id"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                Tg::global().join_requests(arg_str(&a, "chat_id")?)
            },
        )
        .tool(
            "answer_join_request",
            "Approve (approve=true, default) or decline a request to join a Telegram group.",
            json!({"type": "object", "required": ["chat_id", "member"], "properties": {
                "chat_id": {"type": "string", "description": "the group's chat_id"},
                "member": {"type": "string", "description": "the person who asked (@username or user id from telegram_join_requests)"},
                "approve": {"type": "boolean", "description": "true approves (default), false declines"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                Tg::global().answer_join_request(arg_str(&a, "chat_id")?, arg_str(&a, "member")?, arg_bool(&a, "approve", true))
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
