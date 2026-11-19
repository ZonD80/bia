//! Publicly trusted server certificates: Let's Encrypt (or another ACME CA, RFC 8555) for
//! this bia's public IP addresses (the `shortlived` profile, one certificate per address)
//! and for hostnames root adds. Validation is http-01 only, answered on port 80.
//!
//! State lives in `files/state/web/tls/`: `account.json` (the ACME account key, registered
//! without an email), `public.json` (public IPs root declared for a bia behind NAT),
//! `hostnames.json`, and one folder per name with `cert.pem`, `key.pem` and `meta.json`.
//! Everything without a valid certificate here is served the bia's own certificate.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64U, Engine};
use ring::rand::{SecureRandom, SystemRandom};
use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{IpAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const LETS_ENCRYPT: &str = "https://acme-v02.api.letsencrypt.org/directory";
pub const SHORTLIVED: &str = "shortlived";
/// Retries after a failure: 5 minutes, doubling, at most this.
const MAX_BACKOFF: i64 = 6 * 3600;
const CHALLENGE_PREFIX: &str = "/.well-known/acme-challenge/";

/// The ACME directory: Let's Encrypt, or `BIA_ACME_DIRECTORY` (staging, a test CA).
pub fn directory_url() -> String {
    std::env::var("BIA_ACME_DIRECTORY")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| LETS_ENCRYPT.into())
}

