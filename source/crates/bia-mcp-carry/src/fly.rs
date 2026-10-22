//! Land announces on UDP. Fly discovers those hosts, then both sides confirm a code
//! derived from an X25519 handshake before any archive bytes move.
//!
//! Over the internet (a server taken over in land mode, `bia-flash takeover --land`)
//! nobody sits at the landing machine and there is no shared LAN: the flyer connects
//! to its address and both prove they know the land secret (`/etc/bia/land.secret`)
//! with an HMAC over the X25519 handshake instead of comparing a code (protocol
//! `BIA2`). The archive then travels encrypted (ChaCha20-Poly1305, keys from the
//! handshake and the secret). A machine waiting with a secret refuses `BIA1` flights.
use bia_mcp::Ctx;
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, CHACHA20_POLY1305};
use ring::agreement::{self, EphemeralPrivateKey, UnparsedPublicKey, X25519};
use ring::hmac;
use ring::rand::SystemRandom;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream, ToSocketAddrs, UdpSocket};
use std::path::PathBuf;
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

pub const TCP_PORT: u16 = 4791;
const DISC_PORT: u16 = 4791;
// On the root disk: /run is a RAM tmpfs, usually smaller than /home/bia.
const ARCHIVE_OUT: &str = "/var/tmp/bia-carry-out.tar.gz";
const ARCHIVE_IN: &str = "/var/tmp/bia-carry-in.tar.gz";
/// The land secret of a machine that waits for a flight over the internet.
const LAND_SECRET: &str = "/etc/bia/land.secret";
const FRAME: usize = 64 * 1024;
/// Letters and digits a land secret needs at least (26 from 32 symbols: 130 bits).
pub const MIN_SECRET: usize = 26;
const SECRET_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

/// A new land secret: 28 symbols without look-alikes (140 bits), in groups of 4.
pub fn new_secret() -> Result<String, String> {
    let mut b = [0u8; 28];
    ring::rand::SecureRandom::fill(&SystemRandom::new(), &mut b).map_err(|_| "no randomness".to_string())?;
    let s: Vec<char> = b.iter().map(|x| SECRET_ALPHABET[(*x as usize) % 32] as char).collect();
    Ok(s.chunks(4).map(|c| c.iter().collect::<String>()).collect::<Vec<_>>().join("-"))
}

fn land_secret_path() -> PathBuf {
    std::env::var_os("BIA_LAND_SECRET_FILE").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(LAND_SECRET))
}

/// Letters and digits only, upper case: `abcd-efgh` and `ABCDEFGH` are the same secret.
pub fn normalize_secret(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_uppercase()).collect()
}

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
    if let Ok(raw) = std::fs::read_to_string(land_secret_path()) {
        let secret = normalize_secret(&raw);
        if secret.len() >= MIN_SECRET {
            return land_with_secret(ctx, port, timeout, &secret);
        }
        ctx.log(format!("land: the land secret has fewer than {MIN_SECRET} letters and digits; ignoring it and waiting for a flight on this network"));
    }
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

/// Fold this agent and send it to a landing machine (on this network by default); with `secret` (the landing machine's land secret) the flight is `BIA2`:
/// mutual proof instead of a code, and the archive encrypted.
pub fn fly_with(ctx: &Ctx, host: Option<&str>, port: Option<u16>, timeout: Duration, secret: Option<&str>) -> Result<String, String> {
    let mut secret = secret.map(normalize_secret).filter(|s| !s.is_empty());
    if secret.is_none() {
        // a server this agent made a land secret for (carry_new_land_secret(host=…))
        if let Some(saved) = host.filter(|h| !h.trim().is_empty()).and_then(saved_secret) {
            ctx.log(format!("fly: using the land secret saved for {}", host_key(host.unwrap_or(""))));
            secret = Some(normalize_secret(&saved));
        }
    }
    if secret.as_ref().is_some_and(|s| s.len() < MIN_SECRET) {
        return Err(format!("a land secret has at least {MIN_SECRET} letters and digits; this one is shorter"));
    }
    if secret.is_some() && host.map(str::trim).unwrap_or("").is_empty() {
        return Err("a flight with a land secret needs host: the address of the machine waiting for it".into());
    }
    let _hold = acquire()?;
    // Fold before looking for the other machine: a fold that fails (no space) must
    // not leave it holding a confirmed code and a dropped connection.
    let archive = PathBuf::from(ARCHIVE_OUT);
    let len = crate::archive::fold_into(ctx, &archive)?;
    let result = match &secret {
        Some(sec) => fly_archive_secret(ctx, host.unwrap_or(""), port.unwrap_or(TCP_PORT), timeout, &archive, len, sec).map(|ok| {
            forget_secret(host.unwrap_or(""));
            ok
        }),
        None => fly_archive(ctx, host, port, timeout, &archive, len),
    };
    let _ = std::fs::remove_file(&archive);
    result
}

