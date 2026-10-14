//! The bia OS from GitHub releases: `bia-os.json` lists, per architecture, the root
//! image as zstd parts (1 GiB of raw image each, compressed separately) and a boot
//! bundle (ESP tree + kernel, initramfs, rescue image). The writer downloads them with
//! resume, checks every hash, and rebuilds `root.img` in a cache so the next stick
//! needs no download.
//!
//! Cache: `<cache>/bia-flash/os/<tag>/<arch>/{root.img, boot/}` (`BIA_FLASH_CACHE`
//! overrides `<cache>/bia-flash`).

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
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
    pub root: RootImage,
    pub boot: Asset,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RootImage {
    /// Raw image size in bytes.
    pub size: u64,
    pub parts: Vec<RootPart>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RootPart {
    pub file: String,
    /// Where this part's raw bytes go in root.img, and how many there are.
    pub offset: u64,
    pub size: u64,
    /// sha256 of the raw bytes.
    pub sha256: String,
    /// Compressed (downloaded) size and sha256.
    pub zsize: u64,
    pub zsha256: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Asset {
    pub file: String,
    pub size: u64,
    pub sha256: String,
}

/// What the writer needs: the root image and the tree for both `--esp` and `--kern`.
#[derive(Debug, Clone)]
pub struct OsFiles {
    pub tag: String,
    pub root_img: PathBuf,
    pub boot_dir: PathBuf,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(20)))
        .user_agent("bia-flash")
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
    if let Some(p) = std::env::var_os("BIA_FLASH_CACHE") {
        return PathBuf::from(p);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "macos") {
        if let Some(h) = home {
            return h.join("Library/Caches/bia-flash");
        }
    } else if cfg!(windows) {
        if let Some(p) = std::env::var_os("LOCALAPPDATA") {
            return PathBuf::from(p).join("bia-flash").join("cache");
        }
    } else if let Some(p) = std::env::var_os("XDG_CACHE_HOME") {
        return PathBuf::from(p).join("bia-flash");
    } else if let Some(h) = home {
        return h.join(".cache/bia-flash");
    }
    std::env::temp_dir().join("bia-flash")
}

/// Bytes still to download for `arch` of this release (0 = all cached).
pub fn missing_bytes(m: &Manifest, arch: &str) -> u64 {
    let Some(e) = m.os.get(arch) else { return 0 };
    let dir = cache_root().join("os").join(&m.tag).join(arch);
    if dir.join(".complete").is_file() {
        return 0;
    }
    let parts: u64 = e
        .root
        .parts
        .iter()
        .enumerate()
        .filter(|(i, _)| !dir.join(format!(".part-{i}.ok")).is_file())
        .map(|(_, p)| p.zsize)
        .sum();
    let boot = if dir.join(".boot.ok").is_file() { 0 } else { e.boot.size };
    parts + boot
}

/// Download (or reuse) the OS for `arch` from release `m`.
pub fn fetch(m: &Manifest, arch: &str, progress: &mut dyn FnMut(&str)) -> Result<OsFiles> {
    let entry = m
        .os
        .get(arch)
        .with_context(|| format!("release {} has no {arch} OS (has: {})", m.tag, arches(m).join(", ")))?;
    let dir = cache_root().join("os").join(&m.tag).join(arch);
    fs::create_dir_all(&dir).with_context(|| format!("mkdir {}", dir.display()))?;
    let files = OsFiles { tag: m.tag.clone(), root_img: dir.join("root.img"), boot_dir: dir.join("boot") };
    if dir.join(".complete").is_file() && files.root_img.is_file() && files.boot_dir.is_dir() {
        progress(&format!("bia OS {} ({arch}) is already downloaded", m.tag));
        return Ok(files);
    }
    let base = base_url(&m.tag);

    // root.img: each part decompresses into its own range of the image.
    let mut root = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&files.root_img)
        .context("create root.img")?;
    root.set_len(entry.root.size).context("size root.img")?;
    let n = entry.root.parts.len();
    for (i, part) in entry.root.parts.iter().enumerate() {
        let ok = dir.join(format!(".part-{i}.ok"));
        if ok.is_file() {
            continue;
        }
        if part.offset + part.size > entry.root.size {
            bail!("{}: part {i} lies outside the image", MANIFEST);
        }
        let what = format!("system part {}/{n}", i + 1);
        let z = dir.join(format!("{}.download", part.file));
        download(&format!("{base}/{}", part.file), &z, part.zsize, &part.zsha256, &what, progress)?;
        progress(&format!("unpacking {what}"));
        root.seek(SeekFrom::Start(part.offset))?;
        let mut dec = ruzstd::decoding::StreamingDecoder::new(File::open(&z)?).context("zstd")?;
        let mut hash = Sha256::new();
        let mut buf = vec![0u8; 4 << 20];
        let mut got = 0u64;
        loop {
            let k = dec.read(&mut buf).with_context(|| format!("decompress {}", part.file))?;
            if k == 0 {
                break;
            }
            if got + k as u64 > part.size {
                bail!("{}: more data than the manifest says", part.file);
            }
            root.write_all(&buf[..k])?;
            hash.update(&buf[..k]);
            got += k as u64;
        }
        if got != part.size || !hex(&hash.finalize()).eq_ignore_ascii_case(&part.sha256) {
            bail!("{}: unpacked data does not match the release (got {got} bytes)", part.file);
        }
        root.sync_all().ok();
        File::create(&ok)?;
        let _ = fs::remove_file(&z);
    }
    drop(root);

    // boot bundle: ESP tree + kernel files.
    if !dir.join(".boot.ok").is_file() || !files.boot_dir.is_dir() {
        let z = dir.join(format!("{}.download", entry.boot.file));
        download(&format!("{base}/{}", entry.boot.file), &z, entry.boot.size, &entry.boot.sha256, "boot files", progress)?;
        let _ = fs::remove_dir_all(&files.boot_dir);
        fs::create_dir_all(&files.boot_dir)?;
        let dec = ruzstd::decoding::StreamingDecoder::new(File::open(&z)?).context("zstd")?;
        // unpack() keeps every entry inside boot_dir (no absolute paths or "..").
        tar::Archive::new(dec).unpack(&files.boot_dir).context("unpack boot files")?;
        for need in ["EFI", "vmlinuz-lts", "initramfs-lts"] {
            if !files.boot_dir.join(need).exists() {
                bail!("boot files have no {need}");
            }
        }
        File::create(dir.join(".boot.ok"))?;
        let _ = fs::remove_file(&z);
    }
    File::create(dir.join(".complete"))?;
    progress(&format!("bia OS {} ({arch}) ready in {}", m.tag, dir.display()));
    Ok(files)
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
