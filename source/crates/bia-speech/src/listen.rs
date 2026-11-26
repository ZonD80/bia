//! Live call speech, one `Listener` per remote participant: Silero VAD over the
//! incoming audio (resampled to 16 kHz). Segments end at short pauses; an utterance is
//! complete when the speaker has been quiet for `GAP_SECS` (or after `MAX_SECS`).
//! Transcription runs off the audio thread via `Utterance::transcribe`.

use crate::models::{self, Segment, Vad};
use crate::pipeline::{self, Heard, LangState, Opts};
use crate::registry;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

const GAP_SECS: f32 = 0.7;
const MAX_SECS: f32 = 20.0;
/// VAD segments closer than this are one stretch of speech (words of a sentence, numbers
/// counted out) unless both sides are long enough to stand alone.
const JOIN_GAP_SECS: f32 = 1.0;
/// A segment this long keeps its own language identification (pipeline LID_MIN_SECS is 1 s).
const STANDS_ALONE_SECS: f32 = 1.5;
/// Real audio kept around joined segments: Whisper needs the edges of words.
const PAD_SECS: f32 = 0.15;
/// The VAD keeps speech this short in calls (a 0.2 s "six" on phone audio)...
const MIN_SPEECH_SECS: f32 = 0.1;
/// ...but a blip under this with no speech near it (a click, a breath) is dropped.
const LONE_MIN_SECS: f32 = 0.25;

pub struct Listener {
    vad: Vad,
    rate: u32,
    /// Leftover input samples when resampling 48 kHz → 16 kHz in groups of three.
    carry: Vec<f32>,
    fed: usize,
    pieces: Vec<Segment>,
    state: Arc<Mutex<LangState>>,
    /// Quiet that ends an utterance (GAP_SECS unless the bus sets its own).
    gap: f32,
    /// The 16 kHz stream since a little before the current utterance (`audio_at` = index
    /// of its first sample in the stream): joined segments are cut from it, pauses and all.
    audio: Vec<f32>,
    audio_at: usize,
    /// Streaming call model (Kroko): decodes on its own thread while the caller speaks.
    live: Option<Live>,
    /// Language of the utterance's first segment, found while waiting for the end.
    pre: Option<mpsc::Receiver<pipeline::Pre>>,
    /// The call's language, fixed by bia: every phrase is decoded in it, no detection.
    forced: Option<String>,
}

pub struct Utterance {
    pieces: Vec<Segment>,
    state: Arc<Mutex<LangState>>,
    /// The streaming model's text of this utterance, ready about when it ended.
    live: Option<mpsc::Receiver<String>>,
    /// Found ahead for an utterance of one segment.
    pre: Option<mpsc::Receiver<pipeline::Pre>>,
    forced: Option<String>,
}

/// Audio before the VAD noticed speech, fed first (Silero reports a start a little late).
const LIVE_PREROLL_SECS: f32 = 0.6;
/// Speech the VAD dropped (a click): stop feeding after this much quiet.
const LIVE_IDLE_SECS: f32 = 2.0;

enum LiveMsg {
    Audio(Vec<f32>),
    Finish(mpsc::Sender<String>),
}

/// A streaming recognizer on its own thread: audio goes in as it arrives, so the words
/// are already decoded when the speaker stops.
struct Live {
    tx: mpsc::Sender<LiveMsg>,
    /// Feeding the current stretch of speech.
    on: bool,
    /// Samples since the VAD last heard speech while `on`.
    idle: usize,
}

impl Live {
    fn start(id: String) -> Option<Self> {
        let (tx, rx) = mpsc::channel::<LiveMsg>();
        std::thread::Builder::new()
            .name("bia-live-asr".into())
            .spawn(move || {
                models::nice_thread();
                let reg = registry::current();
                let rec = match models::recognizer(&reg, &id) {
                    Ok(r) => r,
                    Err(e) => {
                        crate::diag::log("speech", format!("live {id}: {e}"));
                        return; // senders see a closed channel: the pipeline decodes instead
                    }
                };
                let mut stream: Option<models::OnlineStream> = None;
                while let Ok(m) = rx.recv() {
                    let Ok(r) = rec.lock() else { return };
                    match m {
                        LiveMsg::Audio(x) => {
                            if stream.is_none() {
                                stream = r.stream();
                            }
                            if let Some(s) = &stream {
                                s.accept(&x);
                                r.advance(s);
                            }
                        }
                        LiveMsg::Finish(back) => {
                            let text = stream.take().map(|s| r.finish(&s)).unwrap_or_default();
                            let _ = back.send(text);
                        }
                    }
                }
            })
            .ok()?;
        Some(Self { tx, on: false, idle: 0 })
    }

