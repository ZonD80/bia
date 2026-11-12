//! The chat for a person at a terminal: `bia-bridge` with nothing else (or `chat`, or just an
//! address). It asks for what is missing (a name, the bia's address), files the certificate
//! request, waits for the owner's approval and then is a live conversation: what bia says
//! appears as it comes, a line typed here goes to bia on Enter.
//!
//! On a Unix terminal the line being typed is kept at the bottom while messages scroll above
//! it (raw mode, a small editor of our own: no dependency). Without a terminal (a pipe), and
//! on Windows, it reads whole lines and prints messages as they come.
use crate::profile::{self, Profile};
use crate::{ipc, link, Args};
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::{IsTerminal, Read, Write};
use std::sync::mpsc;
use std::time::Duration;

const CHAT_HELP: &str = "Enter sends the line to bia. Commands:
  /file <path> [text]   send a file (with a caption)
  /history [n]          the last n messages (20)
  /status               the link, the certificate
  /quit                 leave (Ctrl-C too); the link stays up in the background, so what bia
                        says meanwhile is here next time";

// ---- asking, before the chat ----

fn ask(prompt: &str, default: Option<&str>) -> Result<String> {
    let mut out = std::io::stdout();
    match default {
        Some(d) if !d.is_empty() => write!(out, "{prompt} [{d}]: ")?,
        _ => write!(out, "{prompt}: ")?,
    }
    out.flush()?;
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line)? == 0 {
        bail!("no answer (the input ended)");
    }
    let line = line.trim();
    Ok(if line.is_empty() { default.unwrap_or("").to_string() } else { line.to_string() })
}

fn me_file() -> std::path::PathBuf {
    profile::home().join("me.json")
}

/// A name is fit for a person's identity on a bia.
fn check_identity(n: &str) -> Result<()> {
    if !profile::valid_name(n) || n == "root" || n == "bia" {
        bail!("{n:?} cannot be a name here: letters, digits and - _ . @; not root or bia");
    }
    Ok(())
}

/// The first usable form of the computer's user name, as a suggestion.
fn os_user() -> String {
    let raw = std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_default();
    let n: String = raw.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() || "-_.".contains(c) { c } else { '-' }).collect();
    let n = n.trim_matches(|c| c == '-' || c == '.').to_string();
    if check_identity(&n).is_ok() { n } else { String::new() }
}

/// Who is typing: `--agent` / `--identity`, else the name remembered from the last time,
/// else asked once and remembered.
fn who(a: &Args) -> Result<String> {
    if let Some(n) = a.given_agent()? {
        return Ok(n);
    }
    if let Some(n) = a.opt("--identity") {
        let n = crate::with_owner(n, a.owner()?.as_deref()).map_err(|e| anyhow!(e))?;
        check_identity(&n)?;
        return Ok(n);
    }
    let saved: Option<Value> = std::fs::read(me_file()).ok().and_then(|b| serde_json::from_slice(&b).ok());
    if let Some(n) = saved.as_ref().and_then(|v| v["name"].as_str()).filter(|n| check_identity(n).is_ok()) {
        return Ok(n.to_string());
    }
    println!("A bia knows everyone by a name of their own. Yours is asked once and kept on this computer.");
    loop {
        let n = ask("Your name for bias (letters, digits, - _ .)", Some(&os_user()))?;
        match check_identity(&n) {
            Ok(()) => {
                std::fs::create_dir_all(profile::home())?;
                std::fs::write(me_file(), serde_json::to_vec_pretty(&json!({"name": n}))?)?;
                return Ok(n);
            }
            Err(e) => println!("{e:#}"),
        }
    }
}

/// `host`, `host:8443` or a full address → `https://host[:port]`.
pub fn address(s: &str) -> Result<String> {
    let s = s.trim().trim_end_matches('/');
    if s.is_empty() {
        bail!("an address is needed, like https://192.168.1.20 or bia.example.com:8443");
    }
    if s.starts_with("http://") {
        bail!("{s}: a bia answers on https:// only");
    }
    let full = if s.starts_with("https://") { s.to_string() } else { format!("https://{s}") };
    bia_pki::client::origin(&full)
}

