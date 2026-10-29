//! PKCS#12 the way OpenSSL 3 writes it by default (and web/pki.js in the browser):
//! certificates in plain bags, the key in a PBES2 shrouded bag (PBKDF2-HMAC-SHA256,
//! AES-256-CBC), SHA-256 HMAC over the contents. No RC2/3DES, so `openssl pkcs12` opens it
//! without `-legacy`; macOS, Windows 10+, Linux and Firefox import it.
use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
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

// ---- reading (the same layout: bia's own .p12 files and web/pki.js renewals) ----

/// One DER element: (tag, contents, the rest after it).
fn der(b: &[u8]) -> Result<(u8, &[u8], &[u8])> {
    if b.len() < 2 {
        return Err(anyhow!("p12: truncated"));
    }
    let tag = b[0];
    let (len, hdr) = match b[1] {
        n if n < 0x80 => (n as usize, 2),
        0x81..=0x84 => {
            let k = (b[1] & 0x7f) as usize;
            if b.len() < 2 + k {
                return Err(anyhow!("p12: truncated"));
            }
            (b[2..2 + k].iter().fold(0usize, |a, x| (a << 8) | *x as usize), 2 + k)
        }
        _ => return Err(anyhow!("p12: unsupported length")),
    };
    if b.len() < hdr + len {
        return Err(anyhow!("p12: truncated"));
    }
    Ok((tag, &b[hdr..hdr + len], &b[hdr + len..]))
}

/// The elements inside a SEQUENCE / SET / [0].
fn items(b: &[u8]) -> Result<Vec<(u8, &[u8])>> {
    let mut out = vec![];
    let mut rest = b;
    while !rest.is_empty() {
        let (t, c, r) = der(rest)?;
        out.push((t, c));
        rest = r;
    }
    Ok(out)
}

fn want<'a>(v: &[(u8, &'a [u8])], i: usize, tag: u8) -> Result<&'a [u8]> {
    match v.get(i) {
        Some((t, c)) if *t == tag => Ok(c),
        other => Err(anyhow!("p12: unexpected structure (item {i}: want tag {tag:#x}, found {:?})", other.map(|(t, _)| *t))),
    }
}

fn oid_is(c: &[u8], dotted: &str) -> bool {
    oid(dotted).get(2..) == Some(c)
}

fn uint(c: &[u8]) -> u32 {
    c.iter().fold(0u32, |a, x| (a << 8) | *x as u32)
}

fn unshroud(epki: &[u8], password: &str) -> Result<Vec<u8>> {
    let unsupported = || anyhow!("this .p12 uses an older encryption (not PBES2/AES-256); export the certificate and key as PEM instead");
    let top = items(epki)?;
    let alg = items(want(&top, 0, 0x30)?)?;
    if !oid_is(want(&alg, 0, 0x06)?, PBES2) {
        return Err(unsupported());
    }
    let params = items(want(&alg, 1, 0x30)?)?;
    let kdf = items(want(&params, 0, 0x30)?)?;
    if !oid_is(want(&kdf, 0, 0x06)?, PBKDF2) {
        return Err(unsupported());
    }
    let kp = items(want(&kdf, 1, 0x30)?)?;
    let salt = want(&kp, 0, 0x04)?;
    let iter = uint(want(&kp, 1, 0x02)?).max(1);
    let prf_sha256 = kp.iter().skip(2).any(|(t, c)| *t == 0x30 && items(c).ok().and_then(|x| x.first().map(|(_, o)| oid_is(o, HMAC_SHA256))).unwrap_or(false));
    if !prf_sha256 {
        return Err(unsupported());
    }
    let cipher = items(want(&params, 1, 0x30)?)?;
    if !oid_is(want(&cipher, 0, 0x06)?, AES256_CBC) {
        return Err(unsupported());
    }
    let iv = want(&cipher, 1, 0x04)?;
    let ct = want(&top, 1, 0x04)?;
    let mut key = [0u8; 32];
    ring::pbkdf2::derive(ring::pbkdf2::PBKDF2_HMAC_SHA256, NonZeroU32::new(iter).unwrap(), salt, password.as_bytes(), &mut key);
    let iv: [u8; 16] = iv.try_into().map_err(|_| anyhow!("p12: bad iv"))?;
    cbc::Decryptor::<aes::Aes256>::new(&key.into(), &iv.into())
        .decrypt_padded_vec_mut::<Pkcs7>(ct)
        .map_err(|_| anyhow!("wrong .p12 password"))
}

