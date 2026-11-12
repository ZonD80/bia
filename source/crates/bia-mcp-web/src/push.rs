//! Web Push (RFC 8030 delivery, RFC 8291 aes128gcm payloads, RFC 8292 VAPID): a browser with
//! notifications on subscribes through its push service (FCM, Mozilla, Apple); bia then
//! wakes its service worker (`/sw.js`) for new bia messages, calls and human tasks (for identities with trust root),
//! even with the console closed. Nothing is pushed while a tab of that identity is in front
//! (the page reports `{"t":"active"}` over its WebSocket) — that tab notifies by itself.
//! Payloads are end-to-end encrypted: the push service only sees ciphertext.
use crate::bus::Bus;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use ring::rand::{SecureRandom, SystemRandom};
use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};
use ring::{aead, agreement, hkdf};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

/// VAPID `sub`: Apple refuses tokens without a contact.
const CONTACT: &str = "https://ownbia.com";
const MAX_SUBS_PER_IDENTITY: usize = 10;
const MAX_PAYLOAD: usize = 3000;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Sub {
    identity: String,
    endpoint: String,
    p256dh: String,
    auth: String,
    created: u64,
    /// "Chrome on Android", from the subscribing request's User-Agent
    #[serde(default)]
    device: String,
    /// the browser's `expirationTime` (ms; 0 = none)
    #[serde(default)]
    expires: u64,
    /// last delivery the push service accepted (ms)
    #[serde(default)]
    last_ok: u64,
    /// last refusal or network error, and when (ms)
    #[serde(default)]
    last_err: String,
    #[serde(default)]
    last_err_at: u64,
}

impl Sub {
    /// Short stable id for the settings page (the endpoint itself stays on the server).
    fn id(&self) -> String {
        let d = ring::digest::digest(&ring::digest::SHA256, self.endpoint.as_bytes());
        d.as_ref()[..6].iter().map(|b| format!("{b:02x}")).collect()
    }

    fn expired(&self, now: u64) -> bool {
        self.expires > 0 && self.expires <= now
    }
}

struct Push {
    dir: PathBuf,
    /// owner of the files folder (bia): this MCP runs as root, but a backup made as user bia
    /// must be able to read the key and the subscriptions
    owner: Option<(u32, u32)>,
    key: EcdsaKeyPair,
    subs: Mutex<Vec<Sub>>,
    con: Arc<Bus>,
}

static PUSH: OnceLock<Push> = OnceLock::new();

fn dir(files: &Path) -> PathBuf {
    files.join("state").join("web").join("push")
}

/// Load (or make) the VAPID key and the subscriptions; watch human tasks for everyone with trust root.
pub fn init(files: &Path, con: Arc<Bus>) -> Result<(), String> {
    let dir = dir(files);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let owner = {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(files).ok().map(|m| (m.uid(), m.gid()))
    };
    let rng = SystemRandom::new();
    let kp = dir.join("vapid.pk8");
    let pk8 = match std::fs::read(&kp) {
        Ok(b) => b,
        Err(_) => {
            let doc = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng)
                .map_err(|_| "vapid key generation failed")?;
            write_private(&kp, doc.as_ref())?;
            doc.as_ref().to_vec()
        }
    };
    let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &pk8, &rng)
        .map_err(|e| format!("{}: {e}", kp.display()))?;
    let subs: Vec<Sub> = bia_mcp::statefile::read_or_default(&dir.join("subs.json"));
    // also fixes what 1.2.0 left owned by root (and subs.json readable by everyone)
    for p in [
        dir.parent().unwrap_or(&dir).to_path_buf(),
        dir.clone(),
        kp.clone(),
        dir.join("subs.json"),
    ] {
        own(&p, owner);
    }
    let _ = set_private(&dir.join("subs.json"));
    let _ = PUSH.set(Push {
        dir,
        owner,
        key,
        subs: Mutex::new(subs),
        con,
    });
    let files = files.to_path_buf();
    let _ = std::thread::Builder::new()
        .name("web-push-hitl".into())
        .spawn(move || watch_tasks(&files));
    Ok(())
}

