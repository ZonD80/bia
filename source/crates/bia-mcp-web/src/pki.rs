//! Issue client certificates (root asks). Pickup URLs are one-time HTTPS downloads.
use bia_mcp::Ctx;
use bia_pki::Pki;
use serde_json::Value;
use std::path::PathBuf;

fn files_dir() -> PathBuf {
    PathBuf::from(std::env::var("BIA_FILES").unwrap_or_else(|_| "/home/bia/files".into()))
}

fn web_port() -> u16 {
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
    let role = bia_mcp::arg_str_opt(&args, "role").unwrap_or_else(|| "user".into());
    let trust = bia_mcp::arg_str_opt(&args, "trust").unwrap_or_else(|| {
        if identity == "root" {
            "root".into()
        } else {
            "normal".into()
        }
    });
    let pki = Pki::load_or_create(&files_dir()).map_err(|e| format!("pki: {e:#}"))?;
    let (token, meta) = pki
        .issue_pickup(&identity, &role, &trust)
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
            "{}  {}  role={}  trust={}{}{}",
            m.serial,
            m.identity,
            m.role,
            m.trust,
            if m.revoked { "  REVOKED" } else { "" },
            exp
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
            let note = if r.note.is_empty() { String::new() } else { format!("  note (untrusted): \"{}\"", r.note) };
            format!("{}  {}  identity={} role={} from {}{note}", r.short(), r.status.as_str(), r.identity, r.role, r.source)
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

/// Approve or deny a request. The requester polls and collects the certificate itself.
pub fn decide(ctx: &Ctx, args: Value, approve: bool) -> Result<String, String> {
    let id = bia_mcp::arg_str(&args, "id")?.trim().to_string();
    let reason = bia_mcp::arg_str_opt(&args, "reason").unwrap_or_default();
    let pki = Pki::load_or_create(&files_dir()).map_err(|e| format!("pki: {e:#}"))?;
    let r = pki
        .decide_request(&id, approve, "root@agent", &reason)
        .map_err(|e| format!("{e:#}"))?;
    ctx.log(format!("certificate request {} for {}: {}", r.short(), r.identity, r.status.as_str()));
    Ok(if approve {
        format!(
            "approved request {} for {}. It downloads its certificate itself on its next check (it polls by request id); nothing to send.",
            r.short(),
            r.identity
        )
    } else {
        format!("denied request {} for {}.", r.short(), r.identity)
    })
}
