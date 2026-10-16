//! The language registry (/home/bia/files/speech/languages.json): which languages are
//! enabled, the voice per language, installed models and speech settings. bia-mcp-lang
//! writes it (atomically); the telegram/whatsapp MCPs re-read it when its mtime changes.

use crate::paths;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

pub const BUNDLED_VOICE: &str = "piper-en_US-lessac-medium";
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
    /// whisper
    pub kind: String,
    /// Empty: every language Whisper knows.
    pub langs: Vec<String>,
    pub dir: String,
    pub encoder: String,
    pub decoder: String,
    pub tokens: String,
    pub bundled: bool,
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
            asr_denoised: false,
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
                model: s(&piper.join("en_US-lessac-medium.onnx")),
                tokens: s(&piper.join("tokens.txt")),
                data_dir: s(&piper.join("espeak-ng-data")),
                bundled: true,
                ..Default::default()
            },
        );
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
            },
        );
        let mut extras = BTreeMap::new();
        extras.insert("silero-vad".into(), s(&b.join("silero-vad/silero_vad.onnx")));
        extras.insert("gtcrn".into(), s(&b.join("gtcrn/gtcrn_simple.onnx")));
        Self {
            default_lang: "en".into(),
            enabled: vec!["en".into()],
            ignored: vec![],
            voice: [("en".to_string(), BUNDLED_VOICE.to_string())].into(),
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
        for (k, v) in d.voices {
            r.voices.insert(k, v);
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
        let id = self.voice.get(&lang.code)?;
        self.voices.get(id).map(|v| (id.as_str(), v))
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
        assert_eq!(r.voice["en"], BUNDLED_VOICE);
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
        let _ = std::fs::remove_dir_all(&dir);
    }
}
