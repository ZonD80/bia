//! PKCS#12 the way OpenSSL 3 writes it by default (and web/pki.js in the browser):
//! certificates in plain bags, the key in a PBES2 shrouded bag (PBKDF2-HMAC-SHA256,
//! AES-256-CBC), SHA-256 HMAC over the contents. No RC2/3DES, so `openssl pkcs12` opens it
//! without `-legacy`; macOS, Windows 10+, Linux and Firefox import it.
use aes::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
use anyhow::{anyhow, Result};
use ring::rand::{SecureRandom, SystemRandom};
use sha2::{Digest, Sha256};
use std::num::NonZeroU32;

const ITERATIONS: u32 = 2048;

// ---- DER ----
fn tlv(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let n = body.len();
    if n < 0x80 {
        out.push(n as u8);
    } else {
        let b: Vec<u8> = n
            .to_be_bytes()
            .iter()
            .copied()
            .skip_while(|x| *x == 0)
            .collect();
        out.push(0x80 | b.len() as u8);
        out.extend_from_slice(&b);
    }
    out.extend_from_slice(body);
    out
}
fn seq(parts: &[&[u8]]) -> Vec<u8> {
    tlv(0x30, &parts.concat())
}
fn set(parts: &[&[u8]]) -> Vec<u8> {
    tlv(0x31, &parts.concat())
}
fn octets(b: &[u8]) -> Vec<u8> {
    tlv(0x04, b)
}
fn ctx0(b: &[u8]) -> Vec<u8> {
    tlv(0xa0, b)
}
fn null() -> Vec<u8> {
    vec![0x05, 0x00]
}
fn int(v: u32) -> Vec<u8> {
    let mut b: Vec<u8> = v
        .to_be_bytes()
        .iter()
        .copied()
        .skip_while(|x| *x == 0)
        .collect();
    if b.is_empty() || b[0] & 0x80 != 0 {
        b.insert(0, 0);
    }
    tlv(0x02, &b)
}
fn oid(dotted: &str) -> Vec<u8> {
    let p: Vec<u64> = dotted.split('.').map(|x| x.parse().unwrap_or(0)).collect();
    let mut out = vec![(40 * p[0] + p[1]) as u8];
    for &x in &p[2..] {
        let mut b = vec![(x & 0x7f) as u8];
        let mut v = x >> 7;
        while v > 0 {
            b.insert(0, 0x80 | (v & 0x7f) as u8);
            v >>= 7;
        }
        out.extend(b);
    }
    tlv(0x06, &out)
}
fn bmp(s: &str) -> Vec<u8> {
    tlv(
        0x1e,
        &s.encode_utf16()
            .flat_map(|u| u.to_be_bytes())
            .collect::<Vec<_>>(),
    )
}

const DATA: &str = "1.2.840.113549.1.7.1";
const SHROUDED_KEY_BAG: &str = "1.2.840.113549.1.12.10.1.2";
const CERT_BAG: &str = "1.2.840.113549.1.12.10.1.3";
const X509_CERT: &str = "1.2.840.113549.1.9.22.1";
const FRIENDLY_NAME: &str = "1.2.840.113549.1.9.20";
const LOCAL_KEY_ID: &str = "1.2.840.113549.1.9.21";
const SHA256: &str = "2.16.840.1.101.3.4.2.1";
const PBES2: &str = "1.2.840.113549.1.5.13";
const PBKDF2: &str = "1.2.840.113549.1.5.12";
const HMAC_SHA256: &str = "1.2.840.113549.2.9";
const AES256_CBC: &str = "2.16.840.1.101.3.4.1.42";

fn random<const N: usize>() -> Result<[u8; N]> {
    let mut b = [0u8; N];
    SystemRandom::new()
        .fill(&mut b)
        .map_err(|_| anyhow!("no randomness"))?;
    Ok(b)
}

