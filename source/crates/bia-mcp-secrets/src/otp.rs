//! RFC 4226 HOTP / RFC 6238 TOTP, plus otpauth:// import.
use hmac::{Hmac, Mac};
use sha1::Sha1;
use sha2::{Sha256, Sha512};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alg {
    Sha1,
    Sha256,
    Sha512,
}

impl Alg {
    pub fn parse(s: &str) -> Alg {
        match s.trim().to_ascii_uppercase().as_str() {
            "SHA256" | "SHA-256" => Alg::Sha256,
            "SHA512" | "SHA-512" => Alg::Sha512,
            // CXF OTPHashAlgorithm uses lowercase sha1/sha256/sha512
            _ => Alg::Sha1,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Alg::Sha1 => "SHA1",
            Alg::Sha256 => "SHA256",
            Alg::Sha512 => "SHA512",
        }
    }
}

#[derive(Clone, Debug)]
pub struct OtpParams {
    pub secret_b32: String,
    pub digits: u32,
    pub period: u32,
    pub algorithm: Alg,
    pub counter: u64,
    pub issuer: String,
    pub account: String,
    pub hotp: bool,
}

impl Default for OtpParams {
    fn default() -> Self {
        Self {
            secret_b32: String::new(),
            digits: 6,
            period: 30,
            algorithm: Alg::Sha1,
            counter: 0,
            issuer: String::new(),
            account: String::new(),
            hotp: false,
        }
    }
}

pub fn generate(secret: &[u8], counter: u64, digits: u32, alg: Alg) -> Result<String, String> {
    if !(4..=10).contains(&digits) {
        return Err("digits must be 4–10".into());
    }
    let msg = counter.to_be_bytes();
    let digest: Vec<u8> = match alg {
        Alg::Sha1 => {
            let mut mac = <Hmac<Sha1> as Mac>::new_from_slice(secret).map_err(|e| e.to_string())?;
            mac.update(&msg);
            mac.finalize().into_bytes().to_vec()
        }
        Alg::Sha256 => {
            let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(secret).map_err(|e| e.to_string())?;
            mac.update(&msg);
            mac.finalize().into_bytes().to_vec()
        }
        Alg::Sha512 => {
            let mut mac = <Hmac<Sha512> as Mac>::new_from_slice(secret).map_err(|e| e.to_string())?;
            mac.update(&msg);
            mac.finalize().into_bytes().to_vec()
        }
    };
    let offset = (digest[digest.len() - 1] & 0x0f) as usize;
    let bin = u32::from_be_bytes(digest[offset..offset + 4].try_into().unwrap()) & 0x7fff_ffff;
    let wrap = 10u32.pow(digits);
    Ok(format!("{:0width$}", bin % wrap, width = digits as usize))
}

pub fn totp_now(secret: &[u8], period: u32, digits: u32, alg: Alg) -> Result<(String, u32), String> {
    let period = period.max(1);
    let now = unix_now();
    let counter = now / u64::from(period);
    let remaining = period - (now % u64::from(period)) as u32;
    Ok((generate(secret, counter, digits, alg)?, remaining))
}

pub fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Decode RFC 4648 base32 (padding optional, whitespace ignored).
pub fn decode_b32(s: &str) -> Result<Vec<u8>, String> {
    let mut bits: u32 = 0;
    let mut nbits: u32 = 0;
    let mut out = Vec::new();
    for c in s.chars() {
        if c.is_whitespace() || c == '=' {
            continue;
        }
        let v = match c {
            'A'..='Z' => c as u32 - 'A' as u32,
            'a'..='z' => c as u32 - 'a' as u32,
            '2'..='7' => c as u32 - '2' as u32 + 26,
            '0' => 14, // some QR codes confuse O/0
            '1' => 8,  // some QR codes confuse I/1/l
            _ => return Err(format!("invalid base32 character {c:?}")),
        };
        bits = (bits << 5) | v;
        nbits += 5;
        if nbits >= 8 {
            nbits -= 8;
            out.push((bits >> nbits) as u8);
            bits &= (1 << nbits) - 1;
        }
    }
    if out.is_empty() {
        return Err("empty OTP secret".into());
    }
    Ok(out)
}

