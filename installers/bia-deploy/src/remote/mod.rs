//! Put bia on a server from this computer: log in over SSH, run the release's
//! `takeover.sh` there in `--events` mode (once with `--dump-config` for the plan, once
//! for real), fetch root's certificate from the takeover's gate through the same SSH
//! login while the takeover waits for it, then wait for the new bia to answer.
//! Nothing on the server changes before [`run`].
pub mod keys;
pub mod ssh;
pub mod trust;

use crate::fmm::FmmInput;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use ssh::{Ssh, Stream};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub use ssh::{Auth, HostKey, Login};

/// The default release; `BIA_RELEASE_URL` replaces it (tests, local releases).
const RELEASE: &str = "https://github.com/zond80/bia/releases/latest/download";

/// What the server is, before anything is decided.
#[derive(Debug, Clone, Default)]
pub struct Facts {
    pub hostname: String,
    /// `PRETTY_NAME` from os-release, else the kernel name.
    pub os: String,
    pub arch: String,
    pub ram_mib: u64,
    /// The login is root itself (not through sudo).
    pub root_login: bool,
}

pub fn facts(s: &Ssh) -> Result<Facts> {
    let o = s.run(
        "uname -s; uname -m; hostname; (. /etc/os-release 2>/dev/null && echo \"$PRETTY_NAME\") || echo; \
         awk '/^MemTotal:/ { print int($2 / 1024) }' /proc/meminfo 2>/dev/null || echo 0; \
         if [ -e /etc/bia-release ] || [ -e /etc/bia-shipped ]; then echo bia; else echo other; fi",
    )?;
    let t = o.text();
    let l: Vec<&str> = t.lines().map(str::trim).collect();
    let get = |i: usize| l.get(i).copied().unwrap_or("").to_string();
    if get(0) != "Linux" {
        bail!("this server runs {}, not Linux; bia can only take over a Linux server", if get(0).is_empty() { "something unknown".into() } else { get(0) });
    }
    if get(5) == "bia" {
        bail!("this server already runs bia; a takeover would erase it. Nothing was changed. To update it, use its own update (console /update).");
    }
    let arch = match get(1).as_str() {
        "x86_64" | "amd64" => "x86_64",
        "aarch64" | "arm64" => "arm64",
        a => bail!("bia runs on x86_64 and arm64 servers, not {a}"),
    };
    Ok(Facts {
        hostname: get(2),
        os: if get(3).is_empty() { "Linux".into() } else { get(3) },
        arch: arch.into(),
        ram_mib: get(4).parse().unwrap_or(0),
        root_login: s.is_root_login(),
    })
}

/// What goes onto the server.
#[derive(Clone)]
pub struct Deploy {
    /// The address the person typed (goes into bia's HTTPS certificate).
    pub host: String,
    /// A new agent (with its model and language) or land mode with this secret.
    pub agent: Agent,
    /// `latest`, a release tag, or an https:// release directory.
    pub os: Option<String>,
}

#[derive(Clone)]
pub enum Agent {
    New { fmm: FmmInput, lang: String },
    /// bia waits for one of the owner's bias to fly its agent there with this secret.
    Land { secret: String },
}

fn config(d: &Deploy, wipe: Option<&str>) -> Result<Value> {
    let mut v = json!({});
    match &d.agent {
        Agent::New { fmm, lang } => {
            let mut f = json!({"provider": fmm.provider, "api_key": fmm.api_key, "model": fmm.model, "ctx": fmm.ctx});
            if !fmm.base_url.trim().is_empty() {
                f["base_url"] = fmm.base_url.clone().into();
            }
            if let Some(c) = &fmm.chatgpt {
                f["chatgpt"] = serde_json::to_value(c)?;
            }
            v["fmm"] = f;
            v["lang"] = lang.clone().into();
            v["setup"] = "".into();
            // the address typed here reaches bia even behind NAT: put it in the certificate
            let h = d.host.trim().trim_start_matches('[').trim_end_matches(']');
            v["pki"] = if h.parse::<std::net::IpAddr>().is_ok() { json!({"ips": [h]}) } else { json!({"names": [h]}) };
            // the gate is fetched by this app at once; if this app goes away, give up unchanged
            v["gate_wait"] = 900.into();
        }
        Agent::Land { secret } => v["land"] = json!({"secret": secret}),
    }
    if let Some(o) = d.os.as_ref().filter(|o| !o.trim().is_empty()) {
        v["os"] = o.clone().into();
    }
    if let Some(w) = wipe {
        v["disk"] = w.into();
        v["wipe"] = w.into();
        v["reboot"] = true.into();
    }
    Ok(v)
}

