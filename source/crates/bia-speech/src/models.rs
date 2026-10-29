//! Loaded sherpa models, kept for the life of the process (one copy each, behind a
//! mutex): Whisper recognizers (per model id), Whisper language identification,
//! Silero VAD instances (per stream), GTCRN denoiser and the speaker embedder.

use crate::registry::{self, Asr, Registry};
use crate::sherpa::{self, cstr_to_string, Strs, H};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

fn threads() -> i32 {
    std::env::var("BIA_STT_THREADS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| std::thread::available_parallelism().map(|n| n.get().min(4) as i32).unwrap_or(2))
}

fn need(path: &str, what: &str) -> Result<(), String> {
    if path.is_empty() || !Path::new(path).exists() {
        return Err(format!("{what} missing at {path:?}"));
    }
    Ok(())
}

/// Lower this thread's priority: transcription must not starve the call audio pacer.
pub fn nice_thread() {
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, 0, 10);
    }
}

// ---- recognizer ----

pub struct Recognizer {
    h: H,
    asr: Asr,
}

unsafe impl Send for Recognizer {}

impl Recognizer {
    fn config<'a>(asr: &Asr, language: &str, s: &'a mut Strs) -> sherpa::OfflineRecognizerCfg {
        let mut cfg: sherpa::OfflineRecognizerCfg = unsafe { std::mem::zeroed() };
        cfg.feat_config.sample_rate = 16_000;
        cfg.feat_config.feature_dim = 80;
        if asr.kind == "nemo_transducer" {
            cfg.model_config.transducer.encoder = s.p(&asr.encoder);
            cfg.model_config.transducer.decoder = s.p(&asr.decoder);
            cfg.model_config.transducer.joiner = s.p(&asr.joiner);
            cfg.model_config.model_type = s.p("nemo_transducer");
        } else {
            cfg.model_config.whisper.encoder = s.p(&asr.encoder);
            cfg.model_config.whisper.decoder = s.p(&asr.decoder);
            cfg.model_config.whisper.language = s.p(language);
            cfg.model_config.whisper.task = s.p("transcribe");
        }
        cfg.model_config.tokens = s.p(&asr.tokens);
        cfg.model_config.num_threads = threads();
        cfg.model_config.provider = s.p("cpu");
        cfg.decoding_method = s.p("greedy_search");
        cfg
    }

    fn load(asr: &Asr) -> Result<Self, String> {
        if asr.kind != "whisper" && asr.kind != "nemo_transducer" {
            return Err(format!("unsupported speech model kind {}", asr.kind));
        }
        need(&asr.encoder, "speech model encoder")?;
        need(&asr.decoder, "speech model decoder")?;
        need(&asr.tokens, "speech model tokens")?;
        if asr.kind == "nemo_transducer" {
            need(&asr.joiner, "speech model joiner")?;
        }
        let api = sherpa::api()?;
        let mut s = Strs::default();
        let cfg = Self::config(asr, "", &mut s);
        let h = unsafe { (api.create_recognizer)(&cfg) };
        if h.is_null() {
            return Err(format!("could not load speech model {}", asr.dir));
        }
        Ok(Self { h, asr: asr.clone() })
    }

    /// Whisper (detects the language, can be told one); a transducer has neither.
    pub fn is_whisper(&self) -> bool {
        self.asr.kind != "nemo_transducer"
    }

    /// Languages it transcribes; empty = all Whisper knows.
    pub fn langs(&self) -> &[String] {
        &self.asr.langs
    }

    /// Transcribe 16 kHz mono with the language forced ("" = Whisper detects it).
    pub fn decode(&self, samples: &[f32], language: &str) -> Result<String, String> {
        self.decode_lang(samples, language).map(|(text, _)| text)
    }

    /// [`Recognizer::decode`] and the language Whisper decoded in (its own detection
    /// when `language` is ""): one encoder pass gives both.
    pub fn decode_lang(&self, samples: &[f32], language: &str) -> Result<(String, String), String> {
        let api = sherpa::api()?;
        let mut s = Strs::default();
        let cfg = Self::config(&self.asr, language, &mut s);
        unsafe {
            if self.is_whisper() {
                (api.recognizer_set_config)(self.h, &cfg);
            }
            let stream = (api.create_offline_stream)(self.h);
            if stream.is_null() {
                return Err("could not create recognizer stream".into());
            }
            (api.accept_waveform_offline)(stream, 16_000, samples.as_ptr(), samples.len() as i32);
            (api.decode_offline_stream)(self.h, stream);
            let r = (api.get_offline_result)(stream);
            let text = if r.is_null() { String::new() } else { cstr_to_string((*r).text) };
            let lang = if r.is_null() { String::new() } else { cstr_to_string((*r).lang) };
            if !r.is_null() {
                (api.destroy_offline_result)(r);
            }
            (api.destroy_offline_stream)(stream);
            // "<|de|>" from some sherpa builds, "de" from others
            let lang = lang.trim().trim_start_matches("<|").trim_end_matches("|>").to_string();
            let lang = if lang.is_empty() && self.is_whisper() { language.to_string() } else { lang };
            Ok((text.trim().to_string(), lang))
        }
    }
}