fn fingerprint(sha: &str) -> String {
    sha.to_uppercase().as_bytes().chunks(2).map(|c| String::from_utf8_lossy(c).into_owned()).collect::<Vec<_>>().join(":")
}

/// A new profile for `identity` at `url`: pin the CA, file the certificate request (as a
/// person: role user), start the daemon.
fn request(a: &Args, identity: &str, url: &str) -> Result<Profile> {
    let name = crate::profile_name(url, a.opt("--as"))?;
    if let Ok(old) = Profile::load(identity, &name) {
        let _ = ipc::stop(&old);
        let _ = std::fs::remove_dir_all(old.dir());
    }
    println!("Reaching {url} …");
    let (ca, sha) = link::fetch_ca(url, a.opt("--ca-sha256"))?;
    let mut p = Profile {
        name,
        url: url.to_string(),
        identity: identity.to_string(),
        ca_sha256: sha.clone(),
        role: "user".into(),
        note: a.opt("--note").map(str::to_string).unwrap_or_else(|| format!("{identity}, a person at a terminal (bia-bridge)")),
        ..Default::default()
    };
    link::request_cert(&mut p, &ca)?;
    ipc::ensure(&p)?;
    println!("It is a bia. Its CA fingerprint (SHA-256), kept from now on:\n  {}", fingerprint(&sha));
    if a.opt("--ca-sha256").is_none() {
        println!("If you can, compare it with what the bia's owner sees (its web app: Settings → Agent CA).");
    }
    Ok(p)
}

/// Which bia: the address given, `--bia`, the only one known, or a choice.
fn choose(a: &Args, identity: &str, url: Option<&str>) -> Result<Profile> {
    let known = Profile::of_agent(identity);
    if let Some(u) = url {
        let u = address(u)?;
        if let Some(p) = known.iter().find(|p| p.url == u || bia_pki::client::split_url(&p.url).ok().map(|x| x.0) == bia_pki::client::split_url(&u).ok().map(|x| x.0) && a.opt("--as").is_none_or(|n| n == p.name)) {
            return Ok(p.clone());
        }
        return request(a, identity, &u);
    }
    if let Some(n) = a.opt("--bia").map(str::to_string).or_else(|| std::env::var("BIA_BRIDGE_PROFILE").ok()).filter(|n| !n.is_empty()) {
        return Profile::load(identity, &n);
    }
    if known.len() == 1 {
        return Ok(known.into_iter().next().unwrap());
    }
    if known.is_empty() {
        println!("No bia is set up for {identity} on this computer yet.");
    } else {
        println!("Your bias:");
        for (i, p) in known.iter().enumerate() {
            println!("  {}. {}  {}  {}", i + 1, p.name, p.url, if p.state == "active" { "" } else { &p.state });
        }
    }
    loop {
        let prompt = if known.is_empty() { "The bia's address (like 192.168.1.20 or bia.example.com:8443)" } else { "A number, or the address of another bia" };
        let ans = ask(prompt, None)?;
        if let Some(p) = ans.parse::<usize>().ok().and_then(|n| known.get(n.wrapping_sub(1))) {
            return Ok(p.clone());
        }
        if let Some(p) = known.iter().find(|p| p.name == ans) {
            return Ok(p.clone());
        }
        match address(&ans).and_then(|u| request(a, identity, &u)) {
            Ok(p) => return Ok(p),
            Err(e) => println!("{e:#}"),
        }
    }
}