/// The shell line run as root: config from stdin into a private file, takeover.sh from the
/// release, the takeover itself; the config file is removed afterwards.
fn takeover_line(dump: bool) -> String {
    let rel = std::env::var("BIA_RELEASE_URL").unwrap_or_else(|_| RELEASE.into());
    let rel = ssh::quote(rel.trim_end_matches('/'));
    format!(
        "umask 077; f=$(mktemp /root/.bia-deploy.XXXXXX) || exit 1; cat >\"$f\"; \
         if command -v curl >/dev/null 2>&1; then curl -fsSL --retry 3 {rel}/takeover.sh; else wget -qO- {rel}/takeover.sh; fi >\"$f.sh\" \
         || {{ echo 'cannot download takeover.sh from the bia release (does this server reach the internet?)' >&2; rm -f \"$f\" \"$f.sh\"; exit 1; }}; \
         BIA_RELEASE_URL={rel} bash \"$f.sh\" --config \"$f\" --events --yes{}; rc=$?; rm -f \"$f\" \"$f.sh\"; exit $rc",
        if dump { " --dump-config" } else { "" }
    )
}

/// One line the takeover wrote on the server, or what it means.
#[derive(Debug, Clone)]
pub enum Progress {
    /// A line of the takeover's own text (`== …` lines are steps).
    Line(String),
    /// Root's certificate was saved on this computer.
    Certificate(PathBuf),
    /// The takeover reboots the server now.
    Rebooting,
}

#[derive(Debug, Clone)]
pub struct Plan {
    pub disk: String,
    pub land: bool,
    /// (what, value, where it came from), as the takeover prints its summary.
    pub rows: Vec<(String, String, String)>,
}

/// Lines split from a byte stream.
#[derive(Default)]
struct Lines(Vec<u8>);

impl Lines {
    fn feed(&mut self, b: &[u8], out: &mut dyn FnMut(String)) {
        self.0.extend_from_slice(b);
        while let Some(i) = self.0.iter().position(|&c| c == b'\n' || c == b'\r') {
            let l: Vec<u8> = self.0.drain(..=i).collect();
            let s = String::from_utf8_lossy(&l[..l.len() - 1]).trim_end().to_string();
            if !s.is_empty() {
                out(s);
            }
        }
    }
}

/// Run takeover.sh with the config; `event` gets each `--events` line, `line` the text.
fn takeover(s: &Ssh, cfg: &Value, dump: bool, event: &mut dyn FnMut(&Value), line: &mut dyn FnMut(String)) -> Result<()> {
    let (mut out, mut err) = (Lines::default(), Lines::default());
    let mut tail: Vec<String> = vec![];
    let code = s.run_root(&takeover_line(dump), Some(serde_json::to_vec(cfg)?), &mut |st, b| match st {
        Stream::Out => out.feed(b, &mut |l| match serde_json::from_str::<Value>(&l) {
            Ok(v) if v.get("event").is_some() => event(&v),
            _ => line(l),
        }),
        Stream::Err => err.feed(b, &mut |l| {
            tail.push(l.clone());
            if tail.len() > 6 {
                tail.remove(0);
            }
            line(l)
        }),
    })?;
    if code != 0 {
        let why = tail.iter().rev().find(|l| l.starts_with("Error:") || l.starts_with("takeover:")).cloned().or_else(|| tail.last().cloned());
        bail!("{}", why.map(|w| w.trim_start_matches("Error:").trim().to_string()).unwrap_or_else(|| format!("the takeover stopped (exit {code})")));
    }
    Ok(())
}