impl Drop for Recognizer {
    fn drop(&mut self) {
        if let Ok(api) = sherpa::api() {
            unsafe { (api.destroy_recognizer)(self.h) };
        }
    }
}

// ---- language identification ----

pub struct Slid {
    h: H,
}

unsafe impl Send for Slid {}

impl Slid {
    fn load(asr: &Asr) -> Result<Self, String> {
        need(&asr.encoder, "whisper encoder")?;
        need(&asr.decoder, "whisper decoder")?;
        let api = sherpa::api()?;
        let mut s = Strs::default();
        let mut cfg: sherpa::SlidCfg = unsafe { std::mem::zeroed() };
        cfg.whisper.encoder = s.p(&asr.encoder);
        cfg.whisper.decoder = s.p(&asr.decoder);
        cfg.num_threads = threads();
        cfg.provider = s.p("cpu");
        let h = unsafe { (api.create_slid)(&cfg) };
        if h.is_null() {
            return Err("could not load language identification".into());
        }
        Ok(Self { h })
    }

    pub fn detect(&self, samples: &[f32]) -> Result<String, String> {
        let api = sherpa::api()?;
        unsafe {
            let stream = (api.slid_create_stream)(self.h);
            if stream.is_null() {
                return Err("could not create language stream".into());
            }
            (api.accept_waveform_offline)(stream, 16_000, samples.as_ptr(), samples.len() as i32);
            let r = (api.slid_compute)(self.h, stream);
            let lang = if r.is_null() { String::new() } else { cstr_to_string((*r).lang) };
            if !r.is_null() {
                (api.destroy_slid_result)(r);
            }
            (api.destroy_offline_stream)(stream);
            Ok(lang.trim().to_string())
        }
    }
}

impl Drop for Slid {
    fn drop(&mut self) {
        if let Ok(api) = sherpa::api() {
            unsafe { (api.destroy_slid)(self.h) };
        }
    }
}

// ---- VAD ----

/// One Silero VAD instance: stateful, one per audio stream.
pub struct Vad {
    h: H,
    pub min_silence: f32,
}

unsafe impl Send for Vad {}

pub struct Segment {
    /// Sample index (16 kHz) of the first sample, counted from the stream start.
    pub start: usize,
    pub samples: Vec<f32>,
}

impl Vad {
    /// `min_silence`: pause (s) that ends a segment. `max_speech`: forced split (s).
    pub fn new(reg: &Registry, min_silence: f32, max_speech: f32) -> Result<Self, String> {
        let model = reg.extra("silero-vad").ok_or("Silero VAD model missing")?;
        let api = sherpa::api()?;
        let mut s = Strs::default();
        let mut cfg: sherpa::VadCfg = unsafe { std::mem::zeroed() };
        cfg.silero_vad.model = s.p(model);
        cfg.silero_vad.threshold = std::env::var("BIA_VAD_THRESHOLD").ok().and_then(|v| v.parse().ok()).unwrap_or(0.5);
        cfg.silero_vad.min_silence_duration = min_silence;
        cfg.silero_vad.min_speech_duration = 0.25;
        cfg.silero_vad.window_size = 512;
        cfg.silero_vad.max_speech_duration = max_speech;
        cfg.sample_rate = 16_000;
        cfg.num_threads = 1;
        cfg.provider = s.p("cpu");
        let h = unsafe { (api.create_vad)(&cfg, 60.0) };
        if h.is_null() {
            return Err("could not load Silero VAD".into());
        }
        Ok(Self { h, min_silence })
    }

    /// Feed 16 kHz samples (any length). Fed one window at a time and drained after
    /// each: given a long buffer in one call, sherpa keeps only the last of several
    /// segments that end inside it (a short first phrase was lost).
    pub fn accept(&mut self, samples: &[f32]) -> Vec<Segment> {
        let Ok(api) = sherpa::api() else { return vec![] };
        let mut out = Vec::new();
        for w in samples.chunks(512) {
            unsafe { (api.vad_accept)(self.h, w.as_ptr(), w.len() as i32) };
            out.extend(self.drain());
        }
        out
    }

    pub fn speaking(&self) -> bool {
        sherpa::api().map(|api| unsafe { (api.vad_detected)(self.h) } != 0).unwrap_or(false)
    }