/// `BIA_ACME=0` turns all of this off (tests, a bia that must not talk to the CA).
pub fn enabled() -> bool {
    std::env::var("BIA_ACME").map_or(true, |v| v.trim() != "0")
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------------------
// http-01 answers

static CHALLENGES: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

fn challenges() -> &'static Mutex<HashMap<String, String>> {
    CHALLENGES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The key authorization for a request path on port 80, while a validation waits for it.
pub fn http01(path: &str) -> Option<String> {
    let token = path.strip_prefix(CHALLENGE_PREFIX)?;
    challenges().lock().unwrap().get(token).cloned()
}

/// Whether a plain-HTTP path belongs to ACME (never redirected).
pub fn is_challenge_path(path: &str) -> bool {
    path.starts_with(CHALLENGE_PREFIX)
}

/// Port 80 for the time of one validation, when the web MCP does not hold it (its HTTPS
/// port is not 443, so there is no redirect there). Answers challenges only.
struct TempHttp {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl TempHttp {
    fn start() -> Result<TempHttp, String> {
        let l = TcpListener::bind(("0.0.0.0", 80))
            .map_err(|e| format!("port 80 is needed to prove the address to the CA and is taken ({e})"))?;
        l.set_nonblocking(true).map_err(|e| e.to_string())?;
        let stop = Arc::new(AtomicBool::new(false));
        let s = stop.clone();
        let thread = std::thread::Builder::new().name("web-acme-80".into()).spawn(move || {
            while !s.load(Ordering::SeqCst) {
                match l.accept() {
                    Ok((stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        answer_challenge(stream);
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(100)),
                }
            }
        });
        Ok(TempHttp { stop, thread: thread.ok() })
    }
}

impl Drop for TempHttp {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // the port is free only once the thread (which owns the listener) is gone
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// One plain-HTTP request on the temporary port 80: a challenge answer or 404.
fn answer_challenge(mut stream: std::net::TcpStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let Ok(clone) = stream.try_clone() else { return };
    let mut r = BufReader::new(clone);
    let mut line = String::new();
    if r.read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }
    loop {
        let mut h = String::new();
        if r.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
            break;
        }
    }
    let path = line.split_whitespace().nth(1).unwrap_or("");
    let (status, body) = match http01(path) {
        Some(k) => ("200 OK", k),
        None => ("404 Not Found", String::new()),
    };
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

// ---------------------------------------------------------------------------------------
// names and addresses

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Name {
    Ip(IpAddr),
    Host(String),
}

impl Name {
    /// Folder under `tls/` (no ':' for IPv6).
    fn dir_name(&self) -> String {
        match self {
            Name::Ip(ip) => ip.to_string().replace(':', "_"),
            Name::Host(h) => h.clone(),
        }
    }

    fn identifier(&self) -> Value {
        match self {
            Name::Ip(ip) => json!({"type": "ip", "value": ip.to_string()}),
            Name::Host(h) => json!({"type": "dns", "value": h}),
        }
    }

    pub fn shown(&self) -> String {
        match self {
            Name::Ip(ip) => ip.to_string(),
            Name::Host(h) => h.clone(),
        }
    }
}

/// An address the whole internet can reach: anything but private, shared (CGNAT),
/// loopback, link-local, documentation, benchmarking, reserved and multicast ranges.
pub fn is_public(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            let o = v.octets();
            !(v.is_private()
                || v.is_loopback()
                || v.is_link_local()
                || v.is_broadcast()
                || v.is_documentation()
                || v.is_unspecified()
                || v.is_multicast()
                || o[0] == 0
                || (o[0] == 100 && (o[1] & 0xc0) == 64)
                || (o[0] == 192 && o[1] == 0 && o[2] == 0)
                || (o[0] == 198 && (o[1] & 0xfe) == 18)
                || o[0] >= 240)
        }
        IpAddr::V6(v) => {
            if let Some(v4) = v.to_ipv4_mapped() {
                return is_public(&IpAddr::V4(v4));
            }
            let s = v.segments();
            !(v.is_loopback()
                || v.is_unspecified()
                || v.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00
                || (s[0] & 0xffc0) == 0xfe80
                || (s[0] == 0x2001 && s[1] == 0x0db8))
        }
    }
}

/// A hostname root may add: letters, digits and hyphens in dot-separated labels, at least
/// two labels, not an address. Returned lowercased.
pub fn check_hostname(h: &str) -> Result<String, String> {
    let h = h.trim().trim_end_matches('.').to_ascii_lowercase();
    let bad = || format!("{h:?} is not a hostname (like bia.example.com)");
    if h.is_empty() || h.len() > 253 || h.parse::<IpAddr>().is_ok() || !h.contains('.') {
        return Err(bad());
    }
    for label in h.split('.') {
        let ok = !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        if !ok {
            return Err(bad());
        }
    }
    if h.split('.').next_back().is_some_and(|tld| tld.chars().all(|c| c.is_ascii_digit())) {
        return Err(bad());
    }
    Ok(h)
}

pub fn tls_dir(files: &Path) -> PathBuf {
    files.join("state").join("web").join("tls")
}

fn read_list(path: &Path, key: &str) -> Vec<String> {
    bia_mcp::statefile::read::<Value>(path)
        .and_then(|v| v.get(key).cloned())
        .and_then(|v| serde_json::from_value::<Vec<String>>(v).ok())
        .unwrap_or_default()
}

fn write_list(path: &Path, key: &str, items: &[String]) -> Result<(), String> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    bia_mcp::statefile::write_json(path, &json!({ key: items }))
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// Public IPs root declared (a bia behind a router that forwards ports to it).
pub fn declared_ips(files: &Path) -> Vec<IpAddr> {
    read_list(&tls_dir(files).join("public.json"), "ips")
        .iter()
        .filter_map(|s| s.parse().ok())
        .collect()
}

pub fn set_declared_ips(files: &Path, ips: &[IpAddr]) -> Result<(), String> {
    let v: Vec<String> = ips.iter().map(|i| i.to_string()).collect();
    write_list(&tls_dir(files).join("public.json"), "ips", &v)
}

/// Hostnames root added.
pub fn hostnames(files: &Path) -> Vec<String> {
    read_list(&tls_dir(files).join("hostnames.json"), "names")
}

pub fn set_hostnames(files: &Path, names: &[String]) -> Result<(), String> {
    write_list(&tls_dir(files).join("hostnames.json"), "names", names)
}

/// Every name that should have a certificate: public addresses on the interfaces, the
/// declared ones, root's hostnames.
pub fn wanted(files: &Path, own: &[IpAddr]) -> Vec<Name> {
    let mut out: Vec<Name> = vec![];
    for ip in own.iter().chain(declared_ips(files).iter()) {
        let n = Name::Ip(*ip);
        if is_public(ip) && !out.contains(&n) {
            out.push(n);
        }
    }
    for h in hostnames(files) {
        let n = Name::Host(h);
        if !out.contains(&n) {
            out.push(n);
        }
    }
    out
}

// ---------------------------------------------------------------------------------------
// stored certificates

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Meta {
    #[serde(default)]
    pub not_before: i64,
    #[serde(default)]
    pub not_after: i64,
    /// When to renew (the CA's renewal window, else a share of the lifetime).
    #[serde(default)]
    pub renew_at: i64,
    /// When the CA's renewal window was last asked.
    #[serde(default)]
    pub ari_checked: i64,
    /// Failures in a row since the last success.
    #[serde(default)]
    pub fails: u32,
    #[serde(default)]
    pub next_try: i64,
    #[serde(default)]
    pub error: String,
    /// root was already told about this run of failures.
    #[serde(default)]
    pub told: bool,
}

fn name_dir(files: &Path, n: &Name) -> PathBuf {
    tls_dir(files).join(n.dir_name())
}

pub fn read_meta(files: &Path, n: &Name) -> Meta {
    bia_mcp::statefile::read(&name_dir(files, n).join("meta.json")).unwrap_or_default()
}

fn write_meta(files: &Path, n: &Name, m: &Meta) {
    let d = name_dir(files, n);
    let _ = std::fs::create_dir_all(&d);
    let _ = bia_mcp::statefile::write_json(&d.join("meta.json"), m);
}

fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let tmp = path.with_extension("tmp");
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)?;
    f.write_all(data)?;
    f.sync_all()?;
    std::fs::rename(&tmp, path)
}

/// A certificate for `n` that is valid right now, ready to serve.
pub fn load(files: &Path, n: &Name) -> Option<Arc<rustls::sign::CertifiedKey>> {
    let d = name_dir(files, n);
    let m = read_meta(files, n);
    let t = now();
    if m.not_after <= t || m.not_before > t + 300 {
        return None;
    }
    let cert = std::fs::read_to_string(d.join("cert.pem")).ok()?;
    let key = std::fs::read_to_string(d.join("key.pem")).ok()?;
    let certs = bia_pki::pem_to_cert_ders(&cert).ok()?;
    let key = bia_pki::pem_to_key_der(&key).ok()?;
    bia_pki::certified_key(certs, &key).ok()
}

/// Forget `n`'s certificate.
pub fn remove(files: &Path, n: &Name) {
    let _ = std::fs::remove_dir_all(name_dir(files, n));
}

