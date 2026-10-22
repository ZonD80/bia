//! Yaaia-compatible `passwords.json` (v2), extended with HOTP and passkeys (v3).
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs::{self, File};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

use crate::otp::{self, Alg, OtpParams};
use crate::passkey;

const VERSION: u32 = 3;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub uuid: String,
    pub description: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digits: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub period: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub algorithm: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counter: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issuer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rp_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cose_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sign_count: Option<u32>,
}

impl Entry {
    fn public(&self) -> Value {
        let mut o = json!({
            "uuid": self.uuid,
            "description": self.description,
            "type": self.kind,
        });
        if let Some(s) = &self.issuer {
            if !s.is_empty() {
                o["issuer"] = json!(s);
            }
        }
        if let Some(s) = &self.account {
            if !s.is_empty() {
                o["account"] = json!(s);
            }
        }
        if let Some(s) = &self.rp_id {
            o["rp_id"] = json!(s);
        }
        if let Some(s) = &self.user_name {
            o["user_name"] = json!(s);
        }
        if let Some(s) = &self.credential_id {
            o["credential_id"] = json!(s);
        }
        if let Some(n) = self.sign_count {
            o["sign_count"] = json!(n);
        }
        o
    }
}

fn path() -> PathBuf {
    let files = std::env::var("BIA_FILES").unwrap_or_else(|_| "/home/bia/files".into());
    PathBuf::from(files).join("state").join("passwords.json")
}

/// Root writes it here, but backups made as user bia must read it: hand it to the owner of
/// `files` (the agent gives all of /home/bia to bia at boot anyway).
fn own_like_files(p: &Path, files: &Path) {
    use std::os::unix::fs::MetadataExt;
    if let Ok(m) = std::fs::metadata(files) {
        let _ = std::os::unix::fs::chown(p, Some(m.uid()), Some(m.gid()));
    }
}

/// Exclusive lock so the secrets MCP and the chrome MCP can share passwords.json.
struct StoreLock(File);

impl StoreLock {
    fn acquire() -> Result<Self, String> {
        let p = path().with_extension("lock");
        if let Some(d) = p.parent() {
            fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let f = File::options().create(true).read(true).write(true).open(&p).map_err(|e| format!("lock: {e}"))?;
        let rc = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) };
        if rc != 0 {
            return Err(format!("lock passwords.json: {}", std::io::Error::last_os_error()));
        }
        Ok(StoreLock(f))
    }
}

impl Drop for StoreLock {
    fn drop(&mut self) {
        unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN) };
    }
}

fn load() -> Result<Vec<Entry>, String> {
    let p = path();
    let bytes = match fs::read(&p) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(e.to_string()),
    };
    let v: Value = serde_json::from_slice(&bytes).map_err(|e| format!("passwords.json: {e}"))?;
    let items = v.get("items").and_then(|i| i.as_array()).cloned().unwrap_or_default();
    let mut out = Vec::new();
    for item in items {
        if let Ok(e) = serde_json::from_value::<Entry>(item) {
            if !e.uuid.is_empty() && !e.description.is_empty() {
                out.push(e);
            }
        }
    }
    Ok(out)
}

fn save(items: &[Entry]) -> Result<(), String> {
    let p = path();
    if let Some(d) = p.parent() {
        fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let tmp = p.with_extension("json.tmp");
    {
        let mut f = fs::File::create(&tmp).map_err(|e| e.to_string())?;
        let data = json!({"v": VERSION, "items": items});
        f.write_all(serde_json::to_vec_pretty(&data).map_err(|e| e.to_string())?.as_slice()).map_err(|e| e.to_string())?;
        f.write_all(b"\n").map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())?;
    }
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600)).ok();
    if let Some(files) = p.parent().and_then(Path::parent) {
        own_like_files(&tmp, files);
    }
    fs::rename(&tmp, &p).or_else(|_| fs::copy(&tmp, &p).map(|_| ())).map_err(|e| e.to_string())?;
    let _ = fs::remove_file(&tmp);
    unsafe { libc::sync() };
    Ok(())
}

