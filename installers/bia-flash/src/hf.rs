//! Hugging Face Hub: search GGUF repos, list their GGUF files, stream one onto the stick.
//!
//! Split GGUFs (`name-00001-of-00003.gguf`) are one choice: every part is written, and the
//! agent loads the `-00001-of-` part.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::time::Duration;

const HUB: &str = "https://huggingface.co";

#[derive(Debug, Clone)]
pub struct HfRepo {
    pub id: String,
    pub downloads: u64,
    pub likes: u64,
}

#[derive(Debug, Clone)]
pub struct HfFile {
    /// Path inside the repo (may contain folders).
    pub path: String,
    pub size: u64,
    /// LFS sha256 (hex); empty when the Hub does not report one.
    pub sha256: String,
}

impl HfFile {
    /// File name on the stick (`/home/bia/model/<name>`).
    pub fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }
}

/// One selectable model: a single GGUF or all parts of a split one.
#[derive(Debug, Clone)]
pub struct HfModel {
    pub repo: String,
    pub files: Vec<HfFile>,
}

impl HfModel {
    pub fn label(&self) -> String {
        let first = self.files[0].name();
        match self.files.len() {
            1 => first.to_string(),
            n => format!("{first} (+{} parts)", n - 1),
        }
    }

    pub fn size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(20)))
        .user_agent("bia-flash")
        .build()
        .into()
}

fn get_json<T: for<'de> Deserialize<'de>>(url: &str, token: Option<&str>) -> Result<T> {
    let mut req = agent().get(url);
    if let Some(t) = token.filter(|t| !t.is_empty()) {
        req = req.header("Authorization", &format!("Bearer {t}"));
    }
    let resp = req.call().with_context(|| format!("GET {url}"))?;
    serde_json::from_reader(resp.into_body().into_reader()).with_context(|| format!("parse {url}"))
}

/// Percent-encode a query value or path segment.
fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn enc_path(p: &str) -> String {
    p.split('/').map(enc).collect::<Vec<_>>().join("/")
}

/// GGUF repos matching `query`, most downloaded first.
pub fn search(query: &str, limit: usize, token: Option<&str>) -> Result<Vec<HfRepo>> {
    #[derive(Deserialize)]
    struct Item {
        id: String,
        #[serde(default)]
        downloads: u64,
        #[serde(default)]
        likes: u64,
    }
    let url = format!(
        "{HUB}/api/models?search={}&filter=gguf&sort=downloads&direction=-1&limit={limit}",
        enc(query.trim())
    );
    let items: Vec<Item> = get_json(&url, token)?;
    Ok(items
        .into_iter()
        .map(|i| HfRepo {
            id: i.id,
            downloads: i.downloads,
            likes: i.likes,
        })
        .collect())
}

/// GGUF choices in `repo` (split parts grouped), smallest first. Vision projectors are left out.
pub fn models(repo: &str, token: Option<&str>) -> Result<Vec<HfModel>> {
    #[derive(Deserialize)]
    struct Lfs {
        #[serde(default)]
        oid: String,
    }
    #[derive(Deserialize)]
    struct Entry {
        #[serde(rename = "type")]
        kind: String,
        path: String,
        #[serde(default)]
        size: u64,
        lfs: Option<Lfs>,
    }
    let url = format!("{HUB}/api/models/{}/tree/main?recursive=true", enc_path(repo));
    let entries: Vec<Entry> = get_json(&url, token)?;
    let mut files: Vec<HfFile> = entries
        .into_iter()
        .filter(|e| e.kind == "file" && e.path.to_lowercase().ends_with(".gguf"))
        .filter(|e| {
            let name = e.path.rsplit('/').next().unwrap_or(&e.path).to_lowercase();
            !name.starts_with("mmproj")
        })
        .map(|e| HfFile {
            path: e.path,
            size: e.size,
            sha256: e.lfs.map(|l| l.oid).unwrap_or_default(),
        })
        .collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));

    let mut out: Vec<HfModel> = Vec::new();
    for f in files {
        match split_key(&f.path) {
            Some(key) => match out
                .iter_mut()
                .find(|m| split_key(&m.files[0].path).as_deref() == Some(key.as_str()))
            {
                Some(m) => m.files.push(f),
                None => out.push(HfModel {
                    repo: repo.to_string(),
                    files: vec![f],
                }),
            },
            None => out.push(HfModel {
                repo: repo.to_string(),
                files: vec![f],
            }),
        }
    }
    out.sort_by_key(|m| m.size());
    Ok(out)
}

