//! 1Password-friendly login CSV (Title, URL, Username, Password, OTPAuth, Notes).
//! Import maps OTPAuth as “one-time password” in the 1Password.com CSV importer.
use serde_json::{json, Value};

use crate::otp::{self, Alg, OtpParams};
use crate::store::{new_uuid, Entry};

/// Header row 1Password’s CSV importer can label as Title / URL / Username / Password /
/// one-time password / Notes.
pub const HEADER: &[&str] = &["Title", "URL", "Username", "Password", "OTPAuth", "Notes"];

pub fn looks_like_csv(bytes: &[u8]) -> bool {
    let text = match std::str::from_utf8(trim_bom(bytes)) {
        Ok(t) => t.trim_start(),
        Err(_) => return false,
    };
    if text.starts_with('{') || text.starts_with('[') {
        return false;
    }
    let first = text.lines().next().unwrap_or("").trim();
    if first.is_empty() {
        return false;
    }
    let lower = first.to_ascii_lowercase();
    lower.contains("title")
        || lower.contains("username")
        || lower.contains("password")
        || lower.contains("otpauth")
        || first.contains(',')
}

pub fn export_csv(entries: &[Entry]) -> Result<String, String> {
    let rows = rows_from_entries(entries)?;
    if rows.is_empty() {
        return Err("nothing to export as CSV (passkeys are not supported in 1Password CSV)".into());
    }
    let mut out = String::new();
    out.push_str(&csv_line(HEADER));
    out.push('\n');
    for row in &rows {
        out.push_str(&csv_line(&[
            row.title.as_str(),
            row.url.as_str(),
            row.username.as_str(),
            row.password.as_str(),
            row.otpauth.as_str(),
            row.notes.as_str(),
        ]));
        out.push('\n');
    }
    Ok(out)
}