/// Every check on the server, nothing changed: what would happen.
pub fn plan(s: &Ssh, d: &Deploy, line: &mut dyn FnMut(String)) -> Result<Plan> {
    let mut plan = None;
    takeover(s, &config(d, None)?, true, &mut |v| {
        if v["event"] == "plan" {
            plan = Some(Plan {
                disk: v["disk"].as_str().unwrap_or("").to_string(),
                land: v["land"].as_bool().unwrap_or(false),
                rows: v["rows"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|r| (r[0].as_str().unwrap_or("").into(), r[1].as_str().unwrap_or("").into(), r[2].as_str().unwrap_or("").into()))
                    .collect(),
            });
        }
    }, line)?;
    let p = plan.context("the takeover gave no plan (an older release?)")?;
    if p.disk.is_empty() {
        bail!("the takeover named no disk");
    }
    Ok(p)
}

/// The new bia: where its root certificate is and how to know it.
#[derive(Debug, Clone, Default)]
pub struct Done {
    /// Folder on this computer holding `bia-root.p12` and `ca.crt` (new agent only).
    pub folder: Option<PathBuf>,
    pub ca_pem: String,
    pub ca_sha256: String,
    pub agent_name: String,
}

/// This computer's copy of a server's root certificate and CA.
pub fn server_folder(host: &str) -> Result<PathBuf> {
    let safe: String = host.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' }).collect();
    Ok(crate::app_dir().context("no folder for this app's settings")?.join("servers").join(safe))
}

