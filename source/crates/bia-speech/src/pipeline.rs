//! Speech segments → tagged text.
//!
//! Calls, for each VAD segment (split at short pauses): language identification on the
//! (cleaned) audio, routing to an installed language with hysteresis, discovery of a
//! language that is heard but not installed, merging neighbours of the same language
//! into runs, Whisper per run with the language forced, then the filters.
//!
//! Voice notes and files: Whisper once per chunk (segments joined into phrases) with
//! its own language detection, so a note that switches languages comes out right
//! (Whisper keeps one language per pass: a whole mixed note became nonsense).

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
/// Voice notes: a language switch often comes after a breath of only 0.2 s, which
/// Silero keeps inside one segment. Segments longer than `BREATH_MIN_SEG` are also cut
/// at quiet stretches of `BREATH_SECS`, `BREATH_DB` below the segment's speech level.
const BREATH_SECS: f32 = 0.15;
const BREATH_DB: f32 = 20.0;
const BREATH_MIN_SEG: f32 = 3.0;
/// The longest chunk given to Whisper in one pass (its window is 30 s).
const CHUNK_SECS: f32 = 25.0;

/// Per-stream language memory (one call participant, or one file).
#[derive(Debug)]
pub struct LangState {
    pub current: String,
    pending: Option<String>,
    window: VecDeque<(String, f32)>,
    reported: Vec<String>,
    /// Voice notes / files: one-shot, so discovery needs less speech.
    file: bool,
    /// Voice notes: the sender's preferred language, when it is known and learned.
    sender: Option<String>,
}

