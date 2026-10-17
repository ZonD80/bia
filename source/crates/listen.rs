//! Who owns a listening TCP/UDP port. Shared by the agent (bind errors) and the web MCP.
//! Compiled into two binaries with different call sites; unused items in one crate are used in the other.
#![allow(dead_code)]
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub struct Sock {
    pub proto: &'static str,
    pub addr: String,
    pub port: u16,
    pub inode: u64,
}

pub struct Proc {
    pub pid: u32,
    pub ppid: u32,
    pub comm: String,
    pub argv: Vec<String>,
    pub env: Vec<(String, String)>,
    pub root: String,
    pub exe: String,
}

fn web_port(files: &Path) -> Option<u16> {
    let b = fs::read(files.join("state/web.json")).ok()?;
    let v: Value = serde_json::from_slice(&b).ok()?;
    v.get("port")
        .and_then(|x| x.as_u64())
        .filter(|n| *n > 0 && *n <= 65535)
        .map(|n| n as u16)
}

pub fn listening() -> Vec<Sock> {
    let mut out = vec![];
    parse_proc_net("/proc/net/tcp", false, true, &mut out);
    parse_proc_net("/proc/net/tcp6", true, true, &mut out);
    parse_proc_net("/proc/net/udp", false, false, &mut out);
    parse_proc_net("/proc/net/udp6", true, false, &mut out);
    out.sort_by_key(|s| (s.port, s.proto, s.addr.clone()));
    out.dedup_by(|a, b| {
        a.proto == b.proto && a.port == b.port && a.addr == b.addr && a.inode == b.inode
    });
    out
}

/// One line naming who is listening on `port`, or why we cannot tell.
pub fn describe_port(files: &Path, port: u16) -> String {
    let socks = listening();
    let on: Vec<&Sock> = socks.iter().filter(|s| s.port == port).collect();
    if on.is_empty() {
        return format!("nothing listening on {port} in this OS (if this is QEMU, the Mac port is not visible here)");
    }
    let map = inode_to_pid();
    let web = web_port(files);
    on.iter()
        .map(|s| format_sock(files, s, map.get(&s.inode).copied(), web))
        .collect::<Vec<_>>()
        .join("; ")
}

pub fn inode_to_pid() -> HashMap<u64, u32> {
    let mut m = HashMap::new();
    let Ok(rd) = fs::read_dir("/proc") else {
        return m;
    };
    for e in rd.flatten() {
        let name = e.file_name();
        let Some(pid) = name.to_str().and_then(|s| s.parse::<u32>().ok()) else {
            continue;
        };
        add_fd_inodes(&e.path().join("fd"), pid, &mut m);
        if let Ok(tasks) = fs::read_dir(e.path().join("task")) {
            for t in tasks.flatten() {
                add_fd_inodes(&t.path().join("fd"), pid, &mut m);
            }
        }
    }
    m
}

pub fn proc_of(pid: u32) -> Option<Proc> {
    let dir = PathBuf::from(format!("/proc/{pid}"));
    let comm = fs::read_to_string(dir.join("comm"))
        .ok()?
        .trim()
        .to_string();
    let argv = split0(&fs::read(dir.join("cmdline")).ok()?);
    if argv.is_empty() && comm.is_empty() {
        return None;
    }
    let env = split0(&fs::read(dir.join("environ")).unwrap_or_default())
        .into_iter()
        .filter_map(|s| {
            s.split_once('=')
                .map(|(k, v)| (k.to_string(), v.to_string()))
        })
        .collect();
    let root = fs::read_link(dir.join("root"))
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let exe = fs::read_link(dir.join("exe"))
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ppid = fs::read_to_string(dir.join("status"))
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("PPid:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|n| n.parse().ok())
        })
        .unwrap_or(0);
    Some(Proc {
        pid,
        ppid,
        comm,
        argv,
        env,
        root,
        exe,
    })
}

pub fn matching_mcp(_files: &Path, p: &Proc) -> Option<PathBuf> {
    let dirs = [PathBuf::from(
        std::env::var("BIA_MCPS").unwrap_or_else(|_| "/home/bia/MCPs".into()),
    )];
    for mcps in dirs {
        let Ok(rd) = fs::read_dir(&mcps) else {
            continue;
        };
        for e in rd.flatten() {
            let dir = e.path();
            let needle = dir.to_string_lossy();
            if (!p.exe.is_empty() && p.exe.contains(needle.as_ref()))
                || p.argv.iter().any(|a| a.contains(needle.as_ref()))
            {
                return Some(dir);
            }
        }
    }
    None
}

fn format_sock(files: &Path, s: &Sock, pid: Option<u32>, web: Option<u16>) -> String {
    let extra = if let Some(pid) = pid {
        let p = proc_of(pid);
        let comm = p
            .as_ref()
            .map(|p| p.comm.as_str())
            .filter(|c| !c.is_empty())
            .unwrap_or("?");
        let mut tags = vec![format!("pid={pid}"), comm.to_string()];
        if web == Some(s.port) {
            tags.push("web-console".into());
        }
        if let Some(p) = p.as_ref() {
            if let Some(m) = matching_mcp(files, p) {
                tags.push(format!(
                    "mcp={}",
                    m.file_name().unwrap_or_default().to_string_lossy()
                ));
            }
            if !p.exe.is_empty() {
                tags.push(format!("exe={}", p.exe));
            }
            let argv = short_argv(&p.argv);
            if !argv.is_empty() {
                tags.push(format!("argv={argv}"));
            }
        }
        tags.join(" ")
    } else {
        format!(
            "pid=? inode={} (no process fd; kernel, or a pid namespace this tool cannot see)",
            s.inode
        )
    };
    format!("{} {}:{}  {extra}", s.proto, s.addr, s.port)
}