fn is_uuid(s: &str) -> bool {
    let s = s.trim();
    if s.len() != 36 {
        return false;
    }
    let b = s.as_bytes();
    b[8] == b'-' && b[13] == b'-' && b[18] == b'-' && b[23] == b'-' && s.bytes().enumerate().all(|(i, c)| matches!(i, 8 | 13 | 18 | 23) || c.is_ascii_hexdigit())
}

pub(crate) fn new_uuid() -> Result<String, String> {
    let b = passkey::random_bytes(16)?;
    let mut b = b;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    Ok(format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
    ))
}

pub fn find<'a>(items: &'a [Entry], key: &str) -> Result<&'a Entry, String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("id or description is required".into());
    }
    if is_uuid(key) {
        return items.iter().find(|e| e.uuid.eq_ignore_ascii_case(key)).ok_or_else(|| format!("no secret {key}"));
    }
    let exact: Vec<_> = items.iter().filter(|e| e.description == key).collect();
    if exact.len() == 1 {
        return Ok(exact[0]);
    }
    if exact.len() > 1 {
        return Err(format!("several secrets named {key:?}; use uuid"));
    }
    let ci: Vec<_> = items.iter().filter(|e| e.description.eq_ignore_ascii_case(key)).collect();
    match ci.len() {
        1 => Ok(ci[0]),
        0 => Err(format!("no secret {key:?}")),
        _ => Err(format!("several secrets named {key:?}; use uuid")),
    }
}

pub fn list() -> Result<String, String> {
    let _lock = StoreLock::acquire()?;
    let items = load()?;
    Ok(Value::Array(items.iter().map(Entry::public).collect()).to_string())
}

pub fn get(key: &str, raw: bool) -> Result<String, String> {
    let _lock = StoreLock::acquire()?;
    let mut items = load()?;
    let idx = {
        let e = find(&items, key)?;
        items.iter().position(|x| x.uuid == e.uuid).unwrap()
    };
    if raw {
        return Ok(items[idx].value.clone());
    }
    match items[idx].kind.as_str() {
        "totp" => {
            let secret = otp::decode_b32(&items[idx].value)?;
            let digits = items[idx].digits.unwrap_or(6);
            let period = items[idx].period.unwrap_or(30);
            let alg = Alg::parse(items[idx].algorithm.as_deref().unwrap_or("SHA1"));
            let (code, remaining) = otp::totp_now(&secret, period, digits, alg)?;
            Ok(format!("{code} (TOTP, {remaining}s remaining)"))
        }
        "hotp" => {
            let secret = otp::decode_b32(&items[idx].value)?;
            let digits = items[idx].digits.unwrap_or(6);
            let alg = Alg::parse(items[idx].algorithm.as_deref().unwrap_or("SHA1"));
            let counter = items[idx].counter.unwrap_or(0);
            let code = otp::generate(&secret, counter, digits, alg)?;
            items[idx].counter = Some(counter.saturating_add(1));
            save(&items)?;
            Ok(format!("{code} (HOTP, next counter {})", counter + 1))
        }
        "passkey" => {
            let e = &items[idx];
            Ok(passkey::public_json(
                &crate::b64url_decode(e.credential_id.as_deref().unwrap_or("")).unwrap_or_default(),
                &crate::b64url_decode(e.cose_key.as_deref().unwrap_or("")).unwrap_or_default(),
                e.rp_id.as_deref().unwrap_or(""),
                e.user_name.as_deref().unwrap_or(""),
                e.sign_count.unwrap_or(0),
            )
            .to_string())
        }
        _ => Ok(items[idx].value.clone()),
    }
}

