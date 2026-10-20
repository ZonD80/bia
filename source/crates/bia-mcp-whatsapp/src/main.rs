mod audio_out;
mod calls;
mod client;
mod groups;
mod qr_win;
mod tts;

use bia_mcp::{arg_bool, arg_str, arg_str_list, arg_str_opt, arg_u32_opt, Ctx, Server};
use calls::Calls;
use client::Wa;
use serde_json::json;

/// Message ids are opaque strings on WhatsApp (e.g. `3EB0A1B2C3D4`).
fn arg_id_list(args: &serde_json::Value, key: &str) -> Result<Vec<String>, String> {
    let Some(v) = args.get(key) else {
        return Err(format!("missing argument: {key}"));
    };
    let items = match v {
        serde_json::Value::Array(a) => a.clone(),
        other => vec![other.clone()],
    };
    let mut out = Vec::new();
    for item in &items {
        let id = match item {
            serde_json::Value::String(s) => s.trim().to_string(),
            serde_json::Value::Number(n) => n.to_string(),
            other => return Err(format!("bad message id: {other}")),
        };
        if id.is_empty() {
            return Err("empty message id".into());
        }
        out.push(id);
    }
    if out.is_empty() {
        return Err("message_ids is empty".into());
    }
    Ok(out)
}

fn side(revoke: bool) -> &'static str {
    if revoke {
        "for everyone"
    } else {
        "on this account"
    }
}

fn require_login(ctx: &Ctx) -> Result<(), String> {
    Wa::global().set_ctx(ctx.clone());
    Wa::global().require_ready()?;
    Ok(())
}

/// whatsapp-rust logs through the `log` crate; surface warnings/errors on the
/// agent console (rate limited so a flapping link cannot flood it).
struct WaLog;

impl log::Log for WaLog {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Warn
    }
    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        use std::sync::Mutex;
        use std::time::{Duration, Instant};
        static WINDOW: Mutex<Option<(Instant, u64)>> = Mutex::new(None);
        {
            let mut w = WINDOW.lock().unwrap();
            let now = Instant::now();
            let (start, n) = w.get_or_insert((now, 0));
            if now.duration_since(*start) >= Duration::from_secs(10) {
                *start = now;
                *n = 0;
            }
            *n += 1;
            if *n > 20 {
                return;
            }
        }
        Wa::global().log(format!("wa: {} {}", record.target(), record.args()));
    }
    fn flush(&self) {}
}

