//! Talking to the bia: the certificate request (CSR; the key stays here), HTTPS calls with
//! the client certificate, and the chat WebSocket. The bia's CA is pinned; its server name
//! is not checked (bias are reached by addresses that change).
use crate::profile::{now_secs, random_hex, sha256_hex, Profile};
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

/// An HTTPS call with the client certificate. JSON in and out.
pub fn call(p: &Profile, method: &str, path: &str, body: Option<&Value>) -> Result<Value> {
    let (ca, cert, key) = p.creds()?;
    let r = client::request(&p.url, method, path, body, ServerTrust::Ca(&ca), Some((&cert, &key)))?;
    if r.status != 200 {
        bail!("{method} {path}: HTTP {} {}", r.status, r.text().trim());
    }
    r.json()
}

/// Upload a local file; returns the path the bia gave it (for `files` in a send).
pub fn upload(p: &Profile, file: &std::path::Path) -> Result<String> {
    let (ca, cert, key) = p.creds()?;
    let data = std::fs::read(file).with_context(|| format!("read {}", file.display()))?;
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
    let path = format!("/v1/upload?name={}", urlenc(&name));
    let r = client::request_raw(&p.url, "POST", &path, Some((&data, "application/octet-stream")), ServerTrust::Ca(&ca), Some((&cert, &key)))?;
    if r.status != 200 {
        bail!("upload {}: HTTP {} {}", file.display(), r.status, r.text().trim());
    }
    Ok(r.json()?["path"].as_str().unwrap_or("").to_string())
}

/// An attachment of a message, as bytes.
pub fn download(p: &Profile, chat: &str, id: &str, i: usize) -> Result<Vec<u8>> {
    let (ca, cert, key) = p.creds()?;
    let path = format!("/v1/att?chat={}&id={}&i={i}&dl=1", urlenc(chat), urlenc(id));
    let r = client::request(&p.url, "GET", &path, None, ServerTrust::Ca(&ca), Some((&cert, &key)))?;
    if r.status != 200 {
        bail!("attachment {i} of {id}: HTTP {}", r.status);
    }
    Ok(r.body)
}

/// Send a message in a chat (files are local paths, uploaded first). Returns its id.
pub fn send(p: &Profile, chat: &str, text: &str, files: &[String], reply_to: Option<&str>) -> Result<String> {
    let mut paths = vec![];
    for f in files {
        paths.push(upload(p, std::path::Path::new(f))?);
    }
    let mut a = json!({"t": "send", "chat": chat, "text": text, "files": paths});
    if let Some(r) = reply_to.filter(|r| !r.is_empty()) {
        a["reply_to"] = json!(r);
    }
    let v = call(p, "POST", "/v1/chats/action", Some(&a))?;
    Ok(v["id"].as_str().unwrap_or("").to_string())
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

// ---- the chat WebSocket ----

pub struct Ws {
    pub tls: SharedTls,
    pub reader: BufReader<SharedTls>,
}

pub fn ws_connect(p: &Profile) -> Result<Ws> {
    let (ca, cert, key) = p.creds()?;
    let (conn, tcp) = client::connect(&p.url, ServerTrust::Ca(&ca), Some((&cert, &key)))?;
    let (host, port) = client::split_url(&p.url)?;
    let tls = SharedTls::new(conn, tcp)?;
    let k = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, random_hex(16).as_bytes());
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
        bail!("chat link: {}", status.trim());
    }
    Ok(Ws { tls, reader })
}

impl Ws {
    /// A text frame, masked (this side is the client).
    pub fn send_text(tls: &SharedTls, text: &str) -> Result<()> {
        let mask: [u8; 4] = {
            use ring::rand::SecureRandom;
            let mut b = [0u8; 4];
            let _ = ring::rand::SystemRandom::new().fill(&mut b);
            b
        };
        let payload = text.as_bytes();
        let mut f = vec![0x81u8];
        let n = payload.len();
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
        let mut t = tls.clone();
        t.write_all(&f)?;
        t.flush()?;
        Ok(())
    }

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
}
