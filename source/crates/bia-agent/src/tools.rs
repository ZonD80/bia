//! Tool registry (built-in + MCP), prompt rendering and tool-call parsing for the model's format.
use crate::console::{Console, BCYAN, CYAN, GRAY, RESET};
use crate::mcp::{McpServer, McpTool};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub const ID_SYSTEM: &str = "system";
pub const ID_ROOT: &str = "root";
pub const CH_CONSOLE: &str = "console";
pub const CH_WEB: &str = "web";
pub const CH_SCHEDULE: &str = "schedule";

/// MCP functions the model never sees: the agent's built-ins (say, react, delete_messages,
/// pin_message, close_poll, chat_history, read receipts, typing) call them for web chats.
const AGENT_ONLY_WEB: &[&str] = &[
    "send",
    "edit",
    "react",
    "delete_messages",
    "pin",
    "close_poll",
    "history",
    "read",
    "typing",
];

fn agent_only(server: &str, tool: &str) -> bool {
    server == "web" && AGENT_ONLY_WEB.contains(&tool)
}

/// A web chat channel (`web` = the person's main chat, `web:<chat>` = another one). Web is a
/// bus served by the web MCP, like telegram or whatsapp; the person is the envelope identity.
/// Chat-app buses served by an MCP: their channels are `<bus>:<chat>`, and sends, edits,
/// reactions and deletes there go through the outbox to the MCP's tools.
pub const CHAT_BUSES: &[&str] = &["telegram", "whatsapp", "messenger", "instagram"];

/// Chat buses whose MCP can speak into a voice call (call_say).
const CALL_BUSES: &[&str] = &["telegram", "whatsapp"];

/// (bus, chat) of a chat-app channel like `messenger:100012345678`.
pub fn chat_bus(channel: &str) -> Option<(&str, &str)> {
    channel.split_once(':').filter(|(b, _)| CHAT_BUSES.contains(b))
}

pub fn is_web(channel: &str) -> bool {
    channel == CH_WEB || channel.starts_with("web:")
}

