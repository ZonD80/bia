//! Detection (DB) and recognition (CTC) the way RapidOCR runs PP-OCR, simplified for
//! screen text, which is never rotated: boxes are axis-aligned.
use crate::{models, Line, Rect, Word};
use ort::environment::Environment;
use ort::session::Session;
use ort::value::Tensor;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

/// The onnxruntime the image ships for speech (sherpa-onnx).
const ORT_LIB: &str = "/usr/local/lib/bia/speech/lib/libonnxruntime.so";

/// Recognized lines below this mean score are dropped (RapidOCR's text_score).
const MIN_SCORE: f32 = 0.5;

struct Model {
    session: Session,
    /// CTC classes: blank, the model's characters, space
    chars: Vec<String>,
}

fn env() -> Result<&'static Environment, String> {
    static ENV: OnceLock<Result<Environment, String>> = OnceLock::new();
    ENV.get_or_init(|| {
        let lib = std::env::var("BIA_ONNXRUNTIME").unwrap_or_else(|_| ORT_LIB.into());
        ort::init_from(&lib)
            .map_err(|e| format!("onnxruntime {lib}: {e}"))?
            .with_name("bia-ocr")
            .build()
            .map_err(|e| format!("onnxruntime: {e}"))
    })
    .as_ref()
    .map_err(Clone::clone)
}

