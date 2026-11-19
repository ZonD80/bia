//! Links that open bia's web pages for a person, with a key if they have none yet.
//!
//! A link is `https://<bia's address><path>#enroll=<token>`: bia's own page, at its best
//! address (a hostname with a trusted certificate first, then a public address). The fragment
//! never reaches any server's logs. A browser that already has a key for this bia signs in
//! as usual and ignores the token. One without signs up with it: it makes its own key and sends
//! a CSR to `POST /v1/pki/link/<token>`, which certifies it for the identity the link was sent
//! to, once (files/state/applink/<token>.json: identity and trust). Links with a key go only
//! to people bia knows (root, or addressbook contacts, with their trust), in their own chats.
//!
//! The bia deploy phone apps hold bias' keys and open them in the browser; they are where a
//! phone gets a key without a link.

pub const ANDROID: &str = "https://play.google.com/store/apps/details?id=biadeploy.android";
pub const IPHONE: &str = "https://apps.apple.com/app/bia-deploy/id6827948196";

/// The store links in one sentence for the model.
pub fn stores() -> String {
    format!("bia deploy is the phone app: Android {ANDROID}, iPhone {IPHONE}")
}

/// What the key in a link means, in words for the model.
pub const NO_KEY: &str = "The link also brings a one-time key for that person (a browser without one makes its own when the link is first opened), so send it only to them, in their own chat. That key lapses after 10 minutes; if they say the page asked for a key, make them a new link";

/// A link's key grant is redeemable this long.
pub const TOKEN_TTL_SECS: u64 = 10 * 60;

pub fn tokens_dir(files: &std::path::Path) -> std::path::PathBuf {
    files.join("state").join("applink")
}

