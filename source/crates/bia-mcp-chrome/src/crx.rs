//! The extension signing key and the extension id derived from it. Chrome itself packs the CRX (pack.rs).
use rsa::pkcs8::{DecodePrivateKey, EncodePublicKey};
use rsa::sha2::Sha256;
use rsa::RsaPrivateKey;
use sha2::Digest;
use std::fs;
use std::path::Path;

#[derive(Clone)]
pub struct Packed {
    pub extension_id: String,
    pub crx: Vec<u8>,
    pub version: String,
}

/// Fallback when `extension.pem` is missing beside the MCP. The same key ships as
/// `flash/MCPs/chrome/extension.pem`. The extension id is the hash of this key.
/// Public on purpose: passkey and other browser interactions are hard, so agents are
/// meant to improve the extension, repack it with this key (same id) and propose it upstream.
const SHIPPED_KEY: &str = include_str!("../extension.pem");

pub fn load_or_create_key(pem_path: &Path) -> Result<RsaPrivateKey, String> {
    if pem_path.is_file() {
        let text = fs::read_to_string(pem_path).map_err(|e| format!("read {}: {e}", pem_path.display()))?;
        let key = RsaPrivateKey::from_pkcs8_pem(&text).map_err(|e| format!("parse {}: {e}", pem_path.display()))?;
        lock_down(pem_path);
        return Ok(key);
    }
    if let Some(d) = pem_path.parent() {
        fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    fs::write(pem_path, SHIPPED_KEY).map_err(|e| format!("write key: {e}"))?;
    lock_down(pem_path);
    RsaPrivateKey::from_pkcs8_pem(SHIPPED_KEY).map_err(|e| format!("parse shipped key: {e}"))
}

fn lock_down(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    if let Some(s) = path.to_str() {
        let _ = std::process::Command::new("chown").args(["bia:bia", s]).status();
    }
}

pub fn extension_id(key: &RsaPrivateKey) -> Result<String, String> {
    let der = key.to_public_key().to_public_key_der().map_err(|e| format!("public key: {e}"))?;
    Ok(id_from_der(der.as_bytes()))
}

fn id_from_der(der: &[u8]) -> String {
    let hash = Sha256::digest(der);
    hash[..16].iter().flat_map(|b| [nibble(b >> 4), nibble(b & 0x0f)]).collect()
}

fn nibble(n: u8) -> char {
    (b'a' + n) as char
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_key_id() {
        let shipped = RsaPrivateKey::from_pkcs8_pem(SHIPPED_KEY).unwrap();
        assert_eq!(extension_id(&shipped).unwrap(), "oadmcnjaofpjifgeledkdmaidhfgbnmb");
    }

    #[test]
    fn mcp_pem_matches_embedded_key() {
        let mcp = include_str!("../../../flash/MCPs/chrome/extension.pem");
        assert_eq!(SHIPPED_KEY, mcp);
    }
}
