//! Sign-in without mutual TLS (ARCHITECTURE.md, "Sign-in"): the server proves itself with
//! its CA key over the TLS exporter (`bind`), the client with its certificate's key over
//! the server's nonce (`auth`). Both messages are plain text lines, so the browser builds
//! the same bytes with WebCrypto.
//!
//! Signatures: ECDSA in the fixed `r‖s` form WebCrypto makes (P-256/SHA-256,
//! P-384/SHA-384), Ed25519, or RSA PKCS#1 v1.5 with SHA-256.

use crate::{CertProblem, ClientId, Pki};
use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64U, Engine};
use ring::signature::{self, EcdsaKeyPair, UnparsedPublicKey};
use rustls::pki_types::CertificateDer;

/// RFC 9266 label of the TLS 1.3 channel binding.
pub const EXPORTER_LABEL: &[u8] = b"EXPORTER-Channel-Binding";
/// How far a client's clock may be off.
pub const MAX_SKEW: i64 = 300;

/// The bytes the CA signs: `bia-tls-binding-v1\n<nonce>\n<exporter base64url>`.
pub fn bind_message(nonce: &str, exporter: &[u8]) -> String {
    format!("bia-tls-binding-v1\n{nonce}\n{}", B64U.encode(exporter))
}

/// The bytes a client signs:
/// `bia-auth-v1\n<nonce>\n<CA sha256 hex>\n<unix ts>\n<exporter base64url, empty in a browser>`.
pub fn auth_message(nonce: &str, ca_sha256: &str, ts: i64, exporter: &[u8]) -> String {
    format!(
        "bia-auth-v1\n{nonce}\n{ca_sha256}\n{ts}\n{}",
        B64U.encode(exporter)
    )
}

/// A fresh nonce (base64url of 32 random bytes).
pub fn nonce() -> String {
    let mut b = [0u8; 32];
    let _ = ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut b);
    B64U.encode(b)
}

/// Verify `sig` over `msg` with the public key of certificate `der`.
pub fn verify_with_cert(der: &[u8], msg: &[u8], sig: &[u8]) -> Result<()> {
    let (_, cert) =
        x509_parser::parse_x509_certificate(der).map_err(|e| anyhow!("certificate: {e}"))?;
    let spki = cert.public_key();
    let key = spki.subject_public_key.data.as_ref();
    let alg = spki.algorithm.algorithm.to_id_string();
    let curve = spki
        .algorithm
        .parameters
        .as_ref()
        .and_then(|p| p.as_oid().ok())
        .map(|o| o.to_id_string())
        .unwrap_or_default();
    let algs: &[&'static dyn signature::VerificationAlgorithm] = match (alg.as_str(), curve.as_str()) {
        // id-ecPublicKey with prime256v1 / secp384r1
        ("1.2.840.10045.2.1", "1.2.840.10045.3.1.7") => &[&signature::ECDSA_P256_SHA256_FIXED, &signature::ECDSA_P256_SHA256_ASN1],
        ("1.2.840.10045.2.1", "1.3.132.0.34") => &[&signature::ECDSA_P384_SHA384_FIXED, &signature::ECDSA_P384_SHA384_ASN1],
        ("1.3.101.112", _) => &[&signature::ED25519],
        ("1.2.840.113549.1.1.1", _) => &[&signature::RSA_PKCS1_2048_8192_SHA256],
        _ => bail!("unsupported key type {alg}"),
    };
    for a in algs {
        if UnparsedPublicKey::new(*a, key).verify(msg, sig).is_ok() {
            return Ok(());
        }
    }
    bail!("bad signature")
}

impl Pki {
    /// Check a `Bia` Authorization header on a plain HTTP request of this connection.
    pub fn verify_request(
        &self,
        header: &str,
        method: &str,
        target: &str,
        exporter: &[u8],
        now: i64,
        peer: Option<std::net::IpAddr>,
    ) -> std::result::Result<ClientId, CertProblem> {
        let Some((chain, ts, sig)) = parse_request_header(header) else {
            return Err(CertProblem { kind: "invalid", identity: String::new() });
        };
        let ders: Vec<CertificateDer<'static>> = chain.iter().map(|d| CertificateDer::from(d.clone())).collect();
        let id = self
            .check_client_from(Some(&ders), peer)?
            .ok_or(CertProblem { kind: "invalid", identity: String::new() })?;
        if (now - ts).abs() > MAX_SKEW {
            return Err(CertProblem { kind: "clock", identity: id.identity });
        }
        verify_with_cert(&chain[0], request_message(method, target, ts, exporter).as_bytes(), &sig)
            .map_err(|_| CertProblem { kind: "signature", identity: id.identity.clone() })?;
        Ok(id)
    }

