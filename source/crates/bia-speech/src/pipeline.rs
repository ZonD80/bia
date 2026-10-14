//! Speech segments → tagged text.
//!
//! For each VAD segment (split at short pauses): language identification on the
//! (cleaned) audio, routing to an installed language with hysteresis, discovery of a
//! language that is heard but not installed, merging neighbours of the same language
//! into runs, Whisper per run with the language forced, then the filters.

use crate::filter;
use crate::langs;
use crate::models::{self, Segment};
use crate::registry::{self, Registry};
use std::collections::VecDeque;

const RATE: f32 = 16_000.0;
/// Shorter segments are too short for reliable language identification.
const LID_MIN_SECS: f32 = 1.0;
/// A segment this long switches the language on its own; shorter ones need two in a row.
const SWITCH_SECS: f32 = 1.5;
/// A not-installed language is transcribed in that language only from this long a
/// segment (or once discovery has enough evidence); otherwise the current one is used.
const FOREIGN_SECS: f32 = 3.0;

/// Per-stream language memory (one call participant, or one file).
#[derive(Debug)]
pub struct LangState {
    pub current: String,
    pending: Option<String>,
    window: VecDeque<(String, f32)>,
    reported: Vec<String>,
    /// Voice notes / files: one-shot, so discovery needs less speech.
    file: bool,
}

impl LangState {
    pub fn new(default_lang: &str) -> Self {
        Self {
            current: default_lang.to_string(),
            pending: None,
            window: VecDeque::new(),
            reported: Vec::new(),
            file: false,
        }
    }

    pub fn for_file(default_lang: &str) -> Self {
        Self { file: true, ..Self::new(default_lang) }
    }

    fn note(&mut self, lang: &str, secs: f32) {
        self.window.push_back((lang.to_string(), secs));
        let cap = if self.file { 64 } else { 8 };
        while self.window.len() > cap {
            self.window.pop_front();
        }
    }

    /// Enough consistent evidence that `lang` is really being spoken.
    fn evidence(&self, lang: &str) -> bool {
        let (mut n, mut secs, mut total) = (0, 0.0f32, 0.0f32);
        for (l, s) in &self.window {
            total += s;
            if l == lang {
                n += 1;
                secs += s;
            }
        }
        let (min_n, min_secs) = if self.file { (1, 4.0) } else { (3, 6.0) };
        n >= min_n && secs >= min_secs && secs >= total * 0.6
    }

    /// The not-installed language to report now, if any (each at most once).
    fn discover(&mut self, reg: &Registry) -> Option<String> {
        if !reg.settings.discovery {
            return None;
        }
        let mut cands: Vec<String> = self.window.iter().map(|(l, _)| l.clone()).collect();
        cands.sort();
        cands.dedup();
        let found = cands.into_iter().find(|l| {
            !reg.is_enabled(l)
                && !reg.ignored.contains(l)
                && !self.reported.contains(l)
                && langs::can_hear(l)
                && self.evidence(l)
        })?;
        self.reported.push(found.clone());
        Some(found)
    }

    /// How to transcribe a segment that language identification put in `lid`.
    fn route(&mut self, reg: &Registry, lid: Option<&str>, secs: f32) -> Route {
        let Some(l) = lid.filter(|l| langs::can_hear(l)) else {
            return Route::installed(&self.current);
        };
        if reg.is_enabled(l) {
            // a recording is judged as a whole: each segment keeps its own language
            if self.file {
                self.current = l.to_string();
                self.pending = None;
                return Route::installed(l);
            }
            if l == self.current {
                self.pending = None;
            } else if secs >= SWITCH_SECS || self.pending.as_deref() == Some(l) {
                self.current = l.to_string();
                self.pending = None;
            } else {
                self.pending = Some(l.to_string());
            }
            return Route::installed(&self.current);
        }
        // Not learned yet. Never force another language onto it (Whisper would
        // translate): with enough evidence force this one, else let Whisper detect.
        let sure = secs >= FOREIGN_SECS || self.evidence(l);
        Route { lang: l.to_string(), installed: false, decode: if sure { l.to_string() } else { String::new() } }
    }
}

/// Tag language, whether it is learned, and the language forced on Whisper ("" =
/// Whisper detects).
#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    pub lang: String,
    pub installed: bool,
    pub decode: String,
}

