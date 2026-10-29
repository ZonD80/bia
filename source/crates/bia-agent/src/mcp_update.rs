//! One MCP updated from a GitHub release, without updating the system: every release
//! publishes each shipped MCP as `mcp-<name>.tar.gz` (the whole folder, both
//! architectures' binaries, as in /home/bia/MCPs) and lists it in `bia-os.json` under
//! "mcps" with its version, size and sha256.
//!
//! The archive is unpacked over a copy of the installed folder (files the image added
//! there, such as chrome's native host, stay), the copy replaces the folder and the old one
//! is kept as `.<name>.prev`; the caller restarts the MCP and calls `rollback` when the new
//! one does not start.
use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const REPO: &str = "ZonD80/bia";

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub version: String,
    pub file: String,
    pub size: u64,
    pub sha256: String,
}

pub struct Release {
    pub tag: String,
    pub mcps: BTreeMap<String, Entry>,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(600)))
        .tls_config(ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::WebPki).build())
        .build()
        .into()
}

fn base(tag: Option<&str>) -> String {
    match tag.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => format!("https://github.com/{REPO}/releases/download/{}", if t.starts_with('v') { t.to_string() } else { format!("v{t}") }),
        None => format!("https://github.com/{REPO}/releases/latest/download"),
    }
}

/// The release's MCP list (`tag` like v1.7.1, or the latest release).
pub fn release(tag: Option<&str>) -> Result<Release> {
    let url = format!("{}/bia-os.json", base(tag));
    let resp = agent().get(&url).call().with_context(|| format!("download {url}"))?;
    let status = resp.status().as_u16();
    if status != 200 {
        bail!("{url}: HTTP {status}");
    }
    let text = resp.into_body().read_to_string().context("read bia-os.json")?;
    parse(&serde_json::from_str(&text).context("parse bia-os.json")?)
}

fn parse(v: &Value) -> Result<Release> {
    let tag = v["tag"].as_str().unwrap_or("").to_string();
    let mut mcps = BTreeMap::new();
    if let Some(m) = v["mcps"].as_object() {
        for (name, e) in m {
            let s = |k: &str| e[k].as_str().unwrap_or("").to_string();
            let entry = Entry { version: s("version"), file: s("file"), size: e["size"].as_u64().unwrap_or(0), sha256: s("sha256") };
            if !entry.file.is_empty() && entry.sha256.len() == 64 {
                mcps.insert(name.clone(), entry);
            }
        }
    }
    if mcps.is_empty() {
        bail!("release {tag} publishes no MCP archives (they start with bia 1.7.1)");
    }
    Ok(Release { tag, mcps })
}

/// The version in an installed MCP's mcp.json ("" when it has none).
pub fn installed_version(root: &Path, name: &str) -> Option<String> {
    let text = std::fs::read_to_string(root.join(name).join("mcp.json")).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    Some(v["version"].as_str().unwrap_or("").to_string())
}

/// `1.10.2` → (1, 10, 2); anything unparsable sorts first.
fn key(v: &str) -> (u32, u32, u32) {
    let mut it = v.trim().trim_start_matches('v').split(|c: char| !c.is_ascii_digit()).map(|p| p.parse().unwrap_or(0));
    (it.next().unwrap_or(0), it.next().unwrap_or(0), it.next().unwrap_or(0))
}

pub fn newer(a: &str, b: &str) -> bool {
    key(a) > key(b)
}

/// Each MCP of the release next to what is installed.
pub fn check(root: &Path, rel: &Release, system: &str) -> String {
    let mut out = vec![if newer(&rel.tag, system) {
        format!("release {} is newer than this system (bia {system}): carry_update(from=\"internet\") installs it (several reboots). Its MCPs:", rel.tag)
    } else {
        format!("release {} (this system is bia {system}, up to date):", rel.tag)
    }];
    for (name, e) in &rel.mcps {
        let line = match installed_version(root, name) {
            None => format!("  {name}: not installed here ({} available)", e.version),
            Some(v) if newer(&e.version, &v) => format!("  {name}: {v} → {} available ({})", e.version, human(e.size)),
            Some(v) => format!("  {name}: {v}, up to date"),
        };
        out.push(line);
    }
    out.push(
        "mcp_update(name) updates one; an MCP is built with the bia system of its release, so one much newer than the system may need what only a system update brings: the update starts it and puts the old one back if it does not start."
            .into(),
    );
    out.join("\n")
}

