//! The language registry (/home/bia/files/speech/languages.json): which languages are
//! enabled, the voice per language, installed models and speech settings. bia-mcp-lang
//! writes it (atomically); the telegram/whatsapp MCPs re-read it when its mtime changes.

use crate::paths;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

/// The voice in the image: low quality (16 kHz), the fastest Piper; images before 2.5.1
/// had lessac-medium (an old registry naming it falls back to this one).
pub const BUNDLED_VOICE: &str = "piper-en_US-lessac-low";
/// English's voice in the image (since 2.5.1): Gradium Phonon, which streams as it speaks.
/// The Piper voice above stays as its fallback (see `tts::fallback_voice`).
pub const BUNDLED_PHONON: &str = "phonon-31m-en";
/// Phonon voices of other languages in the image (since 2.5.1), used once the language is
/// learned: (voice id, language, locale).
pub const BUNDLED_PHONON_MORE: [(&str, &str, &str); 4] = [
    ("phonon-40m-fr", "fr", "fr_FR"),
    ("phonon-40m-de", "de", "de_DE"),
    ("phonon-40m-es", "es", "es_ES"),
    ("phonon-40m-pt", "pt", "pt_BR"),
];
pub const BUNDLED_ASR: &str = "whisper-tiny";

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Voice {
    /// vits (Piper) or kokoro
    pub kind: String,
    pub langs: Vec<String>,
    pub locale: String,
    pub dir: String,
    pub model: String,
    pub tokens: String,
    pub data_dir: String,
    pub voices: String,
    pub lexicon: String,
    pub dict_dir: String,
    pub sid: i32,
    pub bundled: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Asr {
    /// online_transducer (Kroko: streams, decodes while the speaker talks),
    /// whisper, nemo_transducer (Parakeet), nemo_ctc (one-language models such as
    /// shenava-fa; `encoder` is its one model file) or moonshine (English, fast enough on a
    /// phone to answer as you stop speaking). Only Whisper detects the language; the others
    /// transcribe only `langs`.
    pub kind: String,
    /// Empty: every language Whisper knows.
    pub langs: Vec<String>,
    pub dir: String,
    pub encoder: String,
    pub decoder: String,
    /// nemo_transducer only
    pub joiner: String,
    pub tokens: String,
    pub bundled: bool,
    /// moonshine only: its audio preprocessor and the cached decoder (`decoder` is the
    /// uncached one)
    pub preprocessor: String,
    pub cached_decoder: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    /// Clean audio (GTCRN) before language detection and the speaker check.
    pub denoise: bool,
    /// Feed the cleaned audio to Whisper too (off: Whisper gets the original).
    pub asr_denoised: bool,
    /// Split at short pauses so a language change mid-phrase is caught.
    pub split_pauses: bool,
    /// Drop typical Whisper hallucinations (noise → "Thank you for watching").
    pub hallucination_filter: bool,
    /// Report a language that is heard but not installed.
    pub discovery: bool,
    /// Opt-in: drop call speech that repeats what we are saying (echo).
    pub echo_filter: bool,
    /// Opt-in: drop speech whose voice does not match the enrolled speaker.
    pub speaker_check: bool,
    /// Cosine similarity needed to pass the speaker check.
    pub speaker_threshold: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            denoise: true,
            asr_denoised: true,
            split_pauses: true,
            hallucination_filter: true,
            discovery: true,
            echo_filter: false,
            speaker_check: false,
            speaker_threshold: 0.45,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Registry {
    pub default_lang: String,
    /// Languages speech is routed to (what bia has "learned").
    pub enabled: Vec<String>,
    /// Heard-but-declined languages: no discovery notice for these.
    pub ignored: Vec<String>,
    /// Language → voice id.
    pub voice: BTreeMap<String, String>,
    pub voices: BTreeMap<String, Voice>,
    pub asr: BTreeMap<String, Asr>,
    /// Model for live calls (fast) and for voice notes / files (best installed).
    pub asr_call: String,
    pub asr_file: String,
    /// vad / denoiser / speaker model id → file path.
    pub extras: BTreeMap<String, String>,
    /// Enrolled speakers for the opt-in speaker check: key (telegram:<user id>,
    /// whatsapp:<phone>) → embedding.
    pub speakers: BTreeMap<String, Vec<f32>>,
    pub settings: Settings,
    /// Language → fcitx5 input that overrides the default (`crate::input`); "" = none.
    pub input: BTreeMap<String, String>,
}

impl Default for Registry {
    fn default() -> Self {
        let b = paths::bundled().join("models");
        let piper = b.join(BUNDLED_VOICE);
        let tiny = b.join(BUNDLED_ASR);
        let s = |p: &Path| p.to_string_lossy().into_owned();
        let mut voices = BTreeMap::new();
        voices.insert(
            BUNDLED_VOICE.to_string(),
            Voice {
                kind: "vits".into(),
                langs: vec!["en".into()],
                locale: "en_US".into(),
                dir: s(&piper),
                model: s(&piper.join("en_US-lessac-low.onnx")),
                tokens: s(&piper.join("tokens.txt")),
                data_dir: s(&piper.join("espeak-ng-data")),
                bundled: true,
                ..Default::default()
            },
        );
        let phonon = b.join(BUNDLED_PHONON);
        voices.insert(
            BUNDLED_PHONON.to_string(),
            Voice {
                kind: "phonon".into(),
                langs: vec!["en".into()],
                locale: "en_US".into(),
                dir: s(&phonon),
                model: s(&phonon.join("model.q8.gguf")),
                tokens: s(&phonon.join("tokenizer.json")),
                bundled: true,
                ..Default::default()
            },
        );
        for (id, lang, locale) in BUNDLED_PHONON_MORE {
            let dir = b.join(id);
            voices.insert(
                id.to_string(),
                Voice {
                    kind: "phonon".into(),
                    langs: vec![lang.into()],
                    locale: locale.into(),
                    dir: s(&dir),
                    model: s(&dir.join("model.q8.gguf")),
                    tokens: s(&dir.join("tokenizer.json")),
                    bundled: true,
                    ..Default::default()
                },
            );
        }
        let mut asr = BTreeMap::new();
        asr.insert(
            BUNDLED_ASR.to_string(),
            Asr {
                kind: "whisper".into(),
                langs: vec![],
                dir: s(&tiny),
                encoder: s(&tiny.join("encoder.onnx")),
                decoder: s(&tiny.join("decoder.onnx")),
                tokens: s(&tiny.join("tokens.txt")),
                bundled: true,
                ..Default::default()
            },
        );
        let mut extras = BTreeMap::new();
        extras.insert("silero-vad".into(), s(&b.join("silero-vad/silero_vad.onnx")));
        extras.insert("gtcrn".into(), s(&b.join("gtcrn/gtcrn_simple.onnx")));
        Self {
            default_lang: "en".into(),
            enabled: vec!["en".into()],
            ignored: vec![],
            voice: [("en".to_string(), BUNDLED_PHONON.to_string())].into(),
            voices,
            asr,
            asr_call: BUNDLED_ASR.into(),
            asr_file: BUNDLED_ASR.into(),
            extras,
            speakers: BTreeMap::new(),
            settings: Settings::default(),
            input: BTreeMap::new(),
        }
    }
}

impl Registry {
    /// Read the registry file; bundled entries are always present (a new image may
    /// move them), the rest comes from the file.
    pub fn load() -> Self {
        let mut r: Registry = std::fs::read_to_string(paths::registry())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        let d = Registry::default();
        // a bundled voice of an older image (lessac-medium before 2.5.1) is gone after the update
        r.voices.retain(|k, v| !v.bundled || d.voices.contains_key(k));
        for (k, v) in d.voices {
            if Path::new(&v.dir).exists() {
                r.voices.insert(k, v);
                continue;
            }
            // The image lacks this voice (one before 2.5.1 has no Phonon): a copy installed
            // from the catalog under the same id stays, or is found again in the models
            // folder when an entry saved by mistake points at the image.
            let installed = r.voices.get(&k).is_some_and(|o| !o.bundled && Path::new(&o.dir).exists());
            if installed {
                continue;
            }
            let own = crate::paths::models().join(&k);
            match crate::store::resolve_voice(&own, &v.kind, &v.langs, &v.locale) {
                Ok(found) if own.exists() => {
                    r.voices.insert(k, found);
                }
                _ => {
                    r.voices.insert(k, v);
                }
            }
        }
        for (k, v) in d.asr {
            r.asr.insert(k, v);
        }
        for (k, v) in d.extras {
            let keep = r.extras.get(&k).is_some_and(|p| Path::new(p).exists());
            if !keep {
                r.extras.insert(k, v);
            }
        }
        if r.enabled.is_empty() {
            r.enabled.push(r.default_lang.clone());
        }
        if !r.asr.contains_key(&r.asr_call) {
            r.asr_call = BUNDLED_ASR.into();
        }
        if !r.asr.contains_key(&r.asr_file) {
            r.asr_file = BUNDLED_ASR.into();
        }
        r.voice.retain(|_, v| r.voices.contains_key(v));
        // English always has a voice: the bundled one when its own went away. The image's
        // Piper voice was English's default before Phonon came with the image: it moves on.
        let en = r.voice.entry("en".into()).or_insert_with(|| BUNDLED_PHONON.into());
        if en == BUNDLED_VOICE {
            *en = BUNDLED_PHONON.into();
        }
        r
    }

    pub fn save(&self) -> Result<(), String> {
        let path = paths::registry();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
        }
        let tmp = path.with_extension(format!("json.{}", std::process::id()));
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&tmp, text).map_err(|e| format!("write {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("rename {}: {e}", path.display()))?;
        invalidate();
        Ok(())
    }

    pub fn is_enabled(&self, lang: &str) -> bool {
        self.enabled.iter().any(|l| l == lang)
    }

    /// Voice for `lang`; a voice for the exact locale wins when one is installed.
    pub fn voice_for(&self, lang: &crate::langs::Lang) -> Option<(&str, &Voice)> {
        if let Some(loc) = lang.locale() {
            let exact = self
                .voices
                .iter()
                .find(|(_, v)| v.locale == loc && v.langs.contains(&lang.code));
            if let Some((id, v)) = exact {
                return Some((id.as_str(), v));
            }
        }
        // only a voice that speaks the language: what bia hears in Spanish it answers in a
        // Spanish voice, never with another language's (a wrong mapping gives none)
        let id = self.voice.get(&lang.code)?;
        self.voices.get(id).filter(|v| v.langs.contains(&lang.code)).map(|v| (id.as_str(), v))
    }

    pub fn extra(&self, id: &str) -> Option<&str> {
        self.extras.get(id).map(String::as_str).filter(|p| Path::new(p).exists())
    }
}

struct Lock(libc::c_int);

impl Drop for Lock {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.0, libc::LOCK_UN);
            libc::close(self.0);
        }
    }
}

