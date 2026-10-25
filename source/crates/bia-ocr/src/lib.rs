//! Words on the screen and where they are. PaddleOCR (PP-OCRv5 mobile) on the image's
//! onnxruntime: a text detector finds lines, a recognizer per script reads them, and
//! each word gets the screen point at its center. Recognizers follow Chrome's languages
//! (plus English); a missing model is downloaded first.
use serde_json::Value;
use std::fs;
use std::path::Path;

mod engine;
mod models;

pub use models::{dir as models_dir, ensure};

/// Chrome's profile (bia-mcp-chrome runs it there).
pub const CHROME_PROFILE: &str = "/home/bia/.config/bia-chrome";

pub struct Word {
    pub text: String,
    /// center
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// One detected line of text, words left to right.
pub struct Line {
    pub words: Vec<Word>,
    pub score: f32,
}

/// Pixel rectangle on the screen.
#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// Chrome's UI locale: Local State intl.app_locale, else en-US (on Linux Chrome follows
/// its LANG, and bia starts it with C.UTF-8).
pub fn chrome_ui_locale() -> String {
    read_json(&Path::new(CHROME_PROFILE).join("Local State"))
        .and_then(|v| v["intl"]["app_locale"].as_str().map(str::to_string))
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| "en-US".into())
}

/// The UI locale, then the page languages Chrome asks sites for.
pub fn chrome_locales() -> Vec<String> {
    let mut out = vec![chrome_ui_locale()];
    if let Some(list) = read_json(&Path::new(CHROME_PROFILE).join("Default/Preferences"))
        .and_then(|v| v["intl"]["accept_languages"].as_str().map(str::to_string))
    {
        out.extend(list.split(',').map(|l| l.trim().to_string()).filter(|l| !l.is_empty()));
    }
    out
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

/// The recognizer (script) that reads a locale; None when there is none for it.
pub fn script(locale: &str) -> Option<&'static str> {
    let l = locale.replace('_', "-").to_ascii_lowercase();
    let main = l.split('-').next().unwrap_or("");
    Some(match main {
        "en" => "en",
        "ru" | "uk" | "be" => "eslav",
        "bg" | "sr" | "mk" | "kk" | "ky" | "mn" | "tg" if !l.contains("latn") => "cyrillic",
        "zh" | "ja" => "ch",
        "ko" => "korean",
        "ar" | "fa" | "ur" | "ps" | "ug" => "arabic",
        "hi" | "mr" | "ne" | "sa" => "devanagari",
        "th" => "th",
        "el" => "el",
        "ta" => "ta",
        "te" => "te",
        "af" | "az" | "bs" | "ca" | "cs" | "cy" | "da" | "de" | "es" | "et" | "eu" | "fi" | "fil" | "fr" | "ga" | "gl"
        | "hr" | "hu" | "id" | "is" | "it" | "lt" | "lv" | "ms" | "mt" | "nl" | "no" | "nb" | "nn" | "pl" | "pt" | "ro"
        | "sk" | "sl" | "sq" | "sr" | "sv" | "sw" | "tl" | "tr" | "uz" | "vi" | "zu" | "la" | "oc" => "latin",
        _ => return None,
    })
}

/// Recognizers for `locales`: theirs in order, plus English unless one of them reads
/// Latin letters already; at most three (each one reads every line).
pub fn scripts_for(locales: &[String]) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for s in locales.iter().filter_map(|l| script(l)) {
        if !out.contains(&s) && out.len() < 3 {
            out.push(s);
        }
    }
    if !out.iter().any(|s| ["en", "latin", "eslav", "ch"].contains(s)) {
        out.truncate(2);
        out.push("en");
    }
    out
}

/// Reads the screenshot `png` (inside `area` only, when given) with the recognizers
/// `scripts` (their models must be in place: `ensure`). Lines top to bottom.
pub fn read(png: &Path, area: Option<Rect>, scripts: &[&str]) -> Result<Vec<Line>, String> {
    let img = engine::Rgb::decode(png)?;
    let area = area.unwrap_or(Rect { x: 0, y: 0, w: img.w, h: img.h });
    engine::read(&img, area, scripts)
}

