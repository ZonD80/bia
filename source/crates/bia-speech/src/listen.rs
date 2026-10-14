//! Live call speech, one `Listener` per remote participant: Silero VAD over the
//! incoming audio (resampled to 16 kHz). Segments end at short pauses; an utterance is
//! complete when the speaker has been quiet for `GAP_SECS` (or after `MAX_SECS`).
//! Transcription runs off the audio thread via `Utterance::transcribe`.

use crate::models::{self, Segment, Vad};
use crate::pipeline::{self, Heard, LangState, Opts};
use crate::registry;
use std::sync::{Arc, Mutex};

const GAP_SECS: f32 = 0.7;
const MAX_SECS: f32 = 20.0;

pub struct Listener {
    vad: Vad,
    rate: u32,
    /// Leftover input samples when resampling 48 kHz → 16 kHz in groups of three.
    carry: Vec<f32>,
    fed: usize,
    pieces: Vec<Segment>,
    state: Arc<Mutex<LangState>>,
}

pub struct Utterance {
    pieces: Vec<Segment>,
    state: Arc<Mutex<LangState>>,
}

impl Listener {
    /// `rate`: sample rate of the mono i16 audio passed to `push`.
    pub fn new(rate: u32) -> Result<Self, String> {
        Self::with_lang(rate, None)
    }

    /// Start in `lang` (the caller's preferred language, from the addressbook) instead
    /// of the default; it is enabled for hearing if it was not.
    pub fn with_lang(rate: u32, lang: Option<String>) -> Result<Self, String> {
        let start = lang.filter(|l| crate::langs::can_hear(l));
        if let Some(l) = &start {
            let _ = crate::learn::enable_hearing(l);
        }
        let reg = registry::current();
        let min_silence = if reg.settings.split_pauses { 0.25 } else { 0.5 };
        Ok(Self {
            vad: Vad::new(&reg, min_silence, MAX_SECS)?,
            rate,
            carry: Vec::new(),
            fed: 0,
            pieces: Vec::new(),
            state: Arc::new(Mutex::new(LangState::new(start.as_deref().unwrap_or(&reg.default_lang)))),
        })
    }

    fn to_16k(&mut self, pcm: &[i16]) -> Vec<f32> {
        let mut x = std::mem::take(&mut self.carry);
        x.extend(crate::audio::i16_to_f32(pcm));
        if self.rate == 48_000 {
            let keep = x.len() % 3;
            self.carry = x.split_off(x.len() - keep);
        }
        crate::audio::resample(&x, self.rate, 16_000)
    }

    /// Feed call audio; returns an utterance once the speaker paused.
    pub fn push(&mut self, pcm: &[i16]) -> Option<Utterance> {
        let x = self.to_16k(pcm);
        self.fed += x.len();
        self.pieces.extend(self.vad.accept(&x));
        let last = self.pieces.last()?;
        let end = last.start + last.samples.len();
        let total: usize = self.pieces.iter().map(|p| p.samples.len()).sum();
        let quiet = !self.vad.speaking() && self.fed.saturating_sub(end) as f32 >= GAP_SECS * 16_000.0;
        if quiet || total as f32 >= MAX_SECS * 16_000.0 {
            return Some(Utterance { pieces: std::mem::take(&mut self.pieces), state: self.state.clone() });
        }
        None
    }
}

impl Listener {
    /// The speaker is talking right now (VAD inside a segment).
    pub fn in_speech(&self) -> bool {
        self.vad.speaking()
    }
}

impl Utterance {
    pub fn secs(&self) -> f32 {
        self.pieces.iter().map(|p| p.samples.len()).sum::<usize>() as f32 / 16_000.0
    }

    /// Transcribe with the call model. Utterances of one participant are serialised
    /// on their language state, so they come out in order.
    pub fn transcribe(self, echo: Option<&str>, speaker_key: Option<&str>) -> Result<Heard, String> {
        self.transcribe_call(echo, speaker_key, false)
    }

    /// `echo_always`: drop speech that repeats `echo` even when the echo filter setting
    /// is off (web calls, where the browser's echo cancellation may let bia through).
    pub fn transcribe_call(self, echo: Option<&str>, speaker_key: Option<&str>, echo_always: bool) -> Result<Heard, String> {
        models::nice_thread();
        let reg = registry::current();
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let asr = reg.asr_call.clone();
        let mut heard = pipeline::transcribe(&reg, self.pieces, &mut st, &Opts { asr: &asr, forced: None, echo, speaker_key, echo_always })?;
        // A new language on a call: learn it now; the next phrases route to it.
        if let Some(l) = &heard.new_lang {
            heard.notice = crate::learn::auto_learn(l);
            if heard.notice.is_some() {
                st.current = l.clone();
            }
        }
        Ok(heard)
    }
}
