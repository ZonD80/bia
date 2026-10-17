//! Legacy 1Password unencrypted JSON import (single item, item array, or accounts/vaults export).
//! Export uses CXF; this module stays for backwards-compatible import only.
use serde_json::{json, Value};

use crate::otp;
use crate::passkey;
use crate::store::{new_uuid, Entry};

pub fn collect_items(v: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    walk(v, &mut out);
    out
}

fn walk(v: &Value, out: &mut Vec<Value>) {
    match v {
        Value::Array(a) => {
            for x in a {
                walk(x, out);
            }
        }
        Value::Object(m) => {
            if is_item(v) {
                out.push(v.clone());
                return;
            }
            for k in ["items", "vaults", "accounts", "rows"] {
                if let Some(x) = m.get(k) {
                    walk(x, out);
                }
            }
            if out.is_empty() {
                for x in m.values() {
                    if x.is_array() || x.is_object() {
                        walk(x, out);
                    }
                }
            }
        }
        _ => {}
    }
}

fn is_item(v: &Value) -> bool {
    let has_overview = v.get("overview").is_some();
    let has_details = v.get("details").is_some();
    let has_fields = v.pointer("/details/fields").and_then(|x| x.as_array()).map(|a| !a.is_empty()).unwrap_or(false)
        || v.pointer("/details/loginFields").and_then(|x| x.as_array()).map(|a| !a.is_empty()).unwrap_or(false)
        || v.get("fields").and_then(|x| x.as_array()).map(|a| !a.is_empty()).unwrap_or(false)
        || v.pointer("/details/sections").and_then(|x| x.as_array()).map(|a| !a.is_empty()).unwrap_or(false)
        || v.pointer("/details/passkey").is_some()
        || v.pointer("/overview/passkey").is_some();
    (has_overview && has_details) || (has_details && has_fields) || (v.get("uuid").is_some() && has_fields)
}

pub fn entries_from_item(item: &Value, force_note: &mut Vec<String>) -> Result<Vec<Entry>, String> {
    let title = item
        .pointer("/overview/title")
        .or_else(|| item.get("title"))
        .and_then(|x| x.as_str())
        .unwrap_or("item")
        .trim();
    let title = if title.is_empty() { "item" } else { title };
    let ainfo = item
        .pointer("/overview/ainfo")
        .or_else(|| item.pointer("/overview/subtitle"))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim();
    let mut username = String::new();
    let mut password = String::new();
    let mut totps: Vec<String> = Vec::new();

    for f in fields(item) {
        let designation = f.get("designation").and_then(|x| x.as_str()).unwrap_or("");
        let name = f.get("name").and_then(|x| x.as_str()).unwrap_or("");
        let ty = f
            .get("type")
            .or_else(|| f.get("fieldType"))
            .and_then(|x| x.as_str())
            .unwrap_or("");
        let val = f.get("value").and_then(|x| x.as_str()).unwrap_or("").to_string();
        if val.is_empty() {
            continue;
        }
        if designation == "username" || name.eq_ignore_ascii_case("username") {
            username = val;
        } else if designation == "password" || name.eq_ignore_ascii_case("password") || ty == "P" {
            password = val;
        }
    }
    for f in section_fields(item) {
        // Legacy OP JSON: t/n/k/v. Some exports: title/id/value.{totp|concealed}.
        let t = f
            .get("t")
            .or_else(|| f.get("title"))
            .and_then(|x| x.as_str())
            .unwrap_or("");
        let n = f
            .get("n")
            .or_else(|| f.get("id"))
            .and_then(|x| x.as_str())
            .unwrap_or("");
        let k = f.get("k").and_then(|x| x.as_str()).unwrap_or("");
        let v = section_field_text(&f);
        if v.is_empty() {
            continue;
        }
        let otpish = t.to_ascii_lowercase().contains("one-time")
            || t.to_ascii_lowercase().contains("totp")
            || n.to_ascii_uppercase().starts_with("TOTP")
            || f.pointer("/value/totp").is_some()
            || (k == "concealed" && (v.starts_with("otpauth://") || looks_like_b32(&v)));
        if otpish {
            totps.push(v);
        }
    }

    let rp_id = item
        .pointer("/overview/passkey/rpId")
        .or_else(|| item.pointer("/details/passkey/rpId"))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let cred_id = item.pointer("/overview/passkey/credentialId").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let user_handle = item
        .pointer("/overview/passkey/userHandle")
        .or_else(|| item.pointer("/details/passkey/userHandle"))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let jwk_b64 = item.pointer("/details/passkey/privateKey").and_then(|x| x.as_str()).unwrap_or("");

    let account = if !username.is_empty() {
        username.clone()
    } else {
        ainfo.to_string()
    };

    let mut out = Vec::new();
    if !username.is_empty() && username != password {
        out.push(string_entry(&format!("{title} username"), &username, &account)?);
    }
    if !password.is_empty() {
        out.push(string_entry(&format!("{title} password"), &password, &account)?);
    }
    for (i, seed) in totps.into_iter().enumerate() {
        let desc = if i == 0 { format!("{title} TOTP") } else { format!("{title} TOTP {}", i + 1) };
        match otp_entry(&desc, &seed, &account) {
            Ok(e) => out.push(e),
            Err(e) => force_note.push(format!("{desc}: {e}")),
        }
    }
    if !jwk_b64.is_empty() && !rp_id.is_empty() {
        match passkey_entry(title, &rp_id, &account, &cred_id, &user_handle, jwk_b64) {
            Ok(e) => out.push(e),
            Err(e) => force_note.push(format!("{title} passkey: {e}")),
        }
    }
    if out.is_empty() {
        return Err(format!("1Password item {title:?} had no password, TOTP or passkey"));
    }
    Ok(out)
}

