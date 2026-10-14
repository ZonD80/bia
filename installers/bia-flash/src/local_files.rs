//! `local-files.json`: the bia disk image a local build (`make build` / `make rootfs`)
//! produced, with its absolute path on the build machine.
//! Written by `tools/local-files.sh` as `dist/<arch>/local-files.json`.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

pub const LOCAL_FILES_NAME: &str = "local-files.json";

#[derive(Debug, Clone, Deserialize)]
pub struct LocalFiles {
    /// `x86_64` or `arm64`.
    pub arch: String,
    /// bia disk image (`dist/<arch>/bia-<arch>.img`, from `bia-flash image`).
    pub image: PathBuf,
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
        if !l.image.is_file() {
            bail!("{}: missing {} (rebuild?)", path.display(), l.image.display());
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
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("bia-x86_64.img"), b"x").unwrap();
        let json = d.join(LOCAL_FILES_NAME);
        let write = |image: &Path| {
            let v = serde_json::json!({ "arch": "x86_64", "image": image });
            std::fs::write(&json, v.to_string()).unwrap();
        };
        write(&d.join("bia-x86_64.img"));
        let l = LocalFiles::load(&json).unwrap();
        assert_eq!(l.arch, "x86_64");
        assert_eq!(l.image, d.join("bia-x86_64.img"));
        write(&d.join("gone.img"));
        assert!(format!("{:#}", LocalFiles::load(&json).unwrap_err()).contains("gone.img"));
        std::fs::remove_dir_all(&d).ok();
    }
}
