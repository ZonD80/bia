//! Mutual trust between two bias in one issuance.
//!
//! Every bia has one agent key (`pki/agent.key`) and publishes a CSR for it at the public
//! `GET /v1/pki/agent`, under its agent name. When bia A connects to bia B:
//!
//! 1. A downloads B's CA (pinned by fingerprint when one is given) and B's agent CSR.
//! 2. A signs B's CSR with its own CA: a certificate B can use to call A.
//! 3. A files a certificate request at B with its own agent CSR **and** that certificate,
//!    A's CA and A's address (the *grant*).
//! 4. B checks the grant when the request arrives (B's own key, A's CA signed it). When
//!    B's root approves, B signs A's CSR and keeps the grant as a peer; A collects its
//!    certificate and keeps B as a peer.
//!
//! After that one approval each side holds a certificate from the other's CA for its own
//! agent key and trusts the other's CA for the server: both can call each other.
//! Records: `pki/peers/<name>/peer.json`, `ca.pem` (their CA), `cert.pem` (this bia's
//! certificate at them). The key is always `pki/agent.key`.
use crate::client::{self, ServerTrust};
use crate::{chmod600, now_secs, pem_to_cert_ders, sanitize_id, CertRequest, Pki};
use anyhow::{anyhow, bail, Context, Result};
use rcgen::{CertificateParams, DnType, KeyPair};
use rustls::pki_types::UnixTime;
use rustls::server::WebPkiClientVerifier;
use rustls::RootCertStore;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

/// What a requesting bia offers with its CSR: a certificate for the other side's agent key.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Grant {
    /// Certificate (PEM) for the receiver's agent key, issued by `ca`.
    pub cert: String,
    /// The requester's CA (PEM).
    pub ca: String,
    /// Where the requester's web API is (`https://host[:port]`).
    pub url: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Peer {
    /// Their agent name (their identity in this bia's CA).
    pub name: String,
    pub url: String,
    pub ca_sha256: String,
    /// This bia's identity in their CA.
    #[serde(default)]
    pub my_identity: String,
    /// `pending` (asked, waiting for them), `active`, `denied`, `expired`.
    pub status: String,
    /// `us` (this bia connected) or `them`.
    pub asked_by: String,
    /// The certificate request id (at them when we asked, here when they asked).
    #[serde(default)]
    pub request: String,
    /// Serial of the certificate this bia issued for them.
    #[serde(default)]
    pub issued_serial: String,
    #[serde(default)]
    pub note: String,
    pub since: u64,
    #[serde(default)]
    pub updated: u64,
}

impl Peer {
    pub fn describe(&self) -> String {
        format!(
            "{}  {}  {}  (asked by {}, CA {}){}",
            self.name,
            self.status,
            self.url,
            self.asked_by,
            &self.ca_sha256[..16.min(self.ca_sha256.len())],
            if self.my_identity.is_empty() {
                String::new()
            } else {
                format!("  I am {} there", self.my_identity)
            }
        )
    }
}

impl Pki {
    pub fn agent_key_path(&self) -> PathBuf {
        self.dir.join("agent.key")
    }

    fn agent_key(&self) -> Result<KeyPair> {
        let p = self.agent_key_path();
        if let Ok(pem) = fs::read_to_string(&p) {
            return KeyPair::from_pem(&pem).context("agent.key");
        }
        let key = KeyPair::generate()?;
        fs::write(&p, key.serialize_pem()).context("write agent.key")?;
        chmod600(&p);
        Ok(key)
    }