/// Until the owner has approved; a refusal or an expired request can be asked again.
fn approved(a: &Args, p: &mut Profile) -> Result<()> {
    let mut told = false;
    loop {
        let s = ipc::call(p, json!({"op": "status"}))?;
        match s["state"].as_str().unwrap_or("") {
            "active" => {
                if told {
                    println!("Approved.{}", s["message"].as_str().filter(|m| !m.is_empty()).map(|m| format!(" The owner says: {}", clean(m))).unwrap_or_default());
                }
                *p = Profile::load(&p.identity, &p.name)?;
                return Ok(());
            }
            "pending" => {
                if !told {
                    told = true;
                    let id = s["request"].as_str().unwrap_or("");
                    let short = &id[..8.min(id.len())];
                    println!("\nAsked {} to let {} in (request {short}).", p.url, p.identity);
                    println!("Its owner approves it: in the bia's web app under Settings → Certificates → Requests from other agents,\nor by telling bia “approve {short}”. Waiting for that …");
                    println!("(Ctrl-C stops the waiting, not the request: run bia-bridge again later.)");
                }
                std::thread::sleep(Duration::from_secs(2));
            }
            state => {
                let why = s["message"].as_str().filter(|m| !m.is_empty()).map(|m| format!(": {}", clean(m))).unwrap_or_default();
                println!("{}", if state == "denied" { format!("The owner said no{why}.") } else { format!("The request ran out before anyone approved it{why}.") });
                if !ask("Ask again? (y/n)", Some("y"))?.to_lowercase().starts_with('y') {
                    bail!("not connected to {}", p.name);
                }
                let url = p.url.clone();
                *p = request(a, &p.identity.clone(), &url)?;
                told = false;
            }
        }
    }
}

// ---- what is shown ----

/// Text from the other side, safe for a terminal: no escape sequences or other control
/// characters (they could repaint the screen or rewrite what was said); newlines and tabs stay.
pub fn clean(s: &str) -> String {
    s.chars()
        .filter_map(|c| match c {
            '\n' => Some('\n'),
            '\t' => Some(' '),
            '\r' => None,
            c if c.is_control() => None,
            // right-to-left overrides and their kin could reorder a line
            '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' => None,
            c => Some(c),
        })
        .collect()
}

/// Local HH:MM (UTC with a Z where the system's clock zone is out of reach).
fn clock(secs: u64) -> String {
    #[cfg(unix)]
    {
        let t = secs as libc::time_t;
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        if !unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
            return format!("{:02}:{:02}", tm.tm_hour, tm.tm_min);
        }
    }
    format!("{:02}:{:02}Z", secs % 86_400 / 3600, secs % 3600 / 60)
}

struct Look {
    colour: bool,
}

impl Look {
    fn paint(&self, code: &str, s: &str) -> String {
        if self.colour { format!("\x1b[{code}m{s}\x1b[0m") } else { s.to_string() }
    }
    /// One message: `HH:MM who  text`, further lines indented under the text.
    fn line(&self, ts: u64, who: &str, code: &str, text: &str, files: &[String]) -> String {
        let pad = " ".repeat(6 + who.chars().count() + 2);
        let mut body = clean(text).replace('\n', &format!("\n{pad}"));
        for f in files {
            if !body.is_empty() {
                body.push_str(&format!("\n{pad}"));
            }
            body.push_str(&self.paint("2", &format!("[file] {}", clean(f))));
        }
        format!("{} {}  {body}", self.paint("2", &clock(ts)), self.paint(code, who))
    }
    fn note(&self, text: &str) -> String {
        self.paint("2", &format!("      {}", clean(text)))
    }
    /// An entry of the daemon's inbox (what bia said, or a note of the bridge's own).
    fn entry(&self, m: &Value) -> String {
        let text = m["text"].as_str().unwrap_or("");
        if m["kind"] == "note" {
            return self.note(text);
        }
        let files: Vec<String> = m["files"].as_array().into_iter().flatten().filter_map(|f| f.as_str().map(str::to_string)).collect();
        let who = if m["kind"] == "edit" { "bia (edited)" } else { "bia" };
        self.line(m["ts"].as_u64().unwrap_or(0), who, "1;36", text, &files)
    }
    /// A message of the chat's history (both sides; times in milliseconds, files by name).
    fn past(&self, m: &Value, me: &str) -> String {
        let files: Vec<String> = m["files"].as_array().into_iter().flatten().filter_map(|f| f["name"].as_str().map(str::to_string)).collect();
        let from = m["from"].as_str().unwrap_or("");
        let (who, code) = if from == "bia" { ("bia", "1;36") } else if from == me { ("you", "32") } else { (from, "33") };
        self.line(m["ts"].as_u64().unwrap_or(0) / 1000, &clean(who), code, m["text"].as_str().unwrap_or(""), &files)
    }
}

