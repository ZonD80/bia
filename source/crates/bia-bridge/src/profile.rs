//! Profiles: one per agent and bia, `<home>/<agent>/<bia>/` under the bridge's home folder
//! (`BIA_BRIDGE_HOME`, else the OS config folder). The agent is the certificate's identity:
//! two agents on one machine are two people to a bia, and never share a profile. Each holds
//! the key (made here, never sent), the certificate, the pinned CA, the request, the inbox
//! and the daemon's port.
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
    /// The bia's name for this agent (`--bia`).
    pub name: String,
    /// `https://host[:port]` of the bia.
    pub url: String,
    /// The agent: its identity there (the certificate's CN).
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
    /// Still in the old `<home>/<name>/` folder, on its way to the agent's own.
    #[serde(skip)]
    pub legacy: bool,
}

pub fn valid_name(n: &str) -> bool {
    !n.is_empty() && n.len() <= 64 && n.chars().all(|c| c.is_ascii_alphanumeric() || "-_.@".contains(c)) && !n.starts_with('.')
}

/// Before 2.0.6 a profile was `<home>/<name>/`, for whichever agent ran the command. Each moves
/// to the agent its certificate names; any other agent then has to ask for its own.
pub fn migrate() {
    for e in fs::read_dir(home()).into_iter().flatten().flatten() {
        let (old, name) = (e.path(), e.file_name().to_string_lossy().to_string());
        let Ok(b) = fs::read(old.join("profile.json")) else { continue };
        let Ok(mut p) = serde_json::from_slice::<Profile>(&b) else { continue };
        if !valid_name(&p.identity) || !valid_name(&name) {
            continue;
        }
        p.name = name.clone();
        p.legacy = true;
        let _ = crate::ipc::stop(&p); // its daemon works in the old folder
        // through a side name: the agent can be called the same as the old profile
        let side = home().join(format!(".moving-{name}"));
        let target = Profile::dir_of(&p.identity, &name);
        if target.exists() || fs::rename(&old, &side).is_err() {
            continue;
        }
        let moved = target.parent().is_some_and(|t| fs::create_dir_all(t).is_ok()) && fs::rename(&side, &target).is_ok();
        if !moved {
            let _ = fs::rename(&side, &old);
        }
    }
}

impl Profile {
    pub fn dir_of(agent: &str, name: &str) -> PathBuf {
        home().join(agent).join(name)
    }
    pub fn dir(&self) -> PathBuf {
        if self.legacy {
            return home().join(&self.name);
        }
        Self::dir_of(&self.identity, &self.name)
    }
    pub fn path(&self, f: &str) -> PathBuf {
        self.dir().join(f)
    }

    pub fn load(agent: &str, name: &str) -> Result<Profile> {
        if !valid_name(name) || !valid_name(agent) {
            bail!("bad agent or bia name {agent:?} {name:?}");
        }
        let f = Self::dir_of(agent, name).join("profile.json");
        let b = fs::read(&f).with_context(|| format!("{agent} has no bia called {name} (bia-bridge connect <url> --identity {agent} --as {name} asks for its own certificate)"))?;
        let p: Profile = serde_json::from_slice(&b)?;
        if p.identity != agent {
            bail!("{} belongs to {}, not {agent}", f.display(), p.identity);
        }
        Ok(p)
    }

    pub fn save(&self) -> Result<()> {
        fs::create_dir_all(self.dir())?;
        let tmp = self.path("profile.json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        fs::rename(&tmp, self.path("profile.json"))?;
        Ok(())
    }

    /// One agent's profiles.
    pub fn of_agent(agent: &str) -> Vec<Profile> {
        let mut out: Vec<Profile> = fs::read_dir(home().join(agent))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| Profile::load(agent, &e.file_name().to_string_lossy()).ok())
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// Every agent's profiles on this machine.
    pub fn list() -> Vec<Profile> {
        let mut out: Vec<Profile> = fs::read_dir(home())
            .into_iter()
            .flatten()
            .flatten()
            .flat_map(|e| Profile::of_agent(&e.file_name().to_string_lossy()))
            .collect();
        out.sort_by(|a, b| (&a.identity, &a.name).cmp(&(&b.identity, &b.name)));
        out
    }

    /// `agent`'s profile `--bia <name>`, else `BIA_BRIDGE_PROFILE`, else the only one it has.
    /// Never another agent's: to a bia that would be somebody else talking.
    pub fn pick(agent: &str, name: Option<&str>) -> Result<Profile> {
        if let Some(n) = name.map(str::to_string).or_else(|| std::env::var("BIA_BRIDGE_PROFILE").ok()).filter(|n| !n.is_empty()) {
            return Profile::load(agent, &n);
        }
        let all = Profile::of_agent(agent);
        match all.len() {
            0 => Err(anyhow!("{agent} has no bia yet: bia-bridge connect https://<bia address> --identity {agent}. Each agent asks for its own certificate; another agent's profile is not {agent}'s to use.")),
            1 => Ok(all.into_iter().next().unwrap()),
            _ => Err(anyhow!(
                "{agent} has several bias ({}); choose one with --bia <name>",
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
    use serde_json::json;

    #[test]
    fn names() {
        assert!(valid_name("fred") && valid_name("claude-code@mac") && !valid_name("../x") && !valid_name(".x") && !valid_name(""));
    }

    #[test]
    fn agents_keep_to_their_own_profiles() {
        let h = std::env::temp_dir().join(format!("bia-bridge-prof-{}", std::process::id()));
        let _ = fs::remove_dir_all(&h);
        std::env::set_var("BIA_BRIDGE_HOME", &h);
        // the old layout, once with an agent named like the profile
        for (name, who) in [("fred", "claude-code"), ("sam", "sam")] {
            fs::create_dir_all(h.join(name)).unwrap();
            let old = json!({"name": name, "url": "https://x", "identity": who, "ca_sha256": "", "state": "active"});
            fs::write(h.join(name).join("profile.json"), old.to_string()).unwrap();
            fs::write(h.join(name).join("key.pem"), who).unwrap();
        }
        migrate();
        assert!(!h.join("fred").exists());
        assert_eq!(fs::read_to_string(h.join("claude-code/fred/key.pem")).unwrap(), "claude-code");
        assert_eq!(fs::read_to_string(h.join("sam/sam/key.pem")).unwrap(), "sam");
        migrate();
        assert_eq!(Profile::list().len(), 2, "running it again changes nothing");

        assert_eq!(Profile::pick("claude-code", None).unwrap().name, "fred");
        // the only profile on the machine is still somebody else's
        assert!(Profile::pick("grok", None).unwrap_err().to_string().contains("grok has no bia"));
        assert!(Profile::pick("grok", Some("fred")).is_err());
        assert!(Profile::load("../claude-code", "fred").is_err());

        // two agents, one bia
        let grok = Profile { name: "fred".into(), identity: "grok".into(), url: "https://x".into(), ..Default::default() };
        grok.save().unwrap();
        assert_eq!(grok.dir(), h.join("grok/fred"));
        assert_eq!(Profile::pick("grok", None).unwrap().identity, "grok");
        assert_eq!(Profile::pick("claude-code", Some("fred")).unwrap().identity, "claude-code");

        // a folder holding somebody else's profile is refused
        fs::create_dir_all(h.join("mallory/fred")).unwrap();
        fs::copy(h.join("grok/fred/profile.json"), h.join("mallory/fred/profile.json")).unwrap();
        assert!(Profile::load("mallory", "fred").unwrap_err().to_string().contains("belongs to grok"));
        let _ = fs::remove_dir_all(&h);
    }
}
