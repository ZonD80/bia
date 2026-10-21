//! Minimal MCP server over stdio (JSON-RPC 2.0, one message per line).
//!
//! Supports: initialize, notifications/initialized, ping, tools/list, tools/call,
//! notifications/cancelled, and server -> client `notifications/message` for live
//! progress. tools/call runs on a worker thread so a second request can arrive
//! (and be cancelled) while a long handler is running.

pub mod lanes;
pub mod md;
pub mod share;

use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub type ToolResult = Result<String, String>;
pub type Handler = Box<dyn Fn(&Ctx, Value) -> ToolResult + Send + Sync>;

pub struct Tool {
    pub name: String,
    pub description: String,
    pub schema: Value,
    pub handler: Handler,
}

/// Handle given to tool handlers for emitting progress lines to the agent console.
#[derive(Clone)]
pub struct Ctx {
    out: Arc<Mutex<Box<dyn Write + Send>>>,
    request_id: Option<u64>,
    cancelled: Arc<AtomicBool>,
    /// Console mode: log lines are plain text, not JSON-RPC notifications.
    plain: bool,
}

impl Ctx {
    fn send(&self, v: &Value) {
        let mut o = self.out.lock().unwrap();
        let _ = writeln!(o, "{}", v);
        let _ = o.flush();
    }
    fn note(&self, data: Value) {
        let mut params = json!({"level": "info", "data": data});
        if let Some(id) = self.request_id {
            params["requestId"] = json!(id);
        }
        self.send(&json!({
            "jsonrpc": "2.0",
            "method": "notifications/message",
            "params": params
        }));
    }
    /// Log a progress/info line. Shown live on the agent console (and the job log).
    pub fn log(&self, msg: impl AsRef<str>) {
        if self.plain {
            let mut o = self.out.lock().unwrap();
            let _ = writeln!(o, "{}", msg.as_ref());
            let _ = o.flush();
            return;
        }
        self.note(json!(msg.as_ref()));
    }
    pub fn plain(&self) -> bool {
        self.plain
    }
    /// Logs go to stderr as plain lines. Used by `bia-mcp-carry --land`.
    pub fn console() -> Self {
        Ctx {
            out: Arc::new(Mutex::new(Box::new(std::io::stderr()))),
            request_id: None,
            cancelled: Arc::new(AtomicBool::new(false)),
            plain: true,
        }
    }
    /// A message that arrived on a bus/channel. The agent queues it as
    /// `identity|channel|issued|delivered|id|message`. Pass `id` empty when the bus has none.
    pub fn inbound(&self, identity: &str, channel: &str, id: &str, message: &str) {
        self.inbound_at(identity, channel, id, message, 0);
    }
    /// [`Ctx::inbound`] with when the message was written (Unix seconds, from the
    /// platform), so the envelope's `issued` is not the time bia received it — e.g.
    /// for messages that came while bia was off. 0: issued is when the agent gets it.
    pub fn inbound_at(&self, identity: &str, channel: &str, id: &str, message: &str, issued: i64) {
        let mut v = json!({
            "t": "inbound",
            "identity": identity,
            "channel": channel,
            "id": id,
            "message": message
        });
        if issued > 0 {
            v["issued"] = json!(issued);
        }
        self.note(v);
    }
    /// Progress in percent for long operations.
    pub fn progress(&self, pct: u32, msg: impl AsRef<str>) {
        let line = format!("[{:3}%] {}", pct, msg.as_ref());
        if self.plain {
            self.log(line);
            return;
        }
        self.note(json!(line));
    }
    /// True after the client sent `notifications/cancelled` for this tools/call.
    pub fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
    pub fn check_cancel(&self) -> Result<(), String> {
        if self.cancelled() {
            Err("cancelled".into())
        } else {
            Ok(())
        }
    }
}

pub struct Server {
    name: String,
    version: String,
    tools: Vec<Tool>,
}

impl Server {
    pub fn new(name: &str, version: &str) -> Self {
        Server { name: name.into(), version: version.into(), tools: vec![] }
    }

    pub fn tool<F>(mut self, name: &str, description: &str, schema: Value, f: F) -> Self
    where
        F: Fn(&Ctx, Value) -> ToolResult + Send + Sync + 'static,
    {
        self.tools.push(Tool {
            name: name.into(),
            description: description.into(),
            schema,
            handler: Box::new(f),
        });
        self
    }