// ---- the terminal ----

/// The screen: messages above, the line being typed kept last (when `raw`).
struct Screen {
    raw: bool,
    line: String,
}

fn cols() -> usize {
    #[cfg(unix)]
    {
        let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
        if unsafe { libc::ioctl(1, libc::TIOCGWINSZ, &mut ws) } == 0 && ws.ws_col > 0 {
            return ws.ws_col as usize;
        }
    }
    80
}

/// Columns a character takes, erring wide: a redraw must never spill to a second row.
fn wide(c: char) -> usize {
    if c.is_ascii() { 1 } else { 2 }
}

impl Screen {
    /// The end of the typed line that fits one row after the prompt.
    fn visible(&self, room: usize) -> String {
        let mut used = 0;
        let mut start = self.line.len();
        for (i, c) in self.line.char_indices().rev() {
            if used + wide(c) > room {
                break;
            }
            used += wide(c);
            start = i;
        }
        if start == 0 { self.line.clone() } else { format!("…{}", &self.line[start..]) }
    }
    fn prompt(&self) {
        if self.raw {
            let room = cols().saturating_sub(5).max(8);
            print!("\r\x1b[K> {}", self.visible(room));
            let _ = std::io::stdout().flush();
        }
    }
    /// Print above the typed line.
    fn say(&self, text: &str) {
        if self.raw {
            print!("\r\x1b[K");
        }
        println!("{text}");
        self.prompt();
    }
}

#[cfg(unix)]
struct RawMode(libc::termios);

#[cfg(unix)]
impl RawMode {
    fn on() -> Option<RawMode> {
        let mut t: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(0, &mut t) } != 0 {
            return None;
        }
        let old = t;
        // keys one by one, shown by us; Ctrl-C comes as a key so the terminal is always put back
        t.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG | libc::IEXTEN);
        t.c_iflag &= !(libc::IXON | libc::ICRNL);
        t.c_cc[libc::VMIN] = 1;
        t.c_cc[libc::VTIME] = 0;
        (unsafe { libc::tcsetattr(0, libc::TCSADRAIN, &t) } == 0).then_some(RawMode(old))
    }
}

#[cfg(unix)]
impl Drop for RawMode {
    fn drop(&mut self) {
        unsafe { libc::tcsetattr(0, libc::TCSADRAIN, &self.0) };
    }
}

enum Ev {
    /// Bytes typed (raw mode).
    Keys(Vec<u8>),
    /// A whole line (no terminal).
    Line(String),
    /// The input ended.
    Eof,
    /// The daemon's inbox entries.
    Msgs(Vec<Value>),
    Online(bool),
    Fail(String),
}

/// What a key stroke leads to.
#[derive(Debug, PartialEq)]
enum Key {
    Redraw,
    Submit(String),
    Quit,
}