    /// End of input: return the segment still open, if any.
    pub fn flush(&mut self) -> Vec<Segment> {
        let Ok(api) = sherpa::api() else { return vec![] };
        unsafe { (api.vad_flush)(self.h) };
        self.drain()
    }

    pub fn reset(&mut self) {
        if let Ok(api) = sherpa::api() {
            unsafe { (api.vad_reset)(self.h) };
        }
    }

    fn drain(&mut self) -> Vec<Segment> {
        let Ok(api) = sherpa::api() else { return vec![] };
        let mut out = Vec::new();
        unsafe {
            while (api.vad_empty)(self.h) == 0 {
                let seg = (api.vad_front)(self.h);
                if !seg.is_null() {
                    let s = &*seg;
                    if !s.samples.is_null() && s.n > 0 {
                        out.push(Segment {
                            start: s.start.max(0) as usize,
                            samples: std::slice::from_raw_parts(s.samples, s.n as usize).to_vec(),
                        });
                    }
                    (api.destroy_segment)(seg);
                }
                (api.vad_pop)(self.h);
            }
        }
        out
    }
}

impl Drop for Vad {
    fn drop(&mut self) {
        if let Ok(api) = sherpa::api() {
            unsafe { (api.destroy_vad)(self.h) };
        }
    }
}

/// All speech segments of a whole recording.
pub fn segments(reg: &Registry, samples: &[f32], min_silence: f32) -> Result<Vec<Segment>, String> {
    let mut vad = Vad::new(reg, min_silence, 20.0)?;
    let mut out = Vec::new();
    for chunk in samples.chunks(16_000) {
        out.extend(vad.accept(chunk));
    }
    out.extend(vad.flush());
    Ok(out)
}

// ---- denoiser ----

pub struct Denoiser {
    h: H,
}

unsafe impl Send for Denoiser {}

impl Denoiser {
    fn load(path: &str) -> Result<Self, String> {
        let api = sherpa::api()?;
        let mut s = Strs::default();
        let mut cfg: sherpa::DenoiserCfg = unsafe { std::mem::zeroed() };
        cfg.model.gtcrn.model = s.p(path);
        cfg.model.num_threads = 1;
        cfg.model.provider = s.p("cpu");
        let h = unsafe { (api.create_denoiser)(&cfg) };
        if h.is_null() {
            return Err("could not load GTCRN denoiser".into());
        }
        Ok(Self { h })
    }

    pub fn run(&self, samples: &[f32]) -> Result<Vec<f32>, String> {
        let api = sherpa::api()?;
        unsafe {
            let a = (api.denoiser_run)(self.h, samples.as_ptr(), samples.len() as i32, 16_000);
            if a.is_null() {
                return Err("denoiser failed".into());
            }
            let out = std::slice::from_raw_parts((*a).samples, (*a).n.max(0) as usize).to_vec();
            let rate = (*a).sample_rate as u32;
            (api.destroy_denoised)(a);
            Ok(crate::audio::resample(&out, rate, 16_000))
        }
    }
}

impl Drop for Denoiser {
    fn drop(&mut self) {
        if let Ok(api) = sherpa::api() {
            unsafe { (api.destroy_denoiser)(self.h) };
        }
    }
}

// ---- speaker embedding ----

pub struct Speaker {
    h: H,
}

unsafe impl Send for Speaker {}

impl Speaker {
    fn load(path: &str) -> Result<Self, String> {
        let api = sherpa::api()?;
        let mut s = Strs::default();
        let mut cfg: sherpa::SpeakerCfg = unsafe { std::mem::zeroed() };
        cfg.model = s.p(path);
        cfg.num_threads = 1;
        cfg.provider = s.p("cpu");
        let h = unsafe { (api.create_speaker)(&cfg) };
        if h.is_null() {
            return Err("could not load speaker model".into());
        }
        Ok(Self { h })
    }

    pub fn embed(&self, samples: &[f32]) -> Result<Vec<f32>, String> {
        let api = sherpa::api()?;
        unsafe {
            let dim = (api.speaker_dim)(self.h).max(0) as usize;
            let stream = (api.speaker_create_stream)(self.h);
            if stream.is_null() {
                return Err("could not create speaker stream".into());
            }
            (api.online_accept_waveform)(stream, 16_000, samples.as_ptr(), samples.len() as i32);
            (api.online_input_finished)(stream);
            if (api.speaker_is_ready)(self.h, stream) == 0 {
                (api.destroy_online_stream)(stream);
                return Err("too little speech for a voice print".into());
            }
            let v = (api.speaker_compute)(self.h, stream);
            let out = if v.is_null() { vec![] } else { std::slice::from_raw_parts(v, dim).to_vec() };
            if !v.is_null() {
                (api.speaker_destroy_embedding)(v);
            }
            (api.destroy_online_stream)(stream);
            Ok(out)
        }
    }
}

