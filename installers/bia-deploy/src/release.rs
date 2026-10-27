//! The bia OS from GitHub releases: `bia-os.json` lists, per architecture, the bia disk
//! image `bia-<arch>.img.xz` (see `image.rs`) with the sha256 of the download and of the
//! raw image. The writer downloads it with resume into a cache and decodes it while it
//! writes the stick, so the next stick needs no download.
//!
//! Cache: `<cache>/bia-deploy/os/<tag>/<arch>/bia-<arch>.img.xz` (`BIA_DEPLOY_CACHE`
//! overrides `<cache>/bia-deploy`).

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const REPO: &str = "zond80/bia";
pub const MANIFEST: &str = "bia-os.json";

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub version: String,
    pub tag: String,
    pub os: BTreeMap<String, OsEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OsEntry {
    pub image: ImageAsset,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ImageAsset {
    /// Release asset name, e.g. `bia-x86_64.img.xz`.
    pub file: String,
    /// Download (xz) size and sha256.
    pub size: u64,
    pub sha256: String,
    /// Raw image size and sha256 (checked while the stick is written).
    pub raw_size: u64,
    pub raw_sha256: String,
}

/// A downloaded release image, ready for `write_stick`.
#[derive(Debug, Clone)]
pub struct OsImage {
    pub tag: String,
    pub path: PathBuf,
    pub raw_sha256: String,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(20)))
        .user_agent("bia-deploy")
        .build()
        .into()
}

/// `latest` (or empty) for the newest release, else a tag such as `v1.0.0`.
/// `BIA_RELEASE_URL` replaces GitHub (a directory with bia-os.json and the files; tests).
fn base_url(tag: &str) -> String {
    if let Ok(u) = std::env::var("BIA_RELEASE_URL") {
        return u.trim_end_matches('/').to_string();
    }
    match tag.trim() {
        "" | "latest" => format!("https://github.com/{REPO}/releases/latest/download"),
        t => format!("https://github.com/{REPO}/releases/download/{t}"),
    }
}

pub fn manifest(tag: &str) -> Result<Manifest> {
    let url = format!("{}/{MANIFEST}", base_url(tag));
    let resp = agent().get(&url).call().with_context(|| format!("GET {url} (no bia release with OS images yet?)"))?;
    serde_json::from_reader(resp.into_body().into_reader()).with_context(|| format!("parse {url}"))
}

pub fn arches(m: &Manifest) -> Vec<String> {
    m.os.keys().cloned().collect()
}

pub fn cache_root() -> PathBuf {
    if let Some(p) = std::env::var_os("BIA_DEPLOY_CACHE") {
        return PathBuf::from(p);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "macos") {
        if let Some(h) = home {
            return h.join("Library/Caches/bia-deploy");
        }
    } else if cfg!(windows) {
        if let Some(p) = std::env::var_os("LOCALAPPDATA") {
            return PathBuf::from(p).join("bia-deploy").join("cache");
        }
    } else if let Some(p) = std::env::var_os("XDG_CACHE_HOME") {
        return PathBuf::from(p).join("bia-deploy");
    } else if let Some(h) = home {
        return h.join(".cache/bia-deploy");
    }
    std::env::temp_dir().join("bia-deploy")
}

fn cache_dir(m: &Manifest, arch: &str) -> PathBuf {
    cache_root().join("os").join(&m.tag).join(arch)
}

/// Bytes still to download for `arch` of this release (0 = cached).
pub fn missing_bytes(m: &Manifest, arch: &str) -> u64 {
    let Some(e) = m.os.get(arch) else { return 0 };
    let dir = cache_dir(m, arch);
    if dir.join(".complete").is_file() && dir.join(&e.image.file).is_file() {
        return 0;
    }
    let have = fs::metadata(dir.join(format!("{}.download", e.image.file))).map_or(0, |m| m.len());
    e.image.size.saturating_sub(have)
}

