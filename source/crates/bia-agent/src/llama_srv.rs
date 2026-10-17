//! Prebuilt `llama-server` for CPU, CUDA, ROCm, SYCL and Vulkan.
//!
//! Official Ubuntu builds from ggml-org/llama.cpp are separate glibc binaries with
//! their own GPU stacks. The agent keeps inference out-of-process on 127.0.0.1 so
//! CUDA/Vulkan/ROCm/SYCL backends can be swapped without relinking the agent.
//! GPU backends are tried first; CPU is the fallback.
use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::config::SamplingCfg;
use crate::hw;
use crate::tune::LlamaPlan;

const GPU_ORDER: &[&str] = &["cuda", "rocm", "sycl", "vulkan"];

#[derive(Clone)]
struct Launch {
    bin: PathBuf,
    dir: PathBuf,
    name: String,
    model: PathBuf,
    ctx: u32,
    threads: i32,
    threads_batch: i32,
    n_batch: u32,
    n_ubatch: u32,
    load_mode: String,
}

pub struct Server {
    pub name: String,
    child: Child,
    port: u16,
    launch: Launch,
    log: Arc<Mutex<Vec<String>>>,
    live: bool,
}

pub struct Completion {
    pub text: String,
    pub generated: usize,
    pub reused: usize,
    pub prefill_secs: f64,
    pub gen_secs: f64,
    pub eos: bool,
    pub truncated: bool,
    pub interrupted: bool,
}

/// Start the best prebuilt server. GPU backends first, then the CPU build.
/// `BIA_LLAMA_CPU` skips the GPU binaries.
pub fn start(
    model: &Path,
    plan: &LlamaPlan,
    progress: &mut impl FnMut(f32) -> bool,
) -> Result<(Server, Vec<String>)> {
    let gpus = hw::display_gpus();
    let real: Vec<_> = gpus.iter().filter(|g| !hw::virtual_gpu(g.vendor)).collect();
    let mut notes = Vec::new();
    let mut order: Vec<&str> = Vec::new();
    if std::env::var_os("BIA_LLAMA_CPU").is_some() {
        notes.push("GPU servers skipped (BIA_LLAMA_CPU)".into());
    } else if real.is_empty() {
        if !gpus.is_empty() {
            notes.push("display GPU is virtual".into());
        }
    } else {
        order.extend(GPU_ORDER);
    }
    order.push("cpu");
    let mut saw_bin = false;
    for name in order {
        let Some(dir) = backend_dir(name) else {
            continue;
        };
        let bin = dir.join("llama-server");
        if !bin.is_file() {
            continue;
        }
        saw_bin = true;
        if name != "cpu" {
            if let Err(why) = gate(name, &real) {
                notes.push(format!("{name}: {why}"));
                continue;
            }
        }
        let launch = Launch {
            bin,
            dir,
            name: name.into(),
            model: model.to_path_buf(),
            ctx: plan.ctx,
            threads: plan.threads,
            threads_batch: plan.threads_batch,
            n_batch: plan.n_batch,
            n_ubatch: plan.n_ubatch,
            load_mode: load_mode(plan),
        };
        match spawn(launch, progress) {
            Ok(srv) => {
                notes.push(format!("{} via prebuilt llama-server", srv.name));
                return Ok((srv, notes));
            }
            Err(e) => notes.push(format!("{name}: {e:#}")),
        }
    }
    if !saw_bin {
        notes.push("no llama-server under /usr/local/lib/bia/llama (CPU build is required)".into());
    }
    anyhow::bail!("{}", notes.join("; "))
}

fn load_mode(plan: &LlamaPlan) -> String {
    if !plan.mmap {
        "none".into()
    } else if plan.mlock {
        "mmap+mlock".into()
    } else {
        "mmap".into()
    }
}

fn backend_dir(name: &str) -> Option<PathBuf> {
    let mut roots = Vec::new();
    if let Some(d) = std::env::var_os("BIA_LLAMA_DIR") {
        roots.push(PathBuf::from(d));
    }
    roots.push(PathBuf::from("/usr/local/lib/bia/llama"));
    roots
        .into_iter()
        .map(|r| r.join(name))
        .find(|p| p.join("llama-server").is_file())
}

