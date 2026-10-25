//! Poll Chrome's remote-debugging `/json/list` and push inbound events for tab/window open+close,
//! and for native file dialogs opening.
use bia_mcp::Ctx;
use serde_json::Value;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::{dialog, proc};

const POLL: Duration = Duration::from_secs(1);

#[derive(Clone)]
struct Page {
    title: String,
    url: String,
}

#[derive(Clone)]
pub struct Watcher {
    ctx: Arc<Mutex<Option<Ctx>>>,
}

impl Watcher {
    pub fn start() -> Self {
        let w = Watcher {
            ctx: Arc::new(Mutex::new(None)),
        };
        let run = w.clone();
        thread::Builder::new()
            .name("chrome-tab-watch".into())
            .spawn(move || run.loop_forever())
            .ok();
        w
    }

    pub fn set_ctx(&self, ctx: Ctx) {
        *self.ctx.lock().unwrap() = Some(ctx);
    }

    fn loop_forever(&self) {
        let mut known: HashMap<String, Page> = HashMap::new();
        let mut seeded = false;
        let mut dialogs = dialog::Seen::default();
        loop {
            thread::sleep(POLL);
            let ctx = self.ctx.lock().unwrap().clone();
            dialogs.poll(ctx.as_ref());
            if !proc::debug_ready() {
                // Chrome stopped / debugging down — don't spam [tab closed] for every page.
                known.clear();
                seeded = false;
                continue;
            }
            let Ok(list) = json_list() else {
                continue;
            };
            let pages = page_entries(&list);
            if !seeded {
                known = pages;
                seeded = true;
                continue;
            }
            for (id, page) in &pages {
                if !known.contains_key(id) {
                    self.notify("opened", &page.title, &page.url);
                }
            }
            for (id, page) in &known {
                if !pages.contains_key(id) {
                    self.notify("closed", &page.title, &page.url);
                }
            }
            // Refresh cached title/url for still-open pages (navigations).
            known = pages;
        }
    }

    fn notify(&self, kind: &str, title: &str, url: &str) {
        let title = title.trim();
        let url = url.trim();
        let msg = if title.is_empty() || title == url {
            format!("[tab {kind}] {url}")
        } else {
            format!("[tab {kind}] {title}\n{url}")
        };
        if let Some(ctx) = self.ctx.lock().unwrap().clone() {
            ctx.inbound("system", "chrome", "", &msg);
        }
    }
}

fn page_entries(list: &Value) -> HashMap<String, Page> {
    let mut out = HashMap::new();
    let Some(arr) = list.as_array() else {
        return out;
    };
    for t in arr {
        let ty = t.get("type").and_then(|v| v.as_str()).unwrap_or("");
        // Tabs and windows show up as "page"; keep "app" PWAs too.
        if ty != "page" && ty != "app" {
            continue;
        }
        let Some(id) = t.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        let title = t.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let url = t.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string();
        if url.starts_with("devtools://") || url.starts_with("chrome-devtools://") {
            continue;
        }
        out.insert(id.to_string(), Page { title, url });
    }
    out
}

fn json_list() -> Result<Value, String> {
    let body = http_get(proc::DEBUG_PORT, "/json/list")?;
    serde_json::from_str(&body).map_err(|e| e.to_string())
}

fn http_get(port: u16, path: &str) -> Result<String, String> {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(400))
        .map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|e| e.to_string())?;
    write!(
        stream,
        "GET {path} HTTP/1.0\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
    )
    .map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&buf);
    let body = text
        .split("\r\n\r\n")
        .nth(1)
        .or_else(|| text.split("\n\n").nth(1))
        .ok_or_else(|| "no http body".to_string())?;
    Ok(body.to_string())
}
