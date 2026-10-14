//! Home directory, memory, swap, network checks, reboot. Used when running as the machine service.
use anyhow::{bail, Context, Result};
use std::ffi::CString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Running as the machine service (bia.service), not `bia-agent --volume` on a host.
pub fn is_system() -> bool {
    std::env::var_os("BIA_SYSTEM").is_some() || Path::new("/etc/bia-system").exists()
}

pub fn kernel_version() -> String {
    fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "unknown".into())
}

pub struct MemInfo {
    pub total: u64,
    pub available: u64,
    /// page cache: file-backed pages, mostly the mmap'd model weights
    pub cached: u64,
    pub swap_total: u64,
    pub swap_free: u64,
}

pub fn meminfo() -> MemInfo {
    let mut m = MemInfo {
        total: 0,
        available: 0,
        cached: 0,
        swap_total: 0,
        swap_free: 0,
    };
    if let Ok(s) = fs::read_to_string("/proc/meminfo") {
        for l in s.lines() {
            let mut it = l.split_whitespace();
            let (Some(k), Some(v)) = (it.next(), it.next()) else {
                continue;
            };
            let v: u64 = v.parse::<u64>().unwrap_or(0) * 1024;
            match k {
                "MemTotal:" => m.total = v,
                "MemAvailable:" => m.available = v,
                "Cached:" => m.cached = v,
                "SwapTotal:" => m.swap_total = v,
                "SwapFree:" => m.swap_free = v,
                _ => {}
            }
        }
    }
    m
}

pub fn ncpu() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

/// `(idle, total)` jiffies from `/proc/stat` first `cpu ` line.
pub fn cpu_times() -> (u64, u64) {
    let s = fs::read_to_string("/proc/stat").unwrap_or_default();
    let line = s.lines().next().unwrap_or("");
    let mut it = line.split_whitespace();
    if it.next() != Some("cpu") {
        return (0, 0);
    }
    let mut v = [0u64; 10];
    for (i, p) in it.take(10).enumerate() {
        v[i] = p.parse().unwrap_or(0);
    }
    // user nice system idle iowait irq softirq steal guest guest_nice
    let idle = v[3].saturating_add(v[4]);
    let total: u64 = v.iter().sum();
    (idle, total)
}

pub fn loadavg() -> String {
    fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|s| {
            let mut p = s.split_whitespace();
            Some(format!("{} {} {}", p.next()?, p.next()?, p.next()?))
        })
        .unwrap_or_else(|| "? ? ?".into())
}

/// The default RLIMIT_MEMLOCK is 8 MiB; mlock of the GGUF needs more.
pub fn allow_mlock() {
    let inf = libc::rlimit {
        rlim_cur: libc::RLIM_INFINITY,
        rlim_max: libc::RLIM_INFINITY,
    };
    unsafe { libc::setrlimit(libc::RLIMIT_MEMLOCK, &inf) };
}

/// Enable transparent huge pages for the mmap'd weights.
pub fn tune_vm() -> Vec<String> {
    let mut applied = vec![];
    let settings = [
        ("/sys/kernel/mm/transparent_hugepage/enabled", "always"),
        ("/sys/kernel/mm/transparent_hugepage/defrag", "madvise"),
        ("/proc/sys/vm/swappiness", "100"),
        ("/proc/sys/vm/vfs_cache_pressure", "50"),
        ("/proc/sys/vm/overcommit_memory", "1"),
        ("/sys/module/zswap/parameters/enabled", "Y"),
    ];
    for (p, v) in settings {
        if fs::write(p, v).is_ok() {
            applied.push(format!("{}={}", p.rsplit('/').next().unwrap_or(p), v));
        }
    }
    applied
}

fn partitions() -> Vec<String> {
    let mut v = vec![];
    if let Ok(rd) = fs::read_dir("/sys/class/block") {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let p = e.path();
            if name.starts_with("loop") || name.starts_with("ram") || name.starts_with("zram") {
                continue;
            }
            // partitions, plus whole disks that carry no partition table (superfloppy media)
            let has_parts = fs::read_dir(&p)
                .map(|rd| {
                    rd.flatten()
                        .any(|c| c.file_name().to_string_lossy().starts_with(&name))
                })
                .unwrap_or(false);
            if p.join("partition").exists() || !has_parts {
                v.push(name);
            }
        }
    }
    v.sort();
    // partitions first
    v.sort_by_key(|n| {
        !Path::new("/sys/class/block")
            .join(n)
            .join("partition")
            .exists()
    });
    v
}

