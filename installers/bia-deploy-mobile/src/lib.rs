//! bia deploy on a phone: the server path of `bia-deploy` for the Android (Kotlin) and iOS
//! (Swift) apps, through UniFFI. A phone writes no sticks; it checks bia's brain (an API key
//! or a ChatGPT sign-in), logs in to a server over SSH, takes it over with the release's
//! takeover.sh, keeps bia's key (root's certificate) and waits for the new bia.
//!
//! Every call blocks: the apps call them off the main thread (coroutines, Swift tasks).
use bia_deploy::remote::{self, ssh, Agent, Auth, Deploy, HostKey, Progress};
use bia_deploy::{chatgpt, FmmInput};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

uniffi::setup_scaffolding!();

#[derive(Debug, uniffi::Error)]
pub enum DeployError {
    // not `message`: Kotlin exceptions have one already
    Failed { reason: String },
    /// A server this phone knows answered with another host key (reinstalled, or someone in
    /// between). Connecting again with `accept = now` trusts the new one.
    HostKeyChanged { known: String, now: String },
}

impl std::fmt::Display for DeployError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeployError::Failed { reason } => f.write_str(reason),
            DeployError::HostKeyChanged { known, now } => write!(f, "this server's identity changed: it was {known}, now it is {now}"),
        }
    }
}

impl std::error::Error for DeployError {}

impl From<anyhow::Error> for DeployError {
    fn from(e: anyhow::Error) -> Self {
        match e.downcast_ref::<HostKey>() {
            Some(HostKey::Changed { known, now }) => DeployError::HostKeyChanged { known: known.clone(), now: now.clone() },
            None => DeployError::Failed { reason: format!("{e:#}") },
        }
    }
}

type R<T> = Result<T, DeployError>;

/// The app's own folder (keys, known servers, ChatGPT registration, bia's keys). Call first.
#[uniffi::export]
pub fn init(app_dir: String) {
    std::env::set_var("BIA_DEPLOY_HOME", app_dir);
}

/// How the app shows the ChatGPT sign-in page (Custom Tab, in-app Safari): the page talks
/// to this app's own listener on 127.0.0.1, so it must stay in the app.
#[uniffi::export(with_foreign)]
pub trait BrowserOpener: Send + Sync {
    // not `open`: a Swift keyword, which UniFFI 0.32 escapes into the C header too
    fn open_url(&self, url: String) -> bool;
}

#[uniffi::export]
pub fn set_browser_opener(opener: Arc<dyn BrowserOpener>) {
    chatgpt::set_browser_opener(Box::new(move |url| opener.open_url(url.to_string())));
}

// ---- bia's brain ----

#[derive(uniffi::Record)]
pub struct CloudModel {
    pub id: String,
    pub name: String,
    /// Context window in tokens, 0 when unknown.
    pub ctx: u32,
}

fn models(v: Vec<bia_deploy::CloudModel>) -> Vec<CloudModel> {
    v.into_iter().map(|m| CloudModel { id: m.id, name: m.name, ctx: m.ctx }).collect()
}

#[derive(uniffi::Record)]
pub struct KeyGuide {
    pub provider: String,
    pub name: String,
    pub about: String,
    pub key_url: String,
    pub pay_url: Option<String>,
}

/// Where to get an API key, per provider ("How do I get a key?").
#[uniffi::export]
pub fn key_guides() -> Vec<KeyGuide> {
    bia_deploy::KEY_GUIDES
        .iter()
        .map(|g| KeyGuide {
            provider: g.provider.into(),
            name: g.name.into(),
            about: g.about.into(),
            key_url: g.key_url.into(),
            pay_url: g.pay_url.map(Into::into),
        })
        .collect()
}

#[uniffi::export]
pub fn key_steps() -> Vec<String> {
    bia_deploy::KEY_STEPS.iter().map(|s| s.to_string()).collect()
}

#[derive(uniffi::Record)]
pub struct KeyInfo {
    pub provider: String,
    pub base_url: String,
    pub models: Vec<CloudModel>,
}

