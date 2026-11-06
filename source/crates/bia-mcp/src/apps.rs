//! The bia deploy phone apps: where to get them, and links that open bia's web pages inside them.
//!
//! An app link is `https://ownbia.com/open#ca=<id>&at=<host[:port]>,…&go=<path>&t=<token>`. Messengers make it
//! clickable (a custom scheme they would not). On a phone with the app it opens there (universal
//! link / App Link); the app picks its saved bia whose CA matches `ca` and loads `go` on it with
//! that bia's key. Without the app, ownbia.com/open offers the stores and the direct links. The
//! fragment never reaches ownbia.com's server.
//!
//! `t` lets a phone without that bia's key get one: a single-use grant minted here
//! (files/state/applink/<token>.json: the identity the link was sent to and its trust). The
//! web MCP redeems it at `GET /v1/pki/link/<token>` by issuing a new certificate for that
//! identity (a key made there, a .p12), and forgets the token. Links go only to people bia
//! knows (root, or addressbook contacts, with their trust), in their own chats. `ca` (128 bits
//! of the CA's hash) lets the phone pin bia's CA before it has any key.

pub const OPEN_BASE: &str = "https://ownbia.com/open";
pub const ANDROID: &str = "https://play.google.com/store/apps/details?id=biadeploy.android";
pub const IPHONE: &str = "https://apps.apple.com/app/bia-deploy/id6827948196";

/// The store links in one sentence for the model.
pub fn stores() -> String {
    format!("bia deploy is the phone app: Android {ANDROID}, iPhone {IPHONE}")
}

/// What the person does when the app has no key for this bia, in words for the model.
pub const NO_KEY: &str = "The link also brings a one-time key for that person (a new certificate when first opened on a phone without one), so send it only to them, in their own chat. That key lapses after 10 minutes (enough to install the app); if they say the app found no key, make them a new link";

/// A link's key grant is redeemable this long: time to install the app and open the link.
pub const TOKEN_TTL_SECS: u64 = 10 * 60;

/// Id of a CA as the apps compute it: the first 32 hex digits (128 bits) of sha256 of the CA
/// certificate's DER (the caller hashes; this crate has no digest).
pub fn ca_id(sha256: &[u8]) -> String {
    sha256.iter().take(16).map(|b| format!("{b:02x}")).collect()
}

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
        let keep = b.is_ascii_alphanumeric() || b"-._~:/".contains(&b);
        if keep {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `host[:port]` of an origin like `https://203.0.113.5:8443/`.
fn host_of(origin: &str) -> Option<String> {
    let rest = origin.split_once("://").map(|(_, r)| r).unwrap_or(origin);
    let host = rest.split(['/', '?', '#']).next()?.trim();
    (!host.is_empty()).then(|| host.to_string())
}

/// The app link to `path` (for example `/hitl/ab12cd34`) on the bia with CA `ca` (from
/// [`ca_id`]) at `origins`, with key grant `token` if any. Loopback addresses mean nothing
/// on a phone and are left out.
pub fn open_link(ca: &str, origins: &[String], path: &str, token: Option<&str>) -> Option<String> {
    if ca.is_empty() || !path.starts_with('/') {
        return None;
    }
    let at: Vec<String> = origins
        .iter()
        .filter_map(|o| host_of(o))
        .filter(|h| !h.starts_with("127.") && !h.starts_with("[::1]") && !h.starts_with("localhost"))
        .map(|h| enc(&h))
        .collect();
    let mut link = format!("{OPEN_BASE}#ca={}", enc(ca));
    if !at.is_empty() {
        link.push_str("&at=");
        link.push_str(&at.join(","));
    }
    link.push_str("&go=");
    link.push_str(&enc(path));
    if let Some(t) = token.filter(|t| valid_token(t)) {
        link.push_str("&t=");
        link.push_str(t);
    }
    Some(link)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_has_ca_addresses_and_path() {
        let o = vec![
            "https://127.0.0.1:8443".to_string(),
            "https://203.0.113.5:8443".to_string(),
            "https://192.168.1.5:8443/".to_string(),
            "https://[2001:db8::1]:8443".to_string(),
        ];
        assert_eq!(
            open_link("0123456789abcdef", &o, "/hitl/ab12cd34", Some(&"ab".repeat(16))).unwrap(),
            format!("https://ownbia.com/open#ca=0123456789abcdef&at=203.0.113.5:8443,192.168.1.5:8443,%5B2001:db8::1%5D:8443&go=/hitl/ab12cd34&t={}", "ab".repeat(16))
        );
    }

    #[test]
    fn link_without_addresses_still_names_the_bia() {
        assert_eq!(
            open_link("0123456789abcdef", &[], "/dl/x y", Some("short")).unwrap(),
            "https://ownbia.com/open#ca=0123456789abcdef&go=/dl/x%20y"
        );
        assert!(open_link("", &[], "/dl/x", None).is_none());
        assert!(open_link("ab", &[], "dl/x", None).is_none());
    }

    #[test]
    fn ca_id_is_32_hex() {
        assert_eq!(ca_id(&[0xab; 32]), "ab".repeat(16));
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