fn human(n: u64) -> String {
    if n >= 1_000_000 { format!("{:.1} MB", n as f64 / 1e6) } else { format!("{} KB", n.div_ceil(1000)) }
}

pub struct Staged {
    pub from: String,
    pub to: String,
}

fn new_dir(root: &Path, name: &str) -> PathBuf {
    root.join(format!(".{name}.new"))
}

fn prev_dir(root: &Path, name: &str) -> PathBuf {
    root.join(format!(".{name}.prev"))
}

/// Download `name`'s archive, check it, unpack it over a copy of the installed folder and put
/// the copy in place (the old folder becomes `.<name>.prev`).
pub fn install(root: &Path, rel: &Release, name: &str) -> Result<Staged> {
    let e = rel.mcps.get(name).ok_or_else(|| anyhow!("release {} has no MCP {name} (it has: {})", rel.tag, rel.mcps.keys().cloned().collect::<Vec<_>>().join(", ")))?;
    let dest = root.join(name);
    let from = installed_version(root, name).unwrap_or_default();
    let url = format!("{}/{}", base(Some(&rel.tag)), e.file);
    let resp = agent().get(&url).call().with_context(|| format!("download {url}"))?;
    let status = resp.status().as_u16();
    if status != 200 {
        bail!("{url}: HTTP {status}");
    }
    let mut data = Vec::with_capacity(e.size as usize);
    resp.into_body().with_config().limit(e.size + 1).reader().read_to_end(&mut data).with_context(|| format!("download {url}"))?;
    if data.len() as u64 != e.size {
        bail!("{}: {} bytes, bia-os.json says {}", e.file, data.len(), e.size);
    }
    let sum: String = Sha256::digest(&data).iter().map(|b| format!("{b:02x}")).collect();
    if sum != e.sha256 {
        bail!("{}: sha256 {sum} does not match bia-os.json ({})", e.file, e.sha256);
    }
    let staging = new_dir(root, name);
    let _ = std::fs::remove_dir_all(&staging);
    if dest.is_dir() {
        run("cp", &["-a", &dest.display().to_string(), &staging.display().to_string()])?;
    } else {
        std::fs::create_dir_all(&staging)?;
    }
    let tgz = root.join(format!(".{name}.tar.gz"));
    std::fs::write(&tgz, &data)?;
    // the archive holds <name>/…: unpack its contents straight into the staging copy
    let r = run("tar", &["-xzf", &tgz.display().to_string(), "-C", &staging.display().to_string(), "--strip-components=1", "--no-same-owner"]);
    let _ = std::fs::remove_file(&tgz);
    if let Err(err) = r {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(err);
    }
    let to = installed_version(root, &format!(".{name}.new")).unwrap_or_else(|| e.version.clone());
    let prev = prev_dir(root, name);
    let _ = std::fs::remove_dir_all(&prev);
    if dest.is_dir() {
        std::fs::rename(&dest, &prev).with_context(|| format!("move {} aside", dest.display()))?;
    }
    if let Err(err) = std::fs::rename(&staging, &dest) {
        let _ = std::fs::rename(&prev, &dest);
        return Err(anyhow!("put {} in place: {err}", dest.display()));
    }
    // MCPs run as root or bia; the folder belongs to bia like the rest of /home/bia/MCPs
    let _ = Command::new("chown").args(["-R", "bia:bia", &dest.display().to_string()]).status();
    Ok(Staged { from, to })
}

