//! SSH to the server bia-deploy takes over (russh): password or key login, the server's
//! host key pinned on first contact, commands with stdin and streamed output, and root
//! through sudo when the login is another user.
use anyhow::{anyhow, bail, Context, Result};
use russh::client;
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, PublicKey, PublicKeyOrCertificate};
use russh::ChannelMsg;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone)]
pub enum Auth {
    Password(String),
    /// An OpenSSH, PEM or PuTTY (.ppk) private key, with its passphrase if it has one.
    Key { pem: String, passphrase: Option<String> },
}

#[derive(Clone)]
pub struct Login {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: Auth,
}

impl Login {
    /// `host:port` as the host key store keys it (`[v6]:port` for IPv6).
    pub fn id(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
}

/// The server's host key is not the one this computer knows (or none is known yet).
/// The caller shows the fingerprint and connects again with `accept` set to it.
#[derive(Debug, Clone)]
pub enum HostKey {
    New(String),
    Changed { known: String, now: String },
}

impl std::fmt::Display for HostKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HostKey::New(fp) => write!(f, "first connection to this server (its key is {fp})"),
            HostKey::Changed { known, now } => write!(
                f,
                "this server's identity changed: it was {known}, now it is {now}. A reinstalled server gets a new one; otherwise someone may be in between"
            ),
        }
    }
}

impl std::error::Error for HostKey {}

/// `SHA256:…`, as `ssh-keygen -l` prints it.
pub fn fingerprint(k: &PublicKey) -> String {
    k.fingerprint(HashAlg::Sha256).to_string()
}

fn known_path() -> Option<std::path::PathBuf> {
    crate::app_dir().map(|d| d.join("known_hosts.json"))
}

fn known_all() -> BTreeMap<String, String> {
    known_path().and_then(|p| std::fs::read(p).ok()).and_then(|d| serde_json::from_slice(&d).ok()).unwrap_or_default()
}

/// The host key fingerprint remembered for `host:port`.
pub fn known(id: &str) -> Option<String> {
    known_all().remove(id)
}

pub fn remember(id: &str, fp: &str) -> Result<()> {
    let mut all = known_all();
    all.insert(id.to_string(), fp.to_string());
    let p = known_path().context("no folder for this app's settings")?;
    std::fs::create_dir_all(p.parent().unwrap())?;
    std::fs::write(&p, serde_json::to_vec_pretty(&all)?)?;
    Ok(())
}

pub fn forget(id: &str) -> Result<()> {
    let mut all = known_all();
    if all.remove(id).is_some() {
        if let Some(p) = known_path() {
            std::fs::write(&p, serde_json::to_vec_pretty(&all)?)?;
        }
    }
    Ok(())
}

struct Client {
    want: Option<String>,
    seen: Arc<Mutex<Option<String>>>,
}

impl client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(&mut self, k: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        let fp = match k {
            PublicKeyOrCertificate::PublicKey { key, .. } => fingerprint(key),
            PublicKeyOrCertificate::Certificate(c) => c.public_key().fingerprint(HashAlg::Sha256).to_string(),
        };
        *self.seen.lock().unwrap() = Some(fp.clone());
        Ok(self.want.as_deref() == Some(fp.as_str()))
    }
}

/// What a command printed and how it ended.
pub struct Output {
    pub code: u32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl Output {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).trim().to_string()
    }
}

pub enum Stream {
    Out,
    Err,
}

/// Root's password for sudo (`None`: root itself, or sudo without a password).
#[derive(Clone)]
enum Sudo {
    Root,
    NoPassword,
    Password(String),
}

pub struct Ssh {
    rt: tokio::runtime::Runtime,
    handle: client::Handle<Client>,
    pub user: String,
    pub host_key: String,
    sudo: Sudo,
}

/// Wrap `s` in single quotes for a POSIX shell.
pub fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