/// Runs `f` with the model `name`, loading it once.
fn with_model<T>(name: &str, f: impl FnOnce(&mut Model) -> Result<T, String>) -> Result<T, String> {
    static MODELS: OnceLock<Mutex<HashMap<String, Model>>> = OnceLock::new();
    let mut models = MODELS.get_or_init(Default::default).lock().map_err(|_| "OCR models lock poisoned".to_string())?;
    if !models.contains_key(name) {
        let path = models::path(name).ok_or_else(|| format!("no OCR model {name}"))?;
        let threads = std::thread::available_parallelism().map(|n| n.get().min(4)).unwrap_or(2);
        let session = Session::builder(env()?)
            .map_err(|e| e.to_string())?
            // the CPU as RapidOCR runs it (ort's default may pick a reduced-precision NPU)
            .with_auto_device(ort::session::builder::AutoDevicePolicy::PreferCPU)
            .map_err(|e| e.to_string())?
            .with_intra_threads(threads)
            .map_err(|e| e.to_string())?
            .commit_from_file(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let mut chars = vec!["blank".to_string()];
        if name != models::DET {
            let list = session.metadata().ok().and_then(|m| m.custom("character")).ok_or_else(|| format!("{name}: no character list"))?;
            chars.extend(list.lines().map(str::to_string));
            chars.push(" ".into());
        }
        models.insert(name.to_string(), Model { session, chars });
    }
    f(models.get_mut(name).ok_or("model vanished")?)
}

pub fn read(img: &Rgb, area: Rect, scripts: &[&str]) -> Result<Vec<Line>, String> {
    let boxes = detect(img, area)?;
    let mut lines = Vec::new();
    for b in boxes {
        let mut best: Option<Line> = None;
        for s in scripts {
            if let Some(l) = recognize(img, b, s)? {
                if best.as_ref().is_none_or(|o| l.score > o.score) {
                    best = Some(l);
                }
            }
        }
        lines.extend(best.filter(|l| l.score >= MIN_SCORE && !l.words.is_empty()));
    }
    // rows top to bottom (tops within 10 px are one row), left to right
    lines.sort_by_key(|l| (l.words[0].y - l.words[0].h / 2, l.words[0].x));
    let mut row = 0;
    let mut keyed: Vec<(i32, i32, Line)> = Vec::new();
    let mut last_top: Option<i32> = None;
    for l in lines {
        let top = l.words[0].y - l.words[0].h / 2;
        if last_top.is_some_and(|t| top - t >= 10) {
            row += 1;
        }
        last_top = Some(top);
        keyed.push((row, l.words[0].x - l.words[0].w / 2, l));
    }
    keyed.sort_by_key(|(r, x, _)| (*r, *x));
    Ok(keyed.into_iter().map(|(_, _, l)| l).collect())
}

/// Text boxes in `area`, screen coordinates.
fn detect(img: &Rgb, area: Rect) -> Result<Vec<Rect>, String> {
    let area = img.clip(area);
    if area.w < 4 || area.h < 4 {
        return Ok(Vec::new());
    }
    // small screens are scaled up until the short side is 736; sides multiple of 32
    let ratio = (736.0 / area.w.min(area.h) as f32).max(1.0);
    let side = |v: i32| (((v as f32 * ratio) / 32.0).round() as usize * 32).max(32);
    let (rw, rh) = (side(area.w), side(area.h));
    let input = img.tensor(area, rw, rh);
    let (shape, prob) = with_model(models::DET, |m| {
        let out = m.session.run(ort::inputs![Tensor::from_array(([1usize, 3, rh, rw], input)).map_err(|e| e.to_string())?]).map_err(|e| e.to_string())?;
        let (shape, data) = out[0].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
        Ok((shape.to_vec(), data.to_vec()))
    })?;
    let (mh, mw) = (shape[shape.len() - 2] as usize, shape[shape.len() - 1] as usize);
    // threshold, then dilate by a 2x2 kernel
    let on: Vec<bool> = prob.iter().map(|&p| p > 0.3).collect();
    let mask: Vec<bool> = (0..mw * mh)
        .map(|i| {
            let (x, y) = (i % mw, i / mw);
            on[i] || (x > 0 && on[i - 1]) || (y > 0 && on[i - mw]) || (x > 0 && y > 0 && on[i - mw - 1])
        })
        .collect();
    let mut seen = vec![false; mw * mh];
    let mut out = Vec::new();
    for start in 0..mw * mh {
        if !mask[start] || seen[start] {
            continue;
        }
        // one 8-connected component: its bounding box
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
        let mut todo = vec![start];
        seen[start] = true;
        while let Some(i) = todo.pop() {
            let (x, y) = (i % mw, i / mw);
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx < 0 || ny < 0 || nx >= mw as i32 || ny >= mh as i32 {
                        continue;
                    }
                    let j = ny as usize * mw + nx as usize;
                    if mask[j] && !seen[j] {
                        seen[j] = true;
                        todo.push(j);
                    }
                }
            }
        }
        let (w, h) = ((x1 - x0) as f32, (y1 - y0) as f32);
        if w.min(h) < 3.0 {
            continue;
        }
        let mut sum = 0.0;
        for y in y0..=y1 {
            sum += prob[y * mw + x0..=y * mw + x1].iter().sum::<f32>();
        }
        if sum / (((x1 - x0 + 1) * (y1 - y0 + 1)) as f32) < 0.5 {
            continue;
        }
        // grow the shrunk text kernel back (unclip ratio 1.6)
        let d = w * h * 1.6 / (2.0 * (w + h));
        if w.min(h) + 2.0 * d < 5.0 {
            continue;
        }
        let sx = area.w as f32 / mw as f32;
        let sy = area.h as f32 / mh as f32;
        let fx = |v: f32| ((v * sx).round() as i32).clamp(0, area.w);
        let fy = |v: f32| ((v * sy).round() as i32).clamp(0, area.h);
        let (bx0, by0, bx1, by1) = (fx(x0 as f32 - d), fy(y0 as f32 - d), fx(x1 as f32 + d), fy(y1 as f32 + d));
        if bx1 - bx0 <= 3 || by1 - by0 <= 3 {
            continue;
        }
        out.push(Rect { x: area.x + bx0, y: area.y + by0, w: bx1 - bx0, h: by1 - by0 });
    }
    Ok(out)
}

