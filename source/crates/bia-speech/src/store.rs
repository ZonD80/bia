//! Downloads into /home/bia/files/speech/models: stream with progress, check size and
//! sha256, unpack .tar.bz2, and find the model files inside.

use crate::learn::Report;
use crate::registry::{Asr, Voice};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

pub struct Want<'a> {
    pub url: &'a str,
    pub size: u64,
    pub sha256: &'a str,
    pub label: &'a str,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_body(Some(Duration::from_secs(60)))
        .user_agent("bia-lang")
        .build()
        .into()
}

/// Download to `dest` (verified), reporting progress in `pct_from..pct_to`.
pub fn download(ctx: &dyn Report, w: &Want, dest: &Path, pct_from: u32, pct_to: u32) -> Result<(), String> {
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    }
    let part = dest.with_extension("partial");
    let resp = agent().get(w.url).call().map_err(|e| format!("download {}: {e}", w.label))?;
    let mut body = resp.into_body().into_reader();
    let mut f = std::fs::File::create(&part).map_err(|e| format!("create {}: {e}", part.display()))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    let (mut got, mut shown) = (0u64, u32::MAX);
    loop {
        if ctx.cancelled() {
            let _ = std::fs::remove_file(&part);
            return Err("cancelled".into());
        }
        let n = body.read(&mut buf).map_err(|e| format!("download {}: {e}", w.label))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        f.write_all(&buf[..n]).map_err(|e| format!("write {}: {e}", part.display()))?;
        got += n as u64;
        if w.size > 0 {
            let frac = (got as f64 / w.size as f64).min(1.0);
            let pct = pct_from + ((pct_to - pct_from) as f64 * frac) as u32;
            if pct != shown {
                shown = pct;
                ctx.progress(pct, &format!("{}: {} / {}", w.label, crate::catalog::mb(got), crate::catalog::mb(w.size)));
            }
        }
    }
    drop(f);
    if w.size > 0 && got != w.size {
        let _ = std::fs::remove_file(&part);
        return Err(format!("{}: got {got} bytes, expected {}", w.label, w.size));
    }
    let hex: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if !w.sha256.is_empty() && hex != w.sha256 {
        let _ = std::fs::remove_file(&part);
        return Err(format!("{}: sha256 mismatch ({hex}); not installed", w.label));
    }
    std::fs::rename(&part, dest).map_err(|e| format!("rename {}: {e}", dest.display()))
}

/// Unpack a .tar.bz2 into `dest` (replacing it); the archive's single top folder
/// becomes `dest`.
pub fn unpack(archive: &Path, dest: &Path) -> Result<(), String> {
    let parent = dest.parent().ok_or("bad destination")?;
    let tmp = parent.join(format!(".unpack-{}", crate::paths::stamp()));
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let out = Command::new("tar")
        .arg("-xjf")
        .arg(archive)
        .arg("-C")
        .arg(&tmp)
        .output()
        .map_err(|e| format!("tar: {e}"))?;
    if !out.status.success() {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(format!(
            "unpack {}: {} (is bzip2 installed?)",
            archive.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let entries: Vec<PathBuf> = std::fs::read_dir(&tmp)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .collect();
    let top = if entries.len() == 1 && entries[0].is_dir() { entries[0].clone() } else { tmp.clone() };
    let _ = std::fs::remove_dir_all(dest);
    std::fs::rename(&top, dest).map_err(|e| format!("rename {}: {e}", dest.display()))?;
    let _ = std::fs::remove_dir_all(&tmp);
    Ok(())
}

fn files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir).map(|r| r.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    v.sort();
    v
}

fn name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// Find the model files of an unpacked voice.
pub fn resolve_voice(dir: &Path, kind: &str, langs: &[String], locale: &str) -> Result<Voice, String> {
    let all = files(dir);
    let onnx: Vec<&PathBuf> = all.iter().filter(|p| name(p).ends_with(".onnx")).collect();
    let model = onnx
        .iter()
        .find(|p| name(p).contains("int8"))
        .or_else(|| onnx.first())
        .ok_or_else(|| format!("no .onnx model in {}", dir.display()))?;
    let tokens = dir.join("tokens.txt");
    if !tokens.exists() {
        return Err(format!("no tokens.txt in {}", dir.display()));
    }
    let opt = |p: PathBuf| if p.exists() { s(&p) } else { String::new() };
    let lexicons: Vec<String> = all
        .iter()
        .filter(|p| {
            let n = name(p);
            n.starts_with("lexicon") && n.ends_with(".txt")
        })
        .map(|p| s(p))
        .collect();
    let mut v = Voice {
        kind: kind.to_string(),
        langs: langs.to_vec(),
        locale: locale.to_string(),
        dir: s(dir),
        model: s(model),
        tokens: s(&tokens),
        data_dir: opt(dir.join("espeak-ng-data")),
        dict_dir: opt(dir.join("dict")),
        lexicon: lexicons.join(","),
        ..Default::default()
    };
    if kind == "kokoro" {
        v.voices = opt(dir.join("voices.bin"));
        if v.voices.is_empty() {
            return Err(format!("no voices.bin in {}", dir.display()));
        }
    }
    Ok(v)
}

/// Find the int8 whisper files of an unpacked model; drop the float32 copies.
pub fn resolve_whisper(dir: &Path) -> Result<Asr, String> {
    let all = files(dir);
    let pick = |suffix: &str| {
        all.iter()
            .find(|p| name(p).ends_with(suffix))
            .map(|p| s(p))
            .ok_or_else(|| format!("no *{suffix} in {}", dir.display()))
    };
    let enc = pick("-encoder.int8.onnx")?;
    let dec = pick("-decoder.int8.onnx")?;
    let tok = pick("-tokens.txt")?;
    for p in &all {
        let n = name(p);
        if (n.ends_with("-encoder.onnx") || n.ends_with("-decoder.onnx")) && !n.contains("int8") {
            let _ = std::fs::remove_file(p);
        }
    }
    Ok(Asr {
        kind: "whisper".into(),
        langs: vec![],
        dir: s(dir),
        encoder: enc,
        decoder: dec,
        tokens: tok,
        bundled: false,
        ..Default::default()
    })
}

/// Find the int8 encoder, decoder and joiner of an unpacked NeMo transducer (Parakeet).
pub fn resolve_transducer(dir: &Path, langs: &[String]) -> Result<Asr, String> {
    let all = files(dir);
    let pick = |want: &str| {
        all.iter()
            .find(|p| name(p) == want || name(p).ends_with(&format!("-{want}")))
            .map(|p| s(p))
            .ok_or_else(|| format!("no {want} in {}", dir.display()))
    };
    Ok(Asr {
        kind: "nemo_transducer".into(),
        langs: langs.to_vec(),
        dir: s(dir),
        encoder: pick("encoder.int8.onnx")?,
        decoder: pick("decoder.int8.onnx")?,
        joiner: pick("joiner.int8.onnx")?,
        tokens: pick("tokens.txt")?,
        bundled: false,
    })
}

pub fn dir_size(p: &Path) -> u64 {
    let Ok(md) = std::fs::symlink_metadata(p) else { return 0 };
    if md.is_file() {
        return md.len();
    }
    if !md.is_dir() {
        return 0;
    }
    files(p).iter().map(|c| dir_size(c)).sum()
}
