//! "Login with ChatGPT" on bia itself: the same Sign in with ChatGPT for open-source apps that
//! bia-deploy does (installers/bia-deploy/src/chatgpt.rs), run as a human task so the owner can
//! move the cloud brain onto their ChatGPT plan without reflashing.
//!
//! Everything happens on bia: this process serves a page at 127.0.0.1 with a "Login with
//! ChatGPT" button and tells the agent to open it in Google Chrome on bia's screen with
//! mcp/chrome (chrome_new_page) and to send the person there. The web console task page
//! (`/hitl/<id>`) shows the same screen through /gui. OpenAI redirects Chrome back here. Without
//! Chrome the tool fails and sends the agent to chrome_install (mcp/chrome), which says how
//! to get the official Google Chrome; a sign-in in the
//! person's own browser is not offered. Then the page (and the redirect tab on bia) offers the
//! account's models; the chosen one goes into fmm.json with the token set, and the agent
//! switches to it. From then on the agent owns the tokens (`remote::chatgpt_token` refreshes
//! and rotates them); this process never refreshes.
//!
//! The web console task page finishes the task with `<id>.result` (`{status, model}`) next to
//! `<id>.json` in state/hitl, as for `ask_human`.
use crate::{fmm, hitl};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use bia_mcp::Ctx;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const AUTHORIZE_URL: &str = "https://auth.openai.com/api/accounts/authorize";
const TOKEN_URL: &str = "https://auth.openai.com/api/accounts/oauth/token";
const RESOURCE: &str = "https://api.openai.com/v1";
const SCOPES: &str =
    "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
/// The scope that lets requests bill the user's ChatGPT plan.
const PLAN_SCOPE: &str = "chatgpt.tokens.use.direct";
const ISSUER: &str = "https://auth.openai.com";
const CALLBACK_PATH: &str = "/auth/callback";

/// Google Chrome as mcp/chrome runs it (as user `bia`).
const CHROME_BINS: &[&str] = &[
    "/usr/bin/google-chrome-stable",
    "/usr/bin/google-chrome",
    "/opt/google/chrome/google-chrome",
    "/opt/google/chrome/chrome",
];

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn random_b64(n: usize) -> String {
    let mut b = vec![0u8; n];
    fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut b))
        .expect("/dev/urandom");
    URL_SAFE_NO_PAD.encode(b)
}

fn uuid_urn() -> String {
    let mut b = [0u8; 16];
    let _ = fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b));
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "urn:uuid:{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
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
            b'%' if i + 2 < b.len() => match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                Ok(v) => {
                    out.push(v);
                    i += 2;
                }
                Err(_) => out.push(b'%'),
            },
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn parse_query(q: &str) -> Vec<(String, String)> {
    q.split('&')
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (dec(k), dec(v)))
        .collect()
}

fn get<'a>(q: &'a [(String, String)], k: &str) -> Option<&'a str> {
    q.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

// ---------------- this machine's registration ----------------

/// bia's own client registration: its host ID and, per account, the issued client ID and last
/// ID token (for `id_token_hint`). Tokens live in fmm.json only.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Local {
    host_id: String,
    #[serde(default)]
    accounts: Vec<Account>,
    /// Issued by a registration whose code exchange has not succeeded yet; reused next time.
    #[serde(default)]
    pending_client_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Account {
    email: String,
    client_id: String,
    #[serde(default)]
    id_token: String,
}

fn local_path() -> PathBuf {
    fmm::state_file().with_file_name("chatgpt-client.json")
}

