//! Text to speech with a voice per language. `lang` is required: the text is split
//! into `<lang=xx>…</lang>` spans, every span's voice is checked before anything is
//! spoken, and each span is synthesized with its voice (loaded once, up to three kept
//! while in use; see `idle`).

use crate::langs::{self, Span};
use crate::registry::{self, Registry, Voice};
use crate::sherpa::{self, Strs, H};
use std::ffi::{c_void, CString};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

/// A loaded voice: a sherpa-onnx one (Piper, Kokoro) or Gradium Phonon, which streams
/// 80 ms pieces as it makes them (the first sound comes long before the sentence is done).
pub struct Engine {
    inner: Inner,
    pub sample_rate: u32,
}

enum Inner {
    Sherpa { h: H, sid: i32, speed: f32 },
    Phonon(Box<ptts::synth::Synth>),
}

unsafe impl Send for Engine {}

impl Drop for Engine {
    fn drop(&mut self) {
        if let Inner::Sherpa { h, .. } = &self.inner {
            if let Ok(api) = sherpa::api() {
                unsafe { (api.destroy_tts)(*h) };
            }
        }
    }
}

/// Phonon's text rules (numbers, dates) for a voice language; the five it speaks.
fn phonon_lang(code: &str) -> Result<ptts::preprocess::Lang, String> {
    use ptts::preprocess::Lang;
    Ok(match code {
        "en" => Lang::En,
        "fr" => Lang::Fr,
        "de" => Lang::De,
        "es" => Lang::Es,
        "pt" => Lang::Pt,
        other => return Err(format!("Phonon does not speak {other}")),
    })
}

/// Two: on a phone (Snapdragon 660) four were only ~5% faster for a Piper voice, and
/// took the cores the call's own audio needs (playback underran: clicks in the call).
/// Half the cores, 2 to 4 (`BIA_TTS_THREADS` overrides): on a Snapdragon 660 Phonon makes
/// speech about twice as fast with 4 threads as with 2, which with 2 barely kept up with
/// playing it (holes in a call).
fn threads() -> i32 {
    std::env::var("BIA_TTS_THREADS").ok().and_then(|v| v.parse().ok()).unwrap_or_else(default_threads)
}

fn default_threads() -> i32 {
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2) as i32;
    (cores / 2).clamp(2, 4)
}

impl Engine {
    pub fn load(v: &Voice) -> Result<Self, String> {
        Self::load_for(v, v.langs.first().map(String::as_str).unwrap_or("en"))
    }