    fn handle_sync(self: &Arc<Self>, _ctx: &Ctx, req: &Value) -> Option<Value> {
        let id = req.get("id").cloned();
        let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let result: Result<Value, (i64, String)> = match method {
            "initialize" => Ok(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": self.name, "version": self.version}
            })),
            "notifications/initialized" => return None,
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({
                "tools": self.tools.iter().map(|t| json!({
                    "name": t.name, "description": t.description, "inputSchema": t.schema
                })).collect::<Vec<_>>()
            })),
            _ => Err((-32601, format!("method not found: {method}"))),
        };
        let id = id?;
        Some(match result {
            Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
            Err((code, msg)) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": msg}}),
        })
    }

    fn call_tool(self: &Arc<Self>, ctx: &Ctx, name: &str, args: Value) -> Value {
        match self.tools.iter().find(|t| t.name == name) {
            None => json!({"content": [{"type": "text", "text": format!("unknown tool: {name}")}], "isError": true}),
            Some(t) => {
                if ctx.cancelled() {
                    return json!({"content": [{"type": "text", "text": "cancelled"}], "isError": true});
                }
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (t.handler)(ctx, args)));
                let (text, is_err) = match r {
                    Ok(Ok(s)) => (s, false),
                    Ok(Err(e)) => (e, true),
                    Err(_) => ("tool panicked".to_string(), true),
                };
                json!({"content": [{"type": "text", "text": text}], "isError": is_err})
            }
        }
    }

    /// Serve until stdin closes.
    pub fn run(self) {
        self.run_with(|_| {});
    }

    /// Like `run`, but `on_start` gets a Ctx clone (e.g. to drive a background poll thread).
    pub fn run_with(self, on_start: impl FnOnce(Ctx)) {
        let out: Arc<Mutex<Box<dyn Write + Send>>> = Arc::new(Mutex::new(Box::new(std::io::stdout())));
        let ctx0 = Ctx { out: out.clone(), request_id: None, cancelled: Arc::new(AtomicBool::new(false)), plain: false };
        on_start(ctx0.clone());
        let server = Arc::new(self);
        let inflight: Arc<Mutex<HashMap<u64, Arc<AtomicBool>>>> = Arc::new(Mutex::new(HashMap::new()));
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let req: Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(e) => {
                    ctx0.send(&json!({"jsonrpc": "2.0", "id": null,
                        "error": {"code": -32700, "message": format!("parse error: {e}")}}));
                    continue;
                }
            };
            let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
            if method == "notifications/cancelled" {
                let rid = json_id(req.pointer("/params/requestId"));
                if let Some(rid) = rid {
                    if let Some(flag) = inflight.lock().unwrap().get(&rid) {
                        flag.store(true, Ordering::Relaxed);
                    }
                }
                continue;
            }
            if method == "notifications/initialized" {
                continue;
            }
            if method == "tools/call" {
                let Some(id) = json_id(req.get("id")) else { continue };
                let params = req.get("params").cloned().unwrap_or(Value::Null);
                let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                let flag = Arc::new(AtomicBool::new(false));
                inflight.lock().unwrap().insert(id, flag.clone());
                let ctx = Ctx { out: out.clone(), request_id: Some(id), cancelled: flag, plain: false };
                let srv = server.clone();
                let inflight_c = inflight.clone();
                let _ = std::thread::Builder::new().name(format!("mcp-call-{id}")).spawn(move || {
                    let result = srv.call_tool(&ctx, &name, args);
                    ctx.send(&json!({"jsonrpc": "2.0", "id": id, "result": result}));
                    inflight_c.lock().unwrap().remove(&id);
                });
                continue;
            }
            if let Some(resp) = server.handle_sync(&ctx0, &req) {
                ctx0.send(&resp);
            }
        }
    }
}

fn json_id(v: Option<&Value>) -> Option<u64> {
    let v = v?;
    v.as_u64().or_else(|| v.as_i64().and_then(|n| u64::try_from(n).ok())).or_else(|| v.as_str()?.parse().ok())
}

/// Helper: read a required string argument.
pub fn arg_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key).and_then(|v| v.as_str()).ok_or_else(|| format!("missing argument: {key}"))
}
pub fn arg_str_opt<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(|v| v.as_str())
}
pub fn arg_bool(args: &Value, key: &str, default: bool) -> bool {
    args.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
}
pub fn arg_u32_opt(args: &Value, key: &str) -> Option<u32> {
    args.get(key).and_then(|v| v.as_u64().or_else(|| v.as_i64().and_then(|n| u64::try_from(n).ok()))).map(|n| n as u32)
}
pub fn arg_str_list(args: &Value, key: &str) -> Vec<String> {
    args.get(key).and_then(|v| v.as_array()).map(|a| {
        a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()
    }).unwrap_or_default()
}
