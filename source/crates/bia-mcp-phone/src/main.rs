//! The phone bus: when bia runs on a phone (Debian chroot on Android), the bia app there
//! serves the phone's line on 127.0.0.1 (SMS, calls, USSD, carrier status) and writes
//! files/phone/link.json {url, token}. This MCP turns that into a chat bus like telegram:
//! channels `phone:<number>`, incoming SMS and calls as inbound messages, `send` for SMS
//! (the agent's outbox calls it for `say`).

mod call;

use bia_mcp::{arg_str, arg_str_opt, arg_u32_opt, members, Ctx, Server, ToolResult};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::Duration;

fn files_dir() -> PathBuf {
    PathBuf::from(std::env::var("BIA_FILES").unwrap_or_else(|_| "/home/bia/files".into()))
}
fn dir() -> PathBuf {
    files_dir().join("phone")
}

/// (host:port, token) from files/phone/link.json.
pub(crate) fn link() -> Result<(String, String), String> {
    let s = std::fs::read_to_string(dir().join("link.json"))
        .map_err(|_| "the phone is not linked: files/phone/link.json is missing (the bia app on this phone writes it when it starts)".to_string())?;
    let v: Value = serde_json::from_str(&s).map_err(|e| format!("link.json: {e}"))?;
    let url = v["url"].as_str().unwrap_or("");
    let host = url.trim_start_matches("http://").trim_end_matches('/').to_string();
    let token = v["token"].as_str().unwrap_or("").to_string();
    if host.is_empty() || token.is_empty() {
        return Err("link.json has no url/token".into());
    }
    Ok((host, token))
}

/// One HTTP/1.1 request to the bia app (plain, loopback only).
fn api(method: &str, path: &str, body: Option<&Value>, timeout: Duration) -> Result<Value, String> {
    let (host, token) = link()?;
    let mut s = TcpStream::connect(&host).map_err(|e| format!("the bia app's phone service does not answer ({e}); is the bia app running?"))?;
    s.set_read_timeout(Some(timeout)).ok();
    let b = body.map(|b| b.to_string()).unwrap_or_default();
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{b}",
        b.len()
    );
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    s.read_to_end(&mut out).map_err(|e| format!("reading the answer: {e}"))?;
    let text = String::from_utf8_lossy(&out);
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let code: u16 = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    let v: Value = serde_json::from_str(body).unwrap_or(json!({"raw": body}));
    if code != 200 {
        return Err(format!("phone: {}", v["error"].as_str().unwrap_or(body)));
    }
    Ok(v)
}

/// A chat id from a number as people write it: +34 625-640-810 → +34625640810.
/// Alphanumeric senders (a bank, the carrier) stay as they are.
fn chat_id(raw: &str) -> String {
    let t = raw.trim().trim_start_matches("phone:");
    if t.chars().any(|c| c.is_ascii_alphabetic()) {
        return t.to_string();
    }
    let plus = t.starts_with('+') || t.starts_with("00");
    let digits: String = t.chars().filter(|c| c.is_ascii_digit()).collect();
    let digits = if t.starts_with("00") { digits[2..].to_string() } else { digits };
    if plus { format!("+{digits}") } else { digits }
}

// ---- local history (SMS has no server-side history for bia) ----

fn log_line(chat: &str, dir_: &str, id: &str, text: &str, at: i64) {
    let line = json!({"chat": chat, "dir": dir_, "id": id, "text": text, "at": at});
    let p = dir().join("history.jsonl");
    let _ = std::fs::create_dir_all(dir());
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
        let _ = writeln!(f, "{line}");
    }
}