fn write_private(p: &Path, b: &[u8]) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(p)
        .map_err(|e| format!("{}: {e}", p.display()))?;
    f.write_all(b).map_err(|e| format!("{}: {e}", p.display()))
}

/// Hand `p` to the owner of the files folder (no-op when that is us or unknown).
fn own(p: &Path, owner: Option<(u32, u32)>) {
    if let Some((uid, gid)) = owner {
        if p.exists() {
            let _ = std::os::unix::fs::chown(p, Some(uid), Some(gid));
        }
    }
}

fn set_private(p: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o600))
}

fn push() -> Result<&'static Push, String> {
    PUSH.get()
        .ok_or_else(|| "web push is not started".to_string())
}

/// The VAPID public key (base64url, uncompressed P-256): the page's `applicationServerKey`.
pub fn public_key() -> Result<String, String> {
    Ok(B64.encode(push()?.key.public_key().as_ref()))
}

impl Push {
    fn save(&self, subs: &[Sub]) {
        if let Ok(b) = serde_json::to_vec_pretty(subs) {
            let p = self.dir.join("subs.json");
            let tmp = p.with_extension("tmp");
            if std::fs::write(&tmp, b).is_ok() {
                // the subscriptions' auth secrets: private, like the key
                let _ = set_private(&tmp);
                own(&tmp, self.owner);
                if std::fs::rename(&tmp, &p).is_ok() {
                    let _ = bia_mcp::statefile::backup(&p); // private and owned like subs.json
                }
            }
        }
    }

    fn drop_endpoint(&self, endpoint: &str) {
        let mut subs = self.subs.lock().unwrap_or_else(|e| e.into_inner());
        subs.retain(|s| s.endpoint != endpoint);
        self.save(&subs);
    }

    /// Remember how the last delivery to `endpoint` went (shown in Settings).
    fn record(&self, endpoint: &str, res: &Result<(), String>) {
        let mut subs = self.subs.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = subs.iter_mut().find(|s| s.endpoint == endpoint) else {
            return;
        };
        match res {
            Ok(()) => s.last_ok = crate::chat::now_ms(),
            Err(e) => {
                s.last_err = e.clone();
                s.last_err_at = crate::chat::now_ms();
            }
        }
        self.save(&subs);
    }

    /// The identity's subscriptions to push to now; expired ones are forgotten.
    fn targets(&self, identity: &str) -> Vec<Sub> {
        let now = crate::chat::now_ms();
        let mut subs = self.subs.lock().unwrap_or_else(|e| e.into_inner());
        let n = subs.len();
        subs.retain(|s| !s.expired(now));
        if subs.len() != n {
            self.save(&subs);
        }
        subs.iter()
            .filter(|s| s.identity == identity)
            .cloned()
            .collect()
    }
}

/// "Chrome on macOS" from a User-Agent header (Edge, Opera and Chrome on iOS say so first).
pub(crate) fn device_of(ua: &str) -> String {
    let has = |k: &str| ua.contains(k);
    let browser = if has("Edg") {
        "Edge"
    } else if has("OPR/") || has("Opera") {
        "Opera"
    } else if has("Firefox/") || has("FxiOS") {
        "Firefox"
    } else if has("SamsungBrowser") {
        "Samsung Internet"
    } else if has("Chrome/") || has("CriOS") || has("Chromium") {
        "Chrome"
    } else if has("Safari/") {
        "Safari"
    } else {
        "Browser"
    };
    let os = if has("iPhone") {
        "iPhone"
    } else if has("iPad") {
        "iPad"
    } else if has("Android") {
        "Android"
    } else if has("CrOS") {
        "ChromeOS"
    } else if has("Mac OS X") || has("Macintosh") {
        "macOS"
    } else if has("Windows") {
        "Windows"
    } else if has("Linux") {
        "Linux"
    } else {
        ""
    };
    if os.is_empty() {
        browser.into()
    } else {
        format!("{browser} on {os}")
    }
}