pub fn set(description: &str, kind: &str, value: &str, force: bool) -> Result<String, String> {
    let _lock = StoreLock::acquire()?;
    let description = description.trim();
    if description.is_empty() {
        return Err("description is required".into());
    }
    let value = value.trim();
    if value.is_empty() {
        return Err("value is required".into());
    }
    let mut items = load()?;
    if let Some(existing) = items.iter().find(|e| e.description == description) {
        if !force {
            return Err(format!("Secret \"{description}\" already exists. Use force=true to overwrite."));
        }
        let uuid = existing.uuid.clone();
        let keep_count = if existing.kind == "passkey" { existing.sign_count } else { None };
        items.retain(|e| e.uuid != uuid);
        let mut entry = build_entry(uuid, description, kind, value)?;
        if entry.kind == "passkey" {
            entry.sign_count = keep_count;
        }
        items.push(entry);
        save(&items)?;
        return Ok(items.last().unwrap().uuid.clone());
    }
    let entry = build_entry(new_uuid()?, description, kind, value)?;
    let uuid = entry.uuid.clone();
    items.push(entry);
    save(&items)?;
    Ok(uuid)
}

fn build_entry(uuid: String, description: &str, kind: &str, value: &str) -> Result<Entry, String> {
    let mut kind = kind.trim().to_ascii_lowercase();
    if kind.is_empty() {
        kind = if value.starts_with("otpauth://") { String::new() } else { "string".into() };
    }
    if value.starts_with("otpauth://") {
        let p = otp::parse_otpauth(value)?;
        return Ok(entry_from_otp(uuid, description, p));
    }
    match kind.as_str() {
        "totp" | "hotp" => {
            let p = OtpParams { secret_b32: value.replace(' ', "").to_ascii_uppercase(), hotp: kind == "hotp", ..Default::default() };
            let _ = otp::decode_b32(&p.secret_b32)?;
            Ok(entry_from_otp(uuid, description, p))
        }
        "passkey" => Err("use passkey_create to make a WebAuthn credential (do not paste a password as type=passkey)".into()),
        _ => Ok(Entry {
            uuid,
            description: description.into(),
            kind: "string".into(),
            value: value.into(),
            digits: None,
            period: None,
            algorithm: None,
            counter: None,
            issuer: None,
            account: None,
            rp_id: None,
            user_name: None,
            user_id: None,
            credential_id: None,
            cose_key: None,
            sign_count: None,
        }),
    }
}

fn entry_from_otp(uuid: String, description: &str, p: OtpParams) -> Entry {
    Entry {
        uuid,
        description: description.into(),
        kind: if p.hotp { "hotp".into() } else { "totp".into() },
        value: p.secret_b32,
        digits: Some(p.digits),
        period: if p.hotp { None } else { Some(p.period) },
        algorithm: Some(p.algorithm.as_str().into()),
        counter: if p.hotp { Some(p.counter) } else { None },
        issuer: if p.issuer.is_empty() { None } else { Some(p.issuer) },
        account: if p.account.is_empty() { None } else { Some(p.account) },
        rp_id: None,
        user_name: None,
        user_id: None,
        credential_id: None,
        cose_key: None,
        sign_count: None,
    }
}

pub fn delete(key: &str) -> Result<String, String> {
    let _lock = StoreLock::acquire()?;
    let mut items = load()?;
    let uuid = find(&items, key)?.uuid.clone();
    items.retain(|e| e.uuid != uuid);
    save(&items)?;
    Ok(format!("removed {uuid}"))
}

pub fn passkey_create(description: &str, rp_id: &str, user_name: &str, user_id: Option<&str>, challenge: Option<&str>, origin: Option<&str>, force: bool) -> Result<String, String> {
    let ch = match challenge {
        Some(s) if !s.trim().is_empty() => Some(passkey::decode_challenge(s)?),
        _ => None,
    };
    passkey_create_with(description, rp_id, user_name, user_id, ch, origin, force)
}

