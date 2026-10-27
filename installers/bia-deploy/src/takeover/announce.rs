//! Takeover messages for everyone watching the server, not only the terminal that runs it:
//! every other terminal (ssh sessions, ttys) and the kernel consoles (the provider's web or
//! VNC console, the serial console), as `bia takeover: <line>`. Lines that hold a secret
//! (the gate links, the land secret) stay on the terminal that runs the takeover.

/// Show `line` here (stderr) and on every other terminal and console.
pub fn all(line: &str) {
    eprintln!("{line}");
    mirror(line);
}

/// Show `line` on every other terminal and console only.
#[cfg(unix)]
pub fn mirror(line: &str) {
    use std::io::Write;
    use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt};
    if line.trim().is_empty() {
        return;
    }
    let text: String = line.trim_start_matches('\n').lines().map(|l| format!("bia takeover: {l}\r\n")).collect();
    let mut seen = own_ttys();
    for p in targets() {
        let Ok(m) = std::fs::metadata(&p) else { continue };
        let dev = split(m.rdev());
        if !m.file_type().is_char_device() || seen.contains(&dev) {
            continue;
        }
        seen.push(dev);
        // never block on a stuck serial line, never become its controlling process
        if let Ok(mut f) = std::fs::OpenOptions::new().write(true).custom_flags(libc::O_NOCTTY | libc::O_NONBLOCK).open(&p) {
            let _ = f.write_all(text.as_bytes());
        }
    }
}

#[cfg(not(unix))]
pub fn mirror(_line: &str) {}

/// Open terminals (/dev/pts/N) and the kernel consoles (/sys/class/tty/console/active;
/// tty0 is the VT in front).
#[cfg(unix)]
fn targets() -> Vec<std::path::PathBuf> {
    let mut out = vec![];
    if let Ok(rd) = std::fs::read_dir("/dev/pts") {
        for e in rd.flatten() {
            if e.file_name().to_string_lossy().bytes().all(|b| b.is_ascii_digit()) {
                out.push(e.path());
            }
        }
    }
    for name in std::fs::read_to_string("/sys/class/tty/console/active").unwrap_or_default().split_whitespace() {
        let name = if name == "tty0" {
            std::fs::read_to_string("/sys/class/tty/tty0/active").map(|s| s.trim().to_string()).unwrap_or_else(|_| name.to_string())
        } else {
            name.to_string()
        };
        out.push(std::path::Path::new("/dev").join(name));
    }
    out
}

/// (major, minor) of the terminals this process already writes to: its stdin/stdout/stderr
/// and its controlling terminal.
#[cfg(unix)]
fn own_ttys() -> Vec<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let mut own: Vec<(u64, u64)> = (0..3)
        .filter_map(|fd| std::fs::metadata(format!("/proc/self/fd/{fd}")).ok())
        .map(|m| split(m.rdev()))
        .filter(|d| *d != (0, 0))
        .collect();
    // field 7 of /proc/self/stat (after the parenthesized command name) is tty_nr
    if let Some(n) = std::fs::read_to_string("/proc/self/stat")
        .ok()
        .and_then(|s| s.rsplit_once(')').and_then(|(_, rest)| rest.split_whitespace().nth(4)?.parse::<u64>().ok()))
        .filter(|n| *n != 0)
    {
        own.push(((n >> 8) & 0xfff, (n & 0xff) | ((n >> 12) & 0xfff00)));
    }
    own
}

/// Linux dev_t → (major, minor).
fn split(dev: u64) -> (u64, u64) {
    (((dev >> 8) & 0xfff) | ((dev >> 32) & !0xfff), (dev & 0xff) | ((dev >> 12) & !0xff))
}

#[cfg(test)]
mod tests {
    #[test]
    fn dev_numbers() {
        // /dev/pts/3 is 136:3, /dev/ttyS0 4:64; large minors use the high bits
        assert_eq!(super::split((136 << 8) | 3), (136, 3));
        assert_eq!(super::split((4 << 8) | 64), (4, 64));
        let minor: u64 = 0x12345;
        assert_eq!(super::split((136 << 8) | (minor & 0xff) | ((minor & !0xff) << 12)), (136, minor));
    }
}
