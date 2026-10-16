//! Start, stop and restart the Google Chrome profile this MCP drives.
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

/// Chrome is not in Debian's sources: Google's .deb (amd64 and arm64) installs it and adds
/// Google's apt source, so later apt-get upgrades keep it current.
pub const MISSING: &str = "Google Chrome is not installed. It is not in Debian's sources: install the official package from Google with bash: curl -fLo /tmp/chrome.deb https://dl.google.com/linux/direct/google-chrome-stable_current_$(dpkg --print-architecture).deb && apt-get install -y /tmp/chrome.deb && rm /tmp/chrome.deb (it also adds Google's apt source for updates). Then call chrome_install again.";

const BINS: &[&str] = &[
    "/usr/bin/google-chrome-stable",
    "/usr/bin/google-chrome",
    "/opt/google/chrome/google-chrome",
    "/opt/google/chrome/chrome",
];

pub fn find_bin() -> Option<PathBuf> {
    BINS.iter()
        .map(PathBuf::from)
        .find(|p| p.is_file())
        .and_then(|p| p.canonicalize().ok().or(Some(p)))
}

pub fn require() -> Result<PathBuf, String> {
    find_bin().ok_or_else(|| MISSING.to_string())
}

pub fn running() -> bool {
    let Some(bin) = find_bin() else { return false };
    chrome_pids(&bin)
        .iter()
        .any(|pid| *pid != std::process::id())
}

/// Local CDP endpoint for chrome-devtools-mcp (`--browser-url`).
pub const DEBUG_PORT: u16 = 9222;
pub const DEBUG_URL: &str = "http://127.0.0.1:9222";

/// Non-default profile path. Chrome 136+ ignores `--remote-debugging-port` on the
/// default dir (`~/.config/google-chrome`), so automation must use another path.
pub const PROFILE: &str = "/home/bia/.config/bia-chrome";
const LEGACY_PROFILE: &str = "/home/bia/.config/google-chrome";

pub fn profile_dir() -> PathBuf {
    PathBuf::from(PROFILE)
}

pub fn status() -> Result<serde_json::Value, String> {
    let bin = require()?;
    Ok(serde_json::json!({
        "installed": true,
        "bin": bin,
        "running": running(),
        "profile": PROFILE,
        "debug_url": DEBUG_URL,
        "debug_ready": debug_ready(),
        "user": USER,
        "display": display(),
    }))
}

/// True when Chrome answers on the remote-debugging port.
pub fn debug_ready() -> bool {
    std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], DEBUG_PORT)),
        Duration::from_millis(200),
    )
    .is_ok()
}

/// Wait until the debugging port accepts connections (after start/restart).
pub fn wait_debug(secs: u64) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        if debug_ready() {
            return true;
        }
        sleep(Duration::from_millis(100));
    }
    false
}

pub fn start() -> Result<u32, String> {
    let bin = require()?;
    if running() {
        return Err(
            "Chrome is already running. Use restart to reload it with the extension.".into(),
        );
    }
    spawn(&bin)
}