impl LangState {
    pub fn new(default_lang: &str) -> Self {
        Self {
            current: default_lang.to_string(),
            pending: None,
            window: VecDeque::new(),
            reported: Vec::new(),
            file: false,
            sender: None,
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
    /// Voice notes: chunks transcribed, speech that gave no words, the model and how
    /// long it took.
    pub parts: usize,
    pub unclear_secs: f32,
    pub model: String,
    pub ms: u64,
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

    /// `17.6s, 5 parts, en+de+es, whisper-small 4.2s`, plus the speech that gave no
    /// words; so the model can judge the transcript (and ask again when much is unclear).
    pub fn summary(&self) -> String {
        let mut v = vec![format!("{:.1}s", self.secs)];
        if self.parts > 1 {
            v.push(format!("{} parts", self.parts));
        }
        let mut langs = self.langs();
        langs.sort();
        langs.dedup();
        if !langs.is_empty() {
            v.push(langs.join("+"));
        }
        if !self.model.is_empty() {
            v.push(format!("{} {:.1}s", self.model, self.ms as f32 / 1000.0));
        }
        if self.unclear_secs >= 0.5 {
            v.push(format!("{:.1}s unclear", self.unclear_secs));
        }
        v.join(", ")
    }

    /// The line a chat MCP puts before a received voice message's own text.
    pub fn voice_head(&self) -> String {
        let spoken = self.text();
        if spoken.is_empty() {
            return format!("[voice message, {}: no words recognized]", self.summary());
        }
        format!("[voice message, {}] {spoken}", self.summary())
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

    if st.file && opts.forced.is_none() {
        return transcribe_chunks(reg, pieces, clean, st, opts, heard);
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

/// Cut long segments at short quiet stretches of `level` (the same audio, possibly
/// cleaned): 20 ms frames whose energy is `BREATH_DB` below the segment's loud frames.
fn split_breaths(pieces: Vec<Segment>, level: &[f32]) -> Vec<Segment> {
    const FRAME: usize = 320;
    let mut out = Vec::new();
    for p in pieces {
        let end = (p.start + p.samples.len()).min(level.len());
        if secs(p.samples.len()) < BREATH_MIN_SEG || p.start >= end {
            out.push(p);
            continue;
        }
        let db: Vec<f32> = level[p.start..end]
            .chunks(FRAME)
            .map(|f| 10.0 * (f.iter().map(|x| x * x).sum::<f32>() / f.len() as f32 + 1e-10).log10())
            .collect();
        let mut sorted = db.clone();
        sorted.sort_by(f32::total_cmp);
        let quiet = sorted[sorted.len() * 9 / 10] - BREATH_DB;
        let need = (BREATH_SECS * RATE / FRAME as f32).ceil() as usize;
        // cut points: the middle of each quiet run, leaving pieces of at least 0.5 s
        let mut cuts = vec![0usize];
        let mut run = 0usize;
        for (i, &d) in db.iter().enumerate().chain(std::iter::once((db.len(), &f32::MAX))) {
            if d < quiet {
                run += 1;
                continue;
            }
            if run >= need {
                let mid = (i - run / 2) * FRAME;
                if secs(mid - cuts.last().unwrap()) >= 0.5 && secs(p.samples.len().saturating_sub(mid)) >= 0.5 {
                    cuts.push(mid);
                }
            }
            run = 0;
        }
        cuts.push(p.samples.len());
        for w in cuts.windows(2) {
            out.push(Segment { start: p.start + w[0], samples: p.samples[w[0]..w[1]].to_vec() });
        }
    }
    out
}

/// Voice notes: neighbouring segments that tiny's language identification puts in
/// the same language form one chunk (one Whisper pass each, at most `CHUNK_SECS`); a
/// segment too short to identify goes with its nearest identified neighbour.
fn chunks(pieces: &[Segment], lid: &[Option<String>]) -> Vec<(std::ops::Range<usize>, Option<String>)> {
    // silence between two segments, in samples (0 for the segment itself)
    let gap = |a: &Segment, b: &Segment| {
        let (x, y) = if a.start <= b.start { (a, b) } else { (b, a) };
        y.start.saturating_sub(x.start + x.samples.len())
    };
    let filled: Vec<Option<&str>> = (0..pieces.len())
        .map(|i| {
            if lid[i].is_some() {
                return lid[i].as_deref();
            }
            (0..pieces.len())
                .filter(|&k| lid[k].is_some())
                .min_by_key(|&k| (gap(&pieces[i], &pieces[k]), k > i))
                .and_then(|k| lid[k].as_deref())
        })
        .collect();
    let mut out: Vec<(std::ops::Range<usize>, Option<String>)> = Vec::new();
    for (i, p) in pieces.iter().enumerate() {
        if let Some((last, _)) = out.last_mut() {
            let span = secs(p.start + p.samples.len() - pieces[last.start].start);
            if filled[i] == filled[last.start] && span <= CHUNK_SECS {
                last.end = i + 1;
                continue;
            }
        }
        out.push((i..i + 1, filled[i].map(str::to_string)));
    }
    out
}

/// One chunk's words: Whisper sound-event captions removed, hallucinations dropped
/// (the reason goes to `dropped`); `None` = no words in it.
fn words_of(text: &str, d: f32, filter_on: bool, dropped: &mut Vec<String>) -> Option<String> {
    let (clean, captions) = filter::strip_captions(text);
    if captions {
        dropped.push(format!("captions: {}", text.trim()));
    }
    let (clean, looped) = filter::collapse_loops(&clean);
    if looped {
        dropped.push(format!("decoder loop: {}", text.trim()));
    }
    if clean.is_empty() {
        return None;
    }
    if filter_on && filter::is_hallucination(&clean, d) {
        dropped.push(format!("hallucination: {clean}"));
        return None;
    }
    Some(clean)
}

/// Voice notes: one pass per chunk. A transducer voice-note model (Parakeet) takes the
/// chunks in its languages; Whisper (the voice-note model, or the best one installed)
/// the rest, with its own language detection. A short chunk takes the language of its
/// nearest decoded neighbour, and so does one Whisper heard as a language that is not
/// learned while that neighbour's is.
fn transcribe_chunks(reg: &Registry, pieces: Vec<Segment>, clean: Vec<Vec<f32>>, st: &mut LangState, opts: &Opts, mut heard: Heard) -> Result<Heard, String> {
    let t0 = std::time::Instant::now();
    let s = &reg.settings;
    // only to group segments and route chunks: Whisper decides the language of what it decodes
    let mut lid: Vec<Option<String>> = {
        let slid = models::slid(reg)?;
        let slid = slid.lock().map_err(|_| "slid lock")?;
        clean.iter().map(|c| if secs(c.len()) >= LID_MIN_SECS { slid.detect(c).ok().filter(|l| !l.is_empty()) } else { None }).collect()
    };
    let mut groups = chunks(&pieces, &lid);
    // tiny mixes up languages (Persian heard as Arabic, Portuguese, Turkish …): a chunk it
    // puts in a language that is not learned is checked again by the best Whisper
    // installed; with none, the sender's language (when known) is taken instead
    let better = models::best_whisper(reg);
    let sender = st.sender.clone();
    let judge = if better == registry::BUNDLED_ASR { "the sender's language" } else { better };
    let opinion = |a: &[f32]| -> Result<Option<String>, String> {
        if better == registry::BUNDLED_ASR {
            return Ok(sender.clone());
        }
        let slid = models::slid_for(reg, better)?;
        let got = slid.lock().map_err(|_| "slid lock")?.detect(a).ok().filter(|x| !x.is_empty());
        Ok(got)
    };
    let main_id = if reg.asr.contains_key(opts.asr) { opts.asr } else { registry::BUNDLED_ASR };
    let is_whisper = |id: &str| reg.asr.get(id).is_some_and(|a| a.kind == "whisper");
    // models for some languages only: the voice-note model when it is one (Parakeet),
    // then every one-language model installed (shenava-fa); the rest goes to Whisper
    let mut own: Vec<&str> = vec![];
    if !is_whisper(main_id) {
        own.push(main_id);
    }
    own.extend(reg.asr.iter().filter(|(id, a)| a.kind == "nemo_ctc" && id.as_str() != main_id).map(|(id, _)| id.as_str()));
    let own_for = |lang: &str| own.iter().copied().find(|id| reg.asr[*id].langs.iter().any(|x| x == lang));
    // a language that is not learned; and, with only tiny to tell, one that would go to
    // tiny's own transcript when the sender speaks a language with a model of its own
    let sender_own = better == registry::BUNDLED_ASR && sender.as_deref().is_some_and(|l| own_for(l).is_some());
    let doubtful = |l: &Option<String>| {
        l.as_ref().is_some_and(|l| !reg.is_enabled(l) || (sender_own && Some(l) != sender.as_ref() && own_for(l).is_none()))
    };
    if groups.iter().any(|(_, l)| doubtful(l)) {
        let mut changed = false;
        for (r, l) in &groups {
            if !doubtful(l) {
                continue;
            }
            let again = opinion(&concat(r.clone().map(|k| clean[k].as_slice())))?;
            if let Some(x) = again.filter(|x| Some(x) != l.as_ref()) {
                crate::diag::log("speech", format!("note: tiny heard {l:?} in parts {r:?}, {judge} says {x}"));
                for p in lid[r.clone()].iter_mut().filter(|p| p.is_some()) {
                    *p = Some(x.clone());
                }
                changed = true;
            }
        }
        if changed {
            groups = chunks(&pieces, &lid);
        }
    }
    let whisper_id = if is_whisper(main_id) { main_id } else { better };
    let audio: Vec<Vec<f32>> = groups
        .iter()
        .map(|(g, _)| concat(g.clone().map(|k| if s.asr_denoised { clean[k].as_slice() } else { pieces[k].samples.as_slice() })))
        .collect();
    let mut used = vec![];
    // (language, words) per chunk; None = not decoded yet
    let mut out: Vec<Option<(String, Option<String>)>> = vec![None; groups.len()];
    // decoding in a language-only model; chunk by chunk: over longer audio (or chunks
    // joined with gaps) Parakeet stops early
    let decode_own = |id: &str, a: &[f32], dropped: &mut Vec<String>| -> Result<Option<String>, String> {
        let r = models::recognizer(reg, id)?;
        let (text, _) = r.lock().map_err(|_| "recognizer lock")?.decode_lang(a, "")?;
        Ok(words_of(&text, secs(a.len()), s.hallucination_filter, dropped))
    };
    for i in 0..groups.len() {
        let mut lang = groups[i].1.clone().unwrap_or_else(|| st.current.clone());
        let Some(id) = own_for(&lang) else { continue };
        let mut words = decode_own(id, &audio[i], &mut heard.dropped)?;
        used.push(id);
        if words.is_none() && secs(audio[i].len()) >= LID_MIN_SECS {
            // no words in its language: tiny may have been wrong (Persian taken for English)
            if let Some(x) = opinion(&audio[i])?.filter(|x| *x != lang) {
                crate::diag::log("speech", format!("note: {id} found no words in chunk {i} ({lang}); {judge} says {x}"));
                match own_for(&x) {
                    Some(other) => {
                        words = decode_own(other, &audio[i], &mut heard.dropped)?;
                        used.push(other);
                        lang = x;
                    }
                    // Whisper decodes it below
                    None => continue,
                }
            }
        }
        out[i] = Some((lang, words));
    }
    if out.iter().any(Option::is_none) {
        let wrec = models::recognizer(reg, whisper_id)?;
        let wguard = wrec.lock().map_err(|_| "recognizer lock")?;
        let rec: &models::Recognizer = &wguard;
        let decode = |a: &[f32], lang: &str, dropped: &mut Vec<String>| -> Result<(String, Option<String>), String> {
            let (mut text, got) = rec.decode_lang(a, lang)?;
            if !got.is_empty() && too_short(&text, secs(a.len()), &got) {
                // Whisper may stop after the first sentence: split and keep the longer
                let again = decode_robust(rec, a, &got, 0)?;
                if again.chars().count() > text.chars().count() {
                    text = again;
                }
            }
            Ok((got, words_of(&text, secs(a.len()), s.hallucination_filter, dropped)))
        };
        let todo: Vec<usize> = (0..groups.len()).filter(|&i| out[i].is_none()).collect();
        for &i in &todo {
            if secs(audio[i].len()) >= LID_MIN_SECS || groups.len() == 1 {
                let (lang, words) = decode(&audio[i], "", &mut heard.dropped)?;
                // Whisper heard a language that has a model of its own (tiny got it wrong)
                out[i] = Some(match own_for(&lang) {
                    Some(id) => {
                        used.push(id);
                        let w = decode_own(id, &audio[i], &mut heard.dropped)?;
                        (lang, w)
                    }
                    None => (lang, words),
                });
            }
        }
        let neighbour = |out: &[Option<(String, Option<String>)>], i: usize| {
            (0..out.len())
                .filter(|&k| k != i && matches!(&out[k], Some((l, Some(_))) if reg.is_enabled(l)))
                .min_by_key(|&k| k.abs_diff(i))
                .and_then(|k| out[k].as_ref().map(|(l, _)| l.clone()))
        };
        for &i in &todo {
            let near = neighbour(&out, i);
            let d = secs(audio[i].len());
            let redo = match &out[i] {
                None => true,
                // a short chunk heard as a language that is not learned, between learned ones
                Some((l, _)) if !reg.is_enabled(l) => d < FOREIGN_SECS,
                // a very short one where Whisper disagrees with tiny and the neighbour
                Some((l, _)) => d < SWITCH_SECS + 0.5 && groups[i].1.as_ref().is_some_and(|t| t != l && near.as_ref() == Some(t)),
            };
            if !redo {
                continue;
            }
            match near {
                Some(l) => match own_for(&l) {
                    Some(id) => {
                        out[i] = Some((l, decode_own(id, &audio[i], &mut heard.dropped)?));
                        used.push(id);
                    }
                    None => out[i] = Some(decode(&audio[i], &l, &mut heard.dropped)?),
                },
                None if out[i].is_none() => out[i] = Some(decode(&audio[i], "", &mut heard.dropped)?),
                None => {}
            }
        }
        used.push(whisper_id);
    }
    let mut unclear_before = false;
    for (i, o) in out.into_iter().enumerate() {
        let d = secs(audio[i].len());
        let Some((lang, words)) = o else { continue };
        let Some(words) = words else {
            heard.unclear_secs += d;
            unclear_before = true;
            continue;
        };
        let lang = if lang.is_empty() { st.current.clone() } else { lang };
        if d >= LID_MIN_SECS && langs::can_hear(&lang) {
            st.note(&lang, d);
        }
        let installed = reg.is_enabled(&lang);
        let text = if unclear_before { format!("[unclear] {words}") } else { words };
        unclear_before = false;
        match heard.runs.last_mut() {
            Some(last) if last.lang == lang && last.installed == installed => {
                last.text.push(' ');
                last.text.push_str(&text);
            }
            _ => heard.runs.push(Run { lang: lang.clone(), installed, text }),
        }
        if installed {
            st.current = lang;
        }
    }
    if unclear_before {
        if let Some(last) = heard.runs.last_mut() {
            last.text.push_str(" [unclear]");
        }
    }
    heard.new_lang = st.discover(reg);
    heard.parts = groups.len();
    let mut seen = std::collections::HashSet::new();
    used.retain(|id| seen.insert(*id));
    heard.model = used.join("+");
    heard.ms = t0.elapsed().as_millis() as u64;
    crate::diag::log(
        "speech",
        format!("note {:.1}s: {} segment(s), lid={lid:?}, chunks {:?}, {} ms → {:?} dropped={:?} new_lang={:?}", heard.secs, pieces.len(), groups.iter().map(|(r, l)| (r.clone(), l.clone().unwrap_or_default())).collect::<Vec<_>>(), heard.ms, heard.text(), heard.dropped, heard.new_lang),
    );
    Ok(heard)
}

/// Far less text than `d` seconds of speech hold. Normal speech is 10–15 characters
/// per second (fewer for CJK scripts).
fn too_short(text: &str, d: f32, lang: &str) -> bool {
    let min_chars = if matches!(lang, "zh" | "ja" | "ko" | "yue") { 3.0 } else { 8.0 };
    d >= 3.0 && (text.chars().count() as f32) < d * min_chars
}

fn decode_robust(rec: &models::Recognizer, samples: &[f32], lang: &str, depth: u32) -> Result<String, String> {
    let text = rec.decode(samples, lang)?;
    if depth >= 2 || !too_short(&text, secs(samples.len()), lang) {
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
    let mut pieces = models::segments(&reg, &samples, min_silence)?;
    if reg.settings.split_pauses && lang.is_none() {
        // breaths are found on the cleaned audio (noise fills them); pieces keep the original
        let den = if reg.settings.denoise { models::denoiser(&reg) } else { None };
        let cleaned = den.and_then(|d| d.lock().ok().and_then(|d| d.run(&samples).ok())).filter(|c| c.len() == samples.len());
        pieces = split_breaths(pieces, cleaned.as_deref().unwrap_or(&samples));
    }
    if pieces.is_empty() {
        return Ok(Heard { secs: 0.0, ..Default::default() });
    }
    let start = start.filter(|l| reg.is_enabled(l));
    let mut st = LangState::for_file(start.unwrap_or(&reg.default_lang));
    st.sender = start.map(str::to_string);
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
    fn breaths_split_long_segments() {
        let tone = |n: usize| (0..n).map(|i| (i as f32 * 0.05).sin() * 0.5).collect::<Vec<f32>>();
        // 4 s speech, 0.25 s breath, 3 s speech: one VAD segment, two pieces
        let mut x = tone(64_000);
        x.extend(vec![0.001f32; 4_000]);
        x.extend(tone(48_000));
        let p = vec![Segment { start: 0, samples: x.clone() }];
        let out = split_breaths(p, &x);
        assert_eq!(out.len(), 2);
        assert!((secs(out[0].samples.len()) - 4.12).abs() < 0.1, "{}", secs(out[0].samples.len()));
        assert_eq!(out.iter().map(|s| s.samples.len()).sum::<usize>(), x.len());
        // a 0.1 s gap is not a breath
        let mut y = tone(64_000);
        y.extend(vec![0.001f32; 1_600]);
        y.extend(tone(48_000));
        assert_eq!(split_breaths(vec![Segment { start: 0, samples: y.clone() }], &y).len(), 1);
    }

    fn ranges(c: Vec<(std::ops::Range<usize>, Option<String>)>) -> Vec<std::ops::Range<usize>> {
        c.into_iter().map(|(r, _)| r).collect()
    }

    #[test]
    fn note_chunks_follow_language() {
        let seg = |start: f32, dur: f32| Segment { start: (start * RATE) as usize, samples: vec![0.0; (dur * RATE) as usize] };
        let l = |x: &str| Some(x.to_string());
        // en, a short unidentified piece, de, de, short, es
        let p = vec![seg(0.3, 6.9), seg(7.3, 0.5), seg(7.9, 2.8), seg(11.0, 1.1), seg(12.4, 0.6), seg(13.2, 2.0)];
        let lid = vec![l("en"), None, l("de"), l("de"), None, l("es")];
        assert_eq!(ranges(chunks(&p, &lid)), vec![0..2, 2..4, 4..6]);
        // one language: one chunk until the Whisper window is full
        let p: Vec<Segment> = (0..12).map(|i| seg(i as f32 * 3.0, 2.5)).collect();
        let lid = vec![l("ru"); 12];
        assert_eq!(ranges(chunks(&p, &lid)), vec![0..8, 8..12]);
        assert_eq!(ranges(chunks(&p[..2], &[None, None])), vec![0..2]);
        // pieces cut at a breath touch: each keeps its own language
        let p = vec![seg(0.0, 4.0), seg(4.0, 3.0), seg(7.0, 2.0)];
        assert_eq!(ranges(chunks(&p, &[l("en"), l("de"), l("en")])), vec![0..1, 1..2, 2..3]);
        assert_eq!(chunks(&p, &[l("en"), l("de"), l("en")])[1].1.as_deref(), Some("de"));
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
        let note = Heard { secs: 17.6, parts: 5, model: "parakeet-v3+whisper-small".into(), ms: 3084, unclear_secs: 1.2, ..h.clone() };
        assert_eq!(
            note.voice_head(),
            "[voice message, 17.6s, 5 parts, en+es+ru, parakeet-v3+whisper-small 3.1s, 1.2s unclear] [speech ru] Привет, я буду в восемь [en] see you at the meeting [es?] hasta luego"
        );
        let silent = Heard { secs: 2.0, parts: 1, model: "whisper-base".into(), ms: 400, unclear_secs: 2.0, ..Default::default() };
        assert_eq!(silent.voice_head(), "[voice message, 2.0s, whisper-base 0.4s, 2.0s unclear: no words recognized]");
        assert!(discovery_notice("es", Some("inbox/x.ogg")).contains("lang_install(lang=\"es\")"));
    }
}
