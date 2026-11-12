//! This bia's own web app icon (web_icon): the picture on a phone's home screen, in the browser
//! tab, in notifications and as bia's face in the chat. Without one the built-in potato is served.
//!
//! One picture in, three PNGs out under files/state/web/icon/: 512 and 192 (web app manifest,
//! tab, notifications, chat) and 180 for iOS, which blackens transparency, so that one is
//! flattened onto a background colour. icon.json holds `rev`, the start of the 512's sha256:
//! pages add it to the icon addresses (?v=…) so browsers take the new picture.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use bia_mcp::{arg_str_opt, Ctx};

const NAMES: [(&str, u32); 3] = [("icon-512.png", 512), ("icon-192.png", 192), ("apple-touch-icon.png", 180)];
/// Smaller pictures look blurred on a home screen.
const MIN_SIDE: u32 = 192;
const MAX_BYTES: u64 = 20 * 1024 * 1024;

fn dir(files: &Path) -> PathBuf {
    files.join("state").join("web").join("icon")
}

/// "" while the built-in icon is used.
pub fn rev_in(files: &Path) -> String {
    let v: Option<Value> = bia_mcp::statefile::read(&dir(files).join("icon.json"));
    let rev = v.and_then(|v| v["rev"].as_str().map(str::to_string)).unwrap_or_default();
    // a half-written set (or files removed by hand) falls back to the built-in icon
    if rev.is_empty() || NAMES.iter().any(|(n, _)| !dir(files).join(n).is_file()) {
        return String::new();
    }
    rev
}

pub fn rev() -> String {
    rev_in(&crate::files_dir())
}

/// bia's own PNG for one of the icon addresses, if it chose a picture.
pub fn png(name: &str) -> Option<Vec<u8>> {
    let files = crate::files_dir();
    if !NAMES.iter().any(|(n, _)| *n == name) || rev_in(&files).is_empty() {
        return None;
    }
    std::fs::read(dir(&files).join(name)).ok()
}

fn colour(s: &str) -> Result<[u8; 3], String> {
    let h = s.trim().trim_start_matches('#');
    let bad = || format!("background: a colour like #ffffff, not {s:?}");
    if h.len() != 6 || !h.is_ascii() {
        return Err(bad());
    }
    let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).map_err(|_| bad());
    Ok([p(0)?, p(2)?, p(4)?])
}

fn encode(img: &image::RgbaImage) -> Result<Vec<u8>, String> {
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img.clone())
        .write_to(&mut out, image::ImageFormat::Png)
        .map_err(|e| format!("PNG: {e}"))?;
    Ok(out.into_inner())
}

/// Make the three PNGs from the picture at `src` and keep them. Returns (rev, notes for bia).
pub fn set_in(files: &Path, src: &Path, background: [u8; 3]) -> Result<(String, Vec<String>), String> {
    let meta = std::fs::metadata(src).map_err(|e| format!("{}: {e}", src.display()))?;
    if meta.len() > MAX_BYTES {
        return Err(format!("{}: larger than 20 MB", src.display()));
    }
    let bytes = std::fs::read(src).map_err(|e| format!("{}: {e}", src.display()))?;
    let mut reader = image::ImageReader::new(std::io::Cursor::new(&bytes))
        .with_guessed_format()
        .map_err(|e| format!("{}: {e}", src.display()))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    reader.limits(limits);
    let img = reader
        .decode()
        .map_err(|e| format!("{}: not a PNG, JPEG or WebP picture this can read ({e}). Convert it first (an SVG too: render it to a 512x512 PNG).", src.display()))?
        .to_rgba8();
    let (w, h) = img.dimensions();
    let side = w.min(h);
    if side < MIN_SIDE {
        return Err(format!("the picture is {w}x{h}: too small, it needs at least {MIN_SIDE}x{MIN_SIDE} (512x512 or more looks best)"));
    }
    let mut notes = Vec::new();
    if w != h {
        notes.push(format!("The picture was {w}x{h}, so the middle {side}x{side} square was taken."));
    }
    if side < 512 {
        notes.push(format!("It is only {side} pixels wide: a 512x512 or larger one would look sharper on a home screen."));
    }
    let square = image::imageops::crop_imm(&img, (w - side) / 2, (h - side) / 2, side, side).to_image();
    let d = dir(files);
    std::fs::create_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
    let mut rev = String::new();
    for (name, px) in NAMES {
        let mut out = image::imageops::resize(&square, px, px, image::imageops::FilterType::Lanczos3);
        if name.starts_with("apple") {
            for p in out.pixels_mut() {
                let a = p.0[3] as u32;
                for (c, bg) in background.iter().enumerate() {
                    p.0[c] = ((p.0[c] as u32 * a + *bg as u32 * (255 - a) + 127) / 255) as u8;
                }
                p.0[3] = 255;
            }
        }
        let png = encode(&out)?;
        if px == 512 {
            let sum = ring::digest::digest(&ring::digest::SHA256, &png);
            rev = sum.as_ref()[..6].iter().map(|b| format!("{b:02x}")).collect();
        }
        let tmp = d.join(format!(".{name}.tmp"));
        std::fs::write(&tmp, &png).and_then(|_| std::fs::rename(&tmp, d.join(name))).map_err(|e| format!("{}: {e}", d.join(name).display()))?;
    }
    bia_mcp::statefile::write_json(&d.join("icon.json"), &json!({"rev": rev})).map_err(|e| format!("{}: {e}", d.display()))?;
    Ok((rev, notes))
}