impl Route {
    fn installed(l: &str) -> Self {
        Self { lang: l.to_string(), installed: true, decode: l.to_string() }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub lang: String,
    pub installed: bool,
    pub text: String,
}

#[derive(Clone, Debug, Default)]
pub struct Heard {
    pub runs: Vec<Run>,
    /// A language heard with enough evidence that is not installed.
    pub new_lang: Option<String>,
    /// Why audio was dropped (hallucination, echo, speaker), for logs.
    pub dropped: Vec<String>,
    /// For the model: a new language was learned automatically from this audio.
    pub notice: Option<String>,
    pub secs: f32,
}

impl Heard {
    /// `[speech ru] Привет [en] see you`; a `?` marks a language that is not
    /// installed (rough transcript). Empty when nothing usable was heard.
    pub fn text(&self) -> String {
        let mut out = String::new();
        let mut last: Option<(&str, bool)> = None;
        for r in &self.runs {
            let tag = format!("{}{}", r.lang, if r.installed { "" } else { "?" });
            if last != Some((r.lang.as_str(), r.installed)) {
                if out.is_empty() {
                    out.push_str(&format!("[speech {tag}] "));
                } else {
                    out.push_str(&format!(" [{tag}] "));
                }
            } else {
                out.push(' ');
            }
            out.push_str(&r.text);
            last = Some((r.lang.as_str(), r.installed));
        }
        out
    }

    pub fn langs(&self) -> Vec<String> {
        let mut v: Vec<String> = self.runs.iter().map(|r| r.lang.clone()).collect();
        v.dedup();
        v
    }
}

/// The notice the model gets when a new language is heard.
pub fn discovery_notice(lang: &str, file: Option<&str>) -> String {
    let name = langs::name(lang);
    let mut s = format!(
        "[new language heard: {lang} ({name}), not installed yet. Tell the user (in the language you are using with them) that you are learning {name} now, then call lang_install(lang=\"{lang}\") (lang MCP)."
    );
    if let Some(f) = file {
        s.push_str(&format!(" When it is installed, lang_transcribe(file=\"{f}\") gives an accurate transcript."));
    }
    s.push_str(" If the user does not want it, lang_disable(lang) stops these notices.]");
    s
}

pub struct Opts<'a> {
    /// Whisper model id (registry.asr_call / asr_file).
    pub asr: &'a str,
    /// Transcribe everything in this language (no detection, no discovery).
    pub forced: Option<&'a str>,
    /// What we were saying while this was heard (echo filter).
    pub echo: Option<&'a str>,
    /// Enrolled speaker key for the opt-in speaker check.
    pub speaker_key: Option<&'a str>,
    /// Apply the echo filter even when the setting is off.
    pub echo_always: bool,
}

fn secs(n: usize) -> f32 {
    n as f32 / RATE
}

fn concat<'a>(parts: impl Iterator<Item = &'a [f32]>) -> Vec<f32> {
    let gap = vec![0.0f32; (RATE * 0.2) as usize];
    let mut out = Vec::new();
    for (i, p) in parts.enumerate() {
        if i > 0 {
            out.extend_from_slice(&gap);
        }
        out.extend_from_slice(p);
    }
    out
}