/// Whether `t` looks like a token at all (32 lowercase hex digits).
pub fn valid_token(t: &str) -> bool {
    t.len() == 32 && t.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Who a link's key is for, and with which trust (root or normal).
#[derive(Debug, Clone, PartialEq)]
pub struct Grant {
    pub identity: String,
    pub trust: String,
}

/// A new single-use key grant for `identity` with `trust` under `files`; lapsed ones are
/// swept. None without randomness or a writable folder.
pub fn mint_token(files: &std::path::Path, identity: &str, trust: &str) -> Option<String> {
    use std::io::Read;
    let mut b = [0u8; 16];
    std::fs::File::open("/dev/urandom").ok()?.read_exact(&mut b).ok()?;
    let t: String = b.iter().map(|x| format!("{x:02x}")).collect();
    let dir = tokens_dir(files);
    std::fs::create_dir_all(&dir).ok()?;
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            if grant_at(&e.path()).is_none() {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let rec = serde_json::json!({"identity": identity, "trust": trust, "created": now()});
    let path = dir.join(format!("{t}.json"));
    std::fs::write(&path, rec.to_string()).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Some(t)
}

fn grant_at(path: &std::path::Path) -> Option<Grant> {
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    if now().saturating_sub(v.get("created")?.as_u64()?) > TOKEN_TTL_SECS {
        return None;
    }
    let identity = v.get("identity")?.as_str()?.to_string();
    let trust = match v.get("trust").and_then(|x| x.as_str()) {
        Some("root") => "root",
        _ => "normal",
    };
    (!identity.is_empty()).then(|| Grant { identity, trust: trust.into() })
}

/// Redeem token `t` once: its grant if it is live, and it is gone either way.
pub fn take_token(files: &std::path::Path, t: &str) -> Option<Grant> {
    if !valid_token(t) {
        return None;
    }
    let path = tokens_dir(files).join(format!("{t}.json"));
    let g = grant_at(&path);
    let _ = std::fs::remove_file(&path);
    g
}

fn enc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        let keep = b.is_ascii_alphanumeric() || b"-._~:/?=&".contains(&b);
        if keep {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `host` of an origin like `https://203.0.113.5:8443/` (no port, no brackets).
fn host_of(origin: &str) -> Option<String> {
    let rest = origin.split_once("://").map(|(_, r)| r).unwrap_or(origin);
    let hostport = rest.split(['/', '?', '#']).next()?.trim();
    let host = if let Some(v6) = hostport.strip_prefix('[') {
        v6.split(']').next()?.to_string()
    } else {
        hostport.rsplit_once(':').map(|(h, _)| h).unwrap_or(hostport).to_string()
    };
    (!host.is_empty()).then_some(host)
}

/// How good an address is for a link: a hostname (trusted certificate), a public IP, a
/// private one; loopback never.
fn rank(origin: &str) -> Option<u8> {
    let h = host_of(origin)?;
    match h.parse::<std::net::IpAddr>() {
        Err(_) if h == "localhost" => None,
        Err(_) => Some(0),
        Ok(ip) if ip.is_loopback() => None,
        Ok(std::net::IpAddr::V4(v4)) if v4.is_private() || v4.is_link_local() || (v4.octets()[0] == 100 && (v4.octets()[1] & 0xc0) == 64) => Some(2),
        Ok(std::net::IpAddr::V6(v6)) if (v6.segments()[0] & 0xfe00) == 0xfc00 || (v6.segments()[0] & 0xffc0) == 0xfe80 => Some(2),
        Ok(_) => Some(1),
    }
}

/// The link to `path` (for example `/hitl/ab12cd34`) on this bia at the best of `origins`,
/// with key grant `token` if any.
pub fn page_link(origins: &[String], path: &str, token: Option<&str>) -> Option<String> {
    if !path.starts_with('/') {
        return None;
    }
    let best = origins.iter().filter_map(|o| Some((rank(o)?, o))).min_by_key(|(r, _)| *r)?.1;
    let mut link = format!("{}{}", best.trim_end_matches('/'), enc(path));
    if let Some(t) = token.filter(|t| valid_token(t)) {
        link.push(if link.contains('#') { '&' } else { '#' });
        link.push_str("enroll=");
        link.push_str(t);
    }
    Some(link)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_goes_to_the_best_address() {
        let o = vec![
            "https://127.0.0.1:8443".to_string(),
            "https://192.168.1.5:8443/".to_string(),
            "https://203.0.113.5:8443".to_string(),
            "https://[2001:db8::1]:8443".to_string(),
        ];
        assert_eq!(
            page_link(&o, "/hitl/ab12cd34", Some(&"ab".repeat(16))).unwrap(),
            format!("https://203.0.113.5:8443/hitl/ab12cd34#enroll={}", "ab".repeat(16))
        );
        let mut with_name = o.clone();
        with_name.push("https://bia.example.com".into());
        assert_eq!(page_link(&with_name, "/", None).unwrap(), "https://bia.example.com/");
        assert_eq!(page_link(&o[..2], "/dl/x y", Some("short")).unwrap(), "https://192.168.1.5:8443/dl/x%20y");
        assert!(page_link(&o[..1], "/", None).is_none(), "loopback means nothing elsewhere");
        assert!(page_link(&o, "dl/x", None).is_none());
    }

    #[test]
    fn a_grant_is_redeemed_once() {
        let dir = std::env::temp_dir().join(format!("bia-applink-{}", std::process::id()));
        let t = mint_token(&dir, "root", "root").unwrap();
        assert!(valid_token(&t));
        assert_eq!(take_token(&dir, &t), Some(Grant { identity: "root".into(), trust: "root".into() }));
        assert_eq!(take_token(&dir, &t), None, "single use");
        let t = mint_token(&dir, "ann", "anything").unwrap();
        assert_eq!(take_token(&dir, &t).unwrap().trust, "normal");
        assert_eq!(take_token(&dir, "../x"), None);
        // a lapsed one is refused
        std::fs::write(tokens_dir(&dir).join(format!("{}.json", "1".repeat(32))), r#"{"identity":"root","trust":"root","created":1}"#).unwrap();
        assert_eq!(take_token(&dir, &"1".repeat(32)), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