pub fn stop() -> Result<String, String> {
    let bin = require()?;
    let pids = chrome_pids(&bin);
    if pids.is_empty() {
        return Ok("Chrome is not running".into());
    }
    for pid in &pids {
        signal(*pid, libc::SIGTERM);
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && running() {
        sleep(Duration::from_millis(100));
    }
    for pid in chrome_pids(&bin) {
        signal(pid, libc::SIGKILL);
    }
    let _ = fs::remove_file(pid_path());
    Ok(format!("stopped {} process(es)", pids.len()))
}

pub fn restart() -> Result<u32, String> {
    let _ = stop();
    sleep(Duration::from_millis(300));
    spawn(&require()?)
}

/// Chrome always runs as `bia` on the machine display `:0`.
const USER: &str = "bia";
pub const DISPLAY: &str = ":0";

fn spawn(bin: &Path) -> Result<u32, String> {
    let home = ensure_account()?;
    let profile = ensure_profile()?;
    // stop() may SIGKILL Chrome; without this the next launch shows "Restore pages?".
    clear_crash_restore_flags(&profile);
    let log = log_path();
    if let Some(d) = log.parent() {
        fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let runtime = runtime_dir()?;
    let mut cmd = Command::new("runuser");
    cmd.args(["-u", USER, "--", bin.to_str().unwrap_or("")])
        .args([
            &format!("--user-data-dir={}", profile.display()),
            &format!("--remote-debugging-port={DEBUG_PORT}"),
            "--remote-debugging-address=127.0.0.1",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-infobars",
            "--disable-gpu",
        ])
        .env("HOME", &home)
        .env("USER", USER)
        .env("LOGNAME", USER)
        .env("DISPLAY", DISPLAY)
        .env("XAUTHORITY", home.join(".Xauthority"))
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("LANG", "C.UTF-8")
        // root's session bus and fcitx5 settings from bia-session are not this user's
        .env_remove("DBUS_SESSION_BUS_ADDRESS")
        .env_remove("GLFW_IM_MODULE")
        .env_remove("GTK_IM_MODULE")
        .env_remove("QT_IM_MODULE")
        .env_remove("XMODIFIERS")
        .stdin(Stdio::null());
    if let Ok(f) = fs::OpenOptions::new().create(true).append(true).open(&log) {
        if let Ok(err) = f.try_clone() {
            cmd.stdout(Stdio::from(f)).stderr(Stdio::from(err));
        }
    }
    let child = cmd
        .spawn()
        .map_err(|e| format!("start chrome as {USER}: {e}"))?;
    let pid = child.id();
    remember(pid)?;
    let _ = Command::new("chown")
        .args([USER, crate::host::sock_path().to_str().unwrap_or("")])
        .status();
    let _ = Command::new("chmod")
        .args(["600", crate::host::sock_path().to_str().unwrap_or("")])
        .status();
    Ok(pid)
}

/// Ensure the non-default profile exists; move the legacy default profile once if needed.
pub fn ensure_profile() -> Result<PathBuf, String> {
    let _ = ensure_account()?;
    let profile = profile_dir();
    migrate_legacy_profile(&profile)?;
    fs::create_dir_all(&profile).map_err(|e| format!("{}: {e}", profile.display()))?;
    owned(PROFILE, "700")?;
    Ok(profile)
}

fn profile_populated(dir: &Path) -> bool {
    dir.join("Default").is_dir() || dir.join("Local State").is_file()
}

fn migrate_legacy_profile(profile: &Path) -> Result<(), String> {
    if profile_populated(profile) {
        return Ok(());
    }
    let legacy = PathBuf::from(LEGACY_PROFILE);
    if !profile_populated(&legacy) {
        return Ok(());
    }
    // Prefer rename so cookies/sessions stay on the same inode tree.
    if profile.exists() {
        let _ = fs::remove_dir(profile); // only succeeds if empty
    }
    match fs::rename(&legacy, profile) {
        Ok(()) => {
            let _ = owned(PROFILE, "700");
            Ok(())
        }
        Err(e) => Err(format!(
            "move Chrome profile {} → {}: {e}",
            legacy.display(),
            profile.display()
        )),
    }
}

fn ensure_account() -> Result<PathBuf, String> {
    let home = PathBuf::from("/home/bia");
    let exists = Command::new("id")
        .arg(USER)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !exists {
        let st = Command::new("adduser")
            .args([
                "--disabled-password",
                "--gecos",
                "",
                "--home",
                "/home/bia",
                "--shell",
                "/bin/bash",
                USER,
            ])
            .status()
            .map_err(|e| format!("adduser: {e}"))?;
        if !st.success() {
            return Err("could not create user bia".into());
        }
    }
    let dirs: &[(&str, &str)] = &[
        ("/home/bia", "755"),
        ("/home/bia/.config", "700"),
        (PROFILE, "700"),
        ("/home/bia/.local", "700"),
        ("/home/bia/.local/share", "700"),
    ];
    for (path, mode) in dirs {
        fs::create_dir_all(path).map_err(|e| format!("{path}: {e}"))?;
        owned(path, mode)?;
    }
    let _ = runtime_dir()?;
    Ok(home)
}

fn runtime_dir() -> Result<PathBuf, String> {
    let out = Command::new("id")
        .args(["-u", USER])
        .output()
        .map_err(|e| format!("id: {e}"))?;
    if !out.status.success() {
        return Err("user bia has no uid".into());
    }
    let uid = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let dir = PathBuf::from(format!("/run/user/{uid}"));
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    owned(dir.to_str().unwrap_or(""), "700")?;
    Ok(dir)
}

fn owned(path: &str, mode: &str) -> Result<(), String> {
    let st = Command::new("chown")
        .args(["bia:bia", path])
        .status()
        .map_err(|e| format!("chown {path}: {e}"))?;
    if !st.success() {
        return Err(format!("chown {path} failed"));
    }
    let st = Command::new("chmod")
        .args([mode, path])
        .status()
        .map_err(|e| format!("chmod {path}: {e}"))?;
    if !st.success() {
        return Err(format!("chmod {path} failed"));
    }
    Ok(())
}

pub fn wait_connected(connected: impl Fn() -> bool, secs: u64) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        if connected() {
            return true;
        }
        sleep(Duration::from_millis(200));
    }
    connected()
}