fn history() -> Vec<Value> {
    std::fs::read_to_string(dir().join("history.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// A unique id for one SMS bia sends (time + process + counter).
fn sms_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    format!("b{t:x}{:x}{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed))
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

// ---- events → inbound ----

fn seq_path() -> PathBuf {
    dir().join("seq")
}
fn load_seq() -> Option<i64> {
    std::fs::read_to_string(seq_path()).ok()?.trim().parse().ok()
}
fn save_seq(n: i64) {
    let _ = std::fs::create_dir_all(dir());
    let _ = std::fs::write(seq_path(), n.to_string());
}

fn poll(ctx: Ctx) {
    let mut announced_down = false;
    let mut ringing_from = String::new();
    loop {
        // first start: only what happens from now on (no flood of the phone's past)
        let after = match load_seq() {
            Some(n) => n,
            None => match api("GET", "/events?after=0&wait=0", None, Duration::from_secs(10)) {
                Ok(v) => {
                    let n = v["last"].as_i64().unwrap_or(0);
                    save_seq(n);
                    n
                }
                Err(_) => {
                    std::thread::sleep(Duration::from_secs(5));
                    continue;
                }
            },
        };
        match api("GET", &format!("/events?after={after}&wait=25"), None, Duration::from_secs(40)) {
            Ok(v) => {
                if announced_down {
                    ctx.log("phone: the bia app's phone service answers again");
                    announced_down = false;
                }
                // the app's log restarted (reinstalled): its numbers are lower than ours
                let last = v["last"].as_i64().unwrap_or(0);
                if last < after {
                    save_seq(last);
                    continue;
                }
                for e in v["events"].as_array().cloned().unwrap_or_default() {
                    let seq = e["seq"].as_i64().unwrap_or(0);
                    if seq <= after {
                        continue;
                    }
                    deliver(&ctx, &e, &mut ringing_from);
                    save_seq(seq);
                }
            }
            Err(e) => {
                if !announced_down {
                    ctx.log(format!("phone: {e}"));
                    announced_down = true;
                }
                std::thread::sleep(Duration::from_secs(5));
            }
        }
    }
}

fn deliver(ctx: &Ctx, e: &Value, ringing_from: &mut String) {
    let seq = e["seq"].as_i64().unwrap_or(0);
    let at = e["at"].as_i64().unwrap_or_else(now);
    match e["type"].as_str().unwrap_or("") {
        "sms" => {
            let chat = chat_id(e["from"].as_str().unwrap_or(""));
            let text = e["text"].as_str().unwrap_or("");
            let id = format!("i{seq}");
            log_line(&chat, "in", &id, text, e["sent_at"].as_i64().unwrap_or(at));
            let from = members::From { sender: format!("phone:{chat}"), ..Default::default() };
            ctx.inbound_by(&chat, &format!("phone:{chat}"), &id, text, e["sent_at"].as_i64().unwrap_or(at), &from);
        }
        "mms" => {
            // text parts, then one "file: inbox/phone/<chat>/<name>" line per picture/audio/video
            // (the bia app copied them into files/; same form as the other chat buses)
            let chat = chat_id(e["from"].as_str().unwrap_or(""));
            let mut parts: Vec<String> = Vec::new();
            if let Some(t) = e["text"].as_str().filter(|t| !t.trim().is_empty()) {
                parts.push(t.to_string());
            }
            for f in e["files"].as_array().cloned().unwrap_or_default() {
                if let Some(p) = f.as_str() {
                    parts.push(format!("file: {}", p.trim_start_matches("/home/bia/files/")));
                }
            }
            if parts.is_empty() {
                parts.push("[MMS with nothing bia can read]".into());
            }
            let text = parts.join("\n");
            let id = format!("i{seq}");
            let sent = e["sent_at"].as_i64().unwrap_or(at);
            log_line(&chat, "in", &id, &text, sent);
            let from = members::From { sender: format!("phone:{chat}"), ..Default::default() };
            ctx.inbound_by(&chat, &format!("phone:{chat}"), &id, &text, sent, &from);
        }
        "sms_status" if e["state"] == "failed" => {
            let to = chat_id(e["to"].as_str().unwrap_or(""));
            ctx.inbound(
                "system",
                &format!("phone:{to}"),
                "",
                &format!("[phone] the SMS {} to {to} was not sent: {}", e["id"].as_str().unwrap_or(""), e["error"].as_str().unwrap_or("")),
            );
        }
        "sms_status" if e["state"] == "delivered" => {
            ctx.log(format!("phone: SMS {} delivered to {}", e["id"].as_str().unwrap_or(""), e["to"].as_str().unwrap_or("")));
        }
        "call" => {
            let state = e["state"].as_str().unwrap_or("");
            let prev = e["previous"].as_str().unwrap_or("");
            let num = chat_id(e["from"].as_str().unwrap_or(""));
            match state {
                "ringing" => {
                    *ringing_from = num.clone();
                    let from = members::From { sender: format!("phone:{num}"), ..Default::default() };
                    ctx.inbound_by(
                        &num,
                        &format!("phone:{num}"),
                        &format!("c{seq}"),
                        &format!("[incoming call] from {num} — phone_call_accept() answers it (then you hear them and say() speaks into the call; it is a phone call: answer in one or two short sentences), phone_call_hangup() rejects it"),
                        at,
                        &from,
                    );
                }
                "idle" if prev == "ringing" => {
                    call::ended();
                    let num = if num.is_empty() { ringing_from.clone() } else { num };
                    ctx.inbound("system", &format!("phone:{num}"), "", &format!("[missed call] from {num}"));
                    ringing_from.clear();
                }
                "active" => {
                    let num = if num.is_empty() { ringing_from.clone() } else { num };
                    ctx.log(format!("phone: call active {num}"));
                    if !num.is_empty() {
                        call::started(ctx, &num);
                    }
                }
                "idle" => {
                    ctx.log("phone: call ended");
                    call::ended();
                    ringing_from.clear();
                }
                _ => {}
            }
        }
        "ussd" => {
            let code = e["code"].as_str().unwrap_or("");
            let text = e["text"].as_str().or(e["error"].as_str()).unwrap_or("");
            ctx.inbound("system", "console", "", &format!("[ussd {code}] {text}"));
        }
        _ => {}
    }
}

// ---- tools ----

fn obj(props: Value, required: &[&str]) -> Value {
    json!({"type": "object", "properties": props, "required": required})
}

fn status_text() -> ToolResult {
    let s = api("GET", "/status", None, Duration::from_secs(10))?;
    let sig = match (s["signal_level"].as_i64(), s["signal_dbm"].as_i64()) {
        (Some(l), Some(d)) => format!("{l}/4 ({d} dBm)"),
        (Some(l), None) => format!("{l}/4"),
        _ => "unknown".into(),
    };
    Ok(format!(
        "SIM: {} ({}, {}), number {}\nnetwork: {} ({}), {}, {}{}\nsignal: {sig}\nmobile data: {}\ncall: {}{}\nWi-Fi: {}",
        s["sim"].as_str().unwrap_or("?"),
        s["sim_operator"].as_str().unwrap_or(""),
        s["sim_country"].as_str().unwrap_or("").to_uppercase(),
        s["number"].as_str().filter(|n| !n.is_empty()).unwrap_or("unknown"),
        s["network"].as_str().unwrap_or(""),
        s["network_country"].as_str().unwrap_or("").to_uppercase(),
        s["network_type"].as_str().unwrap_or(""),
        s["service"].as_str().unwrap_or(""),
        if s["roaming"].as_bool() == Some(true) { ", roaming" } else { "" },
        if s["data_enabled"].as_bool() == Some(true) { "on" } else { "off" },
        s["call"].as_str().unwrap_or(""),
        s["call_number"].as_str().filter(|n| !n.is_empty()).map(|n| format!(" ({n})")).unwrap_or_default(),
        s["wifi"]["ssid"].as_str().filter(|n| !n.is_empty() && *n != "<unknown ssid>").unwrap_or("not connected"),
    ))
}

fn wifi_text(scan: bool) -> ToolResult {
    let w = api("GET", if scan { "/wifi?scan=1" } else { "/wifi" }, None, Duration::from_secs(20))?;
    let mut out = match w["current"].as_str().filter(|s| !s.is_empty()) {
        Some(c) => format!("on Wi-Fi {c} ({})", w["ip"].as_str().unwrap_or("")),
        None if w["enabled"].as_bool() == Some(false) => "Wi-Fi is off".into(),
        None => "not on any Wi-Fi network".into(),
    };
    let nets = w["networks"].as_array().cloned().unwrap_or_default();
    if nets.is_empty() {
        out.push_str("\nno networks in range");
    } else {
        out.push_str("\nin range (signal 0-4):");
        for n in &nets {
            let mut tags = vec![n["security"].as_str().unwrap_or("").to_string()];
            if n["saved"].as_bool() == Some(true) {
                tags.push("saved".into());
            }
            if n["current"].as_bool() == Some(true) {
                tags.push("connected".into());
            }
            out.push_str(&format!("\n  {} — {}/4, {}", n["ssid"].as_str().unwrap_or(""), n["level"], tags.join(", ")));
        }
    }
    let saved: Vec<&str> = w["saved"].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).collect()).unwrap_or_default();
    if !saved.is_empty() {
        out.push_str(&format!("\nsaved: {}", saved.join(", ")));
    }
    Ok(out)
}

/// The text a phone camera reads to join a Wi-Fi network (the de-facto WIFI: format).
fn wifi_qr_text(ssid: &str, password: &str) -> String {
    let esc = |s: &str| s.chars().fold(String::new(), |mut o, c| {
        if matches!(c, '\\' | ';' | ',' | ':' | '"') {
            o.push('\\');
        }
        o.push(c);
        o
    });
    if password.is_empty() {
        format!("WIFI:T:nopass;S:{};;", esc(ssid))
    } else {
        format!("WIFI:T:WPA;S:{};P:{};;", esc(ssid), esc(password))
    }
}

/// files/phone/hotspot-qr.png: scanning it joins the hotspot (any chat can carry it).
fn hotspot_qr(ssid: &str, password: &str) -> Result<PathBuf, String> {
    use image::Luma;
    let q = qrcode::QrCode::new(wifi_qr_text(ssid, password).as_bytes()).map_err(|e| format!("qr: {e}"))?;
    let img = q.render::<Luma<u8>>().min_dimensions(480, 480).quiet_zone(true).build();
    let _ = std::fs::create_dir_all(dir());
    let p = dir().join("hotspot-qr.png");
    img.save(&p).map_err(|e| format!("write {}: {e}", p.display()))?;
    Ok(p)
}

fn hotspot_text(h: &Value) -> String {
    let ssid = h["ssid"].as_str().unwrap_or("");
    let pw = h["password"].as_str().unwrap_or("");
    let mut s = format!(
        "hotspot {}: name {ssid}, {}",
        if h["on"].as_bool() == Some(true) { "on" } else { "off" },
        if pw.is_empty() { "open (no password)".to_string() } else { format!("password {pw}") }
    );
    if h["on"].as_bool() == Some(true) {
        let n = h["clients"].as_array().map(|a| a.len()).unwrap_or(0);
        s.push_str(&format!(", {n} device(s) connected"));
    }
    if !ssid.is_empty() {
        match hotspot_qr(ssid, pw) {
            Ok(p) => s.push_str(&format!(
                "\nQR code to join it: {} — a phone camera scanning it joins at once; send it with say(files=[…]) on any chat, or on the web chat to show it on this phone's screen",
                p.display()
            )),
            Err(e) => s.push_str(&format!("\n(no QR code: {e})")),
        }
    }
    s
}

fn main() {
    let srv = Server::new("phone", env!("CARGO_PKG_VERSION"))
        .tool(
            "send",
            "Send an SMS from bia's own phone line. chat_id is the number in international form (+34…). Long texts go as several parts; the delivery report is logged. Files are not supported (no MMS yet).",
            obj(json!({
                "chat_id": {"type": "string", "description": "the number, e.g. +34625640810 (phone:… also accepted)"},
                "text": {"type": "string"},
                "files": {"type": "array", "items": {"type": "string"}}
            }), &["chat_id", "text"]),
            |_ctx, a| {
                let to = chat_id(arg_str(&a, "chat_id")?);
                let text = arg_str(&a, "text")?;
                if a["files"].as_array().is_some_and(|f| !f.is_empty()) {
                    return Err("SMS carries text only; this phone can not send files (MMS) yet".into());
                }
                if text.trim().is_empty() {
                    return Err("empty SMS".into());
                }
                // the agent sends a text copy of what bia said in a call: it was spoken, no SMS
                if call::was_spoken(&to, text) {
                    return Ok(format!("spoken in the call with {to}; not sent as SMS, message id call{}", now()));
                }
                // Our own id: the app sends one id only once, so asking again after a lost
                // answer (the app restarting) can never send the SMS twice.
                let id = sms_id();
                let body = json!({"to": to, "text": text, "id": id});
                let mut last_err = String::new();
                for attempt in 0..5 {
                    if attempt > 0 {
                        std::thread::sleep(Duration::from_secs(4));
                    }
                    match api("POST", "/sms", Some(&body), Duration::from_secs(20)) {
                        Ok(r) => {
                            log_line(&to, "out", &id, text, now());
                            return Ok(format!("SMS to {to} sent ({} part(s)), message id {id}", r["parts"].as_i64().unwrap_or(1)));
                        }
                        Err(e) => last_err = e,
                    }
                }
                Err(format!("{last_err} — the phone app did not confirm the SMS to {to} (id {id}); it may or may not have gone out. Do not resend at once: phone_status shows when the app answers again, and the delivery report then names this id"))
            },
        )
        .tool("read", "Read receipts (SMS has none).", obj(json!({}), &[]), |_c, _a| Ok(String::new()))
        .tool("typing", "Typing indicator (SMS has none).", obj(json!({}), &[]), |_c, _a| Ok(String::new()))
        .tool("status", "bia's phone line: SIM, carrier, roaming, signal, mobile data, call state, Wi-Fi, hotspot.", obj(json!({}), &[]), |_c, _a| {
            let mut s = status_text()?;
            if let Ok(h) = api("GET", "/hotspot", None, Duration::from_secs(10)) {
                s.push_str(&format!("\n{}", hotspot_text(&h)));
            }
            Ok(s)
        })
        .tool(
            "call",
            "Call a number from bia's phone line. Once they answer, their speech arrives on the phone:<number> chat and say() there speaks into the call.",
            obj(json!({"chat_id": {"type": "string", "description": "the number, +34…"}}), &["chat_id"]),
            |_c, a| {
                let to = chat_id(arg_str(&a, "chat_id")?);
                api("POST", "/call", Some(&json!({"to": to})), Duration::from_secs(10))?;
                Ok(format!("calling {to}"))
            },
        )
        .tool(
            "call_say",
            "Speak text into the active phone call with chat_id (the agent calls this for say() during a call).",
            obj(json!({"chat_id": {"type": "string"}, "text": {"type": "string"}, "lang": {"type": "string"}}), &["chat_id", "text"]),
            |ctx, a| {
                let chat = chat_id(arg_str(&a, "chat_id")?);
                call::say(ctx, &chat, arg_str(&a, "text")?, arg_str_opt(&a, "lang").unwrap_or(""))
            },
        )
        .tool("call_accept", "Answer the phone call that is ringing now.", obj(json!({"chat_id": {"type": "string"}}), &[]), |_c, _a| {
            api("POST", "/answer", None, Duration::from_secs(10))?;
            Ok("answered".into())
        })
        .tool("call_hangup", "End the current phone call, or reject the ringing one.", obj(json!({}), &[]), |_c, _a| {
            api("POST", "/hangup", None, Duration::from_secs(10))?;
            Ok("hung up".into())
        })
        .tool(
            "ussd",
            "Send a USSD code (balance *100#, carrier menus). The network's answer arrives as a [ussd <code>] notice.",
            obj(json!({"code": {"type": "string"}}), &["code"]),
            |_c, a| {
                let code = arg_str(&a, "code")?;
                api("POST", "/ussd", Some(&json!({"code": code})), Duration::from_secs(10))?;
                Ok(format!("sent {code}; the answer arrives as a [ussd {code}] notice"))
            },
        )
        .tool(
            "wifi_list",
            "Wi-Fi networks around this phone (scans first, ~4 s) and the one it is on; Android owns the phone's Wi-Fi, so this, not nmcli, is how bia sees it.",
            obj(json!({"scan": {"type": "boolean", "description": "default true; false = last results, instant"}}), &[]),
            |_c, a| wifi_text(a["scan"].as_bool() != Some(false)),
        )
        .tool(
            "wifi_connect",
            "Join a Wi-Fi network on this phone: ssid, password (leave out for an open or already saved network), hidden=true for a network that does not broadcast its name. Waits up to 30 s and reports the address or why it failed (wrong password, not in range); after a failure the phone goes back to its earlier network.",
            obj(json!({
                "ssid": {"type": "string", "description": "the network name exactly as wifi_list shows it"},
                "password": {"type": "string"},
                "hidden": {"type": "boolean"}
            }), &["ssid"]),
            |ctx, a| {
                let ssid = arg_str(&a, "ssid")?;
                let body = json!({"ssid": ssid, "password": arg_str_opt(&a, "password").unwrap_or(""), "hidden": a["hidden"].as_bool().unwrap_or(false)});
                ctx.log(format!("wifi: joining {ssid}"));
                match api("POST", "/wifi/connect", Some(&body), Duration::from_secs(90)) {
                    Ok(r) => Ok(format!("joined Wi-Fi {ssid}: address {}, signal {}/4; Android remembers it", r["ip"].as_str().unwrap_or(""), r["level"])),
                    Err(e) => {
                        let now = api("GET", "/wifi", None, Duration::from_secs(10)).ok()
                            .and_then(|w| w["current"].as_str().filter(|s| !s.is_empty()).map(|s| format!("; the phone is on {s} now")))
                            .unwrap_or_else(|| "; the phone is not on Wi-Fi now".into());
                        Err(format!("{}{now}", e.trim_start_matches("phone: ")))
                    }
                }
            },
        )
        .tool(
            "wifi_forget",
            "Remove a saved Wi-Fi network from this phone (it will not join it again by itself).",
            obj(json!({"ssid": {"type": "string"}}), &["ssid"]),
            |_c, a| {
                let ssid = arg_str(&a, "ssid")?;
                api("POST", "/wifi/forget", Some(&json!({"ssid": ssid})), Duration::from_secs(10))?;
                Ok(format!("forgot {ssid}"))
            },
        )
        .tool(
            "hotspot",
            "This phone's Wi-Fi hotspot: without arguments shows it (on/off, name, password, devices connected); on=true/false switches it; ssid and password (8-63 characters) rename it or set a new password, open=true removes the password. The phone keeps its own Wi-Fi while sharing it; without Wi-Fi it shares mobile data (mind roaming).",
            obj(json!({
                "on": {"type": "boolean"},
                "ssid": {"type": "string"},
                "password": {"type": "string"},
                "open": {"type": "boolean", "description": "true = no password (anyone can join)"}
            }), &[]),
            |ctx, a| {
                let mut body = json!({});
                for k in ["on", "open"] {
                    if let Some(v) = a[k].as_bool() {
                        body[k] = json!(v);
                    }
                }
                for k in ["ssid", "password"] {
                    if let Some(v) = arg_str_opt(&a, k) {
                        body[k] = json!(v);
                    }
                }
                if body.as_object().is_some_and(|o| o.is_empty()) {
                    return Ok(hotspot_text(&api("GET", "/hotspot", None, Duration::from_secs(10))?));
                }
                let mut shown = body.clone();
                if shown.get("password").is_some() {
                    shown["password"] = json!("***");
                }
                ctx.log(format!("hotspot: {shown}"));
                let r = api("POST", "/hotspot", Some(&body), Duration::from_secs(60))?;
                Ok(hotspot_text(&r))
            },
        )
        .tool(
            "history",
            "SMS exchanged with a number on this phone line (newest last).",
            obj(json!({"chat_id": {"type": "string"}, "limit": {"type": "integer"}}), &["chat_id"]),
            |_c, a| {
                let chat = chat_id(arg_str(&a, "chat_id")?);
                let n = arg_u32_opt(&a, "limit").unwrap_or(20) as usize;
                let all: Vec<Value> = history().into_iter().filter(|h| h["chat"] == chat.as_str()).collect();
                let lines: Vec<String> = all[all.len().saturating_sub(n)..]
                    .iter()
                    .map(|h| format!("{} {} {}: {}", h["at"], h["id"].as_str().unwrap_or(""), if h["dir"] == "in" { chat.as_str() } else { "bia" }, h["text"].as_str().unwrap_or("")))
                    .collect();
                Ok(if lines.is_empty() { format!("no SMS with {chat} yet") } else { lines.join("\n") })
            },
        )
        .tool("chats", "Numbers bia exchanged SMS with, newest first.", obj(json!({}), &[]), |_c, _a| {
            let mut last: Vec<(String, Value)> = Vec::new();
            for h in history() {
                let c = h["chat"].as_str().unwrap_or("").to_string();
                last.retain(|(k, _)| *k != c);
                last.push((c, h));
            }
            last.reverse();
            Ok(if last.is_empty() { "no SMS yet".into() } else {
                last.iter().map(|(c, h)| format!("phone:{c} — {}", h["text"].as_str().unwrap_or("").chars().take(60).collect::<String>())).collect::<Vec<_>>().join("\n")
            })
        });
    srv.run_with(|ctx| {
        std::thread::Builder::new().name("phone-events".into()).spawn(move || poll(ctx)).ok();
    });
}

#[cfg(test)]
mod tests {
    use super::{chat_id, wifi_qr_text};

    #[test]
    fn wifi_qr() {
        assert_eq!(wifi_qr_text("bia-test", "bia-pass-123"), "WIFI:T:WPA;S:bia-test;P:bia-pass-123;;");
        assert_eq!(wifi_qr_text("a;b", ""), "WIFI:T:nopass;S:a\\;b;;");
        assert_eq!(wifi_qr_text("x", "p:\\\""), "WIFI:T:WPA;S:x;P:p\\:\\\\\\\";;");
    }

    #[test]
    fn numbers_become_chat_ids() {
        assert_eq!(chat_id("+34 625-640-810"), "+34625640810");
        assert_eq!(chat_id("0034625640810"), "+34625640810");
        assert_eq!(chat_id("phone:+372 5781 6857"), "+37257816857");
        assert_eq!(chat_id("625640810"), "625640810");
        assert_eq!(chat_id("Movistar"), "Movistar");
    }
}
