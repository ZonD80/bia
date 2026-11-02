//! `.biaconfig`: a client certificate and how to reach its bia, in one file a person can pass
//! on. A JSON envelope around AES-256-GCM with a PBKDF2-SHA256 key from a password. It can hold
//! root's own key, so a password is required: there is no default one to fall back to.
//!
//! The one implementation: bia writes these with it, and so do the phone apps through
//! bia-deploy-mobile.

use crate::{p12, P12_PASSWORD};
use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use ring::{aead, pbkdf2, rand::SecureRandom};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::num::NonZeroU32;

pub const EXTENSION: &str = "biaconfig";
pub const MIME: &str = "application/x-biaconfig";
const FORMAT: &str = "biaconfig";
const ALG: &str = "aes-256-gcm";
const KDF: &str = "pbkdf2-sha256";
const VERSION: u64 = 1;
const ITERATIONS: u32 = 600_000;
/// What a file may ask for: below this it is too weak to trust, above it just a way to hang the
/// reader.
const MIN_ITERATIONS: u64 = 100_000;
const MAX_ITERATIONS: u64 = 10_000_000;
const SALT_LEN: usize = 16;
const MAX_FILE: usize = 1024 * 1024;
pub const MIN_PASSWORD: usize = 10;

/// What a `.biaconfig` holds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BiaConfig {
    /// What a person calls this bia; may be empty.
    #[serde(default)]
    pub name: String,
    /// `host` or `host:port`, the best one first (IPv6 in brackets with a port).
    pub addresses: Vec<String>,
    /// bia's certificate authority, PEM.
    pub ca: String,
    /// The client certificate and its key, a .p12 under `P12_PASSWORD`.
    #[serde(with = "b64")]
    pub p12: Vec<u8>,
}

mod b64 {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(b: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&STANDARD.encode(b))
    }

    pub fn deserialize<'d, D: Deserializer<'d>>(d: D) -> Result<Vec<u8>, D::Error> {
        let t = String::deserialize(d)?;
        STANDARD.decode(t).map_err(serde::de::Error::custom)
    }
}

/// What the file's certificate says, for a person to see before relying on it.
#[derive(Clone, Debug, PartialEq)]
pub struct Details {
    pub identity: String,
    pub serial: String,
    pub not_after: i64,
}

pub fn check_password(password: &str) -> Result<()> {
    if password.chars().count() < MIN_PASSWORD {
        bail!("the password needs at least {MIN_PASSWORD} characters: this file can open the bia it belongs to");
    }
    Ok(())
}

/// A password nobody has to invent: about 80 bits, easy to read out.
pub fn generate_password() -> Result<String> {
    const LETTERS: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    let rng = ring::rand::SystemRandom::new();
    let mut out = String::new();
    for i in 0..16 {
        if i > 0 && i % 4 == 0 {
            out.push('-');
        }
        // no modulo bias: a byte outside the largest multiple is drawn again
        let limit = 256 - 256 % LETTERS.len();
        loop {
            let mut b = [0u8; 1];
            rng.fill(&mut b).map_err(|_| anyhow!("no randomness"))?;
            if (b[0] as usize) < limit {
                out.push(LETTERS[b[0] as usize % LETTERS.len()] as char);
                break;
            }
        }
    }
    Ok(out)
}

fn derive(password: &str, salt: &[u8], iterations: u32) -> aead::LessSafeKey {
    let mut key = [0u8; 32];
    let n = NonZeroU32::new(iterations).unwrap_or(NonZeroU32::MIN);
    pbkdf2::derive(pbkdf2::PBKDF2_HMAC_SHA256, n, salt, password.as_bytes(), &mut key);
    let unbound = aead::UnboundKey::new(&aead::AES_256_GCM, &key).expect("a 32-byte key");
    aead::LessSafeKey::new(unbound)
}

/// A readable check of what it holds: a config that opens must also work.
pub fn details(c: &BiaConfig) -> Result<Details> {
    if c.addresses.iter().all(|a| a.trim().is_empty()) {
        bail!("this .biaconfig names no address");
    }
    // an address becomes a URL in the apps: nothing here may change where that URL goes
    if let Some(bad) = c.addresses.iter().find(|a| a.chars().any(|ch| ch.is_whitespace() && !a.trim().is_empty() || "/\\@?#%".contains(ch))) {
        bail!("this .biaconfig has a bad address: {bad:?}");
    }
    let ca_ders = crate::pem_to_cert_ders(&c.ca).context("this .biaconfig's certificate authority")?;
    let ca = ca_ders.first().ok_or_else(|| anyhow!("this .biaconfig has no certificate authority"))?;
    x509_parser::parse_x509_certificate(ca.as_ref()).map_err(|_| anyhow!("this .biaconfig's certificate authority cannot be read"))?;
    let (_, certs) = p12::parse(&c.p12, P12_PASSWORD).context("this .biaconfig's certificate")?;
    let leaf = certs.first().ok_or_else(|| anyhow!("this .biaconfig has no certificate"))?;
    let (_, cert) = x509_parser::parse_x509_certificate(leaf).map_err(|_| anyhow!("this .biaconfig's certificate cannot be read"))?;
    let identity = cert.subject().iter_common_name().next().and_then(|n| n.as_str().ok()).unwrap_or("").trim_start_matches("Identity: ").to_string();
    Ok(Details { identity, serial: cert.raw_serial_as_string().replace(':', ""), not_after: cert.validity().not_after.timestamp() })
}

