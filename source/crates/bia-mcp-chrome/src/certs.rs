//! Certificates for websites: client certificates and private CAs in the NSS database Chrome reads
//! (~/.pki/nssdb of user bia), and which certificate Chrome picks by itself for which site (the
//! AutoSelectCertificateForUrls policy). What lives outside bia's home goes with an OS update, so it
//! is kept in files/state and put back at start: system CAs and the policy file.

use crate::install::policy_dirs;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const USER: &str = "bia";
const HOME: &str = "/home/bia";
const NSS: &str = "sql:/home/bia/.pki/nssdb";
const POLICY_FILE: &str = "bia-client-certs.json";
const SYSTEM_CA_DIR: &str = "usr/local/share/ca-certificates";

fn files_dir() -> PathBuf {
    PathBuf::from(std::env::var("BIA_FILES").unwrap_or_else(|_| format!("{HOME}/files")))
}

fn state_dir() -> PathBuf {
    files_dir().join("state")
}

fn cas_dir() -> PathBuf {
    state_dir().join("chrome-cas")
}

fn rules_path() -> PathBuf {
    state_dir().join("chrome-cert-rules.json")
}

fn tool_missing(name: &str) -> String {
    format!("{name} is not installed: this needs the current system (carry_update)")
}

/// Run as bia, so the database stays bia's own.
fn as_bia(tool: &str, args: &[&str]) -> Result<Output, String> {
    let mut c = Command::new("runuser");
    c.args(["-u", USER, "--", "env", "HOME=/home/bia", tool]).args(args).stdin(Stdio::null());
    let out = c.output().map_err(|e| format!("run {tool}: {e}"))?;
    if out.status.code() == Some(127) {
        return Err(tool_missing(tool));
    }
    Ok(out)
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).trim().to_string()
}

fn checked(tool: &str, args: &[&str]) -> Result<String, String> {
    let out = as_bia(tool, args)?;
    if out.status.success() {
        return Ok(text(&out.stdout));
    }
    let err = text(&out.stderr);
    Err(format!("{tool}: {}", if err.is_empty() { text(&out.stdout) } else { err }))
}

fn chown_bia(path: &Path) {
    let _ = Command::new("chown").args(["-R", "bia:bia"]).arg(path).status();
}

fn ensure_db() -> Result<(), String> {
    let pki = Path::new(HOME).join(".pki");
    let db = pki.join("nssdb");
    fs::create_dir_all(&db).map_err(|e| format!("{}: {e}", db.display()))?;
    for d in [&pki, &db] {
        let _ = fs::set_permissions(d, fs::Permissions::from_mode(0o700));
    }
    chown_bia(&pki);
    if !db.join("cert9.db").exists() {
        checked("certutil", &["-N", "-d", NSS, "--empty-password"])?;
    }
    Ok(())
}

/// A file named by the model: absolute, or relative to bia's files.
fn input_file(a: &Value, key: &str) -> Result<PathBuf, String> {
    let raw = a.get(key).and_then(Value::as_str).map(str::trim).unwrap_or("");
    if raw.is_empty() {
        return Err(format!("{key} required"));
    }
    let p = Path::new(raw);
    let abs = if p.is_absolute() { p.to_path_buf() } else { files_dir().join(raw) };
    if !abs.is_file() {
        return Err(format!("file not found: {}", abs.display()));
    }
    Ok(abs)
}

