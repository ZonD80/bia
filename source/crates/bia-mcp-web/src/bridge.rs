//! bia-bridge for agents, served by this bia: the builds for every platform that ship in
//! `MCPs/web/bridge/` (next to this binary's `bin/`), from the same build as this server, so
//! an agent always gets the bridge that matches this bia. Public: an agent has no
//! certificate yet when it fetches one (it checks the CA fingerprint and the sha256 here).
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

/// `BIA_BRIDGE_DIR`, else `<MCP folder>/bridge` (this binary is `<MCP folder>/bin/<arch>/…`).
pub fn dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("BIA_BRIDGE_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(d));
    }
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.parent()?.parent()?.join("bridge"))
}

/// `bia-bridge-linux-x86_64` → `linux-x86_64`, `bia-bridge-windows-x86_64.exe` → `windows-x86_64`.
fn platform(file: &str) -> Option<String> {
    let p = file.strip_prefix("bia-bridge-")?;
    let p = p.strip_suffix(".exe").unwrap_or(p);
    (!p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')).then(|| p.to_string())
}

static SUMS: Mutex<Option<HashMap<PathBuf, (u64, u64, String)>>> = Mutex::new(None);

fn sha256(p: &std::path::Path, m: &std::fs::Metadata) -> Option<String> {
    let mtime = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    let mut g = SUMS.lock().unwrap();
    let cache = g.get_or_insert_with(HashMap::new);
    if let Some((t, n, s)) = cache.get(p) {
        if *t == mtime && *n == m.len() {
            return Some(s.clone());
        }
    }
    use std::io::Read;
    let mut f = std::fs::File::open(p).ok()?;
    let mut ctx = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        ctx.update(&buf[..n]);
    }
    let s: String = ctx.finish().as_ref().iter().map(|b| format!("{b:02x}")).collect();
    cache.insert(p.to_path_buf(), (mtime, m.len(), s.clone()));
    Some(s)
}

/// What `GET /v1/bridge` answers.
pub fn list() -> Value {
    let mut files = vec![];
    if let Some(d) = dir() {
        let mut names: Vec<String> = std::fs::read_dir(&d)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| platform(n).is_some())
            .collect();
        names.sort();
        for n in names {
            let p = d.join(&n);
            let Ok(m) = std::fs::metadata(&p) else { continue };
            if !m.is_file() {
                continue;
            }
            files.push(json!({
                "platform": platform(&n),
                "file": n,
                "size": m.len(),
                "sha256": sha256(&p, &m),
                "url": format!("/bridge/{n}"),
            }));
        }
    }
    json!({
        "version": env!("CARGO_PKG_VERSION"),
        "files": files,
        "mcp": {"command": "bia-bridge", "args": ["mcp"]},
        "how": "pick your platform, download it with this bia's CA pinned (GET /v1/pki/ca, compare its sha256 with ca_sha256 in /.well-known/bia.json), check the sha256 here, make it executable, then: bia-bridge connect https://<this address> --identity <your name>",
    })
}

/// The file `GET /bridge/<name>` serves (only names `list` shows).
pub fn file(name: &str) -> Option<PathBuf> {
    platform(name)?;
    if name.contains(['/', '\\']) || name.contains("..") {
        return None;
    }
    let p = dir()?.join(name);
    p.is_file().then_some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(platform("bia-bridge-linux-arm64").as_deref(), Some("linux-arm64"));
        assert_eq!(platform("bia-bridge-windows-x86_64.exe").as_deref(), Some("windows-x86_64"));
        assert_eq!(platform("bia-bridge-macos").as_deref(), Some("macos"));
        assert!(platform("bia-bridge-macos.notarized").is_none() && platform("bia-bridge-macos.unsigned").is_none());
        assert!(platform("README.md").is_none() && platform("bia-bridge-").is_none() && platform("bia-bridge-../x").is_none());
    }
}