/// What `web_status` shows for one name.
pub fn describe(files: &Path, n: &Name) -> String {
    let m = read_meta(files, n);
    let t = now();
    let mut s = if m.not_after > t {
        format!(
            "{}: Let's Encrypt until {} (renews from {})",
            n.shown(),
            fmt_time(m.not_after),
            fmt_time(m.renew_at)
        )
    } else {
        format!("{}: no publicly trusted certificate yet (browsers warn)", n.shown())
    };
    if m.fails > 0 {
        s.push_str(&format!(
            "; last try failed ({}), next try {}",
            m.error,
            fmt_time(m.next_try)
        ));
    }
    s
}

pub fn fmt_time(t: i64) -> String {
    let (y, mo, d, h, mi) = civil(t);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02} UTC")
}

fn civil(t: i64) -> (i64, i64, i64, i64, i64) {
    let days = t.div_euclid(86400);
    let secs = t.rem_euclid(86400);
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    (y, m, d, secs / 3600, secs % 3600 / 60)
}

/// RFC 3339 (`2026-11-19T10:00:00Z`, fractions and offsets allowed) to unix seconds.
pub fn parse_rfc3339(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.len() < 20 {
        return None;
    }
    let num = |a: usize, b: usize| s.get(a..b)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (h, mi, se) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    let mut rest = &s[19..];
    if let Some(r) = rest.strip_prefix('.') {
        rest = r.trim_start_matches(|c: char| c.is_ascii_digit());
    }
    let offset = if rest.eq_ignore_ascii_case("z") {
        0
    } else {
        let sign = match rest.chars().next()? {
            '+' => 1,
            '-' => -1,
            _ => return None,
        };
        let oh = rest.get(1..3)?.parse::<i64>().ok()?;
        let om = rest.get(4..6)?.parse::<i64>().ok()?;
        sign * (oh * 3600 + om * 60)
    };
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(days * 86400 + h * 3600 + mi * 60 + se - offset)
}

/// The default renewal time without the CA's advice: half the lifetime for short-lived
/// certificates (Let's Encrypt's advice for 6-day ones), two thirds otherwise.
pub fn default_renew_at(not_before: i64, not_after: i64) -> i64 {
    let life = not_after - not_before;
    if life <= 10 * 86400 {
        not_before + life / 2
    } else {
        not_before + life * 2 / 3
    }
}

/// Validity, and the ARI certificate id (`base64url(AKI keyIdentifier).base64url(serial)`).
fn cert_facts(pem: &str) -> Result<(i64, i64, Option<String>), String> {
    let ders = bia_pki::pem_to_cert_ders(pem).map_err(|e| format!("{e:#}"))?;
    let (_, c) = x509_parser::parse_x509_certificate(ders[0].as_ref())
        .map_err(|e| format!("certificate from the CA: {e}"))?;
    let nb = c.validity().not_before.timestamp();
    let na = c.validity().not_after.timestamp();
    let aki = c.extensions().iter().find_map(|e| match e.parsed_extension() {
        x509_parser::extensions::ParsedExtension::AuthorityKeyIdentifier(a) => {
            a.key_identifier.as_ref().map(|k| k.0.to_vec())
        }
        _ => None,
    });
    let id = aki.map(|a| format!("{}.{}", B64U.encode(a), B64U.encode(c.raw_serial())));
    Ok((nb, na, id))
}

// ---------------------------------------------------------------------------------------
// the ACME client

pub struct Client {
    agent: ureq::Agent,
    dir: Value,
    key: EcdsaKeyPair,
    kid: String,
    nonce: Option<String>,
    rng: SystemRandom,
}

struct Reply {
    status: u16,
    location: Option<String>,
    retry_after: Option<u64>,
    body: Vec<u8>,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }

    /// The CA's problem document in a few words.
    fn problem(&self) -> String {
        let v = self.json();
        let mut s = v["detail"].as_str().unwrap_or("").to_string();
        if let Some(subs) = v["subproblems"].as_array() {
            for p in subs {
                if let Some(d) = p["detail"].as_str() {
                    s.push_str("; ");
                    s.push_str(d);
                }
            }
        }
        if s.is_empty() {
            s = String::from_utf8_lossy(&self.body).chars().take(200).collect();
        }
        format!("{} ({})", s, self.status)
    }
}

fn agent() -> Result<ureq::Agent, String> {
    let roots = match std::env::var("BIA_ACME_CA") {
        Ok(path) if !path.trim().is_empty() => {
            let pem = std::fs::read(&path).map_err(|e| format!("BIA_ACME_CA {path}: {e}"))?;
            let cert = ureq::tls::Certificate::from_pem(&pem).map_err(|e| format!("BIA_ACME_CA: {e}"))?;
            ureq::tls::RootCerts::new_with_certs(&[cert])
        }
        _ => ureq::tls::RootCerts::WebPki,
    };
    Ok(ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(30)))
        .tls_config(ureq::tls::TlsConfig::builder().root_certs(roots).build())
        .build()
        .into())
}

/// JWK of a P-256 public key (uncompressed point).
fn jwk_of(public: &[u8]) -> Value {
    json!({
        "crv": "P-256",
        "kty": "EC",
        "x": B64U.encode(&public[1..33]),
        "y": B64U.encode(&public[33..65]),
    })
}