/// The provider of a pasted key and its models (asks the APIs when the format is shared).
#[uniffi::export]
pub fn identify_key(key: String) -> R<KeyInfo> {
    let key = key.trim().to_string();
    if let Some(p) = bia_deploy::provider_for_key(&key) {
        let m = bia_deploy::list_models(p, &key, "")?;
        return Ok(KeyInfo { provider: p.into(), base_url: String::new(), models: models(m) });
    }
    let (p, base, m) = bia_deploy::probe_key(&key)?;
    Ok(KeyInfo { provider: p.into(), base_url: base, models: models(m) })
}

#[uniffi::export]
pub fn default_model(provider: String) -> String {
    bia_deploy::default_model(&provider).to_string()
}

/// bia's brain: an API key, or a ChatGPT sign-in (`chatgpt_json`, kept by the app as is).
#[derive(uniffi::Record, Clone)]
pub struct Brain {
    pub provider: String,
    pub api_key: String,
    pub model: String,
    pub base_url: String,
    pub ctx: u32,
    pub chatgpt_json: Option<String>,
}

fn fmm(b: &Brain) -> R<FmmInput> {
    let chatgpt = match &b.chatgpt_json {
        Some(j) => Some(serde_json::from_str(j).map_err(|e| DeployError::Failed { reason: format!("ChatGPT sign-in: {e}") })?),
        None => None,
    };
    Ok(FmmInput {
        provider: b.provider.clone(),
        api_key: b.api_key.clone(),
        model: b.model.clone(),
        base_url: b.base_url.clone(),
        ctx: if b.ctx == 0 { 131_072 } else { b.ctx },
        enabled: Some(true),
        chatgpt,
    })
}

#[derive(uniffi::Record)]
pub struct Greeting {
    /// What bia says when it wakes up on this brain.
    pub hello: String,
    pub summary: String,
    /// A ChatGPT sign-in after a refresh: the app keeps this one (the old one stops working).
    pub chatgpt_json: Option<String>,
}

/// Wake bia on this brain: one real request in which the model must answer through bia's
/// own tool. Nothing goes anywhere else.
#[uniffi::export]
pub fn wake(brain: Brain) -> R<Greeting> {
    let mut f = fmm(&brain)?.resolve()?;
    f.refresh_chatgpt()?;
    let g = bia_deploy::test_model(&f)?;
    let chatgpt_json = f.chatgpt.as_ref().map(|a| serde_json::to_string(a).unwrap_or_default());
    Ok(Greeting { hello: g.hello, summary: g.summary, chatgpt_json })
}

#[derive(uniffi::Record)]
pub struct ChatGptSignIn {
    pub email: String,
    pub chatgpt_json: String,
    pub models: Vec<CloudModel>,
    pub default_model: Option<String>,
}

/// "Continue with ChatGPT": the sign-in page opens through the BrowserOpener; this returns
/// when the person finished there (or it failed).
#[uniffi::export]
pub fn chatgpt_sign_in() -> R<ChatGptSignIn> {
    let auth = chatgpt::sign_in(None, &mut |_| {})?;
    let list = chatgpt::list_models(&auth.access_token).unwrap_or_default();
    let default_model = chatgpt::default_model(&list).map(|m| m.id.clone());
    Ok(ChatGptSignIn {
        email: auth.email.clone(),
        chatgpt_json: serde_json::to_string(&auth).map_err(anyhow::Error::from)?,
        models: models(list),
        default_model,
    })
}

#[derive(uniffi::Record)]
pub struct Language {
    pub code: String,
    pub name: String,
    pub voice: bool,
}

#[uniffi::export]
pub fn languages() -> Vec<Language> {
    bia_deploy::lang::languages().into_iter().map(|l| Language { code: l.code, name: l.name, voice: l.voice }).collect()
}

// ---- the server ----

#[derive(uniffi::Record)]
pub struct Login {
    pub host: String,
    pub port: u16,
    pub user: String,
    /// One of these: the server's password, or a private key (OpenSSH, PEM or .ppk).
    pub password: Option<String>,
    pub private_key: Option<String>,
    pub passphrase: Option<String>,
}