fn fly_archive(ctx: &Ctx, host: Option<&str>, port: Option<u16>, timeout: Duration, archive: &std::path::Path, len: u64) -> Result<String, String> {
    let (ip, port) = match host.map(str::trim).filter(|s| !s.is_empty()) {
        Some(h) => {
            let addr = resolve(h, port.unwrap_or(TCP_PORT))?;
            (addr.ip(), addr.port())
        }
        None => {
            ctx.log("fly: looking for a machine that is landing");
            let hosts = discover(ctx, Duration::from_secs(3));
            match hosts.len() {
                0 => return Err("no machine is landing on this network; for a server on the internet pass host (its address) and secret (its land secret)".into()),
                1 => {
                    ctx.log(format!("fly: found {}", hosts[0].label()));
                    (std::net::IpAddr::V4(hosts[0].ip), if port.is_some() { port.unwrap() } else { hosts[0].port })
                }
                _ => {
                    let list = hosts.iter().map(|h| h.label()).collect::<Vec<_>>().join("\n");
                    return Ok(format!("several machines are landing; pass host to fly:\n{list}"));
                }
            }
        }
    };
    ctx.log(format!("fly: connecting to {ip}:{port}"));
    let mut stream = connect_cancel(ctx, SocketAddr::new(ip, port), timeout)?;
    prepare_stream(&stream)?;
    ctx.log(format!("fly: connected to {ip}:{port}"));
    let code = handshake(ctx, &mut stream, true).map_err(|e| {
        if e.contains("not a carry session") {
            format!("{e}: that machine waits for a flight with its land secret; pass secret")
        } else {
            e
        }
    })?;
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

fn connect_cancel(ctx: &Ctx, addr: SocketAddr, timeout: Duration) -> Result<TcpStream, String> {
    let start = Instant::now();
    loop {
        ctx.check_cancel()?;
        if start.elapsed() > timeout {
            return Err(format!("timed out connecting to {addr}"));
        }
        match TcpStream::connect_timeout(&addr, Duration::from_secs(5)) {
            Ok(s) => return Ok(s),
            Err(_) if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(200)),
            Err(e) => return Err(format!("connect {addr}: {e}")),
        }
    }
}

/// `host` is an IPv4 or IPv6 address (brackets optional) or a name.
fn resolve(host: &str, port: u16) -> Result<SocketAddr, String> {
    let (h, port) = split_host_port(host, port);
    let h = h.as_str();
    if let Ok(ip) = h.parse::<std::net::IpAddr>() {
        return Ok(SocketAddr::new(ip, port));
    }
    (h, port)
        .to_socket_addrs()
        .map_err(|e| format!("cannot resolve {h}: {e}"))?
        .next()
        .ok_or_else(|| format!("{h} has no address"))
}

/// `host` may carry its port: `203.0.113.7:14791`, `[2001:db8::1]:14791`, `name:14791`
/// (a bare IPv6 address has several colons and no port). Brackets are dropped.
fn split_host_port(host: &str, port: u16) -> (String, u16) {
    let h = host.trim();
    if let Some(rest) = h.strip_prefix('[') {
        if let Some((ip, tail)) = rest.split_once(']') {
            let p = tail.strip_prefix(':').and_then(|p| p.parse().ok()).unwrap_or(port);
            return (ip.to_string(), p);
        }
    }
    if h.matches(':').count() == 1 {
        if let Some((name, p)) = h.rsplit_once(':') {
            if let Ok(p) = p.parse() {
                return (name.to_string(), p);
            }
        }
    }
    (h.to_string(), port)
}