/// Agent state lives on the root filesystem under /home/bia. /boot is the EFI mount from fstab.
fn mount_source(target: &str) -> Option<String> {
    fs::read_to_string("/proc/mounts")
        .ok()?
        .lines()
        .find_map(|l| {
            let mut it = l.split_whitespace();
            let src = it.next()?;
            let dst = it.next()?;
            (dst == target).then(|| src.trim_start_matches("/dev/").to_string())
        })
}

/// Block device mounted at /boot (the ESP), if any.
pub fn boot_dev() -> Option<String> {
    mount_source("/boot")
}

/// Create `/home/bia/{model,files,MCPs}` on the root filesystem.
pub fn ensure_volume_dirs() -> Result<()> {
    fs::create_dir_all("/home/bia").ok();
    for d in ["model", "files", "MCPs"] {
        let p = format!("/home/bia/{d}");
        fs::create_dir_all(&p).with_context(|| format!("create {p}"))?;
    }
    let _ = std::process::Command::new("chown")
        .args(["-R", "bia:bia", "/home/bia"])
        .status();
    Ok(())
}

/// Create `/home/bia/files/state` and prove the home directory is writable so conversation, FMM and
/// network settings can be resumed after reboot from the same media.
pub fn ensure_volume_writable() -> Result<()> {
    let dir = Path::new("/home/bia/files/state");
    fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    let probe = dir.join(".writable");
    fs::write(&probe, b"ok").with_context(|| format!("write {}", probe.display()))?;
    let _ = fs::remove_file(&probe);
    unsafe { libc::sync() };
    Ok(())
}

/// Telegram and web drops land here. Empty dirs so the folders exist before the first file.
pub fn ensure_inbox(files: &Path) {
    for d in ["inbox/web", "inbox/telegram", "inbox/whatsapp"] {
        let p = files.join(d);
        if fs::create_dir_all(&p).is_ok() {
            let keep = p.join(".keep");
            if !keep.exists() {
                let _ = fs::write(&keep, b"");
            }
        }
    }
    unsafe { libc::sync() };
}

/// Activate any partition carrying a Linux swap signature.
pub fn activate_swap() -> Vec<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut on = vec![];
    for dev in partitions() {
        let path = format!("/dev/{dev}");
        let Ok(mut f) = fs::File::open(&path) else {
            continue;
        };
        let mut sig = [0u8; 10];
        if f.seek(SeekFrom::Start(4086)).is_ok()
            && f.read_exact(&mut sig).is_ok()
            && &sig == b"SWAPSPACE2"
        {
            if let Ok(c) = CString::new(path.clone()) {
                if unsafe { libc::swapon(c.as_ptr(), 0) } == 0 {
                    on.push(path);
                }
            }
        }
    }
    on
}

pub fn reboot(poweroff: bool) -> ! {
    unsafe {
        libc::sync();
    }
    let bin = if poweroff {
        "/sbin/poweroff"
    } else {
        "/sbin/reboot"
    };
    let _ = std::process::Command::new("systemctl")
        .arg(if poweroff { "poweroff" } else { "reboot" })
        .status();
    let _ = std::process::Command::new(bin).status();
    std::process::exit(0)
}

/// Where things are: /home/bia in system mode, subdirectories of --volume in dev mode.
#[derive(Clone, Debug)]
pub struct Paths {
    pub volume: PathBuf,
    pub model: PathBuf,
    pub files: PathBuf,
    pub mcps: PathBuf,
    pub boot: PathBuf,
}

impl Paths {
    pub fn system() -> Paths {
        Paths {
            volume: "/home/bia".into(),
            model: "/home/bia/model".into(),
            files: "/home/bia/files".into(),
            mcps: "/home/bia/MCPs".into(),
            boot: "/boot".into(),
        }
    }
    pub fn dev(volume: &Path) -> Paths {
        Paths {
            volume: volume.into(),
            model: volume.join("model"),
            files: volume.join("files"),
            mcps: volume.join("MCPs"),
            boot: volume.into(),
        }
    }
}

