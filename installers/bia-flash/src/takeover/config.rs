//! Deployment config for `bia-flash takeover`: any number of partial JSON configs (files
//! or HTTPS URLs; later ones win), then flags. Every field is optional; what is still
//! missing afterwards is detected, defaulted or asked for (see `mod.rs`).
//!
//! Objects merge key by key, everything else (lists included) is replaced. The source of
//! each top-level field is kept for the summary the operator confirms.
use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::time::Duration;

pub const KNOWN: &[&str] = &[
    "version", "disk", "wipe", "os", "fmm", "network", "pki", "setup", "instructions", "lang", "reboot",
    "gate_port", "gate_wait", "land",
];

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub version: Option<u32>,
    #[serde(default)]
    pub disk: Option<String>,
    /// Must name the same disk as `disk` for an unattended run.
    #[serde(default)]
    pub wipe: Option<String>,
    /// `latest`, a release tag, or an https:// directory holding bia-os.json.
    #[serde(default)]
    pub os: Option<String>,
    #[serde(default)]
    pub fmm: Option<FmmConf>,
    /// `"auto"` or `{"interfaces": [...]}`.
    #[serde(default)]
    pub network: Option<Value>,
    #[serde(default)]
    pub pki: Option<PkiConf>,
    /// Free text for bia's model, handed over once at first boot.
    #[serde(default, alias = "instructions")]
    pub setup: Option<String>,
    #[serde(default)]
    pub lang: Option<String>,
    /// Reboot into the takeover without asking at the end.
    #[serde(default)]
    pub reboot: Option<bool>,
    #[serde(default)]
    pub gate_port: Option<u16>,
    /// The takeover waits until root's certificate was fetched (gate page or a copy of
    /// /root/bia-root.p12); this gives up after so many seconds,
    /// before anything was changed (default 0: no limit).
    #[serde(default)]
    pub gate_wait: Option<u64>,
    /// Land mode: `true` (a land secret is made) or `{"secret": "…"}`. bia then waits for
    /// another bia to fly its agent here instead of starting a new one.
    #[serde(default)]
    pub land: Option<Value>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct FmmConf {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default, alias = "key")]
    pub api_key: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub ctx: Option<u32>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PkiConf {
    /// DNS names for the HTTPS server certificate.
    #[serde(default)]
    pub names: Option<Vec<String>>,
    /// Extra addresses for it (a public address behind NAT).
    #[serde(default)]
    pub ips: Option<Vec<String>>,
    /// `"generate"` (a .p12 for the operator) or `{"csr": "<PEM>"}` (operator's own key).
    #[serde(default)]
    pub root: Option<Value>,
    /// Networks the root certificate works from (empty = anywhere).
    #[serde(default)]
    pub root_from: Option<Vec<String>>,
    /// The name other bias know this one by.
    #[serde(default)]
    pub agent_name: Option<String>,
}

/// The merged config and where each top-level field came from.
#[derive(Debug, Clone, Default)]
pub struct Merged {
    pub value: Value,
    pub source: BTreeMap<String, String>,
    pub warnings: Vec<String>,
}

impl Merged {
    pub fn new() -> Self {
        Merged { value: Value::Object(Map::new()), ..Default::default() }
    }

    /// Lay `over` on top; `name` is recorded as the source of its top-level keys.
    pub fn add(&mut self, over: Value, name: &str) -> Result<()> {
        let Value::Object(m) = over else {
            bail!("{name}: a config must be a JSON object");
        };
        for k in m.keys() {
            if !KNOWN.contains(&k.as_str()) {
                self.warnings.push(format!("{name}: unknown field {k:?} ignored"));
            }
            let key = if k == "instructions" { "setup" } else { k.as_str() };
            self.source.insert(key.to_string(), name.to_string());
        }
        merge(&mut self.value, Value::Object(m));
        Ok(())
    }

    pub fn config(&self) -> Result<Config> {
        let mut v = self.value.clone();
        // `instructions` is the older name of `setup`; the later one wins anyway
        if let Some(o) = v.as_object_mut() {
            if let Some(i) = o.remove("instructions") {
                o.entry("setup").or_insert(i);
            }
        }
        serde_json::from_value(v).context("deployment config")
    }

    pub fn source_of(&self, key: &str) -> Option<&str> {
        self.source.get(key).map(String::as_str)
    }
}

/// Deep merge: objects key by key, other values replaced; `null` removes a key.
pub fn merge(base: &mut Value, over: Value) {
    match (base, over) {
        (Value::Object(b), Value::Object(o)) => {
            for (k, v) in o {
                if v.is_null() {
                    b.remove(&k);
                } else if let Some(slot) = b.get_mut(&k) {
                    merge(slot, v);
                } else {
                    b.insert(k, v);
                }
            }
        }
        (slot, v) => *slot = v,
    }
}

