//! The owner's language. The stick carries it in `EFI/bia/self.json`; on first boot bia
//! makes it the default for speech, adds its keyboard, downloads its voice and speaks it
//! from the first reply on.

use serde_json::Value;

/// The owner's language and bia's name and gender: `{"lang": "ru", "name": "Fred",
/// "gender": "male"}`. bia reads it once at its first boot and deletes it.
pub const ESP_SELF_REL: &str = "EFI/bia/self.json";

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

/// The name and gender the owner gives bia in the wizard, carried in `EFI/bia/self.json`;
/// bia puts them in the first line of its system prompt (empty: bia asks on its first start).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Persona {
    pub name: String,
    pub gender: String,
}

pub const GENDERS: [&str; 3] = ["male", "female", "neutral"];

impl Persona {
    /// Trimmed and checked (the agent applies the same rules, crate me.rs in bia-agent).
    pub fn new(name: &str, gender: &str) -> anyhow::Result<Persona> {
        let name = name.trim().to_string();
        let gender = gender.trim().to_lowercase();
        if name.chars().count() > 40 || name.chars().any(|c| c.is_control() || matches!(c, '|' | '·')) {
            anyhow::bail!("bia's name: up to 40 characters, no | or ·");
        }
        if !gender.is_empty() && !GENDERS.contains(&gender.as_str()) {
            anyhow::bail!("bia's gender: male, female or neutral");
        }
        Ok(Persona { name, gender })
    }
}

/// `EFI/bia/self.json`: the language (catalog code, if any) and what of name and gender was
/// given; None when there is nothing to say.
pub fn self_json(lang: Option<&str>, p: &Persona) -> Option<Vec<u8>> {
    let mut v = serde_json::json!({});
    if let Some(l) = lang.filter(|l| !l.is_empty()) {
        v["lang"] = l.into();
    }
    if !p.name.is_empty() {
        v["name"] = p.name.clone().into();
    }
    if !p.gender.is_empty() {
        v["gender"] = p.gender.clone().into();
    }
    (v != serde_json::json!({})).then(|| format!("{v}\n").into_bytes())
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
        let j: Value = serde_json::from_slice(&self_json(Some("ru"), &Persona::default()).unwrap()).unwrap();
        assert_eq!(j, serde_json::json!({"lang": "ru"}));
        assert!(self_json(None, &Persona::default()).is_none());
        let p = Persona::new(" Фред ", "Male").unwrap();
        let j: Value = serde_json::from_slice(&self_json(Some("ru"), &p).unwrap()).unwrap();
        assert_eq!(j, serde_json::json!({"lang": "ru", "name": "Фред", "gender": "male"}));
        let j: Value = serde_json::from_slice(&self_json(None, &Persona::new("", "female").unwrap()).unwrap()).unwrap();
        assert_eq!(j, serde_json::json!({"gender": "female"}));
        assert!(Persona::new("a|b", "").is_err());
        assert!(Persona::new("Fred", "robot").is_err());
        assert_eq!(Persona::new("", "").unwrap(), Persona::default());
    }
}
