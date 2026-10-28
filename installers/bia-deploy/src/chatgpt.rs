//! "Continue with ChatGPT": OpenAI's Sign in with ChatGPT for open-source apps, so the user's
//! ChatGPT plan pays for bia's cloud brain (developers.openai.com/siwc/token-sharing-open-source).
//!
//! The flasher runs the browser sign-in (PKCE, loopback callback on 127.0.0.1) and writes the
//! tokens into the stick's fmm.json. bia then owns them: it refreshes the one-hour access token
//! and rotates the 30-day refresh token itself (`bia-agent` `remote.rs`). A sign-in is used for
//! one stick only; two copies of a rotating refresh token would invalidate each other.

use anyhow::{bail, Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const AUTHORIZE_URL: &str = "https://auth.openai.com/api/accounts/authorize";
pub const TOKEN_URL: &str = "https://auth.openai.com/api/accounts/oauth/token";
pub const RESOURCE: &str = "https://api.openai.com/v1";
pub const SCOPES: &str = "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
/// The scope that lets requests bill the user's ChatGPT plan.
pub const PLAN_SCOPE: &str = "chatgpt.tokens.use.direct";
const ISSUER: &str = "https://auth.openai.com";
const CALLBACK_PATH: &str = "/auth/callback";

/// Token set as stored in fmm.json (`chatgpt` object); the agent reads the same shape.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChatGptAuth {
    /// Client ID issued at first registration (never `dynamic_agent_client`).
    pub client_id: String,
    /// Stable per-host ID sent at authorization. bia replaces the flasher's with its own.
    pub host_id: String,
    pub access_token: String,
    pub refresh_token: String,
    #[serde(default)]
    pub id_token: String,
    /// Unix seconds when the access token expires.
    pub expires_at: u64,
    /// Unix seconds of the last sign-in or refresh (bia refreshes daily to keep the session alive).
    #[serde(default)]
    pub refreshed_at: u64,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub scopes: Vec<String>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn random_b64(n: usize) -> String {
    let mut b = vec![0u8; n];
    getrandom::getrandom(&mut b).expect("system randomness");
    URL_SAFE_NO_PAD.encode(b)
}

fn uuid_urn() -> String {
    let mut b = [0u8; 16];
    getrandom::getrandom(&mut b).expect("system randomness");
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("urn:uuid:{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

fn enc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn dec(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    Ok(v) => {
                        out.push(v);
                        i += 2;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// This computer's saved registration: its host ID and, per account, the issued client ID and
/// last ID token (for `id_token_hint`). Tokens are not kept here; they go onto the stick.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Local {
    host_id: String,
    #[serde(default)]
    accounts: Vec<LocalAccount>,
    /// Client ID issued by a registration whose code exchange has not succeeded yet. The next
    /// sign-in reuses it instead of registering yet another client.
    #[serde(default)]
    pending_client_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LocalAccount {
    email: String,
    client_id: String,
    #[serde(default)]
    id_token: String,
}

fn local_path() -> Option<PathBuf> {
    Some(crate::app_dir()?.join("chatgpt.json"))
}

fn load_local() -> Local {
    let mut l: Local = local_path()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|d| serde_json::from_slice(&d).ok())
        .unwrap_or_default();
    if l.host_id.is_empty() {
        l.host_id = uuid_urn();
        save_local(&l);
    }
    l
}

fn save_local(l: &Local) {
    let Some(p) = local_path() else { return };
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let tmp = p.with_extension("json.tmp");
    let ok = (|| -> std::io::Result<()> {
        let mut f = open_private(&tmp)?;
        f.write_all(&serde_json::to_vec_pretty(l).unwrap_or_default())?;
        f.sync_all()
    })();
    if ok.is_ok() {
        let _ = std::fs::rename(&tmp, &p);
    }
}

/// Create/truncate a file readable by the owner only (0600 on Unix).
pub fn open_private(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(path)
}

/// The request target of one HTTP request, after its headers (closing with unread bytes
/// resets the connection and the browser can lose the reply). `None` when the connection
/// sends nothing: such a socket is closed without an answer.
fn read_target(sock: &std::net::TcpStream) -> Option<String> {
    let _ = sock.set_read_timeout(Some(Duration::from_secs(600)));
    let mut r = BufReader::new(sock);
    let mut line = String::new();
    r.read_line(&mut line).ok()?;
    let target = line.split_whitespace().nth(1)?.to_string();
    loop {
        let mut h = String::new();
        match r.read_line(&mut h) {
            Ok(0) | Err(_) => break,
            Ok(_) if h.trim().is_empty() => break,
            Ok(_) => {}
        }
    }
    Some(target)
}

/// How an app opens the sign-in page itself (the phone apps: a Custom Tab, an in-app
/// Safari view); without one the system browser is started.
static OPENER: std::sync::OnceLock<Box<dyn Fn(&str) -> bool + Send + Sync>> = std::sync::OnceLock::new();

pub fn set_browser_opener(f: Box<dyn Fn(&str) -> bool + Send + Sync>) {
    let _ = OPENER.set(f);
}

pub fn open_browser(url: &str) -> Result<()> {
    if let Some(open) = OPENER.get() {
        return if open(url) { Ok(()) } else { bail!("could not open the sign-in page") };
    }
    let status = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(url).status()
    } else if cfg!(target_os = "windows") {
        // `start` would split the URL at every `&`.
        std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", url])
            .status()
    } else {
        std::process::Command::new("xdg-open").arg(url).status()
    };
    match status {
        Ok(s) if s.success() => Ok(()),
        _ => bail!("could not open a browser"),
    }
}

fn http() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .http_status_as_error(false)
        .user_agent("bia-deploy")
        .build()
        .into()
}

/// A request that could not reach the server is tried again for a while. On a phone the
/// sign-in page is in front when the token is fetched, and Android keeps a background app
/// off the network (Data Saver, battery savers: the name lookup fails) until it is back.
fn send_retry(
    what: &str,
    mut send: impl FnMut() -> Result<ureq::http::Response<ureq::Body>, ureq::Error>,
) -> Result<ureq::http::Response<ureq::Body>> {
    let end = std::time::Instant::now() + Duration::from_secs(90);
    loop {
        match send() {
            Ok(r) => return Ok(r),
            Err(ureq::Error::Io(_) | ureq::Error::HostNotFound | ureq::Error::ConnectionFailed) if std::time::Instant::now() < end => {
                std::thread::sleep(Duration::from_secs(3))
            }
            Err(e) => return Err(anyhow::Error::new(e).context(format!("reach {what}"))),
        }
    }
}

/// POST a form to the token endpoint; returns the JSON body or the OAuth error.
fn token_request(form: &[(&str, &str)]) -> Result<Value> {
    let body = form
        .iter()
        .map(|(k, v)| format!("{}={}", enc(k), enc(v)))
        .collect::<Vec<_>>()
        .join("&");
    let resp = send_retry("auth.openai.com", || {
        http().post(TOKEN_URL).header("Content-Type", "application/x-www-form-urlencoded").send(&body)
    })?;
    let status = resp.status().as_u16();
    let text = resp.into_body().read_to_string().unwrap_or_default();
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if status >= 400 {
        let code = v["error"].as_str().unwrap_or("error");
        let msg = v["error_description"].as_str().unwrap_or("");
        return Err(OAuthError {
            code: code.to_string(),
            message: format!("ChatGPT sign-in: {code} {msg} (HTTP {status})"),
        }
        .into());
    }
    Ok(v)
}

#[derive(Debug)]
struct OAuthError {
    code: String,
    message: String,
}

impl std::fmt::Display for OAuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for OAuthError {}

fn is_invalid_grant(e: &anyhow::Error) -> bool {
    e.downcast_ref::<OAuthError>().is_some_and(|o| o.code == "invalid_grant")
}

/// Payload of a JWT, unverified. The ID token comes straight from the token endpoint over TLS,
/// so (OIDC Core 3.1.3.7) TLS authenticates the issuer; we still check iss/aud/exp/nonce.
fn jwt_claims(jwt: &str) -> Option<Value> {
    let payload = jwt.split('.').nth(1)?;
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?).ok()
}

fn apply_tokens(auth: &mut ChatGptAuth, v: &Value) -> Result<()> {
    let access = v["access_token"].as_str().context("no access_token in reply")?;
    auth.access_token = access.to_string();
    if let Some(r) = v["refresh_token"].as_str() {
        auth.refresh_token = r.to_string();
    }
    if let Some(i) = v["id_token"].as_str() {
        auth.id_token = i.to_string();
    }
    let t = now();
    auth.expires_at = t + v["expires_in"].as_u64().unwrap_or(3600);
    auth.refreshed_at = t;
    if let Some(s) = v["scope"].as_str() {
        auth.scopes = s.split_whitespace().map(str::to_string).collect();
    }
    Ok(())
}

/// Run the browser sign-in. `on_status` gets short progress lines for the UI.
pub fn sign_in(login_hint: Option<&str>, on_status: &mut dyn FnMut(&str)) -> Result<ChatGptAuth> {
    let mut local = load_local();
    let mut known = login_hint
        .and_then(|e| local.accounts.iter().find(|a| a.email == e).cloned())
        .or_else(|| {
            (!local.pending_client_id.is_empty()).then(|| LocalAccount {
                email: login_hint.unwrap_or("").to_string(),
                client_id: local.pending_client_id.clone(),
                id_token: String::new(),
            })
        });
    // OpenAI: on invalid_grant, discard the code and authorize again with the issued client ID.
    for attempt in 0..2 {
        match sign_in_once(&mut local, known.as_ref(), login_hint, on_status) {
            Err(e) if attempt == 0 && is_invalid_grant(&e) && !local.pending_client_id.is_empty() => {
                on_status("ChatGPT wants one more confirmation: continue in your browser …");
                known = Some(LocalAccount {
                    email: login_hint.unwrap_or("").to_string(),
                    client_id: local.pending_client_id.clone(),
                    id_token: String::new(),
                });
            }
            r => return r,
        }
    }
    unreachable!()
}

fn sign_in_once(
    local: &mut Local,
    known: Option<&LocalAccount>,
    login_hint: Option<&str>,
    on_status: &mut dyn FnMut(&str),
) -> Result<ChatGptAuth> {

    // The listener must be up before the browser opens. 1455 is the documented port; any
    // other loopback port works as long as the redirect URI matches exactly.
    let listener = TcpListener::bind("127.0.0.1:1455")
        .or_else(|_| TcpListener::bind("127.0.0.1:0"))
        .context("open a local port for the sign-in callback")?;
    let port = listener.local_addr()?.port();
    let redirect = format!("http://127.0.0.1:{port}{CALLBACK_PATH}");

    let verifier = random_b64(48);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = random_b64(24);
    let nonce = random_b64(24);
    let mut params: Vec<(&str, String)> = vec![
        ("response_type", "code".into()),
        ("redirect_uri", redirect.clone()),
        ("scope", SCOPES.into()),
        ("resource", RESOURCE.into()),
        ("state", state.clone()),
        ("nonce", nonce.clone()),
        ("code_challenge_method", "S256".into()),
        ("code_challenge", challenge),
        ("ext_agent_host_id", local.host_id.clone()),
    ];
    match known {
        Some(a) => {
            params.push(("client_id", a.client_id.clone()));
            if !a.id_token.is_empty() {
                params.push(("id_token_hint", a.id_token.clone()));
            }
            if !a.email.is_empty() {
                params.push(("login_hint", a.email.clone()));
            }
        }
        None => {
            params.push(("client_id", "dynamic_agent_client".into()));
            params.push(("agent_name_hint", "bia".into()));
            if let Some(e) = login_hint {
                params.push(("login_hint", e.to_string()));
            }
        }
    }
    let url = format!(
        "{AUTHORIZE_URL}?{}",
        params
            .iter()
            .map(|(k, v)| format!("{k}={}", enc(v)))
            .collect::<Vec<_>>()
            .join("&")
    );
    on_status("opening ChatGPT sign-in in your browser …");
    if open_browser(&url).is_err() {
        on_status(&format!("open this address in your browser: {url}"));
    }

    // Wait for the redirect (5 minutes), ignoring stray requests such as /favicon.ico. Each
    // connection is read on its own thread: browsers open connections before they have a
    // request for them (Chrome preconnects to an address it has seen, so a second sign-in)
    // and send the callback on one much later. Answering such a connection early with a 404
    // would make that 404 the reply to the callback.
    let (tx, rx) = std::sync::mpsc::channel::<(String, std::net::TcpStream)>();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let stop = stop.clone();
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                if stop.load(std::sync::atomic::Ordering::Relaxed) {
                    return;
                }
                let Ok(sock) = conn else { continue };
                let tx = tx.clone();
                std::thread::spawn(move || {
                    if let Some(target) = read_target(&sock) {
                        let _ = tx.send((target, sock));
                    }
                });
            }
        });
    }
    // Free the port when this sign-in ends, however it ends.
    struct Stop(std::sync::Arc<std::sync::atomic::AtomicBool>, u16);
    impl Drop for Stop {
        fn drop(&mut self) {
            self.0.store(true, std::sync::atomic::Ordering::Relaxed);
            let _ = std::net::TcpStream::connect(("127.0.0.1", self.1));
        }
    }
    let _stop = Stop(stop, port);
    let deadline = std::time::Instant::now() + Duration::from_secs(300);
    let query = loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            bail!("ChatGPT sign-in timed out");
        }
        let Ok((target, mut sock)) = rx.recv_timeout(left) else {
            bail!("ChatGPT sign-in timed out");
        };
        let Some(q) = target.strip_prefix(CALLBACK_PATH).and_then(|r| r.strip_prefix('?')) else {
            let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            continue;
        };
        let q: Vec<(String, String)> = q
            .split('&')
            .filter_map(|kv| kv.split_once('='))
            .map(|(k, v)| (dec(k), dec(v)))
            .collect();
        let ok = q.iter().any(|(k, _)| k == "code");
        let page = if ok {
            "<h2>bia is signed in to ChatGPT.</h2><p>You can close this tab and go back to bia deploy.</p>"
        } else {
            "<h2>Sign-in was not completed.</h2><p>Go back to bia deploy to try again.</p>"
        };
        let html = format!("<!doctype html><meta charset=utf-8><title>bia</title><body style=\"font-family:sans-serif;margin:3em\">{page}</body>");
        let _ = sock.write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}",
                html.len()
            )
            .as_bytes(),
        );
        break q;
    };
    let get = |k: &str| query.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
    if get("state").as_deref() != Some(state.as_str()) {
        bail!("ChatGPT sign-in: state mismatch; try again");
    }
    if let Some(e) = get("error") {
        if e == "access_denied" {
            bail!("ChatGPT sign-in was declined");
        }
        bail!("ChatGPT sign-in: {e}");
    }
    let code = get("code").context("ChatGPT sign-in: no code returned")?;
    let issued = get("client_id").filter(|c| !c.is_empty() && c != "dynamic_agent_client");
    if let (Some(k), Some(i)) = (known, &issued) {
        if *i != k.client_id {
            bail!("ChatGPT sign-in returned a different client; try again");
        }
    }
    let client_id = issued
        .or_else(|| known.map(|a| a.client_id.clone()))
        .context("ChatGPT sign-in: no client_id returned")?;
    if known.is_none() {
        // Keep a new registration's client ID even if the exchange below fails.
        local.pending_client_id = client_id.clone();
        save_local(local);
    }

    on_status("finishing sign-in …");
    let v = token_request(&[
        ("grant_type", "authorization_code"),
        ("client_id", &client_id),
        ("code", &code),
        ("code_verifier", &verifier),
        ("redirect_uri", &redirect),
        ("resource", RESOURCE),
    ])?;
    let mut auth = ChatGptAuth {
        client_id: client_id.clone(),
        host_id: local.host_id.clone(),
        ..Default::default()
    };
    apply_tokens(&mut auth, &v)?;
    if auth.scopes.is_empty() {
        if let Some(s) = get("scope") {
            auth.scopes = s.split_whitespace().map(str::to_string).collect();
        }
    }
    let claims = jwt_claims(&auth.id_token).context("ChatGPT sign-in: no valid ID token")?;
    let aud_ok = match &claims["aud"] {
        Value::String(a) => *a == client_id,
        Value::Array(a) => a.iter().any(|x| x.as_str() == Some(&client_id)),
        _ => false,
    };
    if claims["iss"].as_str().map(|s| s.trim_end_matches('/')) != Some(ISSUER)
        || !aud_ok
        || claims["nonce"].as_str() != Some(nonce.as_str())
        || claims["exp"].as_u64().unwrap_or(0) < now()
    {
        bail!("ChatGPT sign-in: the ID token did not check out; try again");
    }
    auth.email = claims["email"].as_str().unwrap_or("").to_string();

    local.pending_client_id.clear();
    local.accounts.retain(|a| a.email != auth.email);
    local.accounts.push(LocalAccount {
        email: auth.email.clone(),
        client_id,
        id_token: auth.id_token.clone(),
    });
    save_local(local);

    if !auth.scopes.iter().any(|s| s == PLAN_SCOPE) {
        bail!(
            "signed in as {}, but ChatGPT plan use was not allowed. Sign in again and allow it, or use an API key",
            auth.email
        );
    }
    Ok(auth)
}