    /// The text so far, for an utterance that just ended (starts a new stretch).
    fn finish(&mut self) -> Option<mpsc::Receiver<String>> {
        let was = std::mem::replace(&mut self.on, false);
        let (back, rx) = mpsc::channel();
        self.tx.send(LiveMsg::Finish(back)).ok()?;
        was.then_some(rx)
    }
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
            vad: Vad::new(&reg, min_silence, MAX_SECS, MIN_SPEECH_SECS)?,
            rate,
            carry: Vec::new(),
            fed: 0,
            pieces: Vec::new(),
            state: Arc::new(Mutex::new(LangState::new(start.as_deref().unwrap_or(&reg.default_lang)))),
            gap: GAP_SECS,
            audio: Vec::new(),
            audio_at: 0,
            live: reg.asr.get(&reg.asr_call).filter(|a| a.kind == models::ONLINE).and_then(|_| Live::start(reg.asr_call.clone())),
            pre: None,
            forced: None,
        })
    }

    /// Hear the whole call in `lang` (bia was told the call's language): no language
    /// detection, which also keeps a short "Hello" from being taken for another language.
    /// The streaming call model is dropped when it does not know `lang` (Whisper takes it).
    pub fn force_lang(&mut self, lang: &str) -> Result<(), String> {
        let code = crate::langs::parse(lang).map(|l| l.code).ok_or_else(|| format!("unknown language {lang}"))?;
        if !crate::langs::can_hear(&code) {
            return Err(format!("{code} can not be heard (no speech model knows it)"));
        }
        let _ = crate::learn::enable_hearing(&code);
        let reg = registry::current();
        if crate::pipeline::call_model_for(&reg, &reg.asr_call, &code) != reg.asr_call {
            self.live = None;
        }
        self.state.lock().unwrap_or_else(|e| e.into_inner()).current = code.clone();
        self.forced = Some(code);
        Ok(())
    }

    /// A longer pause before an utterance counts as finished: phone calls (narrowband,
    /// network dropouts) split sentences at 0.7 s.
    pub fn set_gap(&mut self, secs: f32) {
        self.gap = secs.clamp(0.3, 5.0);
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
        // a live call keeps its models loaded through long silences
        crate::idle::touch();
        let x = self.to_16k(pcm);
        self.fed += x.len();
        self.audio.extend_from_slice(&x);
        let had = self.pieces.len();
        self.pieces.extend(self.vad.accept(&x));
        // the first segment of an utterance: find its language while waiting for the end
        if had == 0 && self.pieces.len() == 1 && self.forced.is_none() {
            let (tx, rx) = mpsc::channel();
            let samples = self.pieces[0].samples.clone();
            let ok = std::thread::Builder::new()
                .name("bia-speech-pre".into())
                .spawn(move || {
                    let _ = tx.send(pipeline::prepare(&registry::current(), &samples));
                })
                .is_ok();
            self.pre = ok.then_some(rx);
        } else if self.pieces.len() > 1 {
            self.pre = None;
        }
        let speaking = self.vad.speaking() || !self.pieces.is_empty();
        if let Some(live) = &mut self.live {
            if live.on {
                let _ = live.tx.send(LiveMsg::Audio(x.clone()));
                live.idle = if self.vad.speaking() { 0 } else { live.idle + x.len() };
                if self.pieces.is_empty() && live.idle as f32 >= LIVE_IDLE_SECS * 16_000.0 {
                    let _ = live.finish(); // a false start: drop what it heard
                }
            } else if speaking {
                let n = ((LIVE_PREROLL_SECS * 16_000.0) as usize).min(self.audio.len());
                let _ = live.tx.send(LiveMsg::Audio(self.audio[self.audio.len() - n..].to_vec()));
                live.on = true;
                live.idle = 0;
            }
        }
        // nothing pending: keep only the last moments (the VAD reports a segment a little
        // after it began, and the padding reaches back)
        let keep = if self.pieces.is_empty() { 2.0 } else { MAX_SECS + 5.0 };
        let keep = (keep * 16_000.0) as usize;
        if self.audio.len() > keep {
            let cut = self.audio.len() - keep;
            self.audio.drain(..cut);
            self.audio_at += cut;
        }
        let last = self.pieces.last()?;
        let end = last.start + last.samples.len();
        let total: usize = self.pieces.iter().map(|p| p.samples.len()).sum();
        let quiet = !self.vad.speaking() && self.fed.saturating_sub(end) as f32 >= self.gap * 16_000.0;
        if quiet || total as f32 >= MAX_SECS * 16_000.0 {
            let pieces = join_close(std::mem::take(&mut self.pieces), &self.audio, self.audio_at);
            let live = self.live.as_mut().and_then(Live::finish);
            let pre = self.pre.take().filter(|_| pieces.len() == 1);
            return Some(Utterance { pieces, state: self.state.clone(), live, pre, forced: self.forced.clone() });
        }
        None
    }
}

