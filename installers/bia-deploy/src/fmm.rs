use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Same relative path the agent seeds from (`remote::ESP_FMM_REL`).
pub const ESP_FMM_REL: &str = "EFI/bia/fmm.json";

/// Host-local lookup order for test / flash keys (never commit these).
pub const LOCAL_FMM_CANDIDATES: &[&str] = &["fmm.json", "source/flash/files/state/fmm.json"];

/// Where a non-technical user gets a key, for the flasher's help (GUI panel, `bia-deploy keys`).
pub struct KeyGuide {
    pub provider: &'static str,
    pub name: &'static str,
    /// One plain sentence: what it is and anything surprising.
    pub about: &'static str,
    /// Page where keys are created (sign-up/sign-in comes first if needed).
    pub key_url: &'static str,
    /// Page to add credit, when it differs from the key page.
    pub pay_url: Option<&'static str>,
}

/// The same three steps work everywhere.
pub const KEY_STEPS: &[&str] = &[
    "Open the provider's page and sign up (or sign in).",
    "Add a little credit with a card. A few dollars lasts a long time.",
    "Press \"Create key\", copy the key, and paste it into bia deploy.",
];

pub const KEY_GUIDES: &[KeyGuide] = &[
    KeyGuide {
        provider: "openrouter",
        name: "OpenRouter (easiest)",
        about: "One key for almost every AI: Grok, Claude, GPT, DeepSeek, Qwen and more. Best place to start.",
        key_url: "https://openrouter.ai/settings/keys",
        pay_url: Some("https://openrouter.ai/settings/credits"),
    },
    KeyGuide {
        provider: "openai",
        name: "OpenAI (GPT)",
        about: "The makers of ChatGPT. Have ChatGPT Plus or Pro? Skip the key and press Continue with ChatGPT instead. An API key is billed separately.",
        key_url: "https://platform.openai.com/api-keys",
        pay_url: Some("https://platform.openai.com/settings/organization/billing/overview"),
    },
    KeyGuide {
        provider: "anthropic",
        name: "Anthropic (Claude)",
        about: "The makers of Claude. A Claude.ai subscription does not include this; API credit is bought separately.",
        key_url: "https://platform.claude.com/settings/keys",
        pay_url: Some("https://platform.claude.com/settings/billing"),
    },
    KeyGuide {
        provider: "xai",
        name: "xAI (Grok)",
        about: "The makers of Grok. Add credit in the console, then create a key.",
        key_url: "https://console.x.ai",
        pay_url: None,
    },
    KeyGuide {
        provider: "deepseek",
        name: "DeepSeek",
        about: "Low prices. Top up first, then create a key.",
        key_url: "https://platform.deepseek.com/api_keys",
        pay_url: Some("https://platform.deepseek.com/top_up"),
    },
    KeyGuide {
        provider: "qwen",
        name: "Qwen (Alibaba Cloud Model Studio)",
        about: "Alibaba's Qwen models. Create the key in Model Studio; new accounts may get free trial tokens.",
        key_url: "https://modelstudio.console.alibabacloud.com/?tab=playground#/api-key",
        pay_url: None,
    },
];

pub const PROVIDERS: &[&str] = &["openrouter", "openai", "anthropic", "xai", "deepseek", "qwen"];

/// Provider implied by an API key's format, when the format alone settles it:
/// OpenRouter `sk-or-`, Anthropic `sk-ant-`, xAI `xai-`, OpenAI `sk-proj-` /
/// `sk-svcacct-` / `sk-admin-` or the `T3BlbkFJ` marker every OpenAI key carries.
/// DeepSeek and Qwen (DashScope) keys are both `sk-` + 32 hex: see [`probe_key`].
pub fn provider_for_key(key: &str) -> Option<&'static str> {
    let k = key.trim();
    if k.starts_with("sk-or-") {
        Some("openrouter")
    } else if k.starts_with("sk-ant-") {
        Some("anthropic")
    } else if k.starts_with("xai-") {
        Some("xai")
    } else if ["sk-proj-", "sk-svcacct-", "sk-admin-"].iter().any(|p| k.starts_with(p))
        || (k.starts_with("sk-") && k.contains("T3BlbkFJ"))
    {
        Some("openai")
    } else {
        None
    }
}

