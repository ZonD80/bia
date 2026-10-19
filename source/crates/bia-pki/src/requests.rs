//! Certificate requests from other agents (or people) that have no certificate yet.
//!
//! The requester makes its own key pair and `POST`s an identity plus a CSR; it gets a
//! random UUID back. Only the public key ever reaches this machine. A human here
//! approves or denies the request, which can take hours, so the requester polls by
//! UUID. Approval signs the CSR's key (identity, role, trust and usages are set here,
//! not taken from the CSR); the requester then downloads the certificate, which is
//! useless without its private key.
//!
//! Records live in `pki/requests/<uuid>.json` so a reboot does not lose them.
use crate::{chmod600, now_secs, random_token, sanitize_id, Pki};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// A pending request waits this long for a decision.
pub const REQUEST_TTL_SECS: u64 = 3 * 24 * 3600;
/// Suggested poll interval for requesters.
pub const POLL_SECS: u64 = 15;
const MAX_PENDING: usize = 20;
const MAX_PENDING_PER_SOURCE: usize = 3;
const NOTE_MAX: usize = 300;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum Status {
    #[serde(rename = "pending")]
    Pending,
    #[serde(rename = "approved")]
    Approved,
    #[serde(rename = "denied")]
    Denied,
    /// Approved and the certificate was downloaded at least once.
    #[serde(rename = "issued")]
    Issued,
    #[serde(rename = "expired")]
    Expired,
}

impl Status {
    pub fn as_str(&self) -> &'static str {
        match self {
            Status::Pending => "pending",
            Status::Approved => "approved",
            Status::Denied => "denied",
            Status::Issued => "issued",
            Status::Expired => "expired",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CertRequest {
    pub id: String,
    pub identity: String,
    pub role: String,
    /// What the requester says about itself (untrusted, shown to the approver).
    #[serde(default)]
    pub note: String,
    /// Peer address the request came from.
    #[serde(default)]
    pub source: String,
    pub created: u64,
    pub status: Status,
    #[serde(default)]
    pub decided: u64,
    #[serde(default)]
    pub decided_by: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub serial: String,
    /// The requester's CSR (PEM), checked when filed.
    #[serde(default)]
    pub csr: String,
    /// Signed certificate + CA (PEM), set on approval.
    #[serde(default)]
    pub cert: String,
    /// A requesting bia's certificate for this bia's agent key (mutual trust); checked
    /// when filed, kept as a peer on approval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grant: Option<crate::Grant>,
}

impl CertRequest {
    /// When a pending request lapses (0 = no deadline in this state).
    pub fn expires_at(&self) -> u64 {
        match self.status {
            Status::Pending => self.created + REQUEST_TTL_SECS,
            _ => 0,
        }
    }

    pub fn short(&self) -> &str {
        &self.id[..8.min(self.id.len())]
    }
}

/// Random RFC 4122 version 4 UUID.
pub fn new_uuid() -> String {
    let hex = random_token(16);
    let mut b: Vec<u8> = (0..16)
        .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap_or(0))
        .collect();
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

fn valid_uuid(s: &str) -> bool {
    s.len() == 36
        && s.chars().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit() && !c.is_ascii_uppercase(),
        })
}

impl Pki {
    fn requests_dir(&self) -> PathBuf {
        self.dir.join("requests")
    }

    fn request_path(&self, id: &str) -> PathBuf {
        self.requests_dir().join(format!("{id}.json"))
    }