// ---- land secrets this agent made, by host (carry_new_land_secret(host=…)) -------

fn secrets_path() -> PathBuf {
    std::env::var_os("BIA_LAND_SECRETS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/home/bia/files/state/land-secrets.json"))
}

/// Root writes it here, but backups made as user bia must read it: hand it to the owner of
/// `files` (the agent gives all of /home/bia to bia at boot anyway).
fn own_like_files(p: &std::path::Path, files: &std::path::Path) {
    use std::os::unix::fs::MetadataExt;
    if let Ok(m) = std::fs::metadata(files) {
        let _ = std::os::unix::fs::chown(p, Some(m.uid()), Some(m.gid()));
    }
}

/// The host part only (no port, lower case): how secrets are filed.
fn host_key(host: &str) -> String {
    split_host_port(host, 0).0.to_ascii_lowercase()
}

/// A new land secret, filed under `host` when given, so carry_fly(host) finds it later.
pub fn new_secret_for(host: Option<&str>) -> Result<String, String> {
    let s = new_secret()?;
    if let Some(h) = host.map(str::trim).filter(|h| !h.is_empty()) {
        let p = secrets_path();
        let mut all: serde_json::Map<String, serde_json::Value> = std::fs::read(&p)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        all.insert(host_key(h), serde_json::json!({"secret": s, "created": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)}));
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&all).unwrap_or_default()).map_err(|e| format!("save the land secret: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
            if let Some(files) = p.parent().and_then(std::path::Path::parent) {
                own_like_files(&tmp, files);
            }
        }
        std::fs::rename(&tmp, &p).map_err(|e| format!("save the land secret: {e}"))?;
        let k = host_key(h);
        return Ok(format!("{s}\n(saved for {k}: carry_fly(host='{k}') uses it without secret)\n{}", land_steps(&k, &s)));
    }
    Ok(s)
}

/// What to do with a new land secret: take the server (`h`, no port) over over ssh, then fly there.
fn land_steps(h: &str, secret: &str) -> String {
    format!(
        "Next (only if root agreed: it erases that server):\n\
         1. bash: ssh root@{h} 'hostname; cat /etc/os-release; findmnt -no SOURCE /; lsblk -d -o NAME,SIZE,TYPE' — check it is that server, not this computer, and note the disk under / (e.g. /dev/vda).\n\
         2. bash, the WHOLE remote command in single quotes (else the pipe runs here and erases this computer): ssh root@{h} 'curl -fsSL https://github.com/zond80/bia/releases/latest/download/takeover.sh | bash -s -- --land --land-secret {secret} --yes --wipe <disk> --reboot' (sudo bash if the login is not root). It checks everything first and changes nothing when a check fails; read its output. \"this machine already runs bia\" means it reached a bia: fix the quoting or the address, never work around it.\n\
         3. carry_fly(host='{h}', timeout_secs=1800): it retries while the server installs (a few minutes; TCP 4791 must be open in the provider's firewall)."
    )
}

/// Drop the secret filed for `host` (the server deletes its copy once an agent landed).
fn forget_secret(host: &str) {
    let p = secrets_path();
    let Ok(b) = std::fs::read(&p) else { return };
    let Ok(mut all) = serde_json::from_slice::<serde_json::Map<String, serde_json::Value>>(&b) else { return };
    if all.remove(&host_key(host)).is_some() {
        let _ = std::fs::write(&p, serde_json::to_vec_pretty(&all).unwrap_or_default());
    }
}

/// The secret filed for `host`, if this agent made one for it.
fn saved_secret(host: &str) -> Option<String> {
    let all: serde_json::Value = serde_json::from_slice(&std::fs::read(secrets_path()).ok()?).ok()?;
    all.get(host_key(host))?.get("secret")?.as_str().map(str::to_string)
}

// ---- BIA2: land secret, mutual proof, encrypted archive ------------------------

/// Both directions of an encrypted flight.
struct Secure {
    tx: LessSafeKey,
    rx: LessSafeKey,
    n_tx: u64,
    n_rx: u64,
}