fn display() -> String {
    DISPLAY.into()
}

/// Pretend the last exit was clean so Chrome skips the "Restore pages?" interstitial.
/// Flags like `--disable-session-crashed-bubble` were removed from Chromium; editing
/// these prefs is the supported workaround after SIGKILL/restart.
fn clear_crash_restore_flags(profile: &Path) {
    let local_state = profile.join("Local State");
    let prefs = profile.join("Default/Preferences");
    patch_json_file(&local_state, |v| {
        let Some(obj) = v.as_object_mut() else {
            return;
        };
        let profile = obj
            .entry("profile")
            .or_insert_with(|| serde_json::json!({}));
        if let Some(p) = profile.as_object_mut() {
            p.insert("exited_cleanly".into(), serde_json::Value::Bool(true));
        }
    });
    patch_json_file(&prefs, |v| {
        let Some(obj) = v.as_object_mut() else {
            return;
        };
        let profile = obj
            .entry("profile")
            .or_insert_with(|| serde_json::json!({}));
        if let Some(p) = profile.as_object_mut() {
            p.insert(
                "exit_type".into(),
                serde_json::Value::String("Normal".into()),
            );
        }
    });
    // MCP often runs as root; Chrome runs as bia — keep ownership correct.
    for path in [&local_state, &prefs] {
        if path.is_file() {
            let _ = owned(path.to_str().unwrap_or(""), "600");
        }
    }
}

fn patch_json_file(path: &Path, f: impl FnOnce(&mut serde_json::Value)) {
    let Ok(raw) = fs::read_to_string(path) else {
        return;
    };
    if let Ok(mut v) = serde_json::from_str::<serde_json::Value>(&raw) {
        if v.is_object() {
            f(&mut v);
            if let Ok(out) = serde_json::to_vec(&v) {
                let _ = fs::write(path, out);
            }
            return;
        }
    }
    // Fallback if Preferences isn't strict JSON for some reason.
    let patched = raw
        .replace("\"exited_cleanly\":false", "\"exited_cleanly\":true")
        .replace("\"exited_cleanly\": false", "\"exited_cleanly\": true");
    let patched = regex_replace_exit_type(&patched);
    if patched != raw {
        let _ = fs::write(path, patched);
    }
}

fn regex_replace_exit_type(s: &str) -> String {
    // "exit_type":"Crashed" / "exit_type": "Crashed" → Normal
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("\"exit_type\"") {
        out.push_str(&rest[..i]);
        let after = &rest[i + "\"exit_type\"".len()..];
        let trimmed = after.trim_start();
        if let Some(colon) = trimmed.strip_prefix(':') {
            let v = colon.trim_start();
            if let Some(rest_v) = v.strip_prefix('"') {
                if let Some(end) = rest_v.find('"') {
                    out.push_str("\"exit_type\": \"Normal\"");
                    rest = &rest_v[end + 1..];
                    continue;
                }
            }
        }
        out.push_str("\"exit_type\"");
        rest = after;
    }
    out.push_str(rest);
    out
}

fn pid_path() -> PathBuf {
    PathBuf::from("/run/bia/chrome.pid")
}

fn log_path() -> PathBuf {
    let files = std::env::var("BIA_FILES").unwrap_or_else(|_| "/home/bia/files".into());
    PathBuf::from(files)
        .join("state")
        .join("chrome")
        .join("chrome.log")
}

fn remember(pid: u32) -> Result<(), String> {
    if let Some(d) = pid_path().parent() {
        let _ = fs::create_dir_all(d);
    }
    let mut f = fs::File::create(pid_path()).map_err(|e| e.to_string())?;
    write!(f, "{pid}").map_err(|e| e.to_string())
}

fn signal(pid: u32, sig: i32) {
    unsafe { libc::kill(pid as i32, sig) };
}

fn chrome_pids(bin: &Path) -> Vec<u32> {
    let name = bin.file_name().and_then(|s| s.to_str()).unwrap_or("chrome");
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir("/proc") else {
        return out;
    };
    for ent in entries.flatten() {
        let pid: u32 = match ent.file_name().to_string_lossy().parse() {
            Ok(n) => n,
            Err(_) => continue,
        };
        let cmd = fs::read(ent.path().join("cmdline")).unwrap_or_default();
        let text = String::from_utf8_lossy(&cmd);
        let exe = text.split('\0').next().unwrap_or("");
        if exe.ends_with(name)
            || exe.ends_with("/chrome")
            || exe.contains("/opt/google/chrome/")
            || exe.contains("google-chrome")
        {
            out.push(pid);
        }
    }
    out
}