fn login(l: &Login) -> R<remote::Login> {
    let host = l.host.trim().trim_start_matches('[').trim_end_matches(']').to_string();
    if host.is_empty() {
        return Err(DeployError::Failed { reason: "type the server's address".into() });
    }
    let auth = match (&l.private_key, &l.password) {
        (Some(k), _) if !k.trim().is_empty() => Auth::Key { pem: k.clone(), passphrase: l.passphrase.clone().filter(|p| !p.is_empty()) },
        (_, Some(p)) if !p.is_empty() => Auth::Password(p.clone()),
        _ => return Err(DeployError::Failed { reason: "type the server's password, or give its key".into() }),
    };
    let user = if l.user.trim().is_empty() { "root".to_string() } else { l.user.trim().to_string() };
    Ok(remote::Login { host, port: if l.port == 0 { 22 } else { l.port }, user, auth })
}

#[derive(uniffi::Record)]
pub struct Facts {
    pub hostname: String,
    pub os: String,
    pub arch: String,
    pub ram_mib: u64,
    pub root_login: bool,
}

#[derive(uniffi::Record)]
pub struct PlanRow {
    pub what: String,
    pub value: String,
}

#[derive(uniffi::Record)]
pub struct Plan {
    pub disk: String,
    pub land: bool,
    pub rows: Vec<PlanRow>,
}

/// What goes onto the server: a new agent with this brain and language, or (no brain) a
/// land waiting for one of the owner's bias with `land_secret`.
#[derive(uniffi::Record)]
pub struct Target {
    pub host: String,
    pub brain: Option<Brain>,
    pub lang: String,
    pub land_secret: Option<String>,
}

fn deploy(t: &Target) -> R<Deploy> {
    let agent = match (&t.brain, &t.land_secret) {
        (Some(b), _) => Agent::New { fmm: fmm(b)?, lang: t.lang.clone() },
        (None, Some(s)) => Agent::Land { secret: s.clone() },
        (None, None) => return Err(DeployError::Failed { reason: "no brain and no land secret".into() }),
    };
    Ok(Deploy { host: t.host.trim().trim_start_matches('[').trim_end_matches(']').to_string(), agent, os: None })
}

#[derive(uniffi::Record)]
pub struct Done {
    /// Folder with bia-root.p12 and ca.crt (a new agent; none for a land).
    pub folder: Option<String>,
    pub ca_pem: String,
    pub ca_sha256: String,
    pub agent_name: String,
}

/// What the takeover reports while it runs.
#[uniffi::export(with_foreign)]
pub trait RunListener: Send + Sync {
    /// A line of the takeover's own text; `== …` lines are steps, with a fraction when the
    /// step reports a percentage.
    fn step(&self, text: String, fraction: Option<f32>, detail: String);
    fn line(&self, text: String);
    /// bia's key (root's certificate) is saved on this phone.
    fn certificate(&self, path: String);
    fn rebooting(&self);
}

/// One server, from login to the reboot into bia's installer.
#[derive(uniffi::Object)]
pub struct Server {
    ssh: Mutex<Option<Arc<ssh::Ssh>>>,
}

impl Server {
    fn session(&self) -> R<Arc<ssh::Ssh>> {
        self.ssh.lock().unwrap().clone().ok_or_else(|| DeployError::Failed { reason: "not connected".into() })
    }
}

#[uniffi::export]
impl Server {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Server { ssh: Mutex::new(None) })
    }

    /// Log in and look at the server. A server seen for the first time is trusted; a
    /// changed host key is HostKeyChanged (then `accept` = its new key, after the person
    /// says it was reinstalled).
    pub fn connect(&self, login_: Login, accept: Option<String>) -> R<Facts> {
        let l = login(&login_)?;
        if accept.is_some() {
            let _ = ssh::forget(&l.id());
        }
        let s = ssh::Ssh::connect(&l, accept.as_deref())?;
        let f = remote::facts(&s)?;
        *self.ssh.lock().unwrap() = Some(Arc::new(s));
        Ok(Facts { hostname: f.hostname, os: f.os, arch: f.arch, ram_mib: f.ram_mib, root_login: f.root_login })
    }

    /// Every check on the server; nothing changes.
    pub fn plan(&self, target: Target, listener: Arc<dyn RunListener>) -> R<Plan> {
        let s = self.session()?;
        let p = remote::plan(&s, &deploy(&target)?, &mut |l| listener.line(l))?;
        Ok(Plan {
            disk: p.disk,
            land: p.land,
            rows: p.rows.into_iter().filter(|r| r.0 != "staging").map(|(what, value, _)| PlanRow { what, value }).collect(),
        })
    }

    /// Replace the server's system with bia (on the disk the plan named). Returns once it
    /// reboots into bia's installer.
    pub fn run(&self, target: Target, disk: String, listener: Arc<dyn RunListener>) -> R<Done> {
        let s = self.session()?;
        let l = listener.clone();
        let on = move |p: Progress| match p {
            Progress::Line(t) => {
                if let Some(step) = t.strip_prefix("== ") {
                    match bia_deploy::parse_progress(step) {
                        Some((what, f, detail)) => l.step(what.into(), Some(f), detail.into()),
                        None => l.step(step.into(), None, String::new()),
                    }
                } else {
                    l.line(t)
                }
            }
            Progress::Certificate(p) => l.certificate(p.display().to_string()),
            Progress::Rebooting => l.rebooting(),
        };
        let d = remote::run(&s, &deploy(&target)?, &disk, &on)?;
        *self.ssh.lock().unwrap() = None;
        Ok(Done {
            folder: d.folder.map(|f| f.display().to_string()),
            ca_pem: d.ca_pem,
            ca_sha256: d.ca_sha256,
            agent_name: d.agent_name,
        })
    }
}

