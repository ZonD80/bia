//! Pre-flash FMM check, done as bia's first words: one real request to the chosen model,
//! the way the agent makes it (streamed chat completions, or Anthropic Messages), asking it
//! to greet the user through a `say` tool. bia drives everything through tool calls, so a
//! model that only answers in text fails. The two lines it says are shown while flashing.

use crate::fmm::{api_base, FmmInput};
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader};
use std::time::{Duration, Instant};

const SYSTEM: &str = "You are bia, an AI that lives on a USB stick and boots a computer straight into yourself. \
Right now your human is writing you onto a new USB stick: your new home. Call the say tool exactly once, \
and do not answer in plain text. hello: one short, warm sentence greeting your human, saying you are bia \
and you are about to prepare your new home. ready: one short sentence for when the stick is written, \
inviting them to plug it in and boot you. Keep each under 20 words; no emoji, no quotes.";

/// What bia said in the test, plus how the request went.
#[derive(Debug, Clone)]
pub struct Greeting {
    /// Shown before flashing, e.g. "Hi! I'm bia, let's prepare my new home."
    pub hello: String,
    /// Shown when the stick is written.
    pub ready: String,
    /// "openrouter x-ai/grok-4.6, 1.2s" for logs.
    pub summary: String,
}

/// Ask the model to greet through the `say` tool. Fails when it can't be reached or answers
/// without calling the tool.
pub fn test_model(input: &FmmInput) -> Result<Greeting> {
    let c = input.clone().resolve()?;
    let base = match c.base_url.trim().trim_end_matches('/') {
        "" => api_base(&c.provider).to_string(),
        b => b.to_string(),
    };
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(120)))
        .http_status_as_error(false)
        .user_agent("bia-deploy")
        .build()
        .into();
    let start = Instant::now();
    let (args, text) = if c.provider == "anthropic" {
        anthropic(&agent, &base, &c)?
    } else if c.provider == "chatgpt" {
        responses(&agent, &base, &c)?
    } else {
        chat_completions(&agent, &base, &c)?
    };
    let secs = start.elapsed().as_secs_f32();
    let Some(args) = args else {
        let said: String = text.trim().chars().take(160).collect();
        bail!(
            "{} {} answered but did not call the tool; bia needs a model with tool calling{}",
            c.provider,
            c.model,
            if said.is_empty() { String::new() } else { format!(" (it said: {said})") }
        );
    };
    let line = |k: &str, fallback: &str| {
        let s: String = args[k].as_str().unwrap_or("").trim().trim_matches('"').chars().take(200).collect();
        if s.is_empty() { fallback.to_string() } else { s }
    };
    Ok(Greeting {
        hello: line("hello", "Hi! I'm bia. Let's prepare my new home."),
        ready: line("ready", "My home is ready. Plug me in and boot!"),
        summary: format!("{} {}, {secs:.1}s", c.provider, c.model),
    })
}

fn say_schema() -> Value {
    json!({"type": "object", "properties": {
        "hello": {"type": "string", "description": "greeting before the stick is written"},
        "ready": {"type": "string", "description": "line for when the stick is written"}
    }, "required": ["hello", "ready"]})
}

const SAY_DESC: &str = "Say two short lines to your human.";

