//! Human-in-the-loop: wait until someone finishes a task on the web console.
use bia_mcp::Ctx;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn files_dir() -> PathBuf {
    PathBuf::from(std::env::var("BIA_FILES").unwrap_or_else(|_| "/home/bia/files".into()))
}

fn hitl_dir() -> PathBuf {
    files_dir().join("state").join("hitl")
}

fn dl_dir() -> PathBuf {
    files_dir().join("state").join("dl")
}

fn new_id() -> String {
    let mut b = [0u8; 4];
    if let Ok(mut f) = fs::File::open("/dev/urandom") {
        use std::io::Read;
        let _ = f.read_exact(&mut b);
    } else {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u32)
            .unwrap_or(1);
        b = n.to_le_bytes();
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn console_urls() -> Vec<String> {
    let p = files_dir().join("state").join("web.json");
    let Ok(raw) = fs::read_to_string(p) else {
        return vec![];
    };
    let Ok(v) = serde_json::from_str::<Value>(&raw) else {
        return vec![];
    };
    let port = v
        .get("port")
        .and_then(|x| x.as_u64())
        .filter(|n| *n > 0 && *n <= 65535)
        .map(|n| n as u16);
    if let Some(port) = port {
        let live = crate::listen::https_origins(port);
        if !live.is_empty() {
            return live;
        }
    }
    v.get("urls")
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

fn with_status(body: String) -> String {
    let p = files_dir().join("state").join("status.txt");
    let st = fs::read_to_string(p).unwrap_or_default();
    let st = st.trim();
    if st.is_empty() {
        body
    } else {
        format!("{body}\n\n{st}")
    }
}

fn with_path(url: &str, path: &str) -> String {
    if let Some((base, q)) = url.split_once('?') {
        format!("{}{path}?{q}", base.trim_end_matches('/'))
    } else {
        format!("{}{path}", url.trim_end_matches('/'))
    }
}

pub fn web_console(_: &Ctx, _: Value) -> Result<String, String> {
    let p = files_dir().join("state").join("web.json");
    let raw = fs::read_to_string(&p).ok();
    let v: Option<Value> = raw.as_deref().and_then(|s| serde_json::from_str(s).ok());
    let err = v
        .as_ref()
        .and_then(|x| x.get("error"))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim();
    let port = v
        .as_ref()
        .and_then(|x| x.get("port"))
        .and_then(|x| x.as_u64())
        .filter(|n| *n > 0 && *n <= 65535)
        .map(|n| n as u16)
        .unwrap_or(0);
    let ca = v
        .as_ref()
        .and_then(|x| x.get("ca_sha256"))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if port == 0 {
        if !err.is_empty() {
            return Err(format!("web console is not listening: {err}"));
        }
        return Err("web console has no address yet (no network, or not started)".into());
    }
    let urls = console_urls();
    if urls.is_empty() {
        return Err(format!(
            "web console is on :{port} (mTLS). CA at /v1/pki/ca. No non-loopback IPv4 yet — wait for dhcp, then call web_console again."
        ));
    }
    let mut lines = urls.clone();
    lines.push("auth: mtls (client cert)".into());
    lines.push(format!("CA (public): {}/v1/pki/ca", urls[0].trim_end_matches('/')));
    if !ca.is_empty() {
        lines.push(format!("ca_sha256: {ca}"));
    }
    lines.push(format!(
        "discovery: {}/.well-known/bia.json",
        urls[0].trim_end_matches('/')
    ));
    lines.push("issue a client cert with pki_issue when root asks; pickup URL is one-time".into());
    Ok(with_status(lines.join("\n")))
}

fn files_root() -> Result<PathBuf, String> {
    let raw = files_dir();
    raw.canonicalize()
        .map_err(|e| format!("{}: {e}", raw.display()))
}

/// Path under $BIA_FILES. Absolute paths must stay inside that directory.
fn file_rel(raw: &str) -> Result<(String, String), String> {
    let root = files_root()?;
    let given = raw.trim();
    if given.is_empty() {
        return Err("path is required".into());
    }
    let candidate = if given.starts_with('/') {
        PathBuf::from(given)
    } else {
        root.join(given.trim_start_matches('/'))
    };
    let abs = candidate
        .canonicalize()
        .map_err(|e| format!("{}: {e}", candidate.display()))?;
    if !abs.starts_with(&root) {
        return Err("path escapes /home/bia/files".into());
    }
    let rel = abs
        .strip_prefix(&root)
        .map_err(|_| "path escapes /home/bia/files".to_string())?
        .to_string_lossy()
        .to_string();
    let name = abs
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("file")
        .to_string();
    Ok((rel, name))
}

pub fn share(ctx: &Ctx, args: Value) -> Result<String, String> {
    let path = bia_mcp::arg_str(&args, "path")?;
    let (rel, name) = file_rel(&path)?;
    let meta = fs::metadata(files_dir().join(&rel)).map_err(|e| format!("{rel}: {e}"))?;
    if !meta.is_file() {
        return Err(format!("{rel} is not a file"));
    }
    let id = new_id();
    let dir = dl_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    let rec = json!({
        "id": id,
        "path": rel,
        "name": name,
        "created": SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
    });
    let path = dir.join(format!("{id}.json"));
    fs::write(
        &path,
        serde_json::to_vec_pretty(&rec).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    unsafe { libc::sync() };
    let links: Vec<String> = {
        let u = console_urls();
        if u.is_empty() {
            vec![format!("/dl/{id} (web console, mTLS)")]
        } else {
            u.into_iter()
                .map(|u| with_path(&u, &format!("/dl/{id}")))
                .collect()
        }
    };
    ctx.log(format!("download {id}: {rel}"));
    let mut out = Vec::new();
    for u in &links {
        ctx.log(format!("open {u}"));
        if u.starts_with("http://") || u.starts_with("https://") {
            out.push(format!("[{name}]({u})"));
        } else {
            out.push(u.clone());
        }
    }
    Ok(format!(
        "send these download links to the user with say, unchanged (recipient needs a client cert):\n{}",
        out.join("\n")
    ))
}

pub fn ask(ctx: &Ctx, args: Value) -> Result<String, String> {
    let title = bia_mcp::arg_str(&args, "title")?.trim().to_string();
    if title.is_empty() {
        return Err("title is required".into());
    }
    let message = bia_mcp::arg_str_opt(&args, "message")
        .unwrap_or_default()
        .trim()
        .to_string();
    let timeout = bia_mcp::arg_u32_opt(&args, "timeout_secs").unwrap_or(0);
    let id = new_id();
    let dir = hitl_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    let task = json!({
        "id": id,
        "title": title,
        "message": message,
        "status": "pending",
        "created": SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
    });
    let path = dir.join(format!("{id}.json"));
    fs::write(
        &path,
        serde_json::to_vec_pretty(&task).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    unsafe { libc::sync() };
    ctx.log(format!("human task {id}: {title}"));
    let hitl_urls: Vec<String> = {
        let u = console_urls();
        if u.is_empty() {
            vec![format!("/hitl/{id} (web console, mTLS)")]
        } else {
            u.into_iter()
                .map(|u| with_path(&u, &format!("/hitl/{id}")))
                .collect()
        }
    };
    for u in &hitl_urls {
        ctx.log(format!(
            "open {u} — the page shows the machine display; do the work there, then Done"
        ));
    }
    ctx.log("display: the task page embeds /gui (VNC 127.0.0.1:5900 of display :0)");
    let result_path = dir.join(format!("{id}.result"));
    let start = Instant::now();
    let cleanup = || {
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&result_path);
    };
    loop {
        ctx.check_cancel()?;
        if result_path.is_file() {
            let raw = fs::read_to_string(&result_path).unwrap_or_default();
            let v: Value = serde_json::from_str(&raw).unwrap_or(json!({}));
            let status = v
                .get("status")
                .and_then(|x| x.as_str())
                .unwrap_or("done");
            let note = v
                .get("note")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            cleanup();
            if status == "failed" || status == "cancelled" {
                let why = if note.is_empty() {
                    status.to_string()
                } else {
                    format!("{status}: {note}")
                };
                return Err(format!("human {why}"));
            }
            return Ok(if note.is_empty() {
                format!("human completed task {id} ({title})")
            } else {
                format!("human completed task {id} ({title}): {note}")
            });
        }
        if timeout > 0 && start.elapsed() >= Duration::from_secs(timeout as u64) {
            cleanup();
            return Err(format!("human did not finish in {timeout}s"));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}