impl Ssh {
    /// Log in. The host key must be the remembered one, or `accept` (a fingerprint the
    /// person agreed to); otherwise the error is a [`HostKey`].
    pub fn connect(l: &Login, accept: Option<&str>) -> Result<Ssh> {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()?;
        let known = known(&l.id());
        let want = accept.map(str::to_string).or(known.clone());
        let seen = Arc::new(Mutex::new(None));
        let cfg = Arc::new(client::Config {
            inactivity_timeout: Some(Duration::from_secs(600)),
            keepalive_interval: Some(Duration::from_secs(15)),
            keepalive_max: 8,
            ..Default::default()
        });
        let handler = Client { want: want.clone(), seen: seen.clone() };
        let addr = (l.host.trim().to_string(), l.port);
        let connected = rt.block_on(async {
            tokio::time::timeout(Duration::from_secs(20), client::connect(cfg, addr, handler)).await
        });
        let mut handle = match connected {
            Err(_) => bail!("{} does not answer on port {} (wrong address or port, or a firewall)", l.host, l.port),
            Ok(Ok(h)) => h,
            Ok(Err(e)) => {
                if let Some(now) = seen.lock().unwrap().clone() {
                    if want.as_deref() != Some(now.as_str()) {
                        return Err(match known {
                            Some(k) if accept.is_none() => HostKey::Changed { known: k, now },
                            _ => HostKey::New(now),
                        }
                        .into());
                    }
                }
                return Err(anyhow!(e)).with_context(|| format!("cannot connect to {} port {}", l.host, l.port));
            }
        };
        let host_key = seen.lock().unwrap().clone().unwrap_or_default();
        let ok = rt.block_on(async {
            match &l.auth {
                Auth::Password(p) => {
                    let r = handle.authenticate_password(&l.user, p).await?;
                    if r.success() {
                        return Ok::<bool, anyhow::Error>(true);
                    }
                    // some servers take the password only as keyboard-interactive
                    let mut r = handle.authenticate_keyboard_interactive_start(&l.user, None).await?;
                    for _ in 0..4 {
                        match r {
                            client::KeyboardInteractiveAuthResponse::Success => return Ok(true),
                            client::KeyboardInteractiveAuthResponse::Failure { .. } => return Ok(false),
                            client::KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => {
                                let answers = prompts.iter().map(|_| p.clone()).collect();
                                r = handle.authenticate_keyboard_interactive_respond(answers).await?;
                            }
                        }
                    }
                    Ok(false)
                }
                Auth::Key { pem, passphrase } => {
                    let key = russh::keys::decode_secret_key(pem.trim(), passphrase.as_deref().filter(|p| !p.is_empty()))
                        .map_err(|e| anyhow!("{}", key_error(&e.to_string(), passphrase.is_some())))?;
                    let hash = handle.best_supported_rsa_hash().await?.flatten();
                    let r = handle.authenticate_publickey(&l.user, PrivateKeyWithHashAlg::new(Arc::new(key), hash)).await?;
                    Ok(r.success())
                }
            }
        })?;
        if !ok {
            bail!(
                "{} did not let {} in: {}",
                l.host,
                l.user,
                match l.auth {
                    Auth::Password(_) => "wrong password, or this server takes only keys",
                    Auth::Key { .. } => "this server does not know that key (or the user name is wrong)",
                }
            );
        }
        if accept.is_some() && known.as_deref() != Some(host_key.as_str()) {
            remember(&l.id(), &host_key)?;
        }
        let mut s = Ssh { rt, handle, user: l.user.clone(), host_key, sudo: Sudo::Root };
        s.sudo = s.find_sudo(match &l.auth {
            Auth::Password(p) => Some(p.as_str()),
            Auth::Key { .. } => None,
        })?;
        Ok(s)
    }