fn service_of(endpoint: &str) -> String {
    let host = host_of(endpoint);
    if host.ends_with("googleapis.com") {
        "Google (FCM)".into()
    } else if host.ends_with("push.apple.com") {
        "Apple".into()
    } else if host.ends_with("mozilla.com") {
        "Mozilla".into()
    } else if host.ends_with("notify.windows.com") {
        "Microsoft (WNS)".into()
    } else {
        host.to_string()
    }
}

/// Push services are public HTTPS hosts; refuse anything else so a subscription can't make
/// bia POST into its own network.
fn endpoint_ok(endpoint: &str) -> bool {
    let Some(rest) = endpoint.strip_prefix("https://") else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit_once('@').map(|(_, h)| h).unwrap_or(host);
    let name = host.split(':').next().unwrap_or("").to_ascii_lowercase();
    name.contains('.')
        && name.parse::<std::net::IpAddr>().is_err()
        && !name.starts_with('[')
        && !name.ends_with(".local")
        && !name.ends_with(".lan")
        && !name.ends_with(".internal")
        && name != "localhost"
        && endpoint.len() <= 2048
}

/// What the person decided about notifications in this browser (`POST /v1/push/permission`):
/// granted (push works, or only while the tab is open), denied (the browser's prompt was
/// rejected), dismissed (the web app's "Not now"), off (turned off in Settings).
/// → the system message for the model, so it can acknowledge it. None: unknown state.
pub fn permission_note(identity: &str, state: &str, push: bool, error: &str, user_agent: &str) -> Option<String> {
    let device = device_of(user_agent);
    let who = if identity.is_empty() { "the web user".to_string() } else { identity.to_string() };
    let body = match state {
        "granted" if push => format!(
            "{who} turned on notifications for the web app on {device}: your messages now reach them as push notifications, also when no bia tab is open."
        ),
        "granted" => format!(
            "{who} allowed notifications on {device}, but push could not be set up{}: they get notifications only while a bia tab is open.",
            if error.trim().is_empty() { String::new() } else { format!(" ({})", error.trim()) }
        ),
        "denied" => format!(
            "{who} blocked notifications for the web app in the browser on {device} (they rejected the browser's permission prompt). Your messages won't pop up there; they can allow them later in the browser's site settings."
        ),
        "dismissed" => format!(
            "{who} chose \"Not now\" when the web app offered notifications on {device}. They can turn them on any time in Settings → Notifications."
        ),
        "off" => format!("{who} turned off notifications for the web app on {device} (Settings → Notifications)."),
        "stopped" => format!(
            "{who} stopped notifications in this browser ({device}) from Settings: your messages no longer pop up there."
        ),
        "removed" => format!(
            "{who} removed another of their browsers from push notifications (Settings, from {device}): your messages no longer pop up on that one."
        ),
        _ => return None,
    };
    Some(format!(
        "[notifications] {body} Acknowledge it in one short message on the web chat; don't ask them to change it."
    ))
}

