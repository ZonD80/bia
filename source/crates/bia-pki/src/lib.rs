//! Agent-owned PKI: CA, TLS server cert, client certs, one-time PKCS#12 pickup.
use anyhow::{anyhow, bail, Context, Result};
use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair,
    KeyUsagePurpose, SanType,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, UnixTime};
use rustls::server::danger::ClientCertVerifier;
use rustls::server::WebPkiClientVerifier;
use rustls::{CertificateError, RootCertStore, ServerConfig};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use time::{Duration, OffsetDateTime};

pub mod biaconfig;
pub mod client;
pub mod signin;
mod net;
mod p12;
mod peers;
mod requests;
mod seed;
pub use net::{parse_ip, Cidr};
pub use peers::{Grant, Peer};
pub use requests::{CertRequest, Status as RequestStatus, POLL_SECS, REQUEST_TTL_SECS};
pub use seed::{seed, RootSeed, SeedOptions, Seeded};

/// Client cert CN prefix so leaf identities are distinct from the CA subject.
pub const IDENTITY_CN_PREFIX: &str = "Identity: ";
/// Since 2.1.0 a client certificate's CN also names the bia that issued it
/// ("Identity: root · Fred"); the identity is the part before this.
pub const ISSUER_CN_SEP: &str = " · ";

/// The identity in a client certificate's CN: "Identity: root · Fred", "Identity: root" and
/// a legacy bare "root" all give "root".
pub fn identity_from_cn(cn: &str) -> String {
    let s = cn.strip_prefix(IDENTITY_CN_PREFIX).unwrap_or(cn);
    s.split(ISSUER_CN_SEP).next().unwrap_or("").trim().to_string()
}
const PICKUP_TTL_SECS: i64 = 15 * 60;
/// Password of every PKCS#12 bia hands out. Not a secret (the file itself is the secret);
/// macOS refuses to import a .p12 with an empty password.
pub const P12_PASSWORD: &str = "12345678";
/// Server and client leaf lifetime, from now. With the day of backdating below that makes 825
/// days in all, the most Apple's TLS accepts: one more and iOS and macOS call the server
/// certificate "not standards compliant", even under a trusted CA.
pub const LEAF_DAYS: i64 = 824;
/// The whole validity Apple's TLS allows a server certificate.
const SERVER_MAX_DAYS: i64 = 825;
/// Reissue server cert this many days before `not_after`.
pub const SERVER_RENEW_WITHIN_DAYS: i64 = 30;
/// What answers at a server address (`Pki::probe_server_ip`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpProbe {
    /// this bia (its CA in the presented chain)
    Ours,
    /// another bia: a bia server certificate under another CA
    OtherBia,
    /// something else, or nothing (refused, timeout): proves nothing, a router may forward
    /// another port to this bia
    Unknown,
}