/// First *.gguf in the model dir (for split models, the -00001-of- part).
pub fn find_model(dir: &Path, wanted: &str) -> Result<PathBuf> {
    if wanted != "auto" {
        let p = dir.join(wanted);
        if p.exists() {
            return Ok(p);
        }
        bail!("model file {} not found in {}", wanted, dir.display());
    }
    let mut names: Vec<String> = fs::read_dir(dir)
        .with_context(|| format!("read {}", dir.display()))?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.to_lowercase().ends_with(".gguf") && !n.starts_with('.'))
        .filter(|n| !n.to_lowercase().starts_with("mmproj"))
        .collect();
    names.sort();
    let pick = names
        .iter()
        .find(|n| !n.contains("-of-") || n.contains("-00001-of-"))
        .or(names.first())
        .with_context(|| format!("no *.gguf in {}", dir.display()))?;
    Ok(dir.join(pick))
}

/// (interface, IPv4) pairs of configured interfaces, loopback excluded.
pub fn ipv4_addrs() -> Vec<(String, String)> {
    crate::listen::ipv4_addrs()
}

/// True when the kernel has a default IPv4 route (destination 0.0.0.0).
pub fn has_default_route() -> bool {
    let Ok(s) = std::fs::read_to_string("/proc/net/route") else {
        return false;
    };
    for line in s.lines().skip(1) {
        let mut cols = line.split_whitespace();
        let _iface = cols.next();
        let dest = cols.next().unwrap_or("");
        // Kernel hex; little-endian bytes. 00000000 = default.
        if dest == "00000000" {
            return true;
        }
    }
    false
}

/// True when resolv.conf lists at least one nameserver (not a comment).
pub fn has_nameserver() -> bool {
    let Ok(s) = std::fs::read_to_string("/etc/resolv.conf") else {
        return false;
    };
    s.lines().any(|l| {
        let t = l.trim_start();
        t.starts_with("nameserver")
            && t.split_whitespace().nth(1).is_some_and(|a| !a.is_empty())
    })
}

/// True when `host` resolves to at least one address (A/AAAA).
pub fn dns_resolves(host: &str) -> bool {
    use std::net::ToSocketAddrs;
    let host = host.trim().trim_matches(|c| c == '[' || c == ']');
    if host.is_empty() {
        return false;
    }
    (host, 443).to_socket_addrs().map(|mut i| i.next().is_some()).unwrap_or(false)
}

/// Hostname to probe for FMM DNS readiness (from base URL or provider default).
pub fn fmm_dns_host(provider: &str, base_url: &str) -> String {
    let b = base_url.trim();
    if let Some(rest) = b
        .strip_prefix("https://")
        .or_else(|| b.strip_prefix("http://"))
    {
        let host = rest
            .split(['/', '?', '#'])
            .next()
            .unwrap_or("")
            .split('@')
            .next_back()
            .unwrap_or("")
            .split(':')
            .next()
            .unwrap_or("")
            .trim_matches(|c| c == '[' || c == ']');
        if !host.is_empty() {
            return host.to_string();
        }
    }
    match provider.to_lowercase().as_str() {
        "openai" => "api.openai.com".into(),
        "openrouter" => "openrouter.ai".into(),
        "xai" => "api.x.ai".into(),
        "deepseek" => "api.deepseek.com".into(),
        "chatgpt" => "api.openai.com".into(),
        "qwen" => "dashscope-intl.aliyuncs.com".into(),
        "anthropic" | "claude" => "api.anthropic.com".into(),
        _ => "openrouter.ai".into(),
    }
}

/// Wait until a default route, DNS config, and (optionally) hostname resolution are ready.
/// Returns true if ready within `timeout`. Calls `tick` about once a second with a status line.
pub fn wait_network(
    timeout: Duration,
    dns_host: Option<&str>,
    mut tick: impl FnMut(&str),
) -> bool {
    let start = std::time::Instant::now();
    let mut last = String::new();
    loop {
        let route = has_default_route();
        let ns = has_nameserver();
        let dns_ok = match dns_host {
            Some(h) if route && ns => dns_resolves(h),
            Some(_) => false,
            None => route && ns,
        };
        let msg = if !route {
            "waiting for default route (dhcp)".to_string()
        } else if !ns {
            "waiting for DNS nameserver".to_string()
        } else if let Some(h) = dns_host {
            if dns_ok {
                format!("network ready (resolved {h})")
            } else {
                format!("waiting for DNS to resolve {h}")
            }
        } else {
            "network ready".to_string()
        };
        if msg != last {
            tick(&msg);
            last = msg.clone();
        }
        if route && ns && dns_host.map(|_| dns_ok).unwrap_or(true) {
            return true;
        }
        if start.elapsed() >= timeout {
            tick(&format!(
                "network not ready after {}s ({msg})",
                timeout.as_secs()
            ));
            return false;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}