#[uniffi::export(with_foreign)]
pub trait WaitListener: Send + Sync {
    fn waited(&self, seconds: u64);
}

/// Wait until the new bia answers (its web app with that CA; a land: its land port).
#[uniffi::export]
pub fn wait_up(host: String, ca_pem: Option<String>, minutes: u32, listener: Arc<dyn WaitListener>) -> R<()> {
    let ca = ca_pem.filter(|c| !c.is_empty());
    remote::wait_up(&host, ca.as_deref(), Duration::from_secs(minutes as u64 * 60), &mut |t| listener.waited(t.as_secs()))?;
    Ok(())
}

// ---- keys ----

#[derive(uniffi::Record)]
pub struct SshKey {
    /// `ssh-ed25519 AAAA… comment`: what the hosting provider asks for.
    pub public: String,
    /// The private key file on this phone (Login.private_key takes its text).
    pub path: String,
}

/// A new SSH key for a server that is still to be ordered.
#[uniffi::export]
pub fn new_key(comment: String) -> R<SshKey> {
    let k = remote::keys::generate(&comment)?;
    Ok(SshKey { public: k.public, path: k.path.display().to_string() })
}

/// The keys made earlier, newest first.
#[uniffi::export]
pub fn saved_keys() -> Vec<SshKey> {
    remote::keys::saved().into_iter().map(|(p, k)| SshKey { public: k, path: p.display().to_string() }).collect()
}

#[uniffi::export]
pub fn read_key(path: String) -> R<String> {
    std::fs::read_to_string(&path).map_err(|e| DeployError::Failed { reason: format!("{path}: {e}") })
}

/// The public line of a pasted private key (to check it reads).
#[uniffi::export]
pub fn public_key_of(private_key: String, passphrase: Option<String>) -> R<String> {
    Ok(remote::keys::public_of(&private_key, passphrase.as_deref())?)
}

/// A land secret for a land (140 bits, groups of four).
#[uniffi::export]
pub fn land_secret() -> String {
    bia_deploy::takeover::land_secret()
}

// ---- bia's key on this phone ----

/// A bia this phone deployed, whose key (bia-root.p12) and CA (ca.crt) are kept here.
#[derive(uniffi::Record)]
pub struct SavedBia {
    pub host: String,
    pub folder: String,
    /// When its key was saved (unix seconds).
    pub saved_at: u64,
}

/// The bias whose keys are on this phone, newest first: for sharing a key again later.
#[uniffi::export]
pub fn saved_bias() -> Vec<SavedBia> {
    let Some(dir) = bia_deploy::app_dir().map(|d| d.join("servers")) else { return vec![] };
    let mut out: Vec<SavedBia> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let folder = e.path();
            let saved_at = std::fs::metadata(folder.join("bia-root.p12"))
                .and_then(|m| m.modified())
                .ok()?
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            Some(SavedBia { host: e.file_name().to_string_lossy().into_owned(), folder: folder.display().to_string(), saved_at })
        })
        .collect();
    out.sort_by(|a, b| b.saved_at.cmp(&a.saved_at));
    out
}