/// Feed typed bytes into the line. `hold` keeps the bytes of an unfinished UTF-8 character or
/// escape sequence between reads.
fn typed(line: &mut String, hold: &mut Vec<u8>, bytes: &[u8]) -> Vec<Key> {
    hold.extend_from_slice(bytes);
    let mut out = Vec::new();
    let mut i = 0;
    while i < hold.len() {
        let b = hold[i];
        match b {
            b'\r' | b'\n' => {
                out.push(Key::Submit(std::mem::take(line)));
                i += 1;
            }
            0x03 => return vec![Key::Quit],
            0x04 => {
                if line.is_empty() {
                    return vec![Key::Quit];
                }
                i += 1;
            }
            0x7f | 0x08 => {
                line.pop();
                i += 1;
            }
            0x15 => {
                line.clear();
                i += 1;
            }
            0x17 => {
                let t = line.trim_end().len();
                line.truncate(line[..t].rfind(' ').map_or(0, |p| p + 1));
                i += 1;
            }
            0x1b => {
                // an escape sequence (arrows, function keys): skipped whole
                let rest = &hold[i + 1..];
                match rest.first() {
                    None => break, // may still be coming
                    Some(b'[') | Some(b'O') => match rest[1..].iter().position(|c| (0x40..=0x7e).contains(c)) {
                        Some(end) => i += 2 + end + 1,
                        None => break,
                    },
                    Some(_) => i += 2,
                }
            }
            b'\t' => {
                line.push(' ');
                i += 1;
            }
            b if b < 0x20 => i += 1,
            b => {
                let n = match b {
                    0x00..=0x7f => 1,
                    0xc0..=0xdf => 2,
                    0xe0..=0xef => 3,
                    0xf0..=0xf7 => 4,
                    _ => {
                        i += 1; // a stray continuation byte
                        continue;
                    }
                };
                if i + n > hold.len() {
                    break; // the rest of the character is still coming
                }
                if let Ok(s) = std::str::from_utf8(&hold[i..i + n]) {
                    line.push_str(s);
                }
                i += n;
            }
        }
    }
    hold.drain(..i);
    out.push(Key::Redraw);
    out
}

// ---- the conversation ----

fn show_history(p: &Profile, look: &Look, scr: &Screen, limit: u64, seen: &mut HashSet<String>) -> Result<usize> {
    let v = ipc::call(p, json!({"op": "history", "limit": limit}))?;
    let msgs = v["messages"].as_array().cloned().unwrap_or_default();
    for m in &msgs {
        seen.insert(m["id"].as_str().unwrap_or("").to_string());
        scr.say(&look.past(m, &p.identity));
    }
    Ok(msgs.len())
}

/// One line typed. Ok(false) = leave.
fn submit(p: &Profile, look: &Look, scr: &Screen, text: &str, seen: &mut HashSet<String>) -> Result<bool> {
    let text = text.trim();
    if text.is_empty() {
        scr.prompt();
        return Ok(true);
    }
    let (cmd, rest) = text.split_once(char::is_whitespace).unwrap_or((text, ""));
    let send = |text: &str, files: Vec<String>| -> Result<()> {
        let v = ipc::call(p, json!({"op": "send", "text": text, "files": files, "reply_to": ""}))?;
        scr.say(&look.line(profile::now_secs(), "you", "32", text, &files));
        if v["sent"] != true {
            scr.say(&look.note("not connected right now: it goes as soon as the link is back"));
        }
        Ok(())
    };
    match cmd {
        "/quit" | "/exit" | "/q" => return Ok(false),
        "/help" | "/?" => scr.say(CHAT_HELP),
        "/status" => scr.say(&crate::status_text(&ipc::call(p, json!({"op": "status"}))?)),
        "/history" => {
            let n = rest.trim().parse::<u64>().unwrap_or(20);
            if let Err(e) = show_history(p, look, scr, n, seen) {
                scr.say(&look.note(&format!("no history right now: {e:#}")));
            }
        }
        "/file" => {
            let (path, caption) = rest.trim().split_once(char::is_whitespace).unwrap_or((rest.trim(), ""));
            let path = path.strip_prefix("~/").and_then(|r| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(r))).unwrap_or_else(|| path.into());
            match std::fs::canonicalize(&path) {
                Ok(f) if f.is_file() => {
                    if let Err(e) = send(caption.trim(), vec![f.display().to_string()]) {
                        scr.say(&look.note(&format!("not sent: {e:#}")));
                    }
                }
                Ok(f) => scr.say(&look.note(&format!("{}: not a file", f.display()))),
                Err(e) => scr.say(&look.note(&format!("{}: {e}", path.display()))),
            }
        }
        c if c.starts_with('/') && !c[1..].contains('/') => scr.say(&look.note(&format!("{c}: no such command; /help lists them"))),
        _ => {
            if let Err(e) = send(text, vec![]) {
                scr.say(&look.note(&format!("not sent: {e:#}")));
            }
        }
    }
    Ok(true)
}

