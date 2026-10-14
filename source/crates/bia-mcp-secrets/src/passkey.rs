//! Software WebAuthn/FIDO2 passkeys (ES256, packed/none). The private key never leaves the store
//! unless the caller asks for a raw dump.
use p256::ecdsa::{signature::Signer, Signature, SigningKey};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// Stable AAGUID for bia software passkeys.
const AAGUID: [u8; 16] = [0xb1, 0xa0, 0x5e, 0xc0, 0xde, 0x00, 0x40, 0x01, 0x8b, 0x1a, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01];

pub struct Created {
    pub private_raw: [u8; 32],
    pub credential_id: Vec<u8>,
    pub cose_key: Vec<u8>,
    /// Same bytes as `authData` inside `attestation_object`. Chrome checks they match.
    pub authenticator_data: Vec<u8>,
    /// SubjectPublicKeyInfo (DER) for the COSE key. Chrome checks it for ES256.
    pub public_key_spki: Vec<u8>,
    pub attestation_object: Vec<u8>,
    pub client_data_json: Vec<u8>,
    pub user_id: Vec<u8>,
}

pub struct Assertion {
    pub authenticator_data: Vec<u8>,
    pub client_data_json: Vec<u8>,
    pub signature_der: Vec<u8>,
}

pub fn create(rp_id: &str, user_name: &str, user_id: Option<&[u8]>, challenge: &[u8], origin: &str) -> Result<Created, String> {
    let sk = generate_key()?;
    let private_raw: [u8; 32] = sk.to_bytes().into();
    let credential_id = random_bytes(32)?;
    let (cose_key, public_key_spki) = cose_public(&sk)?;
    let auth = authenticator_data(rp_id, 0, true, Some((&credential_id, &cose_key)));
    let client_data_json = client_data("webauthn.create", challenge, origin);
    let attestation_object = att_none(&auth);
    let user_id = match user_id {
        Some(u) if !u.is_empty() => u.to_vec(),
        _ => {
            let mut h = Sha256::digest(user_name.as_bytes()).to_vec();
            h.truncate(16);
            h
        }
    };
    Ok(Created {
        private_raw,
        credential_id,
        cose_key,
        authenticator_data: auth,
        public_key_spki,
        attestation_object,
        client_data_json,
        user_id,
    })
}

pub fn assert(private_raw: &[u8], rp_id: &str, sign_count: u32, challenge: &[u8], origin: &str) -> Result<Assertion, String> {
    let sk = SigningKey::from_slice(private_raw).map_err(|e| format!("passkey key: {e}"))?;
    let auth = authenticator_data(rp_id, sign_count, false, None);
    let client_data_json = client_data("webauthn.get", challenge, origin);
    let mut msg = auth.clone();
    msg.extend_from_slice(&Sha256::digest(&client_data_json));
    let sig: Signature = sk.sign(&msg);
    Ok(Assertion {
        authenticator_data: auth,
        client_data_json,
        signature_der: sig.to_der().as_bytes().to_vec(),
    })
}

pub fn from_jwk(jwk_json: &str) -> Result<([u8; 32], Vec<u8>), String> {
    let v: Value = serde_json::from_str(jwk_json).map_err(|e| format!("passkey JWK: {e}"))?;
    from_jwk_value(&v)
}

/// EC P-256 JWK for the private key (matches 1Password `details.passkey.privateKey` payload).
pub fn to_jwk(private_raw: &[u8]) -> Result<Value, String> {
    let sk = SigningKey::from_slice(private_raw).map_err(|e| format!("passkey key: {e}"))?;
    let point = sk.verifying_key().to_encoded_point(false);
    let bytes = point.as_bytes();
    if bytes.len() != 65 || bytes[0] != 0x04 {
        return Err("unexpected public key encoding".into());
    }
    Ok(json!({
        "kty": "EC",
        "crv": "P-256",
        "d": crate::b64url_encode(private_raw),
        "x": crate::b64url_encode(&bytes[1..33]),
        "y": crate::b64url_encode(&bytes[33..65]),
        "alg": "ES256"
    }))
}

/// Base64url of the JWK JSON string — the encoding 1Password stores in `privateKey`.
pub fn to_jwk_private_key_field(private_raw: &[u8]) -> Result<String, String> {
    let jwk = to_jwk(private_raw)?;
    Ok(crate::b64url_encode(jwk.to_string().as_bytes()))
}

pub fn from_jwk_value(v: &Value) -> Result<([u8; 32], Vec<u8>), String> {
    let crv = v.get("crv").and_then(|x| x.as_str()).unwrap_or("");
    if crv != "P-256" && !crv.is_empty() {
        return Err(format!("unsupported passkey curve {crv}"));
    }
    let d = v.get("d").and_then(|x| x.as_str()).ok_or("passkey JWK missing d")?;
    let raw = crate::b64url_decode(d)?;
    if raw.len() != 32 {
        return Err(format!("passkey private key is {} bytes, want 32", raw.len()));
    }
    let mut private_raw = [0u8; 32];
    private_raw.copy_from_slice(&raw);
    let sk = SigningKey::from_slice(&private_raw).map_err(|e| format!("passkey key: {e}"))?;
    Ok((private_raw, cose_public(&sk)?.0))
}

pub fn default_origin(rp_id: &str) -> String {
    if rp_id == "localhost" || rp_id.starts_with("localhost:") {
        format!("http://{rp_id}")
    } else {
        format!("https://{rp_id}")
    }
}

