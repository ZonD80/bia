//! Human-in-the-loop tasks: list/complete files written by system_ask_human.
use crate::web;
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

const PAGE: &str = include_str!("../web/hitl.html");

pub fn dir(files: &Path) -> PathBuf {
    files.join("state").join("hitl")
}

fn safe_id(id: &str) -> Option<&str> {
    let id = id.trim();
    if (4..=16).contains(&id.len()) && id.chars().all(|c| c.is_ascii_alphanumeric()) {
        Some(id)
    } else {
        None
    }
}

pub fn split_path(path: &str) -> Option<(String, String)> {
    let rest = path.strip_prefix("/hitl/")?;
    let (id, extra) = rest.split_once('/').unwrap_or((rest, ""));
    let id = safe_id(id)?;
    Some((id.to_string(), extra.to_string()))
}

pub fn api_finish_id(path: &str) -> Option<&str> {
    let rest = path.strip_prefix("/v1/hitl/")?;
    let (id, extra) = rest.split_once('/')?;
    if extra == "finish" {
        safe_id(id)
    } else {
        None
    }
}

pub fn list(files: &Path) -> Value {
    let mut tasks = vec![];
    if let Ok(rd) = std::fs::read_dir(dir(files)) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !name.ends_with(".json") {
                continue;
            }
            let id = name.trim_end_matches(".json").to_string();
            if dir(files).join(format!("{id}.result")).exists() {
                continue;
            }
            let Ok(b) = std::fs::read(e.path()) else {
                continue;
            };
            let Ok(mut v) = serde_json::from_slice::<Value>(&b) else {
                continue;
            };
            v["id"] = json!(id);
            tasks.push(v);
        }
    }
    tasks.sort_by(|a, b| {
        let ta = a.get("created").and_then(|x| x.as_u64()).unwrap_or(0);
        let tb = b.get("created").and_then(|x| x.as_u64()).unwrap_or(0);
        ta.cmp(&tb)
    });
    json!({"tasks": tasks})
}

pub fn get(files: &Path, id: &str) -> Option<Value> {
    let id = safe_id(id)?;
    let p = dir(files).join(format!("{id}.json"));
    let b = std::fs::read(p).ok()?;
    let mut v: Value = serde_json::from_slice(&b).ok()?;
    v["id"] = json!(id);
    v["done"] = json!(dir(files).join(format!("{id}.result")).exists());
    Some(v)
}

pub fn finish(files: &Path, id: &str, ok: bool, note: &str) -> Result<(), String> {
    let id = safe_id(id).ok_or_else(|| "bad id".to_string())?;
    if get(files, id).is_none() {
        return Err("no such task".into());
    }
    let p = dir(files).join(format!("{id}.result"));
    let body = json!({"status": if ok { "done" } else { "failed" }, "note": note});
    std::fs::write(
        &p,
        serde_json::to_vec_pretty(&body).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    unsafe { libc::sync() };
    Ok(())
}

pub fn page() -> &'static str {
    PAGE
}

pub fn dl_parts(path: &str) -> Option<(String, bool)> {
    let rest = path.strip_prefix("/dl/")?;
    let (id, extra) = rest.split_once('/').unwrap_or((rest, ""));
    let id = safe_id(id)?;
    if extra.is_empty() {
        return Some((id.to_string(), false));
    }
    if extra == "file" {
        return Some((id.to_string(), true));
    }
    None
}

fn under_files(files: &Path, rel: &str) -> Option<PathBuf> {
    if rel.is_empty() || rel.starts_with('/') {
        return None;
    }
    let mut p = files.to_path_buf();
    for c in Path::new(rel).components() {
        match c {
            Component::Normal(s) => p.push(s),
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(p)
}

fn html_escape(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '&' => "&amp;".to_string(),
            '<' => "&lt;".to_string(),
            '>' => "&gt;".to_string(),
            '"' => "&quot;".to_string(),
            _ => c.to_string(),
        })
        .collect()
}