fn passkey_create_with(description: &str, rp_id: &str, user_name: &str, user_id: Option<&str>, challenge: Option<Vec<u8>>, origin: Option<&str>, force: bool) -> Result<String, String> {
    let _lock = StoreLock::acquire()?;
    let description = description.trim();
    let rp_id = rp_id.trim();
    if description.is_empty() {
        return Err("description is required".into());
    }
    if rp_id.is_empty() {
        return Err("rp_id is required (the site hostname, e.g. github.com)".into());
    }
    let user_name = if user_name.trim().is_empty() { "bia" } else { user_name.trim() };
    let mut items = load()?;
    if let Some(existing) = items.iter().find(|e| e.description == description) {
        if !force {
            return Err(format!("Secret \"{description}\" already exists. Use force=true to overwrite."));
        }
        let uuid = existing.uuid.clone();
        items.retain(|e| e.uuid != uuid);
    }
    let ch = match challenge {
        Some(c) => c,
        None => passkey::random_bytes(32)?,
    };
    let origin = origin.filter(|s| !s.trim().is_empty()).map(|s| s.trim().to_string()).unwrap_or_else(|| passkey::default_origin(rp_id));
    let uid = match user_id {
        Some(s) if !s.trim().is_empty() => Some(crate::b64url_decode(s).unwrap_or_else(|_| s.as_bytes().to_vec())),
        _ => None,
    };
    let created = passkey::create(rp_id, user_name, uid.as_deref(), &ch, &origin)?;
    let uuid = new_uuid()?;
    let cred_id = crate::b64url_encode(&created.credential_id);
    items.push(Entry {
        uuid: uuid.clone(),
        description: description.into(),
        kind: "passkey".into(),
        value: crate::b64_encode(&created.private_raw),
        digits: None,
        period: None,
        algorithm: Some("ES256".into()),
        counter: None,
        issuer: None,
        account: None,
        rp_id: Some(rp_id.into()),
        user_name: Some(user_name.into()),
        user_id: Some(crate::b64url_encode(&created.user_id)),
        credential_id: Some(cred_id.clone()),
        cose_key: Some(crate::b64url_encode(&created.cose_key)),
        sign_count: Some(0),
    });
    save(&items)?;
    Ok(json!({
        "uuid": uuid,
        "description": description,
        "type": "passkey",
        "rp_id": rp_id,
        "user_name": user_name,
        "user_id": crate::b64url_encode(&created.user_id),
        "credential_id": cred_id,
        "cose_key": crate::b64url_encode(&created.cose_key),
        "attestationObject": crate::b64url_encode(&created.attestation_object),
        "authenticatorData": crate::b64url_encode(&created.authenticator_data),
        "publicKey": crate::b64url_encode(&created.public_key_spki),
        "publicKeyAlgorithm": -7,
        "clientDataJSON": crate::b64url_encode(&created.client_data_json),
        "origin": origin,
        "alg": "ES256",
        "transports": ["internal"],
    })
    .to_string())
}

pub fn passkey_assert(key: &str, challenge: &str, origin: Option<&str>, rp_id: Option<&str>) -> Result<String, String> {
    passkey_assert_with(key, &passkey::decode_challenge(challenge)?, origin, rp_id)
}

fn passkey_assert_with(key: &str, ch: &[u8], origin: Option<&str>, rp_id: Option<&str>) -> Result<String, String> {
    let _lock = StoreLock::acquire()?;
    let mut items = load()?;
    let idx = {
        let e = find(&items, key)?;
        if e.kind != "passkey" {
            return Err(format!("secret is type {}, not passkey", e.kind));
        }
        items.iter().position(|x| x.uuid == e.uuid).unwrap()
    };
    let rp = rp_id
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string())
        .or_else(|| items[idx].rp_id.clone())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "rp_id missing on this passkey".to_string())?;
    let origin = origin.filter(|s| !s.trim().is_empty()).map(|s| s.trim().to_string()).unwrap_or_else(|| passkey::default_origin(&rp));
    let priv_bytes = crate::b64_decode(&items[idx].value)?;
    let next = items[idx].sign_count.unwrap_or(0).saturating_add(1);
    let assertion = passkey::assert(&priv_bytes, &rp, next, ch, &origin)?;
    items[idx].sign_count = Some(next);
    save(&items)?;
    Ok(json!({
        "uuid": items[idx].uuid,
        "rp_id": rp,
        "origin": origin,
        "credential_id": items[idx].credential_id,
        "authenticatorData": crate::b64url_encode(&assertion.authenticator_data),
        "clientDataJSON": crate::b64url_encode(&assertion.client_data_json),
        "signature": crate::b64url_encode(&assertion.signature_der),
        "userHandle": items[idx].user_id,
        "sign_count": next,
    })
    .to_string())
}

