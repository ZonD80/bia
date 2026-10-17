//! FMM (Frontier Model Mode) configuration: $BIA_FILES/state/fmm.json. The agent reads it after
//! every fmm_* call and at boot (after the network is up) and switches models accordingly.
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

pub fn state_file() -> PathBuf {
    let files = std::env::var("BIA_FILES").unwrap_or_else(|_| "/home/bia/files".into());
    PathBuf::from(files).join("state").join("fmm.json")
}

pub(crate) fn load() -> Option<Value> {
    fs::read(state_file())
        .ok()
        .and_then(|d| serde_json::from_slice(&d).ok())
}

pub(crate) fn save(v: &Value) -> Result<()> {
    let p = state_file();
    if let Some(d) = p.parent() {
        fs::create_dir_all(d)?;
    }
    {
        use std::io::Write;
        let mut f = fs::File::create(&p)?;
        f.write_all(&serde_json::to_vec_pretty(v)?)?;
        f.sync_all()?;
    }
    unsafe { libc::sync() };
    Ok(())
}

fn mask(k: &str) -> String {
    let n = k.chars().count();
    if n > 8 {
        let head: String = k.chars().take(4).collect();
        let tail: String = k.chars().skip(n - 4).collect();
        format!("{head}…{tail}")
    } else {
        "****".into()
    }
}

const PROVIDERS: &[&str] = &[
    "openrouter",
    "openai",
    "anthropic",
    "xai",
    "deepseek",
    "qwen",
];

/// Provider implied by the key's format (same rules as bia-flash `fmm::provider_for_key`).
fn provider_for_key(key: &str) -> Option<&'static str> {
    let k = key.trim();
    if k.starts_with("sk-or-") {
        Some("openrouter")
    } else if k.starts_with("sk-ant-") {
        Some("anthropic")
    } else if k.starts_with("xai-") {
        Some("xai")
    } else if ["sk-proj-", "sk-svcacct-", "sk-admin-"]
        .iter()
        .any(|p| k.starts_with(p))
        || (k.starts_with("sk-") && k.contains("T3BlbkFJ"))
    {
        Some("openai")
    } else {
        None
    }
}

/// DeepSeek and Qwen (DashScope) keys are both `sk-` + 32 hex: ask which API accepts it.
/// Returns the provider and the base_url (set only for a mainland China Qwen key).
fn probe_key(key: &str) -> Result<(&'static str, &'static str)> {
    let hex = key
        .trim()
        .strip_prefix("sk-")
        .is_some_and(|h| h.len() == 32 && h.bytes().all(|b| b.is_ascii_hexdigit()));
    if !hex {
        bail!(
            "cannot tell the provider from this key; pass provider ({})",
            PROVIDERS.join(", ")
        );
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(15)))
        .build()
        .into();
    for (provider, models, base) in [
        ("deepseek", "https://api.deepseek.com/v1/models", ""),
        (
            "qwen",
            "https://dashscope-intl.aliyuncs.com/compatible-mode/v1/models",
            "",
        ),
        (
            "qwen",
            "https://dashscope.aliyuncs.com/compatible-mode/v1/models",
            "https://dashscope.aliyuncs.com/compatible-mode/v1",
        ),
    ] {
        if agent
            .get(models)
            .header("Authorization", &format!("Bearer {}", key.trim()))
            .call()
            .is_ok()
        {
            return Ok((provider, base));
        }
    }
    bail!("neither DeepSeek nor Qwen accepts this key; pass provider")
}

fn default_model(provider: &str) -> &'static str {
    match provider {
        "anthropic" => "claude-opus-4-6",
        "openrouter" => "x-ai/grok-4.6",
        "xai" => "grok-4.6",
        "deepseek" => "deepseek-chat",
        "qwen" => "qwen-max",
        _ => "gpt-5",
    }
}

pub fn configure(
    provider: &str,
    api_key: &str,
    model: &str,
    base_url: Option<&str>,
    ctx: Option<u32>,
) -> Result<String> {
    if api_key.trim().is_empty() {
        bail!("api_key is required");
    }
    let mut provider = provider.trim().to_lowercase();
    if provider == "claude" {
        provider = "anthropic".into();
    }
    let mut base_url = base_url.unwrap_or("").trim().to_string();
    if provider.is_empty() {
        match provider_for_key(api_key) {
            Some(p) => provider = p.into(),
            None => {
                let (p, b) = probe_key(api_key)?;
                provider = p.into();
                if base_url.is_empty() {
                    base_url = b.into();
                }
            }
        }
    } else if !PROVIDERS.contains(&provider.as_str()) {
        bail!("provider must be one of {}", PROVIDERS.join(", "));
    } else if let Some(d) =
        provider_for_key(api_key).filter(|d| *d != provider && base_url.is_empty())
    {
        bail!("this is a {d} key, not {provider}; call again with provider={d} (or leave provider out)");
    }
    let model = match model.trim() {
        "" => default_model(&provider),
        m => m,
    };
    let prev = load();
    let ctx_val = ctx
        .map(|n| n.max(512).min(2_000_000))
        .or_else(|| {
            prev.as_ref()
                .and_then(|c| c["ctx"].as_u64().map(|n| n as u32))
        })
        .unwrap_or(131072);
    let cfg = json!({
        "provider": provider,
        "api_key": api_key.trim(),
        "model": model,
        "base_url": base_url,
        "enabled": true,
        "ctx": ctx_val,
    });
    save(&cfg)?;
    Ok(format!(
        "FMM configured: provider {}, model {}, key {}, context {} tokens. You now test the provider and switch to it if it answers; conversation history continues (streamed). Previous settings are restored if the switch fails. It is enabled again after every boot once the network is up.",
        provider, model, mask(api_key.trim()), ctx_val
    ))
}

pub fn set_enabled(on: bool) -> Result<String> {
    let Some(mut cfg) = load() else {
        bail!("FMM is not configured yet; call fmm_configure first")
    };
    cfg["enabled"] = json!(on);
    save(&cfg)?;
    Ok(if on {
        "FMM enabled: you switch to the configured frontier model; conversation history continues"
            .into()
    } else {
        "FMM disabled: you use the local model; conversation history continues".into()
    })
}

pub fn status() -> String {
    match load() {
        None => "FMM not configured (local model only)".into(),
        Some(c) => format!(
            "FMM {}: provider {}, model {}, key {}, context {} tokens, base_url {}",
            if c["enabled"].as_bool().unwrap_or(false) {
                "enabled"
            } else {
                "disabled"
            },
            c["provider"].as_str().unwrap_or("?"),
            {
                let m = c["model"].as_str().unwrap_or("");
                if m.is_empty() {
                    "(provider default)"
                } else {
                    m
                }
            },
            mask(c["api_key"].as_str().unwrap_or("")),
            {
                let n = c["ctx"].as_u64().unwrap_or(0);
                if n == 0 {
                    131072
                } else {
                    n
                }
            },
            {
                let b = c["base_url"].as_str().unwrap_or("");
                if b.is_empty() {
                    "(default)"
                } else {
                    b
                }
            },
        ),
    }
}

pub fn forget() -> String {
    let _ = fs::remove_file(state_file());
    unsafe { libc::sync() };
    "FMM configuration removed; local model only".into()
}