/// RFC 7292 appendix B.2 with SHA-256 (u = 32, v = 64); id 3 makes the MAC key.
fn p12_kdf(password: &str, salt: &[u8], id: u8, iterations: u32, n: usize) -> Vec<u8> {
    const U: usize = 32;
    const V: usize = 64;
    let mut pw: Vec<u8> = password
        .encode_utf16()
        .flat_map(|u| u.to_be_bytes())
        .collect();
    pw.extend_from_slice(&[0, 0]);
    let fill = |src: &[u8]| -> Vec<u8> {
        if src.is_empty() {
            return vec![];
        }
        let len = V * src.len().div_ceil(V);
        (0..len).map(|i| src[i % src.len()]).collect()
    };
    let d = [id; V];
    let mut i_buf = [fill(salt), fill(&pw)].concat();
    let mut out = Vec::with_capacity(n);
    while out.len() < n {
        let mut a = Sha256::new()
            .chain_update(d)
            .chain_update(&i_buf)
            .finalize()
            .to_vec();
        for _ in 1..iterations {
            a = Sha256::digest(&a).to_vec();
        }
        out.extend_from_slice(&a[..U.min(n - out.len())]);
        if out.len() >= n {
            break;
        }
        let b: Vec<u8> = (0..V).map(|k| a[k % U]).collect();
        for chunk in i_buf.chunks_mut(V) {
            let mut carry = 1u16;
            for k in (0..V).rev() {
                carry += chunk[k] as u16 + b[k] as u16;
                chunk[k] = carry as u8;
                carry >>= 8;
            }
        }
    }
    out
}

/// EncryptedPrivateKeyInfo: PBES2 with PBKDF2-HMAC-SHA256 and AES-256-CBC (UTF-8 password).
fn shroud(pkcs8: &[u8], password: &str) -> Result<Vec<u8>> {
    let salt: [u8; 16] = random()?;
    let iv: [u8; 16] = random()?;
    let mut key = [0u8; 32];
    ring::pbkdf2::derive(
        ring::pbkdf2::PBKDF2_HMAC_SHA256,
        NonZeroU32::new(ITERATIONS).unwrap(),
        &salt,
        password.as_bytes(),
        &mut key,
    );
    let ct = cbc::Encryptor::<aes::Aes256>::new(&key.into(), &iv.into())
        .encrypt_padded_vec_mut::<Pkcs7>(pkcs8);
    let kdf = seq(&[
        &oid(PBKDF2),
        &seq(&[
            &octets(&salt),
            &int(ITERATIONS),
            &seq(&[&oid(HMAC_SHA256), &null()]),
        ]),
    ]);
    let cipher = seq(&[&oid(AES256_CBC), &octets(&iv)]);
    let alg = seq(&[&oid(PBES2), &seq(&[&kdf, &cipher])]);
    Ok(seq(&[&alg, &octets(&ct)]))
}

/// PKCS#12 holding `key_pkcs8` with `cert` (then `chain`), protected by `password`.
pub fn build(
    cert: &[u8],
    key_pkcs8: &[u8],
    chain: &[&[u8]],
    friendly: &str,
    password: &str,
) -> Result<Vec<u8>> {
    let key_id = &Sha256::digest(cert)[..20];
    let attrs = set(&[
        &seq(&[&oid(FRIENDLY_NAME), &set(&[&bmp(friendly)])]),
        &seq(&[&oid(LOCAL_KEY_ID), &set(&[&octets(key_id)])]),
    ]);
    let cert_bag = |der: &[u8], own: bool| {
        let value = ctx0(&seq(&[&oid(X509_CERT), &ctx0(&octets(der))]));
        if own {
            seq(&[&oid(CERT_BAG), &value, &attrs])
        } else {
            seq(&[&oid(CERT_BAG), &value])
        }
    };
    let mut bags = vec![cert_bag(cert, true)];
    bags.extend(chain.iter().map(|c| cert_bag(c, false)));
    let certs = tlv(0x30, &bags.concat());
    let keys = seq(&[&seq(&[
        &oid(SHROUDED_KEY_BAG),
        &ctx0(&shroud(key_pkcs8, password)?),
        &attrs,
    ])]);
    let data = |body: &[u8]| seq(&[&oid(DATA), &ctx0(&octets(body))]);
    let auth_safe = seq(&[&data(&certs), &data(&keys)]);
    let salt: [u8; 16] = random()?;
    let mac_key = p12_kdf(password, &salt, 3, ITERATIONS, 32);
    let mac = ring::hmac::sign(
        &ring::hmac::Key::new(ring::hmac::HMAC_SHA256, &mac_key),
        &auth_safe,
    );
    let mac_data = seq(&[
        &seq(&[&seq(&[&oid(SHA256), &null()]), &octets(mac.as_ref())]),
        &octets(&salt),
        &int(ITERATIONS),
    ]);
    Ok(seq(&[&int(3), &data(&auth_safe), &mac_data]))
}