pub fn transcribe(reg: &Registry, pieces: Vec<Segment>, st: &mut LangState, opts: &Opts) -> Result<Heard, String> {
    let mut heard = Heard { secs: pieces.iter().map(|p| secs(p.samples.len())).sum(), ..Default::default() };
    if pieces.is_empty() {
        return Ok(heard);
    }
    let s = &reg.settings;
    let den = if s.denoise { models::denoiser(reg) } else { None };
    let clean: Vec<Vec<f32>> = pieces
        .iter()
        .map(|p| {
            den.as_ref()
                .and_then(|d| d.lock().ok().and_then(|d| d.run(&p.samples).ok()))
                .filter(|c| !c.is_empty())
                .unwrap_or_else(|| p.samples.clone())
        })
        .collect();

    if s.speaker_check {
        if let Some(enrolled) = opts.speaker_key.and_then(|k| reg.speakers.get(k)) {
            if heard.secs >= 1.0 {
                let spk = models::speaker(reg)?;
                let all = concat(clean.iter().map(Vec::as_slice));
                let v = spk.lock().map_err(|_| "speaker lock")?.embed(&all)?;
                let sim = models::cosine(&v, enrolled);
                if sim < s.speaker_threshold {
                    heard.dropped.push(format!("speaker mismatch ({sim:.2})"));
                    return Ok(heard);
                }
            }
        }
    }

    // Language per segment.
    let mut lid: Vec<Option<String>> = vec![None; pieces.len()];
    let mut all_weak = false;
    if let Some(f) = opts.forced {
        lid.iter_mut().for_each(|l| *l = Some(f.to_string()));
    } else {
        // voice notes: the voice-note model (better at close languages); calls: tiny
        let slid = if st.file { models::slid_for(reg, opts.asr)? } else { models::slid(reg)? };
        let slid = slid.lock().map_err(|_| "slid lock")?;
        for (i, c) in clean.iter().enumerate() {
            if secs(c.len()) >= LID_MIN_SECS {
                lid[i] = slid.detect(c).ok().filter(|l| !l.is_empty());
            }
        }
        if lid.iter().all(Option::is_none) && heard.secs >= 0.6 {
            // Only short segments: detect once over all of them (weak; not counted
            // as discovery evidence, and only a learned language is trusted).
            let all = concat(clean.iter().map(Vec::as_slice));
            if let Ok(l) = slid.detect(&all) {
                if !l.is_empty() && reg.is_enabled(&l) {
                    lid.iter_mut().for_each(|x| *x = Some(l.clone()));
                    all_weak = true;
                }
            }
        }
    }

    // Evidence first (the whole utterance / file counts), then route: segments with
    // their own language identification in order (hysteresis), then short ones take
    // the route of their nearest neighbour (a short "Добрый день" before Russian is
    // Russian, not the previous language).
    let routed: Vec<Route> = if let Some(f) = opts.forced {
        vec![Route { lang: f.to_string(), installed: reg.is_enabled(f), decode: f.to_string() }; pieces.len()]
    } else {
        for (i, p) in pieces.iter().enumerate() {
            let d = secs(p.samples.len());
            if let (Some(l), true) = (&lid[i], d >= LID_MIN_SECS) {
                st.note(l, d);
            }
        }
        let mut own: Vec<Option<Route>> = vec![None; pieces.len()];
        for (i, p) in pieces.iter().enumerate() {
            if lid[i].is_some() {
                let d = if all_weak { heard.secs } else { secs(p.samples.len()) };
                own[i] = Some(st.route(reg, lid[i].as_deref(), d));
            }
        }
        (0..pieces.len())
            .map(|i| {
                if let Some(r) = &own[i] {
                    return r.clone();
                }
                let next = (i + 1..pieces.len()).find(|&k| own[k].is_some());
                let prev = (0..i).rev().find(|&k| own[k].is_some());
                let pick = match (prev, next) {
                    (Some(p), Some(n)) => if i - p < n - i { p } else { n },
                    (Some(p), None) => p,
                    (None, Some(n)) => n,
                    (None, None) => return Route::installed(&st.current),
                };
                own[pick].clone().unwrap()
            })
            .collect()
    };
    if opts.forced.is_none() {
        heard.new_lang = st.discover(reg);
    }
    crate::diag::log(
        "speech",
        format!(
            "{:.1}s in {} segment(s); lid={:?}; routes={:?}; current={}",
            heard.secs,
            pieces.len(),
            lid,
            routed.iter().map(|r| format!("{}{}/{}", r.lang, if r.installed { "" } else { "?" }, if r.decode.is_empty() { "auto" } else { &r.decode })).collect::<Vec<_>>(),
            st.current
        ),
    );

    // Merge neighbours with the same language into runs and transcribe each.
    let rec = models::recognizer(reg, opts.asr)?;
    let rec = rec.lock().map_err(|_| "recognizer lock")?;
    let mut i = 0;
    while i < pieces.len() {
        let mut j = i + 1;
        while j < pieces.len() && routed[j] == routed[i] {
            j += 1;
        }
        let src: Vec<&[f32]> = (i..j)
            .map(|k| if s.asr_denoised { clean[k].as_slice() } else { pieces[k].samples.as_slice() })
            .collect();
        let samples = concat(src.into_iter());
        let Route { lang, installed, decode } = routed[i].clone();
        let mut text = decode_robust(&rec, &samples, &decode, 0)?;
        let d = secs(samples.len());
        if j - i > 1 && (text.chars().count() as f32) < d * 8.0 {
            // A merged run came out short: Whisper skipped part of it. Try the
            // segments one by one and keep the longer result.
            let mut parts = Vec::new();
            for k in i..j {
                let seg = if s.asr_denoised { &clean[k] } else { &pieces[k].samples };
                parts.push(decode_robust(&rec, seg, &decode, 0)?.trim().to_string());
            }
            let alt = parts.join(" ").trim().to_string();
            if alt.chars().count() > text.chars().count() {
                text = alt;
            }
        }
        if s.hallucination_filter && filter::is_hallucination(&text, d) {
            if !text.trim().is_empty() {
                heard.dropped.push(format!("hallucination: {}", text.trim()));
            }
        } else if (s.echo_filter || opts.echo_always) && opts.echo.is_some_and(|e| filter::is_echo(&text, e)) {
            heard.dropped.push(format!("echo: {}", text.trim()));
        } else {
            match heard.runs.last_mut() {
                Some(last) if last.lang == lang && last.installed == installed => {
                    last.text.push(' ');
                    last.text.push_str(text.trim());
                }
                _ => heard.runs.push(Run { lang, installed, text: text.trim().to_string() }),
            }
        }
        i = j;
    }
    crate::diag::log("speech", format!("→ {:?} dropped={:?} new_lang={:?}", heard.text(), heard.dropped, heard.new_lang));
    Ok(heard)
}