    fn save_request(&self, r: &CertRequest) -> Result<()> {
        fs::create_dir_all(self.requests_dir()).context("mkdir pki/requests")?;
        let path = self.request_path(&r.id);
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(r)?).context("write request")?;
        chmod600(&tmp);
        fs::rename(&tmp, &path).context("save request")?;
        Ok(())
    }

    /// Load one request by its full UUID, marking it expired when its time ran out.
    pub fn request(&self, id: &str) -> Option<CertRequest> {
        let id = id.trim().to_ascii_lowercase();
        if !valid_uuid(&id) {
            return None;
        }
        let mut r: CertRequest =
            serde_json::from_slice(&fs::read(self.request_path(&id)).ok()?).ok()?;
        let at = r.expires_at();
        if at > 0 && now_secs() > at {
            r.status = Status::Expired;
            let _ = self.save_request(&r);
        }
        Some(r)
    }

    /// Every request, newest first (expired ones included).
    pub fn list_requests(&self) -> Vec<CertRequest> {
        let mut out: Vec<CertRequest> = fs::read_dir(self.requests_dir())
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                name.strip_suffix(".json").and_then(|id| self.request(id))
            })
            .collect();
        out.sort_by(|a, b| b.created.cmp(&a.created));
        out
    }

    /// Find a request by its UUID or a unique prefix of at least 6 characters (for
    /// people typing it). Only the machine owner uses prefixes; requesters need the UUID.
    pub fn find_request(&self, id_or_prefix: &str) -> Result<CertRequest> {
        let want = id_or_prefix.trim().to_ascii_lowercase();
        if let Some(r) = self.request(&want) {
            return Ok(r);
        }
        if want.len() < 6 {
            bail!("give the request id (at least 6 characters)");
        }
        let hits: Vec<CertRequest> = self
            .list_requests()
            .into_iter()
            .filter(|r| r.id.starts_with(&want))
            .collect();
        match hits.len() {
            0 => bail!("no certificate request {want}"),
            1 => Ok(hits.into_iter().next().unwrap()),
            _ => bail!("{want} matches several requests; give more of the id"),
        }
    }

    /// File a new request. `source` is the peer address (for limits and for the approver).
    pub fn create_request(
        &self,
        identity: &str,
        role: &str,
        note: &str,
        source: &str,
        csr: &str,
    ) -> Result<CertRequest> {
        self.create_request_grant(identity, role, note, source, csr, None)
    }

    /// `create_request` from another bia that also offers a certificate for this bia's
    /// agent key (`grant`); a grant that does not check out refuses the request.
    pub fn create_request_grant(
        &self,
        identity: &str,
        role: &str,
        note: &str,
        source: &str,
        csr: &str,
        grant: Option<crate::Grant>,
    ) -> Result<CertRequest> {
        if let Some(g) = &grant {
            self.check_grant(g)?;
        }
        let identity = sanitize_id(identity)?;
        if identity == "root" {
            bail!("root certificates are not requested; root issues them on this machine");
        }
        let role = match role.trim() {
            "" | "agent" => "agent",
            "user" => "user",
            _ => bail!("role must be agent or user"),
        };
        crate::check_csr(csr)?;
        let note: String = note
            .chars()
            .filter(|c| !c.is_control())
            .take(NOTE_MAX)
            .collect();
        let pending: Vec<CertRequest> = self
            .list_requests()
            .into_iter()
            .filter(|r| r.status == Status::Pending)
            .collect();
        if pending.len() >= MAX_PENDING {
            bail!("too many certificate requests are waiting; try again later");
        }
        if pending.iter().filter(|r| r.source == source).count() >= MAX_PENDING_PER_SOURCE {
            bail!("this address already has {MAX_PENDING_PER_SOURCE} requests waiting");
        }
        let r = CertRequest {
            id: new_uuid(),
            identity,
            role: role.into(),
            note: note.trim().to_string(),
            source: source.to_string(),
            created: now_secs(),
            status: Status::Pending,
            decided: 0,
            decided_by: String::new(),
            reason: String::new(),
            serial: String::new(),
            csr: csr.trim().to_string() + "\n",
            cert: String::new(),
            grant,
        };
        self.save_request(&r)?;
        Ok(r)
    }

    /// Approve or deny a pending request. `by` records who decided (for the log).
    pub fn decide_request(
        &self,
        id_or_prefix: &str,
        approve: bool,
        by: &str,
        reason: &str,
    ) -> Result<CertRequest> {
        self.decide_request_opts(id_or_prefix, approve, by, reason, &[])
    }

    /// `decide_request`; an approved certificate only works from `from` (empty = anywhere).
    pub fn decide_request_opts(
        &self,
        id_or_prefix: &str,
        approve: bool,
        by: &str,
        reason: &str,
        from: &[crate::Cidr],
    ) -> Result<CertRequest> {
        let mut r = self.find_request(id_or_prefix)?;
        if r.status != Status::Pending {
            bail!("request {} is already {}", r.short(), r.status.as_str());
        }
        if approve {
            let (pem, meta) = self.issue_csr_from(&r.identity, &r.role, "normal", &r.csr, from)?;
            r.cert = pem;
            r.serial = meta.serial;
            if let Some(g) = r.grant.clone() {
                // the requester's certificate for us: it is a peer from now on
                if let Err(e) = self.accept_grant(&r, &g) {
                    let _ = self.revoke(&r.serial);
                    return Err(e.context("the requester's certificate for this bia no longer checks out"));
                }
            }
        }
        r.status = if approve {
            Status::Approved
        } else {
            Status::Denied
        };
        r.decided = now_secs();
        r.decided_by = by.to_string();
        r.reason = reason
            .chars()
            .filter(|c| !c.is_control())
            .take(NOTE_MAX)
            .collect();
        self.save_request(&r)?;
        Ok(r)
    }

    /// The signed certificate + CA (PEM) for an approved request. Trust is always
    /// `normal`; root trust is never handed out through a request. Downloading again is
    /// harmless (it is public data without the requester's key).
    pub fn collect_request(&self, id: &str) -> Result<(String, CertRequest)> {
        let Some(mut r) = self.request(id) else {
            bail!("no such request");
        };
        if !matches!(r.status, Status::Approved | Status::Issued) || r.cert.is_empty() {
            bail!("request is {}", r.status.as_str());
        }
        if r.status == Status::Approved {
            r.status = Status::Issued;
            self.save_request(&r)?;
        }
        Ok((r.cert.clone(), r))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_pki(tag: &str) -> (Pki, PathBuf) {
        let dir = std::env::temp_dir().join(format!("bia-pki-req-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        (Pki::load_or_create(&dir).unwrap(), dir)
    }

    /// A CSR the way a requester makes one: its own key, which never leaves it.
    fn csr(cn: &str) -> (String, rcgen::KeyPair) {
        let key = rcgen::KeyPair::generate().unwrap();
        let mut p = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
        p.distinguished_name.push(rcgen::DnType::CommonName, cn);
        (p.serialize_request(&key).unwrap().pem().unwrap(), key)
    }

    #[test]
    fn uuid_shape() {
        let u = new_uuid();
        assert!(valid_uuid(&u), "{u}");
        assert_eq!(&u[14..15], "4");
        assert!("89ab".contains(&u[19..20]));
        assert_ne!(u, new_uuid());
        assert!(!valid_uuid("../../etc/passwd"));
    }

    #[test]
    fn request_flow() {
        crate::install_crypto_provider();
        let (pki, dir) = temp_pki("flow");
        let (pem, key) = csr("root");
        assert!(pki
            .create_request("root", "agent", "", "1.2.3.4", &pem)
            .is_err());
        assert!(
            pki.create_request("peer-bia", "agent", "", "1.2.3.4", "")
                .is_err(),
            "csr required"
        );
        assert!(pki
            .create_request(
                "peer-bia",
                "agent",
                "",
                "1.2.3.4",
                "-----BEGIN CERTIFICATE REQUEST-----\nAAAA\n-----END CERTIFICATE REQUEST-----"
            )
            .is_err());
        let r = pki
            .create_request("peer-bia", "", "hello\u{7}there", "1.2.3.4", &pem)
            .unwrap();
        assert_eq!(r.role, "agent");
        assert_eq!(r.note, "hellothere");
        assert!(pki.collect_request(&r.id).is_err(), "not before approval");
        assert!(
            pki.request(&r.id[..8]).is_none(),
            "requesters need the full id"
        );
        let d = pki.decide_request(&r.id[..8], true, "root", "").unwrap();
        assert_eq!(d.status, Status::Approved);
        assert!(
            pki.decide_request(&r.id, false, "root", "").is_err(),
            "decided once"
        );
        let (chain, after) = pki.collect_request(&r.id).unwrap();
        assert_eq!(after.status, Status::Issued);
        assert!(!chain.contains("PRIVATE KEY"));
        assert_eq!(chain.matches("BEGIN CERTIFICATE").count(), 2, "leaf + CA");
        // the leaf carries the requester's key and the identity set here, not the CSR's CN
        let leaf = rustls_pemfile::certs(&mut chain.as_bytes())
            .next()
            .unwrap()
            .unwrap();
        let (_, x) = x509_parser::parse_x509_certificate(leaf.as_ref()).unwrap();
        assert_eq!(
            x.public_key().subject_public_key.data.as_ref(),
            key.public_key_raw()
        );
        let cn = x
            .subject()
            .iter_common_name()
            .next()
            .unwrap()
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(cn, "Identity: peer-bia");
        assert!(x.subject().to_string().contains("trust:normal"));
        assert!(
            pki.collect_request(&r.id).is_ok(),
            "re-download is harmless"
        );
        let m = pki.list_issued().unwrap();
        assert!(m
            .iter()
            .any(|m| m.serial == after.serial && m.identity == "peer-bia" && m.trust == "normal"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn openssl_csrs_accepted() {
        // made with openssl req -new -newkey rsa:2048 | ec P-256 | ed25519 -nodes -subj /CN=x
        for (name, pem) in [
            ("rsa2048", include_str!("../testdata/rsa2048.csr")),
            ("p256", include_str!("../testdata/p256.csr")),
            ("ed25519", include_str!("../testdata/ed25519.csr")),
        ] {
            if let Err(e) = crate::check_csr(pem) {
                panic!("{name}: {e}");
            }
        }
        let (pki, dir) = temp_pki("openssl");
        let r = pki
            .create_request(
                "o",
                "agent",
                "",
                "1.1.1.1",
                include_str!("../testdata/p256.csr"),
            )
            .unwrap();
        pki.decide_request(&r.id, true, "root", "").unwrap();
        assert!(pki.collect_request(&r.id).is_ok());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn renew_signs_the_holders_key() {
        crate::install_crypto_provider();
        let (pki, dir) = temp_pki("renew");
        let (first, _) = csr("x");
        let (_, old) = pki.issue_csr("alice", "user", "root", &first).unwrap();
        let me = crate::ClientId {
            identity: "alice".into(),
            role: "user".into(),
            trust: "root".into(),
            serial_hint: old.serial.clone(),
            not_after: old.not_after as i64,
        };
        assert!(pki.renew_csr(&me, "").is_err(), "a CSR is required");
        let (next, key) = csr("ignored");
        let (chain, meta) = pki.renew_csr(&me, &next).unwrap();
        assert!(!chain.contains("PRIVATE KEY"));
        assert_eq!(
            (
                meta.identity.as_str(),
                meta.role.as_str(),
                meta.trust.as_str()
            ),
            ("alice", "user", "root")
        );
        let leaf = rustls_pemfile::certs(&mut chain.as_bytes())
            .next()
            .unwrap()
            .unwrap();
        let (_, x) = x509_parser::parse_x509_certificate(leaf.as_ref()).unwrap();
        assert_eq!(
            x.public_key().subject_public_key.data.as_ref(),
            key.public_key_raw()
        );
        assert!(pki.is_revoked(&old.serial), "old serial revoked");
        assert!(
            pki.renew_csr(&me, &next).is_err(),
            "a revoked cert cannot renew again"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn p12_is_modern_and_uses_the_default_password() {
        crate::install_crypto_provider();
        let (pki, dir) = temp_pki("p12");
        let (der, meta) = pki.issue("bob", "user", "normal").unwrap();
        let path = dir.join("bob.p12");
        fs::write(&path, &der).unwrap();
        if let Ok(out) = std::env::var("BIA_P12_OUT") {
            fs::write(out, &der).unwrap();
        }
        // OpenSSL 3 without -legacy: must open with the password, refuse a wrong one
        let run = |pass: &str, extra: &[&str]| {
            std::process::Command::new("openssl")
                .args(["pkcs12", "-in"])
                .arg(&path)
                .args(["-passin", &format!("pass:{pass}")])
                .args(extra)
                .output()
        };
        let Ok(info) = run(crate::P12_PASSWORD, &["-noout", "-info"]) else {
            eprintln!("no openssl; skipping the cross-check");
            let _ = fs::remove_dir_all(dir);
            return;
        };
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&info.stdout),
            String::from_utf8_lossy(&info.stderr)
        );
        assert!(info.status.success(), "{text}");
        assert!(
            text.contains("MAC: sha256") && text.contains("PBES2, PBKDF2, AES-256-CBC"),
            "{text}"
        );
        assert!(!text.contains("RC2") && !text.contains("3DES"), "{text}");
        assert!(!run("wrong", &["-noout"]).unwrap().status.success());
        let pem = run(crate::P12_PASSWORD, &["-nodes"]).unwrap();
        let pem = String::from_utf8_lossy(&pem.stdout).to_string();
        assert!(
            pem.contains("PRIVATE KEY") && pem.matches("BEGIN CERTIFICATE").count() == 2,
            "{pem}"
        );
        assert!(
            pem.contains("Identity: bob")
                && pem.contains(&format!("friendlyName: {}", meta.identity)),
            "{pem}"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn weak_rsa_refused() {
        // 1024-bit RSA CSR made with: openssl req -new -newkey rsa:1024 -nodes -subj /CN=x
        let weak = include_str!("../testdata/rsa1024.csr");
        let e = match crate::check_csr(weak) {
            Ok(_) => panic!("1024-bit RSA accepted"),
            Err(e) => e.to_string(),
        };
        assert!(e.contains("2048"), "{e}");
    }

    #[test]
    fn deny_expire_and_limits() {
        let (pki, dir) = temp_pki("limits");
        let (pem, _) = csr("x");
        let a = pki
            .create_request("a1", "agent", "", "9.9.9.9", &pem)
            .unwrap();
        pki.decide_request(&a.id, false, "root", "no").unwrap();
        assert_eq!(pki.request(&a.id).unwrap().status, Status::Denied);
        assert!(pki.collect_request(&a.id).is_err());
        for i in 0..3 {
            pki.create_request(&format!("b{i}"), "agent", "", "5.5.5.5", &pem)
                .unwrap();
        }
        assert!(
            pki.create_request("b3", "agent", "", "5.5.5.5", &pem)
                .is_err(),
            "per-source cap"
        );
        let mut old = pki
            .create_request("c", "agent", "", "6.6.6.6", &pem)
            .unwrap();
        old.created = 1;
        pki.save_request(&old).unwrap();
        assert_eq!(pki.request(&old.id).unwrap().status, Status::Expired);
        let _ = fs::remove_dir_all(dir);
    }

    /// Client chain and key for `pki`, made the way an agent gets one (CSR).
    fn client_of(
        pki: &Pki,
        id: &str,
    ) -> (
        Vec<rustls::pki_types::CertificateDer<'static>>,
        rcgen::KeyPair,
        String,
    ) {
        let (pem, key) = csr(id);
        let (chain, meta) = pki.issue_csr(id, "user", "normal", &pem).unwrap();
        (crate::pem_to_cert_ders(&chain).unwrap(), key, meta.serial)
    }

    /// TLS handshake in memory between `server` and a client presenting `cert`.
    fn handshake(
        server: &Pki,
        cert: Option<(
            Vec<rustls::pki_types::CertificateDer<'static>>,
            &rcgen::KeyPair,
        )>,
    ) -> rustls::ServerConnection {
        use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer, ServerName};
        crate::install_crypto_provider();
        let ip: std::net::IpAddr = "127.0.0.1".parse().unwrap();
        server.ensure_server_sans(&[ip]).unwrap();
        let mut roots = rustls::RootCertStore::empty();
        for c in crate::pem_to_cert_ders(&server.ca_pem().unwrap()).unwrap() {
            roots.add(c).unwrap();
        }
        let b = rustls::ClientConfig::builder().with_root_certificates(roots);
        let cfg = match cert {
            Some((chain, key)) => {
                let der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der()));
                b.with_client_auth_cert(chain, der).unwrap()
            }
            None => b.with_no_client_auth(),
        };
        let mut c = rustls::ClientConnection::new(
            std::sync::Arc::new(cfg),
            ServerName::IpAddress(ip.into()),
        )
        .unwrap();
        let mut s = rustls::ServerConnection::new(server.server_config().unwrap()).unwrap();
        for _ in 0..20 {
            let mut buf = vec![];
            while c.wants_write() {
                c.write_tls(&mut buf).unwrap();
            }
            if !buf.is_empty() {
                s.read_tls(&mut buf.as_slice()).unwrap();
                s.process_new_packets()
                    .expect("server accepts the handshake");
            }
            let mut buf = vec![];
            while s.wants_write() {
                s.write_tls(&mut buf).unwrap();
            }
            if !buf.is_empty() {
                c.read_tls(&mut buf.as_slice()).unwrap();
                c.process_new_packets()
                    .expect("client accepts the handshake");
            }
            if !c.is_handshaking() && !s.is_handshaking() && !c.wants_write() {
                break;
            }
        }
        assert!(!s.is_handshaking(), "handshake did not finish");
        s
    }

    #[test]
    fn bound_certificate_works_only_from_its_address() {
        let (pki, dir) = temp_pki("bind");
        let (pem, key) = csr("x");
        let from = [crate::Cidr::parse("10.0.0.1").unwrap()];
        let (chain, meta) = pki.issue_csr_from("bob", "agent", "normal", &pem, &from).unwrap();
        assert_eq!(meta.from, vec!["10.0.0.1".to_string()]);
        let chain = crate::pem_to_cert_ders(&chain).unwrap();
        let s = handshake(&pki, Some((chain.clone(), &key)));
        let here: std::net::IpAddr = "10.0.0.1".parse().unwrap();
        let there: std::net::IpAddr = "10.0.0.2".parse().unwrap();
        assert_eq!(
            pki.check_client_from(s.peer_certificates(), Some(here)).unwrap().unwrap().identity,
            "bob"
        );
        assert_eq!(
            pki.check_client_from(s.peer_certificates(), Some(there)).unwrap_err().kind,
            "wrong_source"
        );
        assert_eq!(pki.check_client_from(s.peer_certificates(), None).unwrap_err().kind, "wrong_source");
        // renewal keeps the binding
        let me = crate::ClientId {
            identity: "bob".into(),
            role: "agent".into(),
            trust: "normal".into(),
            serial_hint: meta.serial.clone(),
            not_after: meta.not_after as i64,
        };
        let (next, _) = csr("y");
        let (_, renewed) = pki.renew_csr(&me, &next).unwrap();
        assert_eq!(renewed.from, vec!["10.0.0.1".to_string()]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn foreign_client_cert_passes_tls_and_is_refused_by_check_client() {
        let (a, dir_a) = temp_pki("cc-a");
        let (b, dir_b) = temp_pki("cc-b");

        let (own, own_key, own_serial) = client_of(&a, "alice");
        let s = handshake(&a, Some((own.clone(), &own_key)));
        let id = a.check_client(s.peer_certificates()).unwrap().unwrap();
        assert_eq!(id.identity, "alice");

        // another bia's certificate (same CA name): handshake succeeds, check_client says why
        let (foreign, foreign_key, _) = client_of(&b, "bob");
        let s = handshake(&a, Some((foreign, &foreign_key)));
        let p = a.check_client(s.peer_certificates()).unwrap_err();
        assert_eq!((p.kind, p.identity.as_str()), ("other_ca", "Identity: bob"));

        let s = handshake(&a, None);
        assert!(a.check_client(s.peer_certificates()).unwrap().is_none());

        assert!(a.revoke(&own_serial).unwrap());
        let s = handshake(&a, Some((own, &own_key)));
        assert_eq!(
            a.check_client(s.peer_certificates()).unwrap_err().kind,
            "revoked"
        );

        let _ = fs::remove_dir_all(dir_a);
        let _ = fs::remove_dir_all(dir_b);
    }
}
