//! Pack `extension/` with the shipped `extension.pem` by asking Chrome.
//! Chrome rejects a CRX it did not sign when the extension uses webAuthenticationProxy.
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

/// Pack when `extension.crx` is missing or `extension/` changed since the last pack.
/// A changed tree gets a higher manifest version so Chrome's update check installs it.
pub fn ensure(dir: &Path) -> Result<(), String> {
    let ext = dir.join("extension");
    let manifest = ext.join("manifest.json");
    if !manifest.is_file() {
        return Err(format!("missing {} — the extension source ships next to this MCP", manifest.display()));
    }
    let hash = tree_hash(&ext)?;
    let stamp = fs::read_to_string(dir.join("extension.packstamp")).unwrap_or_default();
    let crx_path = dir.join("extension.crx");
    let have_crx = crx_path.is_file();
    if have_crx && stamp.trim() == hash {
        return Ok(());
    }
    let bin = chrome_bin().ok_or_else(|| crate::proc::MISSING.to_string())?;
    if cfg!(target_os = "linux") && !Path::new("/tmp/.X11-unix/X0").exists() {
        return Err("display :0 is not up, so Chrome cannot pack the extension".into());
    }
    let original = fs::read(&manifest).map_err(|e| format!("read manifest: {e}"))?;
    // Chrome installs a force-installed CRX only when the version increases.
    let replacing = have_crx || (!stamp.trim().is_empty() && stamp.trim() != hash);
    if replacing {
        bump_manifest(&manifest)?;
    }
    let packed = pack_with_chrome(&bin, dir);
    if let Err(e) = packed {
        if replacing {
            let _ = fs::write(&manifest, &original);
        }
        return Err(e);
    }
    let hash = tree_hash(&ext)?;
    fs::write(dir.join("extension.packstamp"), &hash).map_err(|e| format!("write pack stamp: {e}"))?;
    give_bia(&manifest);
    give_bia(&crx_path);
    give_bia(&dir.join("extension.packstamp"));
    Ok(())
}