    /// Load `v` to speak `lang`: Phonon reads numbers, dates and the like by the rules of the
    /// language spoken (a multilingual checkpoint speaks several), the others ignore it.
    pub fn load_for(v: &Voice, lang: &str) -> Result<Self, String> {
        if v.kind == "phonon" {
            return Self::load_phonon(v, lang);
        }
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
            inner: Inner::Sherpa {
                h,
                sid: v.sid,
                speed: std::env::var("BIA_TTS_SPEED").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0),
            },
            sample_rate,
        })
    }

    /// A Phonon checkpoint folder (config.json, tokenizer.json, model.q8.gguf, the
    /// default voice), 8-bit weights on the CPU.
    fn load_phonon(v: &Voice, lang: &str) -> Result<Self, String> {
        use ptts::checkpoint::{Checkpoint, ResolveOptions};
        use ptts::synth::{DeviceKind, Quant};
        static THREADS: std::sync::Once = std::sync::Once::new();
        THREADS.call_once(|| xn::set_num_threads(threads().max(1) as usize));
        if !v.langs.iter().any(|l| l == lang) {
            return Err(format!("this Phonon voice speaks {}, not {lang}", v.langs.join(", ")));
        }
        let lang = phonon_lang(lang)?;
        let ck = Checkpoint::resolve(&v.dir, ResolveOptions { quant: Quant::Q80, weights: Some("model.q8.gguf") })
            .map_err(|e| format!("Phonon voice at {}: {e}", v.dir))?;
        let mut synth = ck.builder(lang).device(DeviceKind::Cpu).build().map_err(|e| format!("Phonon voice at {}: {e}", v.dir))?;
        ck.register_voices(&mut synth);
        let sample_rate = synth.sample_rate();
        Ok(Self { inner: Inner::Phonon(Box::new(synth)), sample_rate })
    }

    /// Stream `text`; `on_chunk(samples, rate)` returns false to stop. Returns the
    /// number of samples produced.
    pub fn generate(&self, text: &str, on_chunk: &mut dyn FnMut(&[f32], u32) -> bool) -> Result<usize, String> {
        let (h, sid, speed) = match &self.inner {
            Inner::Sherpa { h, sid, speed } => (*h, *sid, *speed),
            Inner::Phonon(synth) => {
                let mut produced = 0;
                for chunk in synth.stream(text).map_err(|e| format!("Phonon: {e}"))? {
                    let chunk = chunk.map_err(|e| format!("Phonon: {e}"))?;
                    produced += chunk.len();
                    if !on_chunk(&chunk, self.sample_rate) {
                        break;
                    }
                }
                return Ok(produced);
            }
        };
        let api = sherpa::api()?;
        let c_text = CString::new(text.replace('\0', " ")).map_err(|e| e.to_string())?;
        let mut ctx = ChunkCtx { on_chunk, rate: self.sample_rate, produced: 0 };
        let audio = unsafe {
            (api.tts_generate)(
                h,
                c_text.as_ptr(),
                sid,
                speed,
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

/// Loaded engine for a voice speaking `lang` (LRU, three kept). A Phonon voice is loaded
/// per language (its text rules); the others once.
pub fn engine(id: &str, v: &Voice, lang: &str) -> Result<Shared, String> {
    crate::idle::touch();
    let key = if v.kind == "phonon" { format!("{id}|{}|{lang}", v.model) } else { format!("{id}|{}", v.model) };
    let hit = |e: &mut Vec<(String, Shared)>| {
        let i = e.iter().position(|(k, _)| *k == key)?;
        let hit = e.remove(i);
        let r = hit.1.clone();
        e.push(hit);
        Some(r)
    };
    if let Some(r) = hit(&mut engines().lock().unwrap_or_else(|e| e.into_inner())) {
        return Ok(r);
    }
    // loaded without the list's lock: a voice loading (seconds) never holds up one that is
    // already loaded and about to speak (a call's greeting)
    let loaded = Arc::new(Mutex::new(Engine::load_for(v, lang)?));
    let mut e = engines().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(r) = hit(&mut e) {
        return Ok(r);
    }
    let eng = loaded;
    e.push((key, eng.clone()));
    if e.len() > 3 {
        e.remove(0);
    }
    Ok(eng)
}

/// Drop every loaded voice that is not speaking right now; returns how many.
pub(crate) fn unload_unused() -> usize {
    let mut e = engines().lock().unwrap_or_else(|e| e.into_inner());
    let before = e.len();
    e.retain(|(_, eng)| Arc::strong_count(eng) > 1);
    before - e.len()
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
    // spoken words only: no Markdown marks, URLs or code (voice notes and every call)
    let text = crate::plain::speakable(text);
    let spans = langs::spans(&text, &base.code)?;
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
/// The audio comes a sentence at a time (Piper makes a whole sentence before any of it),
/// so a call hears the first words once the first sentence is made; a long first
/// sentence is cut at its first comma so its opening plays while the rest is made.
pub fn speak(text: &str, lang: &str, on_chunk: &mut dyn FnMut(&[f32], u32) -> bool) -> Result<Spoken, String> {
    speak_with(text, lang, true, on_chunk)
}

fn speak_with(text: &str, lang: &str, quick: bool, on_chunk: &mut dyn FnMut(&[f32], u32) -> bool) -> Result<Spoken, String> {
    let reg = registry::current();
    let plan = plan(&reg, text, lang)?;
    let started = Instant::now();
    let mut first_ms = None;
    let mut total = 0;
    for (i, (span, id, voice)) in plan.into_iter().enumerate() {
        let live = std::cell::Cell::new(true);
        let mut cb = |c: &[f32], r: u32| {
            first_ms.get_or_insert_with(|| started.elapsed().as_millis());
            live.set(on_chunk(c, r));
            live.get()
        };
        // Phonon streams by itself; a sentence-at-a-time voice gets its opening cut early
        let quick = quick && i == 0 && voice.kind != "phonon";
        let mut made = 0;
        let r = say_span(&id, &voice, &span.lang, &span.text, quick, &mut made, &live, &mut cb);
        total += made;
        if let Err(e) = r {
            // A Phonon voice that fails (a damaged download, no memory) must not leave bia
            // silent: the rest of the span is said with the language's other voice.
            let fb = (voice.kind == "phonon").then(|| fallback_voice(&reg, &span.lang)).flatten();
            let Some((fid, fv)) = fb else { return Err(e) };
            crate::diag::log("tts", format!("{id} failed ({e}); speaking with {fid}"));
            let mut more = 0;
            let rest = if made == 0 { span.text.clone() } else { String::new() };
            if !rest.is_empty() {
                say_span(&fid, &fv, &span.lang, &rest, false, &mut more, &live, &mut cb)?;
            }
            total += more;
        }
        if !live.get() {
            break;
        }
    }
    if total == 0 {
        return Err("TTS produced empty audio".into());
    }
    Ok(Spoken { first_ms: first_ms.unwrap_or(0), samples: total })
}

/// One span with one voice; `made` counts the samples given out, even when it fails.
fn say_span(
    id: &str,
    voice: &Voice,
    lang: &str,
    text: &str,
    quick: bool,
    made: &mut usize,
    live: &std::cell::Cell<bool>,
    cb: &mut dyn FnMut(&[f32], u32) -> bool,
) -> Result<(), String> {
    let eng = engine(id, voice, lang)?;
    let eng = eng.lock().map_err(|_| "voice lock")?;
    let parts = if quick { quick_start(text) } else { vec![text.to_string()] };
    for part in parts {
        let mut count = |c: &[f32], r: u32| {
            *made += c.len();
            cb(c, r)
        };
        eng.generate(&part, &mut count)?;
        if !live.get() {
            break;
        }
    }
    Ok(())
}

/// The voice that speaks `lang` when its Phonon voice fails: one of its other installed
/// voices, for English the one in the image.
fn fallback_voice(reg: &Registry, lang: &str) -> Option<(String, Voice)> {
    // only a voice whose files are there: an older image may lack the bundled one
    let usable = |v: &Voice| v.kind != "phonon" && v.langs.iter().any(|l| l == lang) && Path::new(&v.model).exists();
    if lang == "en" {
        if let Some(v) = reg.voices.get(registry::BUNDLED_VOICE).filter(|v| usable(v)) {
            return Some((registry::BUNDLED_VOICE.to_string(), v.clone()));
        }
    }
    reg.voices.iter().find(|(_, v)| usable(v)).map(|(k, v)| (k.clone(), v.clone()))
}

/// A long first sentence (over 60 characters) cut after its first comma, semicolon or
/// colon that comes after 15 characters: the opening is made (and heard) sooner.
fn quick_start(text: &str) -> Vec<String> {
    let t = text.trim_start();
    let end = t.char_indices().find(|&(i, c)| ".!?…".contains(c) && t[i + c.len_utf8()..].chars().next().is_none_or(char::is_whitespace)).map(|(i, c)| i + c.len_utf8()).unwrap_or(t.len());
    if t[..end].chars().count() <= 60 {
        return vec![text.to_string()];
    }
    let cut = t[..end].char_indices().find(|&(i, c)| ",;:".contains(c) && t[..i].chars().count() >= 15 && t[i + 1..].starts_with(' ')).map(|(i, _)| i + 1);
    match cut {
        Some(c) if t[c..].trim().chars().count() >= 10 => vec![t[..c].to_string(), t[c..].trim_start().to_string()],
        _ => vec![text.to_string()],
    }
}

/// Whole utterance as one buffer (voice notes, previews), at the first voice's rate.
pub fn synthesize(text: &str, lang: &str) -> Result<(Vec<f32>, u32), String> {
    let mut out = Vec::new();
    let mut rate = 0u32;
    // a voice note is heard whole: no early cut at a comma
    speak_with(text, lang, false, &mut |c, r| {
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

/// Warm the voice on `text` (bia's greeting, before a call is answered): synthesis runs
/// until its first audio, so the words said once the call is up start at once. Seconds.
pub fn warm_phrase(text: &str, lang: &str) -> f64 {
    let t = Instant::now();
    let _ = speak(text, lang, &mut |_, _| false);
    t.elapsed().as_secs_f64()
}

/// Load the default language's voice in the background, unless voices are loaded already.
pub fn warm() {
    std::thread::spawn(warm_now);
}

/// [`warm`], waiting for it. Only three voices stay loaded, so warming every enabled
/// language (ten on some bias) only pushed out the ones a call had just loaded for its
/// greeting, and kept the voices busy loading while the greeting waited: when a voice is
/// loaded already (a call warmed its own languages) this does nothing.
pub fn warm_now() {
    if !engines().lock().unwrap_or_else(|e| e.into_inner()).is_empty() {
        return;
    }
    let reg = registry::current();
    warm_langs(std::slice::from_ref(&reg.default_lang));
}

/// Load the voices of `langs` (a call's languages), waiting for them.
pub fn warm_langs(list: &[String]) {
    let reg = registry::current();
    for l in list {
        if let Some((id, v)) = reg.voice_for(&langs::Lang { code: l.clone(), region: None }) {
            let _ = engine(id, v, l);
        }
    }
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
        assert_eq!(p[0].1, registry::BUNDLED_PHONON);
        // English speaks with the image's Phonon voice; when it fails, with a Piper voice whose
        // files are there (the image's, else another installed one), never a missing one
        let dir = std::env::temp_dir().join(format!("bia-tts-fb-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let model = dir.join("en_US-ryan-low.onnx");
        std::fs::write(&model, b"x").unwrap();
        let mut r2 = reg.clone();
        assert!(fallback_voice(&r2, "en").is_none(), "the bundled Piper files are not on this machine");
        r2.voices.insert("piper-en_US-ryan-low".into(), Voice { kind: "vits".into(), langs: vec!["en".into()], model: model.to_string_lossy().into(), ..Default::default() });
        assert_eq!(fallback_voice(&r2, "en").unwrap().0, "piper-en_US-ryan-low");
        assert!(fallback_voice(&r2, "xx").is_none());
        let _ = std::fs::remove_dir_all(&dir);

        // what bia answers in a language is said with a voice of that language: Phonon's for
        // the ones it has, never another language's voice by a wrong mapping
        let mut r3 = Registry::default();
        for l in ["fr", "de", "es", "pt"] {
            let id = format!("phonon-40m-{l}");
            r3.voices.insert(id.clone(), Voice { kind: "phonon".into(), langs: vec![l.into()], ..Default::default() });
            r3.voice.insert(l.into(), id.clone());
            r3.enabled.push(l.into());
            assert_eq!(plan(&r3, "hola", l).unwrap()[0].1, id, "{l}");
        }
        r3.voice.insert("de".into(), registry::BUNDLED_PHONON.into());
        assert!(plan(&r3, "hallo", "de").is_err(), "an English voice never speaks German");
        let es = r3.voices["phonon-40m-es"].clone();
        assert!(Engine::load_for(&es, "fr").err().unwrap_or_default().contains("speaks es, not fr"));
        let e = plan(&reg, "hello <lang=es>hola</lang>", "en").unwrap_err();
        assert!(e.contains("lang_install(lang=\"es\")"), "{e}");
        assert!(plan(&reg, "  ", "en").is_err());
    }

    #[test]
    fn a_long_first_sentence_starts_with_its_opening() {
        let t = "Tomorrow in Madrid it will be sunny and mild, around eighteen degrees, with a breeze. Anything else?";
        assert_eq!(super::quick_start(t), vec!["Tomorrow in Madrid it will be sunny and mild,", "around eighteen degrees, with a breeze. Anything else?"]);
        assert_eq!(super::quick_start("Yes, I hear you well. Go ahead and ask me anything you like."), vec!["Yes, I hear you well. Go ahead and ask me anything you like."]);
        assert_eq!(super::quick_start("It is 3,5 degrees colder than yesterday in the whole region of Madrid today"), vec!["It is 3,5 degrees colder than yesterday in the whole region of Madrid today"]);
    }
}