/// Read a config from a file or an https:// URL, checking `pin` (sha256 hex) when given.
pub fn load(src: &str, pin: Option<&str>) -> Result<Value> {
    let bytes = if src.starts_with("https://") {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(60)))
            .user_agent("bia-flash")
            .build()
            .into();
        let resp = agent.get(src).call().map_err(|e| anyhow!("download the config: {e}"))?;
        let mut b = Vec::new();
        use std::io::Read;
        resp.into_body()
            .into_reader()
            .take(1 << 20)
            .read_to_end(&mut b)
            .context("download the config")?;
        b
    } else if src.starts_with("http://") {
        bail!("the config holds API keys: give it over https://, not http://");
    } else {
        std::fs::read(src).with_context(|| format!("read {src}"))?
    };
    if let Some(pin) = pin.map(|p| p.trim().to_ascii_lowercase()).filter(|p| !p.is_empty()) {
        let got: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
        if got != pin {
            bail!("the config has sha256 {got}, not the pinned {pin}");
        }
    }
    serde_json::from_slice(&bytes).map_err(|e| anyhow!("the config is not valid JSON ({e})"))
}

/// A short, log-safe name for a config source (a URL loses its query and path).
pub fn source_name(src: &str) -> String {
    if let Some(rest) = src.strip_prefix("https://") {
        format!("config https://{}/…", rest.split('/').next().unwrap_or(""))
    } else {
        format!("config {src}")
    }
}

/// The merged config with secrets masked (for `--dump-config` and the summary).
pub fn masked(v: &Value, show_secrets: bool) -> Value {
    let mut v = v.clone();
    if show_secrets {
        return v;
    }
    if let Some(f) = v.get_mut("fmm").and_then(Value::as_object_mut) {
        for k in ["api_key", "key"] {
            if let Some(Value::String(s)) = f.get_mut(k) {
                *s = mask(s);
            }
        }
    }
    if let Some(Value::String(s)) = v.pointer_mut("/pki/root/csr") {
        *s = format!("<CSR, {} bytes>", s.len());
    }
    v
}

pub fn mask(s: &str) -> String {
    let n = s.chars().count();
    if n <= 10 {
        "****".into()
    } else {
        format!("{}…{}", s.chars().take(6).collect::<String>(), s.chars().skip(n - 4).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn partial_configs_merge_in_order() {
        let mut m = Merged::new();
        m.add(json!({"fmm": {"key": "sk-or-v1-abcdefabcdef"}}), "config https://x/…").unwrap();
        m.add(json!({"fmm": {"model": "x-ai/grok-4"}, "pki": {"ips": ["203.0.113.7"]}, "bogus": 1}), "config local.json").unwrap();
        m.add(json!({"pki": {"names": ["bia.example.com"]}, "instructions": "connect to 10.0.0.5"}), "flags").unwrap();
        let c = m.config().unwrap();
        let f = c.fmm.unwrap();
        assert_eq!(f.api_key.as_deref(), Some("sk-or-v1-abcdefabcdef"), "key kept from the first config");
        assert_eq!(f.model.as_deref(), Some("x-ai/grok-4"));
        let p = c.pki.unwrap();
        assert_eq!(p.ips.unwrap(), vec!["203.0.113.7".to_string()], "objects merge");
        assert_eq!(p.names.unwrap(), vec!["bia.example.com".to_string()]);
        assert_eq!(c.setup.as_deref(), Some("connect to 10.0.0.5"));
        assert!(c.disk.is_none() && c.network.is_none(), "missing stays missing");
        assert_eq!(m.source_of("fmm"), Some("config local.json"));
        assert_eq!(m.source_of("setup"), Some("flags"));
        assert!(m.warnings.iter().any(|w| w.contains("bogus")));
        let shown = masked(&m.value, false).to_string();
        assert!(!shown.contains("abcdefabcdef") && shown.contains("sk-or-"), "{shown}");
    }

    #[test]
    fn lists_replace_and_null_removes() {
        let mut v = json!({"pki": {"ips": ["1.1.1.1", "2.2.2.2"], "names": ["a"]}});
        merge(&mut v, json!({"pki": {"ips": ["3.3.3.3"], "names": null}}));
        assert_eq!(v, json!({"pki": {"ips": ["3.3.3.3"]}}));
    }

    #[test]
    fn pinned_and_insecure_sources() {
        let p = std::env::temp_dir().join(format!("bia-cfg-{}.json", std::process::id()));
        std::fs::write(&p, br#"{"lang": "ru"}"#).unwrap();
        let s = p.to_string_lossy().to_string();
        assert_eq!(load(&s, None).unwrap()["lang"], "ru");
        assert!(load(&s, Some("00")).is_err());
        let sha: String = Sha256::digest(br#"{"lang": "ru"}"#).iter().map(|b| format!("{b:02x}")).collect();
        assert!(load(&s, Some(&sha)).is_ok());
        assert!(load("http://example.com/c.json", None).is_err());
        assert_eq!(source_name("https://cfg.example.com/a/b?t=secret"), "config https://cfg.example.com/…");
        let _ = std::fs::remove_file(p);
    }
}