/// RFC 7638 thumbprint: SHA-256 of the JWK's required members in lexical order.
fn thumbprint(jwk: &Value) -> String {
    let canon = format!(
        r#"{{"crv":"P-256","kty":"EC","x":"{}","y":"{}"}}"#,
        jwk["x"].as_str().unwrap_or(""),
        jwk["y"].as_str().unwrap_or("")
    );
    B64U.encode(ring::digest::digest(&ring::digest::SHA256, canon.as_bytes()))
}

impl Client {
    /// Load or make the ACME account for the current directory.
    pub fn open(files: &Path) -> Result<Client, String> {
        let agent = agent()?;
        let url = directory_url();
        let dir: Value = {
            let mut r = agent.get(&url).call().map_err(|e| format!("ACME directory {url}: {e}"))?;
            r.body_mut().read_json().map_err(|e| format!("ACME directory {url}: {e}"))?
        };
        let rng = SystemRandom::new();
        let path = tls_dir(files).join("account.json");
        let saved: Option<Value> = bia_mcp::statefile::read(&path);
        let same_ca = saved.as_ref().is_some_and(|v| v["directory"].as_str() == Some(url.as_str()));
        let pkcs8 = match saved.as_ref().and_then(|v| v["key"].as_str()).and_then(|k| B64U.decode(k).ok()) {
            Some(k) => k,
            None => EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng)
                .map_err(|_| "make ACME account key".to_string())?
                .as_ref()
                .to_vec(),
        };
        let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &pkcs8, &rng)
            .map_err(|_| "ACME account key is damaged".to_string())?;
        let mut c = Client {
            agent,
            dir,
            key,
            kid: String::new(),
            nonce: None,
            rng,
        };
        let kid = saved
            .as_ref()
            .filter(|_| same_ca)
            .and_then(|v| v["kid"].as_str())
            .map(str::to_string);
        c.kid = match kid {
            Some(k) => k,
            None => {
                let url = c.url("newAccount")?;
                let r = c.post(&url, Some(&json!({"termsOfServiceAgreed": true})), true, None)?;
                if !(200..300).contains(&r.status) {
                    return Err(format!("ACME account: {}", r.problem()));
                }
                let kid = r.location.ok_or("ACME account: no Location")?;
                let _ = std::fs::create_dir_all(tls_dir(files));
                let body = json!({"directory": url_of_dir(), "key": B64U.encode(&pkcs8), "kid": kid});
                write_private(&path, serde_json::to_string_pretty(&body).unwrap_or_default().as_bytes())
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                kid
            }
        };
        Ok(c)
    }

    fn url(&self, what: &str) -> Result<String, String> {
        self.dir[what]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| format!("the ACME CA has no {what}"))
    }

    /// Profiles the CA offers (`meta.profiles`).
    fn has_profile(&self, p: &str) -> bool {
        self.dir["meta"]["profiles"].get(p).is_some()
    }

    fn jwk(&self) -> Value {
        jwk_of(self.key.public_key().as_ref())
    }

    fn fresh_nonce(&mut self) -> Result<String, String> {
        if let Some(n) = self.nonce.take() {
            return Ok(n);
        }
        let url = self.url("newNonce")?;
        let r = self.agent.head(&url).call().map_err(|e| format!("ACME nonce: {e}"))?;
        r.headers()
            .get("replay-nonce")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
            .ok_or_else(|| "ACME nonce: none given".into())
    }

    /// A signed POST (payload `None` = POST-as-GET). Retries once on badNonce.
    fn post(&mut self, url: &str, payload: Option<&Value>, use_jwk: bool, accept: Option<&str>) -> Result<Reply, String> {
        for attempt in 0..2 {
            let nonce = self.fresh_nonce()?;
            let mut protected = json!({"alg": "ES256", "nonce": nonce, "url": url});
            if use_jwk {
                protected["jwk"] = self.jwk();
            } else {
                protected["kid"] = json!(self.kid);
            }
            let p64 = B64U.encode(protected.to_string());
            let b64 = match payload {
                Some(v) => B64U.encode(v.to_string()),
                None => String::new(),
            };
            let sig = self
                .key
                .sign(&self.rng, format!("{p64}.{b64}").as_bytes())
                .map_err(|_| "ACME: signing failed".to_string())?;
            let body = json!({"protected": p64, "payload": b64, "signature": B64U.encode(sig.as_ref())});
            let mut req = self.agent.post(url).header("Content-Type", "application/jose+json");
            if let Some(a) = accept {
                req = req.header("Accept", a);
            }
            let mut resp = req
                .send(body.to_string())
                .map_err(|e| format!("ACME {}: {e}", short(url)))?;
            let h = |k: &str| resp.headers().get(k).and_then(|v| v.to_str().ok()).map(str::to_string);
            self.nonce = h("replay-nonce");
            let reply = Reply {
                status: resp.status().as_u16(),
                location: h("location"),
                retry_after: h("retry-after").and_then(|s| s.trim().parse().ok()),
                body: resp
                    .body_mut()
                    .with_config()
                    .limit(1024 * 1024)
                    .read_to_vec()
                    .unwrap_or_default(),
            };
            let bad_nonce = reply.status == 400
                && reply.json()["type"].as_str() == Some("urn:ietf:params:acme:error:badNonce");
            if bad_nonce && attempt == 0 {
                continue;
            }
            return Ok(reply);
        }
        unreachable!()
    }

    /// POST-as-GET `url` until its `status` leaves `pending`/`processing`/`ready` (the
    /// latter only when `past_ready`), about two minutes at most.
    fn poll(&mut self, url: &str, past_ready: bool) -> Result<Value, String> {
        for _ in 0..60 {
            let r = self.post(url, None, false, None)?;
            if !(200..300).contains(&r.status) {
                return Err(r.problem());
            }
            let v = r.json();
            let st = v["status"].as_str().unwrap_or("");
            let waiting = st == "pending" || st == "processing" || (past_ready && st == "ready");
            if !waiting {
                return Ok(v);
            }
            std::thread::sleep(Duration::from_secs(r.retry_after.unwrap_or(2).clamp(1, 10)));
        }
        Err("the CA did not finish in time".into())
    }

    /// Order, prove and download a certificate for `n`. `replaces` is the ARI id of the
    /// certificate this one renews. Returns (chain PEM, key PEM).
    pub fn issue(&mut self, n: &Name, replaces: Option<&str>) -> Result<(String, String), String> {
        let mut order = json!({"identifiers": [n.identifier()]});
        if matches!(n, Name::Ip(_)) {
            if self.has_profile(SHORTLIVED) {
                order["profile"] = json!(SHORTLIVED);
            } else if self.dir["meta"]["profiles"].is_object() {
                return Err(format!("this ACME CA has no {SHORTLIVED} profile, which IP certificates need"));
            }
        }
        if let (Some(r), true) = (replaces, self.dir["renewalInfo"].is_string()) {
            order["replaces"] = json!(r);
        }
        let new_order = self.url("newOrder")?;
        let mut r = self.post(&new_order, Some(&order), false, None)?;
        if r.status >= 400 && order.get("replaces").is_some() {
            // the old certificate's order is unknown to this account (or already replaced)
            order.as_object_mut().map(|o| o.remove("replaces"));
            r = self.post(&new_order, Some(&order), false, None)?;
        }
        if r.status != 201 && r.status != 200 {
            return Err(r.problem());
        }
        let order_url = r.location.clone().ok_or("the CA gave the order no address")?;
        let o = r.json();
        let thumb = thumbprint(&self.jwk());
        for authz in o["authorizations"].as_array().cloned().unwrap_or_default() {
            let Some(authz) = authz.as_str() else { continue };
            let a = self.post(authz, None, false, None)?;
            if !(200..300).contains(&a.status) {
                return Err(a.problem());
            }
            let a = a.json();
            if a["status"] == "valid" {
                continue;
            }
            let ch = a["challenges"]
                .as_array()
                .and_then(|cs| cs.iter().find(|c| c["type"] == "http-01"))
                .cloned()
                .ok_or("the CA offered no http-01 challenge")?;
            let token = ch["token"].as_str().ok_or("challenge without token")?.to_string();
            let ch_url = ch["url"].as_str().ok_or("challenge without url")?.to_string();
            challenges().lock().unwrap().insert(token.clone(), format!("{token}.{thumb}"));
            let res = (|| {
                let r = self.post(&ch_url, Some(&json!({})), false, None)?;
                if !(200..300).contains(&r.status) {
                    return Err(r.problem());
                }
                let done = self.poll(authz, false)?;
                if done["status"] != "valid" {
                    let why = done["challenges"]
                        .as_array()
                        .and_then(|cs| cs.iter().find_map(|c| c["error"]["detail"].as_str().map(str::to_string)))
                        .unwrap_or_else(|| format!("status {}", done["status"]));
                    return Err(format!("the CA could not reach http://{}/ on port 80: {why}", n.shown()));
                }
                Ok(())
            })();
            challenges().lock().unwrap().remove(&token);
            res?;
        }
        // the key for this certificate, and the CSR the CA signs
        let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).map_err(|e| e.to_string())?;
        let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).map_err(|e| e.to_string())?;
        params.distinguished_name = rcgen::DistinguishedName::new();
        params.subject_alt_names = vec![match n {
            Name::Ip(ip) => rcgen::SanType::IpAddress(*ip),
            Name::Host(h) => rcgen::SanType::DnsName(h.clone().try_into().map_err(|e| format!("{e}"))?),
        }];
        let csr = params.serialize_request(&key).map_err(|e| e.to_string())?;
        let fin = o["finalize"].as_str().ok_or("the order has no finalize address")?.to_string();
        let ready = self.poll(&order_url, false)?;
        if ready["status"] != "ready" && ready["status"] != "valid" {
            return Err(format!("the order is {}", ready["status"]));
        }
        let r = self.post(&fin, Some(&json!({"csr": B64U.encode(csr.der())})), false, None)?;
        if !(200..300).contains(&r.status) {
            return Err(r.problem());
        }
        let done = self.poll(&order_url, true)?;
        let cert_url = done["certificate"]
            .as_str()
            .ok_or_else(|| format!("the order ended {}", done["status"]))?
            .to_string();
        let r = self.post(&cert_url, None, false, Some("application/pem-certificate-chain"))?;
        if !(200..300).contains(&r.status) {
            return Err(r.problem());
        }
        let chain = String::from_utf8(r.body).map_err(|_| "certificate is not PEM text")?;
        Ok((chain, key.serialize_pem()))
    }

    /// The CA's suggested renewal time (ARI), a random moment in its window.
    fn renewal_time(&self, ari_id: &str) -> Option<i64> {
        let base = self.dir["renewalInfo"].as_str()?;
        let url = format!("{}/{ari_id}", base.trim_end_matches('/'));
        let mut r = self.agent.get(&url).call().ok()?;
        if r.status().as_u16() != 200 {
            return None;
        }
        let v: Value = r.body_mut().read_json().ok()?;
        let start = parse_rfc3339(v["suggestedWindow"]["start"].as_str()?)?;
        let end = parse_rfc3339(v["suggestedWindow"]["end"].as_str()?)?;
        if end <= start {
            return Some(start);
        }
        let mut b = [0u8; 4];
        self.rng.fill(&mut b).ok()?;
        Some(start + (u32::from_le_bytes(b) as i64) % (end - start))
    }
}

