//! Text to speech with a voice per language. `lang` is required: the text is split
//! into `<lang=xx>…</lang>` spans, every span's voice is checked before anything is
//! spoken, and each span is synthesized with its voice (loaded once, up to three kept).

use crate::langs::{self, Span};
use crate::registry::{self, Registry, Voice};
use crate::sherpa::{self, Strs, H};
use std::ffi::{c_void, CString};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

pub struct Engine {
    h: H,
    pub sample_rate: u32,
    sid: i32,
    speed: f32,
}

unsafe impl Send for Engine {}

impl Drop for Engine {
    fn drop(&mut self) {
        if let Ok(api) = sherpa::api() {
            unsafe { (api.destroy_tts)(self.h) };
        }
    }
}

fn threads() -> i32 {
    std::env::var("BIA_TTS_THREADS").ok().and_then(|v| v.parse().ok()).unwrap_or(2)
}

impl Engine {
    pub fn load(v: &Voice) -> Result<Self, String> {
        let api = sherpa::api()?;
        let mut required = vec![("model", &v.model), ("tokens", &v.tokens)];
        if v.kind == "kokoro" {
            required.push(("voices", &v.voices));
        }
        for (label, p) in required {
            if p.is_empty() || !Path::new(p).exists() {
                return Err(format!("voice {label} missing at {p:?}"));
            }
        }
        let mut s = Strs::default();
        let mut cfg: sherpa::TtsCfg = unsafe { std::mem::zeroed() };
        cfg.model.num_threads = threads();
        cfg.model.provider = s.p("cpu");
        // One sentence per callback: first audio is ready after the first sentence.
        cfg.max_num_sentences = 1;
        cfg.silence_scale = 0.2;
        match v.kind.as_str() {
            "vits" => {
                let c = &mut cfg.model.vits;
                c.model = s.p(&v.model);
                c.tokens = s.p(&v.tokens);
                c.data_dir = s.p(&v.data_dir);
                c.lexicon = s.p(&v.lexicon);
                c.dict_dir = s.p(&v.dict_dir);
                c.noise_scale = 0.667;
                c.noise_scale_w = 0.8;
                c.length_scale = 1.0;
            }
            "kokoro" => {
                let c = &mut cfg.model.kokoro;
                c.model = s.p(&v.model);
                c.voices = s.p(&v.voices);
                c.tokens = s.p(&v.tokens);
                c.data_dir = s.p(&v.data_dir);
                c.lexicon = s.p(&v.lexicon);
                c.dict_dir = s.p(&v.dict_dir);
                c.length_scale = 1.0;
            }
            other => return Err(format!("unsupported voice kind {other}")),
        }
        let h = unsafe { (api.create_tts)(&cfg) };
        if h.is_null() {
            return Err(format!("could not load voice {}", v.model));
        }
        let sample_rate = unsafe { (api.tts_sample_rate)(h) }.max(0) as u32;
        Ok(Self {
            h,
            sample_rate,
            sid: v.sid,
            speed: std::env::var("BIA_TTS_SPEED").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0),
        })
    }

    /// Stream `text`; `on_chunk(samples, rate)` returns false to stop. Returns the
    /// number of samples produced.
    pub fn generate(&self, text: &str, on_chunk: &mut dyn FnMut(&[f32], u32) -> bool) -> Result<usize, String> {
        let api = sherpa::api()?;
        let c_text = CString::new(text.replace('\0', " ")).map_err(|e| e.to_string())?;
        let mut ctx = ChunkCtx { on_chunk, rate: self.sample_rate, produced: 0 };
        let audio = unsafe {
            (api.tts_generate)(
                self.h,
                c_text.as_ptr(),
                self.sid,
                self.speed,
                on_chunk_c,
                &mut ctx as *mut ChunkCtx as *mut c_void,
            )
        };
        if !audio.is_null() {
            unsafe { (api.destroy_generated)(audio) };
        }
        Ok(ctx.produced)
    }
}

struct ChunkCtx<'a> {
    on_chunk: &'a mut dyn FnMut(&[f32], u32) -> bool,
    rate: u32,
    produced: usize,
}

/// Returns 1 to continue, 0 to stop.
extern "C" fn on_chunk_c(samples: *const f32, n: i32, arg: *mut c_void) -> i32 {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let ctx = unsafe { &mut *(arg as *mut ChunkCtx) };
        if samples.is_null() || n <= 0 {
            return 1;
        }
        let chunk = unsafe { std::slice::from_raw_parts(samples, n as usize) };
        ctx.produced += chunk.len();
        i32::from((ctx.on_chunk)(chunk, ctx.rate))
    }))
    .unwrap_or(0)
}

type Shared = Arc<Mutex<Engine>>;

fn engines() -> &'static Mutex<Vec<(String, Shared)>> {
    static E: OnceLock<Mutex<Vec<(String, Shared)>>> = OnceLock::new();
    E.get_or_init(|| Mutex::new(Vec::new()))
}