/// Read a .p12 in the layout above: (the key as PKCS#8 DER, certificates DER in order).
/// The SHA-256 MAC is checked, so a wrong password is reported as such.
pub fn parse(p12: &[u8], password: &str) -> Result<(Vec<u8>, Vec<Vec<u8>>)> {
    let (_, pfx, _) = der(p12)?;
    let pfx = items(pfx)?;
    let ci = items(want(&pfx, 1, 0x30)?)?;
    if !oid_is(want(&ci, 0, 0x06)?, DATA) {
        return Err(anyhow!("this .p12 is not password-integrity PKCS#12 data; export PEM instead"));
    }
    let auth_safe = want(&items(want(&ci, 1, 0xa0)?)?, 0, 0x04)?;
    if let Some((0x30, mac_data)) = pfx.get(2).copied() {
        let md = items(mac_data)?;
        let digest = items(want(&md, 0, 0x30)?)?;
        let alg = items(want(&digest, 0, 0x30)?)?;
        if oid_is(want(&alg, 0, 0x06)?, SHA256) {
            let mac = want(&digest, 1, 0x04)?;
            let salt = want(&md, 1, 0x04)?;
            let iter = md.get(2).map(|(_, c)| uint(c)).unwrap_or(1).max(1);
            let key = p12_kdf(password, salt, 3, iter, 32);
            ring::hmac::verify(&ring::hmac::Key::new(ring::hmac::HMAC_SHA256, &key), auth_safe, mac)
                .map_err(|_| anyhow!("wrong .p12 password"))?;
        }
    }
    let mut key = None;
    let mut certs = vec![];
    // AuthenticatedSafe: a SEQUENCE of ContentInfo
    let (_, safes, _) = der(auth_safe)?;
    for (t, content_info) in items(safes)? {
        if t != 0x30 {
            continue;
        }
        let ci = items(content_info)?;
        if !oid_is(want(&ci, 0, 0x06)?, DATA) {
            return Err(anyhow!("this .p12 encrypts its certificates (an older format); export PEM instead"));
        }
        let octets = want(&items(want(&ci, 1, 0xa0)?)?, 0, 0x04)?;
        // SafeContents: a SEQUENCE of bags
        let (_, bags, _) = der(octets)?;
        for (_, bag) in items(bags)? {
            let bag = items(bag)?;
            let kind = want(&bag, 0, 0x06)?;
            let value = want(&bag, 1, 0xa0)?;
            if oid_is(kind, CERT_BAG) {
                let cb = items(want(&items(value)?, 0, 0x30)?)?;
                if oid_is(want(&cb, 0, 0x06)?, X509_CERT) {
                    certs.push(want(&items(want(&cb, 1, 0xa0)?)?, 0, 0x04)?.to_vec());
                }
            } else if oid_is(kind, SHROUDED_KEY_BAG) {
                let (_, epki, _) = der(value)?;
                key = Some(unshroud(epki, password)?);
            }
        }
    }
    Ok((key.ok_or_else(|| anyhow!("the .p12 holds no private key"))?, certs))
}

#[cfg(test)]
mod read_tests {
    use super::*;

    #[test]
    fn reads_what_it_writes() {
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = rcgen::CertificateParams::new(vec!["x".into()]).unwrap().self_signed(&key).unwrap();
        let ca = b"\x30\x03\x02\x01\x05".to_vec();
        let p = build(cert.der(), &key.serialize_der(), &[&ca], "me", "12345678").unwrap();
        let (k, certs) = parse(&p, "12345678").unwrap();
        assert_eq!(k, key.serialize_der());
        assert_eq!(certs, vec![cert.der().to_vec(), ca]);
        assert!(parse(&p, "wrong").unwrap_err().to_string().contains("wrong .p12 password"));
    }
}