fn upsert(items: &mut Vec<Entry>, mut entry: Entry, force: bool) -> Result<Entry, String> {
    if let Some(i) = items.iter().position(|e| e.description == entry.description) {
        if !force {
            return Err(format!("Secret \"{}\" already exists. Use force=true to overwrite.", entry.description));
        }
        entry.uuid = items[i].uuid.clone();
        if entry.kind == "passkey" {
            entry.sign_count = items[i].sign_count.or(entry.sign_count);
        }
        items[i] = entry.clone();
        return Ok(entry);
    }
    items.push(entry.clone());
    Ok(entry)
}

pub fn import_json(text: &str, force: bool) -> Result<String, String> {
    import_bytes(text.as_bytes(), force)
}

pub fn import_bytes(bytes: &[u8], force: bool) -> Result<String, String> {
    let _lock = StoreLock::acquire()?;
    let mut items = load()?;
    let mut created = Vec::new();
    let mut notes = Vec::new();

    if crate::opcsv::looks_like_csv(bytes) {
        let text = std::str::from_utf8(bytes).map_err(|e| format!("CSV is not UTF-8: {e}"))?;
        let ents = crate::opcsv::entries_from_csv(text, &mut notes)?;
        for e in ents {
            match upsert(&mut items, e, force) {
                Ok(saved) => created.push(saved),
                Err(e) => notes.push(e),
            }
        }
        if created.is_empty() {
            return Err(if notes.is_empty() {
                "nothing imported".into()
            } else {
                notes.join("; ")
            });
        }
        save(&items)?;
        return Ok(crate::opcsv::summary(&created, &notes));
    }

    let v = crate::onep::parse_json_bytes(bytes)?;
    let found = crate::onep::collect_items(&v);
    if found.is_empty() {
        return Err("no credentials found (need 1Password CSV, or legacy 1Password unencrypted JSON)".into());
    }
    for item in found {
        match crate::onep::entries_from_item(&item, &mut notes) {
            Ok(ents) => {
                for e in ents {
                    match upsert(&mut items, e, force) {
                        Ok(saved) => created.push(saved),
                        Err(e) => notes.push(e),
                    }
                }
            }
            Err(e) => notes.push(e),
        }
    }
    if created.is_empty() {
        return Err(if notes.is_empty() {
            "nothing imported".into()
        } else {
            notes.join("; ")
        });
    }
    save(&items)?;
    Ok(crate::onep::summary(&created, &notes))
}

pub fn import_path(rel: &str, force: bool) -> Result<String, String> {
    let files = std::env::var("BIA_FILES").unwrap_or_else(|_| "/home/bia/files".into());
    let p = std::path::Path::new(rel);
    let abs = if p.is_absolute() { p.to_path_buf() } else { std::path::PathBuf::from(files).join(rel.trim_start_matches('/')) };
    let bytes = fs::read(&abs).map_err(|e| format!("{}: {e}", abs.display()))?;
    import_bytes(&bytes, force)
}