/// A private folder bia can read, gone when this is dropped. Passwords never go on a command line.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Result<Scratch, String> {
        let d = Path::new(HOME).join(".pki").join(format!("import-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
        fs::set_permissions(&d, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
        Ok(Scratch(d))
    }

    fn write(&self, name: &str, data: &[u8]) -> Result<String, String> {
        let p = self.0.join(name);
        let mut f = fs::OpenOptions::new().create(true).truncate(true).write(true).open(&p).map_err(|e| e.to_string())?;
        f.set_permissions(fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
        f.write_all(data).map_err(|e| e.to_string())?;
        Ok(p.to_string_lossy().into_owned())
    }

    fn copy(&self, name: &str, from: &Path) -> Result<String, String> {
        let data = fs::read(from).map_err(|e| format!("{}: {e}", from.display()))?;
        self.write(name, &data)
    }

    fn hand_over(&self) {
        chown_bia(&self.0);
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn password_of(a: &Value) -> Result<String, String> {
    if let Some(name) = a.get("password_secret").and_then(Value::as_str).filter(|s| !s.trim().is_empty()) {
        return bia_mcp_secrets::store::get(name.trim(), true).map_err(|e| format!("password_secret {name}: {e}"));
    }
    Ok(a.get("password").and_then(Value::as_str).unwrap_or("").to_string())
}

/// (nickname, trust flags) of every certificate in the database.
fn listed() -> Result<Vec<(String, String)>, String> {
    let out = checked("certutil", &["-L", "-d", NSS])?;
    let mut rows = vec![];
    for line in out.lines() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with("Certificate Nickname") || line.contains("SSL,S/MIME") {
            continue;
        }
        if let Some((name, flags)) = line.rsplit_once(char::is_whitespace) {
            if flags.contains(',') {
                rows.push((name.trim().to_string(), flags.to_string()));
            }
        }
    }
    Ok(rows)
}

/// Names of client certificates: trust `u` means this store holds the key. (`certutil -K` is no
/// help: a key keeps the label it came with, whatever its certificate is renamed to.)
fn client_names() -> Vec<String> {
    listed().unwrap_or_default().into_iter().filter(|(_, flags)| flags.contains('u')).map(|(n, _)| n).collect()
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Fields {
    subject: String,
    issuer: String,
    until: String,
    serial: String,
}

/// What a certificate in the store says. `-a` prints every certificate of that subject, the named
/// one first, and openssl reads the first.
fn details(nick: &str) -> Result<Fields, String> {
    let pem = checked("certutil", &["-L", "-d", NSS, "-n", nick, "-a"])?;
    x509_fields(pem.as_bytes())
}

fn x509_fields(pem: &[u8]) -> Result<Fields, String> {
    let mut child = Command::new("openssl")
        .args(["x509", "-noout", "-subject", "-issuer", "-enddate", "-serial", "-nameopt", "RFC2253"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| tool_missing("openssl"))?;
    child.stdin.take().ok_or("no stdin")?.write_all(pem).map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err("this is not a certificate openssl can read".into());
    }
    let mut f = Fields::default();
    for l in text(&out.stdout).lines() {
        if let Some(v) = l.strip_prefix("subject=") {
            f.subject = v.trim().to_string();
        } else if let Some(v) = l.strip_prefix("issuer=") {
            f.issuer = v.trim().to_string();
        } else if let Some(v) = l.strip_prefix("notAfter=") {
            f.until = v.trim().to_string();
        } else if let Some(v) = l.strip_prefix("serial=") {
            f.serial = v.trim().to_string();
        }
    }
    Ok(f)
}

/// A name NSS can find again: it reads what stands before a colon as a token's name.
fn clean_nick(s: &str) -> String {
    let t = s.replace(':', " ").split_whitespace().collect::<Vec<_>>().join(" ");
    if t.is_empty() { "certificate".into() } else { t }
}

fn nick_for(fields: &Fields, taken: &[String]) -> String {
    let base = clean_nick(&name_field(&fields.subject, "CN").unwrap_or_else(|| fields.subject.clone()));
    if !taken.iter().any(|t| t == &base) {
        return base;
    }
    let tail: String = fields.serial.chars().rev().take(8).collect::<Vec<_>>().into_iter().rev().collect();
    let with_serial = format!("{base} #{tail}");
    if !taken.iter().any(|t| t == &with_serial) {
        return with_serial;
    }
    format!("{base} #{}", fields.serial)
}

/// Each certificate gets a name of its own, without a colon. A site's identical certificates (a
/// reissued one, say) otherwise share a name, and `--rename` is the one way to reach them singly:
/// it takes the first of that name.
fn normalize_names() -> Result<(), String> {
    for _ in 0..64 {
        let rows = listed()?;
        let names: Vec<String> = rows.iter().map(|(n, _)| n.clone()).collect();
        let Some(old) = names.iter().find(|n| n.contains(':') || names.iter().filter(|m| m == n).count() > 1) else {
            return Ok(());
        };
        let fields = details(old)?;
        // the old name counts as taken: a duplicate must move off it, while the other keeps it
        checked("certutil", &["--rename", "-d", NSS, "-n", old, "--new-n", &nick_for(&fields, &names)])?;
    }
    Err("too many certificates share a name".into())
}

/// One field (CN=…) of an RFC 2253 name.
fn name_field(dn: &str, key: &str) -> Option<String> {
    let want = format!("{key}=");
    let mut part = String::new();
    let mut parts = vec![];
    let mut escaped = false;
    for c in dn.chars() {
        if escaped {
            part.push(c);
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == ',' {
            parts.push(std::mem::take(&mut part));
        } else {
            part.push(c);
        }
    }
    parts.push(part);
    parts.iter().find_map(|p| p.trim().strip_prefix(&want).map(str::to_string))
}

/// Client certificate from a .p12/.pfx, or a certificate file plus key_path, into the database.
pub fn import(a: &Value) -> Result<String, String> {
    let path = input_file(a, "path")?;
    let password = password_of(a)?;
    let raw = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    if bia_pki::biaconfig::is_biaconfig(&raw) {
        return import_biaconfig(&raw, &password, &path);
    }
    ensure_db()?;
    let scratch = Scratch::new()?;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let mut pass_file = scratch.write("pass", password.as_bytes())?;
    let bundle = if matches!(ext.as_str(), "p12" | "pfx") {
        scratch.copy("in.p12", &path)?
    } else {
        // a certificate and its key as separate files
        let key = input_file(a, "key_path").map_err(|_| "a .p12/.pfx holds its key; any other certificate file needs key_path, its private key".to_string())?;
        let cert = scratch.copy("cert", &path)?;
        let key = scratch.copy("key", &key)?;
        let out = scratch.0.join("in.p12").to_string_lossy().into_owned();
        // the bundle lives for this call: its own throwaway password, as an empty one reads badly
        let bundle_pass = scratch.write("bundle-pass", random_hex()?.as_bytes())?;
        scratch.hand_over();
        let passout = format!("file:{bundle_pass}");
        let passin = format!("file:{pass_file}");
        let mut args = vec!["pkcs12", "-export", "-in", &cert, "-inkey", &key, "-passout", &passout, "-out", &out];
        if !password.is_empty() {
            args.extend(["-passin", &passin]);
        }
        checked("openssl", &args).map_err(|e| format!("could not join the certificate and its key: {e}"))?;
        pass_file = bundle_pass;
        out
    };
    scratch.hand_over();
    normalize_names()?;
    let before = client_names();
    // a file with no password: pk12util cannot read an empty password file
    let mut args = vec!["-i", bundle.as_str(), "-d", NSS, "-K", ""];
    if password.is_empty() && matches!(ext.as_str(), "p12" | "pfx") {
        args.extend(["-W", ""]);
    } else {
        args.extend(["-w", pass_file.as_str()]);
    }
    let out = as_bia("pk12util", &args)?;
    if !out.status.success() {
        let why = format!("{} {}", text(&out.stderr), text(&out.stdout));
        return Err(if why.contains("BAD_PASSWORD") || why.contains("Incorrect password") || why.contains("BAD_DER") && password.is_empty() {
            "the password is wrong (or this file needs one): pass password_secret, the name of a secret holding it".to_string()
        } else {
            format!("pk12util: {why}")
        });
    }
    normalize_names()?;
    let after = client_names();
    let Some(name) = after.iter().find(|n| !before.contains(n)).cloned() else {
        return Ok(format!("nothing new: this certificate is already here. chrome_cert_list shows it. Delete {} if it is not needed.", path.display()));
    };
    let f = details(&name)?;
    let twin = twins_of(&name, &f);
    let note = if twin.is_empty() {
        String::new()
    } else {
        format!(" {} has the same subject and issuer, told apart only by serial: a rule by chrome_cert_select cannot choose between them, so answer the dialog with serial.", twin.join(", "))
    };
    Ok(format!(
        "imported client certificate {name:?}: {}, issued by {}, serial {}, valid until {}.{note} chrome_restart makes Chrome see it. Then chrome_cert_select(site, cert) picks it for a site by itself. Its key now sits in bia's certificate store, so delete {} if it is not needed.",
        f.subject, f.issuer, f.serial, f.until,
        path.display()
    ))
}

/// A `.biaconfig`: its certificate, its CA for Chrome alone (a file someone sent must not get
/// to vouch for websites to this whole machine), and where that bia is.
fn import_biaconfig(raw: &[u8], password: &str, path: &Path) -> Result<String, String> {
    let c = bia_pki::biaconfig::open(raw, password).map_err(|e| format!("{e:#}"))?;
    ensure_db()?;
    let scratch = Scratch::new()?;
    let bundle = scratch.write("in.p12", &c.p12)?;
    let pass = scratch.write("pass", bia_pki::P12_PASSWORD.as_bytes())?;
    let ca = scratch.write("ca.pem", c.ca.as_bytes())?;
    scratch.hand_over();
    normalize_names()?;
    let before = client_names();
    let out = as_bia("pk12util", &["-i", &bundle, "-d", NSS, "-K", "", "-w", &pass])?;
    if !out.status.success() {
        return Err(format!("pk12util: {} {}", text(&out.stderr), text(&out.stdout)));
    }
    let ca_name = clean_nick(&x509_fields(c.ca.as_bytes()).ok().and_then(|f| name_field(&f.subject, "CN")).unwrap_or_else(|| "bia CA".into()));
    let label = if c.name.trim().is_empty() { ca_name } else { clean_nick(&format!("{} CA", c.name.trim())) };
    checked("certutil", &["-A", "-d", NSS, "-n", &label, "-t", "C,,", "-a", "-i", &ca])?;
    normalize_names()?;
    let name = client_names().into_iter().find(|n| !before.contains(n));
    Ok(format!(
        "imported {}{}: bia at https://{}/{}. chrome_restart makes Chrome see it; chrome_cert_select(site, cert) picks it without asking. Delete {} if it is not needed.",
        name.map_or("nothing new (already here)".to_string(), |n| format!("client certificate {n:?}")),
        if c.name.is_empty() { String::new() } else { format!(" for {}", c.name) },
        c.addresses[0],
        if c.addresses.len() > 1 { format!(" (also {})", c.addresses[1..].join(", ")) } else { String::new() },
        path.display(),
    ))
}

/// Other client certificates that Chrome's policy, which knows subject and issuer only, cannot tell
/// from this one.
fn twins_of(name: &str, f: &Fields) -> Vec<String> {
    client_names()
        .into_iter()
        .filter(|n| n != name)
        .filter(|n| details(n).is_ok_and(|o| o.subject == f.subject && o.issuer == f.issuer))
        .collect()
}

fn random_hex() -> Result<String, String> {
    let mut b = [0u8; 16];
    fs::File::open("/dev/urandom").and_then(|mut f| std::io::Read::read_exact(&mut f, &mut b)).map_err(|e| e.to_string())?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

fn safe_name(s: &str) -> String {
    let t: String = s.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '-' }).collect();
    t.trim_matches('-').to_string()
}

fn install_system_ca(name: &str, pem: &[u8]) -> Result<(), String> {
    let dir = crate::install::sysroot().join(SYSTEM_CA_DIR);
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    fs::write(dir.join(format!("bia-{}.crt", safe_name(name))), pem).map_err(|e| e.to_string())?;
    update_system_cas()
}

fn update_system_cas() -> Result<(), String> {
    let out = Command::new("update-ca-certificates").output().map_err(|_| tool_missing("update-ca-certificates"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("update-ca-certificates: {}", text(&out.stderr)))
    }
}

fn to_pem(raw: &[u8]) -> Result<Vec<u8>, String> {
    if raw.windows(11).any(|w| w == b"-----BEGIN ") {
        return Ok(raw.to_vec());
    }
    let mut child = Command::new("openssl")
        .args(["x509", "-inform", "DER", "-outform", "PEM"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| tool_missing("openssl"))?;
    child.stdin.take().ok_or("no stdin")?.write_all(raw).map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err("this is not a certificate (PEM or DER)".into());
    }
    Ok(out.stdout)
}

/// Trust a private CA, in Chrome and by default on the whole system.
pub fn trust(a: &Value) -> Result<String, String> {
    let path = input_file(a, "path")?;
    let pem = to_pem(&fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?)?;
    if pem.windows(11).any(|w| w == b"PRIVATE KEY") {
        return Err("this file holds a private key: a CA to trust is its certificate only".into());
    }
    let Fields { subject, until, .. } = x509_fields(&pem)?;
    let name = a.get("name").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
        .or_else(|| name_field(&subject, "CN"))
        .unwrap_or_else(|| "private-ca".into());
    let name = clean_nick(&name);
    let system = a.get("system").and_then(Value::as_bool).unwrap_or(true);
    ensure_db()?;
    let scratch = Scratch::new()?;
    let file = scratch.write("ca.pem", &pem)?;
    scratch.hand_over();
    checked("certutil", &["-A", "-d", NSS, "-n", &name, "-t", "C,,", "-a", "-i", &file])?;
    if system {
        install_system_ca(&name, &pem)?;
        // an OS update replaces the system store: start() puts this back
        fs::create_dir_all(cas_dir()).map_err(|e| e.to_string())?;
        fs::write(cas_dir().join(format!("{}.crt", safe_name(&name))), &pem).map_err(|e| e.to_string())?;
        chown_bia(&cas_dir());
    }
    Ok(format!(
        "now trusted{}: CA {name:?}, {subject}, valid until {until}. It can vouch for any website, so it stays only as long as root wants it (chrome_cert_remove). chrome_restart makes Chrome see it.",
        if system { " in Chrome and on this system" } else { " in Chrome" }
    ))
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct Rule {
    site: String,
    pattern: String,
    cert: String,
    filter: Value,
}

fn load_rules() -> Vec<Rule> {
    bia_mcp::statefile::read_or_default(&rules_path())
}

fn policy_of(rules: &[Rule]) -> Value {
    let entries: Vec<String> = rules.iter().map(|r| json!({"pattern": r.pattern, "filter": r.filter}).to_string()).collect();
    json!({"AutoSelectCertificateForUrls": entries})
}

fn write_policy(rules: &[Rule]) -> Result<(), String> {
    let body = serde_json::to_vec_pretty(&policy_of(rules)).map_err(|e| e.to_string())?;
    for d in policy_dirs() {
        fs::create_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
        let p = d.join(POLICY_FILE);
        if rules.is_empty() {
            let _ = fs::remove_file(&p);
        } else {
            fs::write(&p, &body).map_err(|e| format!("{}: {e}", p.display()))?;
        }
    }
    Ok(())
}

fn save_rules(rules: &[Rule]) -> Result<(), String> {
    fs::create_dir_all(state_dir()).map_err(|e| e.to_string())?;
    let path = rules_path();
    bia_mcp::statefile::write_json(&path, &rules).map_err(|e| e.to_string())?;
    chown_bia(&path);
    chown_bia(&bia_mcp::statefile::backup_path(&path));
    write_policy(rules)
}

/// What an OS update took away: the policy file and the system CAs. Runs at each start of the MCP.
pub fn restore() {
    let rules = load_rules();
    if !rules.is_empty() {
        if let Err(e) = write_policy(&rules) {
            eprintln!("chrome certificate rules: {e}");
        }
    }
    let Ok(dir) = fs::read_dir(cas_dir()) else { return };
    let target = crate::install::sysroot().join(SYSTEM_CA_DIR);
    let mut added = false;
    for e in dir.flatten() {
        let dest = target.join(format!("bia-{}", e.file_name().to_string_lossy()));
        if !dest.exists() && fs::create_dir_all(&target).is_ok() && fs::copy(e.path(), &dest).is_ok() {
            added = true;
        }
    }
    if added {
        if let Err(e) = update_system_cas() {
            eprintln!("system CAs: {e}");
        }
    }
}

/// `example.com`, `https://example.com/x` or Chrome's own pattern, as a pattern.
fn site_pattern(site: &str) -> Result<String, String> {
    let s = site.trim();
    if s.is_empty() {
        return Err("site required: a host like portal.example.com".into());
    }
    if s.contains("[*.]") {
        return Ok(s.to_string());
    }
    let rest = s.split_once("://").map_or(s, |(_, r)| r);
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if host.is_empty() || host.contains(char::is_whitespace) || host.contains('@') {
        return Err(format!("{site:?} is not a site: pass a host like portal.example.com"));
    }
    Ok(format!("https://{host}"))
}

fn dn_filter(dn: &str) -> Value {
    let mut out = serde_json::Map::new();
    for key in ["CN", "O", "OU", "L"] {
        if let Some(v) = name_field(dn, key) {
            out.insert(key.into(), json!(v));
        }
    }
    Value::Object(out)
}

/// Pick a certificate for a site without asking.
pub fn select(a: &Value) -> Result<String, String> {
    let pattern = site_pattern(a.get("site").and_then(Value::as_str).unwrap_or(""))?;
    normalize_names()?;
    let keys = client_names();
    let cert = match a.get("cert").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
        Some(c) if keys.iter().any(|k| k == c) => c.to_string(),
        Some(c) => return Err(format!("no client certificate {c:?}; there are: {}", if keys.is_empty() { "none".to_string() } else { keys.join(", ") })),
        None if keys.len() == 1 => keys[0].clone(),
        None if keys.is_empty() => return Err("no client certificate is imported: chrome_cert_import first".into()),
        None => return Err(format!("several client certificates: pass cert, one of {}", keys.join(", "))),
    };
    let f = details(&cert)?;
    let twins = twins_of(&cert, &f);
    if !twins.is_empty() {
        return Err(format!(
            "{cert:?} and {} have the same subject and issuer, and Chrome's rule can only name those, so it could send either. Remove the one not wanted (chrome_cert_remove), or make no rule and answer each [certificate dialog opened] with chrome_choose_certificate(serial=…); chrome_cert_list shows serials and expiry.",
            twins.join(", ")
        ));
    }
    let filter = json!({"SUBJECT": dn_filter(&f.subject), "ISSUER": dn_filter(&f.issuer)});
    let mut rules = load_rules();
    rules.retain(|r| r.pattern != pattern);
    rules.push(Rule { site: a["site"].as_str().unwrap_or("").trim().to_string(), pattern: pattern.clone(), cert: cert.clone(), filter });
    save_rules(&rules)?;
    Ok(format!("{pattern} now gets client certificate {cert:?} without asking. chrome_restart makes it take effect."))
}

pub fn unselect(a: &Value) -> Result<String, String> {
    let pattern = site_pattern(a.get("site").and_then(Value::as_str).unwrap_or(""))?;
    let mut rules = load_rules();
    let before = rules.len();
    rules.retain(|r| r.pattern != pattern);
    if rules.len() == before {
        return Ok(format!("no rule for {pattern}"));
    }
    save_rules(&rules)?;
    Ok(format!("{pattern} no longer picks a certificate by itself. chrome_restart makes it take effect."))
}

pub fn list() -> Result<String, String> {
    ensure_db()?;
    normalize_names()?;
    let mut lines = vec![];
    for (name, flags) in listed()? {
        let f = details(&name).unwrap_or_default();
        let kind = if flags.contains('u') { "client" } else if flags.starts_with('C') || flags.starts_with("CT") { "trusted CA" } else { "other" };
        lines.push(format!("{name:?}  {kind}  {}  issued by {}  serial {}  until {}", f.subject, f.issuer, f.serial, f.until));
    }
    if lines.is_empty() {
        lines.push("no certificates".into());
    }
    for r in load_rules() {
        lines.push(format!("rule: {} gets {:?}", r.pattern, r.cert));
    }
    Ok(lines.join("\n"))
}

pub fn remove(a: &Value) -> Result<String, String> {
    let name = a.get("name").and_then(Value::as_str).map(str::trim).unwrap_or("");
    if name.is_empty() {
        return Err("name required: from chrome_cert_list".into());
    }
    normalize_names()?;
    if !listed()?.iter().any(|(n, _)| n == name) {
        return Err(format!("no certificate {name:?}; chrome_cert_list shows them"));
    }
    if client_names().iter().any(|k| k == name) {
        // takes the private key with it
        checked("certutil", &["-F", "-d", NSS, "-n", name, "-f", "/dev/null"])?;
    } else {
        checked("certutil", &["-D", "-d", NSS, "-n", name])?;
    }
    let mut rules = load_rules();
    let had = rules.len();
    rules.retain(|r| r.cert != name);
    if rules.len() != had {
        save_rules(&rules)?;
    }
    let stored = cas_dir().join(format!("{}.crt", safe_name(name)));
    let system = crate::install::sysroot().join(SYSTEM_CA_DIR).join(format!("bia-{}.crt", safe_name(name)));
    let was_system = system.exists();
    let _ = fs::remove_file(&stored);
    let _ = fs::remove_file(&system);
    if was_system {
        let out = Command::new("update-ca-certificates").arg("--fresh").output().map_err(|_| tool_missing("update-ca-certificates"))?;
        if !out.status.success() {
            return Err(format!("removed from Chrome, but update-ca-certificates: {}", text(&out.stderr)));
        }
    }
    Ok(format!("removed {name:?}{}. chrome_restart makes Chrome forget it.", if was_system { " from Chrome and from this system" } else { "" }))
}

// --- Chrome's certificate dialog, through accessibility (bia_desktop.py of the screen MCP) ---

fn desktop_script() -> Option<PathBuf> {
    if let Ok(s) = std::env::var("BIA_DESKTOP_SCRIPT") {
        return Some(PathBuf::from(s));
    }
    // MCPs/chrome/bin/<arch>/bia-mcp-chrome → MCPs/screen/bia_desktop.py
    let exe = std::env::current_exe().ok()?;
    let path = exe.ancestors().nth(3)?.parent()?.join("screen").join("bia_desktop.py");
    path.is_file().then_some(path)
}

fn desktop_command(script: &Path, request: &str) -> Result<Command, String> {
    let passwd = fs::read_to_string("/etc/passwd").map_err(|e| e.to_string())?;
    let uid = passwd
        .lines()
        .find_map(|l| {
            let mut f = l.split(':');
            (f.next() == Some(USER)).then(|| f.nth(1).map(str::to_string)).flatten()
        })
        .ok_or("there is no user bia")?;
    let run = format!("/run/user/{uid}");
    let mut c = Command::new("runuser");
    c.args(["-u", USER, "--", "python3", "-I"]).arg(script).arg(request);
    c.env("HOME", HOME)
        .env("USER", USER)
        .env("LANG", "C.UTF-8")
        .env("DISPLAY", crate::proc::DISPLAY)
        .env("XAUTHORITY", format!("{HOME}/.Xauthority"))
        .env("DBUS_SESSION_BUS_ADDRESS", format!("unix:path={run}/bus"))
        .env("XDG_RUNTIME_DIR", run)
        .env_remove("LD_LIBRARY_PATH");
    Ok(c)
}

/// The words of `chrome_choose_certificate`; `cert` may be a name from chrome_cert_list.
pub fn choose(a: &Value) -> Result<String, String> {
    let script = desktop_script().ok_or("the screen MCP's bia_desktop.py is missing: an OS update (carry_update) puts it back")?;
    let mut args = a.clone();
    // the dialog lists common names; a nickname from this store means its common name
    if let Some(nick) = a.get("cert").and_then(Value::as_str).map(str::trim).filter(|n| listed().is_ok_and(|l| l.iter().any(|(x, _)| x == n))) {
        // its serial alone says which, also when another has the same common name and issuer
        if let Ok(f) = details(nick) {
            args = json!({"serial": f.serial, "cancel": a.get("cancel").cloned().unwrap_or(Value::Null)});
        }
    }
    let request = json!({"cmd": "choose_cert", "args": args}).to_string();
    let out = desktop_command(&script, &request)?.stdin(Stdio::null()).output().map_err(|e| format!("start the desktop helper: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    match stdout.lines().last().and_then(|l| serde_json::from_str::<Value>(l).ok()) {
        Some(v) if v["ok"].as_bool() == Some(true) => Ok(v["text"].as_str().unwrap_or("").to_string()),
        Some(v) => Err(v["error"].as_str().unwrap_or("failed").to_string()),
        None => Err(format!("the desktop helper failed: {}", text(&out.stderr))),
    }
}

pub fn notice(d: &Value) -> String {
    let certs: Vec<String> = d["certs"]
        .as_array()
        .map(|a| a.iter().map(|c| format!("{} (issued by {}, serial {})", c["name"].as_str().unwrap_or(""), c["issuer"].as_str().unwrap_or(""), c["serial"].as_str().unwrap_or(""))).collect())
        .unwrap_or_default();
    format!(
        "[certificate dialog opened] {}\nOn offer: {}\nThe page waits until it is closed: chrome_choose_certificate(cert=<name>), or serial=<serial> when names repeat (chrome_cert_list shows each one's expiry), or cancel=true. chrome_cert_select(site, cert) answers it by itself from then on.",
        d["text"].as_str().unwrap_or(""),
        certs.join("; ")
    )
}

/// Tell the model each time a certificate dialog opens, as with file dialogs. One helper process
/// while Chrome runs: it holds the accessibility connection and says when the open dialogs change.
pub struct DialogWatch {
    child: Option<std::process::Child>,
    lines: Option<std::sync::mpsc::Receiver<String>>,
    known: Vec<String>,
    retry_at: std::time::Instant,
}

impl Default for DialogWatch {
    fn default() -> Self {
        DialogWatch { child: None, lines: None, known: vec![], retry_at: std::time::Instant::now() }
    }
}

impl DialogWatch {
    fn stop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        self.lines = None;
    }

    fn start(&mut self) {
        let now = std::time::Instant::now();
        if now < self.retry_at {
            return;
        }
        self.retry_at = now + std::time::Duration::from_secs(20);
        let Some(script) = desktop_script() else { return };
        let request = json!({"cmd": "watch_cert_dialogs"}).to_string();
        let Ok(mut command) = desktop_command(&script, &request) else { return };
        let Ok(mut child) = command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn() else { return };
        let Some(stdout) = child.stdout.take() else { return };
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            use std::io::BufRead;
            for line in std::io::BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        self.child = Some(child);
        self.lines = Some(rx);
    }

    /// Called about once a second; returns what to announce.
    pub fn poll(&mut self, chrome_up: bool) -> Vec<String> {
        if !chrome_up {
            self.stop();
            self.known.clear();
            return vec![];
        }
        if self.child.as_mut().is_none_or(|c| !matches!(c.try_wait(), Ok(None))) {
            self.stop();
            self.start();
        }
        let mut out = vec![];
        let Some(rx) = self.lines.as_ref() else { return out };
        while let Ok(line) = rx.try_recv() {
            // {"dialogs": [...]} with what is open now; {"ok": ...} only when it ended
            let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
            let Some(open) = v["dialogs"].as_array() else { continue };
            let keys: Vec<String> = open.iter().map(|d| d.to_string()).collect();
            for (d, key) in open.iter().zip(&keys) {
                if !self.known.contains(key) {
                    out.push(notice(d));
                }
            }
            self.known = keys;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dialog_is_told_once_with_what_is_on_offer() {
        let text = notice(&json!({"text": "Select a certificate for a.b:1", "certs": [{"name": "robot", "issuer": "Acme CA", "serial": "7A"}]}));
        assert!(text.starts_with("[certificate dialog opened] Select a certificate for a.b:1"), "{text}");
        assert!(text.contains("robot (issued by Acme CA, serial 7A)") && text.contains("chrome_choose_certificate(cert=<name>)"), "{text}");
        let mut w = DialogWatch::default();
        assert!(w.poll(false).is_empty());
        let (tx, rx) = std::sync::mpsc::channel();
        w.lines = Some(rx);
        w.retry_at = std::time::Instant::now() + std::time::Duration::from_secs(60);
        w.child = Some(Command::new("sleep").arg("30").spawn().unwrap());
        let open = r#"{"dialogs":[{"text":"t","certs":[{"name":"robot","issuer":"i"}]}]}"#;
        tx.send(open.to_string()).unwrap();
        assert_eq!(w.poll(true).len(), 1, "opened");
        tx.send(open.to_string()).unwrap();
        assert!(w.poll(true).is_empty(), "still the same one");
        tx.send(r#"{"dialogs":[]}"#.to_string()).unwrap();
        assert!(w.poll(true).is_empty(), "closed: nothing to tell");
        tx.send(open.to_string()).unwrap();
        assert_eq!(w.poll(true).len(), 1, "opened again");
        w.stop();
    }

    #[test]
    fn names_stay_reachable_and_unique() {
        assert_eq!(clean_nick("Identity: root"), "Identity root");
        assert_eq!(clean_nick(":"), "certificate");
        let f = Fields { subject: "CN=Identity: root".into(), serial: "73CB2678A1490BC8".into(), ..Default::default() };
        assert_eq!(nick_for(&f, &[]), "Identity root");
        let taken = vec!["Identity root".to_string()];
        assert_eq!(nick_for(&f, &taken), "Identity root #A1490BC8");
        let both = vec!["Identity root".to_string(), "Identity root #A1490BC8".to_string()];
        assert_eq!(nick_for(&f, &both), "Identity root #73CB2678A1490BC8");
    }

    #[test]
    fn sites_become_patterns() {
        assert_eq!(site_pattern("portal.example.com").unwrap(), "https://portal.example.com");
        assert_eq!(site_pattern("https://portal.example.com:8443/login?x=1").unwrap(), "https://portal.example.com:8443");
        assert_eq!(site_pattern("https://[*.]example.com").unwrap(), "https://[*.]example.com");
        assert!(site_pattern(" ").is_err());
        assert!(site_pattern("a b").is_err());
        assert!(site_pattern("user@host").is_err());
    }

    #[test]
    fn names_are_read_with_their_escapes() {
        let dn = r"CN=Ann\, Lee,OU=Dev,O=Acme\, Inc,C=DE";
        assert_eq!(name_field(dn, "CN").as_deref(), Some("Ann, Lee"));
        assert_eq!(name_field(dn, "O").as_deref(), Some("Acme, Inc"));
        assert_eq!(name_field(dn, "L"), None);
        assert_eq!(dn_filter(dn), json!({"CN": "Ann, Lee", "O": "Acme, Inc", "OU": "Dev"}));
    }

    #[test]
    fn the_policy_holds_each_rule_as_a_json_string() {
        let rules = vec![Rule { site: "a.b".into(), pattern: "https://a.b".into(), cert: "c".into(), filter: json!({"SUBJECT": {"CN": "c"}}) }];
        let policy = policy_of(&rules);
        let entry = policy["AutoSelectCertificateForUrls"][0].as_str().unwrap();
        let inner: Value = serde_json::from_str(entry).unwrap();
        assert_eq!(inner["pattern"], "https://a.b");
        assert_eq!(inner["filter"]["SUBJECT"]["CN"], "c");
    }
}