pub fn decode_challenge(s: &str) -> Result<Vec<u8>, String> {
    let t = s.trim();
    if t.is_empty() {
        return Err("challenge is empty".into());
    }
    let b64ish = t.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '+' | '/' | '='));
    if b64ish && t.len() >= 16 {
        if let Ok(b) = crate::b64url_decode(t) {
            if b.len() >= 8 {
                return Ok(b);
            }
        }
    }
    if t.len() % 2 == 0 && t.len() >= 16 && t.chars().all(|c| c.is_ascii_hexdigit()) {
        let mut out = Vec::with_capacity(t.len() / 2);
        for i in (0..t.len()).step_by(2) {
            out.push(u8::from_str_radix(&t[i..i + 2], 16).map_err(|e| e.to_string())?);
        }
        return Ok(out);
    }
    Ok(t.as_bytes().to_vec())
}

fn generate_key() -> Result<SigningKey, String> {
    for _ in 0..8 {
        let b = random_bytes(32)?;
        if let Ok(sk) = SigningKey::from_slice(&b) {
            return Ok(sk);
        }
    }
    Err("could not generate a P-256 key".into())
}

/// P-256 SubjectPublicKeyInfo, the encoding BoringSSL's `i2d_PUBKEY` produces.
/// Chrome compares this byte-for-byte with the key it parses out of the COSE key.
fn spki_p256(x: &[u8], y: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(91);
    out.extend_from_slice(&[
        0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce,
        0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00, 0x04,
    ]);
    out.extend_from_slice(x);
    out.extend_from_slice(y);
    out
}

fn cose_public(sk: &SigningKey) -> Result<(Vec<u8>, Vec<u8>), String> {
    let point = sk.verifying_key().to_encoded_point(false);
    let bytes = point.as_bytes();
    if bytes.len() != 65 || bytes[0] != 0x04 {
        return Err("unexpected public key encoding".into());
    }
    let x = &bytes[1..33];
    let y = &bytes[33..65];
    let spki = spki_p256(x, y);
    // COSE_Key (EC2 / ES256 / P-256), canonical key order: 1, 3, -1, -2, -3
    let mut m = Vec::new();
    m.push(0xa5);
    cbor_uint(&mut m, 1);
    cbor_uint(&mut m, 2); // kty EC2
    cbor_uint(&mut m, 3);
    cbor_nint(&mut m, 7); // alg ES256 = -7
    cbor_nint(&mut m, 1); // crv key -1
    cbor_uint(&mut m, 1); // P-256
    cbor_nint(&mut m, 2); // x key -2
    cbor_bytes(&mut m, x);
    cbor_nint(&mut m, 3); // y key -3
    cbor_bytes(&mut m, y);
    Ok((m, spki))
}

fn authenticator_data(rp_id: &str, sign_count: u32, attest: bool, cred: Option<(&[u8], &[u8])>) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&Sha256::digest(rp_id.as_bytes()));
    let mut flags = 0x05u8; // UP | UV
    if attest {
        flags |= 0x40; // AT
    }
    out.push(flags);
    out.extend_from_slice(&sign_count.to_be_bytes());
    if let Some((id, cose)) = cred {
        out.extend_from_slice(&AAGUID);
        let n = u16::try_from(id.len()).unwrap_or(u16::MAX);
        out.extend_from_slice(&n.to_be_bytes());
        out.extend_from_slice(&id[..n as usize]);
        out.extend_from_slice(cose);
    }
    out
}

fn att_none(auth: &[u8]) -> Vec<u8> {
    let mut m = Vec::new();
    m.push(0xa3);
    cbor_text(&mut m, "fmt");
    cbor_text(&mut m, "none");
    cbor_text(&mut m, "attStmt");
    m.push(0xa0);
    cbor_text(&mut m, "authData");
    cbor_bytes(&mut m, auth);
    m
}

fn client_data(ty: &str, challenge: &[u8], origin: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "type": ty,
        "challenge": crate::b64url_encode(challenge),
        "origin": origin,
        "crossOrigin": false,
    }))
    .unwrap_or_else(|_| b"{}".to_vec())
}

pub fn random_bytes(n: usize) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut b = vec![0u8; n];
    std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b)).map_err(|e| format!("urandom: {e}"))?;
    Ok(b)
}

fn cbor_uint(buf: &mut Vec<u8>, n: u64) {
    cbor_head(buf, 0, n);
}
fn cbor_nint(buf: &mut Vec<u8>, n: u64) {
    // encode -n as CBOR negative: value = n-1
    cbor_head(buf, 1, n.saturating_sub(1));
}
fn cbor_bytes(buf: &mut Vec<u8>, b: &[u8]) {
    cbor_head(buf, 2, b.len() as u64);
    buf.extend_from_slice(b);
}
fn cbor_text(buf: &mut Vec<u8>, s: &str) {
    cbor_head(buf, 3, s.len() as u64);
    buf.extend_from_slice(s.as_bytes());
}
fn cbor_head(buf: &mut Vec<u8>, major: u8, n: u64) {
    if n < 24 {
        buf.push((major << 5) | n as u8);
    } else if n < 256 {
        buf.push((major << 5) | 24);
        buf.push(n as u8);
    } else if n < 65536 {
        buf.push((major << 5) | 25);
        buf.extend_from_slice(&(n as u16).to_be_bytes());
    } else {
        buf.push((major << 5) | 26);
        buf.extend_from_slice(&(n as u32).to_be_bytes());
    }
}

pub fn public_json(credential_id: &[u8], cose: &[u8], rp_id: &str, user_name: &str, sign_count: u32) -> Value {
    json!({
        "rp_id": rp_id,
        "user_name": user_name,
        "credential_id": crate::b64url_encode(credential_id),
        "cose_key": crate::b64url_encode(cose),
        "sign_count": sign_count,
        "alg": "ES256",
    })
}