/// Export the named secret plus siblings that share the same title stem (username/password/TOTP)
/// as one 1Password login CSV row.
pub fn export_path(id: &str, rel: &str) -> Result<String, String> {
    let _lock = StoreLock::acquire()?;
    let items = load()?;
    let entry = find(&items, id)?;
    let stem = group_title_public(entry);
    let group: Vec<Entry> = items
        .iter()
        .filter(|e| group_title_public(e) == stem)
        .cloned()
        .collect();
    let csv = crate::opcsv::export_csv(&group)?;
    let bytes = csv.as_bytes();
    let files = std::env::var("BIA_FILES").unwrap_or_else(|_| "/home/bia/files".into());
    let mut rel = rel.trim().to_string();
    if rel.is_empty() {
        return Err("path is required, for example outbox/gmail.csv".into());
    }
    if !rel.to_ascii_lowercase().ends_with(".csv") {
        rel.push_str(".csv");
    }
    let p = std::path::Path::new(&rel);
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::path::PathBuf::from(&files).join(rel.trim_start_matches('/'))
    };
    if let Some(parent) = abs.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let mut f = File::create(&abs).map_err(|e| format!("{}: {e}", abs.display()))?;
    f.write_all(bytes).map_err(|e| format!("{}: {e}", abs.display()))?;
    let _ = f.set_permissions(std::fs::Permissions::from_mode(0o600));
    own_like_files(&abs, Path::new(&files));
    Ok(json!({
        "uuid": entry.uuid,
        "description": entry.description,
        "group": stem,
        "entries": group.len(),
        "path": abs.display().to_string(),
        "format": "csv",
        "headers": crate::opcsv::HEADER,
        "bytes": bytes.len(),
        "hint": "In 1Password.com Import → CSV, label columns Title, URL, Username, Password, one-time password (OTPAuth), Notes"
    })
    .to_string())
}

fn group_title_public(e: &Entry) -> String {
    let d = e.description.trim();
    for suf in [" username", " password", " passkey"] {
        if let Some(stem) = d.strip_suffix(suf) {
            if !stem.is_empty() {
                return stem.to_string();
            }
        }
    }
    if let Some(stem) = d.strip_suffix(" TOTP") {
        if !stem.is_empty() {
            return stem.to_string();
        }
    }
    if let Some((stem, rest)) = d.rsplit_once(" TOTP ") {
        if !stem.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) {
            return stem.to_string();
        }
    }
    d.to_string()
}

fn origin_ok(origin: &str, rp_id: &str) -> Result<(), String> {
    let rest = origin.trim().trim_start_matches("https://").trim_start_matches("http://");
    let host = rest.split(['/', '?', '#']).next().unwrap_or("").split(':').next().unwrap_or("");
    if host.is_empty() {
        return Err("origin is missing a host".into());
    }
    let host_l = host.to_ascii_lowercase();
    let rp_l = rp_id.trim().to_ascii_lowercase();
    if host_l == rp_l || host_l.ends_with(&format!(".{rp_l}")) {
        return Ok(());
    }
    Err(format!("origin {origin} does not match rp_id {rp_id}"))
}

/// Browsers always send base64url, so no format guessing on the ceremony path.
fn ceremony_challenge(challenge: &str) -> Result<Vec<u8>, String> {
    let t = challenge.trim();
    if t.is_empty() {
        return Err("challenge is empty".into());
    }
    crate::b64url_decode(t).map_err(|e| format!("challenge: {e}"))
}

/// Credential ids compare as bytes; padding and alphabet may differ between sites and the store.
fn same_credential(stored: Option<&str>, wanted: &str) -> bool {
    let Some(stored) = stored else { return false };
    match (crate::b64url_decode(stored), crate::b64url_decode(wanted)) {
        (Ok(a), Ok(b)) => a == b,
        _ => stored == wanted,
    }
}

/// Registration from the Chrome extension. Stores a new passkey and returns the create response fields.
/// Errors start with the DOMException name the page should see.
pub fn passkey_ceremony_create(rp_id: &str, origin: &str, challenge: &str, user_name: &str, user_id: Option<&str>, exclude: &[String]) -> Result<String, String> {
    let rp_id = rp_id.trim();
    if rp_id.is_empty() {
        return Err("SecurityError: rp_id is required".into());
    }
    origin_ok(origin, rp_id).map_err(|e| format!("SecurityError: {e}"))?;
    let ch = ceremony_challenge(challenge)?;
    let user_name = if user_name.trim().is_empty() { "bia" } else { user_name.trim() };
    let description = {
        let _lock = StoreLock::acquire()?;
        let items = load()?;
        let excluded = items.iter().any(|e| {
            e.kind == "passkey"
                && e.rp_id.as_deref().unwrap_or("").eq_ignore_ascii_case(rp_id)
                && exclude.iter().any(|id| same_credential(e.credential_id.as_deref(), id))
        });
        if excluded {
            return Err(format!("InvalidStateError: a passkey for {rp_id} is already registered with this account"));
        }
        let base = format!("{user_name} @ {rp_id}");
        if !items.iter().any(|e| e.description == base) {
            base
        } else {
            let mut n = 2u32;
            loop {
                let cand = format!("{base} #{n}");
                if !items.iter().any(|e| e.description == cand) {
                    break cand;
                }
                n += 1;
                if n > 1000 {
                    return Err("too many passkeys for this site".into());
                }
            }
        }
    };
    passkey_create_with(&description, rp_id, user_name, user_id, Some(ch), Some(origin), false)
}