/// `POST /v1/push/subscribe` with the browser's `PushSubscription.toJSON()` → its id.
pub fn subscribe(identity: &str, v: &Value, user_agent: &str) -> Result<String, String> {
    let p = push()?;
    let endpoint = v["endpoint"].as_str().unwrap_or("").trim().to_string();
    let p256dh = v["keys"]["p256dh"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();
    let auth = v["keys"]["auth"].as_str().unwrap_or("").trim().to_string();
    if !endpoint_ok(&endpoint) {
        return Err("endpoint must be a push service https URL".into());
    }
    let pk = B64
        .decode(&p256dh)
        .map_err(|_| "keys.p256dh: not base64url")?;
    let au = B64.decode(&auth).map_err(|_| "keys.auth: not base64url")?;
    if pk.len() != 65 || pk[0] != 4 || au.len() != 16 {
        return Err("keys: need a P-256 p256dh (65 bytes) and a 16-byte auth".into());
    }
    let mut subs = p.subs.lock().unwrap_or_else(|e| e.into_inner());
    // the page re-sends its subscription on every start: keep the history of a known one
    let old = subs
        .iter()
        .position(|s| s.endpoint == endpoint)
        .map(|i| subs.remove(i));
    let sub = Sub {
        identity: identity.to_string(),
        endpoint,
        p256dh,
        auth,
        created: old
            .as_ref()
            .map(|o| o.created)
            .unwrap_or_else(crate::chat::now_ms),
        device: device_of(user_agent),
        expires: v["expirationTime"]
            .as_f64()
            .filter(|x| *x > 0.0)
            .map(|x| x as u64)
            .unwrap_or(0),
        last_ok: old.as_ref().map(|o| o.last_ok).unwrap_or(0),
        last_err: old.as_ref().map(|o| o.last_err.clone()).unwrap_or_default(),
        last_err_at: old.as_ref().map(|o| o.last_err_at).unwrap_or(0),
    };
    let id = sub.id();
    subs.push(sub);
    let mine: Vec<usize> = (0..subs.len())
        .filter(|&i| subs[i].identity == identity)
        .collect();
    if mine.len() > MAX_SUBS_PER_IDENTITY {
        let gone: HashSet<usize> = mine[..mine.len() - MAX_SUBS_PER_IDENTITY]
            .iter()
            .copied()
            .collect();
        let mut i = 0;
        subs.retain(|_| {
            i += 1;
            !gone.contains(&(i - 1))
        });
    }
    p.save(&subs);
    Ok(id)
}

/// `GET /v1/push/subs`: the caller's subscribed browsers (newest first), for Settings.
pub fn list(identity: &str) -> Result<Value, String> {
    let p = push()?;
    let now = crate::chat::now_ms();
    let subs = p.subs.lock().unwrap_or_else(|e| e.into_inner());
    let mut out: Vec<&Sub> = subs.iter().filter(|s| s.identity == identity).collect();
    out.sort_by(|a, b| b.created.cmp(&a.created));
    let rows: Vec<Value> = out
        .iter()
        .map(|s| {
            json!({
                "id": s.id(),
                "device": if s.device.is_empty() { "Browser" } else { s.device.as_str() },
                "service": service_of(&s.endpoint),
                "created": s.created,
                "expires": s.expires,
                "expired": s.expired(now),
                "last_ok": s.last_ok,
                "last_err": s.last_err,
                "last_err_at": s.last_err_at,
                // the last attempt failed
                "failing": s.last_err_at > s.last_ok,
            })
        })
        .collect();
    Ok(json!({"subs": rows}))
}

/// `POST /v1/push/remove {ids}`: forget some of the caller's browsers → how many went.
pub fn remove(identity: &str, ids: &[String]) -> Result<usize, String> {
    let p = push()?;
    let mut subs = p.subs.lock().unwrap_or_else(|e| e.into_inner());
    let n = subs.len();
    subs.retain(|s| !(s.identity == identity && ids.contains(&s.id())));
    let gone = n - subs.len();
    if gone > 0 {
        p.save(&subs);
    }
    Ok(gone)
}

/// `POST /v1/push/unsubscribe {endpoint}` (only the caller's own).
pub fn unsubscribe(identity: &str, endpoint: &str) -> Result<bool, String> {
    let p = push()?;
    let mut subs = p.subs.lock().unwrap_or_else(|e| e.into_inner());
    let n = subs.len();
    subs.retain(|s| !(s.identity == identity && s.endpoint == endpoint));
    let gone = subs.len() != n;
    if gone {
        p.save(&subs);
    }
    Ok(gone)
}

/// A notification for the service worker: `title`, `body`, `tag` (a newer one with the same
/// tag replaces it), `url` (the console route to open on click).
#[derive(Clone)]
pub struct Note {
    pub title: String,
    pub body: String,
    pub tag: String,
    pub url: String,
    /// seconds the push service keeps it while the device is offline
    pub ttl: u32,
    /// calls: high urgency, stays until dismissed
    pub urgent: bool,
}

impl Note {
    fn payload(&self) -> Vec<u8> {
        let mut body = self.body.clone();
        loop {
            let v = json!({
                "title": self.title, "body": body, "tag": self.tag, "url": self.url,
                "urgent": self.urgent, "ts": crate::chat::now_ms(),
            });
            let b = v.to_string().into_bytes();
            if b.len() <= MAX_PAYLOAD || body.is_empty() {
                return b;
            }
            let n = body.chars().count();
            body = body.chars().take(n / 2).collect::<String>() + "…";
        }
    }
}

/// What this bia is called in a notification: the name root gave it, else "bia". Read each
/// time, since the name can change while this runs.
pub fn who() -> String {
    PUSH.get().and_then(|p| name_in(p.dir.parent()?.parent()?)).unwrap_or_else(|| "bia".into())
}

/// `<state>/self.json`'s name, under the rules bia_pki::Pki::chosen_name keeps.
fn name_in(state: &Path) -> Option<String> {
    let v: Value = serde_json::from_slice(&std::fs::read(state.join("self.json")).ok()?).ok()?;
    let n = v["name"].as_str()?.trim().to_string();
    (!n.is_empty() && n.chars().count() <= 40 && !n.chars().any(char::is_control)).then_some(n)
}

/// Notify every subscribed browser of `identity` in the background, unless one of its tabs
/// is in front right now.
pub fn notify(identity: &str, note: Note) {
    let Ok(p) = push() else { return };
    if crate::link::active(identity) {
        return;
    }
    let targets = p.targets(identity);
    if targets.is_empty() {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("web-push".into())
        .spawn(move || {
            for s in &targets {
                send_logged(p, s, &note);
            }
        });
}

/// `POST /v1/push/test`: a notification to the caller's browsers now (also when in front).
pub fn test(identity: &str) -> Result<usize, String> {
    let p = push()?;
    let targets = p.targets(identity);
    if targets.is_empty() {
        return Err("no browser of yours is subscribed".into());
    }
    let note = Note {
        title: who(),
        body: "Notifications work.".into(),
        tag: "bia-test".into(),
        url: String::new(),
        ttl: 60,
        urgent: false,
    };
    let mut ok = 0;
    let mut last = String::new();
    for s in &targets {
        match send(p, s, &note) {
            Ok(()) => ok += 1,
            Err(e) => last = e,
        }
    }
    if ok == 0 {
        return Err(last);
    }
    Ok(ok)
}

fn send_logged(p: &Push, s: &Sub, note: &Note) {
    if let Err(e) = send(p, s, note) {
        p.con
            .log(&format!("  web push to {} ({}): {e}", s.identity, s.device));
    }
}

/// One push, recorded on the subscription. One the service no longer knows (404/410) is forgotten.
fn send(p: &Push, s: &Sub, note: &Note) -> Result<(), String> {
    let res = deliver(p, s, note);
    p.record(&s.endpoint, &res);
    res
}

fn deliver(p: &Push, s: &Sub, note: &Note) -> Result<(), String> {
    let ua = B64.decode(&s.p256dh).map_err(|_| "bad p256dh")?;
    let auth = B64.decode(&s.auth).map_err(|_| "bad auth")?;
    let body = encrypt(&ua, &auth, &note.payload())?;
    let auth_header = vapid(&p.key, &s.endpoint)?;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(20)))
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .root_certs(ureq::tls::RootCerts::WebPki)
                .build(),
        )
        .build()
        .into();
    let resp = agent
        .post(&s.endpoint)
        .header("TTL", note.ttl.to_string())
        .header("Urgency", if note.urgent { "high" } else { "normal" })
        .header("Content-Encoding", "aes128gcm")
        .header("Content-Type", "application/octet-stream")
        .header("Authorization", auth_header)
        .send(&body[..])
        .map_err(|e| format!("{}: {e}", host_of(&s.endpoint)))?;
    let status = resp.status().as_u16();
    match status {
        200..=299 => Ok(()),
        404 | 410 => {
            p.drop_endpoint(&s.endpoint);
            Err(format!(
                "{} forgot this browser ({status}); dropped",
                host_of(&s.endpoint)
            ))
        }
        _ => {
            let text = resp.into_body().read_to_string().unwrap_or_default();
            Err(format!(
                "{} answered {status}: {}",
                host_of(&s.endpoint),
                text.chars().take(200).collect::<String>()
            ))
        }
    }
}

