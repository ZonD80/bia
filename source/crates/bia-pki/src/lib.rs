//! Agent-owned PKI: CA, TLS server cert, client certs, one-time PKCS#12 pickup.
use anyhow::{anyhow, bail, Context, Result};
use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair,
    KeyUsagePurpose, SanType,
};
use rustls::client::danger::HandshakeSignatureValid;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::server::WebPkiClientVerifier;
use rustls::{
    CertificateError, DigitallySignedStruct, DistinguishedName, RootCertStore, ServerConfig,
    SignatureScheme,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use time::{Duration, OffsetDateTime};

pub mod client;
mod net;
mod p12;
mod peers;
mod requests;
mod seed;
pub use net::{parse_ip, Cidr};
pub use peers::{Grant, Peer};
pub use requests::{CertRequest, Status as RequestStatus, POLL_SECS, REQUEST_TTL_SECS};
pub use seed::{seed, RootSeed, SeedOptions, Seeded};

const CA_NAME: &str = "bia agent CA";
/// Client cert CN prefix so leaf identities are distinct from the CA subject.
pub const IDENTITY_CN_PREFIX: &str = "Identity: ";
const PICKUP_TTL_SECS: i64 = 15 * 60;
/// Password of every PKCS#12 bia hands out. Not a secret (the file itself is the secret);
/// macOS refuses to import a .p12 with an empty password.
pub const P12_PASSWORD: &str = "12345678";
/// Server and client leaf lifetime.
pub const LEAF_DAYS: i64 = 825;
/// Reissue server cert this many days before `not_after`.
pub const SERVER_RENEW_WITHIN_DAYS: i64 = 30;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IssuedMeta {
    pub serial: String,
    pub identity: String,
    pub role: String,
    pub trust: String,
    pub created: u64,
    /// Unix seconds (`not_after`). 0 if unknown (older records).
    #[serde(default)]
    pub not_after: u64,
    pub revoked: bool,
    /// Networks the certificate works from (empty = anywhere). Checked per connection.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub from: Vec<String>,
}

pub struct Pki {
    pub dir: PathBuf,
}

impl Pki {
    pub fn dir_for(files: &Path) -> PathBuf {
        files.join("state").join("pki")
    }

    pub fn load_or_create(files: &Path) -> Result<Self> {
        let dir = Self::dir_for(files);
        fs::create_dir_all(dir.join("issued")).context("mkdir pki/issued")?;
        fs::create_dir_all(dir.join("pickup")).context("mkdir pki/pickup")?;
        let pki = Pki { dir };
        if !pki.ca_cert_path().exists() || !pki.ca_key_path().exists() {
            pki.bootstrap_ca()?;
        }
        if !pki.server_cert_path().exists() || !pki.server_key_path().exists() {
            pki.reissue_server(&[])?;
        }
        Ok(pki)
    }

    pub fn ca_cert_path(&self) -> PathBuf {
        self.dir.join("ca.crt")
    }
    pub fn ca_key_path(&self) -> PathBuf {
        self.dir.join("ca.key")
    }
    pub fn server_cert_path(&self) -> PathBuf {
        self.dir.join("server.crt")
    }
    pub fn server_key_path(&self) -> PathBuf {
        self.dir.join("server.key")
    }

    pub fn ca_pem(&self) -> Result<String> {
        fs::read_to_string(self.ca_cert_path()).context("read ca.crt")
    }

    pub fn ca_sha256(&self) -> Result<String> {
        let pem = self.ca_pem()?;
        let der = pem_to_cert_ders(&pem)?
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("empty CA"))?;
        Ok(hex_sha256(der.as_ref()))
    }

    fn bootstrap_ca(&self) -> Result<()> {
        let mut params = CertificateParams::new(Vec::<String>::new())?;
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.distinguished_name.push(DnType::CommonName, CA_NAME);
        params
            .distinguished_name
            .push(DnType::OrganizationName, "bia");
        params.key_usages = vec![
            KeyUsagePurpose::KeyCertSign,
            KeyUsagePurpose::CrlSign,
            KeyUsagePurpose::DigitalSignature,
        ];
        params.not_before = OffsetDateTime::now_utc() - Duration::days(1);
        params.not_after = OffsetDateTime::now_utc() + Duration::days(3650);
        let key = KeyPair::generate()?;
        let cert = params.self_signed(&key)?;
        fs::write(self.ca_cert_path(), cert.pem()).context("write ca.crt")?;
        fs::write(self.ca_key_path(), key.serialize_pem()).context("write ca.key")?;
        chmod600(&self.ca_key_path());
        Ok(())
    }

    /// CA certificate object (DN only; trust anchor remains ca.crt on disk) + key.
    fn load_ca_issuer(&self) -> Result<(rcgen::Certificate, KeyPair, String)> {
        let ca_pem = self.ca_pem()?;
        let key_pem = fs::read_to_string(self.ca_key_path()).context("read ca.key")?;
        let key = KeyPair::from_pem(&key_pem)?;
        let params = CertificateParams::from_ca_cert_pem(&ca_pem)?;
        // Reconstruct issuer handle for signed_by (same key + DN as on-disk CA).
        let issuer = params.self_signed(&key)?;
        Ok((issuer, key, ca_pem))
    }

    fn server_extra_path(&self) -> PathBuf {
        self.dir.join("server_extra.json")
    }

    /// Names and addresses the server certificate always carries besides the LAN ones
    /// (a public NAT address, a DNS name), from the deployment config or root.
    pub fn server_extra(&self) -> (Vec<String>, Vec<std::net::IpAddr>) {
        let v: serde_json::Value = fs::read(self.server_extra_path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let list = |k: &str| -> Vec<String> {
            v.get(k)
                .and_then(|x| x.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(|s| s.trim().to_string()))
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default()
        };
        let names = list("names")
            .into_iter()
            .filter(|n| valid_dns_name(n))
            .collect();
        let ips = list("ips")
            .into_iter()
            .filter_map(|i| parse_ip(&i).ok())
            .collect();
        (names, ips)
    }

    /// Replace the extra server names and addresses (takes effect at the next
    /// `ensure_server_sans`).
    pub fn set_server_extra(&self, names: &[String], ips: &[std::net::IpAddr]) -> Result<()> {
        for n in names {
            if !valid_dns_name(n) {
                bail!("{n:?} is not a DNS name");
            }
        }
        let v = serde_json::json!({
            "names": names,
            "ips": ips.iter().map(|i| i.to_string()).collect::<Vec<_>>(),
        });
        fs::write(self.server_extra_path(), serde_json::to_vec_pretty(&v)?)
            .context("write server_extra.json")?;
        Ok(())
    }

    /// Reissue the HTTPS server certificate. `extra_ips` are additional SANs (LAN IPs);
    /// the names and addresses of `server_extra` are always added.
    pub fn reissue_server(&self, extra_ips: &[std::net::IpAddr]) -> Result<()> {
        let (ca_cert, ca_key, _) = self.load_ca_issuer()?;
        let mut params = CertificateParams::new(vec!["bia".into(), "localhost".into()])?;
        params
            .distinguished_name
            .push(DnType::CommonName, "bia web");
        params
            .distinguished_name
            .push(DnType::OrganizationName, "bia");
        params.use_authority_key_identifier_extension = true;
        params.key_usages = vec![
            KeyUsagePurpose::DigitalSignature,
            KeyUsagePurpose::KeyEncipherment,
        ];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        params.not_before = OffsetDateTime::now_utc() - Duration::days(1);
        params.not_after = OffsetDateTime::now_utc() + Duration::days(LEAF_DAYS);
        let mut sans = vec![
            SanType::DnsName("bia".try_into().unwrap()),
            SanType::DnsName("localhost".try_into().unwrap()),
            SanType::IpAddress(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)),
        ];
        let (names, fixed) = self.server_extra();
        for n in names {
            if let Ok(n) = n.as_str().try_into() {
                sans.push(SanType::DnsName(n));
            }
        }
        let mut seen = std::collections::HashSet::new();
        for ip in extra_ips.iter().chain(fixed.iter()) {
            if seen.insert(*ip) && !ip.is_loopback() {
                sans.push(SanType::IpAddress(*ip));
            }
        }
        params.subject_alt_names = sans;
        let key = KeyPair::generate()?;
        let cert = params.signed_by(&key, &ca_cert, &ca_key)?;
        fs::write(self.server_cert_path(), cert.pem()).context("write server.crt")?;
        fs::write(self.server_key_path(), key.serialize_pem()).context("write server.key")?;
        chmod600(&self.server_key_path());
        Ok(())
    }

    /// Days until server `not_after` (negative if already expired). None if unreadable.
    pub fn server_days_left(&self) -> Option<i64> {
        let pem = fs::read_to_string(self.server_cert_path()).ok()?;
        let der = pem_to_cert_ders(&pem).ok()?.into_iter().next()?;
        let (_, cert) = x509_parser::parse_x509_certificate(der.as_ref()).ok()?;
        let not_after = cert.validity().not_after.timestamp();
        let now = OffsetDateTime::now_utc().unix_timestamp();
        Some((not_after - now) / 86_400)
    }

    /// Reissue when LAN SANs change or the server cert is within `SERVER_RENEW_WITHIN_DAYS` of expiry.
    pub fn ensure_server_sans(&self, ips: &[std::net::IpAddr]) -> Result<bool> {
        let mut want: Vec<String> = ips.iter().map(|i| i.to_string()).collect();
        // the fixed extras are part of what the current certificate must carry
        let (names, fixed) = self.server_extra();
        want.extend(names.iter().map(|n| format!("dns:{n}")));
        want.extend(fixed.iter().map(|i| format!("fixed:{i}")));
        let marker = self.dir.join("server.sans");
        let cur = fs::read_to_string(&marker).unwrap_or_default();
        let cur_list: Vec<&str> = cur.lines().filter(|l| !l.is_empty()).collect();
        let mut want_sorted = want.clone();
        want_sorted.sort();
        let mut cur_sorted: Vec<String> = cur_list.iter().map(|s| s.to_string()).collect();
        cur_sorted.sort();
        let sans_ok = want_sorted == cur_sorted && self.server_cert_path().exists();
        let needs_time_renew = match self.server_days_left() {
            Some(d) => d < SERVER_RENEW_WITHIN_DAYS,
            None => true,
        };
        if sans_ok && !needs_time_renew {
            return Ok(false);
        }
        self.reissue_server(ips)?;
        fs::write(&marker, want.join("\n") + "\n").ok();
        Ok(true)
    }

    /// Verifier of client certificates against this CA (chain and validity time).
    fn client_verifier(&self) -> Result<Arc<dyn ClientCertVerifier>> {
        let mut roots = RootCertStore::empty();
        let ca = pem_to_cert_ders(&self.ca_pem()?)?;
        for c in ca {
            roots
                .add(c)
                .map_err(|e| anyhow!("add CA to root store: {e}"))?;
        }
        WebPkiClientVerifier::builder(Arc::new(roots))
            .allow_unauthenticated()
            .build()
            .map_err(|e| anyhow!("client verifier: {e}"))
    }

    /// The handshake accepts any client certificate whose key the browser holds; the chain
    /// is checked per request by `check_client`. A browser that picked a certificate of
    /// another bia (same CA name) or an expired one then gets a page saying so instead of
    /// a bare TLS alert (ERR_SSL_DECRYPT_ERROR_ALERT), which it would keep hitting until
    /// it is restarted.
    pub fn server_config(&self) -> Result<Arc<ServerConfig>> {
        let cert_pem = fs::read(self.server_cert_path()).context("read server.crt")?;
        let key_pem = fs::read(self.server_key_path()).context("read server.key")?;
        let certs = pem_to_cert_ders(std::str::from_utf8(&cert_pem)?)?;
        let key = pem_to_key_der(std::str::from_utf8(&key_pem)?)?;

        let verifier = Arc::new(AnyClientCert(self.client_verifier()?));

        let mut cfg = ServerConfig::builder()
            .with_client_cert_verifier(verifier)
            .with_single_cert(certs, key)
            .map_err(|e| anyhow!("server config: {e}"))?;
        cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
        Ok(Arc::new(cfg))
    }

    /// Issue a client certificate with a key made here. Returns PKCS#12 bytes (password
    /// `P12_PASSWORD`) and metadata. For people importing into a browser; agents send a CSR
    /// instead (`issue_csr`) so their key never leaves them.
    pub fn issue(&self, identity: &str, role: &str, trust: &str) -> Result<(Vec<u8>, IssuedMeta)> {
        self.issue_from(identity, role, trust, &[])
    }

    /// `issue`, limited to connections from `from` (networks; empty = anywhere).
    pub fn issue_from(
        &self,
        identity: &str,
        role: &str,
        trust: &str,
        from: &[Cidr],
    ) -> Result<(Vec<u8>, IssuedMeta)> {
        let key = KeyPair::generate()?;
        let (cert, meta, ca_pem) = self.sign_client(identity, role, trust, &key, from)?;
        let ca_der = pem_to_cert_ders(&ca_pem)?
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("empty CA"))?;
        let key_der = key.serialize_der();
        let p12 = p12::build(
            cert.der().as_ref(),
            &key_der,
            &[ca_der.as_ref()],
            &meta.identity,
            P12_PASSWORD,
        )?;
        Ok((p12, meta))
    }

    /// Sign the public key of a verified CSR. Identity, role, trust and usages come from
    /// this side; the CSR's subject and extensions are ignored. Returns the certificate
    /// followed by the CA, as PEM.
    pub fn issue_csr(
        &self,
        identity: &str,
        role: &str,
        trust: &str,
        csr_pem: &str,
    ) -> Result<(String, IssuedMeta)> {
        self.issue_csr_from(identity, role, trust, csr_pem, &[])
    }

    /// `issue_csr`, limited to connections from `from` (empty = anywhere).
    pub fn issue_csr_from(
        &self,
        identity: &str,
        role: &str,
        trust: &str,
        csr_pem: &str,
        from: &[Cidr],
    ) -> Result<(String, IssuedMeta)> {
        let csr = check_csr(csr_pem)?;
        let (cert, meta, ca_pem) =
            self.sign_client(identity, role, trust, &csr.public_key, from)?;
        Ok((format!("{}{}", cert.pem(), ca_pem), meta))
    }

    fn sign_client(
        &self,
        identity: &str,
        role: &str,
        trust: &str,
        public_key: &impl rcgen::PublicKeyData,
        from: &[Cidr],
    ) -> Result<(rcgen::Certificate, IssuedMeta, String)> {
        let identity = sanitize_id(identity)?;
        let role = match role {
            "user" | "agent" => role,
            _ => bail!("role must be user or agent"),
        };
        let trust = match trust {
            "root" | "normal" => trust,
            _ => bail!("trust must be root or normal"),
        };
        let (ca_cert, ca_key, ca_pem) = self.load_ca_issuer()?;
        let cn = format!("{IDENTITY_CN_PREFIX}{identity}");
        let mut params = CertificateParams::new(Vec::<String>::new())?;
        params
            .distinguished_name
            .push(DnType::CommonName, cn.as_str());
        params
            .distinguished_name
            .push(DnType::OrganizationName, format!("bia:{role}"));
        params
            .distinguished_name
            .push(DnType::OrganizationalUnitName, format!("trust:{trust}"));
        params.use_authority_key_identifier_extension = true;
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        // Single addresses also go in as IP SANs so anyone looking at the certificate
        // sees where it works; the networks in the metadata are what is enforced.
        params.subject_alt_names = from
            .iter()
            .filter(|c| c.is_single())
            .map(|c| SanType::IpAddress(c.addr))
            .collect();
        params.not_before = OffsetDateTime::now_utc() - Duration::days(1);
        let not_after = OffsetDateTime::now_utc() + Duration::days(LEAF_DAYS);
        params.not_after = not_after;
        let cert = params.signed_by(public_key, &ca_cert, &ca_key)?;
        let serial = hex_sha256(cert.der().as_ref())[..16].to_string();
        let meta = IssuedMeta {
            serial: serial.clone(),
            identity: identity.clone(),
            role: role.to_string(),
            trust: trust.to_string(),
            created: now_secs(),
            not_after: not_after.unix_timestamp().max(0) as u64,
            revoked: false,
            from: from.iter().map(|c| c.to_string()).collect(),
        };
        let meta_path = self.dir.join("issued").join(format!("{serial}.json"));
        fs::write(
            &meta_path,
            serde_json::to_vec_pretty(&meta).context("serialize issued meta")?,
        )?;
        fs::write(
            self.dir.join("issued").join(format!("{serial}.crt")),
            cert.pem(),
        )?;
        Ok((cert, meta, ca_pem))
    }

    pub fn list_issued(&self) -> Result<Vec<IssuedMeta>> {
        let mut out = vec![];
        let rd = match fs::read_dir(self.dir.join("issued")) {
            Ok(r) => r,
            Err(_) => return Ok(out),
        };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !name.ends_with(".json") {
                continue;
            }
            let Ok(b) = fs::read(e.path()) else { continue };
            let Ok(m) = serde_json::from_slice::<IssuedMeta>(&b) else {
                continue;
            };
            out.push(m);
        }
        out.sort_by(|a, b| a.created.cmp(&b.created));
        Ok(out)
    }

    /// True when a non-revoked, not-yet-expired client cert exists for `identity`.
    pub fn has_active_identity(&self, identity: &str) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.list_issued()
            .unwrap_or_default()
            .into_iter()
            .any(|m| m.identity == identity && !m.revoked && m.not_after > now)
    }

    pub fn revoke(&self, serial: &str) -> Result<bool> {
        let serial = sanitize_serial(serial)?;
        let p = self.dir.join("issued").join(format!("{serial}.json"));
        let Ok(b) = fs::read(&p) else {
            return Ok(false);
        };
        let mut m: IssuedMeta = serde_json::from_slice(&b)?;
        m.revoked = true;
        fs::write(&p, serde_json::to_vec_pretty(&m)?)?;
        Ok(true)
    }

    /// Who the presented client certificate belongs to: `Ok(None)` when none was sent,
    /// `Err` when one was sent that this bia does not accept.
    pub fn check_client(
        &self,
        certs: Option<&[CertificateDer<'_>]>,
    ) -> std::result::Result<Option<ClientId>, CertProblem> {
        self.check_client_from(certs, None)
    }

    /// `check_client` for a connection from `peer`: a certificate limited to some
    /// networks (`IssuedMeta::from`) is refused from anywhere else ("wrong_source").
    pub fn check_client_from(
        &self,
        certs: Option<&[CertificateDer<'_>]>,
        peer: Option<std::net::IpAddr>,
    ) -> std::result::Result<Option<ClientId>, CertProblem> {
        let Some((end, rest)) = certs.and_then(|c| c.split_first()) else {
            return Ok(None);
        };
        let problem = |kind| CertProblem {
            kind,
            identity: presented_identity(end),
        };
        let verifier = self.client_verifier().map_err(|_| problem("invalid"))?;
        if let Err(e) = verifier.verify_client_cert(end, rest, UnixTime::now()) {
            return Err(problem(match e {
                rustls::Error::InvalidCertificate(
                    CertificateError::Expired | CertificateError::ExpiredContext { .. },
                ) => "expired",
                rustls::Error::InvalidCertificate(
                    CertificateError::UnknownIssuer | CertificateError::BadSignature,
                ) => "other_ca",
                _ => "invalid",
            }));
        }
        let id = client_id_from_certs(certs).ok_or_else(|| problem("invalid"))?;
        if self.is_revoked(&id.serial_hint) {
            return Err(problem("revoked"));
        }
        if let Some(m) = self.issued_meta(&id.serial_hint) {
            if !m.from.is_empty() {
                let ok = peer.is_some_and(|ip| {
                    m.from
                        .iter()
                        .filter_map(|c| Cidr::parse(c).ok())
                        .any(|c| c.contains(ip))
                });
                if !ok {
                    return Err(problem("wrong_source"));
                }
            }
        }
        Ok(Some(id))
    }

    /// Metadata of the issued certificate `serial_hint` names.
    pub fn issued_meta(&self, serial_hint: &str) -> Option<IssuedMeta> {
        self.list_issued()
            .unwrap_or_default()
            .into_iter()
            .find(|m| m.serial == serial_hint || serial_hint.starts_with(&m.serial))
    }

    pub fn is_revoked(&self, serial_hint: &str) -> bool {
        self.list_issued()
            .unwrap_or_default()
            .into_iter()
            .any(|m| m.revoked && (m.serial == serial_hint || serial_hint.starts_with(&m.serial)))
    }

    /// Store PKCS#12 for one-time HTTPS pickup. Returns the token.
    pub fn store_pickup(&self, p12: &[u8], identity: &str) -> Result<String> {
        let token = random_token(24);
        let path = self.dir.join("pickup").join(format!("{token}.p12"));
        fs::write(&path, p12).context("write pickup p12")?;
        chmod600(&path);
        let meta = serde_json::json!({
            "identity": identity,
            "created": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            "expires": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() + PICKUP_TTL_SECS as u64)
                .unwrap_or(0),
        });
        fs::write(
            self.dir.join("pickup").join(format!("{token}.json")),
            serde_json::to_vec_pretty(&meta)?,
        )?;
        Ok(token)
    }

    /// Take (and delete) a one-time PKCS#12. Returns None if missing/expired.
    pub fn take_pickup(&self, token: &str) -> Option<(Vec<u8>, String)> {
        let token = sanitize_token(token)?;
        let p12_path = self.dir.join("pickup").join(format!("{token}.p12"));
        let meta_path = self.dir.join("pickup").join(format!("{token}.json"));
        let meta: serde_json::Value = fs::read(&meta_path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let expires = meta.get("expires").and_then(|x| x.as_u64()).unwrap_or(0);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if expires > 0 && now > expires {
            let _ = fs::remove_file(&p12_path);
            let _ = fs::remove_file(&meta_path);
            return None;
        }
        let bytes = fs::read(&p12_path).ok()?;
        let identity = meta
            .get("identity")
            .and_then(|x| x.as_str())
            .unwrap_or("client")
            .to_string();
        let _ = fs::remove_file(&p12_path);
        let _ = fs::remove_file(&meta_path);
        Some((bytes, identity))
    }

    /// Issue + store pickup in one step.
    pub fn issue_pickup(
        &self,
        identity: &str,
        role: &str,
        trust: &str,
    ) -> Result<(String, IssuedMeta)> {
        self.issue_pickup_from(identity, role, trust, &[])
    }

    /// `issue_pickup`, limited to connections from `from` (empty = anywhere).
    pub fn issue_pickup_from(
        &self,
        identity: &str,
        role: &str,
        trust: &str,
        from: &[Cidr],
    ) -> Result<(String, IssuedMeta)> {
        let (p12, meta) = self.issue_from(identity, role, trust, from)?;
        let token = self.store_pickup(&p12, &meta.identity)?;
        Ok((token, meta))
    }

    /// Renew while the current cert is still valid: sign the key in `csr_pem` (made by
    /// the holder; no key is made here) for the same identity/role/trust, then revoke
    /// the presented serial. Returns the new certificate followed by the CA, as PEM.
    pub fn renew_csr(&self, current: &ClientId, csr_pem: &str) -> Result<(String, IssuedMeta)> {
        if self.is_revoked(&current.serial_hint) {
            bail!("certificate is revoked; ask root for a new one");
        }
        let now = OffsetDateTime::now_utc().unix_timestamp();
        if current.not_after <= now {
            bail!("certificate is expired; ask root for a new one");
        }
        // a certificate bound to some networks stays bound after renewal
        let from: Vec<Cidr> = self
            .issued_meta(&current.serial_hint)
            .map(|m| m.from.iter().filter_map(|c| Cidr::parse(c).ok()).collect())
            .unwrap_or_default();
        let (pem, meta) = self.issue_csr_from(
            &current.identity,
            &current.role,
            &current.trust,
            csr_pem,
            &from,
        )?;
        let _ = self.revoke(&current.serial_hint);
        Ok((pem, meta))
    }
}

/// A client certificate was presented but is not accepted here.
#[derive(Clone, Debug)]
pub struct CertProblem {
    /// "other_ca" (issued by another bia), "expired", "revoked" or "invalid"
    pub kind: &'static str,
    /// Subject CN of the presented certificate, e.g. "Identity: root"
    pub identity: String,
}

fn presented_identity(der: &CertificateDer<'_>) -> String {
    x509_parser::parse_x509_certificate(der.as_ref())
        .ok()
        .and_then(|(_, c)| {
            c.subject()
                .iter_common_name()
                .next()
                .and_then(|a| a.as_str().ok().map(str::to_string))
        })
        .unwrap_or_default()
}

/// Client verifier for the handshake: requests a certificate (with this CA as the hint, so
/// browsers offer the matching ones first) and checks that the peer holds its key, but
/// leaves the chain to `Pki::check_client`.
#[derive(Debug)]
struct AnyClientCert(Arc<dyn ClientCertVerifier>);

impl ClientCertVerifier for AnyClientCert {
    fn offer_client_auth(&self) -> bool {
        true
    }
    fn client_auth_mandatory(&self) -> bool {
        false
    }
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        self.0.root_hint_subjects()
    }
    fn verify_client_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> std::result::Result<ClientCertVerified, rustls::Error> {
        Ok(ClientCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        self.0.verify_tls12_signature(message, cert, dss)
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        self.0.verify_tls13_signature(message, cert, dss)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.supported_verify_schemes()
    }
}

/// Identity extracted from a verified client certificate.
#[derive(Clone, Debug)]
pub struct ClientId {
    pub identity: String,
    pub role: String,
    pub trust: String,
    pub serial_hint: String,
    /// Unix seconds (`not_after`).
    pub not_after: i64,
}

impl ClientId {
    pub fn days_left(&self) -> i64 {
        let now = OffsetDateTime::now_utc().unix_timestamp();
        (self.not_after - now) / 86_400
    }

    pub fn is_expired(&self) -> bool {
        self.not_after <= OffsetDateTime::now_utc().unix_timestamp()
    }
}

pub fn client_id_from_certs(certs: Option<&[CertificateDer<'_>]>) -> Option<ClientId> {
    let der = certs?.first()?;
    let (_, cert) = x509_parser::parse_x509_certificate(der.as_ref()).ok()?;
    let not_after = cert.validity().not_after.timestamp();
    let now = OffsetDateTime::now_utc().unix_timestamp();
    if not_after <= now {
        return None;
    }
    let subject = cert.subject();
    let mut identity = None;
    let mut role = "user".to_string();
    let mut trust = "normal".to_string();
    for attr in subject.iter_common_name() {
        if let Ok(s) = attr.as_str() {
            // Prefer "Identity: root" form; accept legacy bare CN ("root").
            identity = Some(
                s.strip_prefix(IDENTITY_CN_PREFIX)
                    .unwrap_or(s)
                    .trim()
                    .to_string(),
            );
            break;
        }
    }
    for attr in subject.iter_organization() {
        if let Ok(s) = attr.as_str() {
            if let Some(r) = s.strip_prefix("bia:") {
                role = r.to_string();
            }
        }
    }
    for attr in subject.iter_organizational_unit() {
        if let Ok(s) = attr.as_str() {
            if let Some(t) = s.strip_prefix("trust:") {
                trust = t.to_string();
            }
        }
    }
    let identity = identity?;
    let serial_hint = hex_sha256(der.as_ref())[..16].to_string();
    Some(ClientId {
        identity,
        role,
        trust,
        serial_hint,
        not_after,
    })
}

fn pem_to_cert_ders(pem: &str) -> Result<Vec<CertificateDer<'static>>> {
    let mut slice = pem.as_bytes();
    let mut out = Vec::new();
    for item in rustls_pemfile::certs(&mut slice) {
        out.push(item.context("parse cert pem")?);
    }
    if out.is_empty() {
        bail!("no certificates in PEM");
    }
    Ok(out)
}

fn pem_to_key_der(pem: &str) -> Result<PrivateKeyDer<'static>> {
    let mut slice = pem.as_bytes();
    let item = rustls_pemfile::private_key(&mut slice)
        .context("parse key pem")?
        .ok_or_else(|| anyhow!("no private key in PEM"))?;
    // Ensure we own 'static
    Ok(match item {
        PrivateKeyDer::Pkcs8(k) => {
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(k.secret_pkcs8_der().to_vec()))
        }
        other => other.clone_key(),
    })
}