fn derive(k: &[u8], label: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(k);
    h.update(label);
    h.finalize().into()
}

fn nonce(n: u64) -> Nonce {
    let mut b = [0u8; 12];
    b[4..].copy_from_slice(&n.to_be_bytes());
    Nonce::assume_unique_for_key(b)
}

impl Secure {
    fn send(&mut self, stream: &mut TcpStream, data: &[u8]) -> Result<(), String> {
        let mut buf = data.to_vec();
        self.tx
            .seal_in_place_append_tag(nonce(self.n_tx), Aad::empty(), &mut buf)
            .map_err(|_| "encrypt".to_string())?;
        self.n_tx += 1;
        stream.write_all(&(buf.len() as u32).to_be_bytes()).map_err(|e| e.to_string())?;
        stream.write_all(&buf).map_err(|e| e.to_string())
    }

    fn recv(&mut self, ctx: &Ctx, stream: &TcpStream, timeout: Duration) -> Result<Vec<u8>, String> {
        let mut len = [0u8; 4];
        read_full(ctx, stream, &mut len, timeout)?;
        let len = u32::from_be_bytes(len) as usize;
        if len < 16 || len > FRAME + 64 {
            return Err("bad frame from the peer".into());
        }
        let mut buf = vec![0u8; len];
        read_full(ctx, stream, &mut buf, timeout)?;
        let plain = self
            .rx
            .open_in_place(nonce(self.n_rx), Aad::empty(), &mut buf)
            .map_err(|_| "the data from the peer did not decrypt (tampered with?)".to_string())?
            .len();
        self.n_rx += 1;
        buf.truncate(plain);
        Ok(buf)
    }
}

/// X25519 with `BIA2`, then each side proves it knows `secret`. Returns the channel.
fn handshake_secret(ctx: &Ctx, stream: &mut TcpStream, fly_side: bool, secret: &str) -> Result<Secure, String> {
    let rng = SystemRandom::new();
    let mine = EphemeralPrivateKey::generate(&X25519, &rng).map_err(|_| "x25519".to_string())?;
    let mine_pub = mine.compute_public_key().map_err(|_| "x25519".to_string())?.as_ref().to_vec();
    let mut hello = Vec::with_capacity(36);
    hello.extend_from_slice(b"BIA2");
    hello.extend_from_slice(&mine_pub);
    let mut peer = [0u8; 36];
    if fly_side {
        stream.write_all(&hello).map_err(|e| e.to_string())?;
        read_full(ctx, stream, &mut peer, Duration::from_secs(60))?;
    } else {
        read_full(ctx, stream, &mut peer, Duration::from_secs(60))?;
        if &peer[..4] == b"BIA1" {
            return Err("a flight without this machine's land secret (an older bia, or no secret given); refused".into());
        }
        stream.write_all(&hello).map_err(|e| e.to_string())?;
    }
    if &peer[..4] != b"BIA2" {
        return Err(if fly_side {
            "that machine does not wait with a land secret; fly without secret on its network instead".into()
        } else {
            "peer is not a carry session".into()
        });
    }
    let peer_pub = peer[4..].to_vec();
    let shared = agreement::agree_ephemeral(mine, &UnparsedPublicKey::new(&X25519, &peer_pub), |km| km.to_vec())
        .map_err(|_| "key agreement failed".to_string())?;
    let (fly_pub, land_pub) = if fly_side { (&mine_pub, &peer_pub) } else { (&peer_pub, &mine_pub) };
    let mut h = Sha256::new();
    h.update(b"bia-carry-v2");
    h.update(&shared);
    h.update(fly_pub);
    h.update(land_pub);
    h.update(secret.as_bytes());
    let k: [u8; 32] = h.finalize().into();
    let mac = hmac::Key::new(hmac::HMAC_SHA256, &k);
    let (mine_label, peer_label): (&[u8], &[u8]) = if fly_side { (b"proof fly", b"proof land") } else { (b"proof land", b"proof fly") };
    let mut theirs = [0u8; 32];
    if fly_side {
        stream.write_all(hmac::sign(&mac, mine_label).as_ref()).map_err(|e| e.to_string())?;
        read_full(ctx, stream, &mut theirs, Duration::from_secs(60)).map_err(|_| "the landing machine refused the land secret".to_string())?;
    } else {
        read_full(ctx, stream, &mut theirs, Duration::from_secs(60))?;
        if hmac::verify(&mac, peer_label, &theirs).is_err() {
            return Err("the flyer does not know this machine's land secret; refused".into());
        }
        stream.write_all(hmac::sign(&mac, mine_label).as_ref()).map_err(|e| e.to_string())?;
    }
    if fly_side && hmac::verify(&mac, peer_label, &theirs).is_err() {
        return Err("the landing machine does not know the land secret; not sending".into());
    }
    let key = |label: &[u8]| LessSafeKey::new(UnboundKey::new(&CHACHA20_POLY1305, &derive(&k, label)).expect("32-byte key"));
    let (tx, rx) = if fly_side { (key(b"fly to land"), key(b"land to fly")) } else { (key(b"land to fly"), key(b"fly to land")) };
    Ok(Secure { tx, rx, n_tx: 0, n_rx: 0 })
}