fn gate(name: &str, gpus: &[&hw::DisplayGpu]) -> Result<(), String> {
    let has = |vendors: &[u16]| gpus.iter().any(|g| vendors.contains(&g.vendor));
    match name {
        "cuda" => {
            if !has(&[0x10de]) {
                return Err("no NVIDIA GPU".into());
            }
            if !lib_so("cuda") {
                return Err("no libcuda.so.1 (NVIDIA driver not installed)".into());
            }
            Ok(())
        }
        "rocm" => {
            if !has(&[0x1002]) {
                return Err("no AMD GPU".into());
            }
            if !Path::new("/dev/kfd").exists() {
                return Err("no /dev/kfd (amdgpu compute)".into());
            }
            Ok(())
        }
        "sycl" => {
            if !has(&[0x8086]) {
                return Err("no Intel GPU".into());
            }
            if !lib_so("ze_intel_gpu") {
                return Err("no libze_intel_gpu.so.1 (intel level-zero)".into());
            }
            Ok(())
        }
        "vulkan" => {
            if gpus.is_empty() {
                return Err("no GPU".into());
            }
            if !lib_so("vulkan") {
                return Err("no libvulkan.so.1".into());
            }
            Ok(())
        }
        "cpu" => Ok(()),
        _ => Err("unknown backend".into()),
    }
}

fn lib_so(stem: &str) -> bool {
    let names = [format!("lib{stem}.so.1"), format!("lib{stem}.so")];
    let roots = [
        "/usr/lib/x86_64-linux-gnu",
        "/usr/lib/aarch64-linux-gnu",
        "/usr/lib64",
        "/usr/lib",
        "/lib/x86_64-linux-gnu",
        "/lib/aarch64-linux-gnu",
        "/lib64",
        "/lib",
        "/usr/local/lib",
        "/usr/lib/wsl/lib",
    ];
    roots
        .iter()
        .any(|r| names.iter().any(|n| Path::new(r).join(n).is_file()))
}

fn spawn(launch: Launch, progress: &mut impl FnMut(f32) -> bool) -> Result<Server> {
    let cpu = launch.name == "cpu";
    let (child, port, log) = spawn_child(&launch)?;
    let mut srv = Server {
        name: launch.name.clone(),
        child,
        port,
        launch,
        log,
        live: true,
    };
    if let Err(e) = srv.wait_ready(progress) {
        srv.kill();
        return Err(e);
    }
    if !cpu && offloaded_nothing(&srv.log_text()) {
        let err = anyhow!("{} loaded the model on CPU (0 layers offloaded)", srv.name);
        srv.kill();
        return Err(err);
    }
    Ok(srv)
}

fn spawn_child(launch: &Launch) -> Result<(Child, u16, Arc<Mutex<Vec<String>>>)> {
    let port = free_port()?;
    let mut cmd = Command::new(&launch.bin);
    cmd.process_group(0)
        .current_dir(&launch.dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .env("LD_LIBRARY_PATH", lib_path(&launch.dir))
        .args([
            "-m",
            &launch.model.to_string_lossy(),
            "-c",
            &launch.ctx.to_string(),
            "-t",
            &launch.threads.to_string(),
            "-tb",
            &launch.threads_batch.to_string(),
            "-b",
            &launch.n_batch.to_string(),
            "-ub",
            &launch.n_ubatch.to_string(),
            "-ngl",
            if launch.name == "cpu" { "0" } else { "auto" },
            "--fit",
            if launch.name == "cpu" { "off" } else { "on" },
            "-fa",
            "auto",
            "-np",
            "1",
            "--host",
            "127.0.0.1",
            "--port",
            &port.to_string(),
            "--no-webui",
            "--log-colors",
            "off",
            "--load-mode",
            &launch.load_mode,
            "--prio",
            "1",
        ]);
    if launch.name == "sycl" {
        cmd.env("ONEAPI_DEVICE_SELECTOR", "level_zero:gpu");
    }
    let mut child = cmd
        .spawn()
        .with_context(|| format!("spawn {}", launch.bin.display()))?;
    let stderr = child.stderr.take();
    let log = Arc::new(Mutex::new(Vec::new()));
    if let Some(stderr) = stderr {
        let log = log.clone();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stderr);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        let mut g = log.lock().unwrap();
                        g.push(line.trim_end().to_string());
                        if g.len() > 80 {
                            let n = g.len() - 80;
                            g.drain(0..n);
                        }
                    }
                }
            }
        });
    }
    Ok((child, port, log))
}

fn lib_path(dir: &Path) -> std::ffi::OsString {
    let mut p = std::ffi::OsString::from(dir);
    if let Some(old) = std::env::var_os("LD_LIBRARY_PATH") {
        p.push(":");
        p.push(old);
    }
    p
}

