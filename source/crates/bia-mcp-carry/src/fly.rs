//! Land announces on UDP. Fly discovers those hosts, then both sides confirm a code
//! derived from an X25519 handshake before any archive bytes move.
use bia_mcp::Ctx;
use ring::agreement::{self, EphemeralPrivateKey, UnparsedPublicKey, X25519};
use ring::rand::SystemRandom;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::path::PathBuf;
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

pub const TCP_PORT: u16 = 4791;
const DISC_PORT: u16 = 4791;
// On the root disk: /run is a RAM tmpfs, usually smaller than /home/bia.
const ARCHIVE_OUT: &str = "/var/tmp/bia-carry-out.tar.gz";
const ARCHIVE_IN: &str = "/var/tmp/bia-carry-in.tar.gz";

struct Gate {
    inner: Mutex<GateInner>,
    cv: Condvar,
}
struct GateInner {
    active: bool,
    code: Option<String>,
    decision: Option<bool>,
}

fn gate() -> &'static Gate {
    static G: OnceLock<Gate> = OnceLock::new();
    G.get_or_init(|| Gate {
        inner: Mutex::new(GateInner { active: false, code: None, decision: None }),
        cv: Condvar::new(),
    })
}

struct Hold;
impl Drop for Hold {
    fn drop(&mut self) {
        let g = gate();
        let mut inner = g.inner.lock().unwrap();
        inner.active = false;
        inner.code = None;
        inner.decision = None;
        g.cv.notify_all();
    }
}

fn acquire() -> Result<Hold, String> {
    let mut inner = gate().inner.lock().unwrap();
    if inner.active {
        return Err("a carry session is already running".into());
    }
    inner.active = true;
    inner.code = None;
    inner.decision = None;
    Ok(Hold)
}

pub fn confirm(code: &str) -> Result<String, String> {
    let g = gate();
    let mut inner = g.inner.lock().unwrap();
    if !inner.active || inner.code.is_none() {
        return Err("no carry session is showing a code yet".into());
    }
    if inner.decision.is_some() {
        return Err("this session already has a confirmation".into());
    }
    let expect = inner.code.clone().unwrap_or_default();
    let got = code.trim();
    if got != expect {
        inner.decision = Some(false);
        g.cv.notify_all();
        return Err("code does not match; session dropped".into());
    }
    inner.decision = Some(true);
    g.cv.notify_all();
    Ok("code matches".into())
}

fn wait_decision(ctx: &Ctx, timeout: Duration) -> Result<(), String> {
    let g = gate();
    let mut inner = g.inner.lock().unwrap();
    let start = Instant::now();
    while inner.decision.is_none() {
        if ctx.cancelled() {
            return Err("cancelled".into());
        }
        if !inner.active {
            return Err("session ended".into());
        }
        if start.elapsed() >= timeout {
            return Err("timed out waiting for the code".into());
        }
        let (next, _) = g.cv.wait_timeout(inner, Duration::from_millis(200)).unwrap();
        inner = next;
    }
    if inner.decision == Some(true) {
        Ok(())
    } else {
        Err("code does not match; session dropped".into())
    }
}

#[derive(Clone, Debug)]
pub struct LandHost {
    pub ip: Ipv4Addr,
    pub port: u16,
    pub name: String,
}

impl LandHost {
    pub fn label(&self) -> String {
        if self.name.is_empty() {
            format!("{}:{}", self.ip, self.port)
        } else {
            format!("{}:{} {}", self.ip, self.port, self.name)
        }
    }
}