pub fn format_otpauth(p: &OtpParams) -> String {
    let kind = if p.hotp { "hotp" } else { "totp" };
    let label = if !p.issuer.is_empty() && !p.account.is_empty() {
        format!("{}:{}", urlencoding_encode(&p.issuer), urlencoding_encode(&p.account))
    } else if !p.account.is_empty() {
        urlencoding_encode(&p.account)
    } else if !p.issuer.is_empty() {
        urlencoding_encode(&p.issuer)
    } else {
        "account".into()
    };
    let mut q = format!("secret={}", p.secret_b32.replace(' ', ""));
    if !p.issuer.is_empty() {
        q.push_str(&format!("&issuer={}", urlencoding_encode(&p.issuer)));
    }
    if p.digits != 6 {
        q.push_str(&format!("&digits={}", p.digits));
    }
    if !p.hotp && p.period != 30 {
        q.push_str(&format!("&period={}", p.period));
    }
    if p.algorithm != Alg::Sha1 {
        q.push_str(&format!("&algorithm={}", p.algorithm.as_str()));
    }
    if p.hotp {
        q.push_str(&format!("&counter={}", p.counter));
    }
    format!("otpauth://{kind}/{label}?{q}")
}

fn urlencoding_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => out.push(b as char),
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn parse_otpauth(url: &str) -> Result<OtpParams, String> {
    let rest = url.strip_prefix("otpauth://").ok_or("not an otpauth URL")?;
    let (kind, rest) = rest.split_once('/').ok_or("otpauth URL missing type")?;
    let hotp = match kind.to_ascii_lowercase().as_str() {
        "totp" => false,
        "hotp" => true,
        other => return Err(format!("unsupported otpauth type {other}")),
    };
    let (label, query) = rest.split_once('?').unwrap_or((rest, ""));
    let label = urlencoding_decode(label);
    let (issuer_from_label, account) = match label.split_once(':') {
        Some((a, b)) => (a.trim().to_string(), b.trim().to_string()),
        None => (String::new(), label.trim().to_string()),
    };
    let mut p = OtpParams { hotp, account, issuer: issuer_from_label, ..Default::default() };
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let v = urlencoding_decode(v);
        match k {
            "secret" => p.secret_b32 = v.replace(' ', "").to_ascii_uppercase(),
            "issuer" => p.issuer = v,
            "digits" => p.digits = v.parse().unwrap_or(6),
            "period" => p.period = v.parse().unwrap_or(30),
            "algorithm" => p.algorithm = Alg::parse(&v),
            "counter" => p.counter = v.parse().unwrap_or(0),
            _ => {}
        }
    }
    if p.secret_b32.is_empty() {
        return Err("otpauth URL has no secret".into());
    }
    let _ = decode_b32(&p.secret_b32)?;
    Ok(p)
}

fn urlencoding_decode(s: &str) -> String {
    let mut out = String::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or(""), 16) {
                out.push(v as char);
                i += 3;
                continue;
            }
        }
        out.push(if b[i] == b'+' { ' ' } else { b[i] as char });
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc4226_hotp() {
        let secret = b"12345678901234567890";
        assert_eq!(generate(secret, 0, 6, Alg::Sha1).unwrap(), "755224");
        assert_eq!(generate(secret, 1, 6, Alg::Sha1).unwrap(), "287082");
    }

    #[test]
    fn otpauth_totp() {
        let p = parse_otpauth("otpauth://totp/GitHub:octocat?secret=JBSWY3DPEHPK3PXP&issuer=GitHub&digits=6&period=30").unwrap();
        assert!(!p.hotp);
        assert_eq!(p.issuer, "GitHub");
        assert_eq!(p.account, "octocat");
        assert_eq!(p.secret_b32, "JBSWY3DPEHPK3PXP");
        let again = parse_otpauth(&format_otpauth(&p)).unwrap();
        assert_eq!(again.secret_b32, p.secret_b32);
        assert_eq!(again.issuer, p.issuer);
        assert_eq!(again.account, p.account);
    }
}