fn fields(item: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    if let Some(a) = item.pointer("/details/loginFields").and_then(|x| x.as_array()) {
        out.extend(a.iter().cloned());
    }
    if let Some(a) = item.pointer("/details/fields").and_then(|x| x.as_array()) {
        out.extend(a.iter().cloned());
    }
    if let Some(a) = item.get("fields").and_then(|x| x.as_array()) {
        out.extend(a.iter().cloned());
    }
    out
}

fn section_fields(item: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    if let Some(secs) = item.pointer("/details/sections").and_then(|x| x.as_array()) {
        for s in secs {
            if let Some(fs) = s.get("fields").and_then(|x| x.as_array()) {
                out.extend(fs.iter().cloned());
            }
        }
    }
    out
}

fn section_field_text(f: &Value) -> String {
    if let Some(s) = f.get("v").and_then(|x| x.as_str()) {
        return s.trim().to_string();
    }
    match f.get("value") {
        Some(Value::String(s)) => s.trim().to_string(),
        Some(Value::Object(m)) => m
            .get("totp")
            .or_else(|| m.get("concealed"))
            .or_else(|| m.get("string"))
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .trim()
            .to_string(),
        _ => String::new(),
    }
}

fn looks_like_b32(s: &str) -> bool {
    let t = s.replace(' ', "");
    t.len() >= 8 && t.chars().all(|c| matches!(c, 'A'..='Z' | 'a'..='z' | '2'..='7' | '='))
}

fn string_entry(desc: &str, value: &str, account: &str) -> Result<Entry, String> {
    Ok(Entry {
        uuid: new_uuid()?,
        description: desc.into(),
        kind: "string".into(),
        value: value.into(),
        digits: None,
        period: None,
        algorithm: None,
        counter: None,
        issuer: None,
        account: nonempty(account),
        rp_id: None,
        user_name: None,
        user_id: None,
        credential_id: None,
        cose_key: None,
        sign_count: None,
    })
}

fn otp_entry(desc: &str, seed: &str, account: &str) -> Result<Entry, String> {
    if seed.starts_with("otpauth://") {
        let p = otp::parse_otpauth(seed)?;
        return Ok(Entry {
            uuid: new_uuid()?,
            description: desc.into(),
            kind: if p.hotp { "hotp".into() } else { "totp".into() },
            value: p.secret_b32,
            digits: Some(p.digits),
            period: if p.hotp { None } else { Some(p.period) },
            algorithm: Some(p.algorithm.as_str().into()),
            counter: if p.hotp { Some(p.counter) } else { None },
            issuer: nonempty(&p.issuer),
            account: nonempty(if p.account.is_empty() { account } else { &p.account }),
            rp_id: None,
            user_name: None,
            user_id: None,
            credential_id: None,
            cose_key: None,
            sign_count: None,
        });
    }
    let secret_b32 = seed.replace(' ', "").to_ascii_uppercase();
    let _ = otp::decode_b32(&secret_b32)?;
    Ok(Entry {
        uuid: new_uuid()?,
        description: desc.into(),
        kind: "totp".into(),
        value: secret_b32,
        digits: Some(6),
        period: Some(30),
        algorithm: Some("SHA1".into()),
        counter: None,
        issuer: None,
        account: nonempty(account),
        rp_id: None,
        user_name: None,
        user_id: None,
        credential_id: None,
        cose_key: None,
        sign_count: None,
    })
}