/// Refresh if the access token expires within five minutes. The refresh token rotates:
/// the returned set replaces the old one entirely.
pub fn ensure_fresh(auth: &mut ChatGptAuth) -> Result<bool> {
    if auth.expires_at > now() + 300 {
        return Ok(false);
    }
    let v = token_request(&[
        ("grant_type", "refresh_token"),
        ("client_id", &auth.client_id),
        ("refresh_token", &auth.refresh_token),
        ("resource", RESOURCE),
    ])?;
    apply_tokens(auth, &v)?;
    Ok(true)
}

/// The ChatGPT model bia suggests: the id containing this, shortest first (`gpt-6.1-sol`
/// before a longer variant). bia's own sign-in (`bia-mcp-system` chatgpt.rs) uses the same.
pub const DEFAULT_MODEL: &str = "6.1-sol";

/// The default model among those the account may use, if it is there.
pub fn default_model(models: &[crate::fmm::CloudModel]) -> Option<&crate::fmm::CloudModel> {
    models
        .iter()
        .filter(|m| m.id.contains(DEFAULT_MODEL))
        .min_by_key(|m| m.id.len())
}

/// Models this ChatGPT account may use (`visibility == "list"`).
pub fn list_models(access_token: &str) -> Result<Vec<crate::fmm::CloudModel>> {
    let resp = send_retry("api.openai.com", || {
        http().get(&format!("{RESOURCE}/models")).header("Authorization", &format!("Bearer {access_token}")).call()
    })?;
    let status = resp.status().as_u16();
    let text = resp.into_body().read_to_string().unwrap_or_default();
    if status >= 400 {
        bail!("ChatGPT model list: HTTP {status}: {}", text.chars().take(300).collect::<String>());
    }
    let v: Value = serde_json::from_str(&text).context("parse ChatGPT model list")?;
    let items = v["data"]
        .as_array()
        .or_else(|| v["models"].as_array())
        .or_else(|| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut out: Vec<crate::fmm::CloudModel> = items
        .iter()
        .filter(|m| m["visibility"].as_str().map_or(true, |s| s == "list"))
        .filter_map(|m| {
            let id = m["slug"].as_str().or_else(|| m["id"].as_str())?.to_string();
            Some(crate::fmm::CloudModel {
                name: m["display_name"].as_str().unwrap_or(&id).to_string(),
                ctx: m["context_window"].as_u64().map_or(0, |n| n.min(u32::MAX as u64) as u32),
                id,
            })
        })
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert_eq!(dec("a%20b+c%2Fd"), "a b c/d");
        assert_eq!(enc("a b&c"), "a%20b%26c");
        let u = uuid_urn();
        assert!(u.starts_with("urn:uuid:") && u.len() == 45, "{u}");
        let jwt = format!("x.{}.y", URL_SAFE_NO_PAD.encode(br#"{"aud":"c1","nonce":"n"}"#));
        assert_eq!(jwt_claims(&jwt).unwrap()["aud"], "c1");
        let m = |id: &str| crate::fmm::CloudModel { id: id.into(), name: id.into(), ctx: 0 };
        let list = vec![m("gpt-5"), m("gpt-6.1-sol-mini"), m("gpt-6.1-sol")];
        assert_eq!(default_model(&list).unwrap().id, "gpt-6.1-sol");
        assert!(default_model(&list[..1]).is_none());
    }
}