/// `dir/name-00002-of-00003.gguf` -> `dir/name-of-00003`; None for unsplit files.
fn split_key(path: &str) -> Option<String> {
    let stem = path.strip_suffix(".gguf").or_else(|| path.strip_suffix(".GGUF"))?;
    let (head, total) = stem.rsplit_once("-of-")?;
    let (base, part) = head.rsplit_once('-')?;
    if part.len() == 5 && part.bytes().all(|b| b.is_ascii_digit()) && total.bytes().all(|b| b.is_ascii_digit()) {
        Some(format!("{base}-of-{total}"))
    } else {
        None
    }
}

/// Pick a choice from `models(repo)` by file name/path (any part of a split model), or
/// automatically: the only one, else the first Q4_K_M.
pub fn pick(repo: &str, file: Option<&str>, token: Option<&str>) -> Result<HfModel> {
    let all = models(repo, token)?;
    if all.is_empty() {
        bail!("{repo} has no GGUF files");
    }
    let list = || {
        all.iter()
            .map(|m| format!("  {}  ({} MiB)", m.files[0].path, m.size() >> 20))
            .collect::<Vec<_>>()
            .join("\n")
    };
    if let Some(want) = file {
        return all
            .iter()
            .find(|m| m.files.iter().any(|f| f.path == want || f.name() == want))
            .cloned()
            .with_context(|| format!("{want} not in {repo}; GGUF files:\n{}", list()));
    }
    if all.len() == 1 {
        return Ok(all[0].clone());
    }
    if let Some(m) = all.iter().find(|m| m.files[0].path.to_uppercase().contains("Q4_K_M")) {
        return Ok(m.clone());
    }
    bail!("{repo} has several GGUF files; pick one with repo:file\n{}", list())
}

/// Stream `file` from the Hub into `sink` (called with each chunk in order), resuming with
/// HTTP ranges after dropped connections. Checks size and the LFS sha256.
pub fn download(
    repo: &str,
    file: &HfFile,
    token: Option<&str>,
    sink: &mut dyn FnMut(&[u8]) -> Result<()>,
) -> Result<()> {
    let url = format!("{HUB}/{}/resolve/main/{}", enc_path(repo), enc_path(&file.path));
    let mut hash = Sha256::new();
    let mut got = 0u64;
    let mut buf = vec![0u8; 1 << 20];
    let mut failures = 0;
    while got < file.size {
        let mut req = agent().get(&url);
        if let Some(t) = token.filter(|t| !t.is_empty()) {
            req = req.header("Authorization", &format!("Bearer {t}"));
        }
        if got > 0 {
            req = req.header("Range", &format!("bytes={got}-"));
        }
        // Network trouble is retried from where it stopped; HTTP 4xx and sink
        // (stick) errors are final.
        let transient = match req.call() {
            Err(ureq::Error::StatusCode(code)) if (400..500).contains(&code) => {
                if code == 401 || code == 403 {
                    bail!("{}: HTTP {code} (gated or private repo: accept its license on huggingface.co and pass a token)", file.path);
                }
                bail!("{}: HTTP {code}", file.path);
            }
            Err(e) => anyhow::Error::from(e),
            Ok(resp) => {
                if got > 0 && resp.status().as_u16() != 206 {
                    bail!("{}: server ignored the resume range", file.path);
                }
                let mut body = resp.into_body().into_reader();
                loop {
                    let n = match body.read(&mut buf) {
                        Ok(0) => break anyhow::anyhow!("connection closed early"),
                        Ok(n) => n.min((file.size - got) as usize),
                        Err(e) => break e.into(),
                    };
                    sink(&buf[..n])?;
                    hash.update(&buf[..n]);
                    got += n as u64;
                    failures = 0;
                    if got == file.size {
                        break anyhow::anyhow!("done");
                    }
                }
            }
        };
        if got == file.size {
            break;
        }
        failures += 1;
        if failures > 8 {
            return Err(transient).with_context(|| format!("download {}", file.path));
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    if !file.sha256.is_empty() {
        let sum = hex(&hash.finalize());
        if !sum.eq_ignore_ascii_case(&file.sha256) {
            bail!("{}: sha256 {sum}, Hub says {}", file.path, file.sha256);
        }
    }
    Ok(())
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_parts_group() {
        assert_eq!(
            split_key("Q8/m-Q8_0-00002-of-00003.gguf").as_deref(),
            Some("Q8/m-Q8_0-of-00003")
        );
        assert_eq!(split_key("m-Q4_K_M.gguf"), None);
        assert_eq!(enc("qwen 3/4b"), "qwen%203%2F4b");
        assert_eq!(enc_path("a b/c.gguf"), "a%20b/c.gguf");
    }
}