    /// This bia's agent name: `pki/agent.json` `name`, else `bia-` + the first 6 hex
    /// characters of its CA fingerprint (stable across hostname changes).
    pub fn agent_name(&self) -> String {
        fs::read(self.dir.join("agent.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .and_then(|v| v["name"].as_str().map(str::to_string))
            .filter(|n| sanitize_id(n).is_ok())
            .unwrap_or_else(|| format!("bia-{}", &self.ca_sha256().unwrap_or_default().get(..6).unwrap_or("000000")))
    }

    pub fn set_agent_name(&self, name: &str) -> Result<()> {
        let name = sanitize_id(name)?;
        if name == "root" {
            bail!("an agent cannot be named root");
        }
        fs::write(self.dir.join("agent.json"), serde_json::to_vec_pretty(&json!({"name": name}))?)?;
        Ok(())
    }

    /// CSR (PEM) for the agent key, under the agent name. Public: it holds no secret.
    pub fn agent_csr(&self) -> Result<String> {
        let key = self.agent_key()?;
        let mut p = CertificateParams::new(Vec::<String>::new())?;
        p.distinguished_name.push(DnType::CommonName, self.agent_name());
        Ok(p.serialize_request(&key)?.pem()?)
    }

    /// Check a grant: a currently valid client certificate for *this* bia's agent key,
    /// issued by the CA that comes with it, which is not this bia's own CA.
    /// Returns (their CA fingerprint, the identity they gave this bia).
    pub fn check_grant(&self, g: &Grant) -> Result<(String, String)> {
        let url = client::origin(&g.url).context("grant url")?;
        if url.len() > 300 {
            bail!("grant url is too long");
        }
        let ca = pem_to_cert_ders(&g.ca).context("grant ca")?;
        if ca.len() != 1 {
            bail!("grant ca: one CA certificate expected");
        }
        let ca_sha = crate::hex_sha256(ca[0].as_ref());
        if ca_sha == self.ca_sha256()? {
            bail!("that is this bia's own CA");
        }
        let chain = pem_to_cert_ders(&g.cert).context("grant cert")?;
        let leaf = chain.first().ok_or_else(|| anyhow!("grant cert: empty"))?;
        let mut roots = RootCertStore::empty();
        roots.add(ca[0].clone()).map_err(|e| anyhow!("grant ca: {e}"))?;
        let v = WebPkiClientVerifier::builder(Arc::new(roots))
            .build()
            .map_err(|e| anyhow!("verifier: {e}"))?;
        v.verify_client_cert(leaf, &[], UnixTime::now())
            .map_err(|e| anyhow!("grant cert is not a valid client certificate from that CA ({e})"))?;
        let (_, x) = x509_parser::parse_x509_certificate(leaf.as_ref()).map_err(|_| anyhow!("grant cert: cannot parse"))?;
        if x.public_key().raw != self.agent_key()?.public_key_der().as_slice() {
            bail!("grant cert is for another key, not this bia's agent key (GET /v1/pki/agent)");
        }
        let id = crate::client_id_from_certs(Some(&chain[..1]))
            .map(|c| c.identity)
            .ok_or_else(|| anyhow!("grant cert has no identity"))?;
        Ok((ca_sha, id))
    }

    fn peers_dir(&self) -> PathBuf {
        self.dir.join("peers")
    }

    fn peer_dir(&self, name: &str) -> Result<PathBuf> {
        Ok(self.peers_dir().join(sanitize_id(name)?))
    }

    pub fn peer(&self, name: &str) -> Option<Peer> {
        let d = self.peer_dir(name).ok()?;
        serde_json::from_slice(&fs::read(d.join("peer.json")).ok()?).ok()
    }

    pub fn peers(&self) -> Vec<Peer> {
        let mut out: Vec<Peer> = fs::read_dir(self.peers_dir())
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| self.peer(&e.file_name().to_string_lossy()))
            .collect();
        out.sort_by(|a, b| a.since.cmp(&b.since));
        out
    }

    /// Files to call a peer with: (their CA, this bia's certificate there, agent key).
    pub fn peer_files(&self, name: &str) -> Result<(PathBuf, PathBuf, PathBuf)> {
        let d = self.peer_dir(name)?;
        Ok((d.join("ca.pem"), d.join("cert.pem"), self.agent_key_path()))
    }

    fn save_peer(&self, p: &Peer, ca_pem: Option<&str>, cert_pem: Option<&str>) -> Result<()> {
        let d = self.peer_dir(&p.name)?;
        fs::create_dir_all(&d)?;
        if let Some(ca) = ca_pem {
            fs::write(d.join("ca.pem"), ca)?;
        }
        if let Some(c) = cert_pem {
            fs::write(d.join("cert.pem"), c)?;
        }
        let tmp = d.join("peer.json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(p)?)?;
        chmod600(&tmp);
        fs::rename(&tmp, d.join("peer.json"))?;
        Ok(())
    }

    /// Called when this bia approves a request that carried a grant: the requester
    /// becomes a peer this bia can call.
    pub(crate) fn accept_grant(&self, r: &CertRequest, g: &Grant) -> Result<Peer> {
        let (ca_sha, my_id) = self.check_grant(g)?;
        let p = Peer {
            name: r.identity.clone(),
            url: client::origin(&g.url)?,
            ca_sha256: ca_sha,
            my_identity: my_id,
            status: "active".into(),
            asked_by: "them".into(),
            request: r.id.clone(),
            issued_serial: r.serial.clone(),
            note: r.note.clone(),
            since: now_secs(),
            updated: now_secs(),
        };
        self.save_peer(&p, Some(&g.ca), Some(&g.cert))?;
        Ok(p)
    }

    /// Connect to the bia at `url`: sign its agent key, ask it for a certificate for ours
    /// (with that grant) and keep it as a pending peer. `pin` is its CA fingerprint
    /// (sha256 hex) when known; `my_url` is where it can reach this bia (default: this
    /// side's address on the connection, `port` = this bia's HTTPS port).
    pub fn peer_connect(
        &self,
        url: &str,
        pin: Option<&str>,
        my_url: Option<&str>,
        port: u16,
        note: &str,
    ) -> Result<Peer> {
        let url = client::origin(url)?;
        // 1. their CA, trust on first use unless pinned; the server must hold a cert from it
        let r = client::request(&url, "GET", "/v1/pki/ca", None, ServerTrust::Any, None)
            .with_context(|| format!("reach {url}"))?;
        if r.status != 200 {
            bail!("{url}/v1/pki/ca: HTTP {} (is it a bia?)", r.status);
        }
        let ca_pem = r.text();
        let ca = pem_to_cert_ders(&ca_pem).context("their CA")?;
        let ca_sha = crate::hex_sha256(ca[0].as_ref());
        if let Some(pin) = pin.map(|p| p.trim().to_ascii_lowercase().replace(':', "")).filter(|p| !p.is_empty()) {
            if pin != ca_sha {
                bail!("{url} has CA fingerprint {ca_sha}, not the expected {pin}; not connecting");
            }
        }
        if ca_sha == self.ca_sha256()? {
            bail!("{url} is this bia itself");
        }
        // 2. their agent CSR, now over a connection their CA vouches for
        let r = client::request(&url, "GET", "/v1/pki/agent", None, ServerTrust::Ca(&ca_pem), None)?;
        if r.status == 404 {
            bail!("{url} runs an older bia without mutual trust (no /v1/pki/agent); update it first");
        }
        let a = r.json()?;
        let their_name = a["name"].as_str().unwrap_or("").to_string();
        let their_name = sanitize_id(&their_name).context("their agent name")?;
        if their_name == "root" || their_name == self.agent_name() {
            bail!("{url} calls itself {their_name}, which clashes with a name here; set another agent name on one side");
        }
        if let Some(p) = self.peer(&their_name) {
            if p.status == "active" || p.status == "pending" {
                bail!("{their_name} is already a peer ({}); remove it first to connect again", p.status);
            }
        }
        let their_csr = a["csr"].as_str().unwrap_or("");
        // 3. the grant: their key, signed by our CA
        let (chain, meta) = self.issue_csr(&their_name, "agent", "normal", their_csr)?;
        let leaf_end = chain.find("-----END CERTIFICATE-----").map(|i| i + 25).unwrap_or(chain.len());
        let grant_cert = format!("{}\n", chain[..leaf_end].trim());
        let mine = self.agent_csr()?;
        let mut grant = Grant { cert: grant_cert, ca: self.ca_pem()?, url: String::new() };
        let body = |g: &Grant| {
            json!({"identity": self.agent_name(), "role": "agent", "note": note, "csr": mine, "grant": g})
        };
        grant.url = match my_url {
            Some(u) => client::origin(u)?,
            None => {
                // what they see of us: the local address of a connection to them
                let probe = client::request(&url, "GET", "/v1/pki/agent", None, ServerTrust::Ca(&ca_pem), None)?;
                let ip = probe.local.map(|a| a.ip()).ok_or_else(|| anyhow!("no local address; pass my_url"))?;
                let host = if ip.is_ipv6() { format!("[{ip}]") } else { ip.to_string() };
                if port == 443 { format!("https://{host}") } else { format!("https://{host}:{port}") }
            }
        };
        let r = client::request(&url, "POST", "/v1/pki/requests", Some(&body(&grant)), ServerTrust::Ca(&ca_pem), None)?;
        if r.status != 202 && r.status != 200 {
            let _ = self.revoke(&meta.serial);
            bail!("{url} refused the request: HTTP {} {}", r.status, r.text().trim());
        }
        let v = r.json()?;
        let id = v["id"].as_str().unwrap_or("").to_string();
        let p = Peer {
            name: their_name,
            url,
            ca_sha256: ca_sha,
            my_identity: self.agent_name(),
            status: "pending".into(),
            asked_by: "us".into(),
            request: id,
            issued_serial: meta.serial,
            note: note.to_string(),
            since: now_secs(),
            updated: now_secs(),
        };
        self.save_peer(&p, Some(&ca_pem), None)?;
        Ok(p)
    }

    /// Check a pending peer once. Returns the peer when its state changed.
    pub fn peer_poll(&self, name: &str) -> Result<Option<Peer>> {
        let Some(mut p) = self.peer(name) else { bail!("no peer {name}") };
        if p.status != "pending" {
            return Ok(None);
        }
        let (ca_path, cert_path, _) = self.peer_files(name)?;
        let ca_pem = fs::read_to_string(&ca_path)?;
        let r = client::request(&p.url, "GET", &format!("/v1/pki/requests/{}", p.request), None, ServerTrust::Ca(&ca_pem), None)?;
        let status = if r.status == 404 { "expired".to_string() } else { r.json()?["status"].as_str().unwrap_or("").to_string() };
        match status.as_str() {
            "approved" | "issued" => {
                let c = client::request(&p.url, "GET", &format!("/v1/pki/requests/{}/cert", p.request), None, ServerTrust::Ca(&ca_pem), None)?;
                if c.status != 200 {
                    bail!("collect certificate: HTTP {}", c.status);
                }
                let chain = c.text();
                let ders = pem_to_cert_ders(&chain)?;
                let mut roots = RootCertStore::empty();
                for d in pem_to_cert_ders(&ca_pem)? {
                    roots.add(d).map_err(|e| anyhow!("{e}"))?;
                }
                let v = WebPkiClientVerifier::builder(Arc::new(roots)).build().map_err(|e| anyhow!("{e}"))?;
                v.verify_client_cert(&ders[0], &[], UnixTime::now())
                    .map_err(|e| anyhow!("their certificate does not check out ({e})"))?;
                let (_, x) = x509_parser::parse_x509_certificate(ders[0].as_ref()).map_err(|_| anyhow!("parse"))?;
                if x.public_key().raw != self.agent_key()?.public_key_der().as_slice() {
                    bail!("their certificate is not for this bia's agent key");
                }
                fs::write(&cert_path, &chain)?;
                p.my_identity = crate::client_id_from_certs(Some(&ders[..1])).map(|c| c.identity).unwrap_or(p.my_identity);
                p.status = "active".into();
            }
            "denied" | "expired" => {
                let _ = self.revoke(&p.issued_serial);
                p.status = status;
            }
            _ => return Ok(None),
        }
        p.updated = now_secs();
        self.save_peer(&p, None, None)?;
        Ok(Some(p))
    }

    /// Call a peer's web API with this bia's certificate there.
    pub fn peer_call(&self, name: &str, method: &str, path: &str, body: Option<&serde_json::Value>) -> Result<client::Response> {
        let p = self.peer(name).ok_or_else(|| anyhow!("no peer {name} (see the peer list)"))?;
        if p.status != "active" {
            bail!("peer {name} is {}", p.status);
        }
        if !path.starts_with('/') {
            bail!("path must start with / (e.g. /v1/status)");
        }
        let (ca, cert, key) = self.peer_files(name)?;
        let (ca, cert, key) = (fs::read_to_string(ca)?, fs::read_to_string(cert)?, fs::read_to_string(key)?);
        client::request(&p.url, method, path, body, ServerTrust::Ca(&ca), Some((&cert, &key)))
    }

    /// Forget a peer and revoke the certificate this bia issued for it.
    pub fn peer_remove(&self, name: &str) -> Result<Peer> {
        let p = self.peer(name).ok_or_else(|| anyhow!("no peer {name}"))?;
        if !p.issued_serial.is_empty() {
            let _ = self.revoke(&p.issued_serial);
        }
        fs::remove_dir_all(self.peer_dir(name)?)?;
        Ok(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_pki(tag: &str) -> (Pki, PathBuf) {
        let dir = std::env::temp_dir().join(format!("bia-pki-peer-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        (Pki::load_or_create(&dir).unwrap(), dir)
    }

    /// The whole exchange without the network: A signs B's agent CSR, files a request at
    /// B with that grant, B approves; both sides end up with a certificate from the other.
    #[test]
    fn one_approval_gives_trust_both_ways() {
        crate::install_crypto_provider();
        let (a, da) = temp_pki("a");
        let (b, db) = temp_pki("b");
        assert_ne!(a.agent_name(), b.agent_name());
        // A: sign B's agent key
        let (chain, meta) = a.issue_csr(&b.agent_name(), "agent", "normal", &b.agent_csr().unwrap()).unwrap();
        let leaf = chain[..chain.find("-----END CERTIFICATE-----").unwrap() + 25].to_string();
        let grant = Grant { cert: leaf, ca: a.ca_pem().unwrap(), url: "https://10.0.0.1".into() };
        let (ca_sha, me_at_a) = b.check_grant(&grant).unwrap();
        assert_eq!(ca_sha, a.ca_sha256().unwrap());
        assert_eq!(me_at_a, b.agent_name());
        // a grant for someone else's key, or signed by B's own CA, is refused
        let (other, _) = a.issue_csr("x", "agent", "normal", &a.agent_csr().unwrap()).unwrap();
        assert!(b.check_grant(&Grant { cert: other, ..grant.clone() }).is_err());
        assert!(b.check_grant(&Grant { ca: b.ca_pem().unwrap(), ..grant.clone() }).is_err());
        // B: the request with the grant, approved
        let r = b
            .create_request_grant(&a.agent_name(), "agent", "hi", "10.0.0.1", &a.agent_csr().unwrap(), Some(grant))
            .unwrap();
        let d = b.decide_request(&r.id, true, "root", "").unwrap();
        let peer = b.peer(&a.agent_name()).unwrap();
        assert_eq!((peer.status.as_str(), peer.asked_by.as_str()), ("active", "them"));
        assert_eq!(peer.issued_serial, d.serial);
        // B can now present A's certificate (its agent key) to A
        let (ca_path, cert_path, key_path) = b.peer_files(&a.agent_name()).unwrap();
        let cert = fs::read_to_string(cert_path).unwrap();
        let ders = pem_to_cert_ders(&cert).unwrap();
        let id = a.check_client(Some(&ders)).unwrap().unwrap();
        assert_eq!(id.identity, b.agent_name());
        assert_eq!(id.serial_hint, meta.serial);
        assert!(fs::read_to_string(ca_path).unwrap().contains("BEGIN CERTIFICATE"));
        assert!(fs::read_to_string(key_path).unwrap().contains("PRIVATE KEY"));
        // and A's certificate from B works at B
        let (a_chain, _) = b.collect_request(&r.id).unwrap();
        let ders = pem_to_cert_ders(&a_chain).unwrap();
        assert_eq!(b.check_client(Some(&ders[..1])).unwrap().unwrap().identity, a.agent_name());
        // removing the peer revokes what B issued
        b.peer_remove(&a.agent_name()).unwrap();
        assert!(b.is_revoked(&d.serial));
        let _ = fs::remove_dir_all(da);
        let _ = fs::remove_dir_all(db);
    }

    /// A minimal bia web API over real TLS: CA, agent CSR, requests (approved at once),
    /// status, certificate, and /whoami behind the client certificate.
    fn serve(b: Pki) -> u16 {
        use std::io::{BufRead, BufReader, Read, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        b.ensure_server_sans(&["127.0.0.1".parse().unwrap()]).unwrap();
        let cfg = b.server_config().unwrap();
        std::thread::spawn(move || {
            for tcp in l.incoming().flatten() {
                let mut tcp = tcp;
                let peer = tcp.peer_addr().unwrap().ip();
                let mut conn = rustls::ServerConnection::new(cfg.clone()).unwrap();
                while conn.is_handshaking() {
                    if conn.complete_io(&mut tcp).is_err() {
                        break;
                    }
                }
                let who = b.check_client_from(conn.peer_certificates(), Some(peer));
                let mut tls = rustls::StreamOwned::new(conn, tcp);
                let mut rd = BufReader::new(&mut tls);
                let mut line = String::new();
                rd.read_line(&mut line).unwrap();
                let mut len = 0;
                loop {
                    let mut h = String::new();
                    rd.read_line(&mut h).unwrap();
                    if h.trim().is_empty() {
                        break;
                    }
                    if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap();
                    }
                }
                let mut body = vec![0u8; len];
                rd.read_exact(&mut body).unwrap();
                let mut parts = line.split_whitespace();
                let (m, path) = (parts.next().unwrap().to_string(), parts.next().unwrap().to_string());
                let (code, out) = match (m.as_str(), path.as_str()) {
                    ("GET", "/v1/pki/ca") => (200, b.ca_pem().unwrap()),
                    ("GET", "/v1/pki/agent") => (200, json!({"name": b.agent_name(), "csr": b.agent_csr().unwrap()}).to_string()),
                    ("POST", "/v1/pki/requests") => {
                        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
                        let g: Option<Grant> = serde_json::from_value(v["grant"].clone()).ok();
                        match b.create_request_grant(v["identity"].as_str().unwrap(), "agent", "", &peer.to_string(), v["csr"].as_str().unwrap(), g) {
                            Ok(r) => {
                                b.decide_request(&r.id, true, "root@test", "").unwrap();
                                (202, json!({"id": r.id}).to_string())
                            }
                            Err(e) => (400, format!("{e:#}")),
                        }
                    }
                    ("GET", "/whoami") => match &who {
                        Ok(Some(id)) => (200, id.identity.clone()),
                        _ => (401, "no".into()),
                    },
                    ("GET", p) if p.ends_with("/cert") => {
                        let id = p.trim_start_matches("/v1/pki/requests/").trim_end_matches("/cert");
                        (200, b.collect_request(id).unwrap().0)
                    }
                    ("GET", p) => {
                        let id = p.trim_start_matches("/v1/pki/requests/");
                        (200, json!({"status": b.request(id).map(|r| r.status.as_str()).unwrap_or("expired")}).to_string())
                    }
                    _ => (404, String::new()),
                };
                let _ = write!(tls, "HTTP/1.1 {code} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{out}", out.len());
                tls.conn.send_close_notify();
                let _ = tls.flush();
            }
        });
        port
    }

    #[test]
    fn connect_over_tls_and_call() {
        crate::install_crypto_provider();
        let (a, da) = temp_pki("ta");
        let (b, db) = temp_pki("tb");
        let b_name = b.agent_name();
        let b_ca = b.ca_sha256().unwrap();
        let b_check = Pki { dir: b.dir.clone() };
        let url = format!("https://127.0.0.1:{}", serve(b));
        assert!(a.peer_connect(&url, Some("00"), None, 8443, "").is_err(), "wrong pin");
        let p = a.peer_connect(&url, Some(&b_ca), None, 8443, "hello").unwrap();
        assert_eq!((p.name.as_str(), p.status.as_str()), (b_name.as_str(), "pending"));
        let p = a.peer_poll(&b_name).unwrap().unwrap();
        assert_eq!(p.status, "active");
        let r = a.peer_call(&b_name, "GET", "/whoami", None).unwrap();
        assert_eq!((r.status, r.text()), (200, a.agent_name()));
        // B keeps A as a peer with the address A saw itself at
        let back = b_check.peer(&a.agent_name()).unwrap();
        assert_eq!(back.url, "https://127.0.0.1:8443");
        assert!(a.peer_connect(&url, None, None, 8443, "").is_err(), "already a peer");
        let _ = fs::remove_dir_all(da);
        let _ = fs::remove_dir_all(db);
    }

    #[test]
    fn bad_grant_refuses_the_request() {
        crate::install_crypto_provider();
        let (a, da) = temp_pki("ga");
        let (b, db) = temp_pki("gb");
        let bad = Grant { cert: "x".into(), ca: a.ca_pem().unwrap(), url: "https://10.0.0.1".into() };
        assert!(b
            .create_request_grant("bia-a", "agent", "", "10.0.0.1", &a.agent_csr().unwrap(), Some(bad))
            .is_err());
        let _ = fs::remove_dir_all(da);
        let _ = fs::remove_dir_all(db);
    }
}
