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

fn port_file(files: &std::path::Path) -> std::path::PathBuf {
    files.join("state").join("web").join("port.json")
}

/// The HTTPS port root chose with web_port, if any: it wins over the one the agent passes.
pub fn saved_port(files: &std::path::Path) -> Option<u16> {
    let v: Value = bia_mcp::statefile::read(&port_file(files))?;
    v["port"].as_u64().and_then(|p| u16::try_from(p).ok()).filter(|p| *p > 0)
}

/// Keep another HTTPS port for the next start; `http` is the plain-HTTP redirect's port and
/// `now` the port served at the moment (0: none).
fn set_port(files: &std::path::Path, port: u64, http: u16, now: u16) -> Result<String, String> {
    let path = port_file(files);
    if port == 0 {
        bia_mcp::statefile::remove(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        return Ok("The saved port is gone: the web app is back on its standard port (443) after discover(path=\"mcp/web\", restart=true).".into());
    }
    let port = u16::try_from(port).map_err(|_| "port: 1 to 65535".to_string())?;
    if port == http {
        return Err(format!("port {port} is the plain HTTP one that sends browsers on to HTTPS: pick another"));
    }
    if port != now {
        // taken ports are refused now, not found out after the restart
        if let Err(e) = std::net::TcpListener::bind(("0.0.0.0", port)) {
            return Err(format!("port {port} cannot be used ({}; {e})", crate::listen::describe_port(files, port)));
        }
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    bia_mcp::statefile::write_json(&path, &serde_json::json!({"port": port})).map_err(|e| format!("{}: {e}", path.display()))?;
    if port == now {
        return Ok(format!("The web app already answers on port {port}; it is saved and stays so."));
    }
    let at = if port == 443 { String::new() } else { format!(":{port}") };
    Ok(format!(
        "Saved: the web app answers on port {port} (https://<address>{at}/) once this MCP restarts. First tell root the new address and that a firewall in front of this machine must let TCP {port} in, then discover(path=\"mcp/web\", restart=true). Everyone connected is told the new port as the restart cuts them off: an open web app shows the new address, a bia deploy app with the chat open, bia-bridge and other bias follow by themselves. Off port 443 nothing answers on port 80 any more (no redirect from http://), so the address needs https:// and the port. Whoever is not connected at that moment keeps the old port: a bia deploy app needs a new .biaconfig (web_pki_biaconfig), an offline bia or bia-bridge the new URL."
    ))
}

/// The port web_port chose while this process runs (0: it was not called).
static NEXT_PORT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// The HTTPS port the web app answers on after this process, when web_port just chose another
/// one: what everyone connected is told as the links close.
pub fn moving_to() -> Option<u16> {
    let next = NEXT_PORT.load(std::sync::atomic::Ordering::SeqCst) as u16;
    let now = crate::WEB.get().map_or(0, |w| w.port);
    (next != 0 && now != 0 && next != now).then_some(next)
}

/// The tool: root moves the web app to another HTTPS port.
pub fn port(_: &Ctx, args: Value) -> Result<String, String> {
    let port = args.get("port").and_then(Value::as_u64).ok_or("port: a number, 1 to 65535 (0 = back to the standard port)")?;
    let env = |k: &str, d: u16| std::env::var(k).ok().and_then(|v| v.trim().parse().ok()).unwrap_or(d);
    let said = set_port(&crate::files_dir(), port, env("BIA_WEB_HTTP_PORT", 80), crate::WEB.get().map_or(0, |w| w.port))?;
    // 0 forgot the saved port: the next start is on the one the agent passes
    let next = if port == 0 { env("BIA_WEB_PORT", 443) } else { port as u16 };
    NEXT_PORT.store(next as u32, std::sync::atomic::Ordering::SeqCst);
    Ok(said)
}

#[cfg(test)]
mod port_tests {
    use super::*;

    #[test]
    fn a_saved_port_is_kept_checked_and_dropped() {
        let files = std::env::temp_dir().join(format!("bia-web-port-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&files);
        assert_eq!(saved_port(&files), None);
        // a port somebody holds is refused, and nothing is saved
        let held = std::net::TcpListener::bind(("0.0.0.0", 0)).unwrap();
        let busy = held.local_addr().unwrap().port();
        assert!(set_port(&files, busy as u64, 80, 443).unwrap_err().contains("cannot be used"));
        assert!(set_port(&files, 80, 80, 443).unwrap_err().contains("plain HTTP"));
        assert!(set_port(&files, 70000, 80, 443).is_err());
        assert_eq!(saved_port(&files), None);
        // the one served now needs no free port
        assert!(set_port(&files, busy as u64, 80, busy).unwrap().contains("already answers"));
        assert_eq!(saved_port(&files), Some(busy));
        drop(held);
        let free = std::net::TcpListener::bind(("0.0.0.0", 0)).unwrap().local_addr().unwrap().port();
        let said = set_port(&files, free as u64, 80, 443).unwrap();
        assert!(said.contains(&format!(":{free}/")) && said.contains("restart=true"), "{said}");
        assert_eq!(saved_port(&files), Some(free));
        assert!(set_port(&files, 0, 80, free).is_ok());
        assert_eq!(saved_port(&files), None);
        let _ = std::fs::remove_dir_all(&files);
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
