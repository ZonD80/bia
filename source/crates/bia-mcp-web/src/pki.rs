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

/// `addr`, split into what its certificate must name and how it is dialled with this port.
fn address_of(addr: &str, port: u16) -> Result<(String, String), String> {
    let a = addr.trim().trim_start_matches("https://").trim_end_matches('/');
    if a.is_empty() || a.contains(char::is_whitespace) || a.contains('/') || a.contains('@') {
        return Err(format!("{addr:?} is not an address: a host name or IP, with :port if it is not this bia's"));
    }
    if let Ok(ip) = a.parse::<std::net::IpAddr>() {
        let host = ip.to_string();
        let shown = if ip.is_ipv6() { format!("[{host}]") } else { host.clone() };
        return Ok((host, if port == 443 { shown } else { format!("{shown}:{port}") }));
    }
    if let Some(rest) = a.strip_prefix('[') {
        let (h, tail) = rest.split_once(']').ok_or_else(|| format!("{addr:?} is not an address"))?;
        h.parse::<std::net::Ipv6Addr>().map_err(|_| format!("{addr:?} is not an address"))?;
        return Ok((h.to_string(), if tail.is_empty() { if port == 443 { format!("[{h}]") } else { format!("[{h}]:{port}") } } else { a.to_string() }));
    }
    match a.rsplit_once(':') {
        Some((h, p)) if p.parse::<u16>().is_ok() => Ok((h.to_string(), a.to_string())),
        Some(_) => Err(format!("{addr:?} is not an address")),
        None => Ok((a.to_string(), if port == 443 { a.to_string() } else { format!("{a}:{port}") })),
    }
}

/// What `make_biaconfig` hands back.
pub struct MadeConfig {
    pub sealed: Vec<u8>,
    pub password: String,
    pub generated: bool,
    pub addresses: Vec<String>,
    /// The server certificate had to learn an address: it does within seconds.
    pub added: bool,
    pub serial: String,
    pub identity: String,
    pub role: String,
    pub trust: String,
}

/// A `.biaconfig` for a new client certificate, which can be revoked by itself. Arguments are
/// checked first: a refused request leaves no certificate and no new server name behind.
pub fn make_biaconfig(pki: &Pki, port: u16, args: &Value) -> Result<MadeConfig, String> {
    use bia_pki::biaconfig::{self as file, BiaConfig};
    let identity = args.get("identity").and_then(Value::as_str).unwrap_or("").trim().to_string();
    if identity.is_empty() || identity.len() > 64 || !identity.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '@' | ':')) {
        return Err("identity: letters, digits, - _ . @ :".into());
    }
    let text = |k: &str| args.get(k).and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty());
    let role = match text("role") {
        Some("agent") => "agent",
        _ => "user",
    };
    let trust = match text("trust") {
        Some("root") => "root",
        Some("normal") => "normal",
        _ if identity == "root" => "root",
        _ => "normal",
    };
    let from = cidrs(args, "from")?;
    let generated = text("password").is_none();
    let password = match text("password") {
        Some(p) => p.to_string(),
        None => file::generate_password().map_err(|e| format!("{e:#}"))?,
    };
    file::check_password(&password).map_err(|e| format!("{e:#}"))?;

    // what was asked for first; then what the certificate already names, then this machine's own
    let (mut names, mut ips) = pki.server_extra();
    let mut dial: Vec<String> = args.get("addresses").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
    if let Some(one) = text("address") {
        dial.insert(0, one.to_string());
    }
    let mut added = false;
    let mut addresses = vec![];
    for a in &dial {
        let (host, shown) = address_of(a, port)?;
        // a client refuses a server certificate that does not name the address it dialled
        match host.parse::<std::net::IpAddr>() {
            Ok(ip) if !ips.contains(&ip) => {
                ips.push(ip);
                added = true;
            }
            Err(_) if !names.contains(&host) => {
                names.push(host);
                added = true;
            }
            _ => {}
        }
        addresses.push(shown);
    }
    if dial.is_empty() {
        for n in &names {
            addresses.push(address_of(n, port)?.1);
        }
        for ip in &ips {
            addresses.push(address_of(&ip.to_string(), port)?.1);
        }
        for o in crate::listen::https_origins(port) {
            if let Ok((_, a)) = address_of(&o, port) {
                if !addresses.contains(&a) && !a.starts_with("127.") && !a.starts_with("localhost") {
                    addresses.push(a);
                }
            }
        }
    }
    if addresses.is_empty() {
        return Err("this machine has no address yet: pass address, how the other side reaches it".into());
    }
    if added {
        pki.set_server_extra(&names, &ips).map_err(|e| format!("{e:#}"))?;
    }
    let (p12, meta) = pki.issue_from(&identity, role, trust, &from).map_err(|e| format!("issue: {e:#}"))?;
    let config = BiaConfig {
        // the app lists it under this bia's own name unless told otherwise
        name: text("name").map(str::to_string).or_else(|| pki.chosen_name()).unwrap_or_default(),
        addresses: addresses.clone(),
        ca: pki.ca_pem().map_err(|e| format!("{e:#}"))?,
        p12,
    };
    let sealed = match file::seal(&config, &password) {
        Ok(b) => b,
        Err(e) => {
            // nothing goes out: the certificate made for it must not stay valid
            let _ = pki.revoke(&meta.serial);
            return Err(format!("{e:#}"));
        }
    };
    Ok(MadeConfig { sealed, password, generated, addresses, added, serial: meta.serial, identity: meta.identity, role: meta.role, trust: meta.trust })
}