fn url_of_dir() -> String {
    directory_url()
}

fn short(url: &str) -> &str {
    url.split("://").nth(1).and_then(|r| r.split('/').next()).unwrap_or(url)
}

// ---------------------------------------------------------------------------------------
// renewal

/// What one pass did, for the log and for root.
#[derive(Debug, Default)]
pub struct Pass {
    pub issued: Vec<Name>,
    /// Failures root must hear about now: (name, error).
    pub tell: Vec<(Name, String)>,
}

/// Whether `n` needs an order now.
fn due(m: &Meta, t: i64) -> bool {
    if m.fails > 0 && t < m.next_try {
        return false;
    }
    m.not_after <= t || t >= m.renew_at
}

fn backoff(fails: u32) -> i64 {
    (300i64 << fails.saturating_sub(1).min(16)).min(MAX_BACKOFF)
}

/// Bring every wanted certificate up to date. `port80` says whether the web MCP's own
/// listener answers challenges on port 80; else one is opened for the pass.
pub fn pass(files: &Path, wanted: &[Name], port80: bool, only: Option<&Name>) -> Pass {
    let t = now();
    let mut out = Pass::default();
    let todo: Vec<Name> = wanted
        .iter()
        .filter(|n| only.map_or(true, |o| o == *n))
        .filter(|n| only.is_some() || due(&read_meta(files, n), t))
        .cloned()
        .collect();
    // the CA's advice changes rarely: ask every 6 hours for certificates not yet due
    if only.is_none() {
        refresh_ari(files, wanted, &todo, t);
    }
    if todo.is_empty() {
        return out;
    }
    let _http = if port80 {
        None
    } else {
        match TempHttp::start() {
            Ok(h) => Some(h),
            Err(e) => {
                fail_all(files, &todo, &e, &mut out);
                return out;
            }
        }
    };
    let mut client = match Client::open(files) {
        Ok(c) => c,
        Err(e) => {
            fail_all(files, &todo, &e, &mut out);
            return out;
        }
    };
    for n in &todo {
        let mut m = read_meta(files, n);
        let old = std::fs::read_to_string(name_dir(files, n).join("cert.pem"))
            .ok()
            .and_then(|p| cert_facts(&p).ok())
            .and_then(|f| f.2);
        match client.issue(n, old.as_deref()) {
            Ok((chain, key)) => match cert_facts(&chain) {
                Ok((nb, na, id)) => {
                    let d = name_dir(files, n);
                    let _ = std::fs::create_dir_all(&d);
                    if write_private(&d.join("key.pem"), key.as_bytes()).is_err()
                        || write_private(&d.join("cert.pem"), chain.as_bytes()).is_err()
                    {
                        note_failure(files, n, &mut m, "could not save the certificate", &mut out);
                        continue;
                    }
                    let renew = id
                        .as_deref()
                        .and_then(|i| client.renewal_time(i))
                        .filter(|r| *r > now() && *r < na)
                        .unwrap_or_else(|| default_renew_at(nb, na));
                    write_meta(
                        files,
                        n,
                        &Meta {
                            not_before: nb,
                            not_after: na,
                            renew_at: renew,
                            ari_checked: now(),
                            ..Meta::default()
                        },
                    );
                    out.issued.push(n.clone());
                }
                Err(e) => note_failure(files, n, &mut m, &e, &mut out),
            },
            Err(e) => note_failure(files, n, &mut m, &e, &mut out),
        }
    }
    out
}

