//! A complete PKI made before bia runs (`bia-flash takeover`): CA, server certificate
//! with fixed names and addresses, an active root certificate and auto-approve rules.
//! The files go onto the ESP and `bia-seed` moves them into `files/state/pki` on first
//! boot. Because root is already issued, the public first-time root route
//! (`/v1/pki/bootstrap/root`) is closed from the first second.
use crate::{Cidr, Pki};
use anyhow::{bail, Result};
use std::net::IpAddr;
use std::path::{Path, PathBuf};

/// How the root certificate is made.
#[derive(Clone, Debug)]
pub enum RootSeed {
    /// Key made here; the PKCS#12 (password `P12_PASSWORD`) is handed to the operator.
    Generate,
    /// The operator's own CSR (PEM): only the certificate is made here.
    Csr(String),
}

#[derive(Clone, Debug)]
pub struct SeedOptions {
    /// DNS names for the server certificate.
    pub names: Vec<String>,
    /// Addresses for the server certificate (public NAT address included).
    pub ips: Vec<IpAddr>,
    pub root: RootSeed,
    /// Networks the root certificate works from (empty = anywhere).
    pub root_from: Vec<Cidr>,
    /// The agent name other bias know this one by (default `bia-<CA fingerprint>`).
    pub agent_name: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Seeded {
    /// `<files>/state/pki`
    pub dir: PathBuf,
    pub ca_sha256: String,
    pub root_serial: String,
    /// `RootSeed::Generate`: the PKCS#12 for the operator.
    pub root_p12: Option<Vec<u8>>,
    /// `RootSeed::Csr`: certificate + CA (PEM).
    pub root_pem: Option<String>,
    pub agent_name: String,
}

/// Make a new PKI under `files/state/pki` (which must not exist yet).
pub fn seed(files: &Path, o: &SeedOptions) -> Result<Seeded> {
    let dir = Pki::dir_for(files);
    if dir.join("ca.crt").exists() {
        bail!("{} already holds a CA; not replacing it", dir.display());
    }
    let pki = Pki::load_or_create(files)?;
    pki.set_server_extra(&o.names, &o.ips)?;
    pki.ensure_server_sans(&[])?;
    let (root_p12, root_pem, meta) = match &o.root {
        RootSeed::Generate => {
            let (p12, meta) = pki.issue_from("root", "user", "root", &o.root_from)?;
            (Some(p12), None, meta)
        }
        RootSeed::Csr(csr) => {
            let (pem, meta) = pki.issue_csr_from("root", "user", "root", csr, &o.root_from)?;
            (None, Some(pem), meta)
        }
    };
    if let Some(n) = o.agent_name.as_deref().filter(|n| !n.trim().is_empty()) {
        pki.set_agent_name(n.trim())?;
    }
    // the agent key exists from the start, so its CSR is ready for the first peer
    pki.agent_csr()?;
    Ok(Seeded {
        dir,
        ca_sha256: pki.ca_sha256()?,
        root_serial: meta.serial,
        root_p12,
        root_pem,
        agent_name: pki.agent_name(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeded_pki_has_root_extras_and_agent() {
        crate::install_crypto_provider();
        let files = std::env::temp_dir().join(format!("bia-pki-seed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&files);
        let s = seed(
            &files,
            &SeedOptions {
                names: vec!["bia.example.com".into()],
                ips: vec!["203.0.113.7".parse().unwrap()],
                root: RootSeed::Generate,
                root_from: vec![],
                agent_name: Some("vps-1".into()),
            },
        )
        .unwrap();
        assert!(s.root_p12.is_some() && s.root_pem.is_none());
        assert_eq!(s.agent_name, "vps-1");
        let pki = Pki::load_or_create(&files).unwrap();
        assert!(pki.has_active_identity("root"), "bootstrap is closed");
        let pem = std::fs::read_to_string(pki.server_cert_path()).unwrap();
        let der = crate::pem_to_cert_ders(&pem).unwrap().remove(0);
        let (_, x) = x509_parser::parse_x509_certificate(der.as_ref()).unwrap();
        let san = format!("{:?}", x.subject_alternative_name().unwrap().unwrap().value);
        assert!(san.contains("bia.example.com"), "{san}");
        assert!(san.contains("203.0.113.7") || san.contains("[203, 0, 113, 7]"), "{san}");
        // the extras survive a reissue for LAN addresses
        assert!(pki.ensure_server_sans(&["10.0.0.5".parse().unwrap()]).unwrap());
        let pem = std::fs::read_to_string(pki.server_cert_path()).unwrap();
        let der = crate::pem_to_cert_ders(&pem).unwrap().remove(0);
        let (_, x) = x509_parser::parse_x509_certificate(der.as_ref()).unwrap();
        let san = format!("{:?}", x.subject_alternative_name().unwrap().unwrap().value);
        assert!(san.contains("bia.example.com"), "{san}");
        assert!(!pki.ensure_server_sans(&["10.0.0.5".parse().unwrap()]).unwrap(), "stable");
        assert!(seed(&files, &SeedOptions { names: vec![], ips: vec![], root: RootSeed::Generate, root_from: vec![], agent_name: None }).is_err(), "never over an existing CA");
        let _ = std::fs::remove_dir_all(&files);
    }
}