    /// How this login becomes root.
    fn find_sudo(&self, password: Option<&str>) -> Result<Sudo> {
        let id = self.run_plain("id -u", None)?;
        if id.code != 0 {
            // AWS-style "Please login as the user ubuntu rather than root" and the like
            let said = String::from_utf8_lossy(&id.stdout).trim().to_string();
            bail!("the server refused to run commands as {}{}", self.user, if said.is_empty() { String::new() } else { format!(": {said}") });
        }
        if id.text() == "0" {
            return Ok(Sudo::Root);
        }
        if self.run_plain("sudo -n true", None)?.code == 0 {
            return Ok(Sudo::NoPassword);
        }
        if let Some(p) = password {
            let r = self.run_plain("sudo -k -S -p '' true", Some(format!("{p}\n").into_bytes()))?;
            if r.code == 0 {
                return Ok(Sudo::Password(p.to_string()));
            }
        }
        bail!(
            "{} is not root and cannot use sudo{}; log in as root, or give {} sudo rights",
            self.user,
            if password.is_some() { " with this password" } else { " without a password" },
            self.user
        )
    }

    pub fn is_root_login(&self) -> bool {
        matches!(self.sudo, Sudo::Root)
    }

    fn run_plain(&self, cmd: &str, stdin: Option<Vec<u8>>) -> Result<Output> {
        let (mut out, mut err) = (vec![], vec![]);
        let code = self.exec(cmd, stdin, &mut |s, b| match s {
            Stream::Out => out.extend_from_slice(b),
            Stream::Err => err.extend_from_slice(b),
        })?;
        Ok(Output { code, stdout: out, stderr: err })
    }

    /// Run `cmd` (a shell command line) as the login user.
    pub fn run(&self, cmd: &str) -> Result<Output> {
        self.run_plain(cmd, None)
    }

    /// Run `cmd` as root (through sudo when needed); `stdin` goes to the command.
    pub fn run_root(&self, cmd: &str, stdin: Option<Vec<u8>>, on: &mut dyn FnMut(Stream, &[u8])) -> Result<u32> {
        let (line, stdin) = match &self.sudo {
            Sudo::Root => (format!("sh -c {}", quote(cmd)), stdin),
            Sudo::NoPassword => (format!("sudo -n sh -c {}", quote(cmd)), stdin),
            Sudo::Password(p) => {
                // sudo reads the password line itself, the rest reaches the command; -k: it
                // always asks (a cached sudo would pass the password on to the command)
                let mut b = format!("{p}\n").into_bytes();
                b.extend(stdin.unwrap_or_default());
                (format!("sudo -k -S -p '' sh -c {}", quote(cmd)), Some(b))
            }
        };
        self.exec(&line, stdin, on)
    }

    fn exec(&self, cmd: &str, stdin: Option<Vec<u8>>, on: &mut dyn FnMut(Stream, &[u8])) -> Result<u32> {
        self.rt.block_on(async {
            let mut ch = self.handle.channel_open_session().await.context("open an ssh channel")?;
            ch.exec(true, cmd).await?;
            if let Some(b) = stdin {
                ch.data_bytes(b).await?;
            }
            ch.eof().await?;
            let mut code = None;
            while let Some(m) = ch.wait().await {
                match m {
                    ChannelMsg::Data { data } => on(Stream::Out, &data),
                    ChannelMsg::ExtendedData { data, .. } => on(Stream::Err, &data),
                    ChannelMsg::ExitStatus { exit_status } => code = Some(exit_status),
                    _ => {}
                }
            }
            // no exit status: the connection went away (a reboot ends it like this)
            Ok(code.unwrap_or(255))
        })
    }

}

fn key_error(e: &str, had_passphrase: bool) -> String {
    let l = e.to_ascii_lowercase();
    if l.contains("encrypted") || l.contains("passphrase") || l.contains("decrypt") || l.contains("crypto") {
        if had_passphrase {
            "the key's passphrase is wrong".into()
        } else {
            "this key is protected by a passphrase; type it in".into()
        }
    } else {
        format!("this is not a private key bia-deploy can read ({e}); paste the whole file, from the BEGIN line to the END line")
    }
}