fn fail_all(files: &Path, todo: &[Name], e: &str, out: &mut Pass) {
    for n in todo {
        let mut m = read_meta(files, n);
        note_failure(files, n, &mut m, e, out);
    }
}

fn note_failure(files: &Path, n: &Name, m: &mut Meta, e: &str, out: &mut Pass) {
    let t = now();
    m.fails += 1;
    m.next_try = t + backoff(m.fails);
    m.error = e.chars().take(400).collect();
    // root hears once per run of failures, when browsers warn or soon will
    if !m.told && m.not_after < t + 86400 {
        m.told = true;
        out.tell.push((n.clone(), m.error.clone()));
    }
    write_meta(files, n, m);
}

fn refresh_ari(files: &Path, wanted: &[Name], todo: &[Name], t: i64) {
    let stale: Vec<&Name> = wanted
        .iter()
        .filter(|n| !todo.contains(n))
        .filter(|n| {
            let m = read_meta(files, n);
            m.not_after > t && t - m.ari_checked > 6 * 3600
        })
        .collect();
    if stale.is_empty() {
        return;
    }
    let Ok(client) = Client::open(files) else { return };
    for n in stale {
        let mut m = read_meta(files, n);
        let id = std::fs::read_to_string(name_dir(files, n).join("cert.pem"))
            .ok()
            .and_then(|p| cert_facts(&p).ok())
            .and_then(|f| f.2);
        if let Some(r) = id.and_then(|i| client.renewal_time(&i)) {
            // the CA may ask for an earlier renewal (a revocation coming); never later
            // than half the remaining time for short-lived ones
            if r < m.renew_at {
                m.renew_at = r.max(t);
            }
        }
        m.ari_checked = t;
        write_meta(files, n, &m);
    }
}