fn send_archive_secret(ctx: &Ctx, stream: &mut TcpStream, ch: &mut Secure, path: &std::path::Path, len: u64) -> Result<(), String> {
    ch.send(stream, &len.to_be_bytes())?;
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let (mut left, mut sent) = (len, 0u64);
    let mut buf = vec![0u8; FRAME];
    let mut last = Instant::now();
    while left > 0 {
        ctx.check_cancel()?;
        let n = buf.len().min(left as usize);
        file.read_exact(&mut buf[..n]).map_err(|e| e.to_string())?;
        ch.send(stream, &buf[..n])?;
        hasher.update(&buf[..n]);
        left -= n as u64;
        sent += n as u64;
        if last.elapsed() >= Duration::from_secs(1) {
            ctx.log(format!("fly: sent {sent} / {len} bytes"));
            last = Instant::now();
        }
    }
    ch.send(stream, &hasher.finalize())?;
    ctx.log(format!("fly: sent {len} bytes (encrypted)"));
    Ok(())
}

fn recv_archive_secret(ctx: &Ctx, stream: &TcpStream, ch: &mut Secure, path: &std::path::Path) -> Result<(), String> {
    let first = ch.recv(ctx, stream, Duration::from_secs(120))?;
    let len = u64::from_be_bytes(first.as_slice().try_into().map_err(|_| "bad archive header".to_string())?);
    if len == 0 {
        return Err("archive is empty".into());
    }
    let mut file = File::create(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let (mut got, mut last) = (0u64, Instant::now());
    while got < len {
        ctx.check_cancel()?;
        let chunk = ch.recv(ctx, stream, Duration::from_secs(120))?;
        if got + chunk.len() as u64 > len {
            return Err("the archive is longer than announced".into());
        }
        file.write_all(&chunk).map_err(|e| e.to_string())?;
        hasher.update(&chunk);
        got += chunk.len() as u64;
        if last.elapsed() >= Duration::from_secs(1) {
            ctx.log(format!("land: received {got} / {len} bytes"));
            last = Instant::now();
        }
    }
    let dig = ch.recv(ctx, stream, Duration::from_secs(60))?;
    if dig[..] != hasher.finalize()[..] {
        let _ = std::fs::remove_file(path);
        return Err("archive checksum did not match".into());
    }
    file.flush().map_err(|e| e.to_string())?;
    ctx.log(format!("land: received {len} bytes"));
    Ok(())
}

/// The landing side of one encrypted flight: proof, archive into `path`, ack.
fn land_session_secret(ctx: &Ctx, stream: &mut TcpStream, secret: &str, path: &std::path::Path) -> Result<(), String> {
    let mut ch = handshake_secret(ctx, stream, false, secret)?;
    ctx.log("land: the flyer knows the land secret; receiving its archive");
    recv_archive_secret(ctx, stream, &mut ch, path)?;
    ch.send(stream, &[1u8])
}

/// The flying side of one encrypted flight.
fn fly_session_secret(ctx: &Ctx, stream: &mut TcpStream, secret: &str, archive: &std::path::Path, len: u64) -> Result<(), String> {
    let mut ch = handshake_secret(ctx, stream, true, secret)?;
    ctx.log("fly: the landing machine knows the land secret");
    send_archive_secret(ctx, stream, &mut ch, archive, len)?;
    let ack = ch.recv(ctx, stream, Duration::from_secs(300))?;
    if ack != [1u8] {
        return Err("the landing machine rejected the archive".into());
    }
    Ok(())
}

fn fly_archive_secret(ctx: &Ctx, host: &str, port: u16, timeout: Duration, archive: &std::path::Path, len: u64, secret: &str) -> Result<String, String> {
    let addr = resolve(host, port)?;
    ctx.log(format!("fly: connecting to {addr} (land secret, encrypted)"));
    let mut stream = connect_cancel(ctx, addr, timeout)?;
    prepare_stream(&stream)?;
    fly_session_secret(ctx, &mut stream, secret, archive, len)?;
    Ok(format!("archive accepted by {addr}. That machine unfolds and reboots as this agent. This machine stays as it is."))
}

/// Wait for a flight that proves it knows `secret`; anything else is logged and
/// dropped, and the machine keeps waiting.
fn land_with_secret(ctx: &Ctx, port: u16, timeout: Duration, secret: &str) -> Result<String, String> {
    let listener = TcpListener::bind(("::", port))
        .or_else(|_| TcpListener::bind(("0.0.0.0", port)))
        .map_err(|e| format!("tcp {port}: {e}"))?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    ctx.log(format!("land: waiting on TCP {port} for a flight that knows this machine's land secret"));
    let start = Instant::now();
    let path = PathBuf::from(ARCHIVE_IN);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut failures = 0u32;
    loop {
        ctx.check_cancel()?;
        if start.elapsed() > timeout {
            return Err("timed out waiting for a machine to fly here".into());
        }
        let (mut stream, addr) = match listener.accept() {
            Ok(pair) => pair,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(200));
                continue;
            }
            Err(e) => return Err(e.to_string()),
        };
        ctx.log(format!("land: flyer connected from {addr}"));
        if prepare_stream(&stream).is_err() {
            continue;
        }
        match land_session_secret(ctx, &mut stream, secret, &path) {
            Ok(()) => break,
            Err(e) => {
                failures += 1;
                ctx.log(format!("land: {e}"));
                let _ = std::fs::remove_file(&path);
                // slow down guessing; the secret is long enough that it cannot work anyway
                std::thread::sleep(Duration::from_secs(failures.min(30) as u64));
            }
        }
    }
    // The secret is only for land mode: once an agent has landed it is gone. (While the
    // server still waits, updates keep it: the rescue leaves /etc/bia alone.)
    let _ = std::fs::remove_file(land_secret_path());
    ctx.log("land: archive accepted, unfolding");
    crate::archive::unfold_file(ctx, &path, crate::archive::Mode::Land)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn pair() -> (TcpStream, TcpStream) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let a = TcpStream::connect(l.local_addr().unwrap()).unwrap();
        let (b, _) = l.accept().unwrap();
        prepare_stream(&a).unwrap();
        prepare_stream(&b).unwrap();
        (a, b)
    }

    fn tmp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("bia-fly-{name}-{}", std::process::id()))
    }

    #[test]
    fn secret_flight_moves_the_archive_encrypted() {
        let src = tmp("src");
        let data: Vec<u8> = (0..200_000u32).map(|i| (i * 7 % 251) as u8).collect();
        std::fs::write(&src, &data).unwrap();
        let dst = tmp("dst");
        let (mut fly_s, mut land_s) = pair();
        let d2 = dst.clone();
        let t = std::thread::spawn(move || land_session_secret(&Ctx::console(), &mut land_s, "ABCDEFGHJKLMNPQRSTUVWXYZ2345", &d2));
        fly_session_secret(&Ctx::console(), &mut fly_s, &normalize_secret("abcd-efgh-jklm-npqr-stuv-wxyz-2345"), &src, data.len() as u64).unwrap();
        t.join().unwrap().unwrap();
        assert_eq!(std::fs::read(&dst).unwrap(), data);
        let _ = std::fs::remove_file(&src);
        let _ = std::fs::remove_file(&dst);
    }

    #[test]
    fn wrong_secret_or_old_flyer_is_refused() {
        let src = tmp("src2");
        std::fs::write(&src, b"agent").unwrap();
        let dst = tmp("dst2");
        let (mut fly_s, mut land_s) = pair();
        let d2 = dst.clone();
        let t = std::thread::spawn(move || land_session_secret(&Ctx::console(), &mut land_s, "RIGHTSECRET123RIGHTSECRET12", &d2));
        assert!(fly_session_secret(&Ctx::console(), &mut fly_s, "WRONGSECRET123WRONGSECRET12", &src, 5).is_err());
        assert!(t.join().unwrap().unwrap_err().contains("does not know"));
        assert!(!dst.exists(), "nothing received");
        // a BIA1 (code) flyer
        let (mut fly_s, mut land_s) = pair();
        let d3 = dst.clone();
        let t = std::thread::spawn(move || land_session_secret(&Ctx::console(), &mut land_s, "RIGHTSECRET123RIGHTSECRET12", &d3));
        let _ = handshake(&Ctx::console(), &mut fly_s, true);
        assert!(t.join().unwrap().unwrap_err().contains("without this machine's land secret"));
        let _ = std::fs::remove_file(&src);
    }

    #[test]
    fn new_secrets_are_long_and_readable() {
        let a = new_secret().unwrap();
        assert_eq!(a.len(), 34, "{a}");
        assert_eq!(normalize_secret(&a).len(), 28);
        assert!(normalize_secret(&a).len() >= MIN_SECRET);
        assert!(!a.contains(['0', '1', 'I', 'O']));
        assert_ne!(a, new_secret().unwrap());
    }

    #[test]
    fn host_with_port_and_saved_secrets() {
        assert_eq!(split_host_port("10.0.2.2:14791", 4791), ("10.0.2.2".to_string(), 14791));
        assert_eq!(split_host_port("[2001:db8::1]:14791", 4791), ("2001:db8::1".to_string(), 14791));
        assert_eq!(split_host_port("2001:db8::1", 4791), ("2001:db8::1".to_string(), 4791));
        assert_eq!(split_host_port("vps.example.com:99", 4791), ("vps.example.com".to_string(), 99));
        assert_eq!(resolve("10.0.2.2:14791", 4791).unwrap().to_string(), "10.0.2.2:14791");
        // files/state/land-secrets.json, files owned by user bia (uid 1000 here) while this runs as root
        let files = tmp("own-files");
        std::fs::create_dir_all(files.join("state")).unwrap();
        let root = unsafe { libc::geteuid() } == 0;
        if root {
            std::os::unix::fs::chown(&files, Some(1000), Some(1000)).unwrap();
        }
        let f = files.join("state").join("land-secrets.json");
        std::env::set_var("BIA_LAND_SECRETS", &f);
        let out = new_secret_for(Some("VPS.example.com:14791")).unwrap();
        let s = out.lines().next().unwrap().to_string();
        {
            use std::os::unix::fs::MetadataExt;
            let m = std::fs::metadata(&f).unwrap();
            assert_eq!(m.mode() & 0o777, 0o600);
            if root {
                assert_eq!(m.uid(), 1000, "backups made as user bia can read it");
            }
        }
        assert_eq!(saved_secret("vps.example.com").as_deref(), Some(s.as_str()));
        assert_eq!(saved_secret("vps.example.com:4791").as_deref(), Some(s.as_str()), "the port does not matter");
        assert!(saved_secret("other").is_none());
        forget_secret("vps.example.com");
        assert!(saved_secret("vps.example.com").is_none(), "forgotten after the landing");
        let _ = std::fs::remove_dir_all(files);
    }

    #[test]
    fn hosts_resolve() {
        assert_eq!(resolve("203.0.113.7", 4791).unwrap().to_string(), "203.0.113.7:4791");
        assert_eq!(resolve("[2001:db8::1]", 4791).unwrap().to_string(), "[2001:db8::1]:4791");
        assert_eq!(resolve("localhost", 1).unwrap().port(), 1);
        assert_eq!(normalize_secret(" abcd-EFGH 12 "), "ABCDEFGH12");
    }
}
