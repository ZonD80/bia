//! The OCR models: downloaded on first use into /home/bia/files/ocr/models (carried by
//! fold), size and sha256 checked.
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

const BASE: &str = "https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.10.0/onnx/PP-OCRv5";

pub struct Model {
    pub name: &'static str,
    path: &'static str,
    size: u64,
    sha256: &'static str,
}

pub const DET: &str = "det";

const MODELS: &[Model] = &[
    Model { name: DET, path: "det/ch_PP-OCRv5_det_mobile.onnx", size: 4819576, sha256: "4d97c44a20d30a81aad087d6a396b08f786c4635742afc391f6621f5c6ae78ae" },
    Model { name: "en", path: "rec/en_PP-OCRv5_rec_mobile.onnx", size: 7872351, sha256: "c3461add59bb4323ecba96a492ab75e06dda42467c9e3d0c18db5d1d21924be8" },
    Model { name: "latin", path: "rec/latin_PP-OCRv5_rec_mobile.onnx", size: 7904513, sha256: "b20bd37c168a570f583afbc8cd7925603890efbcdc000a59e22c269d160b5f5a" },
    Model { name: "eslav", path: "rec/eslav_PP-OCRv5_rec_mobile.onnx", size: 7911802, sha256: "08705d6721849b1347d26187f15a5e362c431963a2a62bfff4feac578c489aab" },
    Model { name: "cyrillic", path: "rec/cyrillic_PP-OCRv5_rec_mobile.onnx", size: 8074092, sha256: "90f761b4bfcce0c8c561c0cb5c887b0971d3ec01c32164bdf7374a35b0982711" },
    Model { name: "ch", path: "rec/ch_PP-OCRv5_rec_mobile.onnx", size: 16631306, sha256: "5825fc7ebf84ae7a412be049820b4d86d77620f204a041697b0494669b1742c5" },
    Model { name: "korean", path: "rec/korean_PP-OCRv5_rec_mobile.onnx", size: 13488748, sha256: "cd6e2ea50f6943ca7271eb8c56a877a5a90720b7047fe9c41a2e541a25773c9b" },
    Model { name: "arabic", path: "rec/arabic_PP-OCRv5_rec_mobile.onnx", size: 8023828, sha256: "c1192e632d0baa9146ae5b756a0e635e3dc63c1733737ebfd1629e87144e9295" },
    Model { name: "devanagari", path: "rec/devanagari_PP-OCRv5_rec_mobile.onnx", size: 7940361, sha256: "d6f0a906580e3fa6b324a318718f1f31f268b6ea8ef985f91c2012a37f52c91e" },
    Model { name: "th", path: "rec/th_PP-OCRv5_rec_mobile.onnx", size: 7915294, sha256: "de541dd83161c241ff426f7ecfd602a0ba77d686cf3ab9a6c255ea82fd08006e" },
    Model { name: "el", path: "rec/el_PP-OCRv5_rec_mobile.onnx", size: 7832350, sha256: "b4368bccd557123c702b7549fee6cd1e94b581337d1c9b65310f109131542b7f" },
    Model { name: "ta", path: "rec/ta_PP-OCRv5_rec_mobile.onnx", size: 7909926, sha256: "a42448808b7dea87597336f12438935f40353f1949e8360acd9e06b4da21bfe1" },
    Model { name: "te", path: "rec/te_PP-OCRv5_rec_mobile.onnx", size: 7923102, sha256: "a3690451b50028a09a3316a1274f7c05728151ea3f8fd392696397a7fefcbd92" },
];

pub fn dir() -> PathBuf {
    if let Ok(d) = std::env::var("BIA_OCR_MODELS") {
        return PathBuf::from(d);
    }
    PathBuf::from(std::env::var("BIA_FILES").unwrap_or_else(|_| "/home/bia/files".into())).join("ocr/models")
}

pub fn path(name: &str) -> Option<PathBuf> {
    MODELS.iter().find(|m| m.name == name).map(|m| dir().join(file_name(m)))
}

fn file_name(m: &Model) -> &'static str {
    m.path.rsplit('/').next().unwrap_or(m.path)
}

/// Downloads the detector and the recognizers `scripts` that are missing. Returns the
/// recognizers ready to use and a note on any that could not be downloaded; Err when
/// the detector is missing.
pub fn ensure(scripts: &[&'static str]) -> Result<(Vec<&'static str>, Option<String>), String> {
    let mut failed = Vec::new();
    let mut get = |name: &str| -> bool {
        let Some(m) = MODELS.iter().find(|m| m.name == name) else { return false };
        let dest = dir().join(file_name(m));
        if dest.is_file() {
            return true;
        }
        match download(m, &dest) {
            Ok(()) => true,
            Err(e) => {
                failed.push(e);
                false
            }
        }
    };
    if !get(DET) {
        return Err(format!("OCR is not available: {}", failed.join("; ")));
    }
    let ready: Vec<&'static str> = scripts.iter().copied().filter(|s| get(s)).collect();
    let note = (!failed.is_empty()).then(|| format!("OCR models not downloaded: {}", failed.join("; ")));
    if ready.is_empty() {
        return Err(note.unwrap_or_else(|| "no OCR model for these languages".into()));
    }
    Ok((ready, note))
}

fn download(m: &Model, dest: &Path) -> Result<(), String> {
    let dir = dest.parent().ok_or("bad model path")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_body(Some(Duration::from_secs(120)))
        .user_agent("bia-ocr")
        .build()
        .into();
    let url = format!("{BASE}/{}", m.path);
    let resp = agent.get(&url).call().map_err(|e| format!("{}: {e}", m.name))?;
    let mut body = resp.into_body().into_reader();
    let part = dest.with_extension("partial");
    let mut f = std::fs::File::create(&part).map_err(|e| format!("create {}: {e}", part.display()))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    let mut got = 0u64;
    loop {
        let n = body.read(&mut buf).map_err(|e| format!("{}: {e}", m.name))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        f.write_all(&buf[..n]).map_err(|e| format!("write {}: {e}", part.display()))?;
        got += n as u64;
    }
    drop(f);
    let hex: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if got != m.size || hex != m.sha256 {
        let _ = std::fs::remove_file(&part);
        return Err(format!("{}: download is damaged ({got} bytes, sha256 {hex})", m.name));
    }
    std::fs::rename(&part, dest).map_err(|e| format!("rename {}: {e}", dest.display()))?;
    // root writes these under /home/bia; backups run as bia
    for p in [Some(dest), dest.parent(), dest.parent().and_then(Path::parent)].into_iter().flatten() {
        give_to_bia(p);
    }
    Ok(())
}

fn give_to_bia(path: &Path) {
    if unsafe { libc::geteuid() } != 0 {
        return;
    }
    let pw = unsafe { libc::getpwnam(c"bia".as_ptr()) };
    if !pw.is_null() {
        let (uid, gid) = unsafe { ((*pw).pw_uid, (*pw).pw_gid) };
        let _ = std::os::unix::fs::chown(path, Some(uid), Some(gid));
    }
}
