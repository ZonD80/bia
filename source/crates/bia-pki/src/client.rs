//! A small HTTPS client for talking to another bia's web API: TOFU or a pinned CA for
//! the server, optionally this bia's agent certificate as the client certificate.
//! Server names are not checked against the certificate (bias are reached by LAN or
//! public address that changes); trust comes from the peer's CA alone.
use anyhow::{anyhow, bail, Context, Result};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

/// How the server is trusted.
pub enum ServerTrust<'a> {
    /// Any certificate (first contact; the caller checks the CA it downloads).
    Any,
    /// Only certificates issued by this CA (PEM).
    Ca(&'a str),
}

pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
    /// This side's address on the connection (what the peer sees unless there is NAT).
    pub local: Option<SocketAddr>,
    /// The server's certificate chain as presented.
    pub server_chain: Vec<CertificateDer<'static>>,
}

impl Response {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
    pub fn json(&self) -> Result<serde_json::Value> {
        serde_json::from_slice(&self.body).with_context(|| format!("HTTP {}: not JSON: {}", self.status, first(&self.text(), 200)))
    }
}

fn first(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// `https://host[:port]` → (host, port).
pub fn split_url(url: &str) -> Result<(String, u16)> {
    let rest = url
        .trim()
        .strip_prefix("https://")
        .ok_or_else(|| anyhow!("{url}: use an https:// address"))?;
    let hostport = rest.split('/').next().unwrap_or("");
    if hostport.is_empty() {
        bail!("{url}: no host");
    }
    if let Some(v6) = hostport.strip_prefix('[') {
        let (h, tail) = v6.split_once(']').ok_or_else(|| anyhow!("{url}: bad IPv6 address"))?;
        let port = match tail.strip_prefix(':') {
            Some(p) => p.parse().map_err(|_| anyhow!("{url}: bad port"))?,
            None => 443,
        };
        return Ok((h.to_string(), port));
    }
    match hostport.rsplit_once(':') {
        Some((h, p)) => Ok((h.to_string(), p.parse().map_err(|_| anyhow!("{url}: bad port"))?)),
        None => Ok((hostport.to_string(), 443)),
    }
}

/// The origin part of `url` (`https://host[:port]`), normalised.
pub fn origin(url: &str) -> Result<String> {
    let (h, p) = split_url(url)?;
    let host = if h.contains(':') { format!("[{h}]") } else { h };
    Ok(if p == 443 { format!("https://{host}") } else { format!("https://{host}:{p}") })
}

/// One request (`Connection: close`). `client` is (certificate chain PEM, key PEM).
pub fn request(
    url: &str,
    method: &str,
    path: &str,
    body: Option<&serde_json::Value>,
    trust: ServerTrust<'_>,
    client: Option<(&str, &str)>,
) -> Result<Response> {
    crate::install_crypto_provider();
    let (host, port) = split_url(url)?;
    let verifier: Arc<dyn ServerCertVerifier> = match trust {
        ServerTrust::Any => Arc::new(AnyServer(provider_schemes())),
        ServerTrust::Ca(pem) => {
            let mut roots = RootCertStore::empty();
            for c in crate::pem_to_cert_ders(pem)? {
                roots.add(c).map_err(|e| anyhow!("peer CA: {e}"))?;
            }
            let inner = WebPkiServerVerifier::builder(Arc::new(roots))
                .build()
                .map_err(|e| anyhow!("verifier: {e}"))?;
            Arc::new(CaNoName(inner))
        }
    };
    let b = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(verifier);
    let mut cfg = match client {
        Some((chain, key)) => b
            .with_client_auth_cert(crate::pem_to_cert_ders(chain)?, crate::pem_to_key_der(key)?)
            .map_err(|e| anyhow!("client certificate: {e}"))?,
        None => b.with_no_client_auth(),
    };
    cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
    let name = match crate::parse_ip(&host) {
        Ok(ip) => ServerName::IpAddress(ip.into()),
        Err(_) => ServerName::try_from(host.clone()).map_err(|_| anyhow!("bad host {host}"))?,
    };
    let addr = (host.as_str(), port)
        .to_socket_addrs()
        .with_context(|| format!("resolve {host}"))?
        .next()
        .ok_or_else(|| anyhow!("{host}: no address"))?;
    let mut tcp = TcpStream::connect_timeout(&addr, Duration::from_secs(10))
        .with_context(|| format!("connect {addr}"))?;
    tcp.set_read_timeout(Some(Duration::from_secs(30)))?;
    tcp.set_write_timeout(Some(Duration::from_secs(30)))?;
    let local = tcp.local_addr().ok();
    let mut conn = rustls::ClientConnection::new(Arc::new(cfg), name)
        .map_err(|e| anyhow!("tls: {e}"))?;
    let mut tls = rustls::Stream::new(&mut conn, &mut tcp);
    let payload = body.map(|v| v.to_string()).unwrap_or_default();
    let hosthdr = if host.contains(':') { format!("[{host}]:{port}") } else { format!("{host}:{port}") };
    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {hosthdr}\r\nUser-Agent: bia-peer\r\nAccept: application/json\r\nConnection: close\r\n"
    );
    if body.is_some() {
        req.push_str(&format!("Content-Type: application/json\r\nContent-Length: {}\r\n", payload.len()));
    }
    req.push_str("\r\n");
    tls.write_all(req.as_bytes()).context("send request")?;
    tls.write_all(payload.as_bytes()).context("send body")?;
    tls.flush()?;
    let mut raw = Vec::new();
    // a peer that closes without close_notify still sent the whole response
    if let Err(e) = tls.read_to_end(&mut raw) {
        if raw.is_empty() {
            return Err(anyhow!("read response: {e}"));
        }
    }
    let server_chain: Vec<CertificateDer<'static>> = conn
        .peer_certificates()
        .map(|c| c.iter().map(|d| d.clone().into_owned()).collect())
        .unwrap_or_default();
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| anyhow!("{url}{path}: no HTTP response"))?;
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let mut body = raw[split + 4..].to_vec();
    let status: u16 = head
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| anyhow!("bad status line"))?;
    let lower = head.to_ascii_lowercase();
    if lower.contains("transfer-encoding: chunked") {
        body = dechunk(&body);
    } else if let Some(n) = lower
        .lines()
        .find_map(|l| l.strip_prefix("content-length:"))
        .and_then(|v| v.trim().parse::<usize>().ok())
    {
        body.truncate(n);
    }
    Ok(Response { status, body, local, server_chain })
}