fn main() {
    let _ = log::set_logger(&WaLog).map(|()| log::set_max_level(log::LevelFilter::Warn));

    let server = Server::new("whatsapp", env!("CARGO_PKG_VERSION"))
        .tool(
            "login",
            "Start WhatsApp login (QR only; there is no phone number, SMS code or password). Writes a temporary scannable PNG at /home/bia/files/whatsapp/qr.png (mention that path so the human can open or you can attach it; it is deleted when auth completes), opens it in a WhatsApp QR window on display :0, and sends a system notice. The human scans it in WhatsApp > Settings > Linked devices > Link a device. QR codes rotate about every 20 seconds. Afterwards call whatsapp_status until auth=ready.",
            json!({"type": "object", "properties": {}}),
            |ctx, _| {
                Wa::global().set_ctx(ctx.clone());
                Wa::global().login()
            },
        )
        .tool(
            "submit_code",
            "Not used. WhatsApp links by QR scan only; call whatsapp_login instead.",
            json!({"type": "object", "properties": {"code": {"type": "string"}}}),
            |ctx, _| {
                Wa::global().set_ctx(ctx.clone());
                Err("WhatsApp login is QR-only: there is no code to submit. Call whatsapp_login, then scan the QR with WhatsApp > Settings > Linked devices > Link a device.".into())
            },
        )
        .tool(
            "submit_password",
            "Not used. WhatsApp has no password step; call whatsapp_login and scan the QR.",
            json!({"type": "object", "properties": {"password": {"type": "string"}}}),
            |ctx, _| {
                Wa::global().set_ctx(ctx.clone());
                Err("WhatsApp login is QR-only: there is no password. Call whatsapp_login, then scan the QR with WhatsApp > Settings > Linked devices > Link a device.".into())
            },
        )
        .tool(
            "status",
            "Are you logged into WhatsApp, and which phone number is linked.",
            json!({"type": "object", "properties": {}}),
            |ctx, _| {
                Wa::global().set_ctx(ctx.clone());
                Wa::global().heal_if_needed();
                let c = Wa::load_cfg();
                let phase = Wa::global().phase();
                let banned = Wa::global().banned_ids();
                let banned = if banned.is_empty() {
                    String::new()
                } else {
                    format!(
                        "  banned={}",
                        banned
                            .iter()
                            .map(|id| format!("whatsapp:{id}"))
                            .collect::<Vec<_>>()
                            .join(",")
                    )
                };
                Ok(format!(
                    "stack=whatsapp-rust+voip  auth={}  phone={}{banned}",
                    phase,
                    if c.phone.is_empty() {
                        "(none)".into()
                    } else {
                        format!("+{}", c.phone)
                    }
                ))
            },
        )
        .tool(
            "logout",
            "Unlink WhatsApp on this computer (removes the linked device and the local session). Call whatsapp_login to link again.",
            json!({"type": "object", "properties": {}}),
            |ctx, _| {
                Wa::global().set_ctx(ctx.clone());
                Wa::global().logout()
            },
        )
        .tool(
            "set_profile",
            "Update this WhatsApp account's display name and/or About text. Only the fields you pass are changed. first_name and last_name together form the display name (push name); bio is the About text.",
            json!({"type": "object", "properties": {
                "first_name": {"type": "string", "description": "new first name of the display name"},
                "last_name": {"type": "string", "description": "new last name of the display name. Empty string clears it."},
                "bio": {"type": "string", "description": "new About text"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                Wa::global().set_profile(
                    arg_str_opt(&a, "first_name"),
                    arg_str_opt(&a, "last_name"),
                    arg_str_opt(&a, "bio"),
                )
            },
        )
        .tool(
            "set_photo",
            "Set this WhatsApp account's profile photo. path is an image under /home/bia/files (JPEG; other formats are converted when ffmpeg is installed).",
            json!({"type": "object", "required": ["path"], "properties": {
                "path": {"type": "string", "description": "image path under /home/bia/files"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                Wa::global().set_photo(arg_str(&a, "path")?)
            },
        )
        .tool(
            "send_voice",
            "Send a voice message: text is spoken with the voice of lang and sent as a WhatsApp voice note (push-to-talk). Only when the user asks for a voice reply (otherwise answer in text). lang is the language of the text (defaults to their preferred language from the addressbook); mark other-language parts inline: <lang=en>Grand Hotel</lang>. A language without a voice fails: lang_install it first (lang MCP).",
            json!({"type": "object", "required": ["chat_id", "text"], "properties": {
                "chat_id": {"type": "string", "description": "phone digits, whatsapp:<phone>, or a group id from whatsapp_search"},
                "text": {"type": "string", "description": "what to say"},
                "lang": {"type": "string", "description": "language of the text: en, ru, es-MX, …; defaults to the person's preferred language (addressbook)"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                Wa::global().send_voice(arg_str(&a, "chat_id")?, arg_str(&a, "text")?, arg_str_opt(&a, "lang").unwrap_or(""))
            },
        )
        .tool(
            "send",
            "Send a WhatsApp message. chat_id is a phone number with country code (digits, e.g. 15551234567) or an id from whatsapp_search/whatsapp_chats (groups look like 1203630...@g.us). text is the message. files are paths under /home/bia/files. Pictures (jpg/png/webp) are sent as images. Other files are sent as documents.",
            json!({"type": "object", "required": ["chat_id"], "properties": {
                "chat_id": {"type": "string", "description": "phone digits, whatsapp:<phone>, or a group id from whatsapp_search"},
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
                Wa::global().send_cancellable(chat, text, &files, &|| ctx.cancelled())
            },
        )
        .tool(
            "send_location",
            "Send a location (map pin). With name or address it shows as a place. Coordinates in decimal degrees. Received locations arrive as [location] lines with a map link.",
            json!({"type": "object", "required": ["chat_id", "latitude", "longitude"], "properties": {
                "chat_id": {"type": "string", "description": "phone digits, whatsapp:<phone>, or a group id from whatsapp_search"},
                "latitude": {"type": "number", "description": "-90…90, e.g. 52.520008"},
                "longitude": {"type": "number", "description": "-180…180, e.g. 13.404954"},
                "name": {"type": "string", "description": "optional place name, e.g. Grand Hotel"},
                "address": {"type": "string", "description": "optional street address"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                let num = |k: &str| a.get(k).and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))).ok_or(format!("{k} is required (a number)"));
                Wa::global().send_location(
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
            "Where people sharing a live location in this chat are now: latest position, how old it is, whether they still share, and a map link. Updates arrive silently (no message per move); you hear when sharing ends.",
            json!({"type": "object", "required": ["chat_id"], "properties": {
                "chat_id": {"type": "string", "description": "phone digits, whatsapp:<phone>, or a group id from whatsapp_search"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                Wa::global().live_location(arg_str(&a, "chat_id")?)
            },
        )
        .tool(
            "send_contact",
            "Send a contact card (name and phone number, optional e-mail); the receiver can message or save it. Received contacts arrive as [contact] lines.",
            json!({"type": "object", "required": ["chat_id", "name", "phone"], "properties": {
                "chat_id": {"type": "string", "description": "phone digits, whatsapp:<phone>, or a group id from whatsapp_search"},
                "name": {"type": "string", "description": "the person's name"},
                "phone": {"type": "string", "description": "phone number in international format, e.g. +49 170 1234567"},
                "email": {"type": "string", "description": "optional e-mail address"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                Wa::global().send_contact(
                    arg_str(&a, "chat_id")?,
                    arg_str(&a, "name")?,
                    arg_str(&a, "phone")?,
                    arg_str_opt(&a, "email").unwrap_or(""),
                )
            },
        )
        .tool(
            "edit",
            "Replace the text of a message this account sent (message_id from the whatsapp_send result or whatsapp_history). WhatsApp only allows edits within about 15 minutes of sending.",
            json!({"type": "object", "required": ["chat_id", "message_id", "text"], "properties": {
                "chat_id": {"type": "string", "description": "phone digits, whatsapp:<phone>, or a group id from whatsapp_search"},
                "message_id": {"type": "string", "description": "id of your message to replace"},
                "text": {"type": "string", "description": "the new text"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                Wa::global().edit(arg_str(&a, "chat_id")?, arg_str(&a, "message_id")?, arg_str(&a, "text")?)
            },
        )
        .tool(
            "search",
            "Find chats by phone number, contact name, or group name. A phone number is checked against WhatsApp. Returns lines you can pass to whatsapp_send as chat_id.",
            json!({"type": "object", "required": ["query"], "properties": {
                "query": {"type": "string", "description": "a phone number with country code, or part of a name"},
                "limit": {"type": "integer", "description": "how many results. Default 20."}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                let query = arg_str(&a, "query")?;
                let limit = arg_u32_opt(&a, "limit").unwrap_or(20) as usize;
                let rows = Wa::global().search(query, limit)?;
                if rows.is_empty() {
                    return Ok(format!("(no WhatsApp chats matching {query})"));
                }
                Ok(rows
                    .into_iter()
                    .map(|(key, title)| Wa::format_chat(&key, &title))
                    .collect::<Vec<_>>()
                    .join("\n"))
            },
        )
        .tool(
            "resolve",
            "Turn one phone number, name, or chat id into the chat_id whatsapp_send needs. A bare phone number is verified to be registered on WhatsApp.",
            json!({"type": "object", "required": ["username"], "properties": {
                "username": {"type": "string", "description": "phone number with country code, a known name, or a chat id"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                let t = Wa::global().resolve_verified(arg_str(&a, "username")?)?;
                Ok(Wa::format_chat(&t.key, &t.title))
            },
        )
        .tool(
            "typing",
            "Show typing in a chat. You do not need to call this. It happens while you are replying.",
            json!({"type": "object", "required": ["chat_id"], "properties": {
                "chat_id": {"type": "string", "description": "chat id"},
                "on": {"type": "boolean", "description": "true shows typing. false stops it. Default true."}
            }}),
            |ctx, a| {
                Wa::global().set_ctx(ctx.clone());
                let chat = arg_str(&a, "chat_id")?;
                Wa::global().set_typing(chat, arg_bool(&a, "on", true));
                Ok(String::new())
            },
        )
        .tool(
            "chats",
            "List chats you already know (seen messages or searched). Each line has a chat id and a title.",
            json!({"type": "object", "properties": {}}),
            |ctx, _| {
                Wa::global().set_ctx(ctx.clone());
                match Wa::global().require_ready() {
                    Err(e) => Ok(format!("(not logged in: {e})")),
                    Ok(_) => {
                        let rows = Wa::global().chats()?;
                        if rows.is_empty() {
                            return Ok("(no chats yet; messages and searches fill this list)".into());
                        }
                        Ok(rows
                            .into_iter()
                            .map(|(key, title, banned)| {
                                let mark = if banned { "  banned" } else { "" };
                                format!("{}{mark}", Wa::format_chat(&key, &title))
                            })
                            .collect::<Vec<_>>()
                            .join("\n"))
                    }
                }
            },
        )
        .tool(
            "history",
            "List recent messages in a chat, newest first, from the local cache of messages seen while linked. Each line is the message id (same id field as live inbound envelopes, for whatsapp_delete_messages / whatsapp_react), when it was written (RFC 3339 UTC), the sender and the text. Pass before as the last id from the previous page to go further back.",
            json!({"type": "object", "required": ["chat_id"], "properties": {
                "chat_id": {"type": "string", "description": "chat id from whatsapp_search"},
                "limit": {"type": "integer", "description": "how many messages. Default 30, at most 100."},
                "before": {"type": "string", "description": "message id. Return messages older than this one."}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                let chat = arg_str(&a, "chat_id")?;
                let limit = arg_u32_opt(&a, "limit").unwrap_or(30) as usize;
                let before = arg_str_opt(&a, "before").unwrap_or("");
                let (key, title, lines) = Wa::global().history(chat, limit, before)?;
                let head = Wa::format_chat(&key, &title);
                if lines.is_empty() {
                    Ok(format!("{head}\n(no messages)"))
                } else {
                    Ok(format!("{head}\n{}", lines.join("\n")))
                }
            },
        )
        .tool(
            "react",
            "React to a WhatsApp message with an emoji. message_id comes from whatsapp_history. Pass an empty emoji to remove this account's reaction.",
            json!({"type": "object", "required": ["chat_id", "message_id"], "properties": {
                "chat_id": {"type": "string", "description": "chat id from whatsapp_search"},
                "message_id": {"type": "string", "description": "message id from whatsapp_history"},
                "emoji": {"type": "string", "description": "reaction emoji, e.g. 👍. Empty string removes your reaction."}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                let chat = arg_str(&a, "chat_id")?;
                let msg_id = arg_str(&a, "message_id")?;
                let emoji = arg_str_opt(&a, "emoji").unwrap_or("");
                let (key, title) = Wa::global().react(chat, msg_id, emoji)?;
                let who = Wa::format_chat(&key, &title);
                if emoji.trim().is_empty() {
                    Ok(format!("removed reaction on message {msg_id} → {who}"))
                } else {
                    Ok(format!("reacted {emoji} on message {msg_id} → {who}"))
                }
            },
        )
        .tool(
            "delete_messages",
            "Delete specific messages. message_ids come from whatsapp_history. By default (revoke true) they are deleted for everyone (WhatsApp allows this for your own messages, or any message when you are a group admin); revoke false deletes them only on this account. This cannot be undone.",
            json!({"type": "object", "required": ["chat_id", "message_ids"], "properties": {
                "chat_id": {"type": "string", "description": "chat id from whatsapp_search"},
                "message_ids": {"type": "array", "items": {"type": "string"}, "description": "message ids from whatsapp_history"},
                "revoke": {"type": "boolean", "description": "true (default) deletes for everyone; false only on this account."}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                let chat = arg_str(&a, "chat_id")?;
                let ids = arg_id_list(&a, "message_ids")?;
                let revoke = arg_bool(&a, "revoke", true);
                let (n, key, title) = Wa::global().delete_messages(chat, &ids, revoke)?;
                Ok(format!(
                    "deleted {n} message(s) {} → {}",
                    side(revoke),
                    Wa::format_chat(&key, &title)
                ))
            },
        )
        .tool(
            "delete_history",
            "Delete the whole message history of a chat with a person or a group on this account. By default (revoke true) your own recent messages are also deleted for everyone (WhatsApp cannot wipe the other person's copy); revoke false clears it only on this account. remove true also drops the chat from the chat list. This cannot be undone.",
            json!({"type": "object", "required": ["chat_id"], "properties": {
                "chat_id": {"type": "string", "description": "chat id from whatsapp_search"},
                "revoke": {"type": "boolean", "description": "true (default) also revokes your own recent messages for everyone; false only on this account."},
                "remove": {"type": "boolean", "description": "true also removes the chat from the chat list. Default false."}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                let chat = arg_str(&a, "chat_id")?;
                let revoke = arg_bool(&a, "revoke", true);
                let remove = arg_bool(&a, "remove", false);
                let (key, title) = Wa::global().delete_history(chat, revoke, remove)?;
                let listed = if remove {
                    ", and removed the chat from the list"
                } else {
                    ""
                };
                Ok(format!(
                    "cleared history {}{listed} → {}",
                    if revoke {
                        "on this account (own recent messages revoked for everyone)"
                    } else {
                        side(false)
                    },
                    Wa::format_chat(&key, &title)
                ))
            },
        )
        .tool(
            "create_group",
            "Create a WhatsApp group with this account as its admin and add people to it. members are phone numbers with country code (digits) or names from whatsapp_search. Returns the group's chat_id (…@g.us, use it with whatsapp_send) and an invite link; people whose privacy settings block adding them are listed — send them the link.",
            json!({"type": "object", "required": ["title"], "properties": {
                "title": {"type": "string", "description": "group name"},
                "members": {"type": "array", "items": {"type": "string"}, "description": "phone numbers with country code, or contact names"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                Wa::global().create_group(arg_str(&a, "title")?, &arg_str_list(&a, "members"))
            },
        )
        .tool(
            "add_members",
            "Add people to a WhatsApp group you administer (chat_id …@g.us). Those whose privacy settings block it are listed with an invite link to send them.",
            json!({"type": "object", "required": ["chat_id", "members"], "properties": {
                "chat_id": {"type": "string", "description": "the group id (…@g.us)"},
                "members": {"type": "array", "items": {"type": "string"}, "description": "phone numbers with country code, or contact names"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                Wa::global().add_members(arg_str(&a, "chat_id")?, &arg_str_list(&a, "members"))
            },
        )
        .tool(
            "remove_member",
            "Remove a person from a WhatsApp group you administer.",
            json!({"type": "object", "required": ["chat_id", "member"], "properties": {
                "chat_id": {"type": "string", "description": "the group id (…@g.us)"},
                "member": {"type": "string", "description": "phone number with country code, or contact name"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                Wa::global().remove_member(arg_str(&a, "chat_id")?, arg_str(&a, "member")?)
            },
        )
        .tool(
            "leave_group",
            "Leave a WhatsApp group.",
            json!({"type": "object", "required": ["chat_id"], "properties": {
                "chat_id": {"type": "string", "description": "the group id (…@g.us)"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                Wa::global().leave_group(arg_str(&a, "chat_id")?)
            },
        )
        .tool(
            "group_invite_link",
            "An invite link to a WhatsApp group you administer (anyone with it can join).",
            json!({"type": "object", "required": ["chat_id"], "properties": {
                "chat_id": {"type": "string", "description": "the group id (…@g.us)"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                Wa::global().group_invite_link(arg_str(&a, "chat_id")?)
            },
        )
        .tool(
            "ban",
            "Ban a WhatsApp chat. Messages from that chat are not delivered to you. Each one is answered with I've banned you. Incoming calls from that chat are auto-rejected. chat_id is a phone number, whatsapp:<phone>, or a chat id.",
            json!({"type": "object", "required": ["chat_id"], "properties": {
                "chat_id": {"type": "string", "description": "phone number, whatsapp:<phone>, or chat id"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                let t = Wa::global().resolve_target(arg_str(&a, "chat_id")?)?;
                let changed = Wa::global().set_banned(&t.key, true)?;
                let who = Wa::format_chat(&t.key, &t.title);
                if changed {
                    Ok(format!("banned {who}. Their messages are not delivered. They are answered with I've banned you. Their calls are auto-rejected."))
                } else {
                    Ok(format!("already banned {who}"))
                }
            },
        )
        .tool(
            "unban",
            "Unban a WhatsApp chat so its messages are delivered again. chat_id is a phone number, whatsapp:<phone>, or a chat id.",
            json!({"type": "object", "required": ["chat_id"], "properties": {
                "chat_id": {"type": "string", "description": "phone number, whatsapp:<phone>, or chat id"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                let t = Wa::global().resolve_target(arg_str(&a, "chat_id")?)?;
                let changed = Wa::global().set_banned(&t.key, false)?;
                let who = Wa::format_chat(&t.key, &t.title);
                if changed {
                    Ok(format!("unbanned {who}. Their messages are delivered again."))
                } else {
                    Ok(format!("not banned {who}"))
                }
            },
        )
        .tool(
            "call_status",
            "Show pending/active WhatsApp voice calls and whether local STT/TTS binaries are available.",
            json!({"type": "object", "properties": {}}),
            |ctx, _| {
                require_login(ctx)?;
                Ok(Calls::global().status_cmd())
            },
        )
        .tool(
            "call_start",
            "Start a private WhatsApp voice call to a person. chat_id is a phone number or whatsapp:<phone>. Wait for [call ready] or [call ended] inbound messages. When the conversation is over, end it yourself with call_hangup (do not wait for the other side to disconnect).",
            json!({"type": "object", "required": ["chat_id"], "properties": {
                "chat_id": {"type": "string", "description": "phone number or whatsapp:<phone>"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                Calls::global().start(arg_str(&a, "chat_id")?)
            },
        )
        .tool(
            "call_accept",
            "Accept a pending incoming WhatsApp voice call. chat_id is the whatsapp:<phone> from the [incoming call] message.",
            json!({"type": "object", "required": ["chat_id"], "properties": {
                "chat_id": {"type": "string", "description": "chat id from [incoming call], or whatsapp:<phone>"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                Calls::global().accept(arg_str(&a, "chat_id")?)
            },
        )
        .tool(
            "call_hangup",
            "End the active WhatsApp voice call (or reject a ringing incoming call). Use this when the conversation is done, the user says bye/goodbye/hang up, or you need to leave — you can and should hang up yourself; do not wait for them to disconnect. chat_id optional if only one call exists.",
            json!({"type": "object", "properties": {
                "chat_id": {"type": "string", "description": "phone number, whatsapp:<phone>, or chat id. Optional if only one call."}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                Calls::global().hangup(arg_str_opt(&a, "chat_id"))
            },
        )
        .tool(
            "call_reject",
            "Reject a pending incoming WhatsApp voice call (same as hangup on a ringing call).",
            json!({"type": "object", "properties": {
                "chat_id": {"type": "string", "description": "chat id from [incoming call]. Optional if only one pending call."}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                Calls::global().hangup(arg_str_opt(&a, "chat_id"))
            },
        )
        .tool(
            "call_join_group",
            "Join a group voice call: answers a pending group call invitation for that group, otherwise starts a group-bound call to the group's members. chat_id is the group id (<id>@g.us). Fails with a clear error if WhatsApp refuses the group call. Speaker identity inside group calls is not available; speech is attributed to the group.",
            json!({"type": "object", "required": ["chat_id"], "properties": {
                "chat_id": {"type": "string", "description": "group chat id (<id>@g.us) from whatsapp_search"}
            }}),
            |ctx, a| {
                require_login(ctx)?;
                Calls::global().join_group(arg_str(&a, "chat_id")?)
            },
        )
        .tool(
            "call_say",
            "Speak text into the active WhatsApp call via local TTS. chat_id is the call's chat. lang is the language of the text (defaults to the caller's preferred language from the addressbook); mark other-language parts inline: <lang=fr>Le Petit Bistro</lang>. A language without a voice fails at once: lang_install it (lang MCP) or speak an installed one. Keep it short. Prefer this (or say) while a call is active. Do not call again until you receive [you finished speaking] for the previous utterance — a new call_say cancels the current one ([voice cancelled]). call_hangup while you are still speaking waits until that has played. [speech <lang>] and chat messages may arrive while you are still speaking; wait for [you finished speaking] before speaking again unless you intentionally interrupt. After a farewell, wait for [you finished speaking], then call_hangup.",
            json!({"type": "object", "required": ["chat_id", "text"], "properties": {
                "chat_id": {"type": "string", "description": "chat id of the active call"},
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
            "Reconnect the saved WhatsApp login. This runs by itself at boot. Call it only if status says you are logged out but you were logged in before.",
            json!({"type": "object", "properties": {}}),
            |ctx, _| {
                Wa::global().set_ctx(ctx.clone());
                Wa::global().restore()
            },
        );

    server.run_with(|ctx| {
        Wa::global().set_ctx(ctx);
    });
}

#[cfg(test)]
mod tests {
    use super::arg_id_list;
    use serde_json::json;

    #[test]
    fn message_ids_accept_strings_and_numbers() {
        let a = json!({"message_ids": ["3EB0AA", 7]});
        assert_eq!(arg_id_list(&a, "message_ids").unwrap(), vec!["3EB0AA", "7"]);
        assert_eq!(
            arg_id_list(&json!({"message_ids": "9F"}), "message_ids").unwrap(),
            vec!["9F"]
        );
        assert!(arg_id_list(&json!({"message_ids": []}), "message_ids").is_err());
        assert!(arg_id_list(&json!({"message_ids": [" "]}), "message_ids").is_err());
        assert!(arg_id_list(&json!({}), "message_ids").is_err());
    }
}
