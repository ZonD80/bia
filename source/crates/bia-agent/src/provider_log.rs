//! files/state/provider.jsonl: the last KEEP calls to the cloud brain, in full, one JSON line
//! each (newest last). For turns that end with no text and no tool call: it shows what the
//! provider actually sent — HTTP status and headers, every raw stream line (finish_reason,
//! usage, comments, the end marker or its absence), the text and reasoning as assembled, the
//! tool calls as parsed, the error, and timings.
//!
//! The request is not kept whole (it is the whole conversation each time): its size, tool
//! names and the last REQUEST_TAIL messages, images replaced by their size. The file holds
//! message bodies, so it is 0600 and stays on the machine.

use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

pub const KEEP: usize = 20;
pub const FILE: &str = "provider.jsonl";
/// Messages from the end of the request that are kept.
const REQUEST_TAIL: usize = 4;
/// Raw stream bytes kept per call; past that lines are only counted.
const RAW_MAX: usize = 4 * 1024 * 1024;

static DIR: OnceLock<PathBuf> = OnceLock::new();
static WRITE: Mutex<()> = Mutex::new(());

/// Turn the log on: calls are written to `<state>/provider.jsonl`. Not called in tests, so
/// captures there are never written.
pub fn init(state: &Path) {
    let _ = DIR.set(state.to_path_buf());
}

#[derive(Default)]
pub struct Capture {
    started: Option<Instant>,
    ts: String,
    provider: String,
    model: String,
    url: String,
    request: Value,
    attempts: Vec<Value>,
    status: Option<u16>,
    headers: Value,
    first_byte_ms: Option<u128>,
    first_piece_ms: Option<u128>,
    raw: Vec<String>,
    raw_bytes: usize,
    raw_dropped: usize,
    saw_end: bool,
    reasoning: String,
}

fn ms(t: Option<Instant>) -> Option<u128> {
    t.map(|t| t.elapsed().as_millis())
}

/// Images and other large base64 blobs → their size.
fn strip_blobs(v: &mut Value) {
    match v {
        Value::String(s) => {
            let blob = s.starts_with("data:") && s.len() > 256
                || s.len() > 4096 && !s.bytes().any(|b| b.is_ascii_whitespace());
            if blob {
                *s = format!("[{} bytes omitted]", s.len());
            }
        }
        Value::Array(a) => a.iter_mut().for_each(strip_blobs),
        Value::Object(o) => o.values_mut().for_each(strip_blobs),
        _ => {}
    }
}

impl Capture {
    pub fn new(provider: &str, model: &str) -> Capture {
        Capture {
            started: Some(Instant::now()),
            ts: crate::clock::rfc3339_now(),
            provider: provider.into(),
            model: model.into(),
            ..Default::default()
        }
    }