pub fn chrome_bin() -> Option<PathBuf> {
    if let Some(p) = crate::proc::find_bin() {
        return Some(p);
    }
    for p in [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
    ] {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

fn pack_with_chrome(bin: &Path, dir: &Path) -> Result<(), String> {
    let tmp_root = std::env::temp_dir().join(format!("bia-crx-pack-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp_root);
    fs::create_dir_all(&tmp_root).map_err(|e| format!("{}: {e}", tmp_root.display()))?;
    let guard = Tmp(tmp_root.clone());
    let staged = tmp_root.join("extension");
    copy_tree(&dir.join("extension"), &staged)?;
    let pem = dir.join("extension.pem");
    let produced = tmp_root.join("extension.crx");
    let dest = dir.join("extension.crx");
    let err = run_pack(bin, &staged, &pem, &produced, &tmp_root.join("profile"))?;
    if !produced.is_file() {
        return Err(format!("Chrome did not write a CRX. {err}"));
    }
    let bytes = fs::read(&produced).map_err(|e| format!("read packed crx: {e}"))?;
    if !is_crx3(&bytes) {
        return Err(format!("Chrome wrote a file that is not a CRX3. {err}"));
    }
    fs::copy(&produced, &dest).map_err(|e| format!("install crx: {e}"))?;
    drop(guard);
    Ok(())
}

fn run_pack(bin: &Path, ext: &Path, pem: &Path, crx: &Path, profile: &Path) -> Result<String, String> {
    let _ = fs::remove_dir_all(profile);
    fs::create_dir_all(profile).map_err(|e| format!("{}: {e}", profile.display()))?;
    let mut cmd = Command::new(bin);
    cmd.arg(format!("--user-data-dir={}", profile.display()))
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--disable-gpu")
        .arg("--disable-dev-shm-usage")
        .arg("--disable-sync")
        .arg(format!("--pack-extension={}", ext.display()))
        .arg(format!("--pack-extension-key={}", pem.display()))
        .env("HOME", profile)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    if cfg!(target_os = "linux") {
        cmd.arg("--no-sandbox").arg("--ozone-platform=x11");
        let display = std::env::var("DISPLAY").unwrap_or_else(|_| ":0".into());
        cmd.env("DISPLAY", display);
        if let Some(auth) = xauthority() {
            cmd.env("XAUTHORITY", auth);
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = cmd.spawn().map_err(|e| format!("start chrome to pack: {e}"))?;
    let mut err_pipe = child.stderr.take();
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(pipe) = err_pipe.as_mut() {
            let _ = pipe.read_to_end(&mut buf);
        }
        buf
    });
    let pid = child.id();
    let deadline = Instant::now() + Duration::from_secs(45);
    let mut last = 0u64;
    let mut stable = 0u8;
    loop {
        if let Ok(meta) = fs::metadata(crx) {
            if meta.len() > 16 && meta.len() == last {
                stable += 1;
                if stable >= 3 {
                    let grace = Instant::now() + Duration::from_secs(2);
                    while Instant::now() < grace {
                        if child.try_wait().map_err(|e| format!("chrome pack: {e}"))?.is_some() {
                            break;
                        }
                        sleep(Duration::from_millis(50));
                    }
                    break;
                }
            } else {
                last = meta.len();
                stable = 0;
            }
        }
        if child.try_wait().map_err(|e| format!("chrome pack: {e}"))?.is_some() {
            break;
        }
        if Instant::now() > deadline {
            break;
        }
        sleep(Duration::from_millis(100));
    }
    #[cfg(unix)]
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
    let _ = child.wait();
    let err = err_thread.join().unwrap_or_default();
    let err = tail(&err);
    if !crx.is_file() {
        let why = if err.is_empty() { "Chrome exited without writing a CRX".into() } else { err.clone() };
        return Err(why);
    }
    Ok(err)
}

fn xauthority() -> Option<PathBuf> {
    if let Ok(v) = std::env::var("XAUTHORITY") {
        let p = PathBuf::from(v);
        if p.is_file() {
            return Some(p);
        }
    }
    for p in ["/run/bia/Xauthority", "/home/bia/.Xauthority"] {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

fn tail(buf: &[u8]) -> String {
    let s = String::from_utf8_lossy(buf);
    let s = s.trim();
    let count = s.chars().count();
    if count > 400 {
        s.chars().skip(count - 400).collect()
    } else {
        s.to_string()
    }
}

fn bump_manifest(path: &Path) -> Result<(), String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut v: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("manifest: {e}"))?;
    let cur = v.get("version").and_then(|x| x.as_str()).unwrap_or("0.0.0");
    let next = bump_version(cur);
    v["version"] = serde_json::Value::String(next);
    let mut out = serde_json::to_vec_pretty(&v).map_err(|e| e.to_string())?;
    out.push(b'\n');
    fs::write(path, out).map_err(|e| format!("write manifest: {e}"))?;
    Ok(())
}

fn bump_version(version: &str) -> String {
    let mut parts: Vec<u32> = Vec::new();
    for p in version.split('.') {
        match p.parse::<u32>() {
            Ok(n) if n <= 65535 && parts.len() < 4 => parts.push(n),
            _ => return format!("{version}.1"),
        }
    }
    if parts.is_empty() {
        return "0.0.1".into();
    }
    let last = parts.len() - 1;
    if parts[last] < 65535 {
        parts[last] += 1;
    } else if parts.len() < 4 {
        parts.push(1);
    } else {
        parts[last] = 0;
        let mut i = last;
        while i > 0 {
            i -= 1;
            if parts[i] < 65535 {
                parts[i] += 1;
                return join_version(&parts);
            }
            parts[i] = 0;
        }
        parts[last] = 1;
    }
    join_version(&parts)
}

fn join_version(parts: &[u32]) -> String {
    parts.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(".")
}

fn tree_hash(root: &Path) -> Result<String, String> {
    let mut files = Vec::new();
    collect(root, root, &mut files)?;
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut hasher = Sha256::new();
    for (rel, bytes) in files {
        hasher.update(rel.as_bytes());
        hasher.update([0]);
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
    }
    Ok(hex(&hasher.finalize()))
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) -> Result<(), String> {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    entries.sort_by_key(|e| e.file_name());
    for ent in entries {
        let name = ent.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let path = ent.path();
        if path.is_dir() {
            collect(root, &path, out)?;
        } else if path.is_file() {
            let rel = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
            let bytes = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            out.push((rel, bytes));
        }
    }
    Ok(())
}

fn copy_tree(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| format!("{}: {e}", dst.display()))?;
    for ent in fs::read_dir(src).map_err(|e| format!("{}: {e}", src.display()))? {
        let ent = ent.map_err(|e| e.to_string())?;
        if ent.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let from = ent.path();
        let to = dst.join(ent.file_name());
        if from.is_dir() {
            copy_tree(&from, &to)?;
        } else if from.is_file() {
            fs::copy(&from, &to).map_err(|e| format!("{}: {e}", from.display()))?;
        }
    }
    Ok(())
}

fn is_crx3(bytes: &[u8]) -> bool {
    bytes.len() > 16 && bytes.starts_with(b"Cr24") && bytes[4..8] == 3u32.to_le_bytes()
}

fn hex(bytes: &[u8]) -> String {
    const H: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(H[(b >> 4) as usize] as char);
        s.push(H[(b & 0xf) as usize] as char);
    }
    s
}

fn give_bia(path: &Path) {
    if let Some(s) = path.to_str() {
        let _ = Command::new("chown").args(["bia:bia", s]).status();
    }
}

struct Tmp(PathBuf);
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_bumps_the_last_component() {
        assert_eq!(bump_version("0.2.0"), "0.2.1");
        assert_eq!(bump_version("1"), "2");
        assert_eq!(bump_version("0.2.65535"), "0.2.65535.1");
        assert_eq!(bump_version("1.2.3.65535"), "1.2.4.0");
    }

    #[test]
    fn hash_ignores_dotfiles() {
        let dir = std::env::temp_dir().join(format!("bia-ext-hash-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.js"), b"one").unwrap();
        let h1 = tree_hash(&dir).unwrap();
        fs::write(dir.join("a.js"), b"two").unwrap();
        let h2 = tree_hash(&dir).unwrap();
        assert_ne!(h1, h2);
        fs::write(dir.join(".DS_Store"), b"junk").unwrap();
        assert_eq!(tree_hash(&dir).unwrap(), h2);
        let _ = fs::remove_dir_all(&dir);
    }
}
