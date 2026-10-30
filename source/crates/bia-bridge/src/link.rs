//! Talking to the bia: the certificate request (CSR; the key stays here) and renewal over
//! HTTPS, and the web bus WebSocket (`/ws`, protocol 2) for everything in the chat. The
//! bia's CA is pinned; its server name is not checked (bias are reached by addresses that
//! change).
use crate::profile::{now_secs, sha256_hex, Profile};
use anyhow::{anyhow, bail, Context, Result};
use bia_pki::client::{self, ServerTrust};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};

#[allow(dead_code)]
#[path = "../../tlspipe.rs"]
pub mod tlspipe;
use tlspipe::SharedTls;

/// First contact: the bia's CA, checked against `pin` (sha256 hex) when given.
pub fn fetch_ca(url: &str, pin: Option<&str>) -> Result<(String, String)> {
    let r = client::request(url, "GET", "/v1/pki/ca", None, ServerTrust::Any, None).with_context(|| format!("reach {url}"))?;
    if r.status != 200 {
        bail!("{url}/v1/pki/ca: HTTP {} (is it a bia?)", r.status);
    }
    let pem = r.text();
    let ders = bia_pki::pem_to_cert_ders(&pem).context("the bia's CA")?;
    let sha = sha256_hex(ders[0].as_ref());
    if let Some(pin) = pin.map(|p| p.trim().to_ascii_lowercase().replace(':', "")).filter(|p| !p.is_empty()) {
        if pin != sha {
            bail!("{url} has CA fingerprint {sha}, not the expected {pin}; not connecting");
        }
    }
    Ok((pem, sha))
}

/// A new key and a certificate request at the bia. Returns the request id.
pub fn request_cert(p: &mut Profile, ca: &str) -> Result<()> {
    let (key, csr) = bia_pki::new_key_csr(&p.identity)?;
    std::fs::create_dir_all(p.dir())?;
    bia_pki::write_private(&p.path("key.pem"), key.as_bytes())?;
    std::fs::write(p.path("ca.pem"), ca)?;
    let body = json!({"identity": p.identity, "role": "agent", "note": p.note, "csr": csr});
    let r = client::request(&p.url, "POST", "/v1/pki/requests", Some(&body), ServerTrust::Ca(ca), None)?;
    if r.status != 202 && r.status != 200 {
        bail!("{} refused the request: HTTP {} {}", p.url, r.status, r.text().trim());
    }
    let v = r.json()?;
    p.request = v["id"].as_str().unwrap_or("").to_string();
    p.state = "pending".into();
    p.created = now_secs();
    p.save()
}

/// Check a pending request once; collects the certificate when approved. Returns the
/// seconds to wait before the next check.
pub fn poll_request(p: &mut Profile) -> Result<u64> {
    let ca = p.read("ca.pem")?;
    let r = client::request(&p.url, "GET", &format!("/v1/pki/requests/{}", p.request), None, ServerTrust::Ca(&ca), None)?;
    if r.status == 404 {
        p.state = "expired".into();
        p.save()?;
        return Ok(0);
    }
    let v = r.json()?;
    match v["status"].as_str().unwrap_or("") {
        "approved" | "issued" => {
            let c = client::request(&p.url, "GET", &format!("/v1/pki/requests/{}/cert", p.request), None, ServerTrust::Ca(&ca), None)?;
            if c.status != 200 {
                bail!("collect the certificate: HTTP {}", c.status);
            }
            std::fs::write(p.path("cert.pem"), c.text())?;
            p.state = "active".into();
            p.message = v["message"].as_str().unwrap_or("").to_string();
            p.save()?;
            Ok(0)
        }
        "denied" => {
            p.state = "denied".into();
            p.message = v["reason"].as_str().unwrap_or("").to_string();
            p.save()?;
            Ok(0)
        }
        "expired" => {
            p.state = "expired".into();
            p.save()?;
            Ok(0)
        }
        _ => Ok(v["retry_after"].as_u64().unwrap_or(15).clamp(5, 300)),
    }
}

/// Renew this client's certificate (new key + CSR) when it is still valid.
pub fn renew(p: &Profile) -> Result<()> {
    let (key, csr) = bia_pki::new_key_csr(&p.identity)?;
    let v = json!({"csr": csr});
    let (ca, cert, oldkey) = p.creds()?;
    let r = client::request(&p.url, "POST", "/v1/pki/renew", Some(&v), ServerTrust::Ca(&ca), Some((&cert, &oldkey)))?;
    if r.status != 200 {
        bail!("renew: HTTP {} {}", r.status, r.text().trim());
    }
    bia_pki::write_private(&p.path("key.pem.new"), key.as_bytes())?;
    std::fs::write(p.path("cert.pem.new"), r.text())?;
    std::fs::rename(p.path("key.pem.new"), p.path("key.pem"))?;
    std::fs::rename(p.path("cert.pem.new"), p.path("cert.pem"))?;
    Ok(())
}

/// This build's platform name, as the bia names its bridge builds.
pub fn my_platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(windows) {
        "windows-x86_64"
    } else if cfg!(target_arch = "aarch64") {
        "linux-arm64"
    } else {
        "linux-x86_64"
    }
}

/// The bia's list of bridge builds (`GET /v1/bridge`; public, the CA pinned).
pub fn bridge_list(p: &Profile) -> Result<Value> {
    let ca = p.read("ca.pem")?;
    let r = client::request(&p.url, "GET", "/v1/bridge", None, ServerTrust::Ca(&ca), None)?;
    if r.status == 404 {
        bail!("{} is an older bia that does not serve bia-bridge", p.url);
    }
    if r.status != 200 {
        bail!("GET /v1/bridge: HTTP {}", r.status);
    }
    r.json()
}