/// `identity|channel|issued|delivered|id|message` — RFC 3339 UTC timestamps of when the message
/// was created and when it was handed to the model. `id` is the platform message id (empty if none).
pub fn envelope(
    identity: &str,
    channel: &str,
    issued: std::time::SystemTime,
    id: &str,
    message: &str,
) -> String {
    let id = id.replace('|', "");
    format!(
        "{identity}|{channel}|{}|{}|{id}|{message}",
        crate::clock::rfc3339(issued),
        crate::clock::rfc3339_now()
    )
}
use anyhow::{bail, Result};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolFormat {
    /// LFM2: `<|tool_call_start|>[name(a='x', b=1)]<|tool_call_end|>`; only `discover` is listed in the frozen system prompt. Other functions are appended to history when a branch is enabled.
    Lfm2,
    /// Hermes: `<tool_call>{"name":..,"arguments":..}</tool_call>`
    Hermes,
    /// Qwen3.5: `<tool_call><function=name><parameter=k>v</parameter></function></tool_call>`
    Qwen,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Role {
    #[default]
    User,
    Assistant,
    Tool,
}

#[derive(Clone, Debug, Default)]
pub struct Msg {
    pub role: Role,
    /// assistant: generated text (locally generated text may still contain tool-call markup); tool: the result text
    pub content: String,
    /// tool messages: the tool name
    pub name: String,
    /// assistant messages: structured tool calls (always filled for remote models; for local ones after parsing)
    pub tool_calls: Vec<ToolCall>,
    /// tool messages: id of the call this result answers
    pub call_id: String,
    /// journal id (`m_N`); empty in old saved conversations
    pub id: String,
}

impl Msg {
    pub fn user(content: String) -> Msg {
        Msg {
            role: Role::User,
            content,
            ..Default::default()
        }
    }
    pub fn assistant(content: String, tool_calls: Vec<ToolCall>) -> Msg {
        Msg {
            role: Role::Assistant,
            content,
            tool_calls,
            ..Default::default()
        }
    }
    pub fn tool(name: String, call_id: String, content: String) -> Msg {
        Msg {
            role: Role::Tool,
            name,
            call_id,
            content,
            ..Default::default()
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub args: Value,
}

impl ToolFormat {
    /// Qwen3 thinking models open every turn with `<think>` and close it with `</think>`.
    pub fn opens_think(template: &str) -> bool {
        template.contains("<think>")
    }

    pub fn detect(template: &str) -> ToolFormat {
        if template.contains("<function=") && template.contains("<parameter=") {
            ToolFormat::Qwen
        } else if template.contains("tool_list_start") || template.contains("tool_call_start") {
            ToolFormat::Lfm2
        } else {
            ToolFormat::Hermes
        }
    }

    pub fn markers(&self) -> (&'static str, &'static str) {
        match self {
            ToolFormat::Lfm2 => ("<|tool_call_start|>", "<|tool_call_end|>"),
            ToolFormat::Hermes | ToolFormat::Qwen => ("<tool_call>", "</tool_call>"),
        }
    }

    fn openai_tool(t: &ToolSpec) -> Value {
        json!({"type": "function", "function": {"name": t.name, "description": t.description, "parameters": t.parameters}})
    }

    pub fn system_prompt(&self, system: &str, tools: &[ToolSpec]) -> String {
        if tools.is_empty() {
            return system.to_string();
        }
        match self {
            ToolFormat::Lfm2 => {
                // LFM2 model card format: flat {"name","description","parameters"} objects
                let list: Vec<String> = tools
                    .iter()
                    .map(|t| json!({"name": t.name, "description": t.description, "parameters": t.parameters}).to_string())
                    .collect();
                format!(
                    "{system}\nList of tools: <|tool_list_start|>[{}]<|tool_list_end|>",
                    list.join(", ")
                )
            }
            ToolFormat::Hermes => {
                let mut s = format!("{system}\n\n# Tools\n\nYou may call one or more functions to assist with the user query.\n\nYou are provided with function signatures within <tools></tools> XML tags:\n<tools>\n");
                for t in tools {
                    s.push_str(&Self::openai_tool(t).to_string());
                    s.push('\n');
                }
                s.push_str("</tools>\n\nFor each function call, return a json object with function name and arguments within <tool_call></tool_call> XML tags:\n<tool_call>\n{\"name\": <function-name>, \"arguments\": <args-json-object>}\n</tool_call>");
                s
            }
            ToolFormat::Qwen => {
                let mut s = format!(
                    "{system}\n\n# Tools\n\nYou have access to the following functions:\n\n<tools>"
                );
                for t in tools {
                    s.push('\n');
                    s.push_str(&Self::openai_tool(t).to_string());
                }
                s.push_str("\n</tools>\n\nIf you choose to call a function ONLY reply in the following format with NO suffix:\n\n<tool_call>\n<function=example_function_name>\n<parameter=example_parameter_1>\nvalue_1\n</parameter>\n</function>\n</tool_call>");
                s
            }
        }
    }

    /// ChatML rendering; with `add_generation_prompt` a trailing assistant header is added.
    pub fn render(
        &self,
        system: &str,
        tools: &[ToolSpec],
        history: &[Msg],
        add_generation_prompt: bool,
        think: bool,
    ) -> String {
        let mut p = String::new();
        p.push_str("<|im_start|>system\n");
        p.push_str(&self.system_prompt(system, tools));
        p.push_str("<|im_end|>\n");
        for m in history {
            match m.role {
                Role::User => {
                    p.push_str("<|im_start|>user\n");
                    p.push_str(&m.content);
                    p.push_str("<|im_end|>\n");
                }
                Role::Assistant => {
                    p.push_str("<|im_start|>assistant\n");
                    p.push_str(m.content.trim_end());
                    let (start, _) = self.markers();
                    if !m.tool_calls.is_empty() && !m.content.contains(start) {
                        p.push_str(&self.render_calls(&m.tool_calls));
                    }
                    p.push_str("<|im_end|>\n");
                }
                Role::Tool => match self {
                    ToolFormat::Lfm2 => {
                        p.push_str("<|im_start|>tool\n");
                        p.push_str(&json!([{"name": m.name, "result": m.content}]).to_string());
                        p.push_str("<|im_end|>\n");
                    }
                    ToolFormat::Hermes | ToolFormat::Qwen => {
                        p.push_str("<|im_start|>user\n<tool_response>\n");
                        p.push_str(&m.content);
                        p.push_str("\n</tool_response><|im_end|>\n");
                    }
                },
            }
        }
        if add_generation_prompt {
            p.push_str("<|im_start|>assistant\n");
            if think {
                p.push_str("<think>\n");
            }
        }
        p
    }

    /// Tool-call markup for structured calls (used when a remote model's turn is replayed to the local one).
    pub fn render_calls(&self, calls: &[ToolCall]) -> String {
        let (start, end) = self.markers();
        match self {
            ToolFormat::Lfm2 => {
                let items: Vec<String> = calls
                    .iter()
                    .map(|c| {
                        let args: Vec<String> = c
                            .args
                            .as_object()
                            .map(|o| {
                                o.iter()
                                    .map(|(k, v)| format!("{k}={}", pythonic_value(v)))
                                    .collect()
                            })
                            .unwrap_or_default();
                        format!("{}({})", c.name, args.join(", "))
                    })
                    .collect();
                format!("{start}[{}]{end}", items.join(", "))
            }
            ToolFormat::Hermes => calls
                .iter()
                .map(|c| {
                    format!(
                        "{start}\n{}\n{end}",
                        json!({"name": c.name, "arguments": c.args})
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"),
            ToolFormat::Qwen => calls
                .iter()
                .map(|c| {
                    let mut s = format!("{start}\n<function={}>\n", c.name);
                    if let Some(obj) = c.args.as_object() {
                        for (k, v) in obj {
                            let val = match v {
                                Value::String(t) => t.clone(),
                                other => other.to_string(),
                            };
                            s.push_str(&format!("<parameter={k}>\n{val}\n</parameter>\n"));
                        }
                    }
                    s.push_str(&format!("</function>\n{end}"));
                    s
                })
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }

    /// All tool calls in a generated text.
    pub fn parse_calls(&self, text: &str) -> Vec<ToolCall> {
        let (start, end) = self.markers();
        let mut out = vec![];
        let mut rest = text;
        while let Some(i) = rest.find(start) {
            let after = &rest[i + start.len()..];
            let body = match after.find(end) {
                Some(j) => &after[..j],
                None => after,
            };
            out.extend(parse_call_body(body.trim()));
            rest = match after.find(end) {
                Some(j) => &after[j + end.len()..],
                None => "",
            };
        }
        out
    }

    /// Fallback for models that emit a call without the markers, e.g. `[network_dhcp(interface='eth0')]`
    /// or `{"name": "...", "arguments": {...}}` as plain text. Only calls to known tools are accepted.
    pub fn parse_bare_calls(&self, text: &str, known: &[String]) -> Vec<ToolCall> {
        let t = text.replace("<|im_end|>", "");
        let t = match t.rfind("</think>") {
            Some(i) => &t[i + 8..],
            None => t.as_str(),
        }
        .trim();
        if t.contains("<function=") {
            return parse_qwen_functions(t)
                .into_iter()
                .filter(|c| known.contains(&c.name))
                .collect();
        }
        let looks_like_call = (t.starts_with('[') && t.ends_with(']'))
            || (t.starts_with('{') && t.ends_with('}'))
            || (t.ends_with(')')
                && t.chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .count()
                    > 0
                && t.contains('('));
        if !looks_like_call || t.contains('\n') && !t.starts_with('{') {
            return vec![];
        }
        parse_call_body(t)
            .into_iter()
            .filter(|c| known.contains(&c.name))
            .collect()
    }

    /// Text without tool-call markup and special tokens (for display / logs).
    pub fn display_text(&self, text: &str) -> String {
        let (start, end) = self.markers();
        let mut s = String::new();
        let mut rest = text;
        while let Some(i) = rest.find(start) {
            s.push_str(&rest[..i]);
            rest = match rest[i..].find(end) {
                Some(j) => &rest[i + j + end.len()..],
                None => "",
            };
        }
        s.push_str(rest);
        s.replace("<|im_end|>", "").trim().to_string()
    }
}

fn pythonic_value(v: &Value) -> String {
    match v {
        Value::String(s) => format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'")),
        Value::Bool(b) => {
            if *b {
                "True".into()
            } else {
                "False".into()
            }
        }
        Value::Null => "None".into(),
        other => other.to_string(),
    }
}

/// Give every message structured tool calls and call ids (needed by remote models):
/// locally generated assistant text is parsed and its markup stripped; tool results are
/// matched to the preceding assistant's calls in order.
pub fn normalize_history(format: ToolFormat, history: &[Msg]) -> Vec<Msg> {
    let mut out: Vec<Msg> = Vec::with_capacity(history.len());
    let mut open: std::collections::VecDeque<(String, String)> = Default::default(); // (id, name) of calls awaiting results
    // Chat APIs reject a history where an assistant's tool calls are not answered right after
    // it (DeepSeek: "insufficient tool messages following tool_calls message"), and one such
    // spot breaks every later request. Messages that came between a call and its results wait
    // until the results are in; calls that never got a result are answered as not run.
    let mut held: Vec<Msg> = vec![];
    fn close(out: &mut Vec<Msg>, open: &mut std::collections::VecDeque<(String, String)>, held: &mut Vec<Msg>) {
        for (id, name) in open.drain(..) {
            out.push(Msg::tool(name, id, NOT_RUN.into()));
        }
        out.append(held);
    }
    for (i, m) in history.iter().enumerate() {
        match m.role {
            Role::Assistant => {
                close(&mut out, &mut open, &mut held);
                let mut msg = m.clone();
                if msg.tool_calls.is_empty() {
                    msg.tool_calls = format.parse_calls(&m.content);
                }
                for (k, c) in msg.tool_calls.iter_mut().enumerate() {
                    if c.id.is_empty() {
                        c.id = format!("call_{i}_{k}");
                    }
                }
                msg.content = format.display_text(&m.content);
                open = msg
                    .tool_calls
                    .iter()
                    .map(|c| (c.id.clone(), c.name.clone()))
                    .collect();
                out.push(msg);
            }
            Role::Tool => {
                let mut msg = m.clone();
                let pos = if msg.call_id.is_empty() {
                    // take the first open call with this name, else the first open call
                    open.iter()
                        .position(|(_, n)| n == &m.name)
                        .or(if open.is_empty() { None } else { Some(0) })
                } else {
                    open.iter().position(|(id, _)| id == &msg.call_id)
                };
                match pos {
                    Some(p) => {
                        msg.call_id = open.remove(p).map(|(id, _)| id).unwrap_or_default();
                        out.push(msg);
                        if open.is_empty() {
                            out.append(&mut held);
                        }
                    }
                    None => {
                        // orphan result (its call is gone or already answered): plain text
                        let text = Msg::user(format!("[result of {}] {}", m.name, m.content));
                        if open.is_empty() {
                            out.push(text);
                        } else {
                            held.push(text);
                        }
                    }
                }
            }
            Role::User if !open.is_empty() => held.push(m.clone()),
            Role::User => out.push(m.clone()),
        }
    }
    close(&mut out, &mut open, &mut held);
    out
}

/// Only messages to people in this round (`say`, also through discover/invoke as FMM
/// models call it): such rounds do not count against max_tool_rounds.
pub fn only_say(calls: &[ToolCall]) -> bool {
    calls.iter().all(|c| Tools::expand_call(&c.name, &c.args).0 == "say")
}

/// Marker key for tool-call arguments that arrived as broken JSON (remote::parse_args).
pub const CUT_ARGS: &str = "__bia_cut_off_arguments";

/// A call whose arguments were cut off: what to tell the model instead of running it.
pub fn cut_args_error(args: &Value) -> Option<String> {
    let c = args.get(CUT_ARGS)?;
    let stop = c.get("stop").and_then(|s| s.as_str()).unwrap_or("");
    let why = if matches!(stop, "length" | "max_tokens" | "max_output_tokens") {
        " (your reply hit the provider's output limit)"
    } else {
        ""
    };
    Some(format!(
        "your tool call arrived with incomplete arguments ({} bytes of broken JSON){why}, so nothing was run. Call it again; if the arguments are long (a big file or message), split the work into several smaller calls.",
        c.get("bytes").and_then(|b| b.as_u64()).unwrap_or(0)
    ))
}

/// The result given to a tool call that never ran (round limit, interruption, restart).
pub const NOT_RUN: &str = "not run: this call was never executed (stopped or the tool round limit was reached); call it again if it is still needed";

/// What a set_model_config call asks to change (validated; `None` = keep).
#[derive(Debug, Default, PartialEq)]
pub struct ModelChange {
    pub context: Option<u32>,
    pub max_tool_rounds: Option<usize>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub top_k: Option<i32>,
    pub min_p: Option<f32>,
    pub repeat_penalty: Option<f32>,
    pub repeat_last_n: Option<i32>,
}

impl ModelChange {
    pub fn touches_sampling(&self) -> bool {
        self.temperature.is_some()
            || self.top_p.is_some()
            || self.top_k.is_some()
            || self.min_p.is_some()
            || self.repeat_penalty.is_some()
            || self.repeat_last_n.is_some()
    }

    pub fn apply_sampling(&self, s: &mut crate::config::SamplingCfg) {
        if let Some(v) = self.temperature {
            s.temperature = v;
        }
        if let Some(v) = self.top_p {
            s.top_p = v;
        }
        if let Some(v) = self.top_k {
            s.top_k = v;
        }
        if let Some(v) = self.min_p {
            s.min_p = v;
        }
        if let Some(v) = self.repeat_penalty {
            s.repeat_penalty = v;
        }
        if let Some(v) = self.repeat_last_n {
            s.repeat_last_n = v;
        }
    }
}

/// Check set_model_config's arguments; numbers may come as strings. Unknown fields are refused
/// so a typo does not look like a change.
pub fn parse_model_change(args: &Value) -> Result<ModelChange, String> {
    const KNOWN: &[&str] = &["context", "max_tool_rounds", "temperature", "top_p", "top_k", "min_p", "repeat_penalty", "repeat_last_n"];
    let mut ch = ModelChange::default();
    let Some(o) = args.as_object() else {
        return Ok(ch);
    };
    for k in o.keys() {
        if !KNOWN.contains(&k.as_str()) {
            return Err(format!("unknown setting {k}; known: {}", KNOWN.join(", ")));
        }
    }
    let num = |k: &str| -> Result<Option<f64>, String> {
        match o.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Number(n)) => Ok(n.as_f64()),
            Some(Value::String(s)) => s.trim().parse::<f64>().map(Some).map_err(|_| format!("{k} must be a number")),
            Some(_) => Err(format!("{k} must be a number")),
        }
    };
    let ranged = |k: &str, lo: f64, hi: f64| -> Result<Option<f64>, String> {
        match num(k)? {
            Some(v) if !(lo..=hi).contains(&v) => Err(format!("{k} must be between {lo} and {hi}")),
            v => Ok(v),
        }
    };
    let whole = |k: &str, v: Option<f64>| -> Result<Option<f64>, String> {
        match v {
            Some(x) if x.fract() != 0.0 => Err(format!("{k} must be a whole number")),
            v => Ok(v),
        }
    };
    ch.context = whole("context", ranged("context", 512.0, 2_000_000.0)?)?.map(|v| v as u32);
    ch.max_tool_rounds = whole("max_tool_rounds", ranged("max_tool_rounds", 4.0, 256.0)?)?.map(|v| v as usize);
    ch.temperature = ranged("temperature", 0.0, 2.0)?.map(|v| v as f32);
    ch.top_p = ranged("top_p", 0.0, 1.0)?.map(|v| v as f32);
    ch.top_k = whole("top_k", ranged("top_k", 0.0, 1000.0)?)?.map(|v| v as i32);
    ch.min_p = ranged("min_p", 0.0, 1.0)?.map(|v| v as f32);
    ch.repeat_penalty = ranged("repeat_penalty", 0.5, 2.0)?.map(|v| v as f32);
    ch.repeat_last_n = whole("repeat_last_n", ranged("repeat_last_n", -1.0, 4096.0)?)?.map(|v| v as i32);
    Ok(ch)
}

/// Qwen3.5: `<function=name><parameter=k>value</parameter></function>`.
fn parse_qwen_functions(body: &str) -> Vec<ToolCall> {
    let mut out = vec![];
    let mut rest = body;
    while let Some(i) = rest.find("<function=") {
        let after = &rest[i + "<function=".len()..];
        let Some(name_end) = after.find('>') else {
            break;
        };
        let name = after[..name_end].trim();
        if name.is_empty() {
            break;
        }
        let after = &after[name_end + 1..];
        let func_end = after.find("</function>").unwrap_or(after.len());
        let func_body = &after[..func_end];
        let mut args = serde_json::Map::new();
        let mut p = func_body;
        while let Some(k) = p.find("<parameter=") {
            let pa = &p[k + "<parameter=".len()..];
            let Some(key_end) = pa.find('>') else { break };
            let key = pa[..key_end].trim();
            let pa = &pa[key_end + 1..];
            let val_end = pa.find("</parameter>").unwrap_or(pa.len());
            let raw = pa[..val_end].trim();
            let parsed = serde_json::from_str(raw).unwrap_or(Value::String(raw.to_string()));
            if !key.is_empty() {
                args.insert(key.to_string(), parsed);
            }
            let step = val_end + "</parameter>".len();
            p = if step <= pa.len() { &pa[step..] } else { "" };
        }
        out.push(ToolCall {
            id: String::new(),
            name: name.to_string(),
            args: Value::Object(args),
        });
        rest = match after.find("</function>") {
            Some(j) => &after[j + "</function>".len()..],
            None => "",
        };
    }
    out
}

/// Body between the markers: JSON object, JSON array of objects, Qwen XML, or pythonic `[f(a=1), g()]`.
fn parse_call_body(body: &str) -> Vec<ToolCall> {
    if body.contains("<function=") {
        let calls = parse_qwen_functions(body);
        if !calls.is_empty() {
            return calls;
        }
    }
    if let Ok(v) = serde_json::from_str::<Value>(body) {
        let mut out = vec![];
        let items: Vec<Value> = match v {
            Value::Array(a) => a,
            o => vec![o],
        };
        for it in items {
            if let Some(name) = it.get("name").and_then(|n| n.as_str()) {
                let args = it
                    .get("arguments")
                    .or_else(|| it.get("parameters"))
                    .cloned()
                    .unwrap_or(json!({}));
                let args = match args {
                    Value::String(s) => serde_json::from_str(&s).unwrap_or(Value::String(s)),
                    a => a,
                };
                out.push(ToolCall {
                    id: String::new(),
                    name: name.to_string(),
                    args,
                });
            }
        }
        if !out.is_empty() {
            return out;
        }
    }
    Pythonic::new(body).parse_list().unwrap_or_default()
}

/// Tiny parser for `[name(k='v', k2=3, k3={"a":1}), other()]`.
struct Pythonic<'a> {
    s: &'a str,
    i: usize,
}

impl<'a> Pythonic<'a> {
    fn new(s: &'a str) -> Self {
        Pythonic { s, i: 0 }
    }
    fn peek(&self) -> Option<char> {
        self.s[self.i..].chars().next()
    }
    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.i += c.len_utf8();
        Some(c)
    }
    fn ws(&mut self) {
        while matches!(self.peek(), Some(c) if c.is_whitespace()) {
            self.bump();
        }
    }
    fn eat(&mut self, c: char) -> bool {
        self.ws();
        if self.peek() == Some(c) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn ident(&mut self) -> Result<String> {
        self.ws();
        let start = self.i;
        while matches!(self.peek(), Some(c) if c.is_alphanumeric() || c == '_' || c == '.' || c == '-')
        {
            self.bump();
        }
        if start == self.i {
            bail!("expected identifier at {}", self.i);
        }
        Ok(self.s[start..self.i].to_string())
    }
    fn parse_list(&mut self) -> Result<Vec<ToolCall>> {
        let bracket = self.eat('[');
        let mut out = vec![];
        loop {
            self.ws();
            if self.peek().is_none() || (bracket && self.peek() == Some(']')) {
                break;
            }
            out.push(self.parse_call()?);
            if !self.eat(',') {
                break;
            }
        }
        Ok(out)
    }
    fn parse_call(&mut self) -> Result<ToolCall> {
        let name = self.ident()?;
        let mut args = Map::new();
        if self.eat('(') {
            loop {
                self.ws();
                if self.eat(')') {
                    break;
                }
                let key = self.ident()?;
                if !self.eat('=') {
                    bail!("expected '=' after {key}");
                }
                let v = self.value()?;
                args.insert(key, v);
                if self.eat(',') {
                    continue;
                }
                if self.eat(')') {
                    break;
                }
                bail!("expected ',' or ')'");
            }
        }
        Ok(ToolCall {
            id: String::new(),
            name,
            args: Value::Object(args),
        })
    }
    fn value(&mut self) -> Result<Value> {
        self.ws();
        match self.peek() {
            Some('\'') | Some('"') => {
                let q = self.bump().unwrap();
                let mut s = String::new();
                loop {
                    match self.bump() {
                        None => bail!("unterminated string"),
                        Some('\\') => match self.bump() {
                            Some('n') => s.push('\n'),
                            Some('t') => s.push('\t'),
                            Some(c) => s.push(c),
                            None => bail!("bad escape"),
                        },
                        Some(c) if c == q => break,
                        Some(c) => s.push(c),
                    }
                }
                Ok(Value::String(s))
            }
            Some('{') | Some('[') => {
                // balanced JSON literal
                let start = self.i;
                let mut depth = 0i32;
                let mut in_str: Option<char> = None;
                let mut esc = false;
                while let Some(c) = self.bump() {
                    if let Some(q) = in_str {
                        if esc {
                            esc = false;
                        } else if c == '\\' {
                            esc = true;
                        } else if c == q {
                            in_str = None;
                        }
                        continue;
                    }
                    match c {
                        '"' | '\'' => in_str = Some(c),
                        '{' | '[' => depth += 1,
                        '}' | ']' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                let lit = &self.s[start..self.i];
                Ok(serde_json::from_str(lit).unwrap_or_else(|_| Value::String(lit.to_string())))
            }
            _ => {
                let start = self.i;
                while matches!(self.peek(), Some(c) if !matches!(c, ',' | ')' | ']') && !c.is_whitespace())
                {
                    self.bump();
                }
                let lit = &self.s[start..self.i];
                Ok(match lit {
                    "True" | "true" => Value::Bool(true),
                    "False" | "false" => Value::Bool(false),
                    "None" | "null" => Value::Null,
                    _ => lit
                        .parse::<i64>()
                        .map(Value::from)
                        .or_else(|_| lit.parse::<f64>().map(Value::from))
                        .unwrap_or(Value::String(lit.to_string())),
                })
            }
        }
    }
}

// ---------------- registry ----------------

#[derive(Clone)]
enum Kind {
    Builtin,
    Mcp { server: usize, tool: String },
}

/// What `say` returns for a send held while its MCP starts.
fn held_note(mcp: &str, job: &str) -> String {
    format!("queued until mcp/{mcp} has started (job:{job}); it goes out then, in order with later sends. No local id yet. A failure arrives as system|outbox.")
}

/// An MCP server starting on its own thread (Tools::start_job), and its job.
struct Starting {
    job: String,
    handle: std::thread::JoinHandle<Option<Started>>,
}

/// A send held while its MCP starts (Tools::outbox_submit); in order per MCP.
struct Held {
    mcp: String,
    chat: String,
    op: crate::outbox::Op,
    bytes: u64,
    what: String,
}

/// A started server, not yet part of Tools (attached on the main thread).
struct Started {
    srv: McpServer,
    list: Vec<McpTool>,
    m: crate::mcp::Manifest,
}

/// A start still running after this long gets the jobs' usual mid-job `stale?` notice.
const START_STALE_SECS: u64 = 30;

/// state/<this>: sha256 of each MCP block the model was given (Tools::sent_mcp).
const MCP_BLOCKS: &str = "mcp_blocks.json";

fn block_hash(block: &str) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(block.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// What the model is told about an MCP, from its manifest; examples name its functions
/// with the server prefix (`fold(` → `carry_fold(`).
fn server_info(m: &crate::mcp::Manifest, list: &[McpTool]) -> ServerInfo {
    let examples = m
        .examples
        .iter()
        .map(|ex| {
            list.iter().fold(ex.clone(), |e, t| {
                e.replace(&format!(" {}(", t.name), &format!(" {}_{}(", m.name, t.name))
            })
        })
        .collect();
    ServerInfo {
        name: m.name.clone(),
        description: m.description.clone(),
        keywords: m.keywords.clone(),
        examples,
    }
}

/// What the model is told about an MCP before its functions are loaded.
pub struct ServerInfo {
    pub name: String,
    pub description: String,
    pub keywords: Vec<String>,
    pub examples: Vec<String>,
}

/// A reboot/poweroff requested through a tool; executed by the REPL after the tool round.
#[derive(Clone, Copy, Debug)]
pub struct PendingExit {
    pub poweroff: bool,
}

pub struct Tools {
    /// every known tool: built-ins first, then all MCP functions (prefixed with the server name)
    pub specs: Vec<ToolSpec>,
    kinds: Vec<Kind>,
    /// parallel to specs: builtin branch id (`shell`, `talk`, …), or empty for hidden builtins and MCP functions
    groups: Vec<String>,
    /// builtin branches whose schemas have been given to the model
    enabled: std::collections::HashSet<String>,
    /// paths whose schemas were already returned to the model (`shell`, `mcp/system`, `tool:bash`)
    schemas_sent: std::collections::HashSet<String>,
    /// per MCP server: sha256 of the block (description, examples, every function's full
    /// definition) last sent to the model; kept in state/mcp_blocks.json across restarts
    sent_mcp: HashMap<String, String>,
    /// MCPs restarted by themselves (a crash, a preferred copy): their changed block, for the next tool result
    reload_notes: Vec<String>,
    /// MCP servers starting in the background (start_job), by name: attached when done
    starting: HashMap<String, Starting>,
    /// sends to a messenger MCP that is still starting: into the outbox once it is up
    held: Vec<Held>,
    pub servers: Vec<McpServer>,
    pub infos: Vec<ServerInfo>,
    /// mcp.json entries seen on disk. Not started until that server is enabled.
    catalog: Vec<ServerInfo>,
    /// MCP servers whose functions are currently advertised to the model
    pub loaded: std::collections::HashSet<usize>,
    pub files_dir: PathBuf,
    pub pending_exit: std::cell::Cell<Option<PendingExit>>,
    /// the model asked to forget the conversation; done by the REPL after this turn
    pub pending_forget: std::cell::Cell<bool>,
    /// the model asked to change the active brain's context window; applied by the REPL after this call
    /// compact the live window into reflections after this turn
    pub pending_reflect: std::cell::Cell<bool>,
    con: Arc<Console>,
    pub addressbook: crate::addressbook::AddressBook,
    pub buses: crate::buses::Buses,
    pub schedules: Arc<Mutex<crate::schedules::Store>>,
    pub jobs: Arc<Mutex<crate::jobs::Store>>,
    /// telegram/whatsapp/messenger/instagram sends, edits, reactions and deletes (per-chat queues, local o<n> ids)
    pub outbox: crate::outbox::Outbox,
    /// sends cut short by the last shutdown; main delivers them as notices
    pub outbox_lost: Vec<crate::console::Inbound>,
    /// set when a model-facing MCP call was fired this batch (REPL stops tool rounds)
    pub fired_job: std::cell::Cell<bool>,
    /// Last human inbound this turn: (identity, channel). `say` defaults to these.
    pub reply_to: std::cell::RefCell<Option<(String, String)>>,
    /// identity of the last human inbound on each channel
    pub last_on: std::cell::RefCell<HashMap<String, String>>,
    /// Human messages the model has not closed with answered=[ids] yet.
    pub unanswered: crate::unanswered::Set,
    mcp_arch: String,
    mcp_env: Vec<(String, String)>,
    mcp_log_dir: PathBuf,
    /// the last few calls (name + arguments), to tell the model when it repeats one
    recent_calls: std::collections::VecDeque<String>,
}

struct Branch {
    id: &'static str,
    blurb: &'static str,
    guide: &'static str,
}

const BRANCHES: &[Branch] = &[
    Branch {
        id: "shell",
        blurb: "Run commands on this Debian machine.",
        guide: "bash(command, user, cwd, timeout_secs) is a PTY job: a stub returns now, the finished output arrives later as system|job:<id>|…. After 30s with no output you may get a stale? notice (not the result); answer prompts with job_input. Default user is root; user=bia is the everyday account; it can sudo without a password. cwd defaults to /home/bia. Always work in /home/bia. Do not invent the output. User files are /home/bia/files. The model is /home/bia/model. MCPs are /home/bia/MCPs. Telegram is /home/bia/files/telegram. WhatsApp is /home/bia/files/whatsapp. telegram_ban / whatsapp_ban ban a chat: its messages are not delivered and it is answered with I've banned you. telegram_unban / whatsapp_unban let them through again. There is no /mnt/internal and boot does not switch disks. Open mcp/carry for moving this computer. carry_fold saves all of /home/bia as a gzip backup named name.unfold-<bytes>.tar.gz. It refuses when free space is below the size of /home/bia; ask the user, then pass force=true only after they confirm. carry_unfold restores it and reboots. An older backup whose name has no unfolded size still restores after the user confirms; ask the user, then pass force=true only after they confirm. Do the same when free space is below that size. If packages did not install or MCP binaries were not copied, repair them yourself; ask the user first only before a destructive step. carry_land waits for another computer. carry_fly sends the backup there. carry_confirm is the 6-digit code after the user says both screens match. carry_morph is only on a flash boot: it takes the agent from the internal disk. carry_update (or console /update) updates this OS from the other attached bia system (flash when on the disk, disk when on the flash) through the offline rescue image, then unfolds the agent; only when the user asks; its progress is in /bia-update/update.log (tail it, do not probe). carry_rescue from flash repairs BIADISK. carry_to_disk copies onto a disk you pass then reboots into it (on legacy BIOS it powers off: say to remove the stick and power on first); carry_to_flash copies onto a stick then shut down; use bash lsblk to choose. Pass disk=/dev/… (lsblk). On Windows also pass partition or shrink_from+size_mib or size_mib; wipe=true erases Windows. carry_morph requires disk=. Wired DHCP is already up on eth0. Wi-Fi, apt-get, lsblk/fdisk/mkfs, lscpu/free and /sys are bash (apt-get install pciutils usbutils for lspci and lsusb). Clock, timezone and NTP are bash too: date and timedatectl. Envelope timestamps stay UTC. Web chat is / on the HTTP port; /gui forwards VNC :5900. Apps belong on that display.",
    },
    Branch {
        id: "talk",
        blurb: "Speak to people and keep contacts.",
        guide: "say(identity, channel, message, reply_to, files, buttons, poll, answered, edit). say with edit=<message id> replaces the text of a message you sent instead of sending a new one (web, telegram, whatsapp, messenger, instagram; ids are in say results). Human messages stay open until you close them with answered=[envelope ids] on say or react; do that only when the task is done, and post a plain status say while it is still running. say with answered and no message only closes. An unanswered: notice lists open messages. Always reply on the channel of the inbound envelope, copied exactly: physical screen → console; web → web for the person's main chat, web:<chat> for every other chat (never shorten web:<chat> to web), with identity = that person; telegram:<chat_id>; whatsapp:<phone>; messenger:<chat_id>; instagram:<chat_id>. On web, telegram, whatsapp, messenger and instagram a message that answers another starts with [reply to <id> from <who>: \"…\"] (from you = one of your own messages; , quoting: \"…\" = the part they picked): that is what they mean, read it before answering. Web chats work like Telegram: inbound text may also start with [forwarded from …], ends with files: (uploads under /home/bia/files/inbox/web/<identity>/, voice messages marked), and user actions arrive as [edited <id>], [deleted] <ids>, [reaction] <emoji> on <id>, [button] \"text\" on <id>, [vote] … in poll <id>. You can react, edit (say with edit), delete_messages, pin_message, close_poll and read chat_history there; react, edit and delete_messages also work on telegram/whatsapp/messenger/instagram. On telegram/whatsapp/messenger/instagram an edit arrives as [edited <id>] sent <time>, edited <time> with was: (old text) and now: (new text); the original message stays earlier in your history; answer only if the change matters. Shared places arrive as [location] / [live location] / [contact] lines; a live location then updates silently (telegram_live_location / whatsapp_live_location tell where they are now) and [live location ended] says when sharing stopped (no reply needed). Omitting identity/channel uses the last human inbound this turn. Sends and returns immediately — their next message arrives later as a normal inbound envelope (do not block waiting). The console is the physical screen; web, telegram, whatsapp, messenger and instagram are buses of their MCPs (web calls: web_call_start / web_call_hangup in mcp/web) and reconnect when you boot; a channel is one conversation on a bus. telegram_ban / whatsapp_ban / messenger_ban / instagram_ban ban a chat: its messages are not delivered, it is answered with I've banned you, and its calls are auto-rejected. The *_unban tools let them through again. Voice: [incoming call] → *_call_accept/reject; [speech] is spoken input (may arrive while you are still speaking — do not call_say/say again until [you finished speaking] unless you intentionally interrupt); say on an active call speaks via TTS and a new speak cancels the current one. WhatsApp login is QR-only (whatsapp_login); the QR PNG is at /home/bia/files/whatsapp/qr.png until auth completes. Messenger and Instagram take the login from the session in bia's Chrome (messenger_login / instagram_login; sign in there first); they have no calls. addressbook_* is your contacts (trust normal or root). channels lists buses.",
    },
    Branch {
        id: "calendar",
        blurb: "Tasks and reminders.",
        guide: "schedule_create(title, body markdown, at or cron, kind=task|reminder, repeat, command, user, match). at is RFC 3339 UTC, YYYY-MM-DD HH:MM, a weekday like monday 09:00, or relative in 10m. cron is five crontab fields in the machine timezone (minute hour day-of-month month day-of-week), for example 0 9 * * 1-5, and replaces at and repeat. repeat is none, hourly, daily, weekly, monthly, yearly, weekdays, weekends, or a duration (30m, 2h). command runs as user when due (default bia; root and other accounts can be named). If you write a script or program for command, run it first with real or sample data and confirm it works before you schedule it. match is a regular expression: if set, no system|schedule message is sent unless the program output matches, and the message then includes the task text and the log path. Due text items arrive as system|schedule (issued is the scheduled time), including ones missed while the machine was off. schedule_list, schedule_update (enabled=false pauses), schedule_delete.",
    },
    Branch {
        id: "memory",
        blurb: "Reflections of the live window.",
        guide: "memory_list(from, to, category, depth), memory_get(id), memory_tree(from, to), memory_messages, memory_categories, memory_reflect (compact the live window now). from/to are RFC 3339 UTC or YYYY-MM-DD.",
    },
    Branch {
        id: "jobs",
        blurb: "In-flight bash and MCP work.",
        guide: "bash and MCP calls are jobs, not calendar tasks. The finished result arrives as system|job:<id>|… (issued is when the work finished). A body that starts with stale? is a mid-job notice (bash still running after 30s with no PTY output) — not the result; use job_logs, job_input(id, text) to type into the PTY (include a trailing newline for Enter), Ctrl-C via job_input(id, \"\\u0003\") to interrupt, or job_kill(id) to stop it hard. list_jobs, job_logs(id, tail), job_input, job_kill(id). You may talk to people and inspect jobs while one runs. Do not invent the result. Do not call a dependent tool until the finished envelope arrives (exit …). Builtins other than bash return in the same turn.",
    },
    Branch {
        id: "machine",
        blurb: "Context window, reboot, poweroff, forget.",
        guide: "set_model_config shows or changes how you run: context (tokens, reloads you; on failure the previous window is restored), max_tool_rounds (rounds in a row before a status), and the local model's sampling (temperature, top_p, top_k, min_p, repeat_penalty, repeat_last_n; cloud FMM models keep their own). Only the fields you pass change. reboot and poweroff only when root asks; the conversation is saved and resumes. forget only when root asks to start over. Act with tools first; do not ask root for something you can look up. Changing network, packages, files, disks, or your own logins needs no extra confirmation.",
    },
];

fn brief(s: &str) -> String {
    let s = s.split('\n').next().unwrap_or(s).trim();
    let cut = s.find(". ").map(|i| i + 1).unwrap_or(s.len());
    let s = &s[..cut];
    if s.chars().count() <= 140 {
        return s.to_string();
    }
    let end = s.char_indices().nth(137).map(|(i, _)| i).unwrap_or(s.len());
    format!("{}…", &s[..end])
}

fn query_words(q: &str) -> Vec<String> {
    let words: Vec<String> = q
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 2)
        .map(|s| s.to_string())
        .collect();
    if words.is_empty() {
        vec![q.trim().to_lowercase()]
    } else {
        words
    }
}

fn blob_has(blob: &str, words: &[String]) -> bool {
    let lowered = blob.to_lowercase();
    let tokens: Vec<&str> = lowered
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    words.iter().any(|w| {
        if w.len() <= 3 {
            tokens.contains(&w.as_str())
        } else {
            lowered.contains(w.as_str())
        }
    })
}

impl Tools {
    pub fn new(files_dir: &Path, con: Arc<Console>) -> Tools {
        let state = files_dir.join("state");
        let schedules = Arc::new(Mutex::new(crate::schedules::Store::load(
            &state.join("schedules.json"),
        )));
        crate::schedules::start(schedules.clone(), con.clone());
        let (outbox, outbox_lost) = crate::outbox::Outbox::load(&state, con.clone());
        let mut t = Tools {
            specs: vec![],
            kinds: vec![],
            groups: vec![],
            enabled: Default::default(),
            schemas_sent: Default::default(),
            sent_mcp: std::fs::read(state.join(MCP_BLOCKS))
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default(),
            reload_notes: vec![],
            starting: HashMap::new(),
            held: vec![],
            servers: vec![],
            infos: vec![],
            catalog: vec![],
            loaded: Default::default(),
            files_dir: files_dir.into(),
            pending_exit: std::cell::Cell::new(None),
            pending_forget: std::cell::Cell::new(false),
            pending_reflect: std::cell::Cell::new(false),
            con,
            addressbook: crate::addressbook::AddressBook::load(&state.join("addressbook.json")),
            buses: crate::buses::Buses::load(&state.join("buses.json")),
            schedules,
            jobs: Arc::new(Mutex::new(crate::jobs::Store::load(&state))),
            outbox,
            outbox_lost,
            fired_job: std::cell::Cell::new(false),
            reply_to: std::cell::RefCell::new(None),
            last_on: std::cell::RefCell::new(HashMap::new()),
            unanswered: crate::unanswered::Set::load(&state),
            mcp_arch: std::env::consts::ARCH.into(),
            mcp_env: vec![],
            mcp_log_dir: files_dir.join("logs"),
            recent_calls: Default::default(),
        };
        let obj = |props: Value, req: &[&str]| json!({"type": "object", "properties": props, "required": req});
        t.add_grouped("discover", "discover", "The only tool in the list. An empty path lists the tree. path opens a branch (shell, talk, calendar, memory, jobs, machine, or mcp/<server>). enable=true appends that branch's function schemas once. Then call each function by the name in the schema, for example bash(...) or system_fmm_configure(...). Do not enable the same path again. q searches. call and arguments also run a function. The list never grows.",
            obj(json!({
                "path": {"type": "string", "description": "branch, e.g. shell or mcp/telegram or mcp/whatsapp; empty lists the top"},
                "enable": {"type": "boolean", "description": "append this branch's function schemas"},
                "restart": {"type": "boolean", "description": "mcp/<server> only: start a new process of it from disk (after you updated or edited it); its functions are sent again only if they changed"},
                "q": {"type": "string", "description": "search names, descriptions and keywords"},
                "call": {"type": "string", "description": "function to run, e.g. bash or system_fmm_configure"},
                "arguments": {"type": "object", "description": "arguments for call", "additionalProperties": true}
            }), &[]));
        t.add_builtin("use_mcp", "Load an MCP server's functions so they become callable. Required before telegram_login / whatsapp_login / system_fmm_configure / etc. If a capability is listed in the system prompt, call this — do not say you lack the tool. MCPs live in /home/bia/MCPs. A running server is not restarted when its files change: after updating or editing one, call discover(path=\"mcp/<name>\", restart=true); its definitions come again only if they changed. Definitions are appended to the conversation; the frozen tool list is not rewritten. Then call those functions with invoke(name, arguments).",
            obj(json!({"name": {"type": "string", "description": "MCP server name, e.g. system"}}), &["name"]));
        t.add_builtin("invoke", "Call a loaded MCP function (or any tool) by name. The frozen tool list never grows — after use_mcp, use this instead of expecting a new function in the API tool list. name is the prefixed function (e.g. telegram_login, whatsapp_login, system_fmm_status).",
            obj(json!({
                "name": {"type": "string", "description": "exact tool name, e.g. system_fmm_status"},
                "arguments": {"type": "object", "description": "arguments for that function", "additionalProperties": true}
            }), &["name"]));
        t.add_grouped("shell", "bash", "Run a shell command on this machine as a PTY job (finished result arrives as system|job:<id>|…). Interactive prompts can be answered with job_input. A stale? notice means no output for 30s while still running — not the result. Default user is root; pass user='bia' for the unprivileged account. cwd defaults to /home/bia. Always work in /home/bia. Use this for ls/cat/tee/mkdir, lscpu/free/lsblk, fdisk/mkfs.ext4/e2label, apt-get, ifup, wpa_supplicant, docker, installing a display stack, starting x11vnc on :5900, compiling, anything the OS can do. Do not invent the output — wait for the finished job envelope (exit …).",
            obj(json!({
                "command": {"type": "string", "description": "passed to sh -lc"},
                "user": {"type": "string", "description": "root (default) or bia"},
                "cwd": {"type": "string", "description": "working directory (default /home/bia)"},
                "timeout_secs": {"type": "integer", "description": "kill after this many seconds (default 600, 0 = no limit)"}
            }), &["command"]));
        t.add_grouped("talk", "say", "Send a message to an identity on a channel and return immediately (the result names the new message id). On telegram/whatsapp/messenger/instagram every message gets a local id o<n>; a send still uploading after a few seconds (big files) returns \"queued as o<n>\" and its outcome arrives later ([sent o<n> → …] with the platform ids, or [send failed o<n> …] as system|outbox). o<n> works as a message id for edit, react and delete_messages right away — they wait for that send; o<n>.2 is its second message. cancel=o<n> stops a send that is still queued or uploading; list_jobs shows sends in flight. Reply on the channel of the inbound envelope, copied exactly (web:<chat> is not web; physical screen → console, telegram:<id>, whatsapp:<phone>). Omitting identity/channel uses the last human inbound this turn. Their reply arrives later as a normal inbound message — do not wait inside this tool. answered lists inbound envelope ids whose task is done; leave it out for a status update while still working. With answered and no message, files or poll, nothing is sent and the ids are just closed. files are paths under /home/bia/files (or absolute) sent as attachments (pictures show as photos). On web chats you can also quote a message with reply_to (the envelope id), attach inline buttons (pressing one arrives as [button] \"text\" on <id>) and send a poll (votes arrive as [vote]). edit=<message id> replaces the text of a message you sent earlier (the id from that say result) instead of sending a new one — on web, telegram and whatsapp chats (WhatsApp allows ~15 minutes); text only, no files/voice/poll. voice=true (with lang) sends message as a voice message instead of text on web, telegram and whatsapp chats — only when the user asks for a voice reply; voice messages they send arrive transcribed as [speech <lang>]. If a Telegram or WhatsApp voice call is active on that chat, the message is also spoken into the call: then pass lang (the language of the text, e.g. en, ru; other-language parts inline as <lang=xx>…</lang>), otherwise it is only sent as text. On a call: wait for [you finished speaking] before speaking again; another say/call_say while still speaking cancels the current utterance ([voice cancelled]). [speech <lang>] may arrive mid-utterance — listen, but do not speak over yourself unless you intentionally interrupt.",
            obj(json!({
                "identity": {"type": "string", "description": "recipient identity, e.g. root; defaults to last human inbound"},
                "channel": {"type": "string", "description": "channel, e.g. web, web:<chat> or console; defaults to last human inbound"},
                "message": {"type": "string", "description": "markdown text; optional when files or poll are given"},
                "lang": {"type": "string", "description": "language of message (en, ru, es-MX…) for voice=true and for speaking it in an active call; defaults to the person's preferred language (addressbook)"},
                "voice": {"type": "boolean", "description": "send message as a voice message (spoken with the voice of lang) on web, telegram, whatsapp, messenger or instagram; only when the user asks for a voice reply"},
                "reply_to": {"type": "string", "description": "web: id of the message to quote"},
                "new_chat": {"type": "boolean", "description": "web: true opens a new chat named by channel web:<chat>; a chat that does not exist is refused otherwise"},
                "edit": {"type": "string", "description": "id of a message you sent (from its say result): replace its text with message instead of sending a new one (web, telegram, whatsapp, messenger, instagram)"},
                "cancel": {"type": "string", "description": "telegram/whatsapp/messenger/instagram: local id o<n> of a send that is still queued or uploading; stops it and sends nothing else"},
                "answered": {"type": "array", "items": {"type": "string"}, "description": "envelope ids of inbound messages whose task is now done (closes them); omit while still working"},
                "files": {"type": "array", "items": {"type": "string"}, "description": "attachments: paths under /home/bia/files (web, telegram, whatsapp, messenger, instagram)"},
                "buttons": {"type": "array", "description": "web: inline keyboard, rows of buttons; a button is a string or {text, data, url}", "items": {"type": "array", "items": {}}},
                "poll": {"type": "object", "description": "web: {question, options: [strings], multi: bool}", "properties": {
                    "question": {"type": "string"},
                    "options": {"type": "array", "items": {"type": "string"}},
                    "multi": {"type": "boolean"}
                }}
            }), &[]));
        t.add_grouped("talk", "react", "React to a message with an emoji on web, telegram, whatsapp, messenger or instagram channels (message_id is the envelope id). Empty emoji removes your reaction. answered lists inbound envelope ids whose task is done (a reaction can be the whole answer).",
            obj(json!({
                "channel": {"type": "string", "description": "e.g. web, web:<chat>, telegram:<id>, whatsapp:<phone>, messenger:<id>, instagram:<id>; defaults to last human inbound"},
                "identity": {"type": "string", "description": "web only: chat owner identity; defaults to last human inbound"},
                "message_id": {"type": "string"},
                "emoji": {"type": "string", "description": "e.g. 👍; empty removes"},
                "answered": {"type": "array", "items": {"type": "string"}, "description": "envelope ids of inbound messages whose task is now done (closes them); omit while still working"}
            }), &["message_id"]));
        t.add_grouped("talk", "delete_messages", "Delete messages on a web, telegram, whatsapp, messenger or instagram chat. They are deleted for everyone (revoke=true, the default); revoke=false deletes them only on bia's side (telegram/whatsapp/messenger/instagram).",
            obj(json!({
                "channel": {"type": "string", "description": "web, web:<chat>, telegram:<id>, whatsapp:<phone>, messenger:<id>, instagram:<id>; defaults to last human inbound"},
                "identity": {"type": "string", "description": "web only: chat owner identity; defaults to last human inbound"},
                "message_ids": {"type": "array", "items": {"type": "string"}},
                "revoke": {"type": "boolean", "description": "telegram/whatsapp/messenger/instagram: true (default) deletes for the other person too; false only on this account"}
            }), &["message_ids"]));
        t.add_grouped("talk", "pin_message", "Pin (or unpin with on=false) a message at the top of a web chat.",
            obj(json!({
                "channel": {"type": "string", "description": "web or web:<chat>; defaults to last human inbound"},
                "identity": {"type": "string", "description": "chat owner identity; defaults to last human inbound"},
                "message_id": {"type": "string"},
                "on": {"type": "boolean", "description": "default true"}
            }), &["message_id"]));
        t.add_grouped("talk", "close_poll", "Stop a web poll you sent from taking votes; returns the tally.",
            obj(json!({
                "channel": {"type": "string", "description": "web or web:<chat>; defaults to last human inbound"},
                "identity": {"type": "string", "description": "chat owner identity; defaults to last human inbound"},
                "message_id": {"type": "string"}
            }), &["message_id"]));
        t.add_grouped("talk", "chat_history", "Recent messages of a web chat, newest first: id, time, sender, reply/edit/pin/reaction/file/poll marks and text.",
            obj(json!({
                "channel": {"type": "string", "description": "web or web:<chat>; defaults to last human inbound"},
                "identity": {"type": "string", "description": "chat owner identity; defaults to last human inbound"},
                "limit": {"type": "integer", "description": "default 30, max 100"}
            }), &[]));
        t.add_grouped("machine", "set_model_config", "Show or change how your brain runs: context tokens, tool rounds before a pause, local model sampling (temperature, top_p…). Only the fields you pass change; none shows the current settings. Changes are kept across restarts.",
            obj(json!({
                "context": {"type": "integer", "description": "context window in tokens, 512–2000000: reloads the brain (local GGUF: new KV cache, clamped to the model's trained length; FMM: budget in fmm.json); on failure the previous window is restored"},
                "max_tool_rounds": {"type": "integer", "description": "tool rounds in a row before you post a status and go on, 4–256 (bash, MCP functions and say do not count)"},
                "temperature": {"type": "number", "description": "local model, 0–2"},
                "top_p": {"type": "number", "description": "local model, 0–1"},
                "top_k": {"type": "integer", "description": "local model, 0 (off)–1000"},
                "min_p": {"type": "number", "description": "local model, 0–1"},
                "repeat_penalty": {"type": "number", "description": "local model, 0.5–2"},
                "repeat_last_n": {"type": "integer", "description": "local model, -1 (whole context)–4096"}
            }), &[]));
        t.add_grouped("memory", "memory_list", "List reflections overlapping a time period. from/to are RFC 3339 UTC or YYYY-MM-DD. Optional category (summary, facts, or any category you invented) and depth (0=raw-period, higher=reflections of reflections).",
            obj(json!({
                "from": {"type": "string"},
                "to": {"type": "string"},
                "category": {"type": "string"},
                "depth": {"type": "integer"}
            }), &[]));
        t.add_grouped("memory", "memory_get", "Read one reflection by id (text, period, category, siblings in the same bundle, children).",
            obj(json!({"id": {"type": "string"}}), &["id"]));
        t.add_grouped(
            "memory",
            "memory_tree",
            "Show the reflection tree for a period (depth indented).",
            obj(
                json!({"from": {"type": "string"}, "to": {"type": "string"}}),
                &[],
            ),
        );
        t.add_grouped("memory", "memory_messages", "Dive to original journal messages. Pass a reflection id, or from+to.",
            obj(json!({"id": {"type": "string"}, "from": {"type": "string"}, "to": {"type": "string"}}), &[]));
        t.add_grouped(
            "memory",
            "memory_categories",
            "List reflection categories you have invented so far, with counts.",
            obj(json!({}), &[]),
        );
        t.add_grouped("memory", "memory_reflect", "Request an immediate reflection of the current live window (same as automatic compact when context is full). Writes 2+ categorized memories for the period (summary, facts, …).",
            obj(json!({}), &[]));
        t.add_grouped("machine", "reboot", "Reboot. Your conversation is saved and resumes after boot. Only call when root asked you to reboot.", obj(json!({}), &[]));
        t.add_grouped("machine", "poweroff", "Power yourself off. Your conversation is saved and resumes at the next boot. Only call when root asked you to shut down.", obj(json!({}), &[]));
        t.add_grouped("machine", "forget", "Forget the whole conversation after this turn: history, context and the saved state are erased and you start from scratch. Only call when root asks for it (e.g. 'forget everything', 'start over').", obj(json!({}), &[]));
        t.add_grouped(
            "talk",
            "channels",
            "List known buses and channels (console, telegram chats, whatsapp chats, …).",
            obj(json!({}), &[]),
        );
        t.add_grouped(
            "talk",
            "addressbook_list",
            "List contacts in the addressbook (name, identifier, trust, linked channels).",
            obj(json!({}), &[]),
        );
        t.add_grouped("talk", "addressbook_link", "Hard-link a channel (as listed by channels: telegram:123, whatsapp:15551234567) to a contact: it leaves any other contact, and an auto-created contact left empty is merged in (its language carries over). Use it when you learn that a channel is a person you know (e.g. root's own Telegram). Web and console chats belong to the contact named like the client identity.",
            obj(json!({
                "id": {"type": "string", "description": "contact id"},
                "channel": {"type": "string"}
            }), &["id", "channel"]));
        t.add_grouped(
            "talk",
            "addressbook_unlink",
            "Remove a channel from its contact.",
            obj(json!({"channel": {"type": "string"}}), &["channel"]),
        );
        t.add_grouped("talk", "addressbook_upsert", "Create or update a contact. trust is normal or root. channels is a list of channel ids (e.g. telegram:123, whatsapp:15551234567). lang is their preferred language (en, ru, …): calls and voice messages with them start in it, and it is what speech uses when you leave lang out; it is also learned from their speech unless set here (empty clears).",
            obj(json!({
                "id": {"type": "string"},
                "name": {"type": "string"},
                "identifier": {"type": "string", "description": "phone, @username, email, …"},
                "trust": {"type": "string", "description": "normal or root"},
                "channels": {"type": "array", "items": {"type": "string"}},
                "notes": {"type": "string"},
                "lang": {"type": "string", "description": "preferred language code (en, ru, …)"}
            }), &["id"]));
        t.add_grouped(
            "talk",
            "addressbook_remove",
            "Remove a contact from the addressbook.",
            obj(json!({"id": {"type": "string"}}), &["id"]),
        );
        t.add_grouped("calendar", "schedule_create", "Schedule a task or reminder. body is markdown. at is RFC 3339 UTC, YYYY-MM-DD HH:MM, monday 09:00, or in 10m. cron is five crontab fields in the machine timezone (minute hour day-of-month month day-of-week), for example 0 9 * * 1-5; it replaces at and repeat. kind is task or reminder. repeat is none, hourly, daily, weekly, monthly, yearly, weekdays, weekends, or a duration (30m, 2h). command is a program to run when the item is due, as user (default bia). If you write a script or program for command, run it first with real or sample data and confirm it works before you call schedule_create. match is a regular expression: if set, no system|schedule message is sent unless the program output matches. The message includes the task text and the path of that run's log. Without a command, the item arrives as a system|schedule message when due, including missed ones after boot.",
            obj(json!({
                "title": {"type": "string"},
                "body": {"type": "string", "description": "markdown task text"},
                "at": {"type": "string", "description": "when it is due; omit when cron is set"},
                "cron": {"type": "string", "description": "minute hour day-of-month month day-of-week, local time"},
                "kind": {"type": "string", "description": "task or reminder"},
                "repeat": {"type": "string", "description": "none, daily, weekly, …"},
                "command": {"type": "string", "description": "program to run when due"},
                "user": {"type": "string", "description": "unix account (default bia)"},
                "match": {"type": "string", "description": "regular expression; stay silent unless the output matches"}
            }), &["title"]));
        t.add_grouped(
            "calendar",
            "schedule_list",
            "List scheduled tasks and reminders (id, kind, repeat, next due, title).",
            obj(json!({}), &[]),
        );
        t.add_grouped(
            "calendar", "schedule_update",
            "Change a schedule. Pass only the fields to change. enabled=false pauses it. An empty cron, command, or match clears that field. user defaults to bia when a command is set. If you write a new script or program for command, run it first with real or sample data and confirm it works before you call schedule_update.",
            obj(
                json!({
                    "id": {"type": "string"},
                    "title": {"type": "string"},
                    "body": {"type": "string"},
                    "at": {"type": "string"},
                    "cron": {"type": "string"},
                    "kind": {"type": "string"},
                    "repeat": {"type": "string"},
                    "command": {"type": "string"},
                    "user": {"type": "string"},
                    "match": {"type": "string", "description": "regular expression; empty clears it"},
                    "enabled": {"type": "boolean"}
                }),
                &["id"],
            ),
        );
        t.add_grouped(
            "calendar",
            "schedule_delete",
            "Remove a scheduled task or reminder.",
            obj(json!({"id": {"type": "string"}}), &["id"]),
        );
        t.add_grouped("jobs", "list_jobs", "List in-flight MCP jobs (running) and the last 10 completed. Jobs are not calendar tasks. Each model-facing MCP call is a job; the result arrives as system|job:<id>|…",
            obj(json!({}), &[]));
        t.add_grouped("jobs", "job_logs", "Read the log of a job (progress lines). id is the short job id from the stub or list_jobs.",
            obj(json!({
                "id": {"type": "string"},
                "tail": {"type": "integer", "description": "last N lines (default 80)"}
            }), &["id"]));
        t.add_grouped(
            "jobs",
            "job_input",
            "Write text to a running bash job's PTY (interactive prompts). Include a trailing newline for Enter. For Y/N send y\\n or n\\n. MCP jobs have no PTY.",
            obj(
                json!({
                    "id": {"type": "string"},
                    "text": {"type": "string", "description": "bytes to write; include \\n for Enter; \\u0003 is Ctrl-C (interrupt), \\u0004 is Ctrl-D (end of input)"}
                }),
                &["id", "text"],
            ),
        );
        t.add_grouped(
            "jobs",
            "job_kill",
            "Kill a running job now. bash: SIGKILL to its whole process group and session. MCP job: abandoned at once (the MCP is asked to cancel; a late result is dropped). The result arrives as system|job:<id>|… with killed. To interrupt a bash job gently instead, send Ctrl-C with job_input(id, \"\\u0003\").",
            obj(json!({"id": {"type": "string"}}), &["id"]),
        );
        t
    }

    pub fn set_mcp_spawn(&mut self, arch: &str, env: Vec<(String, String)>, log_dir: PathBuf) {
        self.mcp_arch = arch.into();
        self.mcp_env = env;
        self.mcp_log_dir = log_dir;
    }

    fn add_builtin(&mut self, name: &str, desc: &str, params: Value) {
        self.add_grouped("", name, desc, params);
    }

    fn add_grouped(&mut self, group: &str, name: &str, desc: &str, params: Value) {
        self.specs.push(ToolSpec {
            name: name.into(),
            description: desc.into(),
            parameters: params,
        });
        self.kinds.push(Kind::Builtin);
        self.groups.push(group.into());
    }

    /// After a restart: the server's functions and description as they are now. When the
    /// model already had this MCP's block and its hash changed, returns the new block to
    /// send (as if it had run discover enable=false, then enable=true); the same block is
    /// never sent twice.
    fn reload_server(&mut self, idx: usize, m: &crate::mcp::Manifest) -> Result<Option<String>> {
        let list = self.servers[idx].list_tools()?;
        let name = self.servers[idx].name.clone();
        let keep: Vec<bool> = self.kinds.iter().map(|k| !matches!(k, Kind::Mcp { server, .. } if *server == idx)).collect();
        let mut it = keep.iter();
        self.specs.retain(|_| *it.next().unwrap());
        let mut it = keep.iter();
        self.kinds.retain(|_| *it.next().unwrap());
        let mut it = keep.iter();
        self.groups.retain(|_| *it.next().unwrap());
        self.infos[idx] = server_info(m, &list);
        self.push_tools(idx, &name, list);
        if self.schemas_sent.contains(&format!("mcp/{name}")) {
            return Ok(self.changed_block(idx, true));
        }
        Ok(None)
    }

    /// The model has this MCP's block from earlier (this run, or a resumed conversation):
    /// the new block when it hashes differently, else None. `unknown_sends`: also when no
    /// hash was kept (a resume before this was recorded passes false and only records it).
    fn changed_block(&mut self, idx: usize, unknown_sends: bool) -> Option<String> {
        let name = self.servers[idx].name.clone();
        let block = self.mcp_text(idx, true);
        let hash = block_hash(&block);
        let send = match self.sent_mcp.get(&name) {
            Some(h) => *h != hash,
            None => unknown_sends,
        };
        if self.sent_mcp.get(&name) != Some(&hash) {
            self.remember_block(&name, hash);
        }
        send.then(|| format!(
            "mcp/{name} changed (as discover enable=false, then enable=true). Its functions now (use these, not the earlier ones):\n{block}"
        ))
    }

    /// discover(path="mcp/<name>", restart=true): a new process from disk, its functions
    /// read again; the model gets the block only when it differs from the one it has.
    fn restart_mcp(&mut self, name: &str) -> Result<String> {
        let Some(i) = self.server_index(name) else {
            // not running: as enable=true
            self.refresh_named(name)?;
            let idx = self.server_index(name).ok_or_else(|| anyhow::anyhow!("unknown MCP {name}; discover(path=\"mcp\") lists servers"))?;
            return self.enable_mcp(name, idx);
        };
        let (dir, m) = self.find_on_disk(name).ok_or_else(|| anyhow::anyhow!("mcp/{name} is no longer on disk"))?;
        if self.servers[i].dir != dir {
            self.servers[i].relocate(dir)?;
        } else {
            self.servers[i].respawn()?;
        }
        self.loaded.insert(i);
        self.schemas_sent.insert(format!("mcp/{name}"));
        let names = self.mcp_fn_names(i);
        Ok(match self.reload_server(i, &m)? {
            Some(block) => format!("restarted mcp/{name}. {block}"),
            None => format!("restarted mcp/{name}; its functions are unchanged. Call by name: {}", names.join(", ")),
        })
    }

    fn remember_block(&mut self, name: &str, hash: String) {
        self.sent_mcp.insert(name.to_string(), hash);
        let state = self.files_dir.join("state");
        let _ = std::fs::create_dir_all(&state);
        let tmp = state.join(format!("{MCP_BLOCKS}.tmp"));
        if std::fs::write(&tmp, serde_json::to_vec_pretty(&self.sent_mcp).unwrap_or_default()).is_ok() {
            let _ = std::fs::rename(&tmp, state.join(MCP_BLOCKS));
        }
    }

    /// Start an MCP server in the background as a job (boot: no waiting for slow ones,
    /// such as a messenger restoring its session over the network). The result arrives as
    /// system|job:<id>: started (with its boot step) or why not, plus its block when that
    /// changed since the model last had it. A call to it meanwhile waits for the start.
    pub fn start_job(&mut self, name: &str) -> Result<String> {
        if let Some(msg) = self.start_status(name) {
            return Ok(format!("already: {msg}"));
        }
        if self.server_index(name).is_some() {
            return Ok(format!("mcp/{name} is already running"));
        }
        let (dir, m) = self.find_on_disk(name).ok_or_else(|| anyhow::anyhow!("unknown MCP {name}"))?;
        let job = {
            let mut st = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
            st.alloc("mcp_start", name, "").map_err(|e| anyhow::anyhow!(e))?
        };
        let jid = job.id.clone();
        let (arch, env, log_dir) = (self.mcp_arch.clone(), self.mcp_env.clone(), self.mcp_log_dir.clone());
        let (con, jobs, srv_name) = (self.con.clone(), self.jobs.clone(), name.to_string());
        let handle = std::thread::Builder::new().name(format!("mcp-start-{name}")).spawn(move || {
            let note = |l: &str| jobs.lock().unwrap_or_else(|e| e.into_inner()).append_log(&jid, l);
            note(&format!("starting mcp/{srv_name} from {}", dir.display()));
            let r = (|| -> Result<(McpServer, Vec<McpTool>, String)> {
                let mut srv = McpServer::spawn(&dir, &m, &arch, &env, &log_dir, con.clone(), jobs.clone())?;
                note("process up; reading its functions");
                let list = srv.list_tools()?;
                note(&format!("{} functions", list.len()));
                let mut boot = String::new();
                if let Some(bt) = m.boot_tool.clone().filter(|bt| list.iter().any(|t| &t.name == bt)) {
                    note(&format!("running its boot step {bt}"));
                    boot = match srv.call(&bt, json!({}), &mut |_| {}) {
                        Ok((t, _)) => format!("{bt}: {}", t.trim().replace('\n', "; ")),
                        Err(e) => format!("{bt}: {e:#}"),
                    };
                    note(&boot);
                }
                Ok((srv, list, boot))
            })();
            let (state, body, started) = match r {
                Ok((srv, list, boot)) => (
                    "ok",
                    format!("ok\nmcp/{srv_name} started: {} functions, callable now{}", list.len(), if boot.is_empty() { String::new() } else { format!("\n{boot}") }),
                    Some(Started { srv, list, m }),
                ),
                Err(e) => (
                    "error",
                    format!("error\nmcp/{srv_name} did not start: {e:#} (its log: {}/mcp-{srv_name}.log). discover(path=\"mcp/{srv_name}\", enable=true) tries again.", log_dir.display()),
                    None,
                ),
            };
            let won = jobs.lock().unwrap_or_else(|e| e.into_inner()).finish(&jid, state, &body);
            if !won {
                // job_kill abandoned it while it started: do not keep the process
                if let Some(mut s) = started {
                    s.srv.kill();
                }
                return None;
            }
            con.push_inbound(crate::jobs::inbound(&jid, &body));
            started
        })?;
        let (con, jobs, jid, srv_name) = (self.con.clone(), self.jobs.clone(), job.id.clone(), name.to_string());
        let _ = std::thread::Builder::new().name(format!("mcp-start-stale-{name}")).spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(START_STALE_SECS));
            let last = {
                let st = jobs.lock().unwrap_or_else(|e| e.into_inner());
                if st.get(&jid).map(|j| j.state.as_str()) != Some("running") {
                    return;
                }
                st.logs(&jid, 1).unwrap_or_default()
            };
            con.push_inbound(crate::jobs::inbound(&jid, &format!(
                "stale?\nmcp_start job:{jid} (mcp/{srv_name}) is still starting after {START_STALE_SECS}s; last step: {}.\nThis is not the job result. Its functions are not callable until it finishes (a call to them waits). job_logs(id) shows its progress, job_kill(id) gives it up. It takes no input.",
                last.trim()
            )));
        });
        self.starting.insert(name.to_string(), Starting { job: job.id.clone(), handle });
        Ok(format!("mcp/{name} is starting as job:{}", job.id))
    }

    /// Join a background start (blocking while it runs) and attach the server; the model's
    /// copy of its block counts as the one from before (a resumed conversation): the new
    /// block when it changed.
    fn finish_start(&mut self, name: &str) -> Option<String> {
        let st = self.starting.remove(name)?;
        let Some(Started { srv, list, m }) = st.handle.join().ok().flatten() else {
            self.drop_held(name, &format!("mcp/{name} did not start (job:{})", st.job));
            return None;
        };
        let info = server_info(&m, &list);
        self.add_server(srv, list, info);
        let idx = self.server_index(name)?;
        self.loaded.insert(idx);
        self.schemas_sent.insert(format!("mcp/{name}"));
        let block = self.changed_block(idx, false);
        self.flush_held(name);
        block
    }

    /// Its MCP is up: the held sends go into the outbox, in order (outcomes as for any
    /// slow send: a [sent …] note, or [send failed …] as system|outbox).
    fn flush_held(&mut self, name: &str) {
        let (mine, rest): (Vec<Held>, Vec<Held>) = std::mem::take(&mut self.held).into_iter().partition(|h| h.mcp == name);
        self.held = rest;
        for h in mine {
            let label = format!("{}:{}", h.mcp, h.chat);
            match self.outbox_submit(&h.mcp, &h.chat, h.op, h.bytes, &h.what, std::time::Duration::ZERO) {
                Ok(crate::outbox::Outcome::Done(_, Err(e))) => self.outbox.report_failed(&format!("[send failed → {label}] {e}")),
                Err(e) => self.outbox.report_failed(&format!("[send failed → {label}] {e:#}")),
                Ok(_) => {}
            }
        }
    }

    /// Its MCP will not come up: every held send fails, each as system|outbox.
    fn drop_held(&mut self, name: &str, why: &str) {
        let (mine, rest): (Vec<Held>, Vec<Held>) = std::mem::take(&mut self.held).into_iter().partition(|h| h.mcp == name);
        self.held = rest;
        for h in mine {
            self.outbox.report_failed(&format!("[send failed → {}:{}] {why}; not sent: {}", h.mcp, h.chat, if h.what.is_empty() { "an edit, reaction or delete" } else { &h.what }));
        }
    }

    /// Before a model turn: attach the servers whose start finished; a changed block goes
    /// with that start's job result in `batch` (or with the next tool result).
    pub fn poll_starts(&mut self, batch: &mut [crate::console::Inbound]) {
        let done: Vec<(String, String)> = self
            .starting
            .iter()
            .filter(|(_, s)| s.handle.is_finished())
            .map(|(n, s)| (n.clone(), s.job.clone()))
            .collect();
        for (name, job) in done {
            let Some(block) = self.finish_start(&name) else { continue };
            let ch = format!("{}{job}", crate::jobs::PREFIX);
            match batch.iter_mut().find(|ib| ib.channel == ch && !ib.text.starts_with("stale?")) {
                Some(ib) => ib.text = format!("{}\n{block}", ib.text),
                None => self.reload_notes.push(block),
            }
        }
    }

    /// While `name` starts in the background: what to tell the model. A start that has
    /// finished is attached (or dropped when it failed) and one given up with job_kill is
    /// forgotten (its thread stops the process when its current step ends); then None,
    /// and the caller goes on as for a server that is running or not started.
    fn start_status(&mut self, name: &str) -> Option<String> {
        let st = self.starting.get(name)?;
        let job = st.job.clone();
        if st.handle.is_finished() {
            if let Some(block) = self.finish_start(name) {
                self.reload_notes.push(block);
            }
            return None;
        }
        let running = {
            let js = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
            js.get(&job).map(|j| j.state == "running").unwrap_or(false)
        };
        if !running {
            self.starting.remove(name);
            self.drop_held(name, &format!("its start was given up (job_kill job:{job})"));
            return None;
        }
        Some(format!(
            "mcp/{name} is starting in job:{job}; its functions become callable when system|job:{job} arrives. job_logs(id=\"{job}\") shows its progress; job_kill(id=\"{job}\") gives the start up (then enable it again to start fresh)."
        ))
    }

    /// bia's own sends (say to a messenger or web chat) right after boot: wait for a
    /// background start of that bus, up to `secs`, instead of failing.
    fn await_start(&mut self, name: &str, secs: u64) -> Result<()> {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(secs);
        while let Some(msg) = self.start_status(name) {
            if std::time::Instant::now() >= end {
                bail!("{msg}");
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        Ok(())
    }

    fn push_tools(&mut self, idx: usize, server: &str, tools: Vec<McpTool>) {
        for t in tools {
            if agent_only(server, &t.name) {
                continue; // the agent calls these itself (web_invoke); the model uses say & co.
            }
            self.specs.push(ToolSpec {
                name: format!("{}_{}", server, t.name),
                description: t.description,
                parameters: t.schema,
            });
            self.kinds.push(Kind::Mcp {
                server: idx,
                tool: t.name,
            });
            self.groups.push(String::new());
        }
    }

    pub fn add_server(&mut self, server: McpServer, tools: Vec<McpTool>, info: ServerInfo) {
        let idx = self.servers.len();
        self.infos.push(info);
        let name = server.name.clone();
        self.servers.push(server);
        self.push_tools(idx, &name, tools);
    }

    /// Only `discover`: the frozen system-prompt / FMM tools[] list. Other functions are appended to history when a branch is enabled.
    pub fn prompt_specs(&self) -> Vec<ToolSpec> {
        self.specs
            .iter()
            .filter(|s| s.name == "discover")
            .cloned()
            .collect()
    }

    /// `discover`, enabled builtin branches, and functions of loaded MCP servers.
    pub fn visible_specs(&self) -> Vec<ToolSpec> {
        self.specs
            .iter()
            .enumerate()
            .filter(|(i, s)| match &self.kinds[*i] {
                Kind::Builtin => s.name == "discover" || self.enabled.contains(&self.groups[*i]),
                Kind::Mcp { server, .. } => self.loaded.contains(server),
            })
            .map(|(_, s)| s.clone())
            .collect()
    }

    pub fn loaded_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .loaded
            .iter()
            .map(|i| self.servers[*i].name.clone())
            .collect();
        v.sort();
        v
    }

    fn server_index(&self, name: &str) -> Option<usize> {
        self.servers.iter().position(|s| s.name == name)
    }

    fn mcp_search_roots(&self) -> Vec<PathBuf> {
        let root = self
            .mcp_env
            .iter()
            .find(|(k, _)| k == "BIA_MCPS")
            .map(|(_, v)| PathBuf::from(v))
            .unwrap_or_else(|| PathBuf::from("/home/bia/MCPs"));
        vec![root]
    }

    fn find_on_disk(&self, name: &str) -> Option<(PathBuf, crate::mcp::Manifest)> {
        for root in self.mcp_search_roots() {
            for (dir, mf) in crate::mcp::discover(&root) {
                if let Ok(m) = mf {
                    if m.name == name {
                        return Some((dir, m));
                    }
                }
            }
        }
        None
    }

    /// Spawn from disk, or restart from a preferred copy. A running server is never restarted
    /// because its files changed: the model restarts it (discover restart=true) when it wants.
    fn refresh_named(&mut self, name: &str) -> Result<String> {
        if let Some(msg) = self.start_status(name) {
            // never wait for a background start here: it may hang (a network restore)
            bail!("{msg}");
        }
        let Some((dir, m)) = self.find_on_disk(name) else {
            return Ok(String::new());
        };
        if let Some(i) = self.server_index(name) {
            let old = self.servers[i].dir.clone();
            if old != dir {
                self.servers[i].relocate(dir.clone())?;
                if let Some(block) = self.reload_server(i, &m)? {
                    self.reload_notes.push(block);
                }
                return Ok(format!(
                    "restarted from {} (was {})",
                    dir.display(),
                    old.display()
                ));
            }
            return Ok(String::new());
        }
        self.spawn_from(dir, m)
    }

    fn spawn_from(&mut self, dir: PathBuf, m: crate::mcp::Manifest) -> Result<String> {
        let mut srv = crate::mcp::McpServer::spawn(
            &dir,
            &m,
            &self.mcp_arch,
            &self.mcp_env,
            &self.mcp_log_dir,
            self.con.clone(),
            self.jobs.clone(),
        )?;
        match srv.list_tools() {
            Ok(list) => {
                let n = list.len();
                let names: Vec<String> = list.iter().map(|t| t.name.clone()).collect();
                let info = server_info(&m, &list);
                let mut note = format!(
                    "MCP {} v{}: {n} tools ({})",
                    m.name,
                    srv.version,
                    names.join(", ")
                );
                if let Some(bt) = m.boot_tool.clone() {
                    if list.iter().any(|t| t.name == bt) {
                        match srv.call(&bt, json!({}), &mut |_| {}) {
                            Ok((text, false)) if !text.trim().is_empty() => {
                                note.push_str(&format!(
                                    "\n{bt}: {}",
                                    text.trim().replace('\n', "; ")
                                ));
                            }
                            Ok((text, true)) => note.push_str(&format!("\n{bt}: {}", text.trim())),
                            Err(e) => note.push_str(&format!("\n{bt}: {e:#}")),
                            _ => {}
                        }
                    }
                }
                self.add_server(srv, list, info);
                Ok(note)
            }
            Err(e) => {
                srv.kill();
                anyhow::bail!(
                    "MCP {}: tools/list: {e:#} (see {}/mcp-{}.log)",
                    m.name,
                    self.mcp_log_dir.display(),
                    m.name
                )
            }
        }
    }

    /// Function definitions of one server in the flat tool format (for a message to the model).
    pub fn definitions(&self, idx: usize) -> String {
        let defs: Vec<Value> = self
            .specs
            .iter()
            .zip(&self.kinds)
            .filter(|(_, k)| matches!(k, Kind::Mcp { server, .. } if *server == idx))
            .map(|(t, _)| json!({"name": t.name, "description": t.description, "parameters": t.parameters}))
            .collect();
        Value::Array(defs).to_string()
    }

    /// Mark a server as loaded; returns (newly loaded, message with its definitions).
    pub fn load(&mut self, name: &str) -> Result<(bool, String)> {
        let refresh = self.refresh_named(name)?;
        if self.server_index(name).is_none() {
            let notes = self.adopt_local_mcps();
            if self.server_index(name).is_none() {
                let avail = self
                    .infos
                    .iter()
                    .map(|i| i.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                let extra = if notes.is_empty() {
                    String::new()
                } else {
                    format!("; {}", notes.join("; "))
                };
                anyhow::bail!("unknown MCP: {name}; available: {avail}{extra}");
            }
        }
        let idx = self
            .server_index(name)
            .ok_or_else(|| anyhow::anyhow!("unknown MCP: {name}"))?;
        let new = self.loaded.insert(idx);
        let mut msg = self.mcp_text(idx, true);
        if !refresh.is_empty() {
            msg = format!("{refresh}. {msg}");
        }
        Ok((new, msg))
    }

    /// Mark an MCP callable after resume without sending its schemas to the model.
    pub fn ensure_loaded(&mut self, name: &str) -> Result<()> {
        self.refresh_named(name)?;
        if let Some(idx) = self.server_index(name) {
            self.loaded.insert(idx);
            if self.schemas_sent.insert(format!("mcp/{name}")) {
                // first time this run (a resumed conversation): the model's copy is from before
                if let Some(block) = self.changed_block(idx, false) {
                    self.reload_notes.push(block);
                }
            }
        }
        Ok(())
    }

    /// If the model called `invoke`, unwrap to the real name+args. Otherwise pass through.
    pub fn expand_invoke(name: &str, args: &Value) -> (String, Value) {
        if name != "invoke" {
            return (name.to_string(), args.clone());
        }
        let inner = args
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if inner.is_empty() || inner == "invoke" {
            return (name.to_string(), args.clone());
        }
        let inner_args = match args.get("arguments") {
            Some(Value::String(s)) => serde_json::from_str(s).unwrap_or(json!({})),
            Some(v) => v.clone(),
            None => match args {
                Value::Object(map) => {
                    let mut m = map.clone();
                    m.remove("name");
                    Value::Object(m)
                }
                _ => json!({}),
            },
        };
        (inner.to_string(), inner_args)
    }

    fn take_arguments(args: &Value) -> Value {
        match args.get("arguments") {
            Some(Value::String(s)) => serde_json::from_str(s).unwrap_or(json!({})),
            Some(v) => v.clone(),
            None => match args {
                Value::Object(map) => {
                    let mut m = map.clone();
                    for k in ["path", "enable", "q", "call", "arguments"] {
                        m.remove(k);
                    }
                    if m.is_empty() {
                        json!({})
                    } else {
                        Value::Object(m)
                    }
                }
                _ => json!({}),
            },
        }
    }

    /// Unwrap `invoke` and `discover(call=…)`. A discover that only lists or enables stays `discover`.
    pub fn expand_call(name: &str, args: &Value) -> (String, Value) {
        let (name, args) = Self::expand_invoke(name, args);
        let (name, args) = if name == "discover" {
            let call = args
                .get("call")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if call.is_empty() {
                ("discover".into(), args)
            } else if call == "discover" {
                ("discover".into(), Self::take_arguments(&args))
            } else {
                (call, Self::take_arguments(&args))
            }
        } else {
            (name, args)
        };
        if name == "shell" {
            ("bash".into(), args)
        } else {
            (name, args)
        }
    }

    /// Rewrite every tool call except a discover list/enable into `discover(call=…)` so FMM tools[] stays one function.
    pub fn fold_invoke_for_remote(&self, mut history: Vec<Msg>) -> Vec<Msg> {
        for m in &mut history {
            match m.role {
                Role::Assistant => {
                    for c in &mut m.tool_calls {
                        let (name, args) = Self::expand_call(&c.name, &c.args);
                        if name == "discover" {
                            c.name = "discover".into();
                            c.args = args;
                            continue;
                        }
                        c.args = json!({"call": name, "arguments": args});
                        c.name = "discover".into();
                    }
                }
                Role::Tool if m.name != "discover" => m.name = "discover".into(),
                _ => {}
            }
        }
        history
    }

    fn flag(args: &Value, key: &str) -> bool {
        match args.get(key) {
            Some(Value::Bool(b)) => *b,
            Some(Value::String(s)) => {
                matches!(s.to_ascii_lowercase().as_str(), "true" | "1" | "yes")
            }
            Some(Value::Number(n)) => n.as_u64() == Some(1),
            _ => false,
        }
    }

    fn group_tools(&self, group: &str) -> Vec<usize> {
        self.groups
            .iter()
            .enumerate()
            .filter(|(_, g)| g.as_str() == group)
            .map(|(i, _)| i)
            .collect()
    }

    fn schemas(&self, idxs: &[usize]) -> String {
        let defs: Vec<Value> = idxs
            .iter()
            .map(|&i| {
                let t = &self.specs[i];
                json!({"name": t.name, "description": t.description, "parameters": t.parameters})
            })
            .collect();
        Value::Array(defs).to_string()
    }

    fn schema_one(&self, i: usize) -> String {
        let t = &self.specs[i];
        json!({"name": t.name, "description": t.description, "parameters": t.parameters})
            .to_string()
    }

    fn mcp_text(&self, idx: usize, enabled: bool) -> String {
        let info = &self.infos[idx];
        let mut lines = vec![format!("mcp/{} — {}", info.name, info.description)];
        if !info.examples.is_empty() {
            lines.push("Examples:".into());
            for e in &info.examples {
                lines.push(format!("- {e}"));
            }
        }
        if enabled {
            let names = self.mcp_fn_names(idx);
            lines.push(Self::call_by_name(
                &format!("mcp/{}", info.name),
                &names,
                false,
            ));
            lines.push(self.definitions(idx));
        } else {
            for (spec, kind) in self.specs.iter().zip(&self.kinds) {
                if matches!(kind, Kind::Mcp { server, .. } if *server == idx) {
                    lines.push(format!("{} — {}", spec.name, brief(&spec.description)));
                }
            }
            lines.push(format!(
                "discover(path=\"mcp/{}\", enable=true) loads the schemas once. Then call each function by name.",
                info.name
            ));
        }
        lines.join("\n")
    }

    pub fn listed_mcp_names(&self) -> Vec<String> {
        self.listed_mcps()
            .into_iter()
            .map(|(name, _, _)| name)
            .collect()
    }

    fn listed_mcps(&self) -> Vec<(String, String, bool)> {
        let mut v = vec![];
        let mut seen = std::collections::HashSet::new();
        for info in &self.infos {
            seen.insert(info.name.clone());
            v.push((info.name.clone(), brief(&info.description), true));
        }
        for info in &self.catalog {
            if seen.insert(info.name.clone()) {
                v.push((info.name.clone(), brief(&info.description), false));
            }
        }
        v
    }

    fn mcp_line(name: &str, desc: &str, started: bool) -> String {
        if started {
            format!("mcp/{name} — {desc}")
        } else {
            format!("mcp/{name} — {desc} (not started)")
        }
    }

    fn discover_top(&self) -> String {
        let mut lines: Vec<String> = BRANCHES
            .iter()
            .map(|b| format!("{} — {}", b.id, b.blurb))
            .collect();
        for (name, desc, started) in self.listed_mcps() {
            lines.push(Self::mcp_line(&name, &desc, started));
        }
        lines.push("Open a path with discover(path=\"…\"). Search with discover(q=\"…\"). Enable with discover(path=\"…\", enable=true) starts that server if needed and appends its schemas once. Then call each function by the name in the schema.".into());
        lines.join("\n")
    }

    fn discover_mcp_dir(&self) -> String {
        let listed = self.listed_mcps();
        if listed.is_empty() {
            return "no MCP servers. Drop one under /home/bia/MCPs and discover again.".into();
        }
        let mut lines: Vec<String> = listed
            .iter()
            .map(|(name, desc, started)| Self::mcp_line(name, desc, *started))
            .collect();
        lines.push("discover(path=\"mcp/<name>\") opens one. enable=true starts that server and loads its functions.".into());
        lines.join("\n")
    }

    fn discover_search(&self, q: &str) -> String {
        let words = query_words(q);
        let mut lines = vec![];
        for b in BRANCHES {
            let blob = format!("{} {} {}", b.id, b.blurb, b.guide);
            if blob_has(&blob, &words) {
                lines.push(format!("{} — {}", b.id, b.blurb));
                continue;
            }
            for i in self.group_tools(b.id) {
                let spec = &self.specs[i];
                if blob_has(&format!("{} {}", spec.name, spec.description), &words) {
                    lines.push(format!(
                        "{}/{} — {}",
                        b.id,
                        spec.name,
                        brief(&spec.description)
                    ));
                }
            }
        }
        for (idx, info) in self.infos.iter().enumerate() {
            let blob = format!(
                "{} {} {} {}",
                info.name,
                info.description,
                info.keywords.join(" "),
                info.examples.join(" ")
            );
            if blob_has(&blob, &words) {
                lines.push(format!("mcp/{} — {}", info.name, brief(&info.description)));
                continue;
            }
            for (spec, kind) in self.specs.iter().zip(&self.kinds) {
                if matches!(kind, Kind::Mcp { server, .. } if *server == idx)
                    && blob_has(&format!("{} {}", spec.name, spec.description), &words)
                {
                    lines.push(format!("{} — {}", spec.name, brief(&spec.description)));
                }
            }
        }
        let running: std::collections::HashSet<&str> =
            self.infos.iter().map(|i| i.name.as_str()).collect();
        for info in &self.catalog {
            if running.contains(info.name.as_str()) {
                continue;
            }
            let blob = format!(
                "{} {} {} {}",
                info.name,
                info.description,
                info.keywords.join(" "),
                info.examples.join(" ")
            );
            if blob_has(&blob, &words) {
                lines.push(Self::mcp_line(&info.name, &brief(&info.description), false));
            }
        }
        if lines.is_empty() {
            format!("nothing matches {q}. discover() lists the tree.")
        } else {
            lines.truncate(20);
            lines.join("\n")
        }
    }

    fn fn_names(&self, idxs: &[usize]) -> Vec<String> {
        idxs.iter().map(|&i| self.specs[i].name.clone()).collect()
    }

    fn mcp_fn_names(&self, idx: usize) -> Vec<String> {
        self.specs
            .iter()
            .zip(&self.kinds)
            .filter_map(|(s, k)| match k {
                Kind::Mcp { server, .. } if *server == idx => Some(s.name.clone()),
                _ => None,
            })
            .collect()
    }

    fn call_by_name(what: &str, names: &[String], again: bool) -> String {
        let list = names.join(", ");
        if again {
            format!("{what} is already enabled. Call by name: {list}. Do not enable again.")
        } else {
            format!("{what} enabled. Call by name: {list}. Do not enable again.")
        }
    }

    fn discover_branch(&mut self, branch: &Branch, enable: bool) -> String {
        let idxs = self.group_tools(branch.id);
        let names = self.fn_names(&idxs);
        if !enable {
            let mut lines = vec![
                format!("{} — {}", branch.id, branch.blurb),
                branch.guide.into(),
            ];
            for (i, name) in idxs.iter().zip(&names) {
                lines.push(format!("{name} — {}", brief(&self.specs[*i].description)));
            }
            lines.push(format!(
                "discover(path=\"{}\", enable=true) loads the schemas once. Then call the function by name.",
                branch.id
            ));
            return lines.join("\n");
        }
        if self.schemas_sent.contains(branch.id) {
            return Self::call_by_name(branch.id, &names, true);
        }
        self.schemas_sent.insert(branch.id.into());
        self.enabled.insert(branch.id.into());
        format!(
            "{} — {}\n{}\n{}\n{}",
            branch.id,
            branch.blurb,
            branch.guide,
            Self::call_by_name(branch.id, &names, false),
            self.schemas(&idxs)
        )
    }

    fn discover_one(&mut self, i: usize, enable: bool) -> Result<String> {
        let name = self.specs[i].name.clone();
        if name == "discover" || name == "use_mcp" || name == "invoke" {
            bail!("unknown path {name}; discover() lists the tree");
        }
        let kind = self.kinds[i].clone();
        match kind {
            Kind::Builtin => {
                let group = self.groups[i].clone();
                if group.is_empty() {
                    bail!("unknown path {name}; discover() lists the tree");
                }
                if !enable {
                    return Ok(format!(
                        "{group}/{name} — {}\ndiscover(path=\"{group}/{name}\", enable=true) loads this schema once. Then call {name} by name.",
                        brief(&self.specs[i].description)
                    ));
                }
                let key = format!("tool:{name}");
                self.enabled.insert(group.clone());
                if self.schemas_sent.contains(&group) || self.schemas_sent.contains(&key) {
                    return Ok(Self::call_by_name(&name, std::slice::from_ref(&name), true));
                }
                self.schemas_sent.insert(key);
                Ok(format!(
                    "{}\n{}",
                    Self::call_by_name(&name, std::slice::from_ref(&name), false),
                    self.schema_one(i)
                ))
            }
            Kind::Mcp { server, .. } => {
                let srv = self.servers[server].name.clone();
                if !enable {
                    return Ok(format!(
                        "mcp/{srv} {name} — {}\ndiscover(path=\"mcp/{srv}\", enable=true) loads this server once. Then call {name} by name.",
                        brief(&self.specs[i].description)
                    ));
                }
                let server_key = format!("mcp/{srv}");
                let key = format!("tool:{name}");
                // checked before ensure_loaded, which marks the whole server as sent
                let had = self.schemas_sent.contains(&server_key) || self.schemas_sent.contains(&key);
                self.ensure_loaded(&srv)?;
                if had {
                    return Ok(Self::call_by_name(&name, std::slice::from_ref(&name), true));
                }
                self.schemas_sent.insert(key);
                Ok(format!(
                    "{}\n{}",
                    Self::call_by_name(&name, std::slice::from_ref(&name), false),
                    self.schema_one(i)
                ))
            }
        }
    }

    fn enable_mcp(&mut self, name: &str, idx: usize) -> Result<String> {
        let key = format!("mcp/{name}");
        let names = self.mcp_fn_names(idx);
        if self.schemas_sent.contains(&key) {
            self.ensure_loaded(name)?;
            return Ok(Self::call_by_name(&key, &names, true));
        }
        let msg = self.load(name)?.1;
        self.schemas_sent.insert(key);
        if let Some(i) = self.server_index(name) {
            let hash = block_hash(&self.mcp_text(i, true));
            self.remember_block(name, hash);
        }
        Ok(msg)
    }

    fn attach_notes(body: String, notes: &[String]) -> String {
        if notes.is_empty() {
            body
        } else {
            format!("{body}\n{}", notes.join("\n"))
        }
    }

    fn discover_cmd(&mut self, args: &Value) -> Result<String> {
        let notes = self.rescan_mcps();
        let q = args.get("q").and_then(|v| v.as_str()).unwrap_or("").trim();
        let path_arg = args.get("path").and_then(|v| v.as_str()).unwrap_or("").trim().trim_matches('/');
        // q names one function the model can already call: it lost track of how (its schema
        // went with a compacted part of the conversation, or never stuck), so give it that
        // schema instead of "already enabled" again (a model repeated that search 9 times)
        if let Some(i) = self.enabled_fn(q) {
            let f = self.specs[i].name.clone();
            return Ok(format!(
                "{f} is ready: call {f}(…) by name now, with these parameters. Do not discover it again.\n{}",
                self.schema_one(i)
            ));
        }
        // discover(path=X, q=f) where f is a function in X: the model wants X, so enable it
        // (searching again and again instead was a common loop)
        if !q.is_empty() && !path_arg.is_empty() && self.step_path(q).as_deref() == Some(path_arg) {
            let mut a = args.clone();
            if let Some(o) = a.as_object_mut() {
                o.remove("q");
                o.insert("enable".into(), Value::Bool(true));
            }
            return self.discover_cmd(&a);
        }
        if !q.is_empty() {
            let q_l = q.to_lowercase();
            let extra: Vec<String> = notes
                .into_iter()
                .filter(|n| n.to_lowercase().contains(&q_l))
                .collect();
            let mut found = self.discover_search(q);
            if let Some(step) = self.next_step(q) {
                found = format!("{step}\n{found}");
            }
            return Ok(Self::attach_notes(found, &extra));
        }
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .trim_matches('/');
        let enable = Self::flag(args, "enable");
        if path.is_empty() || path == "discover" {
            if enable {
                bail!("pick a path; discover() lists them");
            }
            return Ok(Self::attach_notes(self.discover_top(), &notes));
        }
        if path == "mcp" {
            if enable {
                let names = self
                    .listed_mcps()
                    .iter()
                    .map(|(n, _, _)| format!("mcp/{n}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                bail!("pick a server: {names}");
            }
            return Ok(Self::attach_notes(self.discover_mcp_dir(), &notes));
        }
        if let Some(branch) = BRANCHES.iter().find(|b| b.id == path) {
            return Ok(self.discover_branch(branch, enable));
        }
        if let Some(rest) = path.strip_prefix("mcp/") {
            let name = rest.split('/').next().unwrap_or(rest);
            if Self::flag(args, "restart") {
                return self.restart_mcp(name);
            }
            if enable && self.server_index(name).is_none() {
                self.refresh_named(name)?;
            }
            if let Some((srv, tool)) = rest.split_once('/') {
                let prefixed = format!("{srv}_{tool}");
                if let Some(i) = self
                    .specs
                    .iter()
                    .position(|s| s.name == tool || s.name == prefixed)
                {
                    return self.discover_one(i, enable);
                }
            }
            if let Some(msg) = self.open_mcp(name, enable)? {
                return Ok(msg);
            }
            if notes.is_empty() {
                bail!("unknown MCP {name}; discover(path=\"mcp\") lists servers");
            }
            bail!("unknown MCP {name}; {}", notes.join("; "));
        }
        if let Some((group, tool)) = path.split_once('/') {
            if let Some(i) = self
                .specs
                .iter()
                .enumerate()
                .find(|(i, s)| s.name == tool && self.groups[*i] == group)
                .map(|(i, _)| i)
            {
                return self.discover_one(i, enable);
            }
        }
        if let Some(i) = self.specs.iter().position(|s| s.name == path) {
            return self.discover_one(i, enable);
        }
        if let Some(msg) = self.open_mcp(path, enable)? {
            return Ok(msg);
        }
        if notes.is_empty() {
            bail!("unknown path {path}; discover() lists the tree");
        }
        bail!("unknown path {path}; {}", notes.join("; "))
    }
    /// Execute a tool. Returns (result text, is_error).
    pub fn call(
        &mut self,
        name: &str,
        args: &Value,
        on_log: &mut dyn FnMut(&str),
    ) -> (String, bool) {
        let (text, err) = self.call_inner(name, args, on_log);
        (self.with_reload_notes(text), err)
    }

    /// Reloaded MCPs' new blocks (reload_server) in front of a tool result.
    fn with_reload_notes(&mut self, text: String) -> String {
        if self.reload_notes.is_empty() {
            return text;
        }
        let notes = std::mem::take(&mut self.reload_notes).join("\n\n");
        format!("{notes}\n\n{text}")
    }

    /// An unknown `<server>_<tool>` name: if that MCP was updated, a restart brings it.
    fn unknown_hint(&mut self, name: &str) -> String {
        let starting = self.starting.keys().find(|s| name.starts_with(&format!("{s}_"))).cloned();
        if let Some(msg) = starting.and_then(|srv| self.start_status(&srv)) {
            return format!(" ({msg})");
        }
        match self.servers.iter().map(|s| s.name.clone()).find(|s| name.starts_with(&format!("{s}_"))) {
            Some(srv) => format!(" (if mcp/{srv} was updated or edited, discover(path=\"mcp/{srv}\", restart=true) loads its new functions)"),
            None => String::new(),
        }
    }

    fn call_inner(
        &mut self,
        name: &str,
        args: &Value,
        on_log: &mut dyn FnMut(&str),
    ) -> (String, bool) {
        let (name_owned, args_owned) = Self::expand_call(name, args);
        let name = name_owned.as_str();
        let args = &args_owned;
        let Some(i) = self.specs.iter().position(|s| s.name == name) else {
            if let Some(branch) = BRANCHES.iter().find(|b| b.id == name) {
                return (format!(
                    "error: {name} is a branch, not a tool. discover(path=\"{}\", enable=true) once, then call the function by name. {}",
                    branch.id, branch.blurb
                ), true);
            }
            return (format!("unknown tool: {name}{}", self.unknown_hint(name)), true);
        };
        if name == "discover" {
            return match self.discover_cmd(args) {
                Ok(s) => (s, false),
                Err(e) => (format!("error: {e}"), true),
            };
        }
        if name == "use_mcp" {
            let mcp = args
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if let Some(idx) = self.server_index(&mcp) {
                return match self.enable_mcp(&mcp, idx) {
                    Ok(msg) => (msg, false),
                    Err(e) => (format!("error: {e}"), true),
                };
            }
            return match self.load(&mcp) {
                Ok((_, msg)) => {
                    self.schemas_sent.insert(format!("mcp/{mcp}"));
                    (msg, false)
                }
                Err(e) => (format!("error: {e}"), true),
            };
        }
        if name == "say" {
            if let Some(id) = args
                .get("cancel")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
            {
                return match self.outbox.cancel(id) {
                    Ok(s) => (s, false),
                    Err(e) => (format!("error: {e}"), true),
                };
            }
            let (_, channel) = self.target(args);
            let blank = |k: &str| match args.get(k) {
                None | Some(Value::Null) => true,
                Some(Value::String(s)) => s.trim().is_empty(),
                Some(Value::Array(a)) => a.is_empty(),
                Some(_) => false,
            };
            if blank("message")
                && blank("files")
                && blank("poll")
                && !str_list(args, "answered").is_empty()
            {
                return (
                    self.note_reply(&channel, args, "nothing sent".into()),
                    false,
                );
            }
            return match self.say(args) {
                Ok(s) => (self.note_reply(&channel, args, s), false),
                Err(e) => (format!("error: {e}"), true),
            };
        }
        match &self.kinds[i] {
            Kind::Builtin => {
                let channel = (name == "react").then(|| self.target(args).1);
                match self.builtin(name, args) {
                    Ok(s) => match channel {
                        Some(ch) => (self.note_reply(&ch, args, s), false),
                        None => (s, false),
                    },
                    Err(e) => (format!("error: {e}"), true),
                }
            }
            Kind::Mcp { server, tool } => {
                let (server, tool) = (*server, tool.clone());
                self.loaded.insert(server);
                if let Err(e) = self.prepare_mcp(server) {
                    return (format!("error: {e:#}"), true);
                }
                if !self.has_mcp_tool(server, &tool) {
                    return (format!("error: {name} no longer exists: mcp/{} was updated", self.servers[server].name), true);
                }
                self.mcp_invoke(server, &tool, args.clone(), on_log)
            }
        }
    }

    fn has_mcp_tool(&self, idx: usize, name: &str) -> bool {
        self.kinds.iter().any(|k| matches!(k, Kind::Mcp { server, tool } if *server == idx && tool == name))
    }

    fn prepare_mcp(&mut self, server: usize) -> Result<()> {
        let name = self.servers[server].name.clone();
        self.refresh_named(&name)?;
        self.servers[server].ensure_alive()
    }

    fn mcp_invoke(
        &mut self,
        server: usize,
        tool: &str,
        args: Value,
        on_log: &mut dyn FnMut(&str),
    ) -> (String, bool) {
        let s = &mut self.servers[server];
        match s.call(tool, args.clone(), on_log) {
            Ok(r) => r,
            Err(e) if crate::mcp::McpServer::transport_error(&e) => match s.respawn() {
                Ok(()) => match s.call(tool, args, on_log) {
                    Ok(r) => r,
                    Err(e2) => (format!("error: {e2:#}"), true),
                },
                Err(re) => (format!("error: {e:#}; restart failed: {re:#}"), true),
            },
            Err(e) => (format!("error: {e:#}"), true),
        }
    }

    fn owner_mcp(&self, tool: &str) -> Option<String> {
        let mut best: Option<String> = None;
        let mut consider = |name: &str| {
            if tool.starts_with(&format!("{name}_"))
                && best.as_ref().map(|b| name.len() > b.len()).unwrap_or(true)
            {
                best = Some(name.to_string());
            }
        };
        for info in &self.catalog {
            consider(&info.name);
        }
        for s in &self.servers {
            consider(&s.name);
        }
        best
    }

    /// Turn a model tool call into a real function and arguments. Loads the owning MCP when the
    /// name is missing. Returns an error the model can follow when the call cannot be repaired.
    pub fn guide_call(
        &mut self,
        name: &str,
        args: &Value,
    ) -> Result<(String, Value, String), String> {
        if let Some(e) = cut_args_error(args) {
            return Err(e);
        }
        let (name, name_note) = self.resolve_tool_name(name)?;
        let (args, arg_note) = self.align_args(&name, args)?;
        let repeat_note = self.repeat_note(&name, &args);
        let note = [repeat_note, name_note, arg_note]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        Ok((name, args, note))
    }

    /// A call the model already made a moment ago with the same arguments: say so, with the
    /// next step, so it does not loop (the call still runs).
    fn repeat_note(&mut self, name: &str, args: &Value) -> String {
        const KEEP: usize = 8;
        let key = format!("{name} {args}");
        let times = self.recent_calls.iter().filter(|k| **k == key).count();
        self.recent_calls.push_back(key);
        while self.recent_calls.len() > KEEP {
            self.recent_calls.pop_front();
        }
        if times == 0 {
            return String::new();
        }
        let q = args.get("q").and_then(|v| v.as_str()).unwrap_or("");
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("").trim_matches('/');
        let ready = self.enabled_fn(q).map(|i| self.specs[i].name.clone());
        let next = match self.next_step(q) {
            _ if name == "discover" && ready.is_some() => {
                let f = ready.unwrap_or_default();
                format!("{f} is already callable: call {f}(…) by name now with the parameters in the schema above.")
            }
            Some(step) if name == "discover" => step,
            _ if name == "discover" && !path.is_empty() && !Self::flag(args, "enable") => {
                format!("To use {path}: discover(path=\"{path}\", enable=true), then call its function by name.")
            }
            _ => "Use the result you already have: do the next step of the task, or answer with say.".into(),
        };
        format!("[repeat] You already called {name} with these arguments ({}x); the result will not change. {next}", times + 1)
    }

    /// `q` names one function exactly (system_fmm_configure, fmm_configure, lang_install): how to
    /// load and call it.
    fn next_step(&self, q: &str) -> Option<String> {
        let q = q.trim().trim_end_matches("()").to_lowercase();
        if q.is_empty() || q.contains(' ') {
            return None;
        }
        let suffix = format!("_{q}");
        // (function, path) of every loaded function named q or ending in _q; an exact name wins
        let mut hits: Vec<(String, String)> = vec![];
        for b in BRANCHES {
            for i in self.group_tools(b.id) {
                hits.push((self.specs[i].name.clone(), b.id.to_string()));
            }
        }
        for (spec, kind) in self.specs.iter().zip(&self.kinds) {
            if let Kind::Mcp { server, .. } = kind {
                hits.push((spec.name.clone(), format!("mcp/{}", self.infos[*server].name)));
            }
        }
        let exact: Vec<_> = hits.iter().filter(|(f, _)| *f == q).collect();
        let near: Vec<_> = hits.iter().filter(|(f, _)| f.ends_with(&suffix)).collect();
        let pick = if exact.len() == 1 { exact.first().copied() } else if exact.is_empty() && near.len() == 1 { near.first().copied() } else { None };
        if let Some((f, path)) = pick {
            return Some(format!("{f} is in {path}: discover(path=\"{path}\", enable=true), then call {f}(…)."));
        }
        let call = format!("{q}(");
        self.infos.iter().chain(&self.catalog).find(|i| {
            i.examples.iter().any(|e| e.contains(&call)) || i.description.contains(&call)
        }).map(|i| format!("{q} is in mcp/{0}: discover(path=\"mcp/{0}\", enable=true), then call it by the name in its schema.", i.name))
    }

    /// The function `q` names exactly, when it is callable now and its branch's schemas
    /// were already given to the model (else enabling gives them).
    fn enabled_fn(&self, q: &str) -> Option<usize> {
        let q = q.trim().trim_end_matches("()");
        if q.is_empty() || q == "discover" {
            return None;
        }
        let i = self.specs.iter().position(|s| s.name == q)?;
        let sent = |k: &str| self.schemas_sent.contains(k);
        let ready = match &self.kinds[i] {
            Kind::Builtin => {
                let g = &self.groups[i];
                !g.is_empty() && self.enabled.contains(g) && (sent(g) || sent(&format!("tool:{q}")))
            }
            Kind::Mcp { server, .. } => {
                self.loaded.contains(server)
                    && (sent(&format!("mcp/{}", self.servers[*server].name)) || sent(&format!("tool:{q}")))
            }
        };
        ready.then_some(i)
    }

    /// Compaction dropped the part of the conversation holding the function schemas: the
    /// next enable gives them again (callable functions stay callable).
    pub fn forget_schemas(&mut self) {
        self.schemas_sent.clear();
    }

    /// The path (shell, mcp/system, …) holding the one function `q` names, if any.
    fn step_path(&self, q: &str) -> Option<String> {
        let step = self.next_step(q)?;
        let start = step.find("discover(path=\"")? + "discover(path=\"".len();
        let len = step[start..].find('"')?;
        Some(step[start..start + len].to_string())
    }

    fn resolve_tool_name(&mut self, name: &str) -> Result<(String, String), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("no function name. discover() lists paths. discover(path=\"mcp/<server>\", enable=true) loads a server, then call the function name from that schema.".into());
        }
        if self.specs.iter().any(|s| s.name == name) {
            return Ok((name.to_string(), String::new()));
        }
        if let Some(branch) = BRANCHES.iter().find(|b| b.id == name) {
            return Err(format!(
                "{name} is a branch, not a function. discover(path=\"{}\", enable=true) once, then call one of its functions by name. {}",
                branch.id, branch.blurb
            ));
        }
        if let Some(owner) = self.owner_mcp(name) {
            self.load(&owner).map_err(|e| format!("{e:#}"))?;
            self.schemas_sent.insert(format!("mcp/{owner}"));
            if self.specs.iter().any(|s| s.name == name) {
                return Ok((name.to_string(), format!("enabled mcp/{owner}")));
            }
        }
        if let Some(mcp) = self.mcp_mentioning(name) {
            if self.server_index(&mcp).is_none() {
                self.load(&mcp).map_err(|e| format!("{e:#}"))?;
                self.schemas_sent.insert(format!("mcp/{mcp}"));
            }
        }
        if let Some(hit) = self.unique_tool(name) {
            let note = if hit == name {
                String::new()
            } else {
                format!("no tool {name}; use {hit}")
            };
            return Ok((hit, note));
        }
        let close = self.close_tools(name);
        if close.is_empty() {
            Err(format!(
                "unknown tool: {name}. discover() lists paths. discover(path=\"mcp/<server>\", enable=true) loads that server's functions. Then call the exact name from the schema."
            ))
        } else {
            Err(format!(
                "unknown tool: {name}. Closest: {}. Call one of those names with the arguments from its schema. Do not invent a name.",
                close.join(", ")
            ))
        }
    }

    fn mcp_mentioning(&self, tool: &str) -> Option<String> {
        let mut hits = vec![];
        for info in self.catalog.iter().chain(self.infos.iter()) {
            let blob = info.examples.join("\n");
            if blob.contains(&format!("{tool}(")) || blob.contains(&format!("_{tool}(")) {
                hits.push(info.name.clone());
            }
        }
        hits.sort();
        hits.dedup();
        if hits.len() == 1 {
            hits.pop()
        } else {
            None
        }
    }

    fn unique_tool(&self, name: &str) -> Option<String> {
        let n = name.to_lowercase();
        let mut hits: Vec<String> = self
            .specs
            .iter()
            .filter(|s| {
                let sn = s.name.to_lowercase();
                sn == n
                    || sn.ends_with(&format!("_{n}"))
                    || sn.rsplit('_').next() == Some(n.as_str())
            })
            .map(|s| s.name.clone())
            .collect();
        hits.sort();
        hits.dedup();
        if hits.len() == 1 {
            hits.pop()
        } else {
            None
        }
    }

    fn close_tools(&self, name: &str) -> Vec<String> {
        let n = name.to_lowercase();
        let mut scored: Vec<(i32, String)> = self
            .specs
            .iter()
            .filter(|s| s.name != "discover")
            .filter_map(|s| {
                let sn = s.name.to_lowercase();
                let d = edit_distance(&sn, &n);
                let score = if d <= 2 {
                    40 - d as i32 * 10
                } else if sn.contains(&n) || n.contains(&sn) {
                    15
                } else {
                    0
                };
                (score > 0).then_some((score, s.name.clone()))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        scored.truncate(4);
        scored.into_iter().map(|(_, n)| n).collect()
    }

    fn align_args(&self, name: &str, args: &Value) -> Result<(Value, String), String> {
        let Some(spec) = self.specs.iter().find(|s| s.name == name) else {
            return Ok((args.clone(), String::new()));
        };
        let props = spec
            .parameters
            .get("properties")
            .and_then(|p| p.as_object());
        let Some(props) = props else {
            return Ok((args.clone(), String::new()));
        };
        if props.is_empty() {
            return Ok((json!({}), String::new()));
        }
        let incoming = match args {
            Value::Object(m) => m.clone(),
            _ => serde_json::Map::new(),
        };
        let required: Vec<String> = spec
            .parameters
            .get("required")
            .and_then(|r| r.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        let mut known: serde_json::Map<String, Value> = serde_json::Map::new();
        let mut stray: Vec<(String, Value)> = vec![];
        let mut notes = vec![];
        for (k, v) in incoming {
            if props.contains_key(&k) {
                known.insert(k, v);
            } else if let Some(real) = props
                .keys()
                .find(|p| edit_distance(&p.to_lowercase(), &k.to_lowercase()) <= 2)
            {
                notes.push(format!("{k} is {real}"));
                known.insert(real.clone(), v);
            } else {
                stray.push((k, v));
            }
        }
        // A value that clearly belongs on a different argument (key vs model id vs provider).
        let keys: Vec<String> = known.keys().cloned().collect();
        for i in 0..keys.len() {
            for j in (i + 1)..keys.len() {
                let a = &keys[i];
                let b = &keys[j];
                let Some(va) = known.get(a).and_then(|v| v.as_str()).map(|s| s.to_string()) else {
                    continue;
                };
                let Some(vb) = known.get(b).and_then(|v| v.as_str()).map(|s| s.to_string()) else {
                    continue;
                };
                let sa = prop_score(props.get(a).unwrap(), a, &va)
                    + prop_score(props.get(b).unwrap(), b, &vb);
                let sb = prop_score(props.get(a).unwrap(), a, &vb)
                    + prop_score(props.get(b).unwrap(), b, &va);
                if sb >= sa + 4 {
                    notes.push(format!("{a} and {b} were swapped"));
                    known.insert(a.clone(), json!(vb));
                    known.insert(b.clone(), json!(va));
                }
            }
        }
        let missing: Vec<String> = required
            .iter()
            .filter(|r| !known.contains_key(*r))
            .cloned()
            .collect();
        for (k, v) in stray {
            let Some(text) = v.as_str() else {
                notes.push(format!("ignored {k}"));
                continue;
            };
            let mut best: Option<(i32, String)> = None;
            for m in &missing {
                if known.contains_key(m) {
                    continue;
                }
                let sc = prop_score(props.get(m).unwrap(), m, text);
                if best.as_ref().map(|(s, _)| sc > *s).unwrap_or(true) {
                    best = Some((sc, m.clone()));
                }
            }
            if let Some((sc, dest)) = best {
                if sc >= 4 {
                    notes.push(format!("{k} belongs in {dest}"));
                    known.insert(dest, v);
                    continue;
                }
            }
            notes.push(format!("ignored {k}"));
        }
        let missing: Vec<String> = required
            .iter()
            .filter(|r| !known.contains_key(*r))
            .cloned()
            .collect();
        if !missing.is_empty() {
            let have = known
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(", ");
            let fields = props
                .iter()
                .map(|(k, schema)| {
                    let desc = schema
                        .get("description")
                        .and_then(|d| d.as_str())
                        .unwrap_or("");
                    let req = if required.iter().any(|r| r == k) {
                        "required"
                    } else {
                        "optional"
                    };
                    if desc.is_empty() {
                        format!("{k} ({req})")
                    } else {
                        format!("{k} ({req}): {desc}")
                    }
                })
                .collect::<Vec<_>>()
                .join("; ");
            let have = if have.is_empty() {
                "(nothing)".to_string()
            } else {
                have
            };
            return Err(format!(
                "{name} needs {}. You passed {have}. Arguments: {fields}. Call {name} again with those names. Do not swap them.",
                missing.join(", ")
            ));
        }
        Ok((Value::Object(known), notes.join("; ")))
    }

    pub fn is_mcp(&self, name: &str) -> bool {
        self.kinds
            .iter()
            .zip(&self.specs)
            .any(|(k, s)| s.name == name && matches!(k, Kind::Mcp { .. }))
    }

    /// Fire a shell command as a job. Returns the job id.
    pub fn fire_bash(&mut self, args: &Value, call_id: &str) -> Result<String, String> {
        let job = {
            let mut st = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
            st.alloc("bash", "bash", call_id)?
        };
        let log = std::path::PathBuf::from(&job.log_path);
        match crate::bash::start(
            args,
            &log,
            self.jobs.clone(),
            job.id.clone(),
            self.con.clone(),
        ) {
            Ok(_) => Ok(job.id),
            Err(e) => {
                let mut st = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
                let _ = st.finish(&job.id, "error", &e);
                Err(e)
            }
        }
    }

    pub fn fire_mcp(&mut self, name: &str, args: &Value, call_id: &str) -> Result<String, String> {
        let r = self.fire_mcp_inner(name, args, call_id);
        match r {
            Ok(t) => Ok(self.with_reload_notes(t)),
            Err(e) => Err(self.with_reload_notes(e)),
        }
    }

    fn fire_mcp_inner(&mut self, name: &str, args: &Value, call_id: &str) -> Result<String, String> {
        let i = self
            .specs
            .iter()
            .position(|s| s.name == name)
            .ok_or_else(|| format!("unknown tool: {name}{}", self.unknown_hint(name)))?;
        let Kind::Mcp { server, tool } = &self.kinds[i] else {
            return Err("not an MCP tool".into());
        };
        let server = *server;
        let tool = tool.clone();
        self.loaded.insert(server);
        if let Err(e) = self.prepare_mcp(server) {
            return Err(format!("{e:#}"));
        }
        if !self.has_mcp_tool(server, &tool) {
            return Err(format!("{name} no longer exists: mcp/{} was updated", self.servers[server].name));
        }
        let mcp_name = self.servers[server].name.clone();
        let job = {
            let mut st = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
            st.alloc(name, &mcp_name, call_id)?
        };
        let s = &mut self.servers[server];
        match s.fire(&tool, args.clone(), &job.id) {
            Ok(rpc) => {
                {
                    let mut st = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
                    st.set_rpc(&job.id, rpc);
                    st.append_log(&job.id, &format!("started {name}"));
                }
                let jobs = self.jobs.clone();
                let con = self.con.clone();
                let jid = job.id.clone();
                let dur = crate::mcp::timeout_for(name);
                let _ = std::thread::Builder::new()
                    .name(format!("job-to-{jid}"))
                    .spawn(move || {
                        std::thread::sleep(dur);
                        let mut st = jobs.lock().unwrap_or_else(|e| e.into_inner());
                        if st.finish(&jid, "error", "timed out") {
                            drop(st);
                            con.push_inbound(crate::jobs::inbound(&jid, "error\ntimed out"));
                        }
                    });
                Ok(job.id)
            }
            Err(e) => {
                let mut st = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
                let _ = st.finish(&job.id, "error", &format!("{e:#}"));
                Err(format!("{e:#}"))
            }
        }
    }

    pub fn kill_job(&mut self, id: &str) -> Result<String> {
        let job = {
            let mut st = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
            st.mark_killing(id)
                .ok_or_else(|| anyhow::anyhow!("job {id} is not running"))?
        };
        if job.pid != 0 {
            // bash: the wait thread reports the result once sh is gone (at most ~2s later).
            crate::bash::kill_tree(job.pid);
            return Ok(format!(
                "killed job:{}; the result arrives as system|job:{} with killed",
                job.id, job.id
            ));
        }
        // MCP job: the server keeps running (other jobs, sessions); stop waiting for this call.
        let asked = match self.servers.iter().position(|s| s.name == job.mcp) {
            Some(i) => self.servers[i].try_cancel(job.jsonrpc_id),
            None => false,
        };
        let body = format!(
            "killed (job_kill)\n{} call abandoned{}; a late result is dropped",
            job.tool,
            if asked {
                ", the MCP was asked to cancel"
            } else {
                ""
            }
        );
        let finished = {
            let mut st = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
            let ok = st.finish(&job.id, "killed", &body);
            if ok {
                st.append_log(&job.id, "killed by job_kill");
            }
            ok
        };
        if finished {
            self.con.push_inbound(crate::jobs::inbound(&job.id, &body));
        }
        Ok(format!(
            "killed job:{} ({} call abandoned)",
            job.id, job.tool
        ))
    }

    /// Re-read mcp.json under /home/bia/MCPs. Does not start any server.
    /// Returns parse errors only.
    pub fn adopt_local_mcps(&mut self) -> Vec<String> {
        self.rescan_mcps()
    }

    /// Read /home/bia/MCPs into `catalog`. Does not spawn.
    pub(crate) fn rescan_mcps(&mut self) -> Vec<String> {
        let mut notes = vec![];
        let mut seen = std::collections::HashSet::new();
        let mut listed = vec![];
        for root in self.mcp_search_roots() {
            notes.extend(Self::incomplete_mcp_dirs(&root));
            for (dir, mf) in crate::mcp::discover(&root) {
                let folder = dir
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                let m = match mf {
                    Ok(m) => m,
                    Err(e) => {
                        notes.push(format!("MCP {folder}: {e:#}"));
                        continue;
                    }
                };
                if !seen.insert(m.name.clone()) {
                    continue;
                }
                listed.push(ServerInfo {
                    name: m.name,
                    description: m.description,
                    keywords: m.keywords,
                    examples: m.examples,
                });
            }
        }
        self.catalog = listed;
        notes
    }

    /// Describe a server, or start it when `enable` is set. None if the name is not on disk.
    fn open_mcp(&mut self, name: &str, enable: bool) -> Result<Option<String>> {
        if !enable {
            if let Some(msg) = self.start_status(name) {
                let desc = self.catalog.iter().find(|c| c.name == name).map(|c| c.description.clone()).unwrap_or_default();
                return Ok(Some(format!("mcp/{name} — {desc}\n{msg}")));
            }
            if let Some(idx) = self.server_index(name) {
                return Ok(Some(self.mcp_text(idx, false)));
            }
            if let Some(info) = self.catalog.iter().find(|c| c.name == name) {
                let desc = info.description.clone();
                return Ok(Some(format!(
                    "mcp/{name} — {desc}\ndiscover(path=\"mcp/{name}\", enable=true) starts this server and loads its schemas once. Then call each function by name."
                )));
            }
            return Ok(None);
        }
        if self.server_index(name).is_none() {
            let note = self.refresh_named(name)?;
            if self.server_index(name).is_none() {
                let _ = note;
                return Ok(None);
            }
        }
        let idx = self.server_index(name).expect("server just started");
        Ok(Some(self.enable_mcp(name, idx)?))
    }

    fn incomplete_mcp_dirs(root: &Path) -> Vec<String> {
        let mut notes = vec![];
        let Ok(rd) = std::fs::read_dir(root) else {
            return notes;
        };
        for e in rd.flatten() {
            let p = e.path();
            if !p.is_dir() {
                continue;
            }
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            if !p.join("mcp.json").exists() && p.join("run.sh").exists() {
                notes.push(format!("MCP {name}: run.sh present but no mcp.json"));
            }
        }
        notes
    }

    /// Restart MCP servers whose child process has exited.
    pub fn reap_dead_mcps(&mut self) {
        for i in 0..self.servers.len() {
            if self.servers[i].alive() {
                continue;
            }
            let name = self.servers[i].name.clone();
            let r = match self.refresh_named(&name) {
                Ok(s) if !s.is_empty() => Ok(()),
                Ok(_) => self.servers[i].respawn().and_then(|()| {
                    // the new process may bring other functions (it was updated, then it died)
                    if let Some((_, m)) = self.find_on_disk(&name) {
                        if let Some(block) = self.reload_server(i, &m)? {
                            self.reload_notes.push(block);
                        }
                    }
                    Ok(())
                }),
                Err(e) => Err(e),
            };
            match r {
                Ok(()) => self.con.writeln(
                    &self
                        .con
                        .c(crate::console::GRAY, &format!("  MCP {name} restarted")),
                ),
                Err(e) => self.con.writeln(&self.con.c(
                    crate::console::GRAY,
                    &format!("  MCP {name} restart failed: {e:#}"),
                )),
            }
        }
    }

    fn builtin(&mut self, name: &str, args: &Value) -> Result<String> {
        let s = |k: &str| args.get(k).and_then(|v| v.as_str()).map(|s| s.to_string());
        match name {
            // the agent runs it (it holds the brain and its settings): Repl::model_config_call
            "set_model_config" => bail!("set_model_config is run by the agent loop"),
            "memory_list" => crate::memory::tool_list(&self.files_dir.join("state"), args),
            "memory_get" => crate::memory::tool_get(&self.files_dir.join("state"), args),
            "memory_tree" => crate::memory::tool_tree(&self.files_dir.join("state"), args),
            "memory_messages" => crate::memory::tool_messages(&self.files_dir.join("state"), args),
            "memory_categories" => crate::memory::tool_categories(&self.files_dir.join("state")),
            "memory_reflect" => {
                self.pending_reflect.set(true);
                Ok("ok: reflecting the current live window after this turn".into())
            }
            "say" => unreachable!("handled in call()"),
            "react" | "delete_messages" | "pin_message" | "close_poll" | "chat_history" => {
                self.chat_action(name, args)
            }
            "bash" => unreachable!("handled as a job"),
            "channels" => {
                let rows: Vec<String> = self
                    .buses
                    .list()
                    .iter()
                    .map(|c| {
                        // the contact this channel is hard-linked to (web/console: by identity)
                        let who = match self.addressbook.find_for("", &c.id) {
                            Some(p) if !p.lang.is_empty() => {
                                format!("  contact={} lang={}", p.id, p.lang)
                            }
                            Some(p) => format!("  contact={}", p.id),
                            None if crate::addressbook::identity_channel(&c.id) => {
                                "  contact=<client identity>".into()
                            }
                            None => "  contact=(none)".into(),
                        };
                        format!("{}  bus={}  {}{who}", c.id, c.bus, c.title)
                    })
                    .collect();
                Ok(if rows.is_empty() {
                    "(no channels)".into()
                } else {
                    rows.join("\n")
                })
            }
            "addressbook_list" => {
                let rows: Vec<String> = self
                    .addressbook
                    .list()
                    .iter()
                    .map(|c| {
                        let lang = match (c.lang.as_str(), c.lang_set) {
                            ("", _) => String::new(),
                            (l, true) => format!("  lang={l} (set)"),
                            (l, false) => format!("  lang={l} (heard)"),
                        };
                        format!(
                            "{}  {}  <{}>  trust={}  channels=[{}]{lang}{}",
                            c.id,
                            c.name,
                            c.identifier,
                            c.trust,
                            c.channels.join(","),
                            if c.notes.is_empty() {
                                String::new()
                            } else {
                                format!("  {}", c.notes)
                            }
                        )
                    })
                    .collect();
                Ok(if rows.is_empty() {
                    "(empty)".into()
                } else {
                    rows.join("\n")
                })
            }
            "addressbook_upsert" => {
                let id = s("id").unwrap_or_default();
                if id.is_empty() {
                    bail!("id required");
                }
                let channels = args.get("channels").and_then(|v| v.as_array()).map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect::<Vec<_>>()
                });
                let c = self.addressbook.upsert(
                    &id,
                    s("name").as_deref(),
                    s("identifier").as_deref(),
                    s("trust").as_deref(),
                    channels,
                    s("notes").as_deref(),
                    s("lang").as_deref(),
                )?;
                Ok(format!(
                    "ok: {} ({}) channels=[{}]{}",
                    c.id,
                    c.name,
                    c.channels.join(","),
                    if c.lang.is_empty() {
                        String::new()
                    } else {
                        format!(" lang={}", c.lang)
                    }
                ))
            }
            "addressbook_link" => {
                let id = s("id").unwrap_or_default();
                let channel = s("channel").unwrap_or_default();
                let c = self.addressbook.link(&id, &channel)?;
                Ok(format!(
                    "ok: {channel} → {} ({}) channels=[{}]",
                    c.id,
                    c.name,
                    c.channels.join(",")
                ))
            }
            "addressbook_unlink" => {
                let channel = s("channel").unwrap_or_default();
                Ok(if self.addressbook.unlink(&channel)? {
                    format!("unlinked {channel}")
                } else {
                    format!("{channel} was not linked")
                })
            }
            "addressbook_remove" => {
                let id = s("id").unwrap_or_default();
                if self.addressbook.remove(&id)? {
                    Ok(format!("removed {id}"))
                } else {
                    Ok(format!("no contact {id}"))
                }
            }
            "schedule_create" => {
                let mut st = self.schedules.lock().unwrap_or_else(|e| e.into_inner());
                let item = st.create(&crate::schedules::NewItem {
                    title: &s("title").unwrap_or_default(),
                    body: &s("body").unwrap_or_default(),
                    at: &s("at").unwrap_or_default(),
                    kind: &s("kind").unwrap_or_else(|| "task".into()),
                    repeat: &s("repeat").unwrap_or_else(|| "none".into()),
                    cron: &s("cron").unwrap_or_default(),
                    command: &s("command").unwrap_or_default(),
                    user: &s("user").unwrap_or_default(),
                    pattern: &s("match").unwrap_or_default(),
                })?;
                Ok(crate::schedules::describe(&item))
            }
            "schedule_list" => {
                let st = self.schedules.lock().unwrap_or_else(|e| e.into_inner());
                Ok(st.format_list())
            }
            "schedule_update" => {
                let id = s("id").unwrap_or_default();
                if id.is_empty() {
                    bail!("id required");
                }
                let enabled = args.get("enabled").and_then(|v| match v {
                    Value::Bool(b) => Some(*b),
                    Value::String(s) => match s.to_ascii_lowercase().as_str() {
                        "true" | "1" | "yes" | "on" => Some(true),
                        "false" | "0" | "no" | "off" => Some(false),
                        _ => None,
                    },
                    _ => None,
                });
                let mut st = self.schedules.lock().unwrap_or_else(|e| e.into_inner());
                let item = st.update(
                    &id,
                    &crate::schedules::UpdateItem {
                        title: s("title").as_deref(),
                        body: s("body").as_deref(),
                        at: s("at").as_deref(),
                        kind: s("kind").as_deref(),
                        repeat: s("repeat").as_deref(),
                        cron: s("cron").as_deref(),
                        command: s("command").as_deref(),
                        user: s("user").as_deref(),
                        pattern: s("match").as_deref(),
                        enabled,
                    },
                )?;
                Ok(crate::schedules::describe(&item))
            }
            "schedule_delete" => {
                let id = s("id").unwrap_or_default();
                let mut st = self.schedules.lock().unwrap_or_else(|e| e.into_inner());
                if st.delete(&id)? {
                    Ok(format!("removed {id}"))
                } else {
                    Ok(format!("no schedule {id}"))
                }
            }
            "list_jobs" => {
                let list = self
                    .jobs
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .format_list();
                let sends = self.outbox.brief();
                Ok(if sends.is_empty() {
                    list
                } else {
                    format!("{list}\nsends in flight (say cancel=<id> stops one):\n{sends}")
                })
            }
            "job_logs" => {
                let id = s("id").unwrap_or_default();
                let tail = args.get("tail").and_then(|v| v.as_u64()).unwrap_or(80) as usize;
                let st = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
                st.logs(&id, tail).map_err(|e| anyhow::anyhow!("{e}"))
            }
            "job_input" => {
                let id = s("id").unwrap_or_default();
                let text = s("text").unwrap_or_default();
                if id.is_empty() {
                    bail!("id required");
                }
                if text.is_empty() {
                    bail!("text required");
                }
                let out = crate::bash::job_input(&id, &text).map_err(|e| anyhow::anyhow!("{e}"))?;
                {
                    let st = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
                    let id = id.trim().trim_start_matches(crate::jobs::PREFIX);
                    st.append_log(id, &format!("<< job_input {} byte(s)", text.len()));
                }
                Ok(out)
            }
            "job_kill" => {
                let id = s("id").unwrap_or_default();
                if id.is_empty() {
                    bail!("id required");
                }
                self.kill_job(&id)
            }
            "reboot" | "poweroff" => {
                self.pending_exit.set(Some(PendingExit {
                    poweroff: name == "poweroff",
                }));
                Ok(format!(
                    "ok: {} now, conversation saved",
                    if name == "poweroff" {
                        "powering off"
                    } else {
                        "rebooting"
                    }
                ))
            }
            "forget" => {
                self.pending_forget.set(true);
                Ok("ok: the conversation will be forgotten after this turn".into())
            }
            "invoke" => {
                bail!("invoke requires name (the prefixed function, e.g. system_fmm_status)")
            }
            _ => bail!("unknown builtin"),
        }
    }

    /// `(identity, channel)` from the arguments, falling back to the last human inbound.
    /// A channel given without an identity takes the identity of the last human inbound on
    /// that same channel (on web the channel only names a chat — every person has their own
    /// `web` — so the person must come from that chat, never from another bus).
    fn target(&self, args: &Value) -> (String, String) {
        let s = |k: &str| {
            args.get(k)
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        };
        let (def_id, def_ch) = self
            .reply_to
            .borrow()
            .clone()
            .unwrap_or_else(|| (ID_ROOT.into(), CH_CONSOLE.into()));
        let channel = s("channel").unwrap_or_else(|| def_ch.clone());
        let identity = s("identity").unwrap_or_else(|| {
            if channel == def_ch {
                def_id
            } else if let Some(id) = self.last_on.borrow().get(&channel) {
                id.clone()
            } else if is_web(&channel) {
                // no one wrote on this web chat yet: the web MCP names its users
                String::new()
            } else {
                def_id
            }
        });
        (identity, channel)
    }

    /// A human message arrived: it is the default target of the next reply.
    pub fn note_inbound(&self, identity: &str, channel: &str) {
        *self.reply_to.borrow_mut() = Some((identity.to_string(), channel.to_string()));
        self.last_on
            .borrow_mut()
            .insert(channel.to_string(), identity.to_string());
    }

    /// A say/react went out on `channel`: record it and close `answered` ids.
    fn note_reply(&mut self, channel: &str, args: &Value, result: String) -> String {
        let answered = str_list(args, "answered");
        // Before closing: a reply to a channel that had nothing open may have been
        // meant for another chat. Checked after closing, a correct answer that
        // closed its own message looked misplaced and the model replied twice.
        let elsewhere_before = self.unanswered.open_elsewhere(channel);
        let closed =
            self.unanswered
                .on_reply(channel, &answered, crate::clock::unix(crate::clock::now()));
        let missing: Vec<&String> = answered
            .iter()
            .filter(|a| !a.trim().is_empty() && !closed.iter().any(|c| c == a.trim()))
            .collect();
        let mut out = result;
        let elsewhere: Vec<String> = elsewhere_before
            .into_iter()
            .filter(|c| self.unanswered.has_open(c))
            .collect();
        if !elsewhere.is_empty() && closed.is_empty() {
            out.push_str(&format!(
                "; note: nothing was open on {channel}; open messages are on {}. If this reply was meant for one of them, send it there with that exact channel; otherwise ignore this",
                elsewhere.join(", ")
            ));
        }
        if !closed.is_empty() {
            out.push_str(&format!("; answered {}", closed.join(", ")));
        }
        if !missing.is_empty() {
            let m: Vec<&str> = missing.iter().map(|s| s.as_str()).collect();
            out.push_str(&format!("; not open: {}", m.join(", ")));
        }
        out
    }

    /// The automatic "still working on it" for a channel that ignored its nudges.
    pub fn send_fallback(&mut self, identity: &str, channel: &str) -> Result<String> {
        self.say(&json!({
            "identity": identity,
            "channel": channel,
            "message": crate::unanswered::FALLBACK_TEXT,
        }))
    }

    fn say(&mut self, args: &Value) -> Result<String> {
        let s = |k: &str| args.get(k).and_then(|v| v.as_str()).map(|s| s.to_string());
        let (identity, channel) = self.target(args);
        let message = s("message").unwrap_or_default();
        let files = str_list(args, "files");
        let bus = channel.split(':').next().unwrap_or(channel.as_str());
        self.buses.upsert(&channel, bus, "");
        let tag = if self.con.color {
            format!("{GRAY}[{identity}@{channel}]{RESET}")
        } else {
            format!("[{identity}@{channel}]")
        };
        let mut shown = message.clone();
        for f in &files {
            shown.push_str(&format!("\n[file] {f}"));
        }
        let echo = |con: &Console| {
            con.write(&format!(
                "\n{} {tag} {}{}{}\n",
                con.c(BCYAN, "<-"),
                if con.color { CYAN } else { "" },
                shown,
                RESET
            ));
        };
        if let Some(id) = s("edit")
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
        {
            return self.edit_sent(&identity, &channel, &id, &message, args, &tag);
        }
        if args.get("voice").and_then(|v| v.as_bool()).unwrap_or(false) {
            let lang = s("lang").unwrap_or_default();
            let detail = if is_web(&channel) {
                let r = self.web_invoke(
                    "send",
                    json!({"identity": identity, "channel": channel, "text": message,
                           "lang": lang, "voice": true, "reply_to": s("reply_to")}),
                )?;
                r.rsplit_once(" (")
                    .map(|(_, d)| d.trim_end_matches(')').to_string())
                    .unwrap_or(r)
            } else if let Some((mcp, chat)) = chat_bus(&channel) {
                let what = format!("voice message, {} chars", message.chars().count());
                let out = self.outbox_submit(
                    mcp,
                    chat,
                    crate::outbox::Op::Voice {
                        text: message.clone(),
                        lang,
                    },
                    0,
                    &what,
                    crate::outbox::WAIT,
                )?;
                match out {
                    crate::outbox::Outcome::Done(id, Ok(t)) => format!("id {id}; {t}"),
                    crate::outbox::Outcome::Done(_, Err(e)) => bail!("{e}"),
                    crate::outbox::Outcome::Pending(id, ahead) => pending_note(&id, &what, ahead),
                    crate::outbox::Outcome::Held(job) => held_note(mcp, &job),
                }
            } else {
                bail!("voice messages go to web, telegram, whatsapp, messenger or instagram chats, not {channel}");
            };
            let tag = if self.con.color {
                format!("{GRAY}[{identity}@{channel}]{RESET}")
            } else {
                format!("[{identity}@{channel}]")
            };
            self.con.write(&format!(
                "\n{} {tag} 🎤 {message}\n",
                self.con.c(BCYAN, "<-")
            ));
            return Ok(format!("sent to {identity}@{channel} ({detail})"));
        }
        if channel == CH_CONSOLE {
            if identity != ID_ROOT {
                bail!("identity {identity} cannot be reached on channel console; available: root");
            }
            echo(&self.con);
            return Ok(format!("sent to {identity}@{channel}"));
        }
        if is_web(&channel) {
            let mut a = json!({"identity": identity, "channel": channel, "text": message,
                               "files": files, "lang": s("lang").unwrap_or_default()});
            for k in ["reply_to", "buttons", "poll", "new_chat"] {
                if let Some(v) = args.get(k).filter(|v| !v.is_null()) {
                    a[k] = v.clone();
                }
            }
            let r = self.web_invoke("send", a)?;
            echo(&self.con);
            return Ok(r);
        }
        let detail = self.send_on_channel(
            &channel,
            &message,
            &files,
            s("lang").as_deref().unwrap_or(""),
        )?;
        echo(&self.con);
        Ok(format!("sent to {identity}@{channel} ({detail})"))
    }

    /// say with edit=<id>: replace the text of a message bia sent, on web, telegram, whatsapp, messenger or instagram.
    fn edit_sent(
        &mut self,
        identity: &str,
        channel: &str,
        id: &str,
        message: &str,
        args: &Value,
        tag: &str,
    ) -> Result<String> {
        if message.trim().is_empty() {
            bail!("edit needs the new text in message");
        }
        let has = |k: &str| match args.get(k) {
            Some(Value::Bool(b)) => *b,
            Some(Value::Array(a)) => !a.is_empty(),
            Some(Value::Null) | None => false,
            Some(_) => true,
        };
        if let Some(k) = ["files", "voice", "poll"].into_iter().find(|k| has(k)) {
            bail!("edit replaces text only; {k} cannot be combined with edit (send a new message instead)");
        }
        if is_web(channel) {
            self.web_invoke(
                "edit",
                json!({"identity": identity, "channel": channel, "message_id": id, "text": message}),
            )?;
        } else if let Some((mcp, chat)) = chat_bus(channel) {
            let r = self.outbox_action(
                mcp,
                chat,
                crate::outbox::Op::Edit {
                    id: id.into(),
                    text: message.into(),
                },
            )?;
            self.con.write(&format!(
                "\n{} {tag} ✎ {id}: {message}\n",
                self.con.c(BCYAN, "<-")
            ));
            return Ok(format!("edit of {id} on {identity}@{channel}: {r}"));
        } else {
            bail!("messages can be edited on web, telegram, whatsapp, messenger or instagram chats, not {channel}");
        }
        self.con.write(&format!(
            "\n{} {tag} ✎ {id}: {message}\n",
            self.con.c(BCYAN, "<-")
        ));
        Ok(format!("edited message {id} on {identity}@{channel}"))
    }

    fn send_on_channel(
        &mut self,
        channel: &str,
        message: &str,
        files: &[String],
        lang: &str,
    ) -> Result<String> {
        if let Some((mcp, chat)) = chat_bus(channel) {
            return self.send_on_mcp_bus(mcp, chat, message, files, lang);
        }
        bail!(
            "unknown channel {channel}; available: {}",
            self.buses.ids_csv()
        );
    }

    /// Queue an outgoing op on a telegram/whatsapp/messenger/instagram chat and wait up to `wait` (see outbox.rs).
    fn outbox_submit(
        &mut self,
        mcp: &str,
        chat: &str,
        op: crate::outbox::Op,
        bytes: u64,
        what: &str,
        wait: std::time::Duration,
    ) -> Result<crate::outbox::Outcome> {
        if self.start_status(mcp).is_some() {
            // its MCP starts in the background: hold it (never block the turn on a start)
            let job = self.starting.get(mcp).map(|s| s.job.clone()).unwrap_or_default();
            self.held.push(Held { mcp: mcp.into(), chat: chat.into(), op, bytes, what: what.into() });
            return Ok(crate::outbox::Outcome::Held(job));
        }
        let _ = self.load(mcp);
        let idx = self
            .server_index(mcp)
            .ok_or_else(|| anyhow::anyhow!("{mcp} MCP is not running"))?;
        self.loaded.insert(idx);
        self.servers[idx].ensure_alive()?;
        let link = self.servers[idx].link();
        Ok(self
            .outbox
            .submit(&format!("{mcp}:{chat}"), chat, op, link, bytes, what, wait))
    }

    /// Edit/react/delete on a telegram/whatsapp/messenger/instagram chat, in order after earlier sends there.
    fn outbox_action(&mut self, mcp: &str, chat: &str, op: crate::outbox::Op) -> Result<String> {
        match self.outbox_submit(mcp, chat, op, 0, "", crate::outbox::WAIT)? {
            crate::outbox::Outcome::Done(_, Ok(t)) => Ok(t),
            crate::outbox::Outcome::Done(_, Err(e)) => bail!("{e}"),
            crate::outbox::Outcome::Pending(_, ahead) => Ok(format!(
                "queued behind {ahead} earlier send(s) on {mcp}:{chat}; it runs when they finish (a failure arrives as system|outbox)"
            )),
            crate::outbox::Outcome::Held(job) => Ok(held_note(mcp, &job)),
        }
    }

    /// Bytes and a short description of attachments (paths as the MCPs resolve them).
    fn attachments(&self, message: &str, files: &[String]) -> (u64, String) {
        let bytes: u64 = files
            .iter()
            .map(|f| {
                let p = Path::new(f);
                let p = if p.is_absolute() {
                    p.to_path_buf()
                } else {
                    self.files_dir.join(f.trim_start_matches('/'))
                };
                std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
            })
            .sum();
        let mut what = vec![];
        if !message.trim().is_empty() {
            what.push(format!("text, {} chars", message.chars().count()));
        }
        if !files.is_empty() {
            what.push(format!("{} file(s), {}", files.len(), human(bytes)));
        }
        (bytes, what.join(" + "))
    }

    /// Shared path for telegram:/whatsapp: channels: speak on active call if any, then send the text
    /// through the outbox.
    fn send_on_mcp_bus(
        &mut self,
        mcp: &str,
        chat: &str,
        message: &str,
        files: &[String],
        lang: &str,
    ) -> Result<String> {
        let _ = self.load(mcp);
        let idx = self
            .server_index(mcp)
            .ok_or_else(|| anyhow::anyhow!("{mcp} MCP is not running"))?;
        self.loaded.insert(idx);
        // Speak it too when a call is active; "no active call" is the normal case.
        let not_spoken = if message.trim().is_empty() || !CALL_BUSES.contains(&mcp) {
            Some(String::new())
        } else {
            let (said, err) = self.mcp_invoke(
                idx,
                "call_say",
                json!({"chat_id": chat, "text": message, "lang": lang}),
                &mut |_| {},
            );
            match (err, said.contains("no active call")) {
                (false, _) => None,
                (true, true) => Some(String::new()),
                (true, false) => Some(said),
            }
        };
        let (bytes, what) = self.attachments(message, files);
        let wait = if bytes >= crate::outbox::BIG_BYTES {
            std::time::Duration::ZERO
        } else {
            crate::outbox::WAIT
        };
        let op = crate::outbox::Op::Send {
            text: message.into(),
            files: files.to_vec(),
        };
        let (text, err) = match self.outbox_submit(mcp, chat, op, bytes, &what, wait) {
            Ok(crate::outbox::Outcome::Done(id, Ok(t))) => (format!("id {id}; {t}"), false),
            Ok(crate::outbox::Outcome::Done(_, Err(e))) => (e, true),
            Ok(crate::outbox::Outcome::Pending(id, ahead)) => {
                (pending_note(&id, &what, ahead), false)
            }
            Ok(crate::outbox::Outcome::Held(job)) => (held_note(mcp, &job), false),
            Err(e) => (format!("{e:#}"), true),
        };
        match not_spoken {
            None => {
                if err {
                    return Ok(format!("spoken in call; text fallback failed: {text}"));
                }
                Ok(format!("spoken in call and sent text ({text})"))
            }
            Some(why) => {
                if err {
                    bail!("{text}");
                }
                if why.is_empty() {
                    Ok(text)
                } else {
                    Ok(format!("{text}; NOT spoken in the active call: {why}"))
                }
            }
        }
    }

    /// react / delete_messages / pin_message / close_poll / chat_history.
    fn chat_action(&mut self, name: &str, args: &Value) -> Result<String> {
        let s = |k: &str| {
            args.get(k)
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .unwrap_or_default()
        };
        let (identity, channel) = self.target(args);
        let web = is_web(&channel);
        let to = |extra: Value| {
            let mut a = json!({"identity": identity, "channel": channel});
            if let (Some(o), Some(e)) = (a.as_object_mut(), extra.as_object()) {
                o.extend(e.clone());
            }
            a
        };
        let bus = chat_bus(&channel);
        let need_web = || -> Result<()> {
            if web {
                Ok(())
            } else {
                bail!("{name} works on web chats only (channel web or web:<chat>), not {channel}")
            }
        };
        match name {
            "react" => {
                let id = s("message_id");
                let emoji = s("emoji");
                if web {
                    self.web_invoke("react", to(json!({"message_id": id, "emoji": emoji})))
                } else if let Some((mcp, chat)) = bus {
                    self.outbox_action(mcp, chat, crate::outbox::Op::React { id, emoji })
                } else {
                    bail!("cannot react on {channel}")
                }
            }
            "delete_messages" => {
                let ids = str_list(args, "message_ids");
                if ids.is_empty() {
                    bail!("message_ids required");
                }
                if web {
                    self.web_invoke("delete_messages", to(json!({"message_ids": ids})))
                } else if let Some((mcp, chat)) = bus {
                    let revoke = args.get("revoke").and_then(|v| v.as_bool()).unwrap_or(true);
                    self.outbox_action(mcp, chat, crate::outbox::Op::Delete { ids, revoke })
                } else {
                    bail!("cannot delete messages on {channel}")
                }
            }
            "pin_message" => {
                need_web()?;
                let id = s("message_id");
                let on = args.get("on").and_then(|v| v.as_bool()).unwrap_or(true);
                self.web_invoke("pin", to(json!({"message_id": id, "on": on})))
            }
            "close_poll" => {
                need_web()?;
                let id = s("message_id");
                self.web_invoke("close_poll", to(json!({"message_id": id})))
            }
            "chat_history" => {
                need_web()?;
                let n = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(30) as usize;
                self.web_invoke("history", to(json!({"limit": n})))
            }
            _ => bail!("unknown chat action"),
        }
    }

    /// Call a web MCP tool (web chats are its bus); the MCP is started on first use.
    pub fn web_invoke(&mut self, tool: &str, args: Value) -> Result<String> {
        self.await_start("web", 60)?;
        let _ = self.load("web");
        let idx = self
            .server_index("web")
            .ok_or_else(|| anyhow::anyhow!("the web MCP is not running"))?;
        let (text, err) = self.mcp_invoke(idx, tool, args, &mut |_| {});
        if err {
            bail!("{text}");
        }
        Ok(text)
    }

    /// The model took this web message: the person sees ✓✓.
    pub fn web_read(&mut self, identity: &str, channel: &str, id: &str) {
        if self.server_index("web").is_some() {
            let _ = self.web_invoke(
                "read",
                json!({"identity": identity, "channel": channel, "message_id": id}),
            );
        }
    }

    /// Web typing indicator while the model is thinking on those chats.
    pub fn set_web_typing(&mut self, chats: &[(String, String)], on: bool) {
        if self.server_index("web").is_none() {
            return;
        }
        for (identity, channel) in chats {
            let _ = self.web_invoke(
                "typing",
                json!({"identity": identity, "channel": channel, "on": on}),
            );
        }
    }

    /// Typing indicator on chat-app channels (telegram:, whatsapp:, messenger:, instagram:)
    /// while the model is thinking about them.
    pub fn set_chat_typing(&mut self, channels: &[String], on: bool) {
        for bus in CHAT_BUSES {
            let prefix = format!("{bus}:");
            if channels.iter().any(|c| c.starts_with(&prefix)) {
                self.set_mcp_typing(bus, &prefix, channels, on);
            }
        }
    }

    fn set_mcp_typing(&mut self, mcp: &str, prefix: &str, channels: &[String], on: bool) {
        let Some(idx) = self.server_index(mcp) else {
            return;
        };
        let mut seen = std::collections::HashSet::new();
        for ch in channels {
            let Some(id) = ch.strip_prefix(prefix) else {
                continue;
            };
            if id.is_empty() || !seen.insert(id) {
                continue;
            }
            let _ = self.mcp_invoke(idx, "typing", json!({"chat_id": id, "on": on}), &mut |_| {});
        }
    }
}

/// say result for a send that is still queued or uploading.
fn pending_note(id: &str, what: &str, ahead: usize) -> String {
    let ahead = if ahead > 0 {
        format!(", after {ahead} earlier send(s) to this chat")
    } else {
        String::new()
    };
    format!(
        "queued as {id} ({what}{ahead}); NOT delivered yet: [sent {id} → …] with the platform ids or [send failed {id} …] follows. \
         Use {id} as the message id for edit, react or delete_messages now — they wait for it. say(cancel=\"{id}\") stops it"
    )
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// How well a string fits one schema argument. Higher means it belongs there.
fn prop_score(schema: &serde_json::Value, name: &str, val: &str) -> i32 {
    let n = name.to_lowercase();
    let desc = schema
        .get("description")
        .and_then(|d| d.as_str())
        .unwrap_or("")
        .to_lowercase();
    let v = val.trim();
    let mut s = 0i32;
    if let Some(en) = schema.get("enum").and_then(|e| e.as_array()) {
        if en.iter().any(|x| x.as_str() == Some(v)) {
            s += 12;
        } else if !en.is_empty() {
            s -= 4;
        }
    }
    let keyish = n.contains("key") || n.contains("token") || n.contains("secret");
    let modelish = n == "model" || desc.contains("model id");
    let providerish = n == "provider" || desc.contains("openrouter");
    if keyish {
        if v.starts_with("sk-") || v.starts_with("sk_") {
            s += 10;
        } else if v.contains('/') {
            s -= 4;
        }
    }
    if modelish {
        if v.contains('/')
            || ["grok", "claude", "gpt", "deepseek", "qwen"]
                .iter()
                .any(|m| v.contains(m))
        {
            s += 8;
        }
        if v.starts_with("sk-") {
            s -= 8;
        }
    }
    if providerish {
        if matches!(
            v,
            "openrouter" | "openai" | "anthropic" | "xai" | "deepseek" | "qwen" | "chatgpt"
        ) {
            s += 10;
        } else {
            s -= 3;
        }
    }
    s
}

fn str_list(args: &Value, k: &str) -> Vec<String> {
    match args.get(k) {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|x| match x {
                Value::String(s) => Some(s.trim().to_string()),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            })
            .filter(|s| !s.is_empty())
            .collect(),
        Some(Value::String(s)) if !s.trim().is_empty() => vec![s.trim().to_string()],
        _ => vec![],
    }
}

pub fn human(bytes: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1000.0 && i < U.len() - 1 {
        v /= 1000.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.2} {}", U[i])
    }
}

#[cfg(test)]
mod web_tests {
    use super::*;

    /// A stand-in web MCP: answers initialize and tools/list, logs every tools/call line.
    const FAKE: &str = r#"#!/bin/sh
log="$(dirname "$0")/calls.log"
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"serverInfo":{"name":"web","version":"t"}}}\n' "$id" ;;
    *'"method":"tools/list"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[{"name":"send","description":"d","inputSchema":{"type":"object"}},{"name":"chats","description":"d","inputSchema":{"type":"object"}}]}}\n' "$id" ;;
    *'"method":"tools/call"'*)
      printf '%s\n' "$line" >> "$log"
      printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[{"type":"text","text":"sent (message id b1)"}]}}\n' "$id" ;;
  esac
done
"#;

    /// A script MCP (`/bin/sh server.sh`): its tools as one tools/list JSON array.
    fn script_mcp(tools: &str) -> String {
        format!(
            r#"while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*) printf '{{"jsonrpc":"2.0","id":%s,"result":{{"serverInfo":{{"name":"tun","version":"t"}}}}}}\n' "$id" ;;
    *'"method":"tools/list"'*) printf '{{"jsonrpc":"2.0","id":%s,"result":{{"tools":{tools}}}}}\n' "$id" ;;
    *'"method":"tools/call"'*) printf '{{"jsonrpc":"2.0","id":%s,"result":{{"content":[{{"type":"text","text":"done"}}]}}}}\n' "$id" ;;
  esac
done
"#
        )
    }

    /// Rewrite the script with a later mtime (edits within one second must count too).
    fn edit_script(path: &Path, text: &str, ahead: u64) {
        std::fs::write(path, text).unwrap();
        let t = std::time::SystemTime::now() + std::time::Duration::from_secs(ahead);
        std::fs::File::options().write(true).open(path).unwrap().set_modified(t).unwrap();
    }

    /// role and call id of each message, the shape chat APIs check
    fn shape(h: &[Msg]) -> Vec<String> {
        h.iter()
            .map(|m| match m.role {
                Role::User => "user".to_string(),
                Role::Assistant => format!("assistant[{}]", m.tool_calls.iter().map(|c| c.id.as_str()).collect::<Vec<_>>().join(",")),
                Role::Tool => format!("tool:{}", m.call_id),
            })
            .collect()
    }

    fn call(id: &str, name: &str) -> ToolCall {
        ToolCall { id: id.into(), name: name.into(), args: json!({}) }
    }

    #[test]
    fn say_rounds_are_free_however_they_are_called() {
        let say = ToolCall { id: "a".into(), name: "say".into(), args: json!({"message": "hi"}) };
        let via_discover = ToolCall { id: "b".into(), name: "discover".into(), args: json!({"call": "say", "arguments": {"message": "status"}}) };
        let via_invoke = ToolCall { id: "c".into(), name: "invoke".into(), args: json!({"name": "say", "arguments": {"message": "x"}}) };
        let work = ToolCall { id: "d".into(), name: "discover".into(), args: json!({"call": "memory_list", "arguments": {}}) };
        assert!(only_say(&[say.clone(), via_discover.clone(), via_invoke]));
        assert!(!only_say(&[via_discover, work.clone()]));
        assert!(!only_say(&[work]));
        assert!(!only_say(&[ToolCall { id: "e".into(), name: "discover".into(), args: json!({"q": "say"}) }]), "a search is a round");
    }

    #[test]
    fn a_cut_off_call_is_not_run() {
        let dir = std::env::temp_dir().join(format!("bia-cut-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("files").join("state")).unwrap();
        let mut t = Tools::new(&dir.join("files"), Console::for_test());
        let cut = json!({CUT_ARGS: {"stop": "length", "received": "{\"command\":\"ec", "bytes": 15}});
        let e = t.guide_call("bash", &cut).unwrap_err();
        assert!(e.contains("15 bytes") && e.contains("output limit"), "{e}");
        assert!(t.guide_call("bash", &json!({"command": "true"})).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn model_config_arguments_are_checked() {
        assert_eq!(parse_model_change(&json!({})).unwrap(), ModelChange::default());
        let ch = parse_model_change(&json!({"context": 65536, "max_tool_rounds": "64", "temperature": 0.7, "top_k": 40})).unwrap();
        assert_eq!((ch.context, ch.max_tool_rounds, ch.temperature, ch.top_k), (Some(65536), Some(64), Some(0.7), Some(40)));
        assert!(ch.touches_sampling());
        let mut s = crate::config::SamplingCfg::default();
        ch.apply_sampling(&mut s);
        assert_eq!((s.temperature, s.top_k, s.min_p), (0.7, 40, crate::config::SamplingCfg::default().min_p));
        assert!(!parse_model_change(&json!({"max_tool_rounds": 8})).unwrap().touches_sampling());
        for (bad, why) in [
            (json!({"context": 100}), "between 512"),
            (json!({"max_tool_rounds": 1000}), "between 4"),
            (json!({"temperature": 3}), "between 0"),
            (json!({"top_k": 2.5}), "whole number"),
            (json!({"tokens": 4096}), "unknown setting tokens"),
            (json!({"top_p": "lots"}), "must be a number"),
        ] {
            let e = parse_model_change(&bad).unwrap_err();
            assert!(e.contains(why), "{bad}: {e}");
        }
    }

    #[test]
    fn unanswered_tool_calls_are_repaired_for_chat_apis() {
        let f = ToolFormat::Hermes;
        // the round limit left two calls without results; the next inbound came after them
        let h = vec![
            Msg::user("hi".into()),
            Msg::assistant(String::new(), vec![call("a", "say"), call("b", "bash")]),
            Msg::user("next message".into()),
        ];
        let n = normalize_history(f, &h);
        assert_eq!(shape(&n), ["user", "assistant[a,b]", "tool:a", "tool:b", "user"]);
        assert!(n[2].content.starts_with("not run:"), "{}", n[2].content);

        // a message that came between two results waits until both are in
        let h = vec![
            Msg::assistant(String::new(), vec![call("a", "say"), call("b", "bash")]),
            Msg::tool("say".into(), "a".into(), "ok".into()),
            Msg::user("system notice".into()),
            Msg::tool("bash".into(), "b".into(), "done".into()),
            Msg::assistant("fine".into(), vec![]),
        ];
        let n = normalize_history(f, &h);
        assert_eq!(shape(&n), ["assistant[a,b]", "tool:a", "tool:b", "user", "assistant[]"]);
        assert_eq!(n[2].content, "done");

        // a result whose call is gone becomes text; a call still open at the end is answered
        let h = vec![
            Msg::user("x".into()),
            Msg::tool("bash".into(), "gone".into(), "old output".into()),
            Msg::assistant(String::new(), vec![call("c", "bash")]),
        ];
        let n = normalize_history(f, &h);
        assert_eq!(shape(&n), ["user", "user", "assistant[c]", "tool:c"]);
        assert!(n[1].content.starts_with("[result of bash] old output"));

        // a well-formed history is left as it is
        let h = vec![
            Msg::user("q".into()),
            Msg::assistant(String::new(), vec![call("a", "say")]),
            Msg::tool("say".into(), "a".into(), "sent".into()),
            Msg::assistant("done".into(), vec![]),
        ];
        assert_eq!(shape(&normalize_history(f, &h)), shape(&h));
    }

    #[test]
    fn a_repeated_call_gets_the_next_step() {
        let dir = std::env::temp_dir().join(format!("bia-repeat-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("files").join("state")).unwrap();
        let mut t = Tools::new(&dir.join("files"), Console::for_test());
        let mut log = |_: &str| {};
        // a search naming one function exactly leads with how to load and call it
        let (r, err) = t.call("discover", &json!({"q": "bash"}), &mut log);
        assert!(!err && r.starts_with("bash is in shell: discover(path=\"shell\", enable=true)"), "{r}");
        let (r, err) = t.call("discover", &json!({"path": "shell", "q": "bash"}), &mut log);
        assert!(!err && r.contains("\"command\""), "path + a function in it enables that path: {r}");
        let q = json!({"path": "shell", "q": "bash"});
        let (_, _, note) = t.guide_call("discover", &q).unwrap();
        assert!(note.is_empty(), "{note}");
        let (_, _, note) = t.guide_call("discover", &q).unwrap();
        // shell is enabled now: call bash, not "enable shell" (which contradicted "already enabled")
        assert!(note.starts_with("[repeat]") && note.contains("(2x)") && note.contains("bash is already callable"), "{note}");
        assert!(!note.contains("enable=true"), "{note}");
        // asking for an enabled function again gives its schema, not "already enabled"
        let (r, err) = t.call("discover", &json!({"q": "bash"}), &mut log);
        assert!(!err && r.starts_with("bash is ready") && r.contains("\"command\""), "{r}");
        let (r, _) = t.call("discover", &json!({"path": "shell", "q": "bash"}), &mut log);
        assert!(r.starts_with("bash is ready"), "{r}");
        // after compaction the next enable sends the schemas again
        let (r, _) = t.call("discover", &json!({"path": "shell", "enable": true}), &mut log);
        assert!(r.contains("already enabled"), "{r}");
        t.forget_schemas();
        let (r, _) = t.call("discover", &json!({"path": "shell", "enable": true}), &mut log);
        assert!(!r.contains("already enabled") && r.contains("\"command\""), "{r}");
        // any other tool: the generic next step; different arguments are not a repeat
        let say = json!({"message": "hi"});
        t.guide_call("say", &say).unwrap();
        let (_, _, note) = t.guide_call("say", &say).unwrap();
        assert!(note.contains("[repeat]") && note.contains("next step"), "{note}");
        let (_, _, note) = t.guide_call("say", &json!({"message": "hello"})).unwrap();
        assert!(!note.contains("[repeat]"), "{note}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn script_mcp_restart_resends_only_changed_blocks() {
        let dir = std::env::temp_dir().join(format!("bia-reload-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mcp = dir.join("MCPs").join("tun");
        std::fs::create_dir_all(&mcp).unwrap();
        std::fs::create_dir_all(dir.join("files").join("state")).unwrap();
        let one = r#"[{"name":"status","description":"tunnel status","inputSchema":{"type":"object"}}]"#;
        let two = r#"[{"name":"status","description":"tunnel status","inputSchema":{"type":"object"}},{"name":"route","description":"add a route","inputSchema":{"type":"object"}}]"#;
        std::fs::write(mcp.join("server.sh"), script_mcp(one)).unwrap();
        std::fs::write(mcp.join("mcp.json"), r#"{"name":"tun","command":"/bin/sh","args":["server.sh"]}"#).unwrap();
        let new_tools = || {
            let mut t = Tools::new(&dir.join("files"), Console::for_test());
            t.mcp_env = vec![("BIA_MCPS".into(), dir.join("MCPs").to_string_lossy().into())];
            t.mcp_log_dir = dir.clone();
            t
        };
        let mut t = new_tools();
        let mut log = |_: &str| {};
        let restart = json!({"path": "mcp/tun", "restart": true});
        let (r, err) = t.call("discover", &json!({"path": "mcp/tun", "enable": true}), &mut log);
        assert!(!err && r.contains("tun_status") && !r.contains("tun_route"), "{r}");
        let (r, err) = t.call("tun_status", &json!({}), &mut log);
        assert!(!err && !r.contains("changed"), "{r}");

        // the script gains a function: nothing restarts by itself
        edit_script(&mcp.join("server.sh"), &script_mcp(two), 5);
        let (r, err) = t.call("tun_status", &json!({}), &mut log);
        assert!(!err && r == "done", "no automatic restart: {r}");
        let (r, err) = t.call("tun_route", &json!({}), &mut log);
        assert!(err && r.contains("unknown tool"), "{r}");
        // the model restarts it: the new block, once
        let (r, err) = t.call("discover", &restart, &mut log);
        assert!(!err && r.contains("restarted mcp/tun") && r.contains("mcp/tun changed") && r.contains("tun_route"), "{r}");
        let (r, err) = t.call("tun_route", &json!({}), &mut log);
        assert!(!err && r == "done", "{r}");

        // restarted again with the same functions (a newer build that changed nothing the model sees): no block
        edit_script(&mcp.join("server.sh"), &format!("# v2\n{}", script_mcp(two)), 10);
        let (r, err) = t.call("discover", &restart, &mut log);
        assert!(!err && r.contains("functions are unchanged") && !r.contains("\"parameters\""), "{r}");

        // a function goes away; the process dies and comes back by itself: its block comes with the next result
        edit_script(&mcp.join("server.sh"), &script_mcp(one), 15);
        t.servers[0].kill();
        t.reap_dead_mcps();
        let (r, err) = t.call("tun_status", &json!({}), &mut log);
        assert!(!err && r.contains("mcp/tun changed") && !r.contains("tun_route") && r.ends_with("done"), "{r}");
        let (r, err) = t.call("tun_route", &json!({}), &mut log);
        assert!(err && r.contains("unknown tool"), "{r}");
        for s in &mut t.servers {
            s.kill();
        }

        // the agent restarts and resumes: the kept hash shows whether the model's copy is current
        let mut t = new_tools();
        t.ensure_loaded("tun").unwrap();
        let (r, _) = t.call("tun_status", &json!({}), &mut log);
        assert!(!r.contains("changed"), "unchanged since the last block: {r}");
        for s in &mut t.servers {
            s.kill();
        }
        edit_script(&mcp.join("server.sh"), &script_mcp(two), 20);
        let mut t = new_tools();
        t.ensure_loaded("tun").unwrap();
        let (r, _) = t.call("tun_status", &json!({}), &mut log);
        assert!(r.contains("mcp/tun changed") && r.contains("tun_route"), "changed while the agent was off: {r}");
        for s in &mut t.servers {
            s.kill();
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The fake script MCP, answering initialize only after `delay` seconds.
    fn slow_mcp(delay: u32) -> String {
        script_mcp(r#"[{"name":"status","description":"s","inputSchema":{"type":"object"}}]"#)
            .replacen("*'\"method\":\"initialize\"'*) ", &format!("*'\"method\":\"initialize\"'*) sleep {delay}; "), 1)
    }

    fn wait_start_done(t: &mut Tools, name: &str) {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while t.starting.get(name).is_some_and(|s| !s.handle.is_finished()) {
            assert!(std::time::Instant::now() < end, "start of {name} did not finish");
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    #[test]
    fn background_start_job_answers_every_command_while_starting() {
        let dir = std::env::temp_dir().join(format!("bia-startjob-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (name, cmd) in [("slow", r#""/bin/sh","args":["server.sh"]"#), ("broken", r#""/nonexistent/server""#)] {
            let mcp = dir.join("MCPs").join(name);
            std::fs::create_dir_all(&mcp).unwrap();
            std::fs::write(mcp.join("server.sh"), slow_mcp(2).replace("\"tun\"", &format!("\"{name}\""))).unwrap();
            std::fs::write(mcp.join("mcp.json"), format!(r#"{{"name":"{name}","command":{cmd}}}"#)).unwrap();
        }
        std::fs::create_dir_all(dir.join("files").join("state")).unwrap();
        let mut t = Tools::new(&dir.join("files"), Console::for_test());
        t.mcp_env = vec![("BIA_MCPS".into(), dir.join("MCPs").to_string_lossy().into())];
        t.mcp_log_dir = dir.clone();
        let mut log = |_: &str| {};

        let r = t.start_job("slow").unwrap();
        let job = r.rsplit("job:").next().unwrap().trim().to_string();
        assert!(r.contains("is starting as job:") && job.len() == 4, "{r}");
        // every command while it starts says so, with the job, and none of them waits
        let t0 = std::time::Instant::now();
        let again = t.start_job("slow").unwrap();
        assert!(again.contains(&format!("already: mcp/slow is starting in job:{job}")), "{again}");
        for args in [json!({"path": "mcp/slow", "enable": true}), json!({"path": "mcp/slow", "restart": true})] {
            let (r, err) = t.call("discover", &args, &mut log);
            assert!(err && r.contains(&format!("is starting in job:{job}")) && r.contains("job_kill"), "{args}: {r}");
        }
        let (r, err) = t.call("discover", &json!({"path": "mcp/slow"}), &mut log);
        assert!(!err && r.contains(&format!("starting in job:{job}")), "describe: {r}");
        let (r, err) = t.call("slow_status", &json!({}), &mut log);
        assert!(err && r.contains(&format!("is starting in job:{job}")), "call: {r}");
        assert!(t0.elapsed() < std::time::Duration::from_secs(1), "a command waited for the start");

        // a send to it meanwhile is held, not refused and not waited for
        let op = crate::outbox::Op::Send { text: "hi".into(), files: vec![] };
        match t.outbox_submit("slow", "c1", op, 0, "text, 2 chars", crate::outbox::WAIT).unwrap() {
            crate::outbox::Outcome::Held(j) => assert_eq!(j, job),
            _ => panic!("not held"),
        }
        assert!(held_note("slow", &job).contains(&format!("job:{job}")));
        assert!(t0.elapsed() < std::time::Duration::from_secs(1), "a command waited for the start");

        // done: attached before the model reads its job result; callable; a new start says running
        wait_start_done(&mut t, "slow");
        let mut batch = vec![crate::jobs::inbound(&job, "ok\nmcp/slow started")];
        t.poll_starts(&mut batch);
        assert!(t.held.is_empty(), "the held send went into the outbox");
        let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut notes = vec![];
        while notes.is_empty() && std::time::Instant::now() < end {
            notes = t.outbox.take_notes();
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert!(notes.iter().any(|n| n.contains("slow:c1")), "the held send was sent: {notes:?}");
        let (r, err) = t.call("slow_status", &json!({}), &mut log);
        assert!(!err && r == "done", "{r}");
        assert!(t.start_job("slow").unwrap().contains("already running"));
        let (r, _) = t.call("discover", &json!({"path": "mcp/slow", "enable": true}), &mut log);
        assert!(r.contains("already enabled"), "{r}");

        // a start that fails: its job says why; the next enable tries again (and fails again)
        let r = t.start_job("broken").unwrap();
        let bjob = r.rsplit("job:").next().unwrap().trim().to_string();
        let op = crate::outbox::Op::Send { text: "lost".into(), files: vec![] };
        assert!(matches!(t.outbox_submit("broken", "c2", op, 0, "text, 4 chars", crate::outbox::WAIT).unwrap(), crate::outbox::Outcome::Held(_)));
        wait_start_done(&mut t, "broken");
        let (r, _) = t.call("discover", &json!({"path": "mcp/broken"}), &mut log);
        assert!(!r.contains("is starting") && t.held.is_empty(), "the held send failed with its start: {r}");
        let state = t.jobs.lock().unwrap().get(&bjob).map(|j| j.state.clone());
        assert_eq!(state.as_deref(), Some("error"));
        let (r, err) = t.call("discover", &json!({"path": "mcp/broken", "enable": true}), &mut log);
        assert!(err && !r.contains("is starting"), "{r}");

        // job_kill gives a start up: forgotten at once, and enable starts it fresh
        for s in &mut t.servers {
            s.kill();
        }
        let mut t2 = Tools::new(&dir.join("files"), Console::for_test());
        t2.mcp_env = t.mcp_env.clone();
        t2.mcp_log_dir = dir.clone();
        let r = t2.start_job("slow").unwrap();
        let kjob = r.rsplit("job:").next().unwrap().trim().to_string();
        let k = t2.kill_job(&kjob).unwrap();
        assert!(k.contains("killed"), "{k}");
        let (r, err) = t2.call("discover", &json!({"path": "mcp/slow"}), &mut log);
        assert!(!err && !r.contains("is starting"), "killed start is forgotten: {r}");
        let (r, err) = t2.call("discover", &json!({"path": "mcp/slow", "enable": true}), &mut log);
        assert!(!err && r.contains("slow_status"), "fresh start after job_kill: {r}");
        wait_start_done(&mut t2, "slow");
        for s in &mut t2.servers {
            s.kill();
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn tools_with_fake_web(tag: &str) -> (PathBuf, Tools) {
        let dir = std::env::temp_dir().join(format!("bia-webtools-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mcp = dir.join("MCPs").join("web");
        std::fs::create_dir_all(&mcp).unwrap();
        std::fs::create_dir_all(dir.join("files").join("state")).unwrap();
        std::fs::write(mcp.join("fake.sh"), FAKE).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(mcp.join("fake.sh"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
        std::fs::write(
            mcp.join("mcp.json"),
            r#"{"name":"web","command":"fake.sh"}"#,
        )
        .unwrap();
        let mut t = Tools::new(&dir.join("files"), Console::for_test());
        t.mcp_env = vec![("BIA_MCPS".into(), dir.join("MCPs").to_string_lossy().into())];
        t.mcp_log_dir = dir.clone();
        (dir, t)
    }

    fn calls(dir: &Path) -> Vec<Value> {
        std::fs::read_to_string(dir.join("MCPs/web/calls.log"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .map(|v| v["params"].clone())
            .collect()
    }

    #[test]
    fn default_target_comes_from_the_same_channel() {
        let t = Tools::new(&std::env::temp_dir(), Console::for_test());
        // nothing yet: root on the console
        assert_eq!(t.target(&json!({})), ("root".into(), "console".into()));
        // a channel nobody wrote on: web needs the person, others keep the old default
        assert_eq!(
            t.target(&json!({"channel": "web"})),
            ("".into(), "web".into())
        );
        t.note_inbound("alex", "web");
        t.note_inbound("Bob", "telegram:42");
        // the last human inbound is the default for both
        assert_eq!(t.target(&json!({})), ("Bob".into(), "telegram:42".into()));
        // web without identity: who wrote on web, never the telegram person or bia
        assert_eq!(
            t.target(&json!({"channel": "web"})),
            ("alex".into(), "web".into())
        );
        // an explicit identity always wins
        assert_eq!(
            t.target(&json!({"channel": "web", "identity": "root"})),
            ("root".into(), "web".into())
        );
        // web:<chat> is its own channel
        assert_eq!(
            t.target(&json!({"channel": "web:trip"})),
            ("".into(), "web:trip".into())
        );
        t.note_inbound("carol", "web:trip");
        assert_eq!(
            t.target(&json!({"channel": "web:trip"})),
            ("carol".into(), "web:trip".into())
        );
    }

    #[test]
    fn say_on_web_goes_to_the_web_mcp_with_the_person() {
        // The 1.1.0 regression: Alex writes on web, then something arrives on another bus,
        // and the reply on web must still go to alex (not to bia or the other sender).
        let (dir, mut t) = tools_with_fake_web("say");
        t.note_inbound("alex", "web");
        t.note_inbound("Bob", "telegram:42");
        let r = t
            .say(&json!({"channel": "web", "message": "hello Alex"}))
            .unwrap();
        assert!(r.contains("message id b1"), "{r}");
        let c = calls(&dir);
        assert_eq!(c.len(), 1, "{c:?}");
        assert_eq!(c[0]["name"], "send");
        assert_eq!(c[0]["arguments"]["identity"], "alex");
        assert_eq!(c[0]["arguments"]["channel"], "web");
        assert_eq!(c[0]["arguments"]["text"], "hello Alex");
        // the model answers with say: web_send is the agent's own path, never a model tool
        let names: Vec<&str> = t.specs.iter().map(|s| s.name.as_str()).collect();
        assert!(
            names.contains(&"web_chats") && !names.contains(&"web_send"),
            "{names:?}"
        );

        // the chat actions address the same person and chat through the MCP
        t.chat_action(
            "react",
            &json!({"channel": "web", "message_id": "b9", "emoji": "👍"}),
        )
        .unwrap();
        t.chat_action(
            "delete_messages",
            &json!({"channel": "web", "message_ids": ["b9"]}),
        )
        .unwrap();
        let c = calls(&dir);
        assert_eq!(
            (c[1]["name"].as_str(), c[2]["name"].as_str()),
            (Some("react"), Some("delete_messages"))
        );
        assert!(c[1..].iter().all(|x| x["arguments"]["identity"] == "alex"));
    }

    #[test]
    fn web_call_tools_are_not_agent_builtins() {
        let t = Tools::new(&std::env::temp_dir(), Console::for_test());
        let names: Vec<&str> = t.specs.iter().map(|s| s.name.as_str()).collect();
        assert!(
            names.contains(&"say") && names.contains(&"react"),
            "{names:?}"
        );
        assert!(!names.iter().any(|n| n.starts_with("web_")), "{names:?}");
    }
}
