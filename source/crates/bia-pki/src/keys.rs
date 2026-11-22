//! Private keys in any PEM form people and older tools made. rcgen (on ring) reads only
//! PKCS#8 (`BEGIN PRIVATE KEY`), but a key made with `openssl ecparam -genkey` is SEC1
//! (`BEGIN EC PRIVATE KEY`) and an older RSA key is PKCS#1 (`BEGIN RSA PRIVATE KEY`): a
//! bia-bridge profile imported from such a key failed every sign-in with "Could not parse
//! key pair". Both are wrapped into PKCS#8 here; the key itself stays the same.
use anyhow::{anyhow, Result};
use rcgen::KeyPair;
use rustls::pki_types::PrivateKeyDer;

/// A key pair from PEM: PKCS#8, SEC1 (P-256, P-384) or PKCS#1 RSA.
pub fn key_pair(pem: &str) -> Result<KeyPair> {
    let mut rd = std::io::BufReader::new(pem.as_bytes());
    let key = rustls_pemfile::private_key(&mut rd)
        .map_err(|e| anyhow!("read key: {e}"))?
        .ok_or_else(|| anyhow!("no private key in the PEM"))?;
    let der = match &key {
        PrivateKeyDer::Pkcs8(k) => k.secret_pkcs8_der().to_vec(),
        PrivateKeyDer::Sec1(k) => sec1_to_pkcs8(k.secret_sec1_der())?,
        PrivateKeyDer::Pkcs1(k) => pkcs1_to_pkcs8(k.secret_pkcs1_der()),
        _ => return Err(anyhow!("unsupported private key form")),
    };
    KeyPair::try_from(der).map_err(|e| anyhow!("{e}"))
}

const OID_EC_PUBLIC_KEY: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01];
const OID_P256: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07];
const OID_P384: &[u8] = &[0x2b, 0x81, 0x04, 0x00, 0x22];
const OID_RSA: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01];

fn tlv(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let n = body.len();
    if n < 0x80 {
        out.push(n as u8);
    } else {
        let bytes: Vec<u8> = n.to_be_bytes().iter().copied().skip_while(|b| *b == 0).collect();
        out.push(0x80 | bytes.len() as u8);
        out.extend(bytes);
    }
    out.extend_from_slice(body);
    out
}

/// One DER element at the start of `d`: (tag, body, rest).
fn take(d: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let (&tag, d) = d.split_first()?;
    let (&first, mut d) = d.split_first()?;
    let len = if first < 0x80 {
        first as usize
    } else {
        let k = (first & 0x7f) as usize;
        if k == 0 || k > 4 || d.len() < k {
            return None;
        }
        let n = d[..k].iter().fold(0usize, |a, b| (a << 8) | *b as usize);
        d = &d[k..];
        n
    };
    (d.len() >= len).then(|| (tag, &d[..len], &d[len..]))
}

/// The curve of a SEC1 ECPrivateKey: its `[0]` parameters, else by the key's length.
fn sec1_curve(sec1: &[u8]) -> Result<&'static [u8]> {
    let bad = || anyhow!("not an EC private key");
    let (0x30, body, _) = take(sec1).ok_or_else(bad)? else { return Err(bad()) };
    let (0x02, _, rest) = take(body).ok_or_else(bad)? else { return Err(bad()) };
    let (0x04, private, mut rest) = take(rest).ok_or_else(bad)? else { return Err(bad()) };
    while let Some((tag, inner, more)) = take(rest) {
        if tag == 0xa0 {
            if let Some((0x06, oid, _)) = take(inner) {
                return match oid {
                    o if o == OID_P256 => Ok(OID_P256),
                    o if o == OID_P384 => Ok(OID_P384),
                    _ => Err(anyhow!("an EC key on a curve other than P-256 or P-384")),
                };
            }
        }
        rest = more;
    }
    match private.len() {
        32 => Ok(OID_P256),
        48 => Ok(OID_P384),
        _ => Err(anyhow!("an EC key of unknown curve")),
    }
}

fn sec1_to_pkcs8(sec1: &[u8]) -> Result<Vec<u8>> {
    let curve = sec1_curve(sec1)?;
    let alg = tlv(0x30, &[tlv(0x06, OID_EC_PUBLIC_KEY), tlv(0x06, curve)].concat());
    Ok(tlv(0x30, &[tlv(0x02, &[0]), alg, tlv(0x04, sec1)].concat()))
}

fn pkcs1_to_pkcs8(pkcs1: &[u8]) -> Vec<u8> {
    let alg = tlv(0x30, &[tlv(0x06, OID_RSA), vec![0x05, 0x00]].concat());
    tlv(0x30, &[tlv(0x02, &[0]), alg, tlv(0x04, pkcs1)].concat())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// A key in each form from openssl; the same key must come out whatever the wrapping.
    fn openssl(args: &[&str], input: Option<&str>) -> Option<String> {
        use std::io::Write;
        let mut c = Command::new("openssl")
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .ok()?;
        if let Some(i) = input {
            c.stdin.take()?.write_all(i.as_bytes()).ok()?;
        } else {
            drop(c.stdin.take());
        }
        let o = c.wait_with_output().ok()?;
        o.status.success().then(|| String::from_utf8_lossy(&o.stdout).into_owned())
    }

    #[test]
    fn sec1_and_pkcs1_keys_load_as_the_same_key() {
        let Some(sec1) = openssl(&["ecparam", "-name", "prime256v1", "-genkey", "-noout"], None) else {
            eprintln!("no openssl here: skipped");
            return;
        };
        assert!(sec1.contains("BEGIN EC PRIVATE KEY"));
        let pkcs8 = openssl(&["pkcs8", "-topk8", "-nocrypt"], Some(&sec1)).unwrap();
        let a = key_pair(&sec1).unwrap();
        let b = key_pair(&pkcs8).unwrap();
        assert_eq!(a.public_key_der(), b.public_key_der());
        // it signs: the bridge's sign-in
        assert!(crate::signin::sign(&sec1, "hello").is_ok());

        let p384 = openssl(&["ecparam", "-name", "secp384r1", "-genkey", "-noout"], None).unwrap();
        let p384_8 = openssl(&["pkcs8", "-topk8", "-nocrypt"], Some(&p384)).unwrap();
        assert_eq!(key_pair(&p384).unwrap().public_key_der(), key_pair(&p384_8).unwrap().public_key_der());

        if let Some(rsa8) = openssl(&["genpkey", "-algorithm", "RSA", "-pkeyopt", "rsa_keygen_bits:2048"], None) {
            if let Some(rsa1) = openssl(&["rsa", "-traditional"], Some(&rsa8)) {
                assert!(rsa1.contains("BEGIN RSA PRIVATE KEY"), "{rsa1}");
                assert_eq!(key_pair(&rsa1).unwrap().public_key_der(), key_pair(&rsa8).unwrap().public_key_der());
            }
        }
        assert!(key_pair("nothing").is_err());
    }
}