/// Turn an HTTP error body into a short message (`error.message` when there is one).
fn http_error(provider: &str, status: u16, body: &str) -> anyhow::Error {
    let msg = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| {
            v.pointer("/error/message")
                .or_else(|| v.get("message"))
                .and_then(|m| m.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| body.chars().take(300).collect());
    match status {
        401 | 403 => anyhow::anyhow!("{provider} rejected the key (HTTP {status}): {msg}"),
        404 => anyhow::anyhow!("{provider}: model or endpoint not found (HTTP 404): {msg}"),
        402 => anyhow::anyhow!("{provider}: no credit on this account (HTTP 402): {msg}"),
        429 => anyhow::anyhow!("{provider}: rate limited or out of quota (HTTP 429): {msg}"),
        _ => anyhow::anyhow!("{provider}: HTTP {status}: {msg}"),
    }
}

/// Streamed OpenAI-style request; returns (say arguments if the tool was called, text).
fn chat_completions(agent: &ureq::Agent, base: &str, c: &FmmInput) -> Result<(Option<Value>, String)> {
    let body = json!({
        "model": c.model,
        "messages": [
            {"role": "system", "content": SYSTEM},
            {"role": "user", "content": "Hi bia!"}
        ],
        "tools": [{"type": "function", "function": {
            "name": "say", "description": SAY_DESC, "parameters": say_schema()}}],
        "stream": true,
    });
    let mut req = agent
        .post(&format!("{base}/chat/completions"))
        .header("Authorization", &format!("Bearer {}", c.api_key))
        .header("Content-Type", "application/json");
    if c.provider == "openrouter" {
        req = req.header("HTTP-Referer", "https://bia.local").header("X-Title", "bia");
    }
    let resp = req.send(body.to_string()).with_context(|| format!("reach {base}"))?;
    let status = resp.status().as_u16();
    if status >= 400 {
        let text = resp.into_body().read_to_string().unwrap_or_default();
        return Err(http_error(&c.provider, status, &text));
    }
    // Tool-call arguments stream in as JSON fragments.
    let (mut called, mut args, mut text) = (false, String::new(), String::new());
    for line in BufReader::new(resp.into_body().into_reader()).lines() {
        let line = line.context("read stream")?;
        let Some(data) = line.strip_prefix("data:").map(str::trim) else {
            continue;
        };
        if data == "[DONE]" {
            break;
        }
        let Ok(v) = serde_json::from_str::<Value>(data) else {
            continue;
        };
        if let Some(e) = v.get("error") {
            bail!("{}: {}", c.provider, e.get("message").and_then(|m| m.as_str()).unwrap_or("stream error"));
        }
        let delta = &v["choices"][0]["delta"];
        for t in delta["tool_calls"].as_array().into_iter().flatten() {
            called |= t["function"]["name"].as_str().is_some_and(|n| n.contains("say"));
            if let Some(a) = t["function"]["arguments"].as_str() {
                args.push_str(a);
            }
        }
        if let Some(s) = delta["content"].as_str() {
            text.push_str(s);
        }
    }
    let args = called.then(|| serde_json::from_str(&args).unwrap_or(Value::Null));
    Ok((args, text))
}

/// Anthropic Messages request; returns (say arguments if the tool was called, text).
fn anthropic(agent: &ureq::Agent, base: &str, c: &FmmInput) -> Result<(Option<Value>, String)> {
    let body = json!({
        "model": c.model,
        "max_tokens": 1024,
        "system": SYSTEM,
        "messages": [{"role": "user", "content": "Hi bia!"}],
        "tools": [{"name": "say", "description": SAY_DESC, "input_schema": say_schema()}],
    });
    let resp = agent
        .post(&format!("{base}/v1/messages"))
        .header("x-api-key", &c.api_key)
        .header("anthropic-version", "2023-06-01")
        .header("Content-Type", "application/json")
        .send(body.to_string())
        .with_context(|| format!("reach {base}"))?;
    let status = resp.status().as_u16();
    let text = resp.into_body().read_to_string().unwrap_or_default();
    if status >= 400 {
        return Err(http_error(&c.provider, status, &text));
    }
    let v: Value = serde_json::from_str(&text).context("parse Anthropic reply")?;
    let blocks = v["content"].as_array().cloned().unwrap_or_default();
    let args = blocks
        .iter()
        .find(|b| b["type"] == "tool_use" && b["name"] == "say")
        .map(|b| b["input"].clone());
    let said = blocks
        .iter()
        .filter_map(|b| b["text"].as_str())
        .collect::<Vec<_>>()
        .join(" ");
    Ok((args, said))
}

/// Responses API with a ChatGPT sign-in (store=false, stream=true, no system message, no
/// sampling fields). The ChatGPT-plan route wants function tools grouped in a namespace; if
/// it refuses that shape, plain top-level function tools are tried once.
fn responses(agent: &ureq::Agent, base: &str, c: &FmmInput) -> Result<(Option<Value>, String)> {
    let say = json!({"type": "function", "name": "say", "description": SAY_DESC, "parameters": say_schema()});
    let shapes = [
        json!([{"type": "namespace", "name": "bia", "description": "bia's tools", "tools": [say.clone()]}]),
        json!([say]),
    ];
    let mut last = None;
    for tools in shapes {
        let body = json!({
            "model": c.model,
            "instructions": SYSTEM,
            "input": [{"role": "user", "content": [{"type": "input_text", "text": "Hi bia!"}]}],
            "tools": tools,
            "store": false,
            "stream": true,
        });
        let resp = agent
            .post(&format!("{base}/responses"))
            .header("Authorization", &format!("Bearer {}", c.api_key))
            .header("Content-Type", "application/json")
            .send(body.to_string())
            .with_context(|| format!("reach {base}"))?;
        let status = resp.status().as_u16();
        if status >= 400 {
            let text = resp.into_body().read_to_string().unwrap_or_default();
            let err = chatgpt_error(status, &text);
            if status == 400 {
                last = Some(err);
                continue;
            }
            return Err(err);
        }
        let (mut args, mut text, mut done) = (None, String::new(), false);
        for line in BufReader::new(resp.into_body().into_reader()).lines() {
            let line = line.context("read stream")?;
            let Some(data) = line.strip_prefix("data:").map(str::trim) else {
                continue;
            };
            let Ok(v) = serde_json::from_str::<Value>(data) else {
                continue;
            };
            match v["type"].as_str().unwrap_or("") {
                "response.output_text.delta" => text.push_str(v["delta"].as_str().unwrap_or("")),
                "response.output_item.done" => {
                    let item = &v["item"];
                    if item["type"] == "function_call" && item["name"].as_str().is_some_and(|n| n.ends_with("say")) {
                        args = Some(serde_json::from_str(item["arguments"].as_str().unwrap_or("{}")).unwrap_or(Value::Null));
                    }
                }
                "response.completed" => {
                    done = true;
                    break;
                }
                "response.failed" | "response.incomplete" | "error" => {
                    let e = v["response"]["error"].clone();
                    let e = if e.is_null() { v.clone() } else { e };
                    bail!("chatgpt: {}", plan_error(e["code"].as_str().unwrap_or(""), e["message"].as_str().unwrap_or("request failed")));
                }
                _ => {}
            }
        }
        if !done && args.is_none() {
            bail!("chatgpt: the reply stream ended early");
        }
        return Ok((args, text));
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("chatgpt: request refused")))
}