/// Replace the server's system with bia. `disk` is the one the plan named. Returns once
/// the takeover reboots the server.
pub fn run(s: &Ssh, d: &Deploy, disk: &str, on: &(dyn Fn(Progress) + Sync)) -> Result<Done> {
    let cfg = config(d, Some(disk))?;
    let (tx, rx) = mpsc::channel::<Value>();
    let mut done = Done::default();
    let mut ready = false;
    let host = d.host.clone();
    std::thread::scope(|sc| -> Result<()> {
        // root's certificate: fetched from the gate on the server's own loopback, as soon as
        // the takeover opens it; the takeover waits for this before it changes anything
        let fetch = sc.spawn(move || -> Result<Option<Done>> {
            let Ok(g) = rx.recv() else { return Ok(None) };
            let (port, token) = (g["port"].as_u64().unwrap_or(0), g["token"].as_str().unwrap_or(""));
            if port == 0 || token.is_empty() || !token.chars().all(|c| c.is_ascii_hexdigit()) {
                bail!("the takeover's gate event is broken");
            }
            let url = format!("https://127.0.0.1:{port}/{token}/root.p12");
            let mut last = String::new();
            for _ in 0..10 {
                let o = s.run(&format!(
                    "if command -v curl >/dev/null 2>&1; then curl -fsk {url}; else wget -qO- --no-check-certificate {url}; fi"
                ))?;
                // PKCS#12 is one DER SEQUENCE
                if o.code == 0 && o.stdout.len() > 100 && o.stdout[0] == 0x30 {
                    let dir = server_folder(&host)?;
                    std::fs::create_dir_all(&dir)?;
                    let p12 = dir.join("bia-root.p12");
                    if p12.exists() {
                        // an older bia at the same address: keep its key next to the new one
                        let _ = std::fs::rename(&p12, dir.join(format!("bia-root.{}.p12", unix_now())));
                    }
                    std::fs::write(&p12, &o.stdout)?;
                    let ca = g["ca_pem"].as_str().unwrap_or("").to_string();
                    std::fs::write(dir.join("ca.crt"), &ca)?;
                    on(Progress::Certificate(p12));
                    return Ok(Some(Done {
                        folder: Some(dir),
                        ca_pem: ca,
                        ca_sha256: g["ca_sha256"].as_str().unwrap_or("").into(),
                        agent_name: g["agent_name"].as_str().unwrap_or("").into(),
                    }));
                }
                last = String::from_utf8_lossy(&o.stderr).trim().to_string();
                std::thread::sleep(Duration::from_secs(2));
            }
            bail!("could not fetch root's certificate from the server ({last}); the takeover gives up by itself, nothing was changed")
        });
        let r = takeover(s, &cfg, false, &mut |v| match v["event"].as_str() {
            Some("gate") => {
                let _ = tx.send(v.clone());
            }
            Some("ready") => {
                ready = true;
                on(Progress::Rebooting);
            }
            _ => {}
        }, &mut |l| on(Progress::Line(l)));
        drop(tx);
        let got = fetch.join().map_err(|_| anyhow::anyhow!("certificate download crashed"))?;
        // the reboot ends the ssh session without an exit status
        if !ready {
            r?;
        }
        if let Some(g) = got? {
            done = g;
        }
        Ok(())
    })?;
    if !ready {
        bail!("the takeover ended without rebooting into bia");
    }
    if matches!(d.agent, Agent::New { .. }) && done.folder.is_none() {
        bail!("the server is rebooting, but root's certificate never arrived");
    }
    Ok(done)
}

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Wait until the new bia answers: its web app with the new CA's certificate (new agent),
/// or the land port (land mode). `limit` is how long to keep trying.
pub fn wait_up(host: &str, ca_pem: Option<&str>, limit: Duration, tick: &mut dyn FnMut(Duration)) -> Result<()> {
    let h = host.trim().trim_start_matches('[').trim_end_matches(']');
    let hp = if h.contains(':') { format!("[{h}]") } else { h.to_string() };
    let start = Instant::now();
    // the old system is still up for a moment after the reboot command
    std::thread::sleep(Duration::from_secs(20));
    let mut last = String::new();
    while start.elapsed() < limit {
        tick(start.elapsed());
        let ok = match ca_pem {
            Some(ca) => bia_pki::client::request(&format!("https://{hp}/"), "GET", "/", None, bia_pki::client::ServerTrust::Ca(ca), None)
                .map(|_| ())
                .map_err(|e| format!("{e:#}")),
            None => std::net::ToSocketAddrs::to_socket_addrs(&(h, 4791u16))
                .map_err(|e| e.to_string())
                .and_then(|mut a| a.next().ok_or_else(|| "no address".to_string()))
                .and_then(|a| std::net::TcpStream::connect_timeout(&a, Duration::from_secs(5)).map(|_| ()).map_err(|e| e.to_string())),
        };
        match ok {
            Ok(()) => return Ok(()),
            Err(e) => last = e,
        }
        std::thread::sleep(Duration::from_secs(10));
    }
    bail!("bia did not answer at {host} within {} minutes ({last}); the provider's console shows what the server is doing, and a firewall may block port {}", limit.as_secs() / 60, if ca_pem.is_some() { 443 } else { 4791 })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmm() -> FmmInput {
        FmmInput { provider: "openrouter".into(), api_key: "sk-or-v1-x".into(), model: "m".into(), ctx: 1000, ..Default::default() }
    }

    #[test]
    fn config_for_new_agent_and_land() {
        let d = Deploy { host: "198.51.100.7".into(), agent: Agent::New { fmm: fmm(), lang: "ru".into() }, os: None };
        let v = config(&d, None).unwrap();
        assert_eq!(v["pki"]["ips"][0], "198.51.100.7");
        assert_eq!(v["setup"], "");
        assert!(v.get("wipe").is_none());
        let d = Deploy { host: "bia.example.com".into(), ..d };
        let v = config(&d, Some("/dev/sda")).unwrap();
        assert_eq!(v["pki"]["names"][0], "bia.example.com");
        assert_eq!((v["wipe"].as_str(), v["reboot"].as_bool()), (Some("/dev/sda"), Some(true)));
        let d = Deploy { host: "h".into(), agent: Agent::Land { secret: "ABCD".into() }, os: Some("v1.4.3".into()) };
        let v = config(&d, None).unwrap();
        assert_eq!(v["land"]["secret"], "ABCD");
        assert!(v.get("fmm").is_none() && v.get("pki").is_none());
        assert_eq!(v["os"], "v1.4.3");
    }

    #[test]
    fn lines_split_across_chunks() {
        let mut l = Lines::default();
        let mut got = vec![];
        l.feed(b"== one\npart", &mut |s| got.push(s));
        l.feed(b"ial\r\n\nlast\n", &mut |s| got.push(s));
        assert_eq!(got, ["== one", "partial", "last"]);
    }
}