/// Reads one box with the recognizer `script`; words get their place from the CTC frames.
fn recognize(img: &Rgb, b: Rect, script: &str) -> Result<Option<Line>, String> {
    const H: usize = 48;
    let rw = ((H as f32 * b.w as f32 / b.h as f32).ceil() as usize).clamp(1, 3200);
    let width = rw.max(320);
    let mut input = vec![0f32; 3 * H * width];
    let part = img.tensor(b, rw, H);
    for c in 0..3 {
        for y in 0..H {
            input[(c * H + y) * width..(c * H + y) * width + rw].copy_from_slice(&part[(c * H + y) * rw..(c * H + y + 1) * rw]);
        }
    }
    with_model(script, |m| {
        let out = m.session.run(ort::inputs![Tensor::from_array(([1usize, 3, H, width], input)).map_err(|e| e.to_string())?]).map_err(|e| e.to_string())?;
        let (shape, data) = out[0].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
        let (t_len, classes) = (shape[1] as usize, shape[2] as usize);
        if classes != m.chars.len() {
            return Err(format!("{script}: {classes} classes but {} characters", m.chars.len()));
        }
        // (class, first frame, last frame, probability) per decoded character
        let mut chars: Vec<(usize, usize, usize, f32)> = Vec::new();
        let mut prev = 0;
        for t in 0..t_len {
            let row = &data[t * classes..(t + 1) * classes];
            let (idx, p) = row.iter().enumerate().fold((0, f32::MIN), |a, (i, &v)| if v > a.1 { (i, v) } else { a });
            if idx != 0 && idx == prev {
                if let Some(last) = chars.last_mut() {
                    last.2 = t;
                }
            } else if idx != 0 {
                chars.push((idx, t, t, p));
            }
            prev = idx;
        }
        if chars.is_empty() {
            return Ok(None);
        }
        let score = chars.iter().map(|c| c.3).sum::<f32>() / chars.len() as f32;
        // frame → box pixels
        let px = width as f32 / t_len as f32 * b.w as f32 / rw as f32;
        let mut words = Vec::new();
        let mut cur: Option<(String, usize, usize)> = None;
        for &(idx, t0, t1, _) in &chars {
            let ch = &m.chars[idx];
            if ch.trim().is_empty() {
                words.extend(cur.take());
                continue;
            }
            match cur.as_mut() {
                Some(w) => {
                    w.0 += ch;
                    w.2 = t1;
                }
                None => cur = Some((ch.clone(), t0, t1)),
            }
        }
        words.extend(cur);
        let words = words
            .into_iter()
            .map(|(text, t0, t1)| {
                let x0 = (t0 as f32 * px) as i32;
                let x1 = (((t1 + 1) as f32 * px) as i32).min(b.w);
                Word { text, x: b.x + (x0 + x1) / 2, y: b.y + b.h / 2, w: (x1 - x0).max(1), h: b.h }
            })
            .collect();
        Ok(Some(Line { words, score }))
    })
}

/// An RGB screenshot.
pub struct Rgb {
    pub w: i32,
    pub h: i32,
    px: Vec<u8>,
}

impl Rgb {
    pub fn decode(path: &Path) -> Result<Self, String> {
        let file = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut dec = png::Decoder::new(std::io::BufReader::new(file));
        dec.set_transformations(png::Transformations::normalize_to_color8());
        let mut reader = dec.read_info().map_err(|e| e.to_string())?;
        let mut buf = vec![0; reader.output_buffer_size().ok_or("png too large")?];
        let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
        let ch = match info.color_type {
            png::ColorType::Grayscale => 1,
            png::ColorType::GrayscaleAlpha => 2,
            png::ColorType::Rgb => 3,
            png::ColorType::Rgba => 4,
            t => return Err(format!("png colour type {t:?}")),
        };
        let px = buf[..info.buffer_size()]
            .chunks(info.line_size)
            .flat_map(|row| row.chunks(ch).take(info.width as usize).flat_map(move |p| if ch < 3 { [p[0]; 3] } else { [p[0], p[1], p[2]] }))
            .collect();
        Ok(Rgb { w: info.width as i32, h: info.height as i32, px })
    }