/// Certificates of names no longer wanted: kept until they expire (an address can come
/// back after a DHCP renewal), then removed. Hostnames root removed go at once (`remove`).
pub fn prune(files: &Path, wanted: &[Name]) {
    let Ok(rd) = std::fs::read_dir(tls_dir(files)) else { return };
    let t = now();
    for e in rd.flatten() {
        if !e.path().is_dir() {
            continue;
        }
        let dn = e.file_name().to_string_lossy().to_string();
        let is_wanted = wanted.iter().any(|n| n.dir_name() == dn);
        if is_wanted {
            continue;
        }
        let m: Meta = bia_mcp::statefile::read(&e.path().join("meta.json")).unwrap_or_default();
        if m.not_after <= t {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

// ---------------------------------------------------------------------------------------
// choosing the certificate per connection

/// The server certificate for one kind of connection: root's hostnames by SNI, else the
/// address certificate, else bia's own.
#[derive(Debug)]
pub struct Pick {
    pub hosts: HashMap<String, Arc<rustls::sign::CertifiedKey>>,
    pub ip: Option<Arc<rustls::sign::CertifiedKey>>,
    pub own: Arc<rustls::sign::CertifiedKey>,
}

impl rustls::server::ResolvesServerCert for Pick {
    fn resolve(&self, hello: rustls::server::ClientHello<'_>) -> Option<Arc<rustls::sign::CertifiedKey>> {
        if let Some(sni) = hello.server_name() {
            if let Some(c) = self.hosts.get(&sni.to_ascii_lowercase()) {
                return Some(c.clone());
            }
        }
        Some(self.ip.clone().unwrap_or_else(|| self.own.clone()))
    }
}

/// Which address certificate a connection gets: the address it arrived on when that one
/// is public and has one; else, for a client from the internet reaching a private address
/// (a router's port forward), the first declared public address that has one.
pub fn ip_for(local: IpAddr, peer: IpAddr, have: &dyn Fn(&IpAddr) -> bool, declared: &[IpAddr]) -> Option<IpAddr> {
    let local = match local {
        IpAddr::V6(v) => v.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(local),
        v => v,
    };
    let peer = match peer {
        IpAddr::V6(v) => v.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(peer),
        v => v,
    };
    if is_public(&local) {
        return have(&local).then_some(local);
    }
    if is_public(&peer) {
        return declared.iter().find(|d| have(d)).copied();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_addresses_are_told_apart() {
        for s in ["54.36.211.82", "1.1.1.1", "2a01:4f8::1"] {
            assert!(is_public(&s.parse().unwrap()), "{s}");
        }
        for s in [
            "10.0.2.15", "192.168.88.18", "172.16.0.1", "100.64.1.1", "127.0.0.1", "169.254.1.1",
            "192.0.2.5", "198.18.0.1", "0.1.2.3", "240.0.0.1", "::1", "fe80::1", "fd00::1",
            "2001:db8::1", "::ffff:192.168.1.1",
        ] {
            assert!(!is_public(&s.parse().unwrap()), "{s}");
        }
    }

    #[test]
    fn hostnames_are_checked() {
        assert_eq!(check_hostname("Bia.Example.com.").unwrap(), "bia.example.com");
        for bad in ["", "localhost", "1.2.3.4", "a..b", "-a.com", "a_b.com", "a.123"] {
            assert!(check_hostname(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn rfc3339_and_back() {
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339("2026-11-19T10:00:00Z"), Some(1795082400));
        assert_eq!(parse_rfc3339("2026-11-19T10:00:00.123456Z"), Some(1795082400));
        assert_eq!(parse_rfc3339("2026-11-19T11:00:00+01:00"), Some(1795082400));
        assert_eq!(parse_rfc3339("2024-02-29T00:00:00Z"), Some(1709164800));
        assert_eq!(fmt_time(1795082400), "2026-11-19 10:00 UTC");
        assert_eq!(parse_rfc3339("nonsense"), None);
    }

    #[test]
    fn renewal_defaults_and_backoff() {
        // a 160-hour certificate renews at 80 hours, a 90-day one at 60 days
        assert_eq!(default_renew_at(0, 160 * 3600), 80 * 3600);
        assert_eq!(default_renew_at(0, 90 * 86400), 60 * 86400);
        assert_eq!(backoff(1), 300);
        assert_eq!(backoff(2), 600);
        assert_eq!(backoff(30), MAX_BACKOFF);
        let t = 1000;
        assert!(due(&Meta::default(), t));
        assert!(!due(&Meta { not_after: 5000, renew_at: 2000, ..Meta::default() }, t));
        assert!(due(&Meta { not_after: 5000, renew_at: 900, ..Meta::default() }, t));
        assert!(!due(&Meta { fails: 1, next_try: 2000, ..Meta::default() }, t));
    }

    #[test]
    fn thumbprint_follows_rfc7638() {
        // RFC 7638 3.1 uses RSA; for EC the canonical member order is crv, kty, x, y
        let jwk = json!({"kty": "EC", "y": "Y", "x": "X", "crv": "P-256"});
        let want = B64U.encode(ring::digest::digest(
            &ring::digest::SHA256,
            br#"{"crv":"P-256","kty":"EC","x":"X","y":"Y"}"#,
        ));
        assert_eq!(thumbprint(&jwk), want);
    }

    #[test]
    fn the_address_certificate_is_chosen_per_connection() {
        let public: IpAddr = "54.36.211.82".parse().unwrap();
        let lan: IpAddr = "192.168.1.5".parse().unwrap();
        let outside: IpAddr = "8.8.8.8".parse().unwrap();
        let have_all = |_: &IpAddr| true;
        let have_none = |_: &IpAddr| false;
        assert_eq!(ip_for(public, outside, &have_all, &[]), Some(public));
        assert_eq!(ip_for(public, outside, &have_none, &[]), None);
        // a port forward: private address, client from the internet
        assert_eq!(ip_for(lan, outside, &have_all, &[public]), Some(public));
        // a client on the LAN keeps the bia's own certificate
        assert_eq!(ip_for(lan, "192.168.1.9".parse().unwrap(), &have_all, &[public]), None);
        // IPv4 seen through an IPv6 socket
        assert_eq!(ip_for("::ffff:54.36.211.82".parse().unwrap(), outside, &have_all, &[]), Some(public));
    }

    #[test]
    fn wanted_names_and_declared_addresses() {
        let files = std::env::temp_dir().join(format!("bia-acme-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&files);
        let own: Vec<IpAddr> = vec!["10.0.2.15".parse().unwrap(), "54.36.211.82".parse().unwrap()];
        set_declared_ips(&files, &["203.0.114.7".parse().unwrap()]).unwrap();
        set_hostnames(&files, &["bia.example.com".into()]).unwrap();
        let w = wanted(&files, &own);
        assert_eq!(
            w,
            vec![
                Name::Ip("54.36.211.82".parse().unwrap()),
                Name::Ip("203.0.114.7".parse().unwrap()),
                Name::Host("bia.example.com".into())
            ]
        );
        assert!(load(&files, &w[0]).is_none());
        assert!(describe(&files, &w[0]).contains("browsers warn"));
        let _ = std::fs::remove_dir_all(&files);
    }
}

#[cfg(test)]
mod real_cert {
    /// `BIA_TEST_LE_CERT=<pem> cargo test real_le_certificate -- --ignored`: a certificate
    /// Let's Encrypt really issued parses, and its ARI id is the one the CA answered for.
    #[test]
    #[ignore]
    fn real_le_certificate() {
        let path = std::env::var("BIA_TEST_LE_CERT").expect("BIA_TEST_LE_CERT");
        let pem = std::fs::read_to_string(path).unwrap();
        let (nb, na, id) = super::cert_facts(&pem).unwrap();
        println!("not_before {} not_after {} ari {:?}", super::fmt_time(nb), super::fmt_time(na), id);
        println!("default renew {}", super::fmt_time(super::default_renew_at(nb, na)));
    }
}

#[cfg(test)]
mod pebble {
    use super::*;

    /// Against a local Pebble (tests/manual/acme-pebble.sh): order, http-01 on a temporary
    /// port 80, finalize, download; then a renewal that names the old certificate.
    #[test]
    #[ignore]
    fn pebble_issues_and_renews_an_ip_certificate() {
        assert!(std::env::var("BIA_ACME_DIRECTORY").is_ok(), "set BIA_ACME_DIRECTORY and BIA_ACME_CA");
        let files = std::env::temp_dir().join(format!("bia-pebble-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&files);
        let n = Name::Ip("127.0.0.1".parse().unwrap());
        let p = pass(&files, &[n.clone()], false, Some(&n));
        assert_eq!(p.issued, vec![n.clone()], "{:?} {:?}", p, read_meta(&files, &n));
        let m = read_meta(&files, &n);
        let life = m.not_after - m.not_before;
        assert!(life <= 6 * 86400 + 3600, "shortlived profile: {life} s");
        assert!(m.renew_at > m.not_before && m.renew_at < m.not_after);
        assert!(load(&files, &n).is_some());
        let first = std::fs::read_to_string(name_dir(&files, &n).join("cert.pem")).unwrap();
        // the account is kept and reused; the renewal replaces the first certificate
        let p = pass(&files, &[n.clone()], false, Some(&n));
        assert_eq!(p.issued, vec![n.clone()], "{:?}", read_meta(&files, &n));
        let second = std::fs::read_to_string(name_dir(&files, &n).join("cert.pem")).unwrap();
        assert_ne!(first, second);
        println!("{}", describe(&files, &n));
        let _ = std::fs::remove_dir_all(&files);
    }
}