fn free_port() -> Result<u16> {
    for port in 18080..18112u16 {
        if std::net::TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return Ok(port);
        }
    }
    anyhow::bail!("no free port for llama-server")
}

impl Server {
    fn wait_ready(&mut self, progress: &mut impl FnMut(f32) -> bool) -> Result<()> {
        let start = Instant::now();
        let mut saw = 0.0f32;
        loop {
            if let Some(st) = self.child.try_wait().ok().flatten() {
                self.live = false;
                anyhow::bail!("llama-server exited {st}; {}", tail(&self.log_text()));
            }
            if start.elapsed() > Duration::from_secs(180) {
                anyhow::bail!(
                    "llama-server did not become ready; {}",
                    tail(&self.log_text())
                );
            }
            if let Some(pct) = last_percent(&self.log_text()) {
                if pct > saw {
                    saw = pct;
                    if !progress(pct / 100.0) {
                        anyhow::bail!("load cancelled");
                    }
                }
            }
            match http(
                self.port,
                "GET",
                "/health",
                None,
                Duration::from_millis(400),
            ) {
                Ok((200, _)) => {
                    let _ = progress(1.0);
                    return Ok(());
                }
                Ok((503, _)) => {}
                _ => {}
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    pub fn n_ctx(&self) -> Option<u32> {
        let (status, body) = http(self.port, "GET", "/props", None, Duration::from_secs(5)).ok()?;
        if status != 200 {
            return None;
        }
        let v: Value = serde_json::from_slice(&body).ok()?;
        v.get("default_generation_settings")
            .and_then(|s| s.get("n_ctx"))
            .and_then(|n| n.as_u64())
            .map(|n| n as u32)
    }

    pub fn chat_template(&self) -> String {
        let Ok((200, body)) = http(self.port, "GET", "/props", None, Duration::from_secs(5)) else {
            return String::new();
        };
        serde_json::from_slice::<Value>(&body)
            .ok()
            .and_then(|v| {
                v.get("chat_template")
                    .and_then(|t| t.as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_default()
    }

    pub fn tokenize(&self, text: &str, add_special: bool) -> Result<Vec<i32>> {
        let body = serde_json::to_vec(&json!({
            "content": text,
            "add_special": add_special,
            "parse_special": true,
        }))?;
        let (status, raw) = http(
            self.port,
            "POST",
            "/tokenize",
            Some(&body),
            Duration::from_secs(60),
        )?;
        if status != 200 {
            anyhow::bail!("tokenize http {status}: {}", String::from_utf8_lossy(&raw));
        }
        let v: Value = serde_json::from_slice(&raw).context("tokenize json")?;
        let toks = v
            .get("tokens")
            .and_then(|t| t.as_array())
            .context("tokenize: no tokens")?;
        Ok(toks
            .iter()
            .filter_map(|t| t.as_i64().map(|n| n as i32))
            .collect())
    }

    pub fn complete(
        &self,
        prompt: &[i32],
        max_new: usize,
        sampling: &SamplingCfg,
        mut on_prefill: impl FnMut(usize, usize) -> bool,
        mut on_piece: impl FnMut(&str) -> bool,
    ) -> Result<Completion> {
        let body = serde_json::to_vec(&json!({
            "prompt": prompt,
            "n_predict": max_new,
            "stream": true,
            "cache_prompt": true,
            "temperature": sampling.temperature,
            "top_k": sampling.top_k,
            "top_p": sampling.top_p,
            "min_p": sampling.min_p,
            "repeat_penalty": sampling.repeat_penalty,
            "repeat_last_n": sampling.repeat_last_n,
        }))?;
        let t0 = Instant::now();
        let mut prefill_secs = 0.0;
        let mut text = String::new();
        let mut generated = 0usize;
        let mut reused = 0usize;
        let mut eos = false;
        let mut truncated = false;
        let mut interrupted = false;
        let mut saw_token = false;
        let prompt_len = prompt.len();
        stream_post(self.port, "/completion", &body, |ev| {
            if prefill_secs == 0.0 {
                if let Some(n) = ev.get("tokens_evaluated").and_then(|n| n.as_u64()) {
                    if !on_prefill(n as usize, prompt_len) {
                        interrupted = true;
                        return false;
                    }
                }
            }
            if let Some(n) = ev.get("tokens_cached").and_then(|n| n.as_u64()) {
                reused = n as usize;
            }
            if ev
                .get("truncated")
                .and_then(|t| t.as_bool())
                .unwrap_or(false)
            {
                truncated = true;
            }
            let piece = ev.get("content").and_then(|c| c.as_str()).unwrap_or("");
            if !piece.is_empty() {
                if !saw_token {
                    saw_token = true;
                    prefill_secs = t0.elapsed().as_secs_f64();
                }
                text.push_str(piece);
                generated += 1;
                if !on_piece(piece) {
                    interrupted = true;
                    return false;
                }
            }
            if ev.get("stop").and_then(|s| s.as_bool()).unwrap_or(false) {
                if ev.get("stop_type").and_then(|s| s.as_str()).unwrap_or("") == "eos" {
                    eos = true
                }
                if let Some(ms) = ev.pointer("/timings/prompt_ms").and_then(|n| n.as_f64()) {
                    prefill_secs = ms / 1000.0;
                }
            }
            true
        })?;
        if prefill_secs == 0.0 {
            prefill_secs = t0.elapsed().as_secs_f64();
        }
        let gen_secs = t0.elapsed().as_secs_f64() - prefill_secs;
        if max_new == 0 {
            on_prefill(prompt_len, prompt_len);
        }
        Ok(Completion {
            text,
            generated,
            reused,
            prefill_secs,
            gen_secs: gen_secs.max(0.0),
            eos,
            truncated,
            interrupted,
        })
    }

    pub fn restart(&mut self, ctx: u32, progress: &mut impl FnMut(f32) -> bool) -> Result<u32> {
        let prev = self.launch.ctx;
        self.kill();
        self.launch.ctx = ctx;
        match spawn_child(&self.launch) {
            Ok((child, port, log)) => {
                self.child = child;
                self.port = port;
                self.log = log;
                self.live = true;
                if let Err(e) = self.wait_ready(progress) {
                    self.kill();
                    self.launch.ctx = prev;
                    let (child, port, log) =
                        spawn_child(&self.launch).context("restore llama-server")?;
                    self.child = child;
                    self.port = port;
                    self.log = log;
                    self.live = true;
                    self.wait_ready(&mut |_| true)
                        .context("restore llama-server")?;
                    return Err(e);
                }
                Ok(self.n_ctx().unwrap_or(ctx))
            }
            Err(e) => {
                self.launch.ctx = prev;
                let (child, port, log) =
                    spawn_child(&self.launch).context("restore llama-server")?;
                self.child = child;
                self.port = port;
                self.log = log;
                self.live = true;
                let _ = self.wait_ready(&mut |_| true);
                Err(e)
            }
        }
    }

    fn log_text(&self) -> String {
        self.log.lock().unwrap().join("\n")
    }

    fn kill(&mut self) {
        if !self.live {
            return;
        }
        self.live = false;
        match self.child.try_wait() {
            Ok(Some(_)) => {}
            Ok(None) => {
                let pid = self.child.id() as i32;
                unsafe { libc::kill(-pid, libc::SIGKILL) };
                let _ = self.child.wait();
            }
            Err(_) => {}
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.kill();
    }
}

fn offloaded_nothing(log: &str) -> bool {
    let mut saw = false;
    let mut any = false;
    for line in log.lines() {
        let Some(rest) = line.split("offloaded ").nth(1) else {
            continue;
        };
        let Some(num) = rest.split('/').next() else {
            continue;
        };
        let Ok(n) = num.trim().parse::<u32>() else {
            continue;
        };
        saw = true;
        if n > 0 {
            any = true;
        }
    }
    saw && !any
}

fn last_percent(log: &str) -> Option<f32> {
    let line = log
        .lines()
        .rev()
        .find(|l| l.contains('%') && (l.contains("load") || l.contains('%')))?;
    let idx = line.rfind('%')?;
    let start = line[..idx]
        .rfind(|c: char| !c.is_ascii_digit() && c != '.')
        .map(|i| i + 1)
        .unwrap_or(0);
    line[start..idx].parse().ok()
}

fn tail(log: &str) -> String {
    let lines: Vec<_> = log.lines().rev().take(12).collect();
    let mut v: Vec<_> = lines.into_iter().rev().map(|s| s.to_string()).collect();
    if v.is_empty() {
        v.push("(no log)".into());
    }
    v.join(" | ")
}

fn http(
    port: u16,
    method: &str,
    path: &str,
    body: Option<&[u8]>,
    timeout: Duration,
) -> Result<(u16, Vec<u8>)> {
    let mut stream = TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_millis(300),
    )?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    write_req(&mut stream, method, path, body)?;
    let mut reader = BufReader::new(stream);
    let (status, headers) = read_head(&mut reader)?;
    let chunked = headers.iter().any(|(k, v)| {
        k.eq_ignore_ascii_case("transfer-encoding") && v.to_ascii_lowercase().contains("chunked")
    });
    let len = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse::<usize>().ok());
    let body = if chunked {
        read_chunked(&mut reader)?
    } else if let Some(n) = len {
        let mut buf = vec![0u8; n];
        reader.read_exact(&mut buf)?;
        buf
    } else {
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf)?;
        buf
    };
    Ok((status, body))
}

fn stream_post(
    port: u16,
    path: &str,
    body: &[u8],
    mut on_event: impl FnMut(&Value) -> bool,
) -> Result<()> {
    let mut stream = TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_secs(2),
    )?;
    stream.set_read_timeout(Some(Duration::from_secs(180)))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    write_req(&mut stream, "POST", path, Some(body))?;
    let mut reader = BufReader::new(stream);
    let (status, headers) = read_head(&mut reader)?;
    if status != 200 {
        let mut err = Vec::new();
        let _ = reader.read_to_end(&mut err);
        anyhow::bail!(
            "completion http {status}: {}",
            String::from_utf8_lossy(&err)
        );
    }
    let chunked = headers.iter().any(|(k, v)| {
        k.eq_ignore_ascii_case("transfer-encoding") && v.to_ascii_lowercase().contains("chunked")
    });
    let mut plain = Vec::new();
    if chunked {
        loop {
            let size_line = read_line_buf(&mut reader)?;
            let size = usize::from_str_radix(size_line.trim().split(';').next().unwrap_or("0"), 16)
                .map_err(|e| anyhow!("chunk size: {e}"))?;
            if size == 0 {
                break;
            }
            let mut chunk = vec![0u8; size];
            reader.read_exact(&mut chunk)?;
            let mut crlf = [0u8; 2];
            let _ = reader.read_exact(&mut crlf);
            plain.extend_from_slice(&chunk);
            if !drain_sse(&mut plain, &mut on_event) {
                break;
            }
        }
    } else {
        reader.read_to_end(&mut plain)?;
        let _ = drain_sse(&mut plain, &mut on_event);
        if plain.starts_with(b"{") {
            if let Ok(v) = serde_json::from_slice::<Value>(&plain) {
                let _ = on_event(&v);
            }
        }
    }
    Ok(())
}

fn drain_sse(buf: &mut Vec<u8>, on_event: &mut impl FnMut(&Value) -> bool) -> bool {
    let mut go = true;
    let mut consumed = 0usize;
    while let Some(rel) = buf[consumed..].iter().position(|b| *b == b'\n') {
        let end = consumed + rel;
        let line = std::str::from_utf8(&buf[consumed..end])
            .unwrap_or("")
            .trim_end_matches('\r');
        consumed = end + 1;
        if let Some(data) = line.strip_prefix("data:") {
            let data = data.trim();
            if data.is_empty() || data == "[DONE]" {
                continue;
            }
            if let Ok(v) = serde_json::from_str::<Value>(data) {
                if !on_event(&v) {
                    go = false;
                    break;
                }
            }
        }
    }
    if consumed > 0 {
        buf.drain(0..consumed);
    }
    go
}

fn write_req(stream: &mut TcpStream, method: &str, path: &str, body: Option<&[u8]>) -> Result<()> {
    let len = body.map(|b| b.len()).unwrap_or(0);
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(head.as_bytes())?;
    if let Some(b) = body {
        stream.write_all(b)?;
    }
    Ok(())
}

fn read_head(reader: &mut BufReader<TcpStream>) -> Result<(u16, Vec<(String, String)>)> {
    let status_line = read_line_buf(reader)?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let mut headers = Vec::new();
    loop {
        let line = read_line_buf(reader)?;
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    Ok((status, headers))
}

fn read_line_buf(reader: &mut BufReader<TcpStream>) -> Result<String> {
    let mut line = String::new();
    let n = reader.read_line(&mut line)?;
    if n == 0 {
        anyhow::bail!("connection closed");
    }
    Ok(line.trim_end_matches(['\r', '\n']).to_string())
}

fn read_chunked(reader: &mut BufReader<TcpStream>) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let size_line = read_line_buf(reader)?;
        let size = usize::from_str_radix(size_line.trim().split(';').next().unwrap_or("0"), 16)
            .map_err(|e| anyhow!("chunk size: {e}"))?;
        if size == 0 {
            break;
        }
        let mut chunk = vec![0u8; size];
        reader.read_exact(&mut chunk)?;
        let mut crlf = [0u8; 2];
        let _ = reader.read_exact(&mut crlf);
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}