/// Group store entries that share a title stem into login CSV rows.
pub fn rows_from_entries(entries: &[Entry]) -> Result<Vec<LoginRow>, String> {
    let mut groups: Vec<(String, Vec<&Entry>)> = Vec::new();
    for e in entries {
        let key = group_title(e);
        if let Some((_, g)) = groups.iter_mut().find(|(k, _)| k == &key) {
            g.push(e);
        } else {
            groups.push((key, vec![e]));
        }
    }
    let mut out = Vec::new();
    for (title, ents) in groups {
        if let Some(row) = row_from_group(&title, &ents)? {
            out.push(row);
        }
    }
    Ok(out)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoginRow {
    pub title: String,
    pub url: String,
    pub username: String,
    pub password: String,
    pub otpauth: String,
    pub notes: String,
}

fn row_from_group(title: &str, ents: &[&Entry]) -> Result<Option<LoginRow>, String> {
    let mut username = String::new();
    let mut password = String::new();
    let mut otpauth = String::new();
    let mut url = String::new();
    let mut notes: Vec<String> = Vec::new();
    let mut saw_loginish = false;

    for e in ents {
        match e.kind.as_str() {
            "string" => {
                saw_loginish = true;
                if e.description.ends_with(" username") {
                    username = e.value.clone();
                } else {
                    // password or bare string secret
                    if password.is_empty() {
                        password = e.value.clone();
                    } else {
                        notes.push(format!("{}: {}", e.description, e.value));
                    }
                    if username.is_empty() {
                        if let Some(a) = e.account.as_deref().filter(|s| !s.is_empty()) {
                            username = a.to_string();
                        }
                    }
                }
            }
            "totp" | "hotp" => {
                saw_loginish = true;
                let p = OtpParams {
                    secret_b32: e.value.replace(' ', "").to_ascii_uppercase(),
                    digits: e.digits.unwrap_or(6),
                    period: e.period.unwrap_or(30),
                    algorithm: Alg::parse(e.algorithm.as_deref().unwrap_or("SHA1")),
                    counter: e.counter.unwrap_or(0),
                    issuer: e.issuer.clone().unwrap_or_default(),
                    account: e
                        .account
                        .clone()
                        .or_else(|| nonempty_str(&username))
                        .unwrap_or_default(),
                    hotp: e.kind == "hotp",
                };
                let _ = otp::decode_b32(&p.secret_b32)?;
                let uri = otp::format_otpauth(&p);
                if otpauth.is_empty() {
                    otpauth = uri;
                } else {
                    notes.push(format!("extra OTP: {uri}"));
                }
                if username.is_empty() {
                    if let Some(a) = e.account.as_deref().filter(|s| !s.is_empty()) {
                        username = a.to_string();
                    }
                }
                if url.is_empty() {
                    if let Some(issuer) = e.issuer.as_deref().filter(|s| looks_like_host(s)) {
                        url = format!("https://{issuer}");
                    }
                }
            }
            "passkey" => {
                // 1Password CSV cannot import passkeys; note them so the login still imports.
                if let Some(rp) = e.rp_id.as_deref().filter(|s| !s.is_empty()) {
                    if url.is_empty() {
                        url = format!("https://{rp}");
                    }
                }
                let who = e.user_name.as_deref().or(e.account.as_deref()).unwrap_or("");
                notes.push(format!(
                    "passkey for {} (user {}); private key not included in CSV",
                    e.rp_id.as_deref().unwrap_or("?"),
                    if who.is_empty() { "?" } else { who }
                ));
                if username.is_empty() && !who.is_empty() {
                    username = who.to_string();
                }
            }
            other => return Err(format!("cannot export type {other}")),
        }
    }

    if !saw_loginish && notes.is_empty() {
        return Ok(None);
    }
    // Pure passkey group with only a note: still emit a row so URL/username are not lost,
    // but password stays empty — user can ignore or fill later in 1Password.
    if password.is_empty() && username.is_empty() && otpauth.is_empty() && notes.is_empty() {
        return Ok(None);
    }

    Ok(Some(LoginRow {
        title: title.to_string(),
        url,
        username,
        password,
        otpauth,
        notes: notes.join("\n"),
    }))
}

pub fn entries_from_csv(text: &str, notes: &mut Vec<String>) -> Result<Vec<Entry>, String> {
    let text = trim_bom_str(text);
    let mut lines = parse_csv(text)?;
    if lines.is_empty() {
        return Err("empty CSV".into());
    }
    let header = lines.remove(0);
    let map = column_map(&header);
    if map.title.is_none() && map.username.is_none() && map.password.is_none() && map.otpauth.is_none() {
        // No header match — treat first row as data with positional Title,URL,Username,Password
        lines.insert(0, header);
        return entries_from_positional(&lines, notes);
    }
    let mut out = Vec::new();
    for (i, row) in lines.iter().enumerate() {
        if row.iter().all(|c| c.trim().is_empty()) {
            continue;
        }
        match entries_from_mapped_row(row, &map) {
            Ok(ents) => out.extend(ents),
            Err(e) => notes.push(format!("row {}: {e}", i + 2)),
        }
    }
    if out.is_empty() {
        return Err(if notes.is_empty() {
            "no login rows in CSV".into()
        } else {
            notes.join("; ")
        });
    }
    Ok(out)
}

struct ColMap {
    title: Option<usize>,
    url: Option<usize>,
    username: Option<usize>,
    password: Option<usize>,
    otpauth: Option<usize>,
    notes: Option<usize>,
}

fn column_map(header: &[String]) -> ColMap {
    let mut m = ColMap {
        title: None,
        url: None,
        username: None,
        password: None,
        otpauth: None,
        notes: None,
    };
    for (i, h) in header.iter().enumerate() {
        let k = normalize_header(h);
        match k.as_str() {
            "title" | "name" => m.title = Some(i),
            "url" | "website" | "websites" | "login url" | "login_url" => m.url = Some(i),
            "username" | "user" | "login" | "email" => m.username = Some(i),
            "password" | "pass" => m.password = Some(i),
            "otpauth" | "otp" | "totp" | "one-time password" | "one time password" | "onetimepassword" => {
                m.otpauth = Some(i)
            }
            "notes" | "note" | "comments" | "comment" => m.notes = Some(i),
            _ => {}
        }
    }
    m
}

fn normalize_header(h: &str) -> String {
    h.trim().to_ascii_lowercase().replace('_', " ")
}

fn entries_from_positional(lines: &[Vec<String>], notes: &mut Vec<String>) -> Result<Vec<Entry>, String> {
    // 1Password sample without header: Title,URL,Username,Password
    let mut out = Vec::new();
    for (i, row) in lines.iter().enumerate() {
        if row.iter().all(|c| c.trim().is_empty()) {
            continue;
        }
        let title = cell(row, 0);
        let url = cell(row, 1);
        let username = cell(row, 2);
        let password = cell(row, 3);
        let otpauth = cell(row, 4);
        match build_entries(&title, &url, &username, &password, &otpauth, "") {
            Ok(ents) => out.extend(ents),
            Err(e) => notes.push(format!("row {}: {e}", i + 1)),
        }
    }
    if out.is_empty() {
        return Err(if notes.is_empty() {
            "no login rows in CSV".into()
        } else {
            notes.join("; ")
        });
    }
    Ok(out)
}

fn entries_from_mapped_row(row: &[String], map: &ColMap) -> Result<Vec<Entry>, String> {
    let title = map.title.map(|i| cell(row, i)).unwrap_or_default();
    let url = map.url.map(|i| cell(row, i)).unwrap_or_default();
    let username = map.username.map(|i| cell(row, i)).unwrap_or_default();
    let password = map.password.map(|i| cell(row, i)).unwrap_or_default();
    let otpauth = map.otpauth.map(|i| cell(row, i)).unwrap_or_default();
    let notes = map.notes.map(|i| cell(row, i)).unwrap_or_default();
    build_entries(&title, &url, &username, &password, &otpauth, &notes)
}

fn build_entries(
    title: &str,
    url: &str,
    username: &str,
    password: &str,
    otpauth: &str,
    notes_text: &str,
) -> Result<Vec<Entry>, String> {
    let title = title.trim();
    let title = if title.is_empty() { "item" } else { title };
    let username = username.trim();
    let password = password.trim();
    let otpauth = otpauth.trim();
    let mut out = Vec::new();
    let account = if !username.is_empty() {
        username
    } else {
        ""
    };

    if !username.is_empty() {
        out.push(string_entry(&format!("{title} username"), username, account)?);
    }
    if !password.is_empty() {
        out.push(string_entry(&format!("{title} password"), password, account)?);
    }
    if !otpauth.is_empty() {
        out.push(otp_entry(&format!("{title} TOTP"), otpauth, account)?);
    }
    if out.is_empty() {
        if !notes_text.trim().is_empty() {
            out.push(string_entry(title, notes_text.trim(), account)?);
        } else if !url.trim().is_empty() {
            return Err(format!("{title:?}: only URL, no username/password/OTP"));
        } else {
            return Err(format!("{title:?}: empty login row"));
        }
    }
    let _ = url; // URL is for 1Password; we don't store it separately
    Ok(out)
}

fn cell(row: &[String], i: usize) -> String {
    row.get(i).map(|s| s.as_str()).unwrap_or("").trim().to_string()
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
        account: nonempty_str(account),
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
            issuer: nonempty_str(&p.issuer),
            account: nonempty_str(if p.account.is_empty() { account } else { &p.account }),
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
        account: nonempty_str(account),
        rp_id: None,
        user_name: None,
        user_id: None,
        credential_id: None,
        cose_key: None,
        sign_count: None,
    })
}