pub fn run(a: &Args, url: Option<&str>) -> Result<()> {
    profile::migrate();
    let tty = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let identity = who(a)?;
    let mut p = choose(a, &identity, url)?;
    ipc::ensure(&p)?;
    approved(a, &mut p)?;

    let look = Look { colour: tty && cfg!(unix) && std::env::var_os("NO_COLOR").is_none() };
    println!("{}", look.paint("1", &format!("{} ({}) — you are {}", p.name, p.url, p.identity)));
    println!("{}", look.paint("2", "Enter sends. /help for commands, Ctrl-C leaves."));

    #[cfg(unix)]
    let raw_mode = if tty { RawMode::on() } else { None };
    #[cfg(unix)]
    let raw = raw_mode.is_some();
    #[cfg(not(unix))]
    let raw = false;
    let mut scr = Screen { raw, line: String::new() };

    // the last of the conversation, then whatever came while away (minus what history showed)
    let mut seen = HashSet::new();
    let mut online = ipc::call(&p, json!({"op": "status"})).map(|s| s["online"] == true).unwrap_or(false);
    if !online {
        // the link takes a moment after the daemon starts
        for _ in 0..25 {
            std::thread::sleep(Duration::from_millis(200));
            online = ipc::call(&p, json!({"op": "status"})).map(|s| s["online"] == true).unwrap_or(false);
            if online {
                break;
            }
        }
    }
    if online {
        let _ = show_history(&p, &look, &scr, 10, &mut seen);
    } else {
        scr.say(&look.note("not connected yet; what you write goes when the link is up"));
    }
    scr.prompt();

    let (tx, rx) = mpsc::channel::<Ev>();
    {
        let (tx, p) = (tx.clone(), p.clone());
        std::thread::spawn(move || loop {
            match ipc::call(&p, json!({"op": "wait", "timeout": 20})) {
                Ok(v) => {
                    let msgs = v["messages"].as_array().cloned().unwrap_or_default();
                    if !msgs.is_empty() && tx.send(Ev::Msgs(msgs)).is_err() {
                        return;
                    }
                }
                Err(e) => {
                    let _ = tx.send(Ev::Fail(format!("{e:#}")));
                    std::thread::sleep(Duration::from_secs(3));
                }
            }
        });
    }
    {
        let (tx, p, mut was) = (tx.clone(), p.clone(), online);
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(2));
            if let Ok(s) = ipc::call(&p, json!({"op": "status"})) {
                let now = s["online"] == true;
                if now != was {
                    was = now;
                    if tx.send(Ev::Online(now)).is_err() {
                        return;
                    }
                }
            }
        });
    }
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin();
        if raw {
            let mut buf = [0u8; 256];
            loop {
                match stdin.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if tx.send(Ev::Keys(buf[..n].to_vec())).is_err() {
                            return;
                        }
                    }
                }
            }
        } else {
            let mut line = String::new();
            while matches!(stdin.read_line(&mut line), Ok(n) if n > 0) {
                if tx.send(Ev::Line(std::mem::take(&mut line))).is_err() {
                    return;
                }
            }
        }
        let _ = tx.send(Ev::Eof);
    });

    let mut hold = Vec::new();
    let mut failing = false;
    'chat: for ev in rx {
        match ev {
            Ev::Msgs(msgs) => {
                failing = false;
                for m in msgs.iter().filter(|m| !seen.contains(m["id"].as_str().unwrap_or(""))) {
                    scr.say(&look.entry(m));
                }
            }
            Ev::Online(on) => scr.say(&look.note(if on { "connected again" } else { "the link is down; trying to get it back (what you write waits)" })),
            Ev::Fail(e) => {
                if !failing {
                    failing = true;
                    scr.say(&look.note(&format!("the background link has a problem: {e}")));
                }
            }
            Ev::Line(l) => {
                if !submit(&p, &look, &scr, &l, &mut seen)? {
                    break 'chat;
                }
            }
            Ev::Keys(k) => {
                for key in typed(&mut scr.line, &mut hold, &k) {
                    match key {
                        Key::Quit => break 'chat,
                        Key::Redraw => scr.prompt(),
                        Key::Submit(l) => {
                            if !submit(&p, &look, &scr, &l, &mut seen)? {
                                break 'chat;
                            }
                        }
                    }
                }
            }
            Ev::Eof => break 'chat,
        }
    }
    if raw {
        print!("\r\x1b[K");
    }
    #[cfg(unix)]
    drop(raw_mode);
    println!("{}", look.paint("2", &format!("Left the chat. The link to {} stays up in the background (bia-bridge stop --agent {} --bia {} ends it).", p.name, p.identity, p.name)));
    // the reader threads are still waiting on the daemon and the keyboard
    let _ = std::io::stdout().flush();
    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_take_a_host_a_port_or_a_full_url() {
        assert_eq!(address("192.168.1.20").unwrap(), "https://192.168.1.20");
        assert_eq!(address(" bia.example.com:8443/ ").unwrap(), "https://bia.example.com:8443");
        assert_eq!(address("https://bia.example.com:443/").unwrap(), "https://bia.example.com");
        assert_eq!(address("[::1]:8443").unwrap(), "https://[::1]:8443");
        assert!(address("http://bia.example.com").is_err() && address("").is_err() && address("host:port").is_err());
    }

    #[test]
    fn what_bia_says_cannot_drive_the_terminal() {
        assert_eq!(clean("hi\x1b[2J\x1b]0;x\x07 there\r\nline\ttwo\u{202e}"), "hi[2J]0;x there\nline two");
        let look = Look { colour: false };
        let l = look.line(0, "bia", "1", "one\ntwo", &["/tmp/a.png".into()]);
        assert!(l.ends_with("bia  one\n           two\n           [file] /tmp/a.png"), "{l:?}");
        assert_eq!(look.entry(&json!({"kind": "note", "text": "[bridge] moved"})), "      [bridge] moved");
    }

    #[test]
    fn typing_edits_the_line() {
        let (mut line, mut hold) = (String::new(), Vec::new());
        assert_eq!(typed(&mut line, &mut hold, b"hello wor"), [Key::Redraw]);
        // arrows and function keys are skipped, also when they arrive in two reads
        typed(&mut line, &mut hold, b"\x1b[");
        assert_eq!(hold, b"\x1b[");
        typed(&mut line, &mut hold, b"Ald\x1bOP\x1b[15~");
        assert_eq!(line, "hello world");
        // a character split between reads
        let e = "é ж 好".as_bytes();
        typed(&mut line, &mut hold, &e[..1]);
        typed(&mut line, &mut hold, &e[1..]);
        assert_eq!(line, "hello worldé ж 好");
        typed(&mut line, &mut hold, b"\x7f\x7f");
        assert_eq!(line, "hello worldé ж");
        typed(&mut line, &mut hold, b"\x17\x17");
        assert_eq!(line, "hello ");
        assert_eq!(typed(&mut line, &mut hold, b"you\rnext"), [Key::Submit("hello you".into()), Key::Redraw]);
        assert_eq!(line, "next");
        // Ctrl-D leaves only an empty line, Ctrl-U empties it, Ctrl-C always leaves
        assert_eq!(typed(&mut line, &mut hold, b"\x04"), [Key::Redraw]);
        typed(&mut line, &mut hold, b"\x15");
        assert_eq!(typed(&mut line, &mut hold, b"\x04"), [Key::Quit]);
        line.push('x');
        assert_eq!(typed(&mut line, &mut hold, b"\x03"), [Key::Quit]);
    }

    #[test]
    fn a_long_line_shows_its_end() {
        let s = Screen { raw: true, line: "abcdefghij".into() };
        assert_eq!(s.visible(20), "abcdefghij");
        assert_eq!(s.visible(4), "…ghij");
        let s = Screen { raw: true, line: "aжжж".into() };
        assert_eq!(s.visible(4), "…жж");
    }
}