/// Load the registration. A stick flashed with a ChatGPT sign-in already has a host ID and a
/// client (in fmm.json); signing in again reuses them.
fn load_local(fmm_cfg: Option<&Value>) -> Local {
    let mut l: Local = fs::read(local_path())
        .ok()
        .and_then(|d| serde_json::from_slice(&d).ok())
        .unwrap_or_default();
    let flashed = fmm_cfg.map(|c| &c["chatgpt"]).filter(|a| a.is_object());
    if l.host_id.is_empty() {
        l.host_id = flashed
            .and_then(|a| a["host_id"].as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(uuid_urn);
    }
    if let Some(a) = flashed {
        let acc = Account {
            email: a["email"].as_str().unwrap_or("").into(),
            client_id: a["client_id"].as_str().unwrap_or("").into(),
            id_token: a["id_token"].as_str().unwrap_or("").into(),
        };
        if !acc.client_id.is_empty() && !l.accounts.iter().any(|x| x.client_id == acc.client_id) {
            l.accounts.push(acc);
        }
    }
    save_local(&l);
    l
}

fn save_local(l: &Local) {
    let p = local_path();
    if let Some(d) = p.parent() {
        let _ = fs::create_dir_all(d);
    }
    let tmp = p.with_extension("json.tmp");
    if fs::write(&tmp, serde_json::to_vec_pretty(l).unwrap_or_default()).is_ok() {
        let _ = fs::rename(&tmp, &p);
    }
    unsafe { libc::sync() };
}

// ---------------- OAuth ----------------

/// One authorization request: PKCE verifier, state and nonce, and the browser URL.
struct Attempt {
    state: String,
    nonce: String,
    verifier: String,
    /// The client ID sent (`None`: a first, dynamic registration).
    client_id: Option<String>,
    url: String,
}

fn attempt(
    local: &Local,
    known: Option<&Account>,
    redirect: &str,
    login_hint: Option<&str>,
) -> Attempt {
    let verifier = random_b64(48);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = random_b64(24);
    let nonce = random_b64(24);
    let mut params: Vec<(&str, String)> = vec![
        ("response_type", "code".into()),
        ("redirect_uri", redirect.into()),
        ("scope", SCOPES.into()),
        ("resource", RESOURCE.into()),
        ("state", state.clone()),
        ("nonce", nonce.clone()),
        ("code_challenge_method", "S256".into()),
        ("code_challenge", challenge),
        ("ext_agent_host_id", local.host_id.clone()),
    ];
    let hint = login_hint
        .or(known.map(|a| a.email.as_str()))
        .filter(|s| !s.is_empty());
    match known {
        Some(a) => {
            params.push(("client_id", a.client_id.clone()));
            if !a.id_token.is_empty() {
                params.push(("id_token_hint", a.id_token.clone()));
            }
        }
        None => {
            params.push(("client_id", "dynamic_agent_client".into()));
            params.push(("agent_name_hint", "bia".into()));
        }
    }
    if let Some(h) = hint {
        params.push(("login_hint", h.to_string()));
    }
    let url = format!(
        "{AUTHORIZE_URL}?{}",
        params
            .iter()
            .map(|(k, v)| format!("{k}={}", enc(v)))
            .collect::<Vec<_>>()
            .join("&")
    );
    Attempt {
        state,
        nonce,
        verifier,
        client_id: known.map(|a| a.client_id.clone()),
        url,
    }
}

fn http() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .http_status_as_error(false)
        .user_agent("bia")
        .build()
        .into()
}

fn token_request(form: &[(&str, &str)]) -> Result<Value, String> {
    let body = form
        .iter()
        .map(|(k, v)| format!("{}={}", enc(k), enc(v)))
        .collect::<Vec<_>>()
        .join("&");
    let resp = http()
        .post(TOKEN_URL)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .send(body)
        .map_err(|e| format!("reach auth.openai.com: {e}"))?;
    let status = resp.status().as_u16();
    let text = resp.into_body().read_to_string().unwrap_or_default();
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if status >= 400 {
        let code = v["error"].as_str().unwrap_or("error");
        let msg = v["error_description"].as_str().unwrap_or("");
        return Err(format!("ChatGPT sign-in: {code} {msg} (HTTP {status})"));
    }
    Ok(v)
}

fn jwt_claims(jwt: &str) -> Option<Value> {
    let payload = jwt.split('.').nth(1)?;
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?).ok()
}

/// A finished sign-in: the token set as fmm.json stores it (`chatgpt`), plus the account.
struct Signed {
    auth: Value,
    email: String,
    access: String,
}