fn lock() -> Result<Lock, String> {
    let dir = paths::store();
    std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    let p = std::ffi::CString::new(dir.join(".lock").to_string_lossy().as_bytes()).unwrap();
    let fd = unsafe { libc::open(p.as_ptr(), libc::O_CREAT | libc::O_RDWR | libc::O_CLOEXEC, 0o644) };
    if fd < 0 {
        return Err("cannot open the language registry lock".into());
    }
    unsafe { libc::flock(fd, libc::LOCK_EX) };
    Ok(Lock(fd))
}

/// Read-modify-write the registry under a file lock (the lang, telegram and whatsapp
/// MCPs and the agent may all change it). The console's input methods follow.
pub fn update<T>(f: impl FnOnce(&mut Registry) -> Result<T, String>) -> Result<T, String> {
    let _l = lock()?;
    let mut r = Registry::load();
    let old = r.clone();
    let out = f(&mut r)?;
    r.save()?;
    if crate::input::affects(&old, &r) {
        crate::input::sync(Some(&old), &r);
    }
    Ok(out)
}

struct Cached {
    mtime: Option<SystemTime>,
    reg: Arc<Registry>,
}

fn cache() -> &'static Mutex<Option<Cached>> {
    static C: OnceLock<Mutex<Option<Cached>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(None))
}

