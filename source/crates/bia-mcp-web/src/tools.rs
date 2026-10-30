//! MCP tool handlers: what the agent does on the web bus.
use crate::{call, chat};
use bia_mcp::{arg_bool, arg_str_list, arg_str_opt, Ctx};
use serde_json::Value;

/// The identity of a web chat. identity is required: the channel does not name the person
/// (everyone has their own `web`). Never bia itself. A channel from before 1.9 still works:
/// `web:<chat>` is the one chat, `peer:<name>` is `<name>`'s.
pub fn target(args: &Value) -> Result<(String, String), String> {
    let channel = arg_str_opt(args, "channel")
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| crate::bus::CH_WEB.to_string());
    let mut identity = arg_str_opt(args, "identity")
        .unwrap_or_default()
        .trim()
        .to_string();
    if let Some(name) = channel.strip_prefix("peer:").filter(|n| !n.is_empty()) {
        if identity.is_empty() {
            identity = name.to_string();
        }
    } else if !chat::is_web(&channel) {
        return Err(format!("{channel} is not a web channel (use web)"));
    }
    if identity.is_empty() {
        return Err(
            "identity is required: whose web chat this is (from their envelope)".into(),
        );
    }
    if identity == chat::BIA {
        return Err(
            "identity bia is this machine itself; use the identity from their envelope"
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
    Ok((identity, crate::bus::CH_WEB.to_string()))
}

fn s(args: &Value, k: &str) -> String {
    arg_str_opt(args, k).unwrap_or_default().trim().to_string()
}

pub fn send(_: &Ctx, args: Value) -> Result<String, String> {
    let (identity, channel) = target(&args)?;
    let text = arg_str_opt(&args, "text").unwrap_or_default().to_string();
    let lang = s(&args, "lang");
    let reply_to = arg_str_opt(&args, "reply_to");
    let waits = if crate::link::online(&identity) { "" } else { "; they are not connected now, it waits there for them" };
    if arg_bool(&args, "voice", false) {
        let id = chat::say_voice(&identity, &text, &lang, reply_to)?;
        return Ok(format!(
            "sent to {identity}@{channel} (voice message id {id}){waits}"
        ));
    }
    let files = arg_str_list(&args, "files");
    let id = chat::say(
        &identity,
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
    Ok(format!("sent to {identity}@{channel} (message id {id}){waits}"))
}

pub fn edit(_: &Ctx, args: Value) -> Result<String, String> {
    let (identity, channel) = target(&args)?;
    let id = s(&args, "message_id");
    let text = arg_str_opt(&args, "text").unwrap_or_default();
    chat::agent_edit(&identity, &id, text)?;
    Ok(format!("edited {id} on {identity}@{channel}"))
}

pub fn react(_: &Ctx, args: Value) -> Result<String, String> {
    let (identity, _) = target(&args)?;
    let id = s(&args, "message_id");
    let emoji = s(&args, "emoji");
    chat::agent_react(&identity, &id, &emoji)?;
    Ok(if emoji.is_empty() {
        format!("removed the reaction on {id}")
    } else {
        format!("reacted {emoji} on {id}")
    })
}

pub fn delete(_: &Ctx, args: Value) -> Result<String, String> {
    let (identity, _) = target(&args)?;
    let ids = arg_str_list(&args, "message_ids");
    if ids.is_empty() {
        return Err("message_ids required".into());
    }
    let n = chat::agent_delete(&identity, &ids)?;
    Ok(format!("deleted {n} message(s) for everyone"))
}

pub fn pin(_: &Ctx, args: Value) -> Result<String, String> {
    let (identity, _) = target(&args)?;
    let id = s(&args, "message_id");
    let on = arg_bool(&args, "on", true);
    chat::agent_pin(&identity, &id, on)?;
    Ok(format!("{} {id}", if on { "pinned" } else { "unpinned" }))
}

pub fn close_poll(_: &Ctx, args: Value) -> Result<String, String> {
    let (identity, _) = target(&args)?;
    let id = s(&args, "message_id");
    chat::agent_close_poll(&identity, &id).map(|t| format!("poll {id} closed: {t}"))
}

pub fn history(_: &Ctx, args: Value) -> Result<String, String> {
    let (identity, _) = target(&args)?;
    let n = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(30) as usize;
    chat::agent_history(&identity, n)
}

pub fn read(_: &Ctx, args: Value) -> Result<String, String> {
    let (identity, _) = target(&args)?;
    let id = s(&args, "message_id");
    chat::agent_read(&identity, &id);
    Ok(format!("read up to {id}"))
}

pub fn typing(_: &Ctx, args: Value) -> Result<String, String> {
    let (identity, _) = target(&args)?;
    let on = arg_bool(&args, "on", true);
    chat::typing(&identity, on);
    Ok(if on { "typing" } else { "stopped typing" }.into())
}

pub fn chats(_: &Ctx, _: Value) -> Result<String, String> {
    let users = chat::users();
    if users.is_empty() {
        return Ok("(no web chats yet)".into());
    }
    Ok(users
        .iter()
        .map(|u| {
            let mut l = format!("{u}  web");
            let n = chat::unread(u);
            if n > 0 {
                l.push_str(&format!("  ({n} unread)"));
            }
            if crate::link::online(u) {
                l.push_str("  (connected)");
            }
            l
        })
        .collect::<Vec<_>>()
        .join("\n"))
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

    fn ctx() -> Ctx {
        Ctx::console()
    }

    #[test]
    fn identity_is_required_and_never_bia() {
        crate::chat::tests::setup();
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
        for ch in ["console", "telegram:1"] {
            let e = send(&ctx(), json!({"identity": "root", "channel": ch, "text": "hi"})).unwrap_err();
            assert!(e.contains("not a web channel"), "{e}");
        }
    }

    #[test]
    fn reply_lands_in_the_one_chat() {
        let (_, files) = crate::chat::tests::setup();
        std::fs::create_dir_all(files.join("state/web/alex")).unwrap();
        let r = send(
            &ctx(),
            json!({"identity": "alex", "channel": "web", "text": "hello Alex"}),
        )
        .unwrap();
        assert!(r.starts_with("sent to alex@web (message id b") && r.contains("not connected now"), "{r}");
        let main = std::fs::read_to_string(files.join("state/web/alex/main.json")).unwrap();
        assert!(main.contains("hello Alex"));
        assert!(
            !files.join("state/web/bia").exists(),
            "nothing is written under web/bia"
        );
        // a chat named the old way is the same one chat
        send(&ctx(), json!({"identity": "alex", "channel": "web:trip", "text": "tickets"})).unwrap();
        assert!(history(&ctx(), json!({"identity": "alex"})).unwrap().contains("tickets"));
        let id = r
            .split("message id ")
            .nth(1)
            .unwrap()
            .split(')')
            .next()
            .unwrap()
            .to_string();
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
        let l = chats(&ctx(), json!({})).unwrap();
        assert!(l.lines().any(|x| x.starts_with("alex  web")), "{l}");
        assert!(!l.lines().any(|x| x.trim() == "bia"), "{l}");
    }

    #[test]
    fn old_peer_channel_names_the_identity() {
        let (_, files) = crate::chat::tests::setup();
        std::fs::create_dir_all(files.join("state/web/fred")).unwrap();
        assert_eq!(target(&json!({"channel": "peer:fred"})).unwrap(), ("fred".to_string(), "web".to_string()));
    }

    #[test]
    fn messages_reach_the_agent_as_web_envelopes() {
        let (bus, files) = crate::chat::tests::setup();
        std::fs::create_dir_all(files.join("state/web/dave")).unwrap();
        chat::action(
            "dave",
            false,
            &json!({"t": "send", "text": "ping"}),
        )
        .unwrap();
        let ib = crate::chat::tests::next_for(&bus, "dave");
        assert_eq!(ib.channel, "web");
        assert!(ib.text.contains("ping"), "{}", ib.text);
        assert!(ib.id.starts_with('b'));
    }
}