/// Plain words for the ChatGPT-plan error codes (developers.openai.com/siwc, Errors and recovery).
fn plan_error(code: &str, msg: &str) -> String {
    match code {
        "subscription_sharing_user_not_eligible" => "this ChatGPT plan can't be used by apps like bia".into(),
        "subscription_sharing_usage_limit_exceeded" => {
            "this ChatGPT plan's usage limit is reached for now (see ChatGPT settings → Usage)".into()
        }
        "subscription_sharing_usage_unavailable" | "subscription_sharing_user_unavailable" => {
            "ChatGPT couldn't check your plan right now; try again in a minute".into()
        }
        "subscription_sharing_invalid_user" => "ChatGPT did not accept this sign-in; sign in again".into(),
        "" => msg.to_string(),
        c => format!("{c}: {msg}"),
    }
}

fn chatgpt_error(status: u16, body: &str) -> anyhow::Error {
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let code = v["error"]["code"].as_str().unwrap_or("");
    let msg = v["error"]["message"].as_str().map(str::to_string).unwrap_or_else(|| body.chars().take(300).collect());
    match (status, code) {
        (_, c) if !c.is_empty() => anyhow::anyhow!("chatgpt: {} (HTTP {status})", plan_error(c, &msg)),
        (401, _) => anyhow::anyhow!("chatgpt: the sign-in was rejected (HTTP 401); sign in again"),
        _ => http_error("chatgpt", status, body),
    }
}
