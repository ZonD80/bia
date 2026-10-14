//! `local-files.json`: what a local build (`make build` / `make rootfs`) produced, with absolute
//! paths on the build machine, so developers pick one file instead of three paths.
//! Written by `tools/local-files.sh` as `dist/<arch>/local-files.json`.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

pub const LOCAL_FILES_NAME: &str = "local-files.json";

#[derive(Debug, Clone, Deserialize)]
pub struct LocalFiles {
    /// `x86_64` or `arm64`.
    pub arch: String,
    /// BIAROOT ext4 image.
    pub root: PathBuf,
    /// Tree with `EFI/` (usually `source/flash`).
    pub esp: PathBuf,
    /// Directory with `vmlinuz-lts` and `initramfs-lts` (and `rescue.cpio.gz`).
    pub kern: PathBuf,
    /// When the build wrote the file (informational).
    #[serde(default)]
    pub built: String,
}

impl LocalFiles {
    /// Read and check that every path it names exists.
    pub fn load(path: &Path) -> Result<Self> {
        let data = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
        let l: LocalFiles =
            serde_json::from_slice(&data).with_context(|| format!("parse {}", path.display()))?;
        if l.arch != "x86_64" && l.arch != "arm64" {
            bail!("{}: unknown arch {:?}", path.display(), l.arch);
        }
        let need = [
            (l.root.clone(), false),
            (l.esp.join("EFI"), true),
            (l.kern.join("vmlinuz-lts"), false),
            (l.kern.join("initramfs-lts"), false),
        ];
        for (p, dir) in need {
            let ok = if dir { p.is_dir() } else { p.is_file() };
            if !ok {
                bail!("{}: missing {} (rebuild?)", path.display(), p.display());
            }
        }
        Ok(l)
    }

    /// `dist/<arch>/local-files.json` under the working directory, newest first.
    pub fn find() -> Option<PathBuf> {
        ["x86_64", "arm64"]
            .iter()
            .map(|a| PathBuf::from("dist").join(a).join(LOCAL_FILES_NAME))
            .filter(|p| p.is_file())
            .max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
            .and_then(|p| p.canonicalize().ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_and_checks_paths() {
        let d = std::env::temp_dir().join(format!("bia-local-files-{}", std::process::id()));
        let kern = d.join("dist");
        std::fs::create_dir_all(kern.join("x")).unwrap();
        std::fs::create_dir_all(d.join("flash/EFI")).unwrap();
        for f in ["root.img", "vmlinuz-lts", "initramfs-lts"] {
            std::fs::write(kern.join(f), b"x").unwrap();
        }
        let json = d.join(LOCAL_FILES_NAME);
        let write = |root: &Path| {
            let v = serde_json::json!({
                "arch": "x86_64",
                "root": root,
                "esp": d.join("flash"),
                "kern": kern,
            });
            std::fs::write(&json, v.to_string()).unwrap();
        };
        write(&kern.join("root.img"));
        let l = LocalFiles::load(&json).unwrap();
        assert_eq!(l.arch, "x86_64");
        assert_eq!(l.kern, kern);
        write(&kern.join("gone.img"));
        assert!(format!("{:#}", LocalFiles::load(&json).unwrap_err()).contains("gone.img"));
        std::fs::remove_dir_all(&d).ok();
    }
}