pub fn discover(ctx: &Ctx, wait: Duration) -> Vec<LandHost> {
    let sock = match UdpSocket::bind("0.0.0.0:0") {
        Ok(s) => s,
        Err(e) => {
            ctx.log(format!("discover: {e}"));
            return vec![];
        }
    };
    let _ = sock.set_broadcast(true);
    let _ = sock.set_read_timeout(Some(Duration::from_millis(200)));
    let local = local_v4();
    let probe = b"BIA?";
    let send = |sock: &UdpSocket| {
        for dest in broadcasts() {
            let _ = sock.send_to(probe, dest);
        }
    };
    send(&sock);
    ctx.log("discover: probing UDP 4791 for machines that are landing");
    let mut found: BTreeMap<(Ipv4Addr, u16), LandHost> = BTreeMap::new();
    let start = Instant::now();
    let mut resent = false;
    while start.elapsed() < wait {
        if ctx.cancelled() {
            break;
        }
        if !resent && start.elapsed() >= Duration::from_millis(800) {
            send(&sock);
            resent = true;
        }
        let mut buf = [0u8; 128];
        match sock.recv_from(&mut buf) {
            Ok((n, addr)) => {
                let SocketAddr::V4(v4) = addr else { continue };
                if local.contains(v4.ip()) {
                    continue;
                }
                if let Some(host) = parse_beacon(&buf[..n], *v4.ip()) {
                    ctx.log(format!("discover: {}", host.label()));
                    found.insert((host.ip, host.port), host);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(_) => break,
        }
    }
    found.into_values().collect()
}

pub fn land(ctx: &Ctx, port: u16, timeout: Duration) -> Result<String, String> {
    let _hold = acquire()?;
    let listener = TcpListener::bind(("0.0.0.0", port)).map_err(|e| format!("tcp {port}: {e}"))?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let udp = UdpSocket::bind(("0.0.0.0", DISC_PORT)).map_err(|e| format!("udp {DISC_PORT}: {e}"))?;
    udp.set_nonblocking(true).map_err(|e| e.to_string())?;
    let _ = udp.set_broadcast(true);
    ctx.log(format!("land: listening on 0.0.0.0:{port}, announcing on UDP {DISC_PORT}"));
    let start = Instant::now();
    let mut last_beacon = Instant::now() - Duration::from_secs(2);
    let (mut stream, addr) = loop {
        ctx.check_cancel()?;
        if start.elapsed() > timeout {
            return Err("timed out waiting for a machine to fly here".into());
        }
        if last_beacon.elapsed() >= Duration::from_secs(1) {
            announce(&udp, port);
            last_beacon = Instant::now();
        }
        let mut buf = [0u8; 128];
        if let Ok((n, from)) = udp.recv_from(&mut buf) {
            if buf.get(..n.min(4)) == Some(b"BIA?") {
                let _ = udp.send_to(&beacon(port), from);
            }
        }
        match listener.accept() {
            Ok(pair) => break pair,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e.to_string()),
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    drop(listener);
    drop(udp);
    ctx.log(format!("land: flyer connected from {addr}"));
    prepare_stream(&stream)?;
    let code = handshake(ctx, &mut stream, false)?;
    publish_code(ctx, &code);
    let decision = wait_decision(ctx, Duration::from_secs(180));
    finish_confirm(ctx, &mut stream, decision)?;
    ctx.log("land: receiving archive");
    let path = PathBuf::from(ARCHIVE_IN);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    recv_archive(ctx, &mut stream, &path)?;
    let _ = stream.write_all(&[1u8]);
    ctx.log("land: archive accepted, unfolding");
    crate::archive::unfold_file(ctx, &path, crate::archive::Mode::Land)
}

pub fn fly(ctx: &Ctx, host: Option<&str>, port: Option<u16>, timeout: Duration) -> Result<String, String> {
    let _hold = acquire()?;
    // Fold before looking for the other machine: a fold that fails (no space) must
    // not leave it holding a confirmed code and a dropped connection.
    let archive = PathBuf::from(ARCHIVE_OUT);
    let len = crate::archive::fold_into(ctx, &archive)?;
    let result = fly_archive(ctx, host, port, timeout, &archive, len);
    let _ = std::fs::remove_file(&archive);
    result
}

fn fly_archive(ctx: &Ctx, host: Option<&str>, port: Option<u16>, timeout: Duration, archive: &std::path::Path, len: u64) -> Result<String, String> {
    let (ip, port) = match host.map(str::trim).filter(|s| !s.is_empty()) {
        Some(h) => {
            let ip: Ipv4Addr = h.parse().map_err(|_| format!("host must be an IPv4 address, got {h}"))?;
            (ip, port.unwrap_or(TCP_PORT))
        }
        None => {
            ctx.log("fly: looking for a machine that is landing");
            let hosts = discover(ctx, Duration::from_secs(3));
            match hosts.len() {
                0 => return Err("no machine is landing on this network".into()),
                1 => {
                    ctx.log(format!("fly: found {}", hosts[0].label()));
                    (hosts[0].ip, if port.is_some() { port.unwrap() } else { hosts[0].port })
                }
                _ => {
                    let list = hosts.iter().map(|h| h.label()).collect::<Vec<_>>().join("\n");
                    return Ok(format!("several machines are landing; pass host to fly:\n{list}"));
                }
            }
        }
    };
    ctx.log(format!("fly: connecting to {ip}:{port}"));
    let mut stream = connect_cancel(ctx, ip, port, timeout)?;
    prepare_stream(&stream)?;
    ctx.log(format!("fly: connected to {ip}:{port}"));
    let code = handshake(ctx, &mut stream, true)?;
    publish_code(ctx, &code);
    let decision = wait_decision(ctx, Duration::from_secs(180));
    finish_confirm(ctx, &mut stream, decision)?;
    ctx.log(format!("fly: sending {len} bytes"));
    send_archive(ctx, &mut stream, archive, len)?;
    let mut ack = [0u8; 1];
    read_full(ctx, &stream, &mut ack, Duration::from_secs(120))?;
    if ack[0] != 1 {
        return Err("the landing machine rejected the archive".into());
    }
    Ok(format!("archive accepted by {ip}:{port}. That machine unfolds and reboots. This machine stays as it is."))
}

fn publish_code(ctx: &Ctx, code: &str) {
    {
        let mut inner = gate().inner.lock().unwrap();
        inner.code = Some(code.to_string());
        inner.decision = None;
    }
    ctx.log(format!("CODE {code}"));
    if ctx.plain() {
        ctx.log("compare this code with the other machine. If both show the same digits, type them here and press enter.");
    } else {
        ctx.log("compare this code with the other machine. If both screens show the same digits, call carry_confirm with them.");
    }
}

fn finish_confirm(ctx: &Ctx, stream: &mut TcpStream, decision: Result<(), String>) -> Result<(), String> {
    let mine = if decision.is_ok() { 1u8 } else { 0u8 };
    let _ = stream.write_all(&[mine]);
    let mut peer = [0u8; 1];
    if read_full(ctx, stream, &mut peer, Duration::from_secs(180)).is_err() {
        return decision.and_then(|_| Err("the other machine closed before confirming".into()));
    }
    if mine != 1 {
        return decision;
    }
    if peer[0] != 1 {
        return Err("the other machine did not confirm the code".into());
    }
    ctx.log("both sides confirmed");
    Ok(())
}

fn handshake(ctx: &Ctx, stream: &mut TcpStream, fly_side: bool) -> Result<String, String> {
    let rng = SystemRandom::new();
    let mine = EphemeralPrivateKey::generate(&X25519, &rng).map_err(|_| "x25519".to_string())?;
    let public = mine.compute_public_key().map_err(|_| "x25519".to_string())?;
    let mine_pub = public.as_ref().to_vec();
    if mine_pub.len() != 32 {
        return Err("x25519 public key".into());
    }
    let mut hello = Vec::with_capacity(36);
    hello.extend_from_slice(b"BIA1");
    hello.extend_from_slice(&mine_pub);
    let mut peer = [0u8; 36];
    if fly_side {
        stream.write_all(&hello).map_err(|e| e.to_string())?;
        read_full(ctx, stream, &mut peer, Duration::from_secs(60))?;
    } else {
        read_full(ctx, stream, &mut peer, Duration::from_secs(60))?;
        stream.write_all(&hello).map_err(|e| e.to_string())?;
    }
    if &peer[..4] != b"BIA1" {
        return Err("peer is not a carry session".into());
    }
    let peer_pub = &peer[4..];
    let peer_key = UnparsedPublicKey::new(&X25519, peer_pub);
    let shared = agreement::agree_ephemeral(mine, &peer_key, |km| km.to_vec()).map_err(|_| "key agreement failed".to_string())?;
    Ok(pin_code(&shared, &mine_pub, peer_pub))
}

fn pin_code(shared: &[u8], a: &[u8], b: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(b"bia-carry-v1");
    h.update(shared);
    if a <= b {
        h.update(a);
        h.update(b);
    } else {
        h.update(b);
        h.update(a);
    }
    let d = h.finalize();
    let n = u32::from_be_bytes([d[0], d[1], d[2], d[3]]) % 1_000_000;
    format!("{n:06}")
}

fn send_archive(ctx: &Ctx, stream: &mut TcpStream, path: &std::path::Path, len: u64) -> Result<(), String> {
    stream.write_all(&len.to_be_bytes()).map_err(|e| e.to_string())?;
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut left = len;
    let mut buf = vec![0u8; 64 * 1024];
    let mut sent = 0u64;
    let mut last = Instant::now();
    while left > 0 {
        ctx.check_cancel()?;
        let n = buf.len().min(left as usize);
        file.read_exact(&mut buf[..n]).map_err(|e| e.to_string())?;
        stream.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        hasher.update(&buf[..n]);
        left -= n as u64;
        sent += n as u64;
        if last.elapsed() >= Duration::from_secs(1) {
            ctx.log(format!("fly: sent {sent} / {len} bytes"));
            last = Instant::now();
        }
    }
    let dig = hasher.finalize();
    stream.write_all(&dig).map_err(|e| e.to_string())?;
    ctx.log(format!("fly: sent {len} bytes"));
    Ok(())
}

fn recv_archive(ctx: &Ctx, stream: &mut TcpStream, path: &std::path::Path) -> Result<(), String> {
    let mut len_buf = [0u8; 8];
    read_full(ctx, stream, &mut len_buf, Duration::from_secs(120))?;
    let len = u64::from_be_bytes(len_buf);
    if len == 0 {
        return Err("archive is empty".into());
    }
    let mut file = File::create(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut left = len;
    let mut buf = vec![0u8; 64 * 1024];
    let mut got = 0u64;
    let mut last = Instant::now();
    while left > 0 {
        ctx.check_cancel()?;
        let n = buf.len().min(left as usize);
        read_full(ctx, stream, &mut buf[..n], Duration::from_secs(120))?;
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        hasher.update(&buf[..n]);
        left -= n as u64;
        got += n as u64;
        if last.elapsed() >= Duration::from_secs(1) {
            ctx.log(format!("land: received {got} / {len} bytes"));
            last = Instant::now();
        }
    }
    let mut peer_hash = [0u8; 32];
    read_full(ctx, stream, &mut peer_hash, Duration::from_secs(60))?;
    let dig = hasher.finalize();
    if dig[..] != peer_hash[..] {
        let _ = std::fs::remove_file(path);
        return Err("archive checksum did not match".into());
    }
    file.flush().map_err(|e| e.to_string())?;
    ctx.log(format!("land: received {len} bytes"));
    Ok(())
}

fn connect_cancel(ctx: &Ctx, ip: Ipv4Addr, port: u16, timeout: Duration) -> Result<TcpStream, String> {
    let start = Instant::now();
    let addr = std::net::SocketAddrV4::new(ip, port);
    loop {
        ctx.check_cancel()?;
        if start.elapsed() > timeout {
            return Err(format!("timed out connecting to {ip}:{port}"));
        }
        match TcpStream::connect_timeout(&std::net::SocketAddr::V4(addr), Duration::from_secs(2)) {
            Ok(s) => return Ok(s),
            Err(_) if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(200)),
            Err(e) => return Err(format!("connect {ip}:{port}: {e}")),
        }
    }
}

fn prepare_stream(stream: &TcpStream) -> Result<(), String> {
    stream.set_nodelay(true).ok();
    stream.set_read_timeout(Some(Duration::from_millis(250))).map_err(|e| e.to_string())?;
    stream.set_write_timeout(Some(Duration::from_secs(30))).map_err(|e| e.to_string())?;
    Ok(())
}

fn read_full(ctx: &Ctx, mut stream: &TcpStream, buf: &mut [u8], timeout: Duration) -> Result<(), String> {
    let mut off = 0;
    let start = Instant::now();
    while off < buf.len() {
        ctx.check_cancel()?;
        if start.elapsed() > timeout {
            return Err("timed out reading from the peer".into());
        }
        match stream.read(&mut buf[off..]) {
            Ok(0) => return Err("peer closed the connection".into()),
            Ok(n) => off += n,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}

fn hostname() -> String {
    let raw = std::fs::read_to_string("/etc/hostname").unwrap_or_else(|_| "bia".into());
    let name: String = raw.trim().chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(63).collect();
    if name.is_empty() { "bia".into() } else { name }
}

fn beacon(port: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity(70);
    out.extend_from_slice(b"BIAL");
    out.extend_from_slice(&port.to_be_bytes());
    out.extend_from_slice(hostname().as_bytes());
    out
}

fn announce(sock: &UdpSocket, port: u16) {
    let pkt = beacon(port);
    for dest in broadcasts() {
        let _ = sock.send_to(&pkt, dest);
    }
}

fn parse_beacon(buf: &[u8], ip: Ipv4Addr) -> Option<LandHost> {
    if buf.len() < 6 || &buf[..4] != b"BIAL" {
        return None;
    }
    let port = u16::from_be_bytes([buf[4], buf[5]]);
    if port == 0 {
        return None;
    }
    let name = String::from_utf8_lossy(&buf[6..]).chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(63).collect();
    Some(LandHost { ip, port, name })
}

fn broadcasts() -> Vec<std::net::SocketAddrV4> {
    let mut out = vec![std::net::SocketAddrV4::new(Ipv4Addr::BROADCAST, DISC_PORT)];
    for ip in iface_v4(true) {
        out.push(std::net::SocketAddrV4::new(ip, DISC_PORT));
    }
    out.sort();
    out.dedup();
    out
}

fn local_v4() -> Vec<Ipv4Addr> {
    iface_v4(false)
}

fn iface_v4(broadcast: bool) -> Vec<Ipv4Addr> {
    let mut out = Vec::new();
    unsafe {
        let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut ifap) != 0 {
            return out;
        }
        let mut p = ifap;
        while !p.is_null() {
            let ifa = &*p;
            let flags = ifa.ifa_flags as u32;
            let up = libc::IFF_UP as u32;
            let loopback = libc::IFF_LOOPBACK as u32;
            if flags & up != 0 && flags & loopback == 0 {
                let ptr = if broadcast { broad_ptr(ifa) } else { ifa.ifa_addr };
                if let Some(ip) = ipv4_of(ptr) {
                    if !ip.is_unspecified() && !ip.is_loopback() {
                        out.push(ip);
                    }
                }
            }
            p = ifa.ifa_next;
        }
        libc::freeifaddrs(ifap);
    }
    out.sort();
    out.dedup();
    out
}

#[cfg(target_os = "linux")]
fn broad_ptr(ifa: &libc::ifaddrs) -> *mut libc::sockaddr {
    if (ifa.ifa_flags as u32) & (libc::IFF_BROADCAST as u32) == 0 {
        std::ptr::null_mut()
    } else {
        ifa.ifa_ifu
    }
}

#[cfg(not(target_os = "linux"))]
fn broad_ptr(ifa: &libc::ifaddrs) -> *mut libc::sockaddr {
    if (ifa.ifa_flags as u32) & (libc::IFF_BROADCAST as u32) == 0 {
        std::ptr::null_mut()
    } else {
        ifa.ifa_dstaddr
    }
}

fn ipv4_of(sa: *const libc::sockaddr) -> Option<Ipv4Addr> {
    if sa.is_null() {
        return None;
    }
    unsafe {
        if (*sa).sa_family as i32 != libc::AF_INET as i32 {
            return None;
        }
        let sin = &*(sa as *const libc::sockaddr_in);
        Some(Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr)))
    }
}