    /// The CA's signature over `bind_message` (P-256, fixed form).
    pub fn sign_bind(&self, nonce: &str, exporter: &[u8]) -> Result<String> {
        let pem = std::fs::read_to_string(self.ca_key_path()).context("read ca.key")?;
        let kp = crate::keys::key_pair(&pem)?;
        let rng = ring::rand::SystemRandom::new();
        let key = EcdsaKeyPair::from_pkcs8(
            &signature::ECDSA_P256_SHA256_FIXED_SIGNING,
            &kp.serialize_der(),
            &rng,
        )
        .map_err(|_| anyhow!("the CA key is not P-256"))?;
        let sig = key
            .sign(&rng, bind_message(nonce, exporter).as_bytes())
            .map_err(|_| anyhow!("sign"))?;
        Ok(B64U.encode(sig.as_ref()))
    }

    /// The CA's signature over `data` in the DER form `openssl dgst -sha256 -verify` takes
    /// (with the CA's public key), base64: for what an outside agent downloads over TLS it
    /// cannot check (bia-bridge's list).
    pub fn sign_der(&self, data: &[u8]) -> Result<String> {
        use base64::Engine;
        let pem = std::fs::read_to_string(self.ca_key_path()).context("read ca.key")?;
        let kp = crate::keys::key_pair(&pem)?;
        let rng = ring::rand::SystemRandom::new();
        let key = EcdsaKeyPair::from_pkcs8(&signature::ECDSA_P256_SHA256_ASN1_SIGNING, &kp.serialize_der(), &rng)
            .map_err(|_| anyhow!("the CA key is not P-256"))?;
        let sig = key.sign(&rng, data).map_err(|_| anyhow!("sign"))?;
        Ok(base64::engine::general_purpose::STANDARD.encode(sig.as_ref()))
    }

    /// Check a client's `auth`: the chain as before (CA, expiry, revocation, networks),
    /// then its signature over this connection's `auth_message`.
    pub fn verify_auth(
        &self,
        chain: &[Vec<u8>],
        nonce: &str,
        ts: i64,
        exporter: &[u8],
        sig: &[u8],
        now: i64,
        peer: Option<std::net::IpAddr>,
    ) -> std::result::Result<ClientId, CertProblem> {
        let ders: Vec<CertificateDer<'static>> =
            chain.iter().map(|d| CertificateDer::from(d.clone())).collect();
        let id = match self.check_client_from(Some(&ders), peer)? {
            Some(id) => id,
            None => {
                return Err(CertProblem {
                    kind: "invalid",
                    identity: String::new(),
                })
            }
        };
        let problem = |kind| CertProblem {
            kind,
            identity: id.identity.clone(),
        };
        if (now - ts).abs() > MAX_SKEW {
            return Err(problem("clock"));
        }
        let ca = self.ca_sha256().map_err(|_| problem("invalid"))?;
        let msg = auth_message(nonce, &ca, ts, exporter);
        verify_with_cert(&chain[0], msg.as_bytes(), sig).map_err(|_| problem("signature"))?;
        Ok(id)
    }
}

/// A client's check of the server's `bind` against the pinned CA (PEM).
pub fn verify_bind(ca_pem: &str, nonce: &str, exporter: &[u8], bind: &str) -> Result<()> {
    let der = crate::pem_to_cert_ders(ca_pem)?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("empty CA"))?;
    let sig = B64U.decode(bind.trim()).context("bind is not base64url")?;
    verify_with_cert(der.as_ref(), bind_message(nonce, exporter).as_bytes(), &sig)
        .map_err(|_| anyhow!("this server could not prove it is the bia this CA belongs to"))
}

/// A client's signature over `auth_message` with its PEM key, base64url.
pub fn sign_auth(key_pem: &str, nonce: &str, ca_sha256: &str, ts: i64, exporter: &[u8]) -> Result<String> {
    sign(key_pem, &auth_message(nonce, ca_sha256, ts, exporter))
}

/// The bytes a client signs for one plain HTTP request on this TLS connection:
/// `bia-req-v1\n<METHOD>\n<path?query>\n<unix ts>\n<exporter base64url>`. TLS keeps the
/// body intact; the exporter keeps the request from being relayed over another connection.
pub fn request_message(method: &str, target: &str, ts: i64, exporter: &[u8]) -> String {
    format!("bia-req-v1\n{method}\n{target}\n{ts}\n{}", B64U.encode(exporter))
}

/// `Authorization: Bia chain=<der.der…>, ts=<unix>, sig=<sig>` for one request.
pub fn request_header(chain_pem: &str, key_pem: &str, method: &str, target: &str, ts: i64, exporter: &[u8]) -> Result<String> {
    let chain: Vec<String> = crate::pem_to_cert_ders(chain_pem)?
        .iter()
        .map(|d| B64U.encode(d.as_ref()))
        .collect();
    let sig = sign(key_pem, &request_message(method, target, ts, exporter))?;
    Ok(format!("Bia chain={}, ts={ts}, sig={sig}", chain.join(".")))
}