/// Login from the Chrome extension. Picks the one stored passkey for this rp and signs.
pub fn passkey_ceremony_get(rp_id: &str, origin: &str, challenge: &str, allow: &[String]) -> Result<String, String> {
    let rp_id = rp_id.trim();
    if rp_id.is_empty() {
        return Err("SecurityError: rp_id is required".into());
    }
    origin_ok(origin, rp_id).map_err(|e| format!("SecurityError: {e}"))?;
    let ch = ceremony_challenge(challenge)?;
    let allow: Vec<String> = allow.iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    let key = {
        let _lock = StoreLock::acquire()?;
        let items = load()?;
        let mut hits: Vec<&Entry> = items
            .iter()
            .filter(|e| e.kind == "passkey" && e.rp_id.as_deref().unwrap_or("").eq_ignore_ascii_case(rp_id))
            .filter(|e| allow.is_empty() || allow.iter().any(|id| same_credential(e.credential_id.as_deref(), id)))
            .collect();
        if hits.is_empty() {
            return Err(format!("NotAllowedError: no passkey for {rp_id}"));
        }
        if hits.len() > 1 {
            let names: Vec<&str> = hits.iter().map(|e| e.description.as_str()).collect();
            return Err(format!("NotAllowedError: several passkeys for {rp_id}: {}", names.join(", ")));
        }
        hits.pop().unwrap().uuid.clone()
    };
    passkey_assert_with(&key, &ch, Some(origin), Some(rp_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::{signature::Verifier, Signature, VerifyingKey};
    use sha2::Digest;

    #[test]
    fn ceremony_round_trip() {
        let dir = std::env::temp_dir().join(format!("bia-secrets-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        std::env::set_var("BIA_FILES", &dir);
        let origin = "https://login.example.com";
        let challenge = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8"; // hex-looking bytes, base64url text

        assert!(passkey_ceremony_create("example.com", "https://evil.test", challenge, "ann", None, &[]).unwrap_err().starts_with("SecurityError:"));
        let created: Value = serde_json::from_str(&passkey_ceremony_create("example.com", origin, challenge, "ann", Some("dXNlcjE"), &[]).unwrap()).unwrap();
        let cred = created["credential_id"].as_str().unwrap().to_string();
        let cdj: Value = serde_json::from_slice(&crate::b64url_decode(created["clientDataJSON"].as_str().unwrap()).unwrap()).unwrap();
        assert_eq!(cdj["challenge"], challenge);
        assert_eq!(cdj["origin"], origin);
        assert_eq!(cdj["type"], "webauthn.create");
        assert_eq!(cdj["crossOrigin"], false);
        assert_eq!(created["publicKeyAlgorithm"], -7);

        let auth_data = crate::b64url_decode(created["authenticatorData"].as_str().unwrap()).unwrap();
        let att = crate::b64url_decode(created["attestationObject"].as_str().unwrap()).unwrap();
        let embedded = auth_data_in_attestation(&att);
        assert_eq!(auth_data, embedded, "authenticatorData must be the authData inside the attestation object");
        assert_eq!(&auth_data[..32], sha2::Sha256::digest(b"example.com").as_slice());
        assert_eq!(auth_data[32], 0x45, "UP|UV|AT");
        assert_eq!(&auth_data[33..37], &[0, 0, 0, 0]);
        let spki = crate::b64url_decode(created["publicKey"].as_str().unwrap()).unwrap();
        assert_eq!(spki.len(), 91);
        assert_eq!(&spki[..27], &[
            0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce,
            0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00, 0x04,
        ]);
        let cose = cose_key_in_auth_data(&auth_data);
        let x_at = cose.len() - 67;
        assert_eq!(&spki[27..59], &cose[x_at..x_at + 32], "SPKI x must match the COSE key");
        assert_eq!(&spki[59..], &cose[cose.len() - 32..], "SPKI y must match the COSE key");

        // Same id with padding still counts as registered.
        let err = passkey_ceremony_create("example.com", origin, challenge, "ann", None, &[format!("{cred}=")]).unwrap_err();
        assert!(err.starts_with("InvalidStateError:"), "{err}");

        let got: Value = serde_json::from_str(&passkey_ceremony_get("example.com", origin, challenge, &[cred.clone()]).unwrap()).unwrap();
        assert_eq!(got["userHandle"], "dXNlcjE");
        let auth = crate::b64url_decode(got["authenticatorData"].as_str().unwrap()).unwrap();
        let cdj_bytes = crate::b64url_decode(got["clientDataJSON"].as_str().unwrap()).unwrap();
        let cdj: Value = serde_json::from_slice(&cdj_bytes).unwrap();
        assert_eq!(cdj["type"], "webauthn.get");
        assert_eq!(cdj["challenge"], challenge);
        assert_eq!(cdj["origin"], origin);
        assert_eq!(cdj["crossOrigin"], false);
        assert_eq!(auth.len(), 37);
        assert_eq!(auth[32], 0x05, "UP|UV, no attested credential data");
        assert_eq!(&auth[..32], sha2::Sha256::digest(b"example.com").as_slice());
        assert_eq!(&auth[33..37], &[0, 0, 0, 1]);
        let sig = Signature::from_der(&crate::b64url_decode(got["signature"].as_str().unwrap()).unwrap()).unwrap();
        let items = load().unwrap();
        let priv_bytes = crate::b64_decode(&items[0].value).unwrap();
        let vk = VerifyingKey::from(&p256::ecdsa::SigningKey::from_slice(&priv_bytes).unwrap());
        let mut msg = auth.clone();
        msg.extend_from_slice(&sha2::Sha256::digest(&cdj_bytes));
        vk.verify(&msg, &sig).unwrap();

        assert!(passkey_ceremony_get("other.com", "https://other.com", challenge, &[]).unwrap_err().starts_with("NotAllowedError:"));
        let _ = fs::remove_dir_all(&dir);
    }

    fn cbor_bstr_after(buf: &[u8], key: &str) -> Vec<u8> {
        let k = key.as_bytes();
        let mut needle = vec![0x60 | k.len() as u8];
        needle.extend_from_slice(k);
        let pos = buf.windows(needle.len()).position(|w| w == needle).unwrap_or_else(|| panic!("missing {key}"));
        let mut i = pos + needle.len();
        let head = buf[i];
        i += 1;
        let len = if head & 0xe0 == 0x40 && head & 0x1f < 24 {
            (head & 0x1f) as usize
        } else if head == 0x58 {
            let n = buf[i] as usize;
            i += 1;
            n
        } else if head == 0x59 {
            let n = u16::from_be_bytes([buf[i], buf[i + 1]]) as usize;
            i += 2;
            n
        } else {
            panic!("bstr head {head:#x}");
        };
        buf[i..i + len].to_vec()
    }

    fn auth_data_in_attestation(att: &[u8]) -> Vec<u8> {
        assert_eq!(att[0], 0xa3, "attestation object is a 3-entry map");
        assert!(att.windows(5).any(|w| w == b"\x64none"), "fmt none");
        assert!(att.windows(9).any(|w| w == b"\x67attStmt\xa0"), "empty attStmt");
        cbor_bstr_after(att, "authData")
    }

    fn cose_key_in_auth_data(auth: &[u8]) -> &[u8] {
        let id_len = u16::from_be_bytes([auth[53], auth[54]]) as usize;
        &auth[55 + id_len..]
    }
}