fn hex_sha256(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn random_token(n_bytes: usize) -> String {
    let mut b = vec![0u8; n_bytes];
    if let Ok(mut f) = fs::File::open("/dev/urandom") {
        use std::io::Read;
        let _ = f.read_exact(&mut b);
    } else {
        for (i, x) in b.iter_mut().enumerate() {
            *x = (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u8)
                .unwrap_or(1))
            .wrapping_add(i as u8);
        }
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Parse a PEM CSR, verify its self-signature (the sender holds the key) and refuse
/// weak keys: RSA below 2048 bits. EC P-256/P-384 and Ed25519 are fine.
pub fn check_csr(pem: &str) -> Result<rcgen::CertificateSigningRequestParams> {
    use x509_parser::prelude::FromDer;
    let pem = pem.trim();
    if pem.is_empty() {
        bail!("csr is required: a PEM certificate signing request for your own key");
    }
    if pem.len() > 16 * 1024 {
        bail!("csr is too large");
    }
    let der = rustls_pemfile::csr(&mut pem.as_bytes())
        .ok()
        .flatten()
        .ok_or_else(|| anyhow!("csr: not PEM"))?;
    let (_, req) =
        x509_parser::certification_request::X509CertificationRequest::from_der(der.as_ref())
            .map_err(|_| anyhow!("csr: cannot parse"))?;
    if let Ok(x509_parser::public_key::PublicKey::RSA(rsa)) =
        req.certification_request_info.subject_pki.parsed()
    {
        if rsa.key_size() < 2048 {
            bail!(
                "csr: RSA key is {} bits; use at least 2048 (or EC P-256)",
                rsa.key_size()
            );
        }
    }
    rcgen::CertificateSigningRequestParams::from_pem(pem)
        .map_err(|e| anyhow!("csr: not a valid signed PEM certificate request ({e})"))
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn sanitize_id(s: &str) -> Result<String> {
    let s = s.trim();
    if s.is_empty() || s.len() > 64 {
        bail!("identity must be 1..64 chars");
    }
    if !s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '@'))
    {
        bail!("identity has invalid characters");
    }
    Ok(s.to_string())
}

fn valid_dns_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 253
        && n.split('.').all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '*')
        })
}

fn sanitize_serial(s: &str) -> Result<String> {
    let s = s.trim();
    if s.len() < 8 || s.len() > 64 || !s.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("bad serial");
    }
    Ok(s.to_lowercase())
}

fn sanitize_token(s: &str) -> Option<String> {
    let s = s.trim();
    if (16..=128).contains(&s.len()) && s.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(s.to_lowercase())
    } else {
        None
    }
}

fn chmod600(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
}

/// Install the process-wide rustls crypto provider (call once at startup).
pub fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}