/// Join neighbouring VAD segments into one stretch of the real audio (with the pauses
/// between them and a little around) when they are close and at least one is short.
/// Decoded one by one, half-second words ("three", "four") come out as noise: the VAD
/// cuts a counted-out number or a slow sentence word by word on phone audio. Long
/// segments stay apart, so a switch of language between sentences is still seen.
/// `audio` is the stream from sample `at`; a piece outside it is kept as it is.
fn join_close(pieces: Vec<Segment>, audio: &[f32], at: usize) -> Vec<Segment> {
    let rate = 16_000.0;
    let long = |n: usize| n as f32 / rate >= STANDS_ALONE_SECS;
    let mut groups: Vec<(usize, usize, Vec<Segment>)> = Vec::new(); // (start, end, parts)
    for p in pieces {
        let (s, e) = (p.start, p.start + p.samples.len());
        if let Some((gs, ge, parts)) = groups.last_mut() {
            let close = s.saturating_sub(*ge) as f32 / rate <= JOIN_GAP_SECS;
            let both_long = long(*ge - *gs) && long(e - s);
            if close && !both_long {
                *ge = e.max(*ge);
                parts.push(p);
                continue;
            }
        }
        groups.push((s, e, vec![p]));
    }
    let pad = (PAD_SECS * rate) as usize;
    groups.retain(|(_, _, parts)| parts.len() > 1 || parts[0].samples.len() as f32 / rate >= LONE_MIN_SECS);
    groups
        .into_iter()
        .map(|(s, e, mut parts)| {
            let from = s.saturating_sub(pad).max(at);
            let to = (e + pad).min(at + audio.len());
            if parts.len() == 1 && from >= s && to <= e {
                return parts.pop().unwrap();
            }
            if s < at || e > at + audio.len() {
                // not in the kept audio (cannot happen within MAX_SECS): the VAD's own pieces
                let mut samples = Vec::new();
                for p in parts {
                    samples.extend(p.samples);
                }
                return Segment { start: s, samples };
            }
            Segment { start: from, samples: audio[from - at..to - at].to_vec() }
        })
        .collect()
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
        let pre = self.pre.and_then(|r| r.recv_timeout(std::time::Duration::from_secs(10)).ok());
        let opts = Opts { asr: &asr, forced: self.forced.as_deref(), echo, speaker_key, echo_always, live: self.live.as_ref(), pre: pre.as_ref() };
        let mut heard = pipeline::transcribe(&reg, self.pieces, &mut st, &opts)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start_s: f32, secs: f32, audio: &[f32]) -> Segment {
        let s = (start_s * 16_000.0) as usize;
        let n = (secs * 16_000.0) as usize;
        Segment { start: s, samples: audio[s..s + n].to_vec() }
    }

    #[test]
    fn counted_words_become_one_stretch_with_their_pauses() {
        let audio: Vec<f32> = (0..16_000 * 10).map(|i| i as f32).collect();
        // one, two, three, four: 0.4 s words 0.5 s apart
        let pieces = vec![seg(1.0, 0.4, &audio), seg(1.9, 0.4, &audio), seg(2.8, 0.4, &audio), seg(3.7, 0.4, &audio)];
        let out = join_close(pieces, &audio, 0);
        assert_eq!(out.len(), 1);
        // from 0.15 s before the first word to 0.15 s after the last, the real samples
        assert_eq!(out[0].start, (0.85 * 16_000.0) as usize);
        assert_eq!(out[0].samples.len(), ((4.1 + 0.15 - 0.85) * 16_000.0) as usize);
        assert_eq!(out[0].samples[0], out[0].start as f32);
    }

    #[test]
    fn long_sentences_keep_their_own_language() {
        let audio = vec![0.0f32; 16_000 * 10];
        let pieces = vec![seg(0.5, 2.0, &audio), seg(3.0, 2.5, &audio)];
        assert_eq!(join_close(pieces, &audio, 0).len(), 2);
    }

    #[test]
    fn a_short_word_joins_the_long_sentence_next_to_it() {
        let audio = vec![0.0f32; 16_000 * 10];
        let pieces = vec![seg(0.5, 0.5, &audio), seg(1.5, 2.5, &audio)];
        let out = join_close(pieces, &audio, 0);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn far_apart_stays_apart_and_lone_segments_are_untouched() {
        let audio = vec![0.0f32; 16_000 * 10];
        let pieces = vec![seg(0.5, 0.4, &audio), seg(3.0, 0.4, &audio)];
        let out = join_close(pieces, &audio, 0);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].start, (0.5 * 16_000.0) as usize - (0.15 * 16_000.0) as usize);
    }

    #[test]
    fn a_lone_blip_is_dropped_but_a_short_word_near_others_is_kept() {
        let audio = vec![0.0f32; 16_000 * 10];
        // a 0.15 s click far from anything, then "five" (0.4 s) and "six" (0.2 s)
        let pieces = vec![seg(0.5, 0.15, &audio), seg(4.0, 0.4, &audio), seg(4.8, 0.2, &audio)];
        let out = join_close(pieces, &audio, 0);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].start, (3.85 * 16_000.0) as usize);
    }

    #[test]
    fn audio_already_dropped_falls_back_to_the_vad_pieces() {
        let audio = vec![0.0f32; 16_000 * 10];
        let pieces = vec![seg(1.0, 0.4, &audio), seg(1.9, 0.4, &audio)];
        // the kept audio starts after the first piece began
        let out = join_close(pieces, &audio[16_000 * 2..], 16_000 * 2);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].samples.len(), (0.8 * 16_000.0) as usize);
    }
}