/// The new MCP did not start: the previous folder back.
pub fn rollback(root: &Path, name: &str) -> Result<()> {
    let prev = prev_dir(root, name);
    if !prev.is_dir() {
        bail!("no previous {name} kept to go back to");
    }
    let dest = root.join(name);
    let failed = root.join(format!(".{name}.failed"));
    let _ = std::fs::remove_dir_all(&failed);
    if dest.exists() {
        std::fs::rename(&dest, &failed)?;
    }
    std::fs::rename(&prev, &dest)?;
    let _ = std::fs::remove_dir_all(&failed);
    Ok(())
}

fn run(cmd: &str, args: &[&str]) -> Result<()> {
    let out = Command::new(cmd).args(args).output().with_context(|| format!("run {cmd}"))?;
    if !out.status.success() {
        bail!("{cmd} failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_the_mcp_list_and_compares_versions() {
        let r = parse(&json!({"tag": "v1.7.1", "mcps": {
            "telegram": {"version": "1.7.1", "file": "mcp-telegram.tar.gz", "size": 12, "sha256": "a".repeat(64)},
            "broken": {"version": "1.7.1", "file": "", "size": 1, "sha256": "x"}
        }}))
        .unwrap();
        assert_eq!(r.tag, "v1.7.1");
        assert_eq!(r.mcps.keys().collect::<Vec<_>>(), ["telegram"]);
        assert!(parse(&json!({"tag": "v1.7.0", "os": {}})).is_err(), "no MCPs published");
        assert!(newer("1.7.1", "1.7.0") && newer("1.10.0", "1.9.9") && !newer("1.7.1", "1.7.1"));
        assert_eq!(base(Some("1.7.1")), "https://github.com/ZonD80/bia/releases/download/v1.7.1");
    }

    #[test]
    fn install_unpacks_over_a_copy_and_rolls_back() {
        let dir = std::env::temp_dir().join(format!("bia-mcp-update-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let root = dir.join("MCPs");
        // installed: old binary plus a file the image added
        std::fs::create_dir_all(root.join("demo/bin/x86_64")).unwrap();
        std::fs::write(root.join("demo/mcp.json"), r#"{"version": "1.7.0"}"#).unwrap();
        std::fs::write(root.join("demo/bin/x86_64/demo"), b"old").unwrap();
        std::fs::write(root.join("demo/native-host"), b"from the image").unwrap();
        // the archive: demo/ with a new mcp.json and binary
        let src = dir.join("src/demo/bin/x86_64");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(dir.join("src/demo/mcp.json"), r#"{"version": "1.7.1"}"#).unwrap();
        std::fs::write(src.join("demo"), b"new").unwrap();
        let tgz = dir.join("mcp-demo.tar.gz");
        run("tar", &["-czf", tgz.to_str().unwrap(), "-C", dir.join("src").to_str().unwrap(), "demo"]).unwrap();
        let data = std::fs::read(&tgz).unwrap();
        let sum: String = Sha256::digest(&data).iter().map(|b| format!("{b:02x}")).collect();
        // install() downloads; here the unpack-and-swap part runs on the local archive
        let staging = new_dir(&root, "demo");
        run("cp", &["-a", root.join("demo").to_str().unwrap(), staging.to_str().unwrap()]).unwrap();
        run("tar", &["-xzf", tgz.to_str().unwrap(), "-C", staging.to_str().unwrap(), "--strip-components=1"]).unwrap();
        std::fs::rename(root.join("demo"), prev_dir(&root, "demo")).unwrap();
        std::fs::rename(&staging, root.join("demo")).unwrap();
        assert_eq!(installed_version(&root, "demo").as_deref(), Some("1.7.1"));
        assert_eq!(std::fs::read(root.join("demo/bin/x86_64/demo")).unwrap(), b"new");
        assert_eq!(std::fs::read(root.join("demo/native-host")).unwrap(), b"from the image", "the image's own files stay");
        assert_eq!(sum.len(), 64);
        rollback(&root, "demo").unwrap();
        assert_eq!(installed_version(&root, "demo").as_deref(), Some("1.7.0"));
        assert!(rollback(&root, "demo").is_err(), "nothing kept any more");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