/// Turn the redirect query into tokens: check state, exchange the code (PKCE), verify the ID
/// token and that plan use was allowed.
fn redeem(
    local: &mut Local,
    att: &Attempt,
    redirect: &str,
    q: &[(String, String)],
) -> Result<Signed, String> {
    if get(q, "state") != Some(att.state.as_str()) {
        return Err(
            "this sign-in page is old or for another task; press Login with ChatGPT again".into(),
        );
    }
    if let Some(e) = get(q, "error") {
        return Err(if e == "access_denied" {
            "ChatGPT sign-in was declined".into()
        } else {
            format!(
                "ChatGPT sign-in: {e} {}",
                get(q, "error_description").unwrap_or("")
            )
        });
    }
    let code = get(q, "code").ok_or("ChatGPT sign-in: no code in the address")?;
    let issued = get(q, "client_id").filter(|c| !c.is_empty() && *c != "dynamic_agent_client");
    if let (Some(k), Some(i)) = (&att.client_id, issued) {
        if i != k {
            return Err("ChatGPT sign-in returned a different client; try again".into());
        }
    }
    let client_id = issued
        .map(str::to_string)
        .or_else(|| att.client_id.clone())
        .ok_or("ChatGPT sign-in: no client_id returned")?;
    if att.client_id.is_none() {
        // Keep a new registration's client ID even if the exchange below fails.
        local.pending_client_id = client_id.clone();
        save_local(local);
    }
    let v = token_request(&[
        ("grant_type", "authorization_code"),
        ("client_id", &client_id),
        ("code", code),
        ("code_verifier", &att.verifier),
        ("redirect_uri", redirect),
        ("resource", RESOURCE),
    ])?;
    let access = v["access_token"]
        .as_str()
        .ok_or("no access_token in reply")?
        .to_string();
    let refresh = v["refresh_token"]
        .as_str()
        .ok_or("no refresh_token in reply")?
        .to_string();
    let id_token = v["id_token"].as_str().unwrap_or("").to_string();
    let scopes: Vec<String> = v["scope"]
        .as_str()
        .or(get(q, "scope"))
        .unwrap_or("")
        .split_whitespace()
        .map(str::to_string)
        .collect();
    let claims = jwt_claims(&id_token).ok_or("ChatGPT sign-in: no valid ID token")?;
    let aud_ok = match &claims["aud"] {
        Value::String(a) => *a == client_id,
        Value::Array(a) => a.iter().any(|x| x.as_str() == Some(&client_id)),
        _ => false,
    };
    if claims["iss"].as_str().map(|s| s.trim_end_matches('/')) != Some(ISSUER)
        || !aud_ok
        || claims["nonce"].as_str() != Some(att.nonce.as_str())
        || claims["exp"].as_u64().unwrap_or(0) < now()
    {
        return Err("ChatGPT sign-in: the ID token did not check out; try again".into());
    }
    let email = claims["email"].as_str().unwrap_or("").to_string();
    local.pending_client_id.clear();
    local
        .accounts
        .retain(|a| a.email != email && a.client_id != client_id);
    local.accounts.push(Account {
        email: email.clone(),
        client_id: client_id.clone(),
        id_token: id_token.clone(),
    });
    save_local(local);
    if !scopes.iter().any(|s| s == PLAN_SCOPE) {
        return Err(format!(
            "signed in as {email}, but ChatGPT plan use was not allowed. Press Login with ChatGPT again and allow it"
        ));
    }
    let t = now();
    let auth = json!({
        "client_id": client_id,
        "host_id": local.host_id,
        "access_token": access,
        "refresh_token": refresh,
        "id_token": id_token,
        "expires_at": t + v["expires_in"].as_u64().unwrap_or(3600),
        "refreshed_at": t,
        "email": email,
        "scopes": scopes,
    });
    Ok(Signed {
        auth,
        email,
        access,
    })
}

#[derive(Debug, Clone, Serialize, PartialEq)]
struct Model {
    id: String,
    name: String,
    /// context window in tokens, 0 if not given
    ctx: u64,
}

fn list_models(access: &str) -> Result<Vec<Model>, String> {
    let resp = http()
        .get(&format!("{RESOURCE}/models"))
        .header("Authorization", &format!("Bearer {access}"))
        .call()
        .map_err(|e| format!("reach api.openai.com: {e}"))?;
    let status = resp.status().as_u16();
    let text = resp.into_body().read_to_string().unwrap_or_default();
    if status >= 400 {
        return Err(format!(
            "ChatGPT model list: HTTP {status}: {}",
            text.chars().take(300).collect::<String>()
        ));
    }
    let v: Value = serde_json::from_str(&text).map_err(|e| format!("ChatGPT model list: {e}"))?;
    Ok(parse_models(&v))
}