fn passkey_entry(title: &str, rp_id: &str, user_name: &str, cred_id: &str, user_handle: &str, jwk_b64: &str) -> Result<Entry, String> {
    let raw = crate::b64url_decode(jwk_b64).or_else(|_| crate::b64_decode(jwk_b64))?;
    let jwk = std::str::from_utf8(&raw).map_err(|_| "passkey privateKey is not JSON".to_string())?;
    let (private_raw, cose) = passkey::from_jwk(jwk)?;
    let cred = if cred_id.is_empty() {
        crate::b64url_encode(&passkey::random_bytes(32)?)
    } else {
        cred_id.trim().to_string()
    };
    Ok(Entry {
        uuid: new_uuid()?,
        description: format!("{title} passkey"),
        kind: "passkey".into(),
        value: crate::b64_encode(&private_raw),
        digits: None,
        period: None,
        algorithm: Some("ES256".into()),
        counter: None,
        issuer: None,
        account: nonempty(user_name),
        rp_id: Some(rp_id.into()),
        user_name: nonempty(user_name),
        user_id: nonempty(user_handle),
        credential_id: Some(cred),
        cose_key: Some(crate::b64url_encode(&cose)),
        sign_count: Some(0),
    })
}

fn nonempty(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() { None } else { Some(s.into()) }
}

pub fn summary(created: &[Entry], notes: &[String]) -> String {
    let rows: Vec<Value> = created.iter().map(|e| json!({"uuid": e.uuid, "description": e.description, "type": e.kind})).collect();
    json!({"imported": rows.len(), "items": rows, "notes": notes, "format": "1password-json"}).to_string()
}

pub fn parse_json_bytes(bytes: &[u8]) -> Result<Value, String> {
    let trimmed = if bytes.starts_with(&[0xef, 0xbb, 0xbf]) { &bytes[3..] } else { bytes };
    let text = std::str::from_utf8(trimmed).map_err(|e| format!("not UTF-8 JSON: {e}"))?;
    serde_json::from_str(text.trim()).map_err(|e| format!("not JSON: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_1password_item() {
        let v: Value = serde_json::from_str(r#"{
            "overview": {"title": "Gmail", "ainfo": "a@b.c", "passkey": {"credentialId": "abc", "rpId": "google.com", "userHandle": "QQ"}},
            "details": {
                "fields": [
                    {"value": "a@b.c", "name": "username", "type": "T", "designation": "username"},
                    {"value": "secret", "name": "password", "type": "P", "designation": "password"}
                ],
                "sections": [{"fields": [{"t": "one-time password", "n": "TOTP_x", "k": "concealed", "v": "JBSWY3DPEHPK3PXP"}]}],
                "passkey": {"type": "webauthn", "privateKey": "eyJrdHkiOiJFQyIsImNydiI6IlAtMjU2IiwiZCI6IlM0QzRrRVN0QlBCUVVnak5xZ2ljWmhJVHZuc1RpeDMxUnFmQ05ETDB6X3ciLCJ4IjoiYW52OXQ5UFI0STNkUWREVUVEY1I3X29LeWJJOVdsUFlERXI2SHhHelB6YyIsInkiOiJDdDVqVlMyUGI2cmptQXNuRTFmLURTZzUyNG1EUGxDWnZmbzBxUFB0d29FIiwiYWxnIjoiRVMyNTYifQ", "userHandle": "QQ"}
            }
        }"#).unwrap();
        let items = collect_items(&v);
        assert_eq!(items.len(), 1);
        let mut notes = vec![];
        let ents = entries_from_item(&items[0], &mut notes).unwrap();
        assert!(ents.iter().any(|e| e.kind == "string" && e.description.contains("password")), "{ents:?}");
        assert!(ents.iter().any(|e| e.kind == "totp"), "{ents:?}");
        assert!(ents.iter().any(|e| e.kind == "passkey" && e.rp_id.as_deref() == Some("google.com")), "{ents:?}");
    }

    /// A real 1Password export (personal data, never in git):
    /// cargo test -p bia-mcp-secrets -- --ignored sample_file_gmail
    #[test]
    #[ignore = "needs source/1password_sample.json, a real export that is not in git"]
    fn sample_file_gmail() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../1password_sample.json");
        let text = std::fs::read_to_string(&p).expect("1password_sample.json");
        let v: Value = serde_json::from_str(&text).unwrap();
        let items = collect_items(&v);
        assert_eq!(items.len(), 1);
        let mut notes = vec![];
        let ents = entries_from_item(&items[0], &mut notes).unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        assert!(ents.iter().any(|e| e.description == "Gmail username"));
        assert!(ents.iter().any(|e| e.description == "Gmail password" && e.value.contains('@')));
        assert!(ents.iter().any(|e| e.kind == "totp"));
        assert!(ents.iter().any(|e| e.kind == "passkey" && e.rp_id.as_deref() == Some("google.com")));
    }
}