    /// The request about to be sent (the last one of a retried call wins).
    pub fn request(&mut self, url: &str, body: &Value) {
        self.url = url.into();
        let list = ["messages", "input"].iter().find_map(|k| body.get(*k).and_then(|m| m.as_array()));
        let mut tail: Vec<Value> = list
            .map(|l| l[l.len().saturating_sub(REQUEST_TAIL)..].to_vec())
            .unwrap_or_default();
        tail.iter_mut().for_each(strip_blobs);
        let tools: Vec<Value> = body
            .get("tools")
            .and_then(|t| t.as_array())
            .map(|a| {
                a.iter()
                    .flat_map(|t| match t.get("tools").and_then(|x| x.as_array()) {
                        Some(inner) => inner.clone(),
                        None => vec![t.clone()],
                    })
                    .filter_map(|t| {
                        t.pointer("/function/name").or_else(|| t.get("name")).cloned()
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut opts = body.clone();
        if let Some(o) = opts.as_object_mut() {
            for k in ["messages", "input", "tools", "system", "instructions"] {
                o.remove(k);
            }
        }
        let system_len = ["system", "instructions"]
            .iter()
            .filter_map(|k| body.get(*k))
            .map(|s| s.to_string().len())
            .sum::<usize>();
        self.request = json!({
            "bytes": body.to_string().len(),
            "messages": list.map(|l| l.len()).unwrap_or(0),
            "system_len": system_len,
            "options": opts,
            "tools": tools,
            "tail": tail,
        });
    }

    /// The response arrived (status and headers). A second call means a retry.
    pub fn response(&mut self, status: u16, headers: &ureq::http::HeaderMap) {
        if let Some(s) = self.status {
            self.attempts.push(json!({"status": s, "headers": self.headers.take()}));
        }
        self.status = Some(status);
        self.first_byte_ms = ms(self.started);
        let mut h = serde_json::Map::new();
        for (k, v) in headers {
            h.insert(k.to_string(), json!(v.to_str().unwrap_or("?")));
        }
        self.headers = Value::Object(h);
    }

    /// The body of a refused request (status >= 400).
    pub fn error_body(&mut self, text: &str) {
        self.line(text);
    }

    /// One raw stream line, as received.
    pub fn line(&mut self, line: &str) {
        let d = line.strip_prefix("data:").map(str::trim).unwrap_or("");
        if d == "[DONE]"
            || d.contains("\"type\":\"message_stop\"")
            || d.contains("\"type\":\"response.completed\"")
            || d.contains("\"type\":\"response.incomplete\"")
        {
            self.saw_end = true;
        }
        if self.raw_bytes + line.len() > RAW_MAX {
            self.raw_dropped += 1;
            return;
        }
        self.raw_bytes += line.len();
        self.raw.push(line.to_string());
    }

    /// When the first text or reasoning piece came (ms) and the reasoning as streamed.
    pub fn pieces(&mut self, first_ms: Option<u128>, reasoning: String) {
        self.first_piece_ms = first_ms;
        self.reasoning = reasoning;
    }

    pub fn finish(self, out: &anyhow::Result<crate::remote::RemoteResult>) {
        let Some(dir) = DIR.get() else { return };
        let result = match out {
            Ok(r) => json!({
                "stop": r.stop,
                "text": r.text,
                "text_len": r.text.chars().count(),
                "calls": r.calls.iter().map(|c| json!({"id": c.id, "name": c.name, "args": c.args})).collect::<Vec<_>>(),
                "input_tokens": r.input_tokens,
                "output_tokens": r.output_tokens,
                "empty": r.text.trim().is_empty() && r.calls.is_empty(),
            }),
            Err(e) => json!({"error": format!("{e:#}")}),
        };
        let entry = json!({
            "ts": self.ts,
            "provider": self.provider,
            "model": self.model,
            "url": self.url,
            "ms": {"first_byte": self.first_byte_ms, "first_piece": self.first_piece_ms, "total": ms(self.started)},
            "status": self.status,
            "headers": self.headers,
            "retries": self.attempts,
            "result": result,
            "reasoning": self.reasoning,
            "reasoning_len": self.reasoning.chars().count(),
            "stream": {"saw_end": self.saw_end, "lines": self.raw.len() + self.raw_dropped, "bytes": self.raw_bytes, "dropped_lines": self.raw_dropped},
            "request": self.request,
            "raw": self.raw,
        });
        let _ = append(dir, &entry.to_string());
    }
}

/// Add one line, keep the KEEP newest (written to a temp file, then renamed).
fn append(dir: &Path, line: &str) -> std::io::Result<()> {
    let _g = WRITE.lock().unwrap_or_else(|e| e.into_inner());
    let path = dir.join(FILE);
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<&str> = old.lines().filter(|l| !l.trim().is_empty()).collect();
    lines.push(line);
    let keep = &lines[lines.len().saturating_sub(KEEP)..];
    let tmp = dir.join(format!(".{FILE}.tmp"));
    {
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        let mut f = o.open(&tmp)?;
        for l in keep {
            f.write_all(l.as_bytes())?;
            f.write_all(b"\n")?;
        }
    }
    std::fs::rename(&tmp, &path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_newest_and_strips_images() {
        let dir = std::env::temp_dir().join(format!("bia-provlog-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..KEEP + 5 {
            append(&dir, &json!({"n": i}).to_string()).unwrap();
        }
        let text = std::fs::read_to_string(dir.join(FILE)).unwrap();
        let ns: Vec<u64> = text.lines().map(|l| serde_json::from_str::<Value>(l).unwrap()["n"].as_u64().unwrap()).collect();
        assert_eq!(ns.len(), KEEP);
        assert_eq!(ns[0], 5);
        assert_eq!(*ns.last().unwrap(), (KEEP + 4) as u64);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join(FILE)).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        let mut c = Capture::new("openrouter", "m");
        let img = format!("data:image/png;base64,{}", "A".repeat(5000));
        c.request(
            "u",
            &json!({"model": "m", "messages": [{"role":"system","content":"s"}, {"role":"user","content":[{"type":"image_url","image_url":{"url": img}}]}], "tools": [{"type":"function","function":{"name":"say"}}]}),
        );
        assert_eq!(c.request["tools"], json!(["say"]));
        assert!(c.request["tail"][1]["content"][0]["image_url"]["url"].as_str().unwrap().contains("omitted"));
        c.line("data: {\"choices\":[{\"finish_reason\":\"stop\"}]}");
        assert!(!c.saw_end);
        c.line("data: [DONE]");
        assert!(c.saw_end);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A real call through Remote::generate against a local server that streams a turn with
    /// reasoning, finish_reason "stop", no text, no tool call and no [DONE].
    #[test]
    fn empty_turn_is_logged_in_full() {
        use std::io::{BufRead, BufReader, Read};
        let dir = std::env::temp_dir().join(format!("bia-provlog-e2e-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        init(&dir);
        let srv = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = srv.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut s, _) = srv.accept().unwrap();
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut len = 0;
            loop {
                let mut l = String::new();
                r.read_line(&mut l).unwrap();
                if let Some(v) = l.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap();
                }
                if l == "\r\n" {
                    break;
                }
            }
            let mut body = vec![0; len];
            r.read_exact(&mut body).unwrap();
            let sse = ": OPENROUTER PROCESSING\n\n\
data: {\"id\":\"gen-1\",\"choices\":[{\"delta\":{\"reasoning\":\"thinking it over\"}}]}\n\n\
data: {\"id\":\"gen-1\",\"choices\":[{\"delta\":{\"content\":\"\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":9,\"completion_tokens\":4}}\n\n";
            let head = format!("HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nx-request-id: req-42\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", sse.len());
            s.write_all(head.as_bytes()).unwrap();
            s.write_all(sse.as_bytes()).unwrap();
        });
        let c: crate::remote::FmmConfig = serde_json::from_value(
            json!({"provider": "openai", "api_key": "k", "model": "m1", "base_url": format!("http://127.0.0.1:{port}")}),
        )
        .unwrap();
        let r = crate::remote::Remote::from_config(&c).unwrap();
        let out = r
            .generate("sys", &[], &[crate::tools::Msg::user("hello".into())], &mut |_, _| true, false, std::time::Duration::from_secs(10))
            .unwrap();
        assert_eq!(out.stop, "stop");
        let text = std::fs::read_to_string(dir.join(FILE)).unwrap();
        let e: Value = serde_json::from_str(text.lines().last().unwrap()).unwrap();
        assert_eq!(e["status"], 200);
        assert_eq!(e["headers"]["x-request-id"], "req-42");
        assert_eq!(e["result"]["empty"], true);
        assert_eq!(e["result"]["stop"], "stop");
        assert_eq!(e["result"]["output_tokens"], 4);
        assert_eq!(e["reasoning"], "thinking it over");
        assert_eq!(e["stream"]["saw_end"], false);
        assert_eq!(e["raw"][0], ": OPENROUTER PROCESSING");
        assert_eq!(e["request"]["tail"][1]["content"], "hello");
        assert!(e["ms"]["first_piece"].is_u64());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