/// Models this account may use (`visibility == "list"`), as bia-deploy lists them.
fn parse_models(v: &Value) -> Vec<Model> {
    let items = v["data"]
        .as_array()
        .or_else(|| v["models"].as_array())
        .or_else(|| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut out: Vec<Model> = items
        .iter()
        .filter(|m| m["visibility"].as_str().is_none_or(|s| s == "list"))
        .filter_map(|m| {
            let id = m["slug"].as_str().or_else(|| m["id"].as_str())?.to_string();
            Some(Model {
                name: m["display_name"].as_str().unwrap_or(&id).to_string(),
                ctx: m["context_window"].as_u64().unwrap_or(0),
                id,
            })
        })
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out.dedup_by(|a, b| a.id == b.id);
    out
}

/// The ChatGPT model bia suggests: the id containing this, shortest first (`gpt-6.1-sol`
/// before a longer variant). Same rule as bia-deploy `chatgpt::default_model`.
const DEFAULT_MODEL: &str = "6.1-sol";

/// The model to preselect: the one asked for, the one in use, else the default above, else
/// plain `gpt-5`, else the first `gpt-5…`, else the first.
fn preselect(models: &[Model], wanted: &[&str]) -> String {
    let has = |id: &str| models.iter().any(|m| m.id == id);
    wanted
        .iter()
        .copied()
        .find(|w| !w.is_empty() && has(w))
        .map(str::to_string)
        .or_else(|| {
            models
                .iter()
                .filter(|m| m.id.contains(DEFAULT_MODEL))
                .min_by_key(|m| m.id.len())
                .map(|m| m.id.clone())
        })
        .or_else(|| has("gpt-5").then(|| "gpt-5".to_string()))
        .or_else(|| {
            models
                .iter()
                .find(|m| m.id.starts_with("gpt-5"))
                .map(|m| m.id.clone())
        })
        .or_else(|| models.first().map(|m| m.id.clone()))
        .unwrap_or_default()
}

// ---------------- the browser on bia ----------------

fn chrome_bin() -> Option<&'static str> {
    CHROME_BINS.iter().copied().find(|p| Path::new(p).is_file())
}

fn user_ok(user: &str) -> bool {
    Command::new("id")
        .arg(user)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// A browser bia can show on its own display.
fn browser_here() -> bool {
    chrome_bin().is_some() && user_ok("bia")
}

// ---------------- the task ----------------

struct Task {
    id: String,
    dir: PathBuf,
    v: Value,
}

impl Task {
    fn path(&self, ext: &str) -> PathBuf {
        self.dir.join(format!("{}.{ext}", self.id))
    }
    fn save(&self) -> Result<(), String> {
        let tmp = self.path("json.tmp");
        fs::write(
            &tmp,
            serde_json::to_vec_pretty(&self.v).map_err(|e| e.to_string())?,
        )
        .and_then(|_| fs::rename(&tmp, self.path("json")))
        .map_err(|e| format!("write task: {e}"))
    }
    fn set(&mut self, k: &str, v: Value) {
        self.v[k] = v;
        let _ = self.save();
    }
    /// Read and remove a signal file the web console wrote.
    fn take(&self, ext: &str) -> Option<String> {
        let p = self.path(ext);
        let s = fs::read_to_string(&p).ok()?;
        let _ = fs::remove_file(&p);
        Some(s)
    }
    fn cleanup(&self) {
        for ext in ["json", "result"] {
            let _ = fs::remove_file(self.path(ext));
        }
        unsafe { libc::sync() };
    }
}

fn page(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=utf-8><meta name=viewport content=\"width=device-width,initial-scale=1\"><title>bia · {t}</title>\
<style>body{{font:16px/1.45 system-ui,sans-serif;margin:0;background:#0c0e12;color:#e8edf5}}main{{max-width:34rem;margin:8vh auto;padding:24px}}\
p{{color:#8b95a7}}select,button{{font:inherit;padding:10px 12px;border-radius:8px;border:1px solid #2a3140;background:#14181f;color:#e8edf5;width:100%;margin-top:10px}}\
button{{background:#10a37f;border:0;color:#fff;font-weight:600;cursor:pointer}}</style></head><body><main>{body}</main></body></html>",
        t = html_escape(title)
    )
}

