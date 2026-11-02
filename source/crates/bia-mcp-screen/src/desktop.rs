//! Runs bia_desktop.py (in this MCP's folder) for each call, as user bia on bia's session bus: that
//! is where the apps it starts register for accessibility; root's own bus is for fcitx5 only.

use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::{Command, Stdio};

const USER: &str = "bia";
/// wait may take 120 s; nothing else comes near it.
const TIMEOUT_SECS: &str = "135";

fn script() -> Result<PathBuf, String> {
    if let Ok(s) = std::env::var("BIA_DESKTOP_SCRIPT") {
        return Ok(PathBuf::from(s));
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    // MCPs/screen/bin/<arch>/bia-mcp-screen → MCPs/screen/bia_desktop.py
    let path = exe.ancestors().nth(3).map(|d| d.join("bia_desktop.py")).unwrap_or_default();
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!("{} is missing: an OS update (carry_update) puts it back", path.display()))
    }
}

/// The uid of a user, from /etc/passwd.
fn uid_of(user: &str) -> Option<u32> {
    let text = std::fs::read_to_string("/etc/passwd").ok()?;
    text.lines().find_map(|l| {
        let mut f = l.split(':');
        (f.next() == Some(user)).then(|| f.nth(1).and_then(|u| u.parse().ok())).flatten()
    })
}

fn command(script: &PathBuf, request: &str) -> Result<Command, String> {
    let uid = uid_of(USER).ok_or("there is no user bia")?;
    let mut c = Command::new("runuser");
    c.args(["-u", USER, "--", "timeout", TIMEOUT_SECS, "python3", "-I"]).arg(script).arg(request);
    let run = format!("/run/user/{uid}");
    c.env("HOME", "/home/bia")
        .env("USER", USER)
        .env("LOGNAME", USER)
        .env("LANG", "C.UTF-8")
        .env("DISPLAY", std::env::var("DISPLAY").unwrap_or_else(|_| ":0".into()))
        .env("XAUTHORITY", "/home/bia/.Xauthority")
        .env("DBUS_SESSION_BUS_ADDRESS", format!("unix:path={run}/bus"))
        .env("XDG_RUNTIME_DIR", run)
        .env_remove("LD_LIBRARY_PATH")
        .stdin(Stdio::null());
    Ok(c)
}

pub fn answer(stdout: &str, stderr: &str, ok: bool) -> Result<String, String> {
    match stdout.lines().last().and_then(|l| serde_json::from_str::<Value>(l).ok()) {
        Some(v) if v["ok"].as_bool() == Some(true) => Ok(v["text"].as_str().unwrap_or("").to_string()),
        Some(v) => Err(v["error"].as_str().unwrap_or("failed").to_string()),
        None if ok => Err("the desktop helper gave no answer".into()),
        None => {
            let tail: Vec<&str> = stderr.lines().rev().take(3).collect();
            Err(format!("the desktop helper failed: {}", tail.into_iter().rev().collect::<Vec<_>>().join(" ")))
        }
    }
}

pub fn call(cmd: &str, args: Value) -> Result<String, String> {
    let script = script()?;
    let request = json!({"cmd": cmd, "args": args}).to_string();
    let out = command(&script, &request)?.output().map_err(|e| format!("start the desktop helper: {e}"))?;
    answer(&String::from_utf8_lossy(&out.stdout), &String::from_utf8_lossy(&out.stderr), out.status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers() {
        assert_eq!(answer("noise\n{\"ok\":true,\"text\":\"fine\"}\n", "", true).unwrap(), "fine");
        assert_eq!(answer("{\"ok\":false,\"error\":\"stale\"}", "", true).unwrap_err(), "stale");
        assert!(answer("", "a\nb\nImportError: gi", false).unwrap_err().contains("ImportError"));
        assert!(answer("", "", true).is_err());
    }
}