/// `sk-` + 32 hex: a DeepSeek or a Qwen (DashScope) key; only the APIs can tell.
pub fn is_deepseek_or_qwen_key(key: &str) -> bool {
    key.trim()
        .strip_prefix("sk-")
        .is_some_and(|h| h.len() == 32 && h.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Qwen keys from the mainland China region only work against this address.
const QWEN_CN: &str = "https://dashscope.aliyuncs.com/compatible-mode/v1";

/// Ask DeepSeek, then Qwen international, then Qwen mainland China, which one accepts
/// `key`. Returns the provider, its base_url (set only for a China key) and its models.
pub fn probe_key(key: &str) -> Result<(&'static str, String, Vec<CloudModel>)> {
    for (provider, base) in [("deepseek", ""), ("qwen", ""), ("qwen", QWEN_CN)] {
        if let Ok(models) = list_models(provider, key, base) {
            return Ok((provider, base.to_string(), models));
        }
    }
    bail!("neither DeepSeek nor Qwen (DashScope) accepts this key; set provider")
}

/// API root for `provider` (the agent's defaults in `remote::Remote::from_config`).
pub fn api_base(provider: &str) -> &'static str {
    match provider {
        "openrouter" => "https://openrouter.ai/api/v1",
        "xai" => "https://api.x.ai/v1",
        "anthropic" | "claude" => "https://api.anthropic.com",
        "deepseek" => "https://api.deepseek.com/v1",
        "chatgpt" => crate::chatgpt::RESOURCE,
        "qwen" => "https://dashscope-intl.aliyuncs.com/compatible-mode/v1",
        _ => "https://api.openai.com/v1",
    }
}

#[derive(Debug, Clone)]
pub struct CloudModel {
    pub id: String,
    /// Display name when the provider gives one, else the id.
    pub name: String,
    /// Context window in tokens when the provider reports it (OpenRouter), else 0.
    pub ctx: u32,
}

/// Models `key` can use at `provider` (its `/models` list), sorted by id.
pub fn list_models(provider: &str, key: &str, base_url: &str) -> Result<Vec<CloudModel>> {
    #[derive(Deserialize)]
    struct Item {
        id: String,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        display_name: Option<String>,
        #[serde(default)]
        context_length: Option<u64>,
    }
    #[derive(Deserialize)]
    struct List {
        data: Vec<Item>,
    }
    let base = match base_url.trim().trim_end_matches('/') {
        "" => api_base(provider),
        b => b,
    };
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(20)))
        .user_agent("bia-deploy")
        .build()
        .into();
    let req = if matches!(provider, "anthropic" | "claude") {
        agent
            .get(&format!("{base}/v1/models?limit=1000"))
            .header("x-api-key", key.trim())
            .header("anthropic-version", "2023-06-01")
    } else {
        agent
            .get(&format!("{base}/models"))
            .header("Authorization", &format!("Bearer {}", key.trim()))
    };
    let resp = req.call().map_err(|e| match e {
        ureq::Error::StatusCode(401 | 403) => anyhow::anyhow!("{provider} rejected this key"),
        e => anyhow::anyhow!("list {provider} models: {e}"),
    })?;
    let list: List = serde_json::from_reader(resp.into_body().into_reader())
        .with_context(|| format!("parse {provider} model list"))?;
    let mut out: Vec<CloudModel> = list
        .data
        .into_iter()
        .map(|i| CloudModel {
            name: i.display_name.or(i.name).unwrap_or_else(|| i.id.clone()),
            ctx: i.context_length.map_or(0, |n| n.min(u32::MAX as u64) as u32),
            id: i.id,
        })
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

/// Models whose id or name contains every word of `query` (case-insensitive).
pub fn filter_models<'a>(models: &'a [CloudModel], query: &str) -> Vec<&'a CloudModel> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    models
        .iter()
        .filter(|m| {
            let hay = format!("{} {}", m.id, m.name).to_lowercase();
            words.iter().all(|w| hay.contains(w))
        })
        .collect()
}

