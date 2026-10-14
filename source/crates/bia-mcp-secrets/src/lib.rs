//! Password, OTP and WebAuthn passkey store. The chrome MCP signs ceremonies through this crate.
mod onep;
mod opcsv;
mod otp;
pub mod passkey;
pub mod store;

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;

pub(crate) fn b64_encode(b: &[u8]) -> String {
    STANDARD.encode(b)
}
pub(crate) fn b64_decode(s: &str) -> Result<Vec<u8>, String> {
    STANDARD.decode(s.trim()).or_else(|_| URL_SAFE_NO_PAD.decode(s.trim())).map_err(|e| format!("base64: {e}"))
}
pub(crate) fn b64url_encode(b: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(b)
}
pub(crate) fn b64url_decode(s: &str) -> Result<Vec<u8>, String> {
    let t = s.trim().trim_end_matches('=');
    URL_SAFE_NO_PAD.decode(t).or_else(|_| STANDARD.decode(s.trim())).map_err(|e| format!("base64url: {e}"))
}
