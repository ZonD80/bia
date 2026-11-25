//! Acknowledgments: what a call says the moment bia understands a phrase ("Got it.",
//! "One moment."), while the model thinks about the answer. Built in for a few languages;
//! for any other one bia writes its own when it learns it (lang_acks), kept in
//! files/speech/acks.json (which also overrides the built-in ones).

use crate::paths;
use std::collections::BTreeMap;
use std::path::Path;

/// Neutral (no agreement, no gender), short.
const BUILT_IN: &[(&str, &[&str])] = &[
    ("en", &["Got it.", "One moment.", "Okay, let me see.", "Mm-hm, one second.", "Understood, one moment."]),
    ("ru", &["Так, секунду.", "Хорошо, сейчас.", "Минутку.", "Ясно, сейчас посмотрю."]),
    ("es", &["Vale, un momento.", "Entendido.", "A ver, un segundo.", "Un momento."]),
    ("fr", &["D'accord, un instant.", "Compris.", "Voyons voir.", "Une seconde."]),
    ("de", &["Okay, einen Moment.", "Verstanden.", "Mal sehen.", "Eine Sekunde."]),
    ("pt", &["Certo, um momento.", "Entendi.", "Deixa eu ver.", "Um segundo."]),
    ("it", &["Va bene, un attimo.", "Capito.", "Vediamo.", "Un secondo."]),
];

/// At most this many per language, each at most `MAX_CHARS`.
pub const MAX_PHRASES: usize = 8;
pub const MAX_CHARS: usize = 60;

fn file() -> std::path::PathBuf {
    paths::store().join("acks.json")
}

fn own(path: &Path) -> BTreeMap<String, Vec<String>> {
    std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// The acknowledgments of `lang` (bia's own, else built in; empty when it has none).
pub fn phrases(lang: &str) -> Vec<String> {
    phrases_in(&file(), lang)
}

fn phrases_in(path: &Path, lang: &str) -> Vec<String> {
    if let Some(p) = own(path).get(lang).filter(|p| !p.is_empty()) {
        return p.clone();
    }
    BUILT_IN.iter().find(|(l, _)| *l == lang).map(|(_, p)| p.iter().map(|s| s.to_string()).collect()).unwrap_or_default()
}

/// Set bia's own acknowledgments for `lang` (empty: back to the built-in ones, if any).
pub fn set(lang: &str, list: &[String]) -> Result<Vec<String>, String> {
    set_in(&file(), lang, list)
}

fn set_in(path: &Path, lang: &str, list: &[String]) -> Result<Vec<String>, String> {
    let lang = lang.trim().to_lowercase();
    if lang.is_empty() || lang.len() > 8 {
        return Err("lang: a language code such as es".into());
    }
    let clean: Vec<String> = list.iter().map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect();
    if clean.len() > MAX_PHRASES {
        return Err(format!("at most {MAX_PHRASES} phrases"));
    }
    if let Some(p) = clean.iter().find(|p| p.chars().count() > MAX_CHARS) {
        return Err(format!("too long for an acknowledgment (max {MAX_CHARS} characters): {p}"));
    }
    let mut all = own(path);
    if clean.is_empty() {
        all.remove(&lang);
    } else {
        all.insert(lang.clone(), clean);
    }
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_string_pretty(&all).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, body).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(phrases_in(path, &lang))
}

/// For the model, when it learns `lang` and calls have nothing to say in it yet.
pub fn missing_hint(lang: &str) -> Option<String> {
    hint_if(phrases(lang).is_empty(), lang)
}

fn hint_if(missing: bool, lang: &str) -> Option<String> {
    missing.then(|| {
        format!(
            "Calls in {lang} have no acknowledgments yet (said the moment you understand a phrase, while you think): write 3-5 short neutral ones in {lang}, like \"Got it.\", \"One moment.\", \"Let me see.\" (no yes/no, no grammatical gender), with lang_acks(lang=\"{lang}\", phrases=[…])."
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_ones_override_and_clear() {
        let d = std::env::temp_dir().join(format!("bia-acks-{}", std::process::id()));
        let f = d.join("acks.json");
        assert_eq!(phrases_in(&f, "en")[0], "Got it.");
        assert!(phrases_in(&f, "nl").is_empty());
        assert!(hint_if(true, "nl").is_some_and(|h| h.contains("lang_acks(lang=\"nl\"")));
        assert_eq!(set_in(&f, "nl", &["Begrepen.".into(), " Een moment. ".into()]).unwrap(), ["Begrepen.", "Een moment."]);
        assert_eq!(set_in(&f, "en", &["Right.".into()]).unwrap(), ["Right."]);
        assert_eq!(set_in(&f, "en", &[]).unwrap()[0], "Got it."); // back to built in
        assert_eq!(phrases_in(&f, "nl"), ["Begrepen.", "Een moment."]);
        assert!(set_in(&f, "nl", &["x".repeat(61)]).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
