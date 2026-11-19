//! Browser sessions for plain HTTP requests (pages, HITL, the screen, downloads, the root
//! API). A page signs in with the key it keeps: `GET /v1/auth/challenge` gives a one-time
//! nonce, `POST /v1/auth/signin` with the signed `auth` sets the `bia_s` cookie (HttpOnly,
//! so page scripts never hold it), and the page signs in again before it ends. Kept in
//! memory: after a restart the next sign-in makes a new one.

use bia_pki::ClientId;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

pub const COOKIE: &str = "bia_s";
/// A session lasts this long; the live socket hands out a new ticket before it ends.
pub const SESSION: Duration = Duration::from_secs(3600);
const NONCE: Duration = Duration::from_secs(120);

#[derive(Default)]
struct Store {
    nonces: HashMap<String, Instant>,
    sessions: HashMap<String, (ClientId, Instant)>,
}

fn store() -> &'static Mutex<Store> {
    static S: OnceLock<Mutex<Store>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(Store::default()))
}

fn token() -> String {
    use base64::Engine;
    let mut b = [0u8; 32];
    let _ = ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut b);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

fn sweep(s: &mut Store) {
    let now = Instant::now();
    s.nonces.retain(|_, until| *until > now);
    s.sessions.retain(|_, (_, until)| *until > now);
}

/// A one-time sign-in nonce (valid two minutes).
pub fn nonce() -> String {
    let mut s = store().lock().unwrap();
    sweep(&mut s);
    if s.nonces.len() > 10_000 {
        s.nonces.clear();
    }
    let n = bia_pki::signin::nonce();
    s.nonces.insert(n.clone(), Instant::now() + NONCE);
    n
}

/// Use up a nonce: true when it was handed out and not used yet.
pub fn take_nonce(n: &str) -> bool {
    let mut s = store().lock().unwrap();
    sweep(&mut s);
    s.nonces.remove(n).is_some()
}

/// A new session for `id`; returns its token.
pub fn start(id: &ClientId) -> String {
    let mut s = store().lock().unwrap();
    sweep(&mut s);
    let t = token();
    s.sessions.insert(t.clone(), (id.clone(), Instant::now() + SESSION));
    t
}

/// Who a session token belongs to, if it is still valid.
pub fn lookup(token: &str) -> Option<ClientId> {
    let mut s = store().lock().unwrap();
    sweep(&mut s);
    s.sessions.get(token).map(|(id, _)| id.clone())
}

pub fn end(token: &str) {
    store().lock().unwrap().sessions.remove(token);
}

/// End every session of a revoked certificate.
pub fn end_serial(serial: &str) {
    let mut s = store().lock().unwrap();
    s.sessions.retain(|_, (id, _)| id.serial_hint != serial);
}

/// The `bia_s` value in a Cookie header.
pub fn from_cookie_header(h: &str) -> Option<&str> {
    h.split(';')
        .filter_map(|p| p.trim().split_once('='))
        .find(|(k, _)| *k == COOKIE)
        .map(|(_, v)| v.trim())
}

pub fn set_cookie(token: &str) -> String {
    format!(
        "Set-Cookie: {COOKIE}={token}; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age={}",
        SESSION.as_secs()
    )
}

pub fn clear_cookie() -> String {
    format!("Set-Cookie: {COOKIE}=; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=0")
}

/// Whether a cookie-authenticated request comes from this bia's own page: `Origin` names
/// the host the request was sent to. Required for websockets and for anything but
/// GET/HEAD (a page elsewhere cannot use the cookie to act).
pub fn same_origin(origin: Option<&str>, host: Option<&str>, method: &str, websocket: bool) -> bool {
    match origin {
        Some(o) => {
            let Some(h) = host else { return false };
            o.eq_ignore_ascii_case(&format!("https://{h}"))
                || (h.ends_with(":443") && o.eq_ignore_ascii_case(&format!("https://{}", h.trim_end_matches(":443"))))
        }
        None => !websocket && (method == "GET" || method == "HEAD"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(serial: &str) -> ClientId {
        ClientId {
            identity: "root".into(),
            role: "user".into(),
            trust: "root".into(),
            serial_hint: serial.into(),
            not_after: 0,
        }
    }

    #[test]
    fn a_nonce_works_once_and_a_session_ends_with_its_certificate() {
        let n = nonce();
        assert!(take_nonce(&n));
        assert!(!take_nonce(&n));
        assert!(!take_nonce("made-up"));
        let s = start(&id("s1"));
        assert_eq!(lookup(&s).unwrap().identity, "root");
        end_serial("s1");
        assert!(lookup(&s).is_none());
        assert!(lookup("nonsense").is_none());
    }

    #[test]
    fn cookies_and_origins() {
        assert_eq!(from_cookie_header("a=1; bia_s=xyz; b=2"), Some("xyz"));
        assert_eq!(from_cookie_header("a=1"), None);
        assert!(same_origin(Some("https://10.0.0.5"), Some("10.0.0.5"), "POST", true));
        assert!(same_origin(Some("https://10.0.0.5:8443"), Some("10.0.0.5:8443"), "POST", false));
        assert!(same_origin(Some("https://10.0.0.5"), Some("10.0.0.5:443"), "GET", true));
        assert!(!same_origin(Some("https://evil.example"), Some("10.0.0.5"), "GET", true));
        assert!(!same_origin(None, Some("10.0.0.5"), "GET", true));
        assert!(!same_origin(None, Some("10.0.0.5"), "POST", false));
        assert!(same_origin(None, Some("10.0.0.5"), "GET", false));
    }
}
