//! MCP tool handlers: what the agent does on the web bus.
use crate::{call, chat};
use bia_mcp::{arg_bool, arg_str_list, arg_str_opt, Ctx};
use serde_json::Value;

/// `(identity, channel)` of a web chat. identity is required: a web channel only names a chat,
/// the person is the identity (every person has their own `web`). Never bia itself.
pub fn target(args: &Value) -> Result<(String, String), String> {
    let identity = arg_str_opt(args, "identity")
        .unwrap_or_default()
        .trim()
        .to_string();
    let channel = arg_str_opt(args, "channel")
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| crate::bus::CH_WEB.to_string());
    if !chat::is_web(&channel) {
        return Err(format!(
            "{channel} is not a web channel (web or web:<chat>)"
        ));
    }
    if identity.is_empty() {
        return Err(
            "identity is required: the person whose web chat this is (from their envelope)".into(),
        );
    }
    if identity == chat::BIA {
        return Err(
            "identity bia is this machine itself; use the person's identity from their envelope"
                .into(),
        );
    }
    if !chat::known_identity(&identity) {
        let users = chat::users();
        return Err(format!(
            "no web user {identity}; web users: {}",
            if users.is_empty() {
                "(none)".into()
            } else {
                users.join(", ")
            }
        ));
    }
    // naming a chat that does not exist is a typo unless the model asks for a new chat
    if !chat::has_chat(&identity, &channel) && !arg_bool(args, "new_chat", false) {
        return Err(format!(
            "{identity} has no web chat {channel}; their chats: {}. Reply on the channel from their envelope, or pass new_chat=true to open a new chat",
            chat::chat_lines(&identity)
                .iter()
                .map(|l| l.split_whitespace().next().unwrap_or_default().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Ok((identity, channel))
}

fn s(args: &Value, k: &str) -> String {
    arg_str_opt(args, k).unwrap_or_default().trim().to_string()
}

pub fn send(_: &Ctx, args: Value) -> Result<String, String> {
    let (identity, channel) = target(&args)?;
    let text = arg_str_opt(&args, "text").unwrap_or_default().to_string();
    let lang = s(&args, "lang");
    let reply_to = arg_str_opt(&args, "reply_to");
    if arg_bool(&args, "voice", false) {
        let id = chat::say_voice(&identity, &channel, &text, &lang, reply_to)?;
        return Ok(format!(
            "sent to {identity}@{channel} (voice message id {id})"
        ));
    }
    let files = arg_str_list(&args, "files");
    let id = chat::say(
        &identity,
        &channel,
        &text,
        reply_to,
        &files,
        args.get("buttons"),
        args.get("poll"),
    )?;
    if call::active(&identity, &channel) && !text.trim().is_empty() {
        return Ok(match call::speak(&identity, &channel, &text, &lang) {
            Ok(_) => format!("spoken in the web call and sent to {identity}@{channel} (message id {id}); [you finished speaking] arrives when it has played"),
            Err(e) => format!("sent to {identity}@{channel} (message id {id}); NOT spoken in the active web call: {e}"),
        });
    }
    Ok(format!("sent to {identity}@{channel} (message id {id})"))
}

pub fn edit(_: &Ctx, args: Value) -> Result<String, String> {
    let (identity, channel) = target(&args)?;
    let id = s(&args, "message_id");
    let text = arg_str_opt(&args, "text").unwrap_or_default();
    chat::agent_edit(&identity, &channel, &id, text)?;
    Ok(format!("edited {id} on {identity}@{channel}"))
}

pub fn react(_: &Ctx, args: Value) -> Result<String, String> {
    let (identity, channel) = target(&args)?;
    let id = s(&args, "message_id");
    let emoji = s(&args, "emoji");
    chat::agent_react(&identity, &channel, &id, &emoji)?;
    Ok(if emoji.is_empty() {
        format!("removed the reaction on {id}")
    } else {
        format!("reacted {emoji} on {id}")
    })
}

pub fn delete(_: &Ctx, args: Value) -> Result<String, String> {
    let (identity, channel) = target(&args)?;
    let ids = arg_str_list(&args, "message_ids");
    if ids.is_empty() {
        return Err("message_ids required".into());
    }
    let n = chat::agent_delete(&identity, &channel, &ids)?;
    Ok(format!("deleted {n} message(s) for everyone"))
}

pub fn pin(_: &Ctx, args: Value) -> Result<String, String> {
    let (identity, channel) = target(&args)?;
    let id = s(&args, "message_id");
    let on = arg_bool(&args, "on", true);
    chat::agent_pin(&identity, &channel, &id, on)?;
    Ok(format!("{} {id}", if on { "pinned" } else { "unpinned" }))
}

pub fn close_poll(_: &Ctx, args: Value) -> Result<String, String> {
    let (identity, channel) = target(&args)?;
    let id = s(&args, "message_id");
    chat::agent_close_poll(&identity, &channel, &id).map(|t| format!("poll {id} closed: {t}"))
}

pub fn history(_: &Ctx, args: Value) -> Result<String, String> {
    let (identity, channel) = target(&args)?;
    let n = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(30) as usize;
    chat::agent_history(&identity, &channel, n)
}

pub fn read(_: &Ctx, args: Value) -> Result<String, String> {
    let (identity, channel) = target(&args)?;
    let id = s(&args, "message_id");
    chat::agent_read(&identity, &channel, &id);
    Ok(format!("read up to {id}"))
}

pub fn typing(_: &Ctx, args: Value) -> Result<String, String> {
    let (identity, channel) = target(&args)?;
    let on = arg_bool(&args, "on", true);
    chat::typing(&identity, &channel, on);
    Ok(if on { "typing" } else { "stopped typing" }.into())
}

pub fn chats(_: &Ctx, _: Value) -> Result<String, String> {
    let users = chat::users();
    if users.is_empty() {
        return Ok("(no web chats yet)".into());
    }
    let mut out = Vec::new();
    for u in users {
        let open = if chat::has_client(&u) {
            "  (browser open)"
        } else {
            ""
        };
        out.push(format!("{u}{open}"));
        for l in chat::chat_lines(&u) {
            out.push(format!("  {l}"));
        }
    }
    Ok(out.join("\n"))
}

pub fn call_start(_: &Ctx, args: Value) -> Result<String, String> {
    let (identity, channel) = target(&args)?;
    call::ring(&identity, &channel)
}

pub fn call_cancel(_: &Ctx, args: Value) -> Result<String, String> {
    let (identity, channel) = target(&args)?;
    call::cancel_ring(&identity, &channel)
}

pub fn call_hangup(_: &Ctx, args: Value) -> Result<String, String> {
    let (identity, channel) = target(&args)?;
    call::hangup(&identity, &channel)
}

pub fn status(_: &Ctx, _: Value) -> Result<String, String> {
    match crate::WEB.get() {
        Some(w) => Ok(format!(
            "{}\nCA fingerprint (sha256): {}\nAccess needs a client certificate from this machine's CA (web_pki_issue when root asks).",
            w.info_line(),
            w.ca_sha256()
        )),
        None => {
            let st = std::fs::read_to_string(crate::files_dir().join("state").join("web.json"))
                .ok()
                .and_then(|s| serde_json::from_str::<Value>(&s).ok());
            let err = st
                .as_ref()
                .and_then(|v| v.get("error"))
                .and_then(|e| e.as_str())
                .unwrap_or("not started");
            Err(format!("web server is not listening: {err}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::{Arc, Once};

    /// One chat store per test process (it is a global, like in the server).
    fn setup() -> Arc<crate::bus::Bus> {
        static INIT: Once = Once::new();
        static mut BUS: Option<Arc<crate::bus::Bus>> = None;
        INIT.call_once(|| {
            let dir = std::env::temp_dir().join(format!("bia-mcp-web-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let bus = Arc::new(crate::bus::Bus::new(dir.clone()));
            chat::init(&dir, bus.clone());
            unsafe { BUS = Some(bus) };
        });
        #[allow(static_mut_refs)]
        unsafe {
            BUS.clone().unwrap()
        }
    }

    fn ctx() -> Ctx {
        Ctx::console()
    }

    #[test]
    fn identity_is_required_and_never_bia() {
        setup();
        let e = send(&ctx(), json!({"channel": "web", "text": "hi"})).unwrap_err();
        assert!(e.contains("identity is required"), "{e}");
        let e = send(
            &ctx(),
            json!({"identity": "bia", "channel": "web", "text": "hi"}),
        )
        .unwrap_err();
        assert!(e.contains("this machine itself"), "{e}");
        let e = send(&ctx(), json!({"identity": "nobody", "text": "hi"})).unwrap_err();
        assert!(e.contains("no web user nobody"), "{e}");
        let e = send(
            &ctx(),
            json!({"identity": "root", "channel": "console", "text": "hi"}),
        )
        .unwrap_err();
        assert!(e.contains("not a web channel"), "{e}");
        let e = send(
            &ctx(),
            json!({"identity": "root", "channel": "telegram:1", "text": "hi"}),
        )
        .unwrap_err();
        assert!(e.contains("not a web channel"), "{e}");
    }

    #[test]
    fn reply_lands_in_the_senders_own_chat() {
        // Alex writes on web; the answer must go to web/alex/main.json, next to Alex's message.
        let bus = setup();
        let files = bus.files.clone();
        std::fs::create_dir_all(files.join("state/web/alex")).unwrap();
        let r = send(
            &ctx(),
            json!({"identity": "alex", "channel": "web", "text": "hello Alex"}),
        )
        .unwrap();
        assert!(r.starts_with("sent to alex@web (message id b"), "{r}");
        let main = std::fs::read_to_string(files.join("state/web/alex/main.json")).unwrap();
        assert!(main.contains("hello Alex"));
        assert!(
            !files.join("state/web/bia").exists(),
            "nothing is written under web/bia"
        );
        // history, react, pin, edit and delete address the same chat
        let id = r
            .split("message id ")
            .nth(1)
            .unwrap()
            .trim_end_matches(')')
            .to_string();
        assert!(history(&ctx(), json!({"identity": "alex"}))
            .unwrap()
            .contains("hello Alex"));
        react(
            &ctx(),
            json!({"identity": "alex", "message_id": id, "emoji": "👍"}),
        )
        .unwrap();
        pin(&ctx(), json!({"identity": "alex", "message_id": id})).unwrap();
        edit(
            &ctx(),
            json!({"identity": "alex", "message_id": id, "text": "hello again"}),
        )
        .unwrap();
        assert!(history(&ctx(), json!({"identity": "alex"}))
            .unwrap()
            .contains("hello again"));
        assert_eq!(
            delete(&ctx(), json!({"identity": "alex", "message_ids": [id]})).unwrap(),
            "deleted 1 message(s) for everyone"
        );
    }

    #[test]
    fn other_chats_and_listing() {
        let bus = setup();
        std::fs::create_dir_all(bus.files.join("state/web/carol")).unwrap();
        // a chat that does not exist is refused (a typo like web:bia must not open one)
        let e = send(
            &ctx(),
            json!({"identity": "carol", "channel": "web:trip", "text": "tickets"}),
        )
        .unwrap_err();
        assert!(
            e.contains("carol has no web chat web:trip") && e.contains("new_chat=true"),
            "{e}"
        );
        assert!(!bus.files.join("state/web/carol/trip.json").exists());
        send(
            &ctx(),
            json!({"identity": "carol", "channel": "web:trip", "text": "tickets", "new_chat": true}),
        )
        .unwrap();
        // once it exists, no flag is needed
        send(
            &ctx(),
            json!({"identity": "carol", "channel": "web:trip", "text": "and hotel"}),
        )
        .unwrap();
        let l = chats(&ctx(), json!({})).unwrap();
        assert!(l.contains("carol") && l.contains("web:trip"), "{l}");
        assert!(!l.lines().any(|x| x.trim() == "bia"), "{l}");
    }

    #[test]
    fn people_messages_reach_the_agent_as_web_envelopes() {
        let bus = setup();
        std::fs::create_dir_all(bus.files.join("state/web/dave")).unwrap();
        chat::action(
            "dave",
            false,
            &json!({"t": "send", "chat": "main", "text": "ping"}),
        )
        .unwrap();
        let ib = loop {
            let ib = bus
                .next_held(std::time::Duration::from_secs(10))
                .expect("delivered");
            if ib.identity == "dave" {
                break ib;
            }
        };
        assert_eq!(ib.channel, "web");
        assert!(ib.text.contains("ping"), "{}", ib.text);
        assert!(ib.id.starts_with('b'));
    }
}