fn chooser(state: &str, email: &str, models: &[Model], selected: &str) -> String {
    let opts: String = models
        .iter()
        .map(|m| {
            let mut label = m.id.clone();
            if m.name != m.id {
                label += &format!(" — {}", m.name);
            }
            if m.ctx > 0 {
                label += &format!(" ({}k context)", m.ctx / 1000);
            }
            format!(
                "<option value=\"{}\"{}>{}</option>",
                html_escape(&m.id),
                if m.id == selected { " selected" } else { "" },
                html_escape(&label)
            )
        })
        .collect();
    page(
        "ChatGPT",
        &format!(
            "<h2>Signed in to ChatGPT</h2><p>{}</p><p>Pick the model bia should think with, then press Continue.</p><form action=/choose method=get>\
<input type=hidden name=state value=\"{}\"><label>Model<select name=model>{opts}</select></label>\
<button>Continue</button></form>",
            html_escape(email),
            html_escape(state)
        ),
    )
}

/// Chrome's first page on bia: the button that starts the sign-in.
fn start_page(error: &str) -> String {
    let err = if error.is_empty() {
        String::new()
    } else {
        format!("<p style=\"color:#ff6b6b\">{}</p>", html_escape(error))
    };
    page(
        "Login with ChatGPT",
        &format!(
            "<h2>Use your ChatGPT plan for bia</h2><p>Sign in with the ChatGPT account whose plan (Plus, Pro, Business) should pay for bia's thinking. \
Then pick the model.</p>{err}<form action=/login method=get><button>Login with ChatGPT</button></form>"
        ),
    )
}

/// One HTTP request from the browser, read on its own thread.
struct Req {
    path: String,
    query: String,
    sock: TcpStream,
}

/// The loopback web server. Browsers open connections before they have a request for them
/// (Chrome preconnects to an address it has seen, such as this one on a second sign-in) and
/// send the request much later, after the person has signed in. Each connection therefore
/// gets its own reader that waits, and one that never sends a request is closed without a
/// reply: an early 404 written to it would be read as the answer to the callback.
struct Server {
    rx: Receiver<Req>,
    stop: Arc<AtomicBool>,
    addr: std::net::SocketAddr,
}

impl Server {
    fn start(listener: TcpListener) -> Result<Server, String> {
        let addr = listener.local_addr().map_err(|e| e.to_string())?;
        let (tx, rx) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let halt = stop.clone();
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                if halt.load(Ordering::Relaxed) {
                    return; // the listener drops here and frees the port
                }
                let Ok(sock) = conn else { continue };
                let tx = tx.clone();
                std::thread::spawn(move || {
                    if let Some(r) = read_request(sock) {
                        let _ = tx.send(r);
                    }
                });
            }
        });
        Ok(Server { rx, stop, addr })
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // wake the blocked accept so the thread sees `stop`
        let _ = TcpStream::connect_timeout(&self.addr, Duration::from_secs(1));
    }
}

fn read_request(sock: TcpStream) -> Option<Req> {
    let _ = sock.set_nonblocking(false);
    let _ = sock.set_read_timeout(Some(Duration::from_secs(600)));
    let mut r = BufReader::new(sock.try_clone().ok()?);
    let mut line = String::new();
    r.read_line(&mut line).ok()?;
    let target = line.split_whitespace().nth(1)?.to_string();
    // Read the headers too: closing with unread bytes resets the connection, and the
    // browser can lose the reply.
    loop {
        let mut h = String::new();
        match r.read_line(&mut h) {
            Ok(0) | Err(_) => break,
            Ok(_) if h.trim().is_empty() => break,
            Ok(_) => {}
        }
    }
    let (path, query) = target.split_once('?').unwrap_or((target.as_str(), ""));
    Some(Req {
        path: path.to_string(),
        query: query.to_string(),
        sock,
    })
}

fn respond(sock: &mut TcpStream, status: &str, html: &str) {
    let _ = sock.write_all(
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{html}",
            html.len()
        )
        .as_bytes(),
    );
}