/// One bridge build from the bia, checked against the sha256 it lists.
pub fn bridge_download(p: &Profile, file: &str, sha: &str) -> Result<Vec<u8>> {
    let ca = p.read("ca.pem")?;
    let r = client::request(&p.url, "GET", &format!("/bridge/{}", urlenc(file)), None, ServerTrust::Ca(&ca), None)?;
    if r.status != 200 {
        bail!("GET /bridge/{file}: HTTP {}", r.status);
    }
    if sha256_hex(&r.body) != sha {
        bail!("{file} from the bia does not match its sha256; not using it");
    }
    Ok(r.body)
}

pub fn urlenc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

// ---- the web bus WebSocket ----

pub struct Ws {
    pub tls: SharedTls,
    pub reader: BufReader<SharedTls>,
}

pub fn ws_connect(p: &Profile) -> Result<Ws> {
    let (ca, cert, key) = p.creds()?;
    let (conn, tcp) = client::connect(&p.url, ServerTrust::Ca(&ca), Some((&cert, &key)))?;
    let (host, port) = client::split_url(&p.url)?;
    let tls = SharedTls::new(conn, tcp)?;
    let k = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, rnd::<16>());
    let hosthdr = if host.contains(':') { format!("[{host}]:{port}") } else { format!("{host}:{port}") };
    let mut w = tls.clone();
    write!(
        w,
        "GET /ws HTTP/1.1\r\nHost: {hosthdr}\r\nUser-Agent: bia-bridge\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {k}\r\nSec-WebSocket-Version: 13\r\n\r\n"
    )?;
    w.flush()?;
    let mut reader = BufReader::new(tls.clone());
    let mut status = String::new();
    reader.read_line(&mut status)?;
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h)? == 0 || h.trim().is_empty() {
            break;
        }
    }
    if !status.contains(" 101 ") {
        bail!("the bia refused the link: {}", status.trim());
    }
    Ok(Ws { tls, reader })
}

fn rnd<const N: usize>() -> [u8; N] {
    use ring::rand::SecureRandom;
    let mut b = [0u8; N];
    let _ = ring::rand::SystemRandom::new().fill(&mut b);
    b
}

/// One client frame (masked, as RFC 6455 asks of a client).
pub fn frame(op: u8, payload: &[u8]) -> Vec<u8> {
    let mask = rnd::<4>();
    let n = payload.len();
    let mut f = Vec::with_capacity(n + 14);
    f.push(0x80 | op);
    if n < 126 {
        f.push(0x80 | n as u8);
    } else if n < 65536 {
        f.push(0x80 | 126);
        f.extend_from_slice(&(n as u16).to_be_bytes());
    } else {
        f.push(0x80 | 127);
        f.extend_from_slice(&(n as u64).to_be_bytes());
    }
    f.extend_from_slice(&mask);
    f.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    f
}

/// A file chunk: `[fid 32 bytes][offset u64 BE][data]`.
pub fn chunk(fid: &[u8; 32], off: u64, data: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(40 + data.len());
    b.extend_from_slice(fid);
    b.extend_from_slice(&off.to_be_bytes());
    b.extend_from_slice(data);
    b
}

pub fn parse_chunk(b: &[u8]) -> Option<(String, u64, &[u8])> {
    if b.len() < 40 {
        return None;
    }
    let fid: String = b[..32].iter().map(|x| format!("{x:02x}")).collect();
    let off = u64::from_be_bytes(b[32..40].try_into().ok()?);
    Some((fid, off, &b[40..]))
}

pub fn unhex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

impl Ws {
    /// The next message: (opcode, payload). Fragments are joined.
    pub fn recv(&mut self) -> Result<(u8, Vec<u8>)> {
        let r = &mut self.reader;
        let mut first = None;
        let mut data = vec![];
        loop {
            let mut h = [0u8; 2];
            r.read_exact(&mut h)?;
            let fin = h[0] & 0x80 != 0;
            let op = h[0] & 0x0f;
            let masked = h[1] & 0x80 != 0;
            let mut len = (h[1] & 0x7f) as u64;
            if len == 126 {
                let mut b = [0u8; 2];
                r.read_exact(&mut b)?;
                len = u16::from_be_bytes(b) as u64;
            } else if len == 127 {
                let mut b = [0u8; 8];
                r.read_exact(&mut b)?;
                len = u64::from_be_bytes(b);
            }
            if len + data.len() as u64 > 16 << 20 {
                return Err(anyhow!("frame too large"));
            }
            let mut mask = [0u8; 4];
            if masked {
                r.read_exact(&mut mask)?;
            }
            let mut payload = vec![0u8; len as usize];
            r.read_exact(&mut payload)?;
            if masked {
                for (i, b) in payload.iter_mut().enumerate() {
                    *b ^= mask[i % 4];
                }
            }
            if op >= 8 {
                if first.is_none() {
                    return Ok((op, payload));
                }
                continue;
            }
            if op != 0 {
                first = Some(op);
            }
            data.extend_from_slice(&payload);
            if fin {
                return Ok((first.unwrap_or(1), data));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_encoding() {
        assert_eq!(urlenc("a b/ü.txt"), "a%20b%2F%C3%BC.txt");
    }

    #[test]
    fn frames_and_chunks() {
        let f = frame(1, b"hi");
        assert_eq!((f[0], f[1] & 0x7f), (0x81, 2));
        let k = [f[2], f[3], f[4], f[5]];
        assert_eq!([f[6] ^ k[0], f[7] ^ k[1]], *b"hi");
        let fid = [7u8; 32];
        let c = chunk(&fid, 70_000, b"data");
        let (h, off, d) = parse_chunk(&c).unwrap();
        assert_eq!((unhex32(&h), off, d), (Some(fid), 70_000, &b"data"[..]));
    }
}