/// Back to the built-in icon.
pub fn clear_in(files: &Path) -> Result<(), String> {
    let d = dir(files);
    match std::fs::remove_dir_all(&d) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("{}: {e}", d.display())),
    }
}

/// What open web apps are told when the name or the icon changes.
pub fn self_frame(called: Option<String>) -> Value {
    json!({"t": "self", "called": called.unwrap_or_default(), "icon": rev()})
}

/// The tool: bia picks its own web app icon, or goes back to the built-in one.
pub fn tool(_: &Ctx, args: Value) -> Result<String, String> {
    let files = crate::files_dir();
    let path = arg_str_opt(&args, "path").unwrap_or_default();
    let path = path.trim();
    let called = || crate::WEB.get().and_then(|w| w.pki.chosen_name());
    if path.is_empty() || path == "default" {
        clear_in(&files)?;
        crate::link::broadcast(&self_frame(called()));
        return Ok("The web app is back on the built-in icon. Open web apps show it at once; an app already on a phone's home screen keeps its old picture until it is installed again (Android may refresh it by itself within days).".into());
    }
    let bg = colour(&arg_str_opt(&args, "background").unwrap_or_else(|| "#ffffff".into()))?;
    let src = crate::chat::att_abs(&files, path).ok_or_else(|| format!("path: {path:?} is not a file path"))?;
    let (_, notes) = set_in(&files, &src, bg)?;
    crate::link::broadcast(&self_frame(called()));
    let mut said = "The web app has its new icon: browser tab, notifications, bia's picture in the chat and the home-screen icon of the installed web app. Open web apps show it at once, no restart needed. A web app already on a phone's home screen keeps its old picture: on iPhone and iPad remove it and add it to the Home Screen again; Android may refresh it by itself within days, or install it again.".to_string();
    for n in notes {
        said.push(' ');
        said.push_str(&n);
    }
    Ok(said)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_becomes_three_icons_and_can_be_dropped() {
        let files = std::env::temp_dir().join(format!("bia-web-icon-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&files);
        std::fs::create_dir_all(&files).unwrap();
        assert_eq!(rev_in(&files), "");
        // 600x400, half transparent red
        let img = image::RgbaImage::from_pixel(600, 400, image::Rgba([255, 0, 0, 128]));
        let src = files.join("pic.png");
        std::fs::write(&src, encode(&img).unwrap()).unwrap();
        let (rev, notes) = set_in(&files, &src, [255, 255, 255]).unwrap();
        assert_eq!(rev.len(), 12);
        assert_eq!(rev_in(&files), rev);
        assert!(notes.iter().any(|n| n.contains("400x400")) && notes.iter().any(|n| n.contains("sharper")), "{notes:?}");
        for (name, px) in NAMES {
            let i = image::open(dir(&files).join(name)).unwrap().to_rgba8();
            assert_eq!(i.dimensions(), (px, px), "{name}");
            let p = i.get_pixel(px / 2, px / 2).0;
            if name.starts_with("apple") {
                // flattened onto white: opaque
                assert_eq!(p[3], 255);
                assert!(p[0] == 255 && (126..=128).contains(&p[1]), "{p:?}");
            } else {
                assert_eq!(p, [255, 0, 0, 128]);
            }
        }
        // the same picture gives the same rev, another one another
        assert_eq!(set_in(&files, &src, [0, 0, 0]).unwrap().0, rev);
        std::fs::write(&src, encode(&image::RgbaImage::from_pixel(512, 512, image::Rgba([0, 0, 255, 255]))).unwrap()).unwrap();
        let (rev2, notes) = set_in(&files, &src, [255, 255, 255]).unwrap();
        assert!(rev2 != rev && notes.is_empty(), "{notes:?}");
        // too small, not a picture: refused and the icon stays
        std::fs::write(&src, encode(&image::RgbaImage::from_pixel(100, 100, image::Rgba([0, 0, 0, 255]))).unwrap()).unwrap();
        assert!(set_in(&files, &src, [255, 255, 255]).unwrap_err().contains("too small"));
        std::fs::write(&src, b"<svg/>").unwrap();
        assert!(set_in(&files, &src, [255, 255, 255]).unwrap_err().contains("Convert it first"));
        assert_eq!(rev_in(&files), rev2);
        // a missing file of the set means the built-in icon
        std::fs::remove_file(dir(&files).join("icon-192.png")).unwrap();
        assert_eq!(rev_in(&files), "");
        clear_in(&files).unwrap();
        clear_in(&files).unwrap();
        assert_eq!(rev_in(&files), "");
        assert!(colour("#17212b").unwrap() == [0x17, 0x21, 0x2b] && colour("red").is_err() && colour("#ffé1").is_err());
        let _ = std::fs::remove_dir_all(&files);
    }
}
