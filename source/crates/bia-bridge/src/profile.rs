//! Profiles: one per bia this machine talks to, under the bridge's home folder
//! (`BIA_BRIDGE_HOME`, else the OS config folder). Each holds the key (made here, never
//! sent), the certificate, the pinned CA, the request, the inbox and the daemon's port.
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

pub fn home() -> PathBuf {
    if let Some(h) = std::env::var_os("BIA_BRIDGE_HOME").filter(|h| !h.is_empty()) {
        return PathBuf::from(h);
    }
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    if cfg!(windows) {
        if let Some(a) = env("APPDATA") {
            return a.join("bia-bridge");
        }
    } else if cfg!(target_os = "macos") {
        if let Some(h) = env("HOME") {
            return h.join("Library").join("Application Support").join("bia-bridge");
        }
    } else if let Some(x) = env("XDG_CONFIG_HOME") {
        return x.join("bia-bridge");
    } else if let Some(h) = env("HOME") {
        return h.join(".config").join("bia-bridge");
    }
    PathBuf::from(".bia-bridge")
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Profile {
    /// Profile name (`--bia`).
    pub name: String,
    /// `https://host[:port]` of the bia.
    pub url: String,
    /// This client's identity there (the certificate's CN).
    pub identity: String,
    /// The bia's CA fingerprint (sha256 hex), pinned at the first contact.
    pub ca_sha256: String,
    /// Certificate request id (UUID).
    #[serde(default)]
    pub request: String,
    /// `pending`, `active`, `denied` or `expired`.
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub note: String,
    /// The owner's message with the approval or the reason of a denial.
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub created: u64,
}

pub fn valid_name(n: &str) -> bool {
    !n.is_empty() && n.len() <= 64 && n.chars().all(|c| c.is_ascii_alphanumeric() || "-_.@".contains(c)) && !n.starts_with('.')
}

impl Profile {
    pub fn dir_of(name: &str) -> PathBuf {
        home().join(name)
    }
    pub fn dir(&self) -> PathBuf {
        Self::dir_of(&self.name)
    }
    pub fn path(&self, f: &str) -> PathBuf {
        self.dir().join(f)
    }

    pub fn load(name: &str) -> Result<Profile> {
        if !valid_name(name) {
            bail!("bad profile name {name:?}");
        }
        let p = Self::dir_of(name).join("profile.json");
        let b = fs::read(&p).with_context(|| format!("no profile {name} (bia-bridge connect <url> makes one)"))?;
        Ok(serde_json::from_slice(&b)?)
    }

    pub fn save(&self) -> Result<()> {
        fs::create_dir_all(self.dir())?;
        let tmp = self.path("profile.json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        fs::rename(&tmp, self.path("profile.json"))?;
        Ok(())
    }

    pub fn list() -> Vec<Profile> {
        let mut out: Vec<Profile> = fs::read_dir(home())
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| Profile::load(&e.file_name().to_string_lossy()).ok())
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// `--bia <name>`, else `BIA_BRIDGE_PROFILE`, else the only profile there is.
    pub fn pick(name: Option<&str>) -> Result<Profile> {
        if let Some(n) = name.map(str::to_string).or_else(|| std::env::var("BIA_BRIDGE_PROFILE").ok()).filter(|n| !n.is_empty()) {
            return Profile::load(&n);
        }
        let all = Profile::list();
        match all.len() {
            0 => Err(anyhow!("no bia yet: bia-bridge connect https://<bia address> --identity <your name>")),
            1 => Ok(all.into_iter().next().unwrap()),
            _ => Err(anyhow!(
                "several bias ({}); choose one with --bia <name>",
                all.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ")
            )),
        }
    }

    pub fn read(&self, f: &str) -> Result<String> {
        fs::read_to_string(self.path(f)).with_context(|| format!("{}: {f} missing", self.name))
    }

    /// (CA, certificate, key) PEM for talking to the bia.
    pub fn creds(&self) -> Result<(String, String, String)> {
        if self.state != "active" {
            bail!("{} is {} (bia-bridge status)", self.name, if self.state.is_empty() { "not set up" } else { &self.state });
        }
        Ok((self.read("ca.pem")?, self.read("cert.pem")?, self.read("key.pem")?))
    }

    pub fn days_left(&self) -> Option<i64> {
        let pem = self.read("cert.pem").ok()?;
        let ders = bia_pki::pem_to_cert_ders(&pem).ok()?;
        bia_pki::client_id_from_certs(Some(&ders[..1])).map(|c| c.days_left())
    }
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn sha256_hex(b: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, b).as_ref().iter().map(|x| format!("{x:02x}")).collect()
}

pub fn random_hex(n: usize) -> String {
    use ring::rand::SecureRandom;
    let mut b = vec![0u8; n];
    let _ = ring::rand::SystemRandom::new().fill(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert!(valid_name("fred") && valid_name("claude-code@mac") && !valid_name("../x") && !valid_name(".x") && !valid_name(""));
    }
}
