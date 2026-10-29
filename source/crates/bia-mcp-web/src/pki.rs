//! Issue client certificates (root asks). Pickup URLs are one-time HTTPS downloads.
use bia_mcp::Ctx;
use bia_pki::Pki;
use serde_json::Value;

use crate::files_dir;

fn web_port() -> u16 {
    if let Some(w) = crate::WEB.get() {
        return w.port;
    }
    let p = files_dir().join("state").join("web.json");
    let Ok(raw) = std::fs::read_to_string(p) else {
        return 443;
    };
    let Ok(v) = serde_json::from_str::<Value>(&raw) else {
        return 443;
    };
    v.get("port")
        .and_then(|x| x.as_u64())
        .filter(|n| *n > 0 && *n <= 65535)
        .map(|n| n as u16)
        .unwrap_or(443)
}

pub fn issue(ctx: &Ctx, args: Value) -> Result<String, String> {
    let identity = bia_mcp::arg_str(&args, "identity")?.trim().to_string();
    if identity.is_empty() {
        return Err("identity is required".into());
    }
    let role = bia_mcp::arg_str_opt(&args, "role").unwrap_or("user");
    let trust = bia_mcp::arg_str_opt(&args, "trust").unwrap_or(if identity == "root" {
        "root"
    } else {
        "normal"
    });
    let from = cidrs(&args, "from")?;
    let pki = Pki::load_or_create(&files_dir()).map_err(|e| format!("pki: {e:#}"))?;
    let (token, meta) = pki
        .issue_pickup_from(&identity, role, trust, &from)
        .map_err(|e| format!("issue: {e:#}"))?;
    let port = web_port();
    let urls = crate::listen::pickup_urls(port, &token);
    ctx.log(format!(
        "issued {} cert for {} (role={} trust={})",
        meta.serial, meta.identity, meta.role, meta.trust
    ));
    for u in &urls {
        ctx.log(format!("one-time PKCS#12 (password 12345678): {u}"));
    }
    if urls.is_empty() {
        Ok(format!(
            "issued cert for {identity} (serial {}). No LAN URL yet — pickup token {token} at /v1/pki/pickup/{token} once HTTPS has an address.",
            meta.serial
        ))
    } else {
        Ok(format!(
            "issued cert for {identity} (serial {}). One-time download (deleted after GET; .p12 password 12345678):\n{}",
            meta.serial,
            urls.join("\n")
        ))
    }
}

pub fn list(_: &Ctx, _: Value) -> Result<String, String> {
    let pki = Pki::load_or_create(&files_dir()).map_err(|e| format!("pki: {e:#}"))?;
    let list = pki.list_issued().map_err(|e| format!("{e:#}"))?;
    if list.is_empty() {
        return Ok("(no client certificates issued)".into());
    }
    let mut lines = Vec::new();
    for m in list {
        let exp = if m.not_after > 0 {
            format!("  not_after={}", m.not_after)
        } else {
            String::new()
        };
        lines.push(format!(
            "{}  {}  role={}  trust={}{}{}{}",
            m.serial,
            m.identity,
            m.role,
            m.trust,
            if m.revoked { "  REVOKED" } else { "" },
            exp,
            if m.from.is_empty() {
                String::new()
            } else {
                format!("  only from {}", m.from.join(","))
            }
        ));
    }
    Ok(lines.join("\n"))
}

pub fn revoke(ctx: &Ctx, args: Value) -> Result<String, String> {
    let serial = bia_mcp::arg_str(&args, "serial")?.trim().to_string();
    let pki = Pki::load_or_create(&files_dir()).map_err(|e| format!("pki: {e:#}"))?;
    if pki.revoke(&serial).map_err(|e| format!("{e:#}"))? {
        ctx.log(format!("revoked client cert {serial}"));
        Ok(format!("revoked {serial}"))
    } else {
        Err(format!("no cert with serial {serial}"))
    }
}