fn host_of(endpoint: &str) -> &str {
    endpoint
        .strip_prefix("https://")
        .and_then(|r| r.split('/').next())
        .unwrap_or(endpoint)
}

/// RFC 8292: `vapid t=<ES256 JWT for the endpoint's origin>, k=<public key>`.
fn vapid(key: &EcdsaKeyPair, endpoint: &str) -> Result<String, String> {
    let aud = format!("https://{}", host_of(endpoint));
    let exp = crate::chat::now_ms() / 1000 + 12 * 3600;
    let head = B64.encode(br#"{"typ":"JWT","alg":"ES256"}"#);
    let claims = B64.encode(json!({"aud": aud, "exp": exp, "sub": CONTACT}).to_string());
    let signing = format!("{head}.{claims}");
    let sig = key
        .sign(&SystemRandom::new(), signing.as_bytes())
        .map_err(|_| "vapid signing failed")?;
    Ok(format!(
        "vapid t={signing}.{}, k={}",
        B64.encode(sig.as_ref()),
        B64.encode(key.public_key().as_ref())
    ))
}

struct Len(usize);
impl hkdf::KeyType for Len {
    fn len(&self) -> usize {
        self.0
    }
}

fn hkdf(salt: &[u8], ikm: &[u8], info: &[&[u8]], len: usize) -> Result<Vec<u8>, String> {
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, salt).extract(ikm);
    let okm = prk.expand(info, Len(len)).map_err(|_| "hkdf")?;
    let mut out = vec![0u8; len];
    okm.fill(&mut out).map_err(|_| "hkdf")?;
    Ok(out)
}