/// The file for `config`, encrypted with `password`.
pub fn seal(config: &BiaConfig, password: &str) -> Result<Vec<u8>> {
    check_password(password)?;
    details(config)?;
    let mut plain = serde_json::to_vec(&json!({"v": VERSION, "kind": "bia", "bia": config}))?;
    let rng = ring::rand::SystemRandom::new();
    let mut salt = [0u8; SALT_LEN];
    let mut nonce = [0u8; aead::NONCE_LEN];
    rng.fill(&mut salt).map_err(|_| anyhow!("no randomness"))?;
    rng.fill(&mut nonce).map_err(|_| anyhow!("no randomness"))?;
    let key = derive(password, &salt, ITERATIONS);
    key.seal_in_place_append_tag(aead::Nonce::assume_unique_for_key(nonce), aead::Aad::from(FORMAT), &mut plain)
        .map_err(|_| anyhow!("could not encrypt"))?;
    let envelope = json!({
        "v": VERSION, "fmt": FORMAT, "alg": ALG, "kdf": KDF, "iter": ITERATIONS,
        "salt": STANDARD.encode(salt), "nonce": STANDARD.encode(nonce), "data": STANDARD.encode(&plain),
    });
    Ok(serde_json::to_vec_pretty(&envelope)?)
}

/// The file for a key kept in a folder as `bia-root.p12` and `ca.crt`, how the deploy tools keep
/// them. Whoever opens it holds that same certificate.
pub fn seal_folder(dir: &std::path::Path, address: &str, name: &str, password: &str) -> Result<Vec<u8>> {
    let read = |f: &str| std::fs::read(dir.join(f)).with_context(|| format!("{}", dir.join(f).display()));
    let config = BiaConfig {
        name: name.to_string(),
        addresses: vec![address.to_string()],
        ca: String::from_utf8_lossy(&read("ca.crt")?).into_owned(),
        p12: read("bia-root.p12")?,
    };
    seal(&config, password)
}

/// A file name for what reaches `address`, safe on every system.
pub fn file_name(address: &str) -> String {
    let safe: String = address.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '-' }).collect();
    format!("bia-{safe}.{EXTENSION}")
}

pub fn is_biaconfig(bytes: &[u8]) -> bool {
    serde_json::from_slice::<Value>(bytes).is_ok_and(|v| v["fmt"] == FORMAT)
}