/// The fields of a `Bia` Authorization header: (chain DERs, ts, signature).
pub fn parse_request_header(h: &str) -> Option<(Vec<Vec<u8>>, i64, Vec<u8>)> {
    let rest = h.trim().strip_prefix("Bia ")?;
    let mut chain = None;
    let mut ts = None;
    let mut sig = None;
    for part in rest.split(',') {
        let (k, v) = part.trim().split_once('=')?;
        match k.trim() {
            "chain" => chain = Some(v.split('.').map(|d| B64U.decode(d.trim()).ok()).collect::<Option<Vec<_>>>()?),
            "ts" => ts = v.trim().parse().ok(),
            "sig" => sig = B64U.decode(v.trim()).ok(),
            _ => {}
        }
    }
    let chain = chain.filter(|c| !c.is_empty())?;
    Some((chain, ts?, sig?))
}

/// Sign `msg` with a PEM key (P-256 or P-384 in the fixed form, Ed25519, RSA), base64url.
pub fn sign(key_pem: &str, msg: &str) -> Result<String> {
    let kp = crate::keys::key_pair(key_pem)?;
    let rng = ring::rand::SystemRandom::new();
    let der = kp.serialize_der();
    if let Ok(k) = EcdsaKeyPair::from_pkcs8(&signature::ECDSA_P256_SHA256_FIXED_SIGNING, &der, &rng) {
        let s = k.sign(&rng, msg.as_bytes()).map_err(|_| anyhow!("sign"))?;
        return Ok(B64U.encode(s.as_ref()));
    }
    if let Ok(k) = EcdsaKeyPair::from_pkcs8(&signature::ECDSA_P384_SHA384_FIXED_SIGNING, &der, &rng) {
        let s = k.sign(&rng, msg.as_bytes()).map_err(|_| anyhow!("sign"))?;
        return Ok(B64U.encode(s.as_ref()));
    }
    if let Ok(k) = signature::Ed25519KeyPair::from_pkcs8_maybe_unchecked(&der) {
        return Ok(B64U.encode(k.sign(msg.as_bytes()).as_ref()));
    }
    if let Ok(k) = signature::RsaKeyPair::from_pkcs8(&der) {
        let mut out = vec![0u8; k.public().modulus_len()];
        k.sign(&signature::RSA_PKCS1_SHA256, &rng, msg.as_bytes(), &mut out)
            .map_err(|_| anyhow!("sign"))?;
        return Ok(B64U.encode(out));
    }
    bail!("unsupported key type")
}