/// Relative path stored by system_share_file, if the record and the file still exist.
pub fn dl_open(files: &Path, id: &str) -> Option<(String, String, u64)> {
    let id = safe_id(id)?;
    let p = files.join("state").join("dl").join(format!("{id}.json"));
    let b = std::fs::read(p).ok()?;
    let v: Value = serde_json::from_slice(&b).ok()?;
    let rel = v.get("path").and_then(|x| x.as_str()).unwrap_or("").trim();
    let file = under_files(files, rel)?;
    let meta = std::fs::metadata(&file).ok()?;
    if !meta.is_file() {
        return None;
    }
    let name = v
        .get("name")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| file.file_name().and_then(|s| s.to_str()).unwrap_or("file"))
        .to_string();
    Some((rel.to_string(), name, meta.len()))
}

pub fn dl_page(id: &str, name: &str, rel: &str, size: u64) -> String {
    let name = html_escape(name);
    let rel = html_escape(rel);
    format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>bia · {name}</title>
<style>
body {{ margin:0; background:#0c0e12; color:#e8edf5; font:15px/1.4 ui-sans-serif, system-ui, sans-serif; }}
main {{ max-width:36rem; margin:10vh auto; padding:24px; }}
a {{ display:inline-block; margin-top:16px; background:#1558b0; color:#fff; text-decoration:none; padding:10px 16px; border-radius:8px; }}
p {{ color:#8b95a7; }}
</style></head>
<body><main>
<h1>{name}</h1>
<p>{rel} · {size} bytes</p>
<a href="/dl/{id}/file" download="{name}">Download</a>
</main></body></html>
"#
    )
}

pub fn proxy(stream: &mut dyn Write, files: &Path, id: &str, extra: &str, query: &str) {
    let Some(task) = get(files, id) else {
        web::respond(stream, 404, "text/plain", b"no such task", None);
        return;
    };
    let url = task
        .get("url")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim();
    let Some((host, port, default_path)) = parse_http_url(url) else {
        web::respond(
            stream,
            400,
            "text/plain",
            b"task has no local http url",
            None,
        );
        return;
    };
    if !is_private_host(&host) {
        web::respond(
            stream,
            400,
            "text/plain",
            b"refusing to proxy a non-local url",
            None,
        );
        return;
    }
    let rest = extra.strip_prefix("site").unwrap_or(extra);
    let path = if rest.is_empty() || rest == "/" {
        default_path
    } else {
        rest.to_string()
    };
    let q = if query.is_empty() {
        String::new()
    } else {
        format!("?{query}")
    };
    let full = format!("http://{host}:{port}{path}{q}");
    let cfg = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .http_status_as_error(false)
        .build();
    let agent: ureq::Agent = cfg.into();
    match agent.get(&full).call() {
        Ok(mut resp) => {
            let status = resp.status().as_u16();
            let ctype = resp
                .headers()
                .get(ureq::http::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("text/html; charset=utf-8")
                .to_string();
            let body = resp
                .body_mut()
                .with_config()
                .limit(8 * 1024 * 1024)
                .read_to_vec()
                .unwrap_or_default();
            web::respond(stream, status, &ctype, &body, None);
        }
        Err(e) => {
            let msg = format!("proxy {full}: {e}");
            web::respond(
                stream,
                502,
                "text/plain; charset=utf-8",
                msg.as_bytes(),
                None,
            );
        }
    }
}

fn parse_http_url(url: &str) -> Option<(String, u16, String)> {
    let rest = url.strip_prefix("http://")?;
    let (hostport, path) = match rest.split_once('/') {
        Some((h, p)) => (h, format!("/{p}")),
        None => (rest, "/".into()),
    };
    let (host, port) = split_hostport(hostport).ok()?;
    Some((host, port, path))
}

fn split_hostport(hostport: &str) -> Result<(String, u16), String> {
    if let Some((h, p)) = hostport.rsplit_once(':') {
        Ok((
            h.trim_matches(|c| c == '[' || c == ']').to_string(),
            p.parse::<u16>().map_err(|_| "bad port".to_string())?,
        ))
    } else {
        Ok((hostport.to_string(), 80))
    }
}

fn is_private_host(host: &str) -> bool {
    let h = host.trim_matches(|c| c == '[' || c == ']');
    if h.eq_ignore_ascii_case("localhost") || h == "127.0.0.1" || h == "::1" {
        return true;
    }
    if let Ok(ip) = h.parse::<std::net::Ipv4Addr>() {
        return ip.is_loopback() || ip.is_private() || ip.is_link_local();
    }
    false
}