/// Download (or reuse) the bia image for `arch` from release `m`.
pub fn fetch(m: &Manifest, arch: &str, progress: &mut dyn FnMut(&str)) -> Result<OsImage> {
    let entry = m
        .os
        .get(arch)
        .with_context(|| format!("release {} has no {arch} OS (has: {})", m.tag, arches(m).join(", ")))?;
    let name = &entry.image.file;
    if name.contains(['/', '\\']) || name.starts_with('.') {
        bail!("{MANIFEST}: bad file name {name:?}");
    }
    let dir = cache_dir(m, arch);
    fs::create_dir_all(&dir).with_context(|| format!("mkdir {}", dir.display()))?;
    let out = OsImage { tag: m.tag.clone(), path: dir.join(name), raw_sha256: entry.image.raw_sha256.clone() };
    if dir.join(".complete").is_file() && out.path.is_file() {
        progress(&format!("bia OS {} ({arch}) is already downloaded", m.tag));
        return Ok(out);
    }
    // Older writers cached root.img and boot files here; they are not used any more.
    let _ = fs::remove_file(dir.join("root.img"));
    let _ = fs::remove_dir_all(dir.join("boot"));
    let part = dir.join(format!("{name}.download"));
    let url = format!("{}/{name}", base_url(&m.tag));
    download(&url, &part, entry.image.size, &entry.image.sha256, &format!("bia {arch} image"), progress)?;
    fs::rename(&part, &out.path)?;
    File::create(dir.join(".complete"))?;
    progress(&format!("bia OS {} ({arch}) ready in {}", m.tag, dir.display()));
    Ok(out)
}