/// Whisper (tiny especially) sometimes ends after the first sentence of a longer
/// segment. When the text is far too short for the audio, split at the quietest point
/// near the middle and transcribe both halves; keep whichever result is longer.
fn decode_robust(rec: &models::Recognizer, samples: &[f32], lang: &str, depth: u32) -> Result<String, String> {
    let text = rec.decode(samples, lang)?;
    let d = secs(samples.len());
    // Normal speech is 10–15 characters per second (fewer for CJK scripts).
    let min_chars = if matches!(lang, "zh" | "ja" | "ko" | "yue") { 3.0 } else { 8.0 };
    if depth >= 2 || d < 3.0 || text.chars().count() as f32 >= d * min_chars {
        return Ok(text);
    }
    let cut = quietest(samples, samples.len() * 3 / 10, samples.len() * 7 / 10);
    let a = decode_robust(rec, &samples[..cut], lang, depth + 1)?;
    let b = decode_robust(rec, &samples[cut..], lang, depth + 1)?;
    let joined = format!("{} {}", a.trim(), b.trim()).trim().to_string();
    Ok(if joined.chars().count() > text.chars().count() { joined } else { text })
}

/// Start of the lowest-energy 50 ms window in `from..to`.
fn quietest(x: &[f32], from: usize, to: usize) -> usize {
    let w = 800;
    let mut best = (f32::MAX, (from + to) / 2);
    let mut i = from;
    while i + w <= to.min(x.len()) {
        let e: f32 = x[i..i + w].iter().map(|s| s * s).sum();
        if e < best.0 {
            best = (e, i + w / 2);
        }
        i += w / 2;
    }
    best.1
}

/// A received voice message: transcribe it; if it is in a language that is not
/// learned yet, learn it (hearing at once, voice in the background) and transcribe
/// again so the text is accurate. `heard.notice` then says what was learned.
pub fn transcribe_note(path: &std::path::Path) -> Result<Heard, String> {
    transcribe_note_from(path, None)
}

/// `transcribe_note`, starting in the sender's preferred language (`lang`).
pub fn transcribe_note_from(path: &std::path::Path, lang: Option<&str>) -> Result<Heard, String> {
    if let Some(l) = lang.filter(|l| langs::can_hear(l)) {
        let _ = crate::learn::enable_hearing(l);
    }
    let start = lang.map(str::to_string);
    let file = |p: &std::path::Path| transcribe_file_hint(p, start.as_deref());
    let heard = file(path)?;
    let Some(l) = heard.new_lang.clone() else { return Ok(heard) };
    let Some(notice) = crate::learn::auto_learn(&l) else { return Ok(heard) };
    let mut again = file(path).unwrap_or(heard);
    again.new_lang = Some(l);
    again.notice = Some(notice);
    Ok(again)
}

/// Transcribe a whole audio file (voice note). `lang` forces one language.
pub fn transcribe_file(path: &std::path::Path, lang: Option<&str>, asr: Option<&str>) -> Result<Heard, String> {
    transcribe_file_with(path, lang, asr, None)
}

fn transcribe_file_hint(path: &std::path::Path, start: Option<&str>) -> Result<Heard, String> {
    transcribe_file_with(path, None, None, start)
}