/// `fmm_chatgpt`: run the sign-in as a human task and switch FMM to the chosen model.
pub fn login(ctx: &Ctx, args: Value) -> Result<String, String> {
    let wanted_model = bia_mcp::arg_str_opt(&args, "model")
        .unwrap_or("")
        .trim()
        .to_string();
    let login_hint = bia_mcp::arg_str_opt(&args, "email")
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let timeout = bia_mcp::arg_u32_opt(&args, "timeout_secs").unwrap_or(1800);
    if !browser_here() {
        return Err("Google Chrome is not installed on bia, and the ChatGPT sign-in runs only in bia's own browser. Set it up first: discover(path='mcp/chrome', enable=true), then chrome_install and do what it says to get the official Google Chrome. Then call system_fmm_chatgpt again.".into());
    }

    // The listener is up before any browser can be sent to it. 1455 is the documented port;
    // any loopback port works as long as the redirect URI matches exactly.
    let listener = TcpListener::bind("127.0.0.1:1455")
        .or_else(|_| TcpListener::bind("127.0.0.1:0"))
        .map_err(|e| format!("open a local port for the sign-in callback: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect = format!("http://127.0.0.1:{port}{CALLBACK_PATH}");
    let server = Server::start(listener)?;

    let prev = fmm::load();
    let mut local = load_local(prev.as_ref());
    let current_model = prev
        .as_ref()
        .filter(|c| c["provider"] == "chatgpt")
        .and_then(|c| c["model"].as_str())
        .unwrap_or("")
        .to_string();
    let known = |local: &Local| -> Option<Account> {
        login_hint
            .and_then(|e| local.accounts.iter().find(|a| a.email == e).cloned())
            .or_else(|| {
                (!local.pending_client_id.is_empty()).then(|| Account {
                    client_id: local.pending_client_id.clone(),
                    ..Default::default()
                })
            })
    };
    let mut att = attempt(&local, known(&local).as_ref(), &redirect, login_hint);

    let id = hitl::new_id();
    let dir = hitl::hitl_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    let mut task = Task {
        id: id.clone(),
        dir,
        v: json!({
            "id": id,
            "kind": "chatgpt",
            "title": "Login with ChatGPT",
            "message": "The sign-in is in Chrome on bia's screen (below). Press Login with ChatGPT there, sign in, then pick the model bia should use (there or here). Your ChatGPT plan pays for it.",
            "status": "pending",
            "stage": "login",
            "auth_url": att.url,
            "error": "",
            "created": now(),
        }),
    };
    task.save()?;
    unsafe { libc::sync() };
    ctx.log(format!("human task {id}: Login with ChatGPT"));
    for u in hitl::task_links(&id) {
        ctx.log(format!(
            "web console task page {u} — bia's screen and the model list"
        ));
    }
    let start_url = format!("http://127.0.0.1:{port}/");
    let links = hitl::task_links(&id).join(" ");
    ctx.log(format!(
        "Login with ChatGPT page: {start_url} (open it in Chrome on this screen)"
    ));
    // The job's result comes only when it is over; this tells the model now, so it opens the
    // page and sends the person to Chrome while the tool waits.
    ctx.inbound(
        "system",
        "fmm",
        "",
        &format!("Login with ChatGPT is waiting at {start_url}. Now: 1) open it in Chrome on bia's screen with mcp/chrome: chrome_new_page(url='{start_url}') (chrome_start first if Chrome is not running); 2) tell root with say to switch to Chrome, press Login with ChatGPT, sign in, then pick the model. From another computer the same screen is on the web console task page: {links}. Do not press the button or sign in yourself. The result of system_fmm_chatgpt arrives as its job when they are done."),
    );

    let start = Instant::now();
    let mut signed: Option<Signed> = None;
    let mut models: Vec<Model> = vec![];
    let finish = loop {
        if let Err(e) = ctx.check_cancel() {
            task.cleanup();
            return Err(e);
        }
        if timeout > 0 && start.elapsed() >= Duration::from_secs(timeout as u64) {
            task.cleanup();
            return Err(format!("ChatGPT sign-in was not finished in {timeout}s"));
        }
        if let Some(raw) = task.take("result") {
            let v: Value = serde_json::from_str(&raw).unwrap_or(json!({}));
            if v["status"] != "done" {
                task.cleanup();
                return Err("the person cancelled the ChatGPT sign-in".into());
            }
            if signed.is_some() {
                break v["model"].as_str().unwrap_or("").to_string();
            }
            task.set("error", json!("sign in first"));
        }
        // The redirect from bia's browser.
        let mut query: Option<String> = None;
        let mut sock: Option<TcpStream> = None;
        if let Ok(Req {
            path,
            query: q,
            sock: mut s,
        }) = server.rx.recv_timeout(Duration::from_millis(200))
        {
            let q = q.as_str();
            match path.as_str() {
                CALLBACK_PATH => {
                    query = Some(q.to_string());
                    sock = Some(s);
                }
                "/choose" => {
                    let q = parse_query(q);
                    let ok = signed.is_some() && get(&q, "state") == Some(att.state.as_str());
                    let m = get(&q, "model").unwrap_or("").to_string();
                    if ok && models.iter().any(|x| x.id == m) {
                        respond(&mut s, "200 OK", &page("ChatGPT", &format!(
                            "<h2>Done</h2><p>bia is switching to {} on your ChatGPT plan. You can close this tab.</p>", html_escape(&m))));
                        break m;
                    }
                    respond(
                        &mut s,
                        "400 Bad Request",
                        &page("ChatGPT", "<h2>Pick a model from the list.</h2>"),
                    );
                }
                "/" if signed.is_some() => {
                    let html = chooser(
                        &att.state,
                        &signed.as_ref().unwrap().email,
                        &models,
                        task.v["model"].as_str().unwrap_or(""),
                    );
                    respond(&mut s, "200 OK", &html);
                }
                "/" => respond(
                    &mut s,
                    "200 OK",
                    &start_page(task.v["error"].as_str().unwrap_or("")),
                ),
                "/login" => {
                    let _ = s.write_all(
                        format!("HTTP/1.1 302 Found\r\nLocation: {}\r\nCache-Control: no-store\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", att.url)
                            .as_bytes(),
                    );
                }
                _ => respond(&mut s, "404 Not Found", ""),
            }
        }
        let Some(q) = query else {
            continue;
        };
        if signed.is_some() {
            if let Some(mut s) = sock {
                let html = chooser(
                    &att.state,
                    &signed.as_ref().unwrap().email,
                    &models,
                    task.v["model"].as_str().unwrap_or(""),
                );
                respond(&mut s, "200 OK", &html);
            }
            continue;
        }
        task.set("stage", json!("signing"));
        let q = parse_query(&q);
        let r = redeem(&mut local, &att, &redirect, &q).and_then(|s| {
            let m = list_models(&s.access)?;
            if m.is_empty() {
                return Err(format!(
                    "{} is signed in, but ChatGPT lists no models for it",
                    s.email
                ));
            }
            Ok((s, m))
        });
        match r {
            Ok((s, m)) => {
                let pick = preselect(&m, &[&wanted_model, &current_model]);
                ctx.log(format!(
                    "ChatGPT: signed in as {}; {} models",
                    s.email,
                    m.len()
                ));
                if let Some(mut sock) = sock {
                    respond(
                        &mut sock,
                        "200 OK",
                        &chooser(&att.state, &s.email, &m, &pick),
                    );
                }
                task.v["stage"] = json!("model");
                task.v["email"] = json!(s.email);
                task.v["models"] = json!(m);
                task.v["model"] = json!(pick);
                task.v["error"] = json!("");
                let _ = task.save();
                models = m;
                signed = Some(s);
            }
            Err(e) => {
                ctx.log(format!("ChatGPT sign-in: {e}"));
                if let Some(mut sock) = sock {
                    respond(&mut sock, "200 OK", &page("ChatGPT", &format!(
                        "<h2>Sign-in was not completed</h2><p>{}</p><p><a href=/ style=\"color:#3d8bfd\">Try again</a></p>",
                        html_escape(&e))));
                }
                // A code is single-use: the next try needs a new request (with the client
                // a first registration may have issued).
                att = attempt(&local, known(&local).as_ref(), &redirect, login_hint);
                task.v["stage"] = json!("login");
                task.v["auth_url"] = json!(att.url);
                task.v["error"] = json!(e);
                let _ = task.save();
            }
        }
    };
    let signed = signed.expect("signed in before a model is chosen");
    let model = if models.iter().any(|m| m.id == finish) {
        finish
    } else {
        task.v["model"].as_str().unwrap_or("").to_string()
    };
    let model_ctx = models.iter().find(|m| m.id == model).map_or(0, |m| m.ctx);
    let ctx_val = if model_ctx > 0 {
        model_ctx.clamp(512, 2_000_000)
    } else {
        prev.as_ref()
            .and_then(|c| c["ctx"].as_u64())
            .filter(|n| *n > 0)
            .unwrap_or(131072)
    };
    let cfg = json!({
        "provider": "chatgpt",
        "api_key": signed.access,
        "model": model,
        "base_url": "",
        "enabled": true,
        "ctx": ctx_val,
        "chatgpt": signed.auth,
    });
    let saved = fmm::save(&cfg).map_err(|e| format!("save fmm.json: {e:#}"));
    task.cleanup();
    saved?;
    Ok(format!(
        "FMM configured: ChatGPT plan of {}, model {model}, context {ctx_val} tokens. You now test it and switch to it if it answers; conversation history continues (streamed). Previous settings are restored if the switch fails. bia keeps the sign-in alive and it is used again after every boot.",
        signed.email
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirect_query() {
        let q = parse_query("code=a%2Fb&state=s+t");
        assert_eq!(get(&q, "code"), Some("a/b"));
        assert_eq!(get(&q, "state"), Some("s t"));
    }

    #[test]
    fn models_and_default() {
        let v = json!({"data": [
            {"slug": "gpt-5-mini", "display_name": "GPT-5 mini", "context_window": 400000},
            {"slug": "gpt-5", "context_window": 400000},
            {"slug": "hidden", "visibility": "hide"},
            {"id": "gpt-4.1"}
        ]});
        let m = parse_models(&v);
        assert_eq!(
            m.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["gpt-4.1", "gpt-5", "gpt-5-mini"]
        );
        assert_eq!(preselect(&m, &["", ""]), "gpt-5");
        let mut sol = m.clone();
        for id in ["gpt-6.1-sol-mini", "gpt-6.1-sol"] {
            sol.push(Model {
                id: id.into(),
                name: id.into(),
                ctx: 0,
            });
        }
        assert_eq!(preselect(&sol, &["", ""]), "gpt-6.1-sol");
        assert_eq!(preselect(&sol, &["gpt-5-mini", ""]), "gpt-5-mini");
        assert_eq!(preselect(&m, &["gpt-5-mini", "gpt-5"]), "gpt-5-mini");
        assert_eq!(preselect(&m, &["nope", "gpt-4.1"]), "gpt-4.1");
    }

    #[test]
    fn pages() {
        assert!(start_page("")
            .contains("<form action=/login method=get><button>Login with ChatGPT</button>"));
        assert!(start_page("declined <x>").contains("declined &lt;x&gt;"));
        let m = vec![
            Model {
                id: "gpt-5".into(),
                name: "GPT-5".into(),
                ctx: 400000,
            },
            Model {
                id: "gpt-5-mini".into(),
                name: "gpt-5-mini".into(),
                ctx: 0,
            },
        ];
        let html = chooser("st", "a@b", &m, "gpt-5-mini");
        assert!(html.contains("<option value=\"gpt-5\">gpt-5 — GPT-5 (400k context)</option>"));
        assert!(html.contains("<option value=\"gpt-5-mini\" selected>gpt-5-mini</option>"));
        assert!(
            html.contains("name=state value=\"st\"") && html.contains("<button>Continue</button>")
        );
    }

    #[test]
    fn authorize_url() {
        let l = Local {
            host_id: "urn:uuid:x".into(),
            ..Default::default()
        };
        let a = attempt(&l, None, "http://127.0.0.1:1455/auth/callback", None);
        assert!(a.url.contains("client_id=dynamic_agent_client"));
        assert!(a
            .url
            .contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A1455%2Fauth%2Fcallback"));
        let acc = Account {
            email: "a@b".into(),
            client_id: "c1".into(),
            id_token: "t".into(),
        };
        let a = attempt(&l, Some(&acc), "http://x", None);
        assert!(
            a.url.contains("client_id=c1")
                && a.url.contains("login_hint=a%40b")
                && a.url.contains("id_token_hint=t")
        );
        assert_eq!(a.client_id.as_deref(), Some("c1"));
    }
}