/// Download `url` into `path`, resuming a partial file; check size and sha256.
fn download(url: &str, path: &Path, size: u64, sha256: &str, what: &str, progress: &mut dyn FnMut(&str)) -> Result<()> {
    let mut hash = Sha256::new();
    let mut got = 0u64;
    if let Ok(mut f) = File::open(path) {
        // hash what is already there, so a resumed download is still checked whole
        let mut buf = vec![0u8; 4 << 20];
        loop {
            let k = f.read(&mut buf)?;
            if k == 0 {
                break;
            }
            hash.update(&buf[..k]);
            got += k as u64;
        }
        if got > size {
            drop(f);
            fs::remove_file(path)?;
            hash = Sha256::new();
            got = 0;
        }
    }
    let mut out = OpenOptions::new().create(true).append(true).open(path).with_context(|| format!("open {}", path.display()))?;
    let mut buf = vec![0u8; 1 << 20];
    let mut failures = 0;
    let mut last_pct = u64::MAX;
    while got < size {
        let mut req = agent().get(url);
        if got > 0 {
            req = req.header("Range", &format!("bytes={got}-"));
        }
        let err = match req.call() {
            Err(ureq::Error::StatusCode(code)) if (400..500).contains(&code) => bail!("{what}: HTTP {code} for {url}"),
            Err(e) => anyhow::Error::from(e),
            Ok(resp) => {
                if got > 0 && resp.status().as_u16() != 206 {
                    bail!("{what}: the server ignored the resume range");
                }
                let mut body = resp.into_body().into_reader();
                loop {
                    let k = match body.read(&mut buf) {
                        Ok(0) => break anyhow::anyhow!("connection closed early"),
                        Ok(k) => k.min((size - got) as usize),
                        Err(e) => break e.into(),
                    };
                    out.write_all(&buf[..k]).with_context(|| format!("write {}", path.display()))?;
                    hash.update(&buf[..k]);
                    got += k as u64;
                    failures = 0;
                    let pct = got * 100 / size.max(1);
                    if pct != last_pct && (pct % 5 == 0 || got == size) {
                        last_pct = pct;
                        progress(&format!("downloading {what}: {pct}% ({} / {} MiB)", got >> 20, size >> 20));
                    }
                    if got == size {
                        break anyhow::anyhow!("done");
                    }
                }
            }
        };
        if got == size {
            break;
        }
        failures += 1;
        if failures > 8 {
            return Err(err).with_context(|| format!("download {what}"));
        }
        progress(&format!("{what}: {err}; retrying"));
        std::thread::sleep(Duration::from_secs(2));
    }
    out.sync_all().ok();
    if !hex(&hash.finalize()).eq_ignore_ascii_case(sha256) {
        let _ = fs::remove_file(path);
        bail!("{what}: checksum does not match the release; the download was removed, try again");
    }
    Ok(())
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;

    /// Serves files from `dir` over HTTP with Range support; `cut` makes the first answer
    /// for the image stop half way (a dropped connection) so the download must resume.
    fn serve(dir: PathBuf, cut: bool) -> String {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        std::thread::spawn(move || {
            let mut first = cut;
            for s in l.incoming() {
                let mut s = s.unwrap();
                let mut r = BufReader::new(s.try_clone().unwrap());
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                let path = line.split_whitespace().nth(1).unwrap_or("/").trim_start_matches('/').to_string();
                let mut from = 0usize;
                loop {
                    let mut h = String::new();
                    if r.read_line(&mut h).unwrap() == 0 || h.trim().is_empty() {
                        break;
                    }
                    if let Some(v) = h.to_ascii_lowercase().strip_prefix("range: bytes=") {
                        from = v.trim().trim_end_matches('-').parse().unwrap();
                    }
                }
                let Ok(data) = fs::read(dir.join(&path)) else {
                    let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    continue;
                };
                let body = &data[from..];
                let status = if from > 0 { "206 Partial Content" } else { "200 OK" };
                let _ = write!(s, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                if first && path.ends_with(".xz") {
                    first = false;
                    let _ = s.write_all(&body[..body.len() / 2]);
                    continue; // drop the connection
                }
                let _ = s.write_all(body);
            }
        });
        format!("http://{addr}")
    }

    #[test]
    fn downloads_resumes_and_writes_a_release() {
        let dir = std::env::temp_dir().join(format!("bia-release-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let rel = dir.join("release");
        fs::create_dir_all(&rel).unwrap();
        let src = crate::write::image_tests::fake_build(&dir);
        let img = dir.join("bia-x86_64.img");
        crate::image::build_image(&src, &img, &mut (Box::new(|_: &str| {}) as crate::ProgressFn)).unwrap();
        crate::write::image_tests::xz(&img, &rel.join("bia-x86_64.img.xz"));
        let sum = |p: &Path| hex(&Sha256::digest(fs::read(p).unwrap()));
        let listing = serde_json::json!({
            "version": "9.9.9", "tag": "v9.9.9",
            "os": { "x86_64": { "image": {
                "file": "bia-x86_64.img.xz",
                "size": fs::metadata(rel.join("bia-x86_64.img.xz")).unwrap().len(),
                "sha256": sum(&rel.join("bia-x86_64.img.xz")),
                "raw_size": fs::metadata(&img).unwrap().len(),
                "raw_sha256": sum(&img),
            }}}
        });
        fs::write(rel.join(MANIFEST), listing.to_string()).unwrap();

        // Only this test touches these variables.
        std::env::set_var("BIA_RELEASE_URL", serve(rel.clone(), true));
        std::env::set_var("BIA_DEPLOY_CACHE", dir.join("cache"));
        let m = manifest("latest").unwrap();
        assert_eq!(m.tag, "v9.9.9");
        assert_eq!(missing_bytes(&m, "x86_64"), m.os["x86_64"].image.size);
        let mut log = Vec::new();
        let got = fetch(&m, "x86_64", &mut |s| log.push(s.to_string())).unwrap();
        assert!(log.iter().any(|l| l.contains("retrying")), "the dropped connection was resumed: {log:?}");
        assert_eq!(missing_bytes(&m, "x86_64"), 0);
        assert_eq!(fs::read(&got.path).unwrap(), fs::read(rel.join("bia-x86_64.img.xz")).unwrap());
        assert!(fetch(&m, "x86_64", &mut |_| {}).is_ok(), "cached");
        assert!(fetch(&m, "arm64", &mut |_| {}).is_err());

        let stick = dir.join("stick.img");
        crate::write_stick(
            crate::FlashOptions {
                disk: stick.clone(),
                image: got.path,
                image_sha256: Some(got.raw_sha256),
                size_bytes: 400 << 20,
                fmm: None,
                create_image: true,
                model: None,
                hf_token: None,
                lang: None,
            },
            Box::new(|_| {}),
        )
        .unwrap();
        let d = fs::read(&stick).unwrap();
        assert_eq!(&d[512..520], b"EFI PART");
        std::env::remove_var("BIA_RELEASE_URL");
        std::env::remove_var("BIA_DEPLOY_CACHE");
        fs::remove_dir_all(&dir).ok();
    }
}
