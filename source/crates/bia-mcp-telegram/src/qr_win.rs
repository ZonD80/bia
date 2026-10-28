//! Show the Telegram login QR in a dedicated kitty window on DISPLAY=:0 so it stays
//! visible while the agent console scrolls (as the WhatsApp MCP does for its pairing QR).
//! The window shows only the QR image — no captions underneath.

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

const CLASS: &str = "bia-telegram-qr";
const TITLE: &str = "Telegram QR";

static WINDOW_PID: AtomicU32 = AtomicU32::new(0);

fn display() -> String {
    std::env::var("DISPLAY").unwrap_or_else(|_| ":0".into())
}

fn xauthority() -> PathBuf {
    if let Ok(p) = std::env::var("XAUTHORITY") {
        let pb = PathBuf::from(&p);
        if pb.exists() {
            return pb;
        }
    }
    for cand in ["/run/bia/Xauthority", "/home/bia/.Xauthority"] {
        let p = PathBuf::from(cand);
        if p.exists() {
            return p;
        }
    }
    PathBuf::from("/run/bia/Xauthority")
}

fn gui_available() -> bool {
    Path::new("/tmp/.X11-unix/X0").exists()
        && Command::new("kitty")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
}

fn qr_dir() -> PathBuf {
    let d = crate::client::Tg::files_dir().join("telegram");
    let _ = fs::create_dir_all(&d);
    d
}

pub fn png_path() -> PathBuf {
    qr_dir().join("qr.png")
}

fn write_png(code: &str) -> Result<(), String> {
    use image::Luma;
    use qrcode::QrCode;
    let q = QrCode::new(code.as_bytes()).map_err(|e| format!("qr encode: {e}"))?;
    let img = q
        .render::<Luma<u8>>()
        .min_dimensions(480, 480)
        .quiet_zone(true)
        .build();
    let path = png_path();
    img.save(&path)
        .map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(())
}

fn kill_pid(pid: u32) {
    if pid == 0 {
        return;
    }
    unsafe {
        libc::kill(pid as i32, libc::SIGTERM);
    }
    let _ = Command::new("xdotool")
        .args(["search", "--class", CLASS, "windowkill"])
        .env("DISPLAY", display())
        .env("XAUTHORITY", xauthority())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Close the QR window if one is open and delete the temporary PNG.
pub fn close() {
    let pid = WINDOW_PID.swap(0, Ordering::SeqCst);
    kill_pid(pid);
    let _ = fs::remove_file(png_path());
}

fn window_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

fn spawn_watcher() -> Result<u32, String> {
    let png = png_path();
    // Image only — no text under the QR. Refresh when qr.png changes.
    let script = format!(
        r#"set -eu
PNG={png}
LAST=
while true; do
  SHA=$(sha256sum "$PNG" 2>/dev/null | awk '{{print $1}}' || true)
  if [ -n "$SHA" ] && [ "$SHA" != "$LAST" ]; then
    LAST=$SHA
    clear 2>/dev/null || true
    kitty +kitten icat --align=center "$PNG" 2>/dev/null || true
  fi
  sleep 1
done
"#,
        png = shell_quote(&png.to_string_lossy()),
    );
    let log = qr_dir().join("qr-window.log");
    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log)
        .ok();
    let mut cmd = Command::new("kitty");
    cmd.args([
        "--class",
        CLASS,
        "--title",
        TITLE,
        "--override",
        "confirm_os_window_close=0",
        "--override",
        "remember_window_size=no",
        "--override",
        "initial_window_width=36c",
        "--override",
        "initial_window_height=24c",
        "--override",
        "font_size=12",
        "--override",
        "font_family=DejaVu Sans Mono",
        "sh",
        "-c",
        &script,
    ])
    .env("DISPLAY", display())
    .env("XAUTHORITY", xauthority())
    .env("LANG", "C.UTF-8")
    .stdin(Stdio::null());
    if let Some(f) = log_file {
        if let Ok(err) = f.try_clone() {
            cmd.stdout(Stdio::from(f)).stderr(Stdio::from(err));
        }
    } else {
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
    }
    let child = cmd
        .spawn()
        .map_err(|e| format!("open QR window (kitty): {e}"))?;
    Ok(child.id())
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Write the latest QR PNG and ensure a dedicated window is showing it.
/// Returns a short human notice (for the console), or an error if GUI is unavailable.
pub fn show(code: &str, _art: &str, seq: u64, timeout_secs: u64) -> Result<String, String> {
    write_png(code)?;
    if !gui_available() {
        return Err("display :0 / kitty not available".into());
    }
    let cur = WINDOW_PID.load(Ordering::SeqCst);
    if !window_alive(cur) {
        if cur != 0 {
            WINDOW_PID.store(0, Ordering::SeqCst);
        }
        let pid = spawn_watcher()?;
        WINDOW_PID.store(pid, Ordering::SeqCst);
        let _ = Command::new("xdotool")
            .args(["search", "--sync", "--class", CLASS, "windowactivate", "--sync"])
            .env("DISPLAY", display())
            .env("XAUTHORITY", xauthority())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let png = png_path();
    Ok(format!(
        "[telegram qr #{seq}] PNG at {} (temporary; deleted when logged in). Also opened on display :0 in window \"{TITLE}\" and as a popup in root's web app (valid ~{timeout_secs}s; refreshes when it rotates). Scan it in the Telegram app of that account: Settings > Devices > Link Desktop Device.",
        png.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::shell_quote;

    #[test]
    fn quotes_paths() {
        assert_eq!(shell_quote("/tmp/a b"), "'/tmp/a b'");
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }
}