/// What is inside a file, if `password` opens it.
pub fn open(bytes: &[u8], password: &str) -> Result<BiaConfig> {
    if bytes.len() > MAX_FILE {
        bail!("this is too big to be a .biaconfig");
    }
    let v: Value = serde_json::from_slice(bytes).map_err(|_| anyhow!("this is not a .biaconfig file"))?;
    if v["fmt"] != FORMAT {
        bail!("this is not a .biaconfig file");
    }
    if v["v"] != VERSION || v["alg"] != ALG || v["kdf"] != KDF {
        bail!("this .biaconfig was made by a newer bia: update this app");
    }
    let iterations = v["iter"].as_u64().unwrap_or(0);
    if !(MIN_ITERATIONS..=MAX_ITERATIONS).contains(&iterations) {
        bail!("this .biaconfig is damaged");
    }
    let field = |k: &str| v[k].as_str().and_then(|s| STANDARD.decode(s).ok()).ok_or_else(|| anyhow!("this .biaconfig is damaged"));
    let (salt, nonce, mut data) = (field("salt")?, field("nonce")?, field("data")?);
    let nonce: [u8; aead::NONCE_LEN] = nonce.try_into().map_err(|_| anyhow!("this .biaconfig is damaged"))?;
    let key = derive(password, &salt, iterations as u32);
    let plain = key
        .open_in_place(aead::Nonce::assume_unique_for_key(nonce), aead::Aad::from(FORMAT), &mut data)
        .map_err(|_| anyhow!("wrong password, or the file is damaged"))?;
    let inner: Value = serde_json::from_slice(plain).map_err(|_| anyhow!("this .biaconfig is damaged"))?;
    if inner["kind"] != "bia" {
        bail!("this .biaconfig was made by a newer bia: update this app");
    }
    let config: BiaConfig = serde_json::from_value(inner["bia"].clone()).map_err(|_| anyhow!("this .biaconfig is damaged"))?;
    details(&config)?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Pki;

    fn sample() -> BiaConfig {
        crate::install_crypto_provider();
        let dir = std::env::temp_dir().join(format!("bia-config-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&dir);
        let pki = Pki::load_or_create(&dir).unwrap();
        let (p12, _) = pki.issue("root", "user", "root").unwrap();
        let ca = pki.ca_pem().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        BiaConfig { name: "Fred".into(), addresses: vec!["bia.example.com:8443".into(), "10.0.0.5".into()], ca, p12 }
    }

    #[test]
    fn a_config_comes_back_with_its_password_and_keeps_its_secrets_in() {
        let c = sample();
        let file = seal(&c, "correct horse battery").unwrap();
        assert!(is_biaconfig(&file));
        assert_eq!(open(&file, "correct horse battery").unwrap(), c);
        let text = String::from_utf8(file.clone()).unwrap();
        for hidden in ["bia.example.com", "Fred", "BEGIN CERTIFICATE", &STANDARD.encode(&c.p12)[..40]] {
            assert!(!text.contains(hidden), "{hidden} is readable without the password");
        }
        let d = details(&c).unwrap();
        assert_eq!(d.identity, "root");
        assert!(d.not_after > 0 && !d.serial.is_empty());
    }

    #[test]
    fn the_wrong_password_and_a_changed_file_are_refused() {
        let c = sample();
        let file = seal(&c, "correct horse battery").unwrap();
        assert!(open(&file, "wrong horse battery").unwrap_err().to_string().contains("wrong password"));
        let mut v: Value = serde_json::from_slice(&file).unwrap();
        let mut data = STANDARD.decode(v["data"].as_str().unwrap()).unwrap();
        data[10] ^= 1;
        v["data"] = json!(STANDARD.encode(&data));
        let changed = serde_json::to_vec(&v).unwrap();
        assert!(open(&changed, "correct horse battery").is_err());
    }

    #[test]
    fn a_weak_password_cannot_make_one_and_there_is_no_default() {
        let c = sample();
        for weak in ["", "short", "123456789"] {
            assert!(seal(&c, weak).unwrap_err().to_string().contains("at least"), "{weak:?}");
        }
    }

    #[test]
    fn what_a_file_asks_for_is_checked_before_any_work() {
        let c = sample();
        let file = seal(&c, "correct horse battery").unwrap();
        for bad in [0u64, 1000, u32::MAX as u64 * 4] {
            let mut v: Value = serde_json::from_slice(&file).unwrap();
            v["iter"] = json!(bad);
            let e = open(&serde_json::to_vec(&v).unwrap(), "correct horse battery").unwrap_err();
            assert!(e.to_string().contains("damaged"), "{bad}: {e}");
        }
        let mut v: Value = serde_json::from_slice(&file).unwrap();
        v["v"] = json!(2);
        assert!(open(&serde_json::to_vec(&v).unwrap(), "x").unwrap_err().to_string().contains("newer"));
        assert!(open(b"not json", "x").unwrap_err().to_string().contains("not a .biaconfig"));
        assert!(open(br#"{"fmt":"hopperconf"}"#, "x").unwrap_err().to_string().contains("not a .biaconfig"));
        assert!(!is_biaconfig(b"{}"));
    }

    #[test]
    fn it_will_not_wrap_something_that_would_not_work() {
        let mut c = sample();
        c.addresses = vec![" ".into()];
        assert!(seal(&c, "correct horse battery").unwrap_err().to_string().contains("no address"));
        for bad in ["evil.example/path", "user@evil.example", "a.example?x", "a b.example", "a.example#x"] {
            let mut c = sample();
            c.addresses = vec![bad.into()];
            assert!(seal(&c, "correct horse battery").unwrap_err().to_string().contains("bad address"), "{bad}");
        }
        let mut c = sample();
        c.p12 = b"nonsense".to_vec();
        assert!(seal(&c, "correct horse battery").is_err());
        let mut c = sample();
        c.ca = "nonsense".into();
        assert!(seal(&c, "correct horse battery").is_err());
    }

    #[test]
    fn a_key_in_a_folder_wraps_with_its_address() {
        let c = sample();
        let dir = std::env::temp_dir().join(format!("bia-config-folder-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ca.crt"), &c.ca).unwrap();
        std::fs::write(dir.join("bia-root.p12"), &c.p12).unwrap();
        let file = seal_folder(&dir, "[2001:db8::1]:8443", "Fred", "correct horse battery").unwrap();
        let back = open(&file, "correct horse battery").unwrap();
        assert_eq!((back.p12, back.ca, back.name), (c.p12, c.ca, "Fred".to_string()));
        assert_eq!(back.addresses, vec!["[2001:db8::1]:8443".to_string()]);
        assert_eq!(file_name("[2001:db8::1]:8443"), "bia--2001-db8--1--8443.biaconfig");
        assert!(seal_folder(&dir.join("nothing"), "a", "", "correct horse battery").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_generated_password_is_long_enough_and_different() {
        let a = generate_password().unwrap();
        assert_eq!(a.len(), 19);
        assert!(check_password(&a).is_ok());
        assert_ne!(a, generate_password().unwrap());
    }
}