fn invalidate() {
    if let Ok(mut c) = cache().lock() {
        *c = None;
    }
}

/// The current registry; re-read when the file changes (another MCP saved it).
pub fn current() -> Arc<Registry> {
    let mtime = std::fs::metadata(paths::registry()).and_then(|m| m.modified()).ok();
    let mut c = cache().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(cached) = c.as_ref() {
        if cached.mtime == mtime {
            return cached.reg.clone();
        }
    }
    let reg = Arc::new(Registry::load());
    *c = Some(Cached { mtime, reg: reg.clone() });
    reg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_roundtrip() {
        let dir = std::env::temp_dir().join(format!("bia-speech-reg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::env::set_var("BIA_SPEECH_STORE", &dir);
        let r = Registry::load();
        assert_eq!(r.enabled, ["en"]);
        assert_eq!(r.voice["en"], BUNDLED_PHONON);
        let mut r2 = r.clone();
        r2.enabled.push("ru".into());
        r2.voices.insert(
            "piper-ru_RU-irina-medium".into(),
            Voice { kind: "vits".into(), langs: vec!["ru".into()], locale: "ru_RU".into(), ..Default::default() },
        );
        r2.voice.insert("ru".into(), "piper-ru_RU-irina-medium".into());
        r2.voice.insert("de".into(), "gone".into());
        r2.save().unwrap();
        let r3 = current();
        assert!(r3.is_enabled("ru"));
        assert!(!r3.voice.contains_key("de"), "dangling voice dropped");
        let es_mx = crate::langs::parse("ru-RU").unwrap();
        assert_eq!(r3.voice_for(&es_mx).unwrap().0, "piper-ru_RU-irina-medium");
        assert!(r3.voice_for(&crate::langs::parse("es").unwrap()).is_none());

        // an image before 2.5.1 bundled lessac-medium: after the update its entry is gone
        // and English speaks with the new bundled voice
        let mut old = Registry::load();
        old.voices.insert(
            "piper-en_US-lessac-medium".into(),
            Voice { kind: "vits".into(), langs: vec!["en".into()], bundled: true, ..Default::default() },
        );
        old.voice.insert("en".into(), "piper-en_US-lessac-medium".into());
        old.save().unwrap();
        let up = Registry::load();
        assert!(!up.voices.contains_key("piper-en_US-lessac-medium"), "stale bundled voice dropped");
        assert_eq!(up.voice["en"], BUNDLED_PHONON);
        assert!(up.voices[BUNDLED_VOICE].model.ends_with("en_US-lessac-low.onnx"));
        assert!(up.voices[BUNDLED_PHONON].bundled && up.voices[BUNDLED_PHONON].kind == "phonon");

        // an image before 2.5.1 has no Phonon of its own: the one installed from the catalog
        // (same id) stays, it is not replaced by the image's entry that points nowhere
        let own = dir.join("models").join(BUNDLED_PHONON);
        std::fs::create_dir_all(&own).unwrap();
        let mut inst = Registry::load();
        inst.voices.insert(
            BUNDLED_PHONON.into(),
            Voice { kind: "phonon".into(), langs: vec!["en".into()], dir: own.to_string_lossy().into(), ..Default::default() },
        );
        inst.save().unwrap();
        let kept = Registry::load();
        assert!(!kept.voices[BUNDLED_PHONON].bundled, "the installed Phonon voice is kept");
        assert_eq!(kept.voices[BUNDLED_PHONON].dir, own.to_string_lossy());
        // saved by mistake as the image's (which this image lacks): the installed copy in the
        // models folder is found again
        let models_own = crate::paths::models().join(BUNDLED_PHONON);
        std::fs::create_dir_all(&models_own).unwrap();
        for f in ["config.json", "tokenizer.json", "model.q8.gguf"] {
            std::fs::write(models_own.join(f), b"x").unwrap();
        }
        let mut wrong = Registry::load();
        wrong.voices.insert(BUNDLED_PHONON.into(), Registry::default().voices[BUNDLED_PHONON].clone());
        wrong.save().unwrap();
        let fixed = Registry::load();
        assert!(!fixed.voices[BUNDLED_PHONON].bundled);
        assert_eq!(fixed.voices[BUNDLED_PHONON].dir, models_own.to_string_lossy());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