/// The words in `png` (inside `area`, when given) with their points, read in Chrome's
/// languages; models are downloaded first when missing. Ends with a note on languages
/// that could not be set up.
pub fn words(png: &Path, area: Option<Rect>) -> Result<String, String> {
    let (scripts, note) = ensure(&scripts_for(&chrome_locales()))?;
    let lines = read(png, area, &scripts)?;
    let mut out = if lines.is_empty() { "(no text found)".to_string() } else { describe(&lines) };
    if let Some(n) = note {
        out += &format!("\n({n})");
    }
    Ok(out)
}

/// One output line per row of text (lines side by side, like buttons, share it): each
/// word with the point to click it.
pub fn describe(lines: &[Line]) -> String {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut last: Option<&Word> = None;
    for l in lines {
        let words = l.words.iter().map(|w| format!("{} ({},{})", w.text, w.x, w.y)).collect::<Vec<_>>().join("  ");
        let first = &l.words[0];
        match (rows.last_mut(), last) {
            (Some(row), Some(p)) if (first.y - p.y).abs() < p.h.min(first.h) / 2 && first.x > p.x => row.push(words),
            _ => rows.push(vec![words]),
        }
        last = l.words.last();
    }
    rows.iter().map(|r| r.join("   ")).collect::<Vec<_>>().join("\n")
}

/// Where `label` (one or more words) is written: (line, center x, center y) of each
/// place, in reading order. Case, spaces and zero-width characters do not matter.
pub fn find(lines: &[Line], label: &str) -> Vec<(usize, i32, i32)> {
    let want = norm(label);
    let mut out = Vec::new();
    if want.is_empty() {
        return out;
    }
    for (i, line) in lines.iter().enumerate() {
        for start in 0..line.words.len() {
            let mut have = String::new();
            for end in start..line.words.len() {
                have += &norm(&line.words[end].text);
                if have == want {
                    let (a, b) = (&line.words[start], &line.words[end]);
                    let (x0, x1) = (a.x - a.w / 2, b.x + b.w / 2);
                    out.push((i, (x0 + x1) / 2, (a.y + b.y) / 2));
                }
                if have.len() >= want.len() {
                    break;
                }
            }
        }
    }
    out
}

fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, '\u{200b}'..='\u{200d}' | '\u{2060}' | '\u{feff}'))
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(words: &[(&str, i32, i32, i32)]) -> Line {
        Line {
            words: words.iter().map(|&(t, x, y, w)| Word { text: t.into(), x, y, w, h: 14 }).collect(),
            score: 1.0,
        }
    }

    #[test]
    fn locales_map_to_recognizers() {
        assert_eq!(script("en-US"), Some("en"));
        assert_eq!(script("ru"), Some("eslav"));
        assert_eq!(script("sr"), Some("cyrillic"));
        assert_eq!(script("sr-Latn"), Some("latin"));
        assert_eq!(script("zh-TW"), Some("ch"));
        assert_eq!(script("pt_BR"), Some("latin"));
        assert_eq!(script("xx"), None);
        let s = |v: &[&str]| scripts_for(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(s(&["ru", "en-US", "en"]), ["eslav", "en"]);
        assert_eq!(s(&["de", "fr"]), ["latin"]);
        assert_eq!(s(&["ar", "hi", "th", "el"]), ["arabic", "devanagari", "en"]);
    }

    #[test]
    fn labels_are_found_across_words() {
        let lines = [line(&[("Upload", 40, 20, 50), ("2", 70, 20, 8)]), line(&[("Laai", 200, 60, 30), ("op", 230, 60, 16)])];
        assert_eq!(find(&lines, "upload"), [(0, 40, 20)]);
        assert_eq!(find(&lines, "Laai op"), [(1, 211, 60)]);
        assert_eq!(norm("ອັບ\u{200b}ໂຫຼດ"), norm("ອັບໂຫຼດ"));
        assert_eq!(describe(&lines), "Upload (40,20)  2 (70,20)\nLaai (200,60)  op (230,60)");
        let buttons = [line(&[("Cancel", 600, 258, 50)]), line(&[("Upload", 690, 259, 50)])];
        assert_eq!(describe(&buttons), "Cancel (600,258)   Upload (690,259)");
    }
}