/// Same defaults as the agent (`remote::default_model`).
pub fn default_model(provider: &str) -> &'static str {
    match provider {
        "anthropic" | "claude" => "claude-opus-4-6",
        "openrouter" => "x-ai/grok-4.6",
        "xai" => "grok-4.6",
        "deepseek" => "deepseek-chat",
        "qwen" => "qwen-max",
        _ => "gpt-5",
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FmmInput {
    /// Empty: taken from the key's prefix.
    #[serde(default)]
    pub provider: String,
    pub api_key: String,
    /// Empty: the provider's default model.
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default = "default_ctx")]
    pub ctx: u32,
    /// Accepted from agent-shaped files; always written as true onto the stick.
    #[serde(default)]
    pub enabled: Option<bool>,
    /// Provider `chatgpt`: tokens from "Continue with ChatGPT" (`api_key` mirrors the access token).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chatgpt: Option<crate::chatgpt::ChatGptAuth>,
}

fn default_ctx() -> u32 {
    131_072
}

impl FmmInput {
    pub fn load(path: &Path) -> Result<FmmInput> {
        let data = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
        let c: FmmInput =
            serde_json::from_slice(&data).with_context(|| format!("parse {}", path.display()))?;
        c.resolve()
    }

    /// Validate and normalize, asking the APIs when only they can name the provider.
    pub fn resolve(mut self) -> Result<FmmInput> {
        self.validate()?;
        if self.provider.trim().is_empty() && provider_for_key(&self.api_key).is_none() {
            let (provider, base, _) = probe_key(&self.api_key)?;
            self.provider = provider.into();
            if self.base_url.trim().is_empty() {
                self.base_url = base;
            }
        }
        Ok(self.normalize())
    }

    /// FMM settings for a ChatGPT sign-in: the user's plan pays, bia rotates the tokens.
    pub fn from_chatgpt(auth: crate::chatgpt::ChatGptAuth, model: &str, ctx: u32) -> FmmInput {
        FmmInput {
            provider: "chatgpt".into(),
            api_key: auth.access_token.clone(),
            model: model.trim().to_string(),
            base_url: String::new(),
            ctx,
            enabled: Some(true),
            chatgpt: Some(auth),
        }
    }

    /// For a ChatGPT sign-in, renew the access token if it is about to expire. Call once
    /// before testing and writing, and keep the result: the refresh token rotates.
    pub fn refresh_chatgpt(&mut self) -> Result<()> {
        if let Some(a) = self.chatgpt.as_mut() {
            crate::chatgpt::ensure_fresh(a)
                .context("ChatGPT sign-in expired; press Continue with ChatGPT again")?;
            self.api_key = a.access_token.clone();
        }
        Ok(())
    }

    /// First existing path among `explicit`, then [`LOCAL_FMM_CANDIDATES`].
    pub fn find_local(explicit: Option<&Path>) -> Option<std::path::PathBuf> {
        if let Some(p) = explicit {
            if p.is_file() {
                return Some(p.to_path_buf());
            }
            return None;
        }
        for rel in LOCAL_FMM_CANDIDATES {
            let p = Path::new(rel);
            if p.is_file() {
                return Some(p.to_path_buf());
            }
        }
        None
    }

    pub fn validate(&self) -> Result<()> {
        if self.provider.trim() == "chatgpt" {
            match &self.chatgpt {
                Some(a) if !a.refresh_token.is_empty() && !a.client_id.is_empty() => {}
                _ => bail!("ChatGPT is not signed in; press Continue with ChatGPT"),
            }
            if self.model.trim().is_empty() {
                bail!("pick a ChatGPT model");
            }
            return Ok(());
        }
        if self.api_key.trim().is_empty() {
            bail!("api_key is required");
        }
        let mut p = self.provider.trim().to_lowercase();
        if p == "claude" {
            p = "anthropic".into();
        }
        let detected = provider_for_key(&self.api_key);
        if p.is_empty() {
            if detected.is_none() && !is_deepseek_or_qwen_key(&self.api_key) {
                bail!("cannot tell the provider from this key; set provider ({})", PROVIDERS.join(", "));
            }
            return Ok(());
        }
        if !PROVIDERS.contains(&p.as_str()) {
            bail!("provider must be one of {}", PROVIDERS.join(", "));
        }
        // A custom base_url may be a proxy that takes any key.
        if let Some(d) = detected.filter(|d| *d != p && self.base_url.trim().is_empty()) {
            bail!("this looks like a {d} key, but provider is {p}");
        }
        Ok(())
    }