/// The password on bia-root.p12.
#[uniffi::export]
pub fn p12_password() -> String {
    remote::trust::P12_PASSWORD.into()
}

/// iOS: a configuration profile with bia's key and its CA. Safari (in the app) opens the URL
/// this returns; iOS then offers the profile in Settings. Served once, from this phone only.
#[uniffi::export]
pub fn serve_profile(folder: String, name: String) -> R<String> {
    let profile = mobileconfig(Path::new(&folder), &name)?;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(anyhow::Error::from)?;
    let port = listener.local_addr().map_err(anyhow::Error::from)?.port();
    std::thread::spawn(move || {
        // a few tries: Safari may probe first
        for _ in 0..5 {
            let Ok((mut c, _)) = listener.accept() else { return };
            use std::io::{Read, Write};
            let mut buf = [0u8; 4096];
            let _ = c.read(&mut buf);
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/x-apple-aspen-config\r\nContent-Disposition: attachment; filename=\"bia.mobileconfig\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                profile.len()
            );
            let _ = c.write_all(head.as_bytes()).and_then(|_| c.write_all(&profile));
        }
    });
    Ok(format!("http://127.0.0.1:{port}/bia.mobileconfig"))
}

fn mobileconfig(folder: &Path, name: &str) -> R<Vec<u8>> {
    use base64::Engine;
    let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
    let read = |f: &str| -> R<Vec<u8>> {
        std::fs::read(folder.join(f)).map_err(|e| DeployError::Failed { reason: format!("{f}: {e}") })
    };
    let p12 = read("bia-root.p12")?;
    let ca_pem = String::from_utf8_lossy(&read("ca.crt")?).to_string();
    let ca_der: String = ca_pem.lines().filter(|l| !l.starts_with("-----")).collect();
    let id = uuid_like(&ca_der);
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>PayloadType</key><string>Configuration</string>
  <key>PayloadVersion</key><integer>1</integer>
  <key>PayloadIdentifier</key><string>biadeploy.{id}</string>
  <key>PayloadUUID</key><string>{id}-0000</string>
  <key>PayloadDisplayName</key><string>{name}</string>
  <key>PayloadDescription</key><string>bia's key (root's certificate) and the certificate authority of this bia.</string>
  <key>PayloadContent</key>
  <array>
    <dict>
      <key>PayloadType</key><string>com.apple.security.root</string>
      <key>PayloadVersion</key><integer>1</integer>
      <key>PayloadIdentifier</key><string>biadeploy.{id}.ca</string>
      <key>PayloadUUID</key><string>{id}-0001</string>
      <key>PayloadDisplayName</key><string>{name} certificate authority</string>
      <key>PayloadContent</key><data>{ca_der}</data>
    </dict>
    <dict>
      <key>PayloadType</key><string>com.apple.security.pkcs12</string>
      <key>PayloadVersion</key><integer>1</integer>
      <key>PayloadIdentifier</key><string>biadeploy.{id}.root</string>
      <key>PayloadUUID</key><string>{id}-0002</string>
      <key>PayloadDisplayName</key><string>{name} root</string>
      <key>Password</key><string>{pw}</string>
      <key>PayloadContent</key><data>{p12}</data>
    </dict>
  </array>
</dict>
</plist>
"#,
        name = esc(name),
        pw = remote::trust::P12_PASSWORD,
        p12 = b64(&p12),
    );
    Ok(xml.into_bytes())
}

/// A stable UUID-shaped id from the CA (the same bia gets the same profile id: a new one
/// replaces the old in Settings).
fn uuid_like(seed: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut a = std::collections::hash_map::DefaultHasher::new();
    seed.hash(&mut a);
    let x = a.finish();
    let mut b = std::collections::hash_map::DefaultHasher::new();
    (seed, x).hash(&mut b);
    let y = b.finish();
    format!("{:08X}-{:04X}-{:04X}-{:04X}", (x >> 32) as u32, (x >> 16) as u16, x as u16, (y >> 48) as u16)
}

/// Where bia's keys for `host` are kept on this phone.
#[uniffi::export]
pub fn server_folder(host: String) -> R<String> {
    Ok(remote::server_folder(&host)?.display().to_string())
}