/// A native client's answer to the server's `challenge` frame: checks the server's `bind`
/// against the pinned CA (PEM), then signs in with its certificate chain and key (PEM).
/// Returns the `auth` frame.
pub fn answer_challenge(
    ca_pem: &str,
    cert_pem: &str,
    key_pem: &str,
    exporter: &[u8],
    challenge: &serde_json::Value,
) -> Result<serde_json::Value> {
    let nonce = challenge["nonce"].as_str().ok_or_else(|| anyhow!("challenge without nonce"))?;
    let bind = challenge["bind"].as_str().ok_or_else(|| anyhow!("challenge without bind"))?;
    verify_bind(ca_pem, nonce, exporter, bind)?;
    let ca_der = crate::pem_to_cert_ders(ca_pem)?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("empty CA"))?;
    let ca_sha: String = ring::digest::digest(&ring::digest::SHA256, ca_der.as_ref())
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let sig = sign_auth(key_pem, nonce, &ca_sha, ts, exporter)?;
    let chain: Vec<String> = crate::pem_to_cert_ders(cert_pem)?
        .iter()
        .map(|d| B64U.encode(d.as_ref()))
        .collect();
    Ok(serde_json::json!({"t": "auth", "chain": chain, "ts": ts, "sig": sig}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("bia-signin-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn a_client_signs_in_and_the_server_proves_itself() {
        let files = tmp("ok");
        let pki = Pki::load_or_create(&files).unwrap();
        let (key_pem, csr) = crate::new_key_csr("alex").unwrap();
        let (cert_pem, _) = pki.issue_csr("alex", "user", "normal", &csr).unwrap();
        let chain: Vec<Vec<u8>> = crate::pem_to_cert_ders(&cert_pem)
            .unwrap()
            .into_iter()
            .map(|d| d.as_ref().to_vec())
            .collect();
        let exporter = [7u8; 32];
        let n = nonce();
        // the server's bind verifies against the pinned CA, and only for this exporter
        let bind = pki.sign_bind(&n, &exporter).unwrap();
        verify_bind(&pki.ca_pem().unwrap(), &n, &exporter, &bind).unwrap();
        assert!(verify_bind(&pki.ca_pem().unwrap(), &n, &[8u8; 32], &bind).is_err());
        // the client's auth
        let ca = pki.ca_sha256().unwrap();
        let ts = 1_800_000_000;
        let sig = B64U.decode(sign_auth(&key_pem, &n, &ca, ts, &exporter).unwrap()).unwrap();
        let id = pki.verify_auth(&chain, &n, ts, &exporter, &sig, ts + 5, None).unwrap();
        assert_eq!(id.identity, "alex");
        // another nonce, another connection, a stale clock: refused
        assert_eq!(pki.verify_auth(&chain, &nonce(), ts, &exporter, &sig, ts, None).unwrap_err().kind, "signature");
        assert_eq!(pki.verify_auth(&chain, &n, ts, &[1u8; 32], &sig, ts, None).unwrap_err().kind, "signature");
        assert_eq!(pki.verify_auth(&chain, &n, ts, &exporter, &sig, ts + 900, None).unwrap_err().kind, "clock");
        // a browser signs with an empty exporter
        let sig = B64U.decode(sign_auth(&key_pem, &n, &ca, ts, &[]).unwrap()).unwrap();
        assert!(pki.verify_auth(&chain, &n, ts, &[], &sig, ts, None).is_ok());
        // a certificate of another bia's CA
        let other = Pki::load_or_create(&tmp("other")).unwrap();
        let (k2, csr2) = crate::new_key_csr("eve").unwrap();
        let (c2, _) = other.issue_csr("eve", "user", "normal", &csr2).unwrap();
        let chain2: Vec<Vec<u8>> = crate::pem_to_cert_ders(&c2).unwrap().into_iter().map(|d| d.as_ref().to_vec()).collect();
        let sig2 = B64U.decode(sign_auth(&k2, &n, &ca, ts, &exporter).unwrap()).unwrap();
        assert_eq!(pki.verify_auth(&chain2, &n, ts, &exporter, &sig2, ts, None).unwrap_err().kind, "other_ca");
        // a plain HTTP request
        let ex = [3u8; 32];
        let h = request_header(&cert_pem, &key_pem, "POST", "/v1/pki/renew", ts, &ex).unwrap();
        assert_eq!(pki.verify_request(&h, "POST", "/v1/pki/renew", &ex, ts, None).unwrap().identity, "alex");
        assert_eq!(pki.verify_request(&h, "POST", "/v1/pki/renew", &[4u8; 32], ts, None).unwrap_err().kind, "signature");
        assert_eq!(pki.verify_request(&h, "GET", "/v1/pki/renew", &ex, ts, None).unwrap_err().kind, "signature");
        assert_eq!(pki.verify_request("Bearer x", "GET", "/", &ex, ts, None).unwrap_err().kind, "invalid");
        let _ = std::fs::remove_dir_all(&files);
    }

    #[test]
    fn the_setup_code_opens_root_once() {
        let files = tmp("setup");
        let pki = Pki::load_or_create(&files).unwrap();
        let code = pki.setup_code().unwrap();
        assert_eq!(pki.setup_code().unwrap(), code, "the same code until root exists");
        assert!(pki.check_setup_code(&code.to_uppercase().replace('-', " ")));
        assert!(!pki.check_setup_code("aaaa-bbbb") && !pki.check_setup_code(""));
        pki.issue("root", "user", "root").unwrap();
        assert!(pki.setup_code().is_none() && !pki.check_setup_code(&code), "gone once root is set up");
        assert!(!files.join("state/pki/setup-code").exists());
        let _ = std::fs::remove_dir_all(&files);
    }

    #[test]
    fn a_renewal_is_told_from_a_revocation() {
        let files = tmp("renewed");
        let pki = Pki::load_or_create(&files).unwrap();
        let (_, csr) = crate::new_key_csr("ann").unwrap();
        let (_, m1) = pki.issue_csr("ann", "user", "normal", &csr).unwrap();
        let cur = crate::ClientId { identity: "ann".into(), role: "user".into(), trust: "normal".into(), serial_hint: m1.serial.clone(), not_after: m1.not_after as i64 };
        let (_, csr2) = crate::new_key_csr("ann").unwrap();
        let (_, m2) = pki.renew_csr(&cur, &csr2).unwrap();
        assert!(pki.is_revoked(&m1.serial) && pki.renewed(&m1.serial), "renewed, so its live link may stay");
        let m2 = pki.issued_meta(&m2.serial).unwrap();
        assert_eq!((m2.via.as_str(), m2.parent.as_str()), ("renewal", m1.serial.as_str()));
        assert!(pki.revoke(&m2.serial).unwrap() && !pki.renewed(&m2.serial), "revoked on purpose: everything goes");
        let _ = std::fs::remove_dir_all(&files);
    }
}