pub fn requests(_: &Ctx, _: Value) -> Result<String, String> {
    let pki = Pki::load_or_create(&files_dir()).map_err(|e| format!("pki: {e:#}"))?;
    let list = pki.list_requests();
    if list.is_empty() {
        return Ok("(no certificate requests)".into());
    }
    Ok(list
        .iter()
        .map(|r| {
            let note = if r.note.is_empty() {
                String::new()
            } else {
                format!("  note (untrusted): \"{}\"", r.note)
            };
            format!(
                "{}  {}  identity={} role={} from IP {}{note}{}",
                r.short(),
                r.status.as_str(),
                r.identity,
                r.role,
                r.source,
                match &r.grant {
                    Some(g) => format!("  mutual: a bia that says it is at {}", g.url),
                    None => String::new(),
                }
            )
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

/// Approve or deny a request. The requester polls and collects the certificate itself.
pub fn decide(ctx: &Ctx, args: Value, approve: bool) -> Result<String, String> {
    // the request notice says "request <id>", so accept that name too
    let id = bia_mcp::arg_str_opt(&args, "id")
        .or_else(|| bia_mcp::arg_str_opt(&args, "request"))
        .ok_or("id is required (the request id from web_pki_requests or the notice)")?
        .trim()
        .to_string();
    // deny: why; approve: root's message for the requester (it reads it when it polls)
    let reason = bia_mcp::arg_str_opt(&args, "reason")
        .or_else(|| bia_mcp::arg_str_opt(&args, "message"))
        .unwrap_or_default();
    let bind_ip = approve && args.get("bind_ip").and_then(|x| x.as_bool()).unwrap_or(false);
    let pki = Pki::load_or_create(&files_dir()).map_err(|e| format!("pki: {e:#}"))?;
    let from = if bind_ip {
        let src = pki.find_request(&id).map_err(|e| format!("{e:#}"))?.source;
        vec![bia_pki::parse_ip(&src)
            .map(bia_pki::Cidr::single)
            .map_err(|e| format!("request source {src}: {e:#}"))?]
    } else {
        vec![]
    };
    let r = pki
        .decide_request_opts(&id, approve, "root@agent", reason, &from)
        .map_err(|e| format!("{e:#}"))?;
    ctx.log(format!(
        "certificate request {} for {}: {}",
        r.short(),
        r.identity,
        r.status.as_str()
    ));
    Ok(if approve {
        format!(
            "approved request {} for {}. It downloads its certificate itself on its next check (it polls by request id){}{}",
            r.short(),
            r.identity,
            if r.reason.is_empty() { "; nothing to send.".to_string() } else { format!(" and reads your message with it: \"{}\"", r.reason) },
            if r.grant.is_some() {
                format!(" Trust is now set up both ways: {0} is a peer. The link with it opens by itself; a [peer {0}] notice comes when it is up, then talk with say(channel=\"peer:{0}\").", r.identity)
            } else {
                String::new()
            }
        )
    } else {
        format!("denied request {} for {}.", r.short(), r.identity)
    })
}

/// `args[key]`: a list (or one comma-separated string) of addresses / CIDR networks.
fn cidrs(args: &Value, key: &str) -> Result<Vec<bia_pki::Cidr>, String> {
    let items: Vec<String> = match args.get(key) {
        Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect(),
        Some(Value::String(s)) => s.split(',').map(str::to_string).collect(),
        _ => vec![],
    };
    items
        .iter()
        .filter(|s| !s.trim().is_empty())
        .map(|s| bia_pki::Cidr::parse(s).map_err(|e| format!("{key}: {e:#}")))
        .collect()
}

pub fn peer_add(ctx: &Ctx, args: Value) -> Result<String, String> {
    let url = bia_mcp::arg_str(&args, "url")?.trim().to_string();
    let pin = bia_mcp::arg_str_opt(&args, "ca_sha256");
    let note = bia_mcp::arg_str_opt(&args, "note").unwrap_or_default();
    let my_url = bia_mcp::arg_str_opt(&args, "my_url").filter(|s| !s.trim().is_empty());
    let pki = Pki::load_or_create(&files_dir()).map_err(|e| format!("pki: {e:#}"))?;
    let p = pki
        .peer_connect(&url, pin, my_url, web_port(), note)
        .map_err(|e| format!("{e:#}"))?;
    ctx.log(format!("asked {} at {} for mutual trust (request {})", p.name, p.url, &p.request[..8.min(p.request.len())]));
    Ok(format!(
        "Asked {name} at {url} (CA {ca}). It already holds a certificate from this bia; once its root approves, this bia gets its certificate too, the link opens by itself and a [peer {name}] notice arrives. Pending until then (its request lives 3 days).",
        name = p.name,
        url = p.url,
        ca = &p.ca_sha256[..16]
    ))
}

/// Peers with trust and link state, one block each.
pub fn peer_list_text() -> Result<String, String> {
    let pki = Pki::load_or_create(&files_dir()).map_err(|e| format!("pki: {e:#}"))?;
    let peers = pki.peers();
    if peers.is_empty() {
        return Ok(format!(
            "(no peers) This bia is {} to other bias; web_peer_add sets up trust with one.",
            pki.agent_name()
        ));
    }
    let mut out = vec![format!("This bia is {} to other bias.", pki.agent_name())];
    for p in peers {
        out.push(p.describe());
        if p.status != "active" {
            continue;
        }
        if let Some(l) = crate::peer::links() {
            let d = l.describe(&p.name);
            out.push(format!("  link: {}  (channel peer:{})", d["state"].as_str().unwrap_or(""), p.name));
            if let Some(e) = d["last_error"].as_str().filter(|e| !e.is_empty() && d["online"] != true) {
                out.push(format!("  last try: {e}"));
            }
            for t in d["transfers"].as_array().into_iter().flatten() {
                out.push(format!("  file {}", t.as_str().unwrap_or("")));
            }
        }
    }
    Ok(out.join("\n"))
}

pub fn peer_list(_: &Ctx, _: Value) -> Result<String, String> {
    peer_list_text()
}

pub fn peer_remove(ctx: &Ctx, args: Value) -> Result<String, String> {
    let name = bia_mcp::arg_str(&args, "peer")?.trim().to_string();
    let pki = Pki::load_or_create(&files_dir()).map_err(|e| format!("pki: {e:#}"))?;
    // trust first, so the link manager cannot take the peer up again meanwhile
    let p = pki.peer_remove(&name).map_err(|e| format!("{e:#}"))?;
    if let Some(l) = crate::peer::links() {
        l.forget(&name);
    }
    ctx.log(format!("removed peer {} ({})", p.name, p.url));
    Ok(format!(
        "Removed {}. Its link is closed, the certificate this bia gave it is revoked and its certificate for this bia is forgotten here.",
        p.name
    ))
}