/// Loaded engine for a voice (LRU, three kept).
pub fn engine(id: &str, v: &Voice) -> Result<Shared, String> {
    let key = format!("{id}|{}", v.model);
    let mut e = engines().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(i) = e.iter().position(|(k, _)| *k == key) {
        let hit = e.remove(i);
        let r = hit.1.clone();
        e.push(hit);
        return Ok(r);
    }
    let eng = Arc::new(Mutex::new(Engine::load(v)?));
    e.push((key, eng.clone()));
    if e.len() > 3 {
        e.remove(0);
    }
    Ok(eng)
}

pub fn missing_voice(code: &str) -> String {
    format!(
        "no voice installed for {code} ({}): call lang_install(lang=\"{code}\") (lang MCP) first, or speak in an installed language",
        langs::name(code)
    )
}

/// Spans of `text` with the voice for each; fails before anything is spoken when a
/// language is unknown or has no voice.
pub fn plan(reg: &Registry, text: &str, lang: &str) -> Result<Vec<(Span, String, Voice)>, String> {
    let base = langs::parse(lang).ok_or_else(|| {
        if lang.trim().is_empty() {
            "lang is required: the language of the text, e.g. \"en\" or \"ru\" (no preferred language is known for this person; addressbook_upsert(lang=…) sets one)".to_string()
        } else {
            format!("unknown lang {lang:?}; use a code such as \"en\", \"ru\", \"es-MX\"")
        }
    })?;
    let spans = langs::spans(text, &base.code)?;
    if spans.is_empty() {
        return Err("text is empty".into());
    }
    spans
        .into_iter()
        .map(|sp| {
            let want = if sp.lang == base.code { base.clone() } else { langs::Lang { code: sp.lang.clone(), region: None } };
            let (id, v) = reg.voice_for(&want).ok_or_else(|| missing_voice(&sp.lang))?;
            Ok((sp, id.to_string(), v.clone()))
        })
        .collect()
}

pub struct Spoken {
    pub first_ms: u128,
    pub samples: usize,
}

/// Speak `text` in `lang`, streaming chunks to `on_chunk(samples, rate)`.
pub fn speak(text: &str, lang: &str, on_chunk: &mut dyn FnMut(&[f32], u32) -> bool) -> Result<Spoken, String> {
    let reg = registry::current();
    let plan = plan(&reg, text, lang)?;
    let started = Instant::now();
    let mut first_ms = None;
    let mut total = 0;
    for (span, id, voice) in plan {
        let eng = engine(&id, &voice)?;
        let eng = eng.lock().map_err(|_| "voice lock")?;
        let mut live = true;
        let mut cb = |c: &[f32], r: u32| {
            first_ms.get_or_insert_with(|| started.elapsed().as_millis());
            live = on_chunk(c, r);
            live
        };
        total += eng.generate(&span.text, &mut cb)?;
        if !live {
            break;
        }
    }
    if total == 0 {
        return Err("TTS produced empty audio".into());
    }
    Ok(Spoken { first_ms: first_ms.unwrap_or(0), samples: total })
}

/// Whole utterance as one buffer (voice notes, previews), at the first voice's rate.
pub fn synthesize(text: &str, lang: &str) -> Result<(Vec<f32>, u32), String> {
    let mut out = Vec::new();
    let mut rate = 0u32;
    speak(text, lang, &mut |c, r| {
        if rate == 0 {
            rate = r;
        }
        if r == rate {
            out.extend_from_slice(c);
        } else {
            out.extend(crate::audio::resample(c, r, rate));
        }
        // a short pause between sentences
        out.extend(std::iter::repeat(0.0).take((rate / 10) as usize));
        true
    })?;
    Ok((out, rate))
}

/// Synthesize into an OGG/Opus voice note; returns (path, seconds, samples).
pub fn voice_note(text: &str, lang: &str, out: &Path) -> Result<(f32, Vec<f32>), String> {
    let (samples, rate) = synthesize(text, lang)?;
    crate::audio::encode_voice_note(&samples, rate, out)?;
    Ok((samples.len() as f32 / rate.max(1) as f32, samples))
}

/// Load the voices of the enabled languages in the background.
pub fn warm() {
    std::thread::spawn(|| {
        let reg = registry::current();
        for l in &reg.enabled {
            if let Some((id, v)) = reg.voice_for(&langs::Lang { code: l.clone(), region: None }) {
                let _ = engine(id, v);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_requires_lang_and_voices() {
        let reg = Registry::default();
        assert!(plan(&reg, "hello", "").unwrap_err().contains("lang is required"));
        assert!(plan(&reg, "hello", "xx").unwrap_err().contains("unknown lang"));
        let p = plan(&reg, "hello", "en").unwrap();
        assert_eq!(p[0].1, registry::BUNDLED_VOICE);
        let e = plan(&reg, "hello <lang=es>hola</lang>", "en").unwrap_err();
        assert!(e.contains("lang_install(lang=\"es\")"), "{e}");
        assert!(plan(&reg, "  ", "en").is_err());
    }
}