fn group_title(e: &Entry) -> String {
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

fn looks_like_host(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty() && !s.contains(' ') && s.contains('.') && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

fn nonempty_str(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() {
        None
    } else {
        Some(s.into())
    }
}

fn csv_line(fields: &[&str]) -> String {
    fields.iter().map(|f| csv_escape(f)).collect::<Vec<_>>().join(",")
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn parse_csv(text: &str) -> Result<Vec<Vec<String>>, String> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut chars = text.chars().peekable();
    let mut in_quotes = false;
    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    in_quotes = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' => in_quotes = true,
            ',' => {
                row.push(std::mem::take(&mut field));
            }
            '\n' => {
                row.push(std::mem::take(&mut field));
                if row.iter().any(|f| !f.is_empty()) || !row.is_empty() {
                    rows.push(std::mem::take(&mut row));
                } else {
                    row.clear();
                }
            }
            '\r' => {}
            _ => field.push(c),
        }
    }
    if in_quotes {
        return Err("unterminated quoted field in CSV".into());
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    Ok(rows)
}

fn trim_bom(b: &[u8]) -> &[u8] {
    if b.starts_with(&[0xef, 0xbb, 0xbf]) {
        &b[3..]
    } else {
        b
    }
}

fn trim_bom_str(s: &str) -> &str {
    s.strip_prefix('\u{feff}').unwrap_or(s)
}

pub fn summary(created: &[Entry], notes: &[String]) -> String {
    let rows: Vec<Value> = created.iter().map(|e| json!({"uuid": e.uuid, "description": e.description, "type": e.kind})).collect();
    json!({"imported": rows.len(), "items": rows, "notes": notes, "format": "csv"}).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ent(desc: &str, kind: &str, value: &str, account: Option<&str>) -> Entry {
        Entry {
            uuid: new_uuid().unwrap(),
            description: desc.into(),
            kind: kind.into(),
            value: value.into(),
            digits: if kind == "totp" { Some(6) } else { None },
            period: if kind == "totp" { Some(30) } else { None },
            algorithm: if kind == "totp" { Some("SHA1".into()) } else { None },
            counter: None,
            issuer: if kind == "totp" { Some("Google".into()) } else { None },
            account: account.map(|s| s.into()),
            rp_id: None,
            user_name: None,
            user_id: None,
            credential_id: None,
            cose_key: None,
            sign_count: None,
        }
    }

    #[test]
    fn export_groups_login_and_totp() {
        let ents = vec![
            ent("Gmail username", "string", "a@b.c", Some("a@b.c")),
            ent("Gmail password", "string", "s3cret", Some("a@b.c")),
            ent("Gmail TOTP", "totp", "JBSWY3DPEHPK3PXP", Some("a@b.c")),
        ];
        let csv = export_csv(&ents).unwrap();
        assert!(csv.starts_with("Title,URL,Username,Password,OTPAuth,Notes\n"));
        assert!(csv.contains("Gmail,"));
        assert!(csv.contains("a@b.c"));
        assert!(csv.contains("s3cret"));
        assert!(csv.contains("otpauth://totp/"));
        assert!(csv.contains("JBSWY3DPEHPK3PXP"));
        let mut notes = vec![];
        let again = entries_from_csv(&csv, &mut notes).unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        assert!(again.iter().any(|e| e.kind == "string" && e.value == "s3cret"));
        assert!(again.iter().any(|e| e.kind == "totp"));
    }

    #[test]
    fn import_headerless_sample() {
        let text = "Twitter,https://twitter.com/,wendy_appleseed,d2o4dmLxKBKGdzxHAA\n";
        let mut notes = vec![];
        let ents = entries_from_csv(text, &mut notes).unwrap();
        assert!(ents.iter().any(|e| e.description.contains("username") && e.value == "wendy_appleseed"));
        assert!(ents.iter().any(|e| e.description.contains("password") && e.value == "d2o4dmLxKBKGdzxHAA"));
    }

    #[test]
    fn escapes_commas_in_password() {
        let ents = vec![ent("X password", "string", "a,b\"c", None)];
        let csv = export_csv(&ents).unwrap();
        assert!(csv.contains("\"a,b\"\"c\""));
        let mut notes = vec![];
        let again = entries_from_csv(&csv, &mut notes).unwrap();
        assert_eq!(again.iter().find(|e| e.kind == "string").unwrap().value, "a,b\"c");
    }
}