/// RFC 8291 + RFC 8188: one aes128gcm record for the browser's key `ua` (65 bytes) and `auth`.
fn encrypt(ua: &[u8], auth: &[u8], plain: &[u8]) -> Result<Vec<u8>, String> {
    let rng = SystemRandom::new();
    let eph = agreement::EphemeralPrivateKey::generate(&agreement::ECDH_P256, &rng)
        .map_err(|_| "ecdh key")?;
    let as_pub = eph.compute_public_key().map_err(|_| "ecdh key")?;
    let as_pub = as_pub.as_ref().to_vec();
    let peer = agreement::UnparsedPublicKey::new(&agreement::ECDH_P256, ua);
    let ecdh = agreement::agree_ephemeral(eph, &peer, |s| s.to_vec())
        .map_err(|_| "bad browser key (p256dh)")?;
    let mut salt = [0u8; 16];
    rng.fill(&mut salt).map_err(|_| "rng")?;
    seal(&ecdh, &as_pub, ua, auth, &salt, plain)
}

fn seal(
    ecdh: &[u8],
    as_pub: &[u8],
    ua: &[u8],
    auth: &[u8],
    salt: &[u8; 16],
    plain: &[u8],
) -> Result<Vec<u8>, String> {
    let ikm = hkdf(auth, ecdh, &[b"WebPush: info\0", ua, as_pub], 32)?;
    let cek = hkdf(salt, &ikm, &[b"Content-Encoding: aes128gcm\0"], 16)?;
    let nonce = hkdf(salt, &ikm, &[b"Content-Encoding: nonce\0"], 12)?;
    let key = aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::AES_128_GCM, &cek).map_err(|_| "aes key")?,
    );
    let mut rec = plain.to_vec();
    rec.push(2); // padding delimiter of the last (only) record
    let nonce = aead::Nonce::try_assume_unique_for_key(&nonce).map_err(|_| "nonce")?;
    key.seal_in_place_append_tag(nonce, aead::Aad::empty(), &mut rec)
        .map_err(|_| "aes-gcm")?;
    let rs: u32 = 4096;
    let mut out = Vec::with_capacity(21 + as_pub.len() + rec.len());
    out.extend_from_slice(salt);
    out.extend_from_slice(&rs.to_be_bytes());
    out.push(as_pub.len() as u8);
    out.extend_from_slice(as_pub);
    out.extend_from_slice(&rec);
    Ok(out)
}

// ---- human tasks (for everyone with trust root) ----