    fn clip(&self, r: Rect) -> Rect {
        let x0 = r.x.clamp(0, self.w);
        let y0 = r.y.clamp(0, self.h);
        Rect { x: x0, y: y0, w: (r.x + r.w).clamp(x0, self.w) - x0, h: (r.y + r.h).clamp(y0, self.h) - y0 }
    }

    /// `r` resized (bilinear) to `w`x`h` as a CHW tensor, BGR, scaled to -1..1.
    fn tensor(&self, r: Rect, w: usize, h: usize) -> Vec<f32> {
        let r = self.clip(r);
        let mut out = vec![0f32; 3 * w * h];
        let (sx, sy) = (r.w as f32 / w as f32, r.h as f32 / h as f32);
        let at = |x: i32, y: i32, c: usize| {
            let (x, y) = ((r.x + x.clamp(0, r.w - 1)) as usize, (r.y + y.clamp(0, r.h - 1)) as usize);
            self.px[(y * self.w as usize + x) * 3 + c] as f32
        };
        for y in 0..h {
            let fy = ((y as f32 + 0.5) * sy - 0.5).max(0.0);
            let (y0, ty) = (fy.floor() as i32, fy.fract());
            for x in 0..w {
                let fx = ((x as f32 + 0.5) * sx - 0.5).max(0.0);
                let (x0, tx) = (fx.floor() as i32, fx.fract());
                for (plane, c) in [2usize, 1, 0].into_iter().enumerate() {
                    let top = at(x0, y0, c) * (1.0 - tx) + at(x0 + 1, y0, c) * tx;
                    let bottom = at(x0, y0 + 1, c) * (1.0 - tx) + at(x0 + 1, y0 + 1, c) * tx;
                    let v = top * (1.0 - ty) + bottom * ty;
                    out[(plane * h + y) * w + x] = v / 127.5 - 1.0;
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Needs onnxruntime (BIA_ONNXRUNTIME) and the models (BIA_OCR_MODELS).
    fn ready() -> bool {
        let ok = std::env::var("BIA_ONNXRUNTIME").is_ok() && models::path(models::DET).is_some_and(|p| p.is_file());
        if !ok {
            eprintln!("skipped: set BIA_ONNXRUNTIME and BIA_OCR_MODELS");
        }
        ok
    }

    fn testdata(name: &str) -> PathBuf {
        PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/src/testdata")).join(name)
    }

    /// Google Chrome 155's folder-upload prompt, Cancel focused (white on blue).
    #[test]
    fn reads_chrome_upload_prompt() {
        if !ready() {
            return;
        }
        let lines = crate::read(&testdata("upload-prompt.png"), None, &["en"]).unwrap();
        println!("{}", crate::describe(&lines));
        let cancel = crate::find(&lines, "Cancel");
        let upload = crate::find(&lines, "Upload");
        assert_eq!(cancel.len(), 1, "Cancel");
        let button = upload.iter().find(|u| u.1 > cancel[0].1 && (u.2 - cancel[0].2).abs() < 8).expect("Upload button");
        assert!((button.1 - 494).abs() < 12 && (button.2 - 158).abs() < 6, "{button:?}");
    }

    /// The same prompt in Russian (a page drawn like it), with the eslav recognizer.
    #[test]
    fn reads_russian_prompt() {
        if !ready() || !models::path("eslav").is_some_and(|p| p.is_file()) {
            return;
        }
        let lines = crate::read(&testdata("upload-prompt-ru.png"), None, &["eslav", "en"]).unwrap();
        println!("{}", crate::describe(&lines));
        let cancel = crate::find(&lines, "Отмена");
        let upload = crate::find(&lines, "Загрузить");
        assert_eq!(cancel.len(), 1, "Отмена");
        assert!(upload.iter().any(|u| u.1 > cancel[0].1 && (u.2 - cancel[0].2).abs() < 8), "Загрузить");
    }
}