fn short_argv(argv: &[String]) -> String {
    if argv.is_empty() {
        return String::new();
    }
    let s = argv.join(" ");
    const MAX: usize = 220;
    if s.chars().count() > MAX {
        format!("{}…", s.chars().take(MAX - 1).collect::<String>())
    } else {
        s
    }
}

fn add_fd_inodes(fd: &Path, pid: u32, m: &mut HashMap<u64, u32>) {
    let Ok(fds) = fs::read_dir(fd) else { return };
    for f in fds.flatten() {
        let Ok(t) = fs::read_link(f.path()) else {
            continue;
        };
        let s = t.to_string_lossy();
        if let Some(rest) = s.strip_prefix("socket:[") {
            if let Some(n) = rest.strip_suffix(']') {
                if let Ok(ino) = n.parse::<u64>() {
                    m.entry(ino).or_insert(pid);
                }
            }
        }
    }
}

fn parse_proc_net(path: &str, v6: bool, tcp: bool, out: &mut Vec<Sock>) {
    let Ok(text) = fs::read_to_string(path) else {
        return;
    };
    let proto = if tcp { "tcp" } else { "udp" };
    for (i, line) in text.lines().enumerate() {
        if i == 0 {
            continue;
        }
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 10 {
            continue;
        }
        if tcp && cols[3] != "0A" {
            continue;
        }
        let Some((ip_hex, port_hex)) = cols[1].split_once(':') else {
            continue;
        };
        let Ok(port) = u16::from_str_radix(port_hex, 16) else {
            continue;
        };
        if port == 0 {
            continue;
        }
        let Ok(inode) = cols[9].parse::<u64>() else {
            continue;
        };
        let addr = if v6 {
            parse_ip6(ip_hex)
        } else {
            parse_ip4(ip_hex)
        };
        out.push(Sock {
            proto,
            addr,
            port,
            inode,
        });
    }
}

fn parse_ip4(h: &str) -> String {
    let n = u32::from_str_radix(h, 16).unwrap_or(0);
    let b = n.to_le_bytes();
    if b == [0, 0, 0, 0] {
        "0.0.0.0".into()
    } else if b == [127, 0, 0, 1] {
        "127.0.0.1".into()
    } else {
        format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3])
    }
}

fn parse_ip6(h: &str) -> String {
    if h.chars().all(|c| c == '0') {
        return "[::]".into();
    }
    let mut bytes = [0u8; 16];
    for i in 0..4 {
        let chunk = u32::from_str_radix(h.get(i * 8..i * 8 + 8).unwrap_or("0"), 16).unwrap_or(0);
        bytes[i * 4..i * 4 + 4].copy_from_slice(&chunk.to_le_bytes());
    }
    if bytes[0..12] == [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff] {
        return format!("{}.{}.{}.{}", bytes[12], bytes[13], bytes[14], bytes[15]);
    }
    if bytes[15] == 1 && bytes[..15].iter().all(|b| *b == 0) {
        return "[::1]".into();
    }
    "[::]".into()
}

fn split0(b: &[u8]) -> Vec<String> {
    b.split(|c| *c == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect()
}

/// Non-loopback IPv4 addresses (iface, ip).
pub fn ipv4_addrs() -> Vec<(String, String)> {
    let mut out = vec![];
    unsafe {
        let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut ifap) != 0 {
            return out;
        }
        let mut p = ifap;
        while !p.is_null() {
            let a = &*p;
            if !a.ifa_addr.is_null() && (*a.ifa_addr).sa_family as i32 == libc::AF_INET {
                let sin = &*(a.ifa_addr as *const libc::sockaddr_in);
                let ip = std::net::Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr));
                let name = std::ffi::CStr::from_ptr(a.ifa_name)
                    .to_string_lossy()
                    .to_string();
                if !ip.is_loopback() {
                    out.push((name, ip.to_string()));
                }
            }
            p = a.ifa_next;
        }
        libc::freeifaddrs(ifap);
    }
    out
}

pub fn http_origins(port: u16) -> Vec<String> {
    https_origins(port)
}

/// HTTPS origins for the web console (no secrets in the URL).
pub fn https_origins(port: u16) -> Vec<String> {
    let ips = ipv4_addrs();
    let mut out = vec![];
    // QEMU user networking (10.0.2.x): the host reaches us through its forward on 127.0.0.1
    if ips.iter().any(|(_, ip)| ip.starts_with("10.0.2.")) {
        let p = if port == 443 { 8443 } else { port };
        out.push(format!("https://127.0.0.1:{p}"));
    }
    out.extend(
        ips.into_iter()
            .filter(|(_, ip)| !ip.starts_with("169.254.") && !ip.starts_with("10.0.2."))
            .map(|(_, ip)| {
                if port == 443 {
                    format!("https://{ip}")
                } else {
                    format!("https://{ip}:{port}")
                }
            }),
    );
    out
}

/// Public CA download URLs (no auth).
pub fn ca_urls(port: u16) -> Vec<String> {
    https_origins(port)
        .into_iter()
        .map(|o| format!("{}/v1/pki/ca", o.trim_end_matches('/')))
        .collect()
}

pub fn pickup_urls(port: u16, token: &str) -> Vec<String> {
    if token.is_empty() {
        return vec![];
    }
    https_origins(port)
        .into_iter()
        .map(|o| format!("{o}/v1/pki/pickup/{token}"))
        .collect()
}

/// @deprecated name kept for MCP call sites; returns HTTPS console roots (no code).
pub fn login_urls(port: u16, _code: &str) -> Vec<String> {
    https_origins(port)
}