fn root_identities(files: &Path) -> Vec<String> {
    let mut ids = bia_pki::Pki::load_or_create(files)
        .map(|p| p.root_identities())
        .unwrap_or_default();
    if !ids.iter().any(|i| i == crate::bus::ID_ROOT) {
        ids.push(crate::bus::ID_ROOT.into());
    }
    ids
}

fn watch_tasks(files: &Path) {
    let ids = |files: &Path| -> Vec<(String, String)> {
        crate::hitl::list(files)["tasks"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|t| {
                        (
                            t["id"].as_str().unwrap_or("").to_string(),
                            t["title"].as_str().unwrap_or("").to_string(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut seen: HashSet<String> = ids(files).into_iter().map(|(id, _)| id).collect();
    loop {
        std::thread::sleep(Duration::from_secs(3));
        let now = ids(files);
        for (id, title) in &now {
            if seen.insert(id.clone()) {
                let note = Note {
                    title: format!("{} needs you", who()),
                    body: title.clone(),
                    tag: format!("hitl-{id}"),
                    url: format!("task/{id}"),
                    ttl: 24 * 3600,
                    urgent: true,
                };
                // every identity with trust root can do human tasks
                for who in root_identities(files) {
                    notify(&who, note.clone());
                }
            }
        }
        let live: HashSet<&String> = now.iter().map(|(id, _)| id).collect();
        seen.retain(|id| live.contains(id));
    }
}

#[cfg(test)]
mod permission_tests {
    use super::permission_note;

    const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/156.0.0.0 Safari/537.36";

    #[test]
    fn each_decision_becomes_a_note() {
        let on = permission_note("root", "granted", true, "", UA).unwrap();
        assert!(on.starts_with("[notifications] root turned on notifications") && on.contains("Acknowledge"), "{on}");
        let tab = permission_note("root", "granted", false, "no Web Push", UA).unwrap();
        assert!(tab.contains("only while a bia tab is open") && tab.contains("(no Web Push)"), "{tab}");
        assert!(permission_note("root", "denied", false, "", UA).unwrap().contains("blocked notifications"));
        assert!(permission_note("root", "dismissed", false, "", UA).unwrap().contains("Not now"));
        assert!(permission_note("root", "off", false, "", UA).unwrap().contains("turned off"));
        assert!(permission_note("root", "stopped", false, "", UA).unwrap().contains("stopped notifications in this browser"));
        assert!(permission_note("root", "removed", false, "", UA).unwrap().contains("removed another of their browsers"));
        assert!(permission_note("root", "maybe", false, "", UA).is_none());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 8291 Appendix A, with its fixed keys and salt.
    #[test]
    fn rfc8291_example() {
        let d = |s: &str| B64.decode(s).unwrap();
        let ua = d("BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4");
        let as_pub = d("BP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8");
        let auth = d("BTBZMqHH6r4Tts7J_aSIgg");
        let salt: [u8; 16] = d("DGv6ra1nlYgDCS1FRnbzlw").try_into().unwrap();
        // ECDH(as_private, ua_public) from the RFC's intermediate values
        let ecdh = d("kyrL1jIIOHEzg3sM2ZWRHDRB62YACZhhSlknJ672kSs");
        let out = seal(
            &ecdh,
            &as_pub,
            &ua,
            &auth,
            &salt,
            b"When I grow up, I want to be a watermelon",
        )
        .unwrap();
        assert_eq!(
            B64.encode(out),
            "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN"
        );
    }

    #[test]
    fn encrypt_round_trips_through_ecdh() {
        let rng = SystemRandom::new();
        let ua_priv =
            agreement::EphemeralPrivateKey::generate(&agreement::ECDH_P256, &rng).unwrap();
        let ua = ua_priv.compute_public_key().unwrap().as_ref().to_vec();
        let auth = [7u8; 16];
        let out = encrypt(&ua, &auth, b"hello").unwrap();
        let salt: [u8; 16] = out[..16].try_into().unwrap();
        assert_eq!(u32::from_be_bytes(out[16..20].try_into().unwrap()), 4096);
        assert_eq!(out[20], 65);
        let as_pub = out[21..86].to_vec();
        let peer = agreement::UnparsedPublicKey::new(&agreement::ECDH_P256, &as_pub);
        let ecdh = agreement::agree_ephemeral(ua_priv, &peer, |s| s.to_vec()).unwrap();
        let ikm = hkdf(&auth, &ecdh, &[b"WebPush: info\0", &ua, &as_pub], 32).unwrap();
        let cek = hkdf(&salt, &ikm, &[b"Content-Encoding: aes128gcm\0"], 16).unwrap();
        let nonce = hkdf(&salt, &ikm, &[b"Content-Encoding: nonce\0"], 12).unwrap();
        let key = aead::LessSafeKey::new(aead::UnboundKey::new(&aead::AES_128_GCM, &cek).unwrap());
        let mut rec = out[86..].to_vec();
        let plain = key
            .open_in_place(
                aead::Nonce::try_assume_unique_for_key(&nonce).unwrap(),
                aead::Aad::empty(),
                &mut rec,
            )
            .unwrap();
        assert_eq!(plain, b"hello\x02");
    }

    #[test]
    fn vapid_token_verifies() {
        let rng = SystemRandom::new();
        let doc = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
        let key =
            EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, doc.as_ref(), &rng).unwrap();
        let h = vapid(&key, "https://fcm.googleapis.com/fcm/send/abc").unwrap();
        let t = h
            .strip_prefix("vapid t=")
            .unwrap()
            .split(", k=")
            .next()
            .unwrap();
        let (signed, sig) = t.rsplit_once('.').unwrap();
        let claims: Value =
            serde_json::from_slice(&B64.decode(signed.split('.').nth(1).unwrap()).unwrap())
                .unwrap();
        assert_eq!(claims["aud"], "https://fcm.googleapis.com");
        ring::signature::UnparsedPublicKey::new(
            &ring::signature::ECDSA_P256_SHA256_FIXED,
            key.public_key().as_ref(),
        )
        .verify(signed.as_bytes(), &B64.decode(sig).unwrap())
        .unwrap();
    }

    #[test]
    fn endpoints_must_be_public_https() {
        assert!(endpoint_ok("https://fcm.googleapis.com/fcm/send/x"));
        assert!(endpoint_ok("https://web.push.apple.com/QK"));
        assert!(endpoint_ok(
            "https://updates.push.services.mozilla.com/wpush/v2/x"
        ));
        for bad in [
            "http://fcm.googleapis.com/x",
            "https://192.168.1.1/x",
            "https://[::1]/x",
            "https://localhost/x",
            "https://router.lan/x",
            "https://nas.local/x",
            "https://user@10.0.0.1/x",
        ] {
            assert!(!endpoint_ok(bad), "{bad}");
        }
    }

    #[test]
    fn devices_from_user_agents() {
        for (ua, want) in [
            ("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36", "Chrome on macOS"),
            ("Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Mobile Safari/537.36", "Chrome on Android"),
            ("Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1", "Safari on iPhone"),
            ("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36 Edg/141.0.0.0", "Edge on Windows"),
            ("Mozilla/5.0 (X11; Linux x86_64; rv:140.0) Gecko/20100101 Firefox/140.0", "Firefox on Linux"),
            ("", "Browser"),
        ] {
            assert_eq!(device_of(ua), want, "{ua}");
        }
    }

    #[test]
    fn notifications_carry_the_name_root_gave() {
        let d = std::env::temp_dir().join(format!("bia-push-name-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(name_in(&d), None);
        std::fs::write(d.join("self.json"), r#"{"name":" Fred ","gender":"male"}"#).unwrap();
        assert_eq!(name_in(&d).as_deref(), Some("Fred"));
        std::fs::write(d.join("self.json"), r#"{"name":""}"#).unwrap();
        assert_eq!(name_in(&d), None);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn long_bodies_are_cut_to_fit() {
        let n = Note {
            title: "bia".into(),
            body: "x".repeat(20000),
            tag: "t".into(),
            url: String::new(),
            ttl: 60,
            urgent: false,
        };
        assert!(n.payload().len() <= MAX_PAYLOAD);
    }
}