impl Drop for Speaker {
    fn drop(&mut self) {
        if let Ok(api) = sherpa::api() {
            unsafe { (api.destroy_speaker)(self.h) };
        }
    }
}

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

// ---- process-wide cache ----

type Shared<T> = Arc<Mutex<T>>;

#[derive(Default)]
struct Cache {
    /// Model id + encoder path → recognizer (at most two kept).
    recognizers: Vec<(String, Shared<Recognizer>)>,
    slid: Option<(String, Shared<Slid>)>,
    denoiser: Option<(String, Shared<Denoiser>)>,
    speaker: Option<(String, Shared<Speaker>)>,
}

fn cache() -> &'static Mutex<Cache> {
    static C: OnceLock<Mutex<Cache>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(Cache::default()))
}

/// Recognizer for model `id` (falls back to the bundled whisper-tiny if missing).
pub fn recognizer(reg: &Registry, id: &str) -> Result<Shared<Recognizer>, String> {
    let (id, asr) = match reg.asr.get(id) {
        Some(a) => (id, a),
        None => (registry::BUNDLED_ASR, reg.asr.get(registry::BUNDLED_ASR).ok_or("no speech model")?),
    };
    let key = format!("{id}|{}", asr.encoder);
    let mut c = cache().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(i) = c.recognizers.iter().position(|(k, _)| *k == key) {
        let hit = c.recognizers.remove(i);
        let r = hit.1.clone();
        c.recognizers.push(hit);
        return Ok(r);
    }
    let r = Arc::new(Mutex::new(Recognizer::load(asr)?));
    c.recognizers.push((key, r.clone()));
    if c.recognizers.len() > 2 {
        c.recognizers.remove(0);
    }
    Ok(r)
}

/// The most accurate Whisper installed (for what a transducer such as Parakeet does
/// not know): small, then base, then the bundled tiny.
pub fn best_whisper(reg: &Registry) -> &str {
    ["whisper-small", "whisper-base"]
        .into_iter()
        .find(|id| reg.asr.get(*id).is_some_and(|a| a.kind == "whisper"))
        .unwrap_or(registry::BUNDLED_ASR)
}

/// Language identification for calls uses the bundled whisper-tiny: fast, and it
/// knows every language, so detection works for languages that are not installed.
pub fn slid(reg: &Registry) -> Result<Shared<Slid>, String> {
    slid_for(reg, registry::BUNDLED_ASR)
}

/// Language identification with Whisper model `id` (voice notes use the bigger
/// voice-note model: tiny mixes up close languages such as Russian and Ukrainian).
pub fn slid_for(reg: &Registry, id: &str) -> Result<Shared<Slid>, String> {
    let asr = reg
        .asr
        .get(id)
        .filter(|a| a.kind == "whisper")
        .or_else(|| reg.asr.get(registry::BUNDLED_ASR))
        .ok_or("no whisper model")?;
    let mut c = cache().lock().unwrap_or_else(|e| e.into_inner());
    if let Some((k, s)) = &c.slid {
        if *k == asr.encoder {
            return Ok(s.clone());
        }
    }
    let s = Arc::new(Mutex::new(Slid::load(asr)?));
    c.slid = Some((asr.encoder.clone(), s.clone()));
    Ok(s)
}

pub fn denoiser(reg: &Registry) -> Option<Shared<Denoiser>> {
    let path = reg.extra("gtcrn")?.to_string();
    let mut c = cache().lock().unwrap_or_else(|e| e.into_inner());
    if let Some((k, d)) = &c.denoiser {
        if *k == path {
            return Some(d.clone());
        }
    }
    let d = Arc::new(Mutex::new(Denoiser::load(&path).ok()?));
    c.denoiser = Some((path, d.clone()));
    Some(d)
}

pub fn speaker(reg: &Registry) -> Result<Shared<Speaker>, String> {
    let path = reg
        .extra("speaker-campplus")
        .ok_or("speaker model not installed (lang_settings(speaker_check=true) installs it)")?
        .to_string();
    let mut c = cache().lock().unwrap_or_else(|e| e.into_inner());
    if let Some((k, s)) = &c.speaker {
        if *k == path {
            return Ok(s.clone());
        }
    }
    let s = Arc::new(Mutex::new(Speaker::load(&path)?));
    c.speaker = Some((path, s.clone()));
    Ok(s)
}

/// Load the call-path models in the background so the first utterance is fast.
pub fn warm() {
    std::thread::spawn(|| {
        let reg = registry::current();
        let _ = slid(&reg);
        let _ = recognizer(&reg, &reg.asr_call);
        let _ = denoiser(&reg);
    });
}
