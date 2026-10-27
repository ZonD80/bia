//! The owner's language. The stick carries it as `EFI/bia/lang.json`; on first boot bia
//! makes it the default for speech, adds its keyboard, downloads its voice and speaks it
//! from the first reply on.

use serde_json::Value;

pub const ESP_LANG_REL: &str = "EFI/bia/lang.json";

/// The lang MCP's catalog: every language Whisper hears, with the voices that speak it.
const CATALOG: &str = include_str!("../../../source/flash/MCPs/lang/catalog.json");

#[derive(Debug, Clone, PartialEq)]
pub struct Language {
    pub code: String,
    pub name: String,
    /// A voice exists: bia can speak it, not only read and write it.
    pub voice: bool,
}

/// All languages, English first, the rest by name.
pub fn languages() -> Vec<Language> {
    let cat: Value = serde_json::from_str(CATALOG).unwrap_or_default();
    let mut out: Vec<Language> = cat["languages"]
        .as_object()
        .map(|m| {
            m.iter()
                .map(|(code, v)| Language {
                    code: code.clone(),
                    name: v["name"].as_str().unwrap_or(code).to_string(),
                    voice: v["voices"].as_array().is_some_and(|a| !a.is_empty()),
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| (a.code != "en", &a.name).cmp(&(b.code != "en", &b.name)));
    out
}

/// A language by code (`ru`, `pt-BR` → pt) or English name (`Russian`).
pub fn find(input: &str) -> Option<Language> {
    let s = input.trim();
    let base = s.split(['-', '_']).next().unwrap_or(s).to_ascii_lowercase();
    languages()
        .into_iter()
        .find(|l| l.code == base || l.name.eq_ignore_ascii_case(s))
}

/// The host's language from its locale variables (macOS apps opened from the Finder have
/// none: the system locale then), if bia knows it; else English.
pub fn host_default() -> String {
    let env = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .find(|v| !v.is_empty() && v != "C" && v != "POSIX" && !v.starts_with("C."));
    #[cfg(target_os = "macos")]
    let env = env.or_else(|| {
        std::process::Command::new("defaults")
            .args(["read", "-g", "AppleLocale"])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    });
    env.and_then(|v| find(&v)).map_or_else(|| "en".to_string(), |l| l.code)
}

/// `EFI/bia/lang.json` for `code`.
pub fn lang_json(code: &str) -> Vec<u8> {
    format!("{{\"lang\": \"{code}\"}}\n").into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_languages() {
        let all = languages();
        assert!(all.len() > 90);
        assert_eq!(all[0].code, "en");
        assert!(all.iter().find(|l| l.code == "ru").unwrap().voice);
        assert_eq!(find("Russian").unwrap().code, "ru");
        assert_eq!(find("pt-BR").unwrap().code, "pt");
        assert_eq!(find("ru_RU.UTF-8").unwrap().code, "ru");
        assert!(find("xx").is_none());
        let j: Value = serde_json::from_slice(&lang_json("ru")).unwrap();
        assert_eq!(j["lang"], "ru");
    }
}