fn dechunk(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let Some(nl) = b[i..].windows(2).position(|w| w == b"\r\n") else { break };
        let size = usize::from_str_radix(String::from_utf8_lossy(&b[i..i + nl]).trim().split(';').next().unwrap_or(""), 16).unwrap_or(0);
        i += nl + 2;
        if size == 0 || i + size > b.len() {
            break;
        }
        out.extend_from_slice(&b[i..i + size]);
        i += size + 2;
    }
    out
}

fn provider_schemes() -> Vec<SignatureScheme> {
    rustls::crypto::ring::default_provider()
        .signature_verification_algorithms
        .supported_schemes()
}

/// First contact: accept the server's certificate; the caller pins the CA afterwards.
#[derive(Debug)]
struct AnyServer(Vec<SignatureScheme>);

impl ServerCertVerifier for AnyServer {
    fn verify_server_cert(
        &self,
        _end: &CertificateDer<'_>,
        _inter: &[CertificateDer<'_>],
        _name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(m, c, d, &rustls::crypto::ring::default_provider().signature_verification_algorithms)
    }
    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(m, c, d, &rustls::crypto::ring::default_provider().signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.clone()
    }
}

/// The peer's CA must have issued the server certificate; the name in it does not matter.
#[derive(Debug)]
struct CaNoName(Arc<WebPkiServerVerifier>);

impl ServerCertVerifier for CaNoName {
    fn verify_server_cert(
        &self,
        end: &CertificateDer<'_>,
        inter: &[CertificateDer<'_>],
        name: &ServerName<'_>,
        ocsp: &[u8],
        now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        match self.0.verify_server_cert(end, inter, name, ocsp, now) {
            Err(rustls::Error::InvalidCertificate(
                CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. },
            )) => Ok(ServerCertVerified::assertion()),
            other => other,
        }
    }
    fn verify_tls12_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        self.0.verify_tls12_signature(m, c, d)
    }
    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        self.0.verify_tls13_signature(m, c, d)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.supported_verify_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        assert_eq!(split_url("https://10.0.0.5").unwrap(), ("10.0.0.5".into(), 443));
        assert_eq!(split_url("https://10.0.0.5:8443/x").unwrap(), ("10.0.0.5".into(), 8443));
        assert_eq!(split_url("https://[2001:db8::1]:8443").unwrap(), ("2001:db8::1".into(), 8443));
        assert!(split_url("http://10.0.0.5").is_err());
        assert_eq!(origin("https://10.0.0.5:443/v1").unwrap(), "https://10.0.0.5");
        assert_eq!(dechunk(b"3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n"), b"abcde");
    }
}