fn transcribe_file_with(path: &std::path::Path, lang: Option<&str>, asr: Option<&str>, start: Option<&str>) -> Result<Heard, String> {
    models::nice_thread();
    let reg = registry::current();
    let decoded = crate::audio::decode_file(path)?;
    if decoded.is_empty() {
        return Err("the audio is empty".into());
    }
    // Leading silence: speech that starts in the very first frame is easy for the VAD to miss.
    let mut samples = vec![0.0f32; 8_000];
    samples.extend(decoded);
    let min_silence = if reg.settings.split_pauses { 0.25 } else { 0.5 };
    let pieces = models::segments(&reg, &samples, min_silence)?;
    if pieces.is_empty() {
        return Ok(Heard { secs: 0.0, ..Default::default() });
    }
    let mut st = LangState::for_file(start.filter(|l| reg.is_enabled(l)).unwrap_or(&reg.default_lang));
    let asr = asr.unwrap_or(&reg.asr_file).to_string();
    transcribe(&reg, pieces, &mut st, &Opts { asr: &asr, forced: lang, echo: None, speaker_key: None, echo_always: false })
}

/// Voice print of a recording, for the opt-in speaker check.
pub fn voice_print(path: &std::path::Path) -> Result<Vec<f32>, String> {
    let reg = registry::current();
    let samples = crate::audio::decode_file(path)?;
    let pieces = models::segments(&reg, &samples, 0.5)?;
    let speech = concat(pieces.iter().map(|p| p.samples.as_slice()));
    if secs(speech.len()) < 3.0 {
        return Err(format!("only {:.1}s of speech; need at least 3s", secs(speech.len())));
    }
    let spk = models::speaker(&reg)?;
    let v = spk.lock().map_err(|_| "speaker lock")?.embed(&speech)?;
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg(enabled: &[&str]) -> Registry {
        Registry { enabled: enabled.iter().map(|s| s.to_string()).collect(), ..Default::default() }
    }

    #[test]
    fn routing_hysteresis() {
        let r = reg(&["en", "ru"]);
        let mut st = LangState::new("en");
        assert_eq!(st.route(&r, Some("ru"), 1.2), Route::installed("en"), "one short ru segment does not switch");
        assert_eq!(st.route(&r, Some("ru"), 1.1), Route::installed("ru"), "second in a row switches");
        assert_eq!(st.route(&r, None, 0.5), Route::installed("ru"), "short segments stay");
        assert_eq!(st.route(&r, Some("en"), 2.0), Route::installed("en"), "long segment switches at once");
    }

    #[test]
    fn foreign_and_discovery_call() {
        let r = reg(&["en"]);
        let mut st = LangState::new("en");
        // a short foreign segment is not forced into any language until there is evidence
        assert_eq!(st.route(&r, Some("es"), 1.5), Route { lang: "es".into(), installed: false, decode: String::new() });
        st.note("es", 2.0);
        st.note("es", 2.5);
        assert_eq!(st.discover(&r), None, "two segments are not enough on a call");
        st.note("es", 2.0);
        assert_eq!(st.route(&r, Some("es"), 1.5), Route { lang: "es".into(), installed: false, decode: "es".into() });
        assert_eq!(st.discover(&r), Some("es".into()));
        assert_eq!(st.discover(&r), None, "reported once");
        // a long foreign segment is transcribed in that language right away
        let mut st = LangState::new("en");
        assert_eq!(st.route(&r, Some("de"), 3.5).decode, "de");
    }

    #[test]
    fn discovery_respects_majority_and_ignored() {
        let mut r = reg(&["en"]);
        let mut st = LangState::new("en");
        for _ in 0..3 {
            st.note("es", 2.0);
        }
        for _ in 0..5 {
            st.note("en", 2.0);
        }
        assert_eq!(st.discover(&r), None, "es is a minority of the speech");
        let mut st = LangState::for_file("en");
        st.note("es", 4.5);
        r.ignored.push("es".into());
        assert_eq!(st.discover(&r), None, "ignored");
        r.ignored.clear();
        assert_eq!(st.discover(&r), Some("es".into()), "a 4.5 s voice note is enough");
        r.settings.discovery = false;
        let mut st = LangState::for_file("en");
        st.note("fr", 9.0);
        assert_eq!(st.discover(&r), None, "discovery off");
    }

    #[test]
    fn heard_text_tags() {
        let h = Heard {
            runs: vec![
                Run { lang: "ru".into(), installed: true, text: "Привет, я буду в восемь".into() },
                Run { lang: "en".into(), installed: true, text: "see you at the meeting".into() },
                Run { lang: "es".into(), installed: false, text: "hasta luego".into() },
            ],
            ..Default::default()
        };
        assert_eq!(h.text(), "[speech ru] Привет, я буду в восемь [en] see you at the meeting [es?] hasta luego");
        assert_eq!(Heard::default().text(), "");
        assert!(discovery_notice("es", Some("inbox/x.ogg")).contains("lang_install(lang=\"es\")"));
    }
}
