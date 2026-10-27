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

/// Provider implied by the key's format (same rules as bia-deploy `fmm::provider_for_key`).
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

/// Model the ChatGPT plan uses when none is named (same default as fmm_chatgpt).
const CHATGPT_DEFAULT_MODEL: &str = "gpt-6.1-sol";

/// The ChatGPT sign-in kept in fmm.json, with the model it last used. It stays there while
/// another provider is active, so switching back needs no new browser sign-in.
fn saved_chatgpt(prev: Option<&Value>) -> Option<Value> {
    let prev = prev?;
    let mut auth = prev.get("chatgpt").filter(|a| a.is_object())?.clone();
    if prev["provider"] == "chatgpt" {
        if let Some(m) = prev["model"].as_str().filter(|m| !m.is_empty()) {
            auth["model"] = json!(m);
        }
    }
    Some(auth)
}

/// Switch back to the saved ChatGPT sign-in (provider=chatgpt, no key).
fn configure_chatgpt(prev: Option<&Value>, model: &str, ctx: Option<u32>) -> Result<String> {
    let Some(auth) = saved_chatgpt(prev) else {
        bail!("no saved ChatGPT sign-in; use fmm_chatgpt to sign in");
    };
    let access = auth["access_token"].as_str().unwrap_or("").to_string();
    if access.is_empty() || auth["refresh_token"].as_str().unwrap_or("").is_empty() {
        bail!("the saved ChatGPT sign-in is incomplete; use fmm_chatgpt to sign in again");
    }
    let model = match model.trim() {
        "" => auth["model"]
            .as_str()
            .filter(|m| !m.is_empty())
            .unwrap_or(CHATGPT_DEFAULT_MODEL)
            .to_string(),
        m => m.to_string(),
    };
    let same = prev.is_some_and(|p| p["provider"] == "chatgpt" && p["model"] == model.as_str());
    let ctx_val = ctx
        .map(|n| n.clamp(512, 2_000_000))
        .or_else(|| {
            same.then(|| prev.and_then(|c| c["ctx"].as_u64()).map(|n| n as u32))
                .flatten()
        })
        .unwrap_or(131072);
    save(&json!({
        "provider": "chatgpt",
        "api_key": access,
        "model": model,
        "base_url": "",
        "enabled": true,
        "ctx": ctx_val,
        "chatgpt": auth,
    }))?;
    Ok(format!(
        "FMM configured: the saved ChatGPT sign-in, model {model}, context {ctx_val} tokens. You now test it with this conversation and switch to it if it answers; the previous settings stay if it does not."
    ))
}

pub fn configure(
    provider: &str,
    api_key: &str,
    model: &str,
    base_url: Option<&str>,
    ctx: Option<u32>,
) -> Result<String> {
    let mut provider = provider.trim().to_lowercase();
    if provider == "claude" {
        provider = "anthropic".into();
    }
    let prev = load();
    if provider == "chatgpt" && api_key.trim().is_empty() {
        return configure_chatgpt(prev.as_ref(), model, ctx);
    }
    if api_key.trim().is_empty() {
        bail!("api_key is required");
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
    // Keep the context only for the same provider and model: a window sized for another
    // model (e.g. a ChatGPT plan's) can exceed this one's, and every real request then fails.
    let same = prev
        .as_ref()
        .is_some_and(|p| p["provider"] == provider.as_str() && p["model"] == model);
    let ctx_val = ctx
        .map(|n| n.clamp(512, 2_000_000))
        .or_else(|| {
            same.then(|| {
                prev.as_ref()
                    .and_then(|c| c["ctx"].as_u64())
                    .map(|n| n as u32)
            })
            .flatten()
        })
        .unwrap_or(131072);
    let mut cfg = json!({
        "provider": provider,
        "api_key": api_key.trim(),
        "model": model,
        "base_url": base_url,
        "enabled": true,
        "ctx": ctx_val,
    });
    if let Some(auth) = saved_chatgpt(prev.as_ref()) {
        cfg["chatgpt"] = auth;
    }
    save(&cfg)?;
    Ok(format!(
        "FMM configured: provider {}, model {}, key {}, context {} tokens. You now test it with this conversation and switch to it if it answers; the previous settings stay if it does not, and come back if its first real reply fails. It is enabled again after every boot once the network is up.",
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
        Some(c) => {
            format!(
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
            ) + if c["provider"] != "chatgpt" && c["chatgpt"].is_object() {
                "; a ChatGPT sign-in is saved (fmm_configure provider=chatgpt switches back to it)"
            } else {
                ""
            }
        }
    }
}

/// Swap in the settings from before the last switch (the agent saves them as fmm.prev.json);
/// the current ones become the previous ones, so calling it again swaps back.
pub fn back() -> Result<String> {
    let cur_path = state_file();
    let prev_path = cur_path.with_file_name("fmm.prev.json");
    let Ok(prev) = fs::read(&prev_path) else {
        bail!("no previous FMM settings saved")
    };
    let prev_v: Value = serde_json::from_slice(&prev)?;
    if let Ok(cur) = fs::read(&cur_path) {
        fs::write(&prev_path, cur)?;
    }
    save(&prev_v)?;
    Ok(format!(
        "FMM settings from before the last switch are back: provider {}, model {}. You now test them with this conversation and switch if they answer.",
        prev_v["provider"].as_str().unwrap_or("?"),
        prev_v["model"].as_str().unwrap_or("(default)")
    ))
}

pub fn forget() -> String {
    let _ = fs::remove_file(state_file());
    unsafe { libc::sync() };
    "FMM configuration removed; local model only".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switches_keep_the_chatgpt_sign_in_and_go_back() {
        let dir = std::env::temp_dir().join(format!("bia-fmm-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        std::env::set_var("BIA_FILES", &dir);
        save(&json!({
            "provider": "chatgpt", "api_key": "at1", "model": "gpt-6.1-sol", "base_url": "",
            "enabled": true, "ctx": 400000,
            "chatgpt": {"client_id": "c", "host_id": "h", "access_token": "at1",
                        "refresh_token": "rt1", "expires_at": 9}
        }))
        .unwrap();

        // to DeepSeek on OpenRouter: the sign-in stays, the ChatGPT window does not carry over
        configure(
            "openrouter",
            "sk-or-v1-abc",
            "deepseek/deepseek-chat",
            None,
            None,
        )
        .unwrap();
        let c = load().unwrap();
        assert_eq!(c["provider"], "openrouter");
        assert_eq!(c["ctx"], 131072);
        assert_eq!(c["chatgpt"]["refresh_token"], "rt1");
        assert_eq!(c["chatgpt"]["model"], "gpt-6.1-sol");
        assert!(status().contains("ChatGPT sign-in is saved"));

        // back to the plan without a key or a new sign-in
        configure("chatgpt", "", "", None, None).unwrap();
        let c = load().unwrap();
        assert_eq!(
            (c["provider"].as_str(), c["model"].as_str()),
            (Some("chatgpt"), Some("gpt-6.1-sol"))
        );
        assert_eq!(c["api_key"], "at1");

        // fmm_back swaps with the agent's fmm.prev.json, both ways
        assert!(back().is_err());
        fs::write(
            state_file().with_file_name("fmm.prev.json"),
            br#"{"provider":"deepseek","api_key":"sk-x","model":"deepseek-chat","ctx":65536}"#,
        )
        .unwrap();
        back().unwrap();
        assert_eq!(load().unwrap()["provider"], "deepseek");
        back().unwrap();
        assert_eq!(load().unwrap()["provider"], "chatgpt");

        forget();
        assert!(configure("chatgpt", "", "", None, None).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