/// Ports tried at a fixed address besides the one this bia listens on.
const PROBE_PORTS: [u16; 2] = [443, 8443];

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
    /// 0600 for a key or PKCS#12 this (root) process wrote, owned like the files folder.
    fn private(&self, path: &Path) {
        chmod600(path);
        if let Some(files) = self.dir.parent().and_then(Path::parent) {
            own_like_files(path, files);
        }
    }

    /// While nobody holds root's certificate: the one-time code that lets the first visitor
    /// set root up (shown only on this machine's own console), made when first asked for.
    /// None once root exists (the code is deleted then).
    pub fn setup_code(&self) -> Option<String> {
        let path = self.dir.join("setup-code");
        if self.has_active_identity("root") {
            let _ = fs::remove_file(&path);
            return None;
        }
        if let Some(c) = fs::read_to_string(&path).ok().map(|c| c.trim().to_string()).filter(|c| c.len() >= 8) {
            return Some(c);
        }
        // 8 letters a person reads off a screen, about 40 bits (wrong tries are rate-limited)
        const LETTERS: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
        let mut b = [0u8; 8];
        ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut b).ok()?;
        let s: String = b.iter().map(|x| LETTERS[*x as usize % LETTERS.len()] as char).collect();
        let code = format!("{}-{}", &s[..4], &s[4..]);
        fs::write(&path, &code).ok()?;
        self.private(&path);
        Some(code)
    }

    /// Whether `given` is the setup code (spaces, dashes and case do not matter).
    pub fn check_setup_code(&self, given: &str) -> bool {
        let norm = |s: &str| s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
        match self.setup_code() {
            Some(c) => {
                // equal-length digests compared without an early exit
                let (a, b) = (Sha256::digest(norm(&c).as_bytes()), Sha256::digest(norm(given).as_bytes()));
                a.iter().zip(b.iter()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
            }
            None => false,
        }
    }

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
        } else if !pki.server_self_signed() {
            // before 2.3.0 the CA signed it: same names, now self-signed
            let ips = pki.server_ips();
            pki.reissue_server(&ips)?;
        }
        Ok(pki)
    }

    /// Whether server.crt is its own issuer.
    fn server_self_signed(&self) -> bool {
        fs::read_to_string(self.server_cert_path())
            .ok()
            .and_then(|p| pem_to_cert_ders(&p).ok())
            .and_then(|d| d.into_iter().next())
            .and_then(|der| {
                let (_, c) = x509_parser::parse_x509_certificate(der.as_ref()).ok()?;
                Some(c.issuer() == c.subject())
            })
            .unwrap_or(false)
    }

    /// The IP addresses server.crt carries now.
    fn server_ips(&self) -> Vec<std::net::IpAddr> {
        fs::read_to_string(self.server_cert_path())
            .ok()
            .and_then(|p| pem_to_cert_ders(&p).ok())
            .and_then(|d| d.into_iter().next())
            .map(|der| {
                let Ok((_, c)) = x509_parser::parse_x509_certificate(der.as_ref()) else { return vec![] };
                c.subject_alternative_name()
                    .ok()
                    .flatten()
                    .map(|san| {
                        san.value
                            .general_names
                            .iter()
                            .filter_map(|g| match g {
                                x509_parser::extensions::GeneralName::IPAddress(b) => match b.len() {
                                    4 => Some(std::net::IpAddr::from(<[u8; 4]>::try_from(*b).ok()?)),
                                    16 => Some(std::net::IpAddr::from(<[u8; 16]>::try_from(*b).ok()?)),
                                    _ => None,
                                },
                                _ => None,
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            })
            .unwrap_or_default()
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

    /// The CA's subject CN: "bia-5a3f2c agent CA" (since 2.1.0) or "bia agent CA" (older).
    pub fn ca_name(&self) -> String {
        self.ca_pem()
            .ok()
            .and_then(|pem| pem_to_cert_ders(&pem).ok())
            .and_then(|d| d.into_iter().next())
            .and_then(|der| {
                let (_, c) = x509_parser::parse_x509_certificate(der.as_ref()).ok()?;
                let cn = c.subject().iter_common_name().next()?.as_str().ok()?.to_string();
                Some(cn)
            })
            .unwrap_or_else(|| "bia agent CA".into())
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
        // The bia id is chosen before the CA exists (a certificate cannot hold its own
        // fingerprint), from the CA key, and kept in agent.json, so the CA can carry it.
        let key = KeyPair::generate()?;
        let id = match self.stored_id() {
            Some(id) => id,
            None => {
                let id = format!("bia-{}", &hex_sha256(&key.public_key_der())[..6]);
                self.write_agent_json(|v| v["id"] = id.clone().into())?;
                id
            }
        };
        let mut params = CertificateParams::new(Vec::<String>::new())?;
        // It only ever signs client identities (since 2.3.0): no CAs below it, and
        // nothing it signs can serve TLS even where someone trusted it as a root.
        params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        params.distinguished_name.push(DnType::CommonName, format!("{id} agent CA"));
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
        let cert = params.self_signed(&key)?;
        fs::write(self.ca_cert_path(), cert.pem()).context("write ca.crt")?;
        fs::write(self.ca_key_path(), key.serialize_pem()).context("write ca.key")?;
        self.private(&self.ca_key_path());
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
        let mut params = CertificateParams::new(vec!["bia".into(), "localhost".into()])?;
        params
            .distinguished_name
            .push(DnType::CommonName, format!("{}{ISSUER_CN_SEP}bia web", self.display_name()));
        params
            .distinguished_name
            .push(DnType::OrganizationName, "bia");
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
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
        // Self-signed (since 2.3.0), not by the CA: the CA only signs client identities.
        // The key stays across reissues, so a browser that accepted it once is not asked
        // again for every new address; apps and bias check the bia by `bind`, not by this.
        let key = fs::read_to_string(self.server_key_path())
            .ok()
            .and_then(|p| KeyPair::from_pem(&p).ok())
            .map_or_else(KeyPair::generate, Ok)?;
        let cert = params.self_signed(&key)?;
        fs::write(self.server_cert_path(), cert.pem()).context("write server.crt")?;
        fs::write(self.server_key_path(), key.serialize_pem()).context("write server.key")?;
        self.private(&self.server_key_path());
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

    /// Whether the server certificate is valid for longer than Apple's TLS accepts: bias before
    /// 2.0.0 issued one a day too long.
    fn server_too_long(&self) -> bool {
        let Ok(pem) = fs::read_to_string(self.server_cert_path()) else { return false };
        let Some(der) = pem_to_cert_ders(&pem).ok().and_then(|d| d.into_iter().next()) else { return false };
        let Ok((_, cert)) = x509_parser::parse_x509_certificate(der.as_ref()) else { return false };
        let v = cert.validity();
        (v.not_after.timestamp() - v.not_before.timestamp()) / 86_400 > SERVER_MAX_DAYS
    }

    /// Reissue when LAN SANs change, the server cert is within `SERVER_RENEW_WITHIN_DAYS` of expiry,
    /// or it is valid for longer than Apple's TLS accepts.
    pub fn ensure_server_sans(&self, ips: &[std::net::IpAddr]) -> Result<bool> {
        let mut want: Vec<String> = ips.iter().map(|i| i.to_string()).collect();
        // the fixed extras are part of what the current certificate must carry
        let (names, fixed) = self.server_extra();
        want.extend(names.iter().map(|n| format!("dns:{n}")));
        want.extend(fixed.iter().map(|i| format!("fixed:{i}")));
        // a new name for this bia goes into the server certificate
        want.push(format!("name:{}", self.display_name()));
        let marker = self.dir.join("server.sans");
        let cur = fs::read_to_string(&marker).unwrap_or_default();
        let cur_list: Vec<&str> = cur.lines().filter(|l| !l.is_empty()).collect();
        let mut want_sorted = want.clone();
        want_sorted.sort();
        let mut cur_sorted: Vec<String> = cur_list.iter().map(|s| s.to_string()).collect();
        cur_sorted.sort();
        let sans_ok = want_sorted == cur_sorted && self.server_cert_path().exists();
        let needs_time_renew = match self.server_days_left() {
            Some(d) => d < SERVER_RENEW_WITHIN_DAYS || self.server_too_long(),
            None => true,
        };
        if sans_ok && !needs_time_renew {
            return Ok(false);
        }
        self.reissue_server(ips)?;
        fs::write(&marker, want.join("\n") + "\n").ok();
        Ok(true)
    }

    /// What answers at `ip:port`: this bia, another bia, or something unknown.
    pub fn probe_server_ip(&self, ip: std::net::IpAddr, port: u16) -> IpProbe {
        let url = match ip {
            std::net::IpAddr::V6(v6) => format!("https://[{v6}]:{port}"),
            std::net::IpAddr::V4(v4) => format!("https://{v4}:{port}"),
        };
        let Ok(ours) = fs::read_to_string(self.server_cert_path())
            .map_err(anyhow::Error::from)
            .and_then(|p| pem_to_cert_ders(&p))
        else {
            return IpProbe::Unknown;
        };
        let Ok((conn, _)) = client::connect(&url) else { return IpProbe::Unknown };
        let chain = conn.peer_certificates().unwrap_or_default();
        if chain.iter().any(|c| ours.iter().any(|o| o.as_ref() == c.as_ref())) {
            return IpProbe::Ours;
        }
        let bia_leaf = chain.first().and_then(|d| {
            let (_, c) = x509_parser::parse_x509_certificate(d.as_ref()).ok()?;
            let org = c.subject().iter_organization().next()?.as_str().ok()?.to_string();
            Some(org == "bia")
        });
        if bia_leaf == Some(true) { IpProbe::OtherBia } else { IpProbe::Unknown }
    }

    /// Drop the fixed extra addresses (`server_extra.json` "ips") where another bia lives:
    /// one answers on `port`, 443 or 8443 there, and none of them leads to this bia.
    /// Addresses on this machine (`own`) are not probed; an address where nothing or
    /// something else answers is kept (it may be a NAT that forwards another port here).
    /// Returns what was removed; the next `ensure_server_sans` reissues without it.
    pub fn prune_server_ips(&self, port: u16, own: &[std::net::IpAddr]) -> Vec<std::net::IpAddr> {
        self.prune_server_ips_with(port, own, |ip, p| self.probe_server_ip(ip, p))
    }

    fn prune_server_ips_with(
        &self,
        port: u16,
        own: &[std::net::IpAddr],
        probe: impl Fn(std::net::IpAddr, u16) -> IpProbe,
    ) -> Vec<std::net::IpAddr> {
        let (names, ips) = self.server_extra();
        let mut gone = vec![];
        let mut ports = vec![port];
        ports.extend(PROBE_PORTS.iter().filter(|p| **p != port));
        for ip in &ips {
            if own.contains(ip) || ip.is_loopback() {
                continue;
            }
            let seen: Vec<IpProbe> = ports.iter().map(|p| probe(*ip, *p)).collect();
            if seen.contains(&IpProbe::OtherBia) && !seen.contains(&IpProbe::Ours) {
                gone.push(*ip);
            }
        }
        if !gone.is_empty() {
            let keep: Vec<std::net::IpAddr> = ips.into_iter().filter(|i| !gone.contains(i)).collect();
            if self.set_server_extra(&names, &keep).is_err() {
                return vec![];
            }
        }
        gone
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

    /// This bia's own self-signed server certificate (every LAN address in it), ready for
    /// rustls. The fallback for every address without a Let's Encrypt certificate.
    pub fn own_certified_key(&self) -> Result<Arc<rustls::sign::CertifiedKey>> {
        let cert_pem = fs::read(self.server_cert_path()).context("read server.crt")?;
        let key_pem = fs::read(self.server_key_path()).context("read server.key")?;
        let certs = pem_to_cert_ders(std::str::from_utf8(&cert_pem)?)?;
        let key = pem_to_key_der(std::str::from_utf8(&key_pem)?)?;
        certified_key(certs, &key)
    }

    /// TLS settings with bia's own certificate for every connection.
    pub fn server_config(&self) -> Result<Arc<ServerConfig>> {
        let own = self.own_certified_key()?;
        self.server_config_with(Arc::new(FixedCert(own)))
    }

    /// `server_config` with the server certificate chosen by `resolver` (per SNI or address).
    pub fn server_config_with(
        &self,
        resolver: Arc<dyn rustls::server::ResolvesServerCert>,
    ) -> Result<Arc<ServerConfig>> {
        // No client certificates: who is talking is proved inside the app (signin.rs).
        // TLS 1.3 only, so the channel binding exporter (RFC 9266) is defined.
        let mut cfg = ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
            .with_no_client_auth()
            .with_cert_resolver(resolver);
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
        let cn = format!("{IDENTITY_CN_PREFIX}{identity}{ISSUER_CN_SEP}{}", self.display_name());
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

    /// Identities with an active (unrevoked, unexpired) certificate of trust root.
    pub fn root_identities(&self) -> Vec<String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut ids: Vec<String> = self
            .list_issued()
            .unwrap_or_default()
            .into_iter()
            .filter(|m| m.trust == "root" && !m.revoked && m.not_after > now)
            .map(|m| m.identity)
            .collect();
        ids.sort();
        ids.dedup();
        ids
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
        self.private(&path);
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

    /// Another certificate for `current`'s identity, role, trust and networks on a new key
    /// (one per browser or phone): `current` stays valid. Not for another bia's
    /// certificate, whose peer record holds one serial.
    pub fn enroll_csr(&self, current: &ClientId, csr_pem: &str) -> Result<(String, IssuedMeta)> {
        if self.is_revoked(&current.serial_hint) {
            bail!("certificate is revoked; ask root for a new one");
        }
        if current.not_after <= OffsetDateTime::now_utc().unix_timestamp() {
            bail!("certificate is expired; ask root for a new one");
        }
        if self.is_peer_cert(current).is_some() {
            bail!("another bia's certificate is renewed, not enrolled");
        }
        let from: Vec<Cidr> = self
            .issued_meta(&current.serial_hint)
            .map(|m| m.from.iter().filter_map(|c| Cidr::parse(c).ok()).collect())
            .unwrap_or_default();
        self.issue_csr_from(&current.identity, &current.role, &current.trust, csr_pem, &from)
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
            // "Identity: root · Fred", "Identity: root" or a legacy bare CN ("root")
            identity = Some(identity_from_cn(s));
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

/// Always the same server certificate.
#[derive(Debug)]
pub struct FixedCert(pub Arc<rustls::sign::CertifiedKey>);

impl rustls::server::ResolvesServerCert for FixedCert {
    fn resolve(&self, _: rustls::server::ClientHello<'_>) -> Option<Arc<rustls::sign::CertifiedKey>> {
        Some(self.0.clone())
    }
}

/// A rustls certified key from a DER chain (leaf first) and its private key.
pub fn certified_key(
    certs: Vec<CertificateDer<'static>>,
    key: &PrivateKeyDer<'_>,
) -> Result<Arc<rustls::sign::CertifiedKey>> {
    let signer = rustls::crypto::ring::sign::any_supported_type(key)
        .map_err(|e| anyhow!("server key: {e}"))?;
    Ok(Arc::new(rustls::sign::CertifiedKey::new(certs, signer)))
}

pub fn pem_to_cert_ders(pem: &str) -> Result<Vec<CertificateDer<'static>>> {
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

pub fn pem_to_key_der(pem: &str) -> Result<PrivateKeyDer<'static>> {
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

/// A new P-256 key and a CSR for it (`CN` = identity), both PEM: for a client that asks a
/// bia for a certificate (bia-bridge). The key never leaves the caller.
pub fn new_key_csr(identity: &str) -> Result<(String, String)> {
    let key = KeyPair::generate()?;
    let mut p = CertificateParams::new(Vec::<String>::new())?;
    p.distinguished_name.push(DnType::CommonName, identity);
    let csr = p.serialize_request(&key)?.pem()?;
    Ok((key.serialize_pem(), csr))
}

/// A .p12 (bia's own layout: PBES2/AES-256, SHA-256 MAC) → (key PEM, certificates PEM in
/// order: the client certificate first). For clients importing a certificate they have.
pub fn read_p12(p12: &[u8], password: &str) -> Result<(String, Vec<String>)> {
    let (key, certs) = p12::parse(p12, password)?;
    let pem = |label: &str, der: &[u8]| {
        use base64::Engine as _;
        let b = base64::engine::general_purpose::STANDARD.encode(der);
        let body: Vec<&str> = b.as_bytes().chunks(64).map(|c| std::str::from_utf8(c).unwrap_or("")).collect();
        format!("-----BEGIN {label}-----\n{}\n-----END {label}-----\n", body.join("\n"))
    };
    Ok((pem("PRIVATE KEY", &key), certs.iter().map(|c| pem("CERTIFICATE", c)).collect()))
}

/// Write `data` readable by the owner only (keys).
pub fn write_private(path: &Path, data: &[u8]) -> Result<()> {
    fs::write(path, data).with_context(|| format!("write {}", path.display()))?;
    chmod600(path);
    Ok(())
}

fn chmod600(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// Root writes it here, but backups made as user bia must read it: hand it to the owner of
/// `files` (the agent gives all of /home/bia to bia at boot anyway).
fn own_like_files(p: &Path, files: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let Ok(m) = std::fs::metadata(files) {
            let _ = std::os::unix::fs::chown(p, Some(m.uid()), Some(m.gid()));
        }
    }
    #[cfg(not(unix))]
    let _ = (p, files);
}

/// Install the process-wide rustls crypto provider (call once at startup).
pub fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

#[cfg(test)]
mod server_cert_tests {
    use super::*;

    fn validity_days(pki: &Pki) -> i64 {
        let pem = fs::read_to_string(pki.server_cert_path()).unwrap();
        let der = pem_to_cert_ders(&pem).unwrap().into_iter().next().unwrap();
        let (_, cert) = x509_parser::parse_x509_certificate(der.as_ref()).unwrap();
        let v = cert.validity();
        (v.not_after.timestamp() - v.not_before.timestamp()) / 86_400
    }

    #[test]
    fn a_server_certificate_stays_within_what_apple_accepts_and_an_old_one_is_replaced() {
        install_crypto_provider();
        let dir = std::env::temp_dir().join(format!("bia-pki-days-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let pki = Pki::load_or_create(&dir).unwrap();
        pki.ensure_server_sans(&[]).unwrap();
        assert!(validity_days(&pki) <= SERVER_MAX_DAYS, "{}", validity_days(&pki));
        assert!(!pki.ensure_server_sans(&[]).unwrap(), "a good one stays");
        // what a bia before 2.0.0 has: 826 days
        let (ca, key, _) = pki.load_ca_issuer().unwrap();
        let mut params = CertificateParams::new(vec!["bia".into()]).unwrap();
        params.not_before = OffsetDateTime::now_utc() - Duration::days(1);
        params.not_after = OffsetDateTime::now_utc() + Duration::days(825);
        let leaf = KeyPair::generate().unwrap();
        fs::write(pki.server_cert_path(), params.signed_by(&leaf, &ca, &key).unwrap().pem()).unwrap();
        assert_eq!(validity_days(&pki), 826);
        assert!(pki.ensure_server_sans(&[]).unwrap(), "it is reissued");
        assert!(validity_days(&pki) <= SERVER_MAX_DAYS);
        let _ = fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod naming_tests {
    use super::*;

    fn cn_of(pem: &str) -> (String, String) {
        let der = pem_to_cert_ders(pem).unwrap().into_iter().next().unwrap();
        let (_, c) = x509_parser::parse_x509_certificate(der.as_ref()).unwrap();
        let cn = |n: &x509_parser::x509::X509Name| n.iter_common_name().next().unwrap().as_str().unwrap().to_string();
        (cn(c.subject()), cn(c.issuer()))
    }

    fn fresh(tag: &str) -> (PathBuf, Pki) {
        install_crypto_provider();
        let dir = std::env::temp_dir().join(format!("bia-pki-name-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let pki = Pki::load_or_create(&dir).unwrap();
        (dir, pki)
    }

    #[test]
    fn a_new_ca_carries_the_bia_id_which_comes_from_its_key() {
        let (dir, pki) = fresh("ca");
        let id = pki.bia_id();
        assert!(id.starts_with("bia-") && id.len() == 10, "{id}");
        assert_eq!(pki.agent_name(), id);
        let (subject, _) = cn_of(&pki.ca_pem().unwrap());
        assert_eq!(subject, format!("{id} agent CA"));
        assert_eq!(pki.ca_name(), subject);
        // recomputable from the CA certificate: sha256 of its public key
        let der = pem_to_cert_ders(&pki.ca_pem().unwrap()).unwrap().into_iter().next().unwrap();
        let (_, c) = x509_parser::parse_x509_certificate(der.as_ref()).unwrap();
        assert_eq!(&id[4..], &hex_sha256(c.public_key().raw)[..6]);
        // a peer name set later keeps the id
        pki.set_agent_name("fred").unwrap();
        assert_eq!((pki.agent_name().as_str(), pki.bia_id()), ("fred", id));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_old_ca_keeps_its_fingerprint_id() {
        let (dir, pki) = fresh("old");
        // a bia from before 2.1.0: no id in agent.json
        fs::remove_file(pki.dir.join("agent.json")).unwrap();
        assert_eq!(pki.bia_id(), format!("bia-{}", &pki.ca_sha256().unwrap()[..6]));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn certificates_name_the_bia_and_the_identity_still_reads() {
        let (dir, pki) = fresh("certs");
        let id = pki.bia_id();
        assert_eq!(cn_of(&fs::read_to_string(pki.server_cert_path()).unwrap()).0, format!("{id} · bia web"));
        fs::write(dir.join("state/self.json"), r#"{"name": "Фред", "gender": "male"}"#).unwrap();
        assert_eq!(pki.display_name(), "Фред");
        assert!(pki.ensure_server_sans(&[]).unwrap(), "a new name reissues the server certificate");
        assert_eq!(cn_of(&fs::read_to_string(pki.server_cert_path()).unwrap()).0, "Фред · bia web");
        let key = KeyPair::generate().unwrap();
        let (cert, _, _) = pki.sign_client("root", "user", "root", &key, &[]).unwrap();
        let (subject, issuer) = cn_of(&cert.pem());
        assert_eq!(subject, "Identity: root · Фред");
        assert_eq!(issuer, format!("{id} agent CA"));
        let der = cert.der().clone();
        assert_eq!(client_id_from_certs(Some(&[der])).unwrap().identity, "root");
        for (cn, want) in [("Identity: root · Фред", "root"), ("Identity: root", "root"), ("root", "root"), ("Identity: a@b.c · x · y", "a@b.c")] {
            assert_eq!(identity_from_cn(cn), want);
        }
        let _ = fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod server_ip_tests {
    use super::*;
    use std::net::IpAddr;

    fn fresh(tag: &str) -> (PathBuf, Pki) {
        install_crypto_provider();
        let dir = std::env::temp_dir().join(format!("bia-pki-ips-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let pki = Pki::load_or_create(&dir).unwrap();
        (dir, pki)
    }

    /// A TLS listener on 127.0.0.1 with `p`'s server certificate (handshake only).
    fn listen(p: &Pki) -> u16 {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        p.ensure_server_sans(&["127.0.0.1".parse().unwrap()]).unwrap();
        let cfg = p.server_config().unwrap();
        std::thread::spawn(move || {
            for mut tcp in l.incoming().flatten() {
                let mut conn = rustls::ServerConnection::new(cfg.clone()).unwrap();
                while conn.is_handshaking() {
                    if conn.complete_io(&mut tcp).is_err() {
                        break;
                    }
                }
            }
        });
        port
    }

    #[test]
    fn the_probe_tells_this_bia_from_another_and_from_anything_else() {
        let (da, a) = fresh("a");
        let (db, b) = fresh("b");
        let lo: IpAddr = "127.0.0.1".parse().unwrap();
        assert_eq!(a.probe_server_ip(lo, listen(&a)), IpProbe::Ours);
        assert_eq!(a.probe_server_ip(lo, listen(&b)), IpProbe::OtherBia, "another bia's CA");
        let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        assert_eq!(a.probe_server_ip(lo, free), IpProbe::Unknown, "refused");
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let plain = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            use std::io::Write;
            for mut s in l.incoming().flatten() {
                let _ = s.write_all(b"SSH-2.0-OpenSSH_9.6\r\n");
            }
        });
        assert_eq!(a.probe_server_ip(lo, plain), IpProbe::Unknown, "not TLS");
        let _ = fs::remove_dir_all(da);
        let _ = fs::remove_dir_all(db);
    }

    #[test]
    fn an_address_where_another_bia_lives_leaves_the_certificate() {
        let (dir, p) = fresh("prune");
        let ip = |s: &str| -> IpAddr { s.parse().unwrap() };
        let (other, nat, silent, mine, lan) = (ip("51.91.177.37"), ip("203.0.113.9"), ip("198.51.100.4"), ip("192.0.2.10"), ip("10.0.0.5"));
        p.set_server_extra(&["bia.example.org".into()], &[other, nat, silent, mine, lan]).unwrap();
        p.ensure_server_sans(&[lan]).unwrap();
        let answers = |i: IpAddr, port: u16| match (i, port) {
            (i, 443) if i == other || i == nat || i == lan => IpProbe::OtherBia,
            // a router forwards 8443 to this bia, its 443 to another one
            (i, 8443) if i == nat => IpProbe::Ours,
            (i, _) if i == mine => IpProbe::Ours,
            _ => IpProbe::Unknown,
        };
        assert_eq!(p.prune_server_ips_with(443, &[lan], answers), vec![other]);
        let (names, ips) = p.server_extra();
        assert_eq!(names, vec!["bia.example.org".to_string()]);
        assert_eq!(ips, vec![nat, silent, mine, lan], "forwarded, silent, ours and the machine's own stay");
        assert!(p.prune_server_ips_with(443, &[lan], answers).is_empty(), "stable");
        // the other bia on 8443, this one on 443
        p.set_server_extra(&[], &[other]).unwrap();
        let on_8443 = |i: IpAddr, port: u16| if i == other && port == 8443 { IpProbe::OtherBia } else { IpProbe::Unknown };
        assert_eq!(p.prune_server_ips_with(443, &[], on_8443), vec![other]);
        p.set_server_extra(&["bia.example.org".into()], &[nat, silent, mine, lan]).unwrap();
        // the certificate is made again without it
        assert!(p.ensure_server_sans(&[lan]).unwrap());
        let marker = fs::read_to_string(p.dir.join("server.sans")).unwrap();
        assert!(!marker.contains("51.91.177.37") && marker.contains("fixed:203.0.113.9"), "{marker}");
        let pem = fs::read_to_string(p.server_cert_path()).unwrap();
        let der = pem_to_cert_ders(&pem).unwrap().into_iter().next().unwrap();
        let (_, c) = x509_parser::parse_x509_certificate(der.as_ref()).unwrap();
        let sans = format!("{:?}", c.subject_alternative_name().unwrap().unwrap().value);
        assert!(!sans.contains("[51, 91, 177, 37]") && sans.contains("[198, 51, 100, 4]"), "{sans}");
        let _ = fs::remove_dir_all(dir);
    }
}