/// The tool: the same file, kept in files/biaconfig for say(files=…).
pub fn biaconfig(ctx: &Ctx, args: Value) -> Result<String, String> {
    use bia_pki::biaconfig as file;
    let pki = Pki::load_or_create(&files_dir()).map_err(|e| format!("pki: {e:#}"))?;
    let m = make_biaconfig(&pki, web_port(), &args)?;
    let dir = files_dir().join("biaconfig");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let safe: String = m.identity.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect();
    let tail: String = m.serial.chars().take(8).collect();
    let path = dir.join(format!("{safe}-{tail}.{}", file::EXTENSION));
    std::fs::write(&path, &m.sealed).map_err(|e| format!("{}: {e}", path.display()))?;
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        // the agent is root: bia's own backups and say(files=…) must still read it
        if let Ok(owner) = std::fs::metadata(files_dir()) {
            let _ = std::os::unix::fs::chown(&dir, Some(owner.uid()), Some(owner.gid()));
            let _ = std::os::unix::fs::chown(&path, Some(owner.uid()), Some(owner.gid()));
        }
    }
    ctx.log(format!("issued {} cert for {} in a .biaconfig (role={} trust={})", m.serial, m.identity, m.role, m.trust));
    let password_line = if m.generated { format!("password {}", m.password) } else { "the password you were given".to_string() };
    Ok(format!(
        "{}, serial {}, for {}: {} Encrypted with {password_line}. Send the file with say(files=[…]) and tell the password another way, never in the same chat: whoever has both is {} on this machine. web_pki_revoke(serial) ends it at any time. Delete the file once it has arrived. On a phone it opens in the bia deploy app ({}).",
        path.display(),
        m.serial,
        m.addresses.join(", "),
        if m.added { "The server certificate names these addresses within seconds." } else { "" },
        m.identity,
        bia_mcp::apps::stores(),
    ))
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
                format!(" Trust is now set up both ways: {0} is a peer. The link with it opens by itself; talk with say(identity=\"{0}\", channel=\"web\") like with anyone on web.", r.identity)
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
        if let Some(l) = crate::link::links() {
            let d = l.describe(&p.name);
            let waiting = d["waiting"].as_u64().unwrap_or(0);
            out.push(format!(
                "  link: {}  (say(identity=\"{}\", channel=\"web\")){}",
                d["state"].as_str().unwrap_or(""),
                p.name,
                if waiting > 0 { format!("; {waiting} change(s) of yours waiting to go") } else { String::new() }
            ));
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
    if let Some(l) = crate::link::links() {
        l.forget(&name);
    }
    ctx.log(format!("removed peer {} ({})", p.name, p.url));
    Ok(format!(
        "Removed {}. Its link is closed, the certificate this bia gave it is revoked and its certificate for this bia is forgotten here.",
        p.name
    ))
}