    pub fn normalize(mut self) -> Self {
        self.provider = self.provider.trim().to_lowercase();
        if self.provider == "claude" {
            self.provider = "anthropic".into();
        }
        self.api_key = self.api_key.trim().to_string();
        if self.provider.is_empty() {
            self.provider = provider_for_key(&self.api_key).unwrap_or_default().into();
        }
        self.model = self.model.trim().to_string();
        if self.model.is_empty() && !self.provider.is_empty() {
            self.model = default_model(&self.provider).into();
        }
        self.base_url = self.base_url.trim().to_string();
        if self.ctx == 0 {
            self.ctx = default_ctx();
        }
        self.ctx = self.ctx.max(512).min(2_000_000);
        self
    }
}

/// Pretty JSON matching agent `FmmConfig` (`enabled: true`).
pub fn fmm_config_json(input: &FmmInput) -> Result<Vec<u8>> {
    let c = input.clone().resolve()?;
    let mut v = serde_json::json!({
        "provider": c.provider,
        "api_key": c.api_key,
        "model": c.model,
        "base_url": c.base_url,
        "enabled": true,
        "ctx": c.ctx,
    });
    if let Some(a) = &c.chatgpt {
        v["chatgpt"] = serde_json::to_value(a)?;
    }
    Ok(serde_json::to_vec_pretty(&v)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_enabled_true() {
        let j = fmm_config_json(&FmmInput {
            provider: "OpenRouter".into(),
            api_key: "sk-or-test".into(),
            model: "x-ai/grok-4.6".into(),
            base_url: String::new(),
            ctx: 0,
            enabled: None,
            chatgpt: None,
        })
        .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&j).unwrap();
        assert_eq!(v["provider"], "openrouter");
        assert_eq!(v["enabled"], true);
        assert_eq!(v["ctx"], 131072);
    }

    #[test]
    fn provider_from_key() {
        let input = |provider: &str, key: &str| FmmInput {
            provider: provider.into(),
            api_key: key.into(),
            model: String::new(),
            base_url: String::new(),
            ctx: 0,
            enabled: None,
            chatgpt: None,
        };
        assert_eq!(provider_for_key("sk-or-v1-abc"), Some("openrouter"));
        assert_eq!(provider_for_key(" sk-ant-api03-abc"), Some("anthropic"));
        assert_eq!(provider_for_key("xai-abc"), Some("xai"));
        assert_eq!(provider_for_key("sk-proj-abc"), Some("openai"));
        // Built at runtime: a literal sk-…T3BlbkFJ… trips GitHub secret scanning.
        let legacy_openai = format!("sk-abc{}{}xyz", "T3Blbk", "FJ");
        assert_eq!(provider_for_key(&legacy_openai), Some("openai"));
        assert_eq!(provider_for_key("abc"), None);
        let ds = "sk-0123456789abcdef0123456789abcdef";
        assert_eq!(provider_for_key(ds), None);
        assert!(is_deepseek_or_qwen_key(ds));
        assert!(input("", ds).validate().is_ok());
        assert!(input("qwen", ds).validate().is_ok());
        let v: serde_json::Value =
            serde_json::from_slice(&fmm_config_json(&input("", "sk-ant-api03-x")).unwrap()).unwrap();
        assert_eq!(v["provider"], "anthropic");
        assert_eq!(v["model"], "claude-opus-4-6");
        assert!(input("openrouter", "sk-ant-api03-x").validate().is_err());
        assert!(input("", "abc").validate().is_err());
        assert!(input("openai", "abc").validate().is_ok());
    }
}
