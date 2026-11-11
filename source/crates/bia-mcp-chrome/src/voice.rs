//! Voice in a web call (Google Meet or any WebRTC page) running in a Chrome tab.
//!
//! `call_open` has the add-on open the page in a tab it marks; in that tab the page script
//! taps every remote audio track and hands the page bia's voice as its microphone. Audio
//! comes here as `voice` frames from the add-on: 16 kHz speech per voice (a conference
//! server's contributing source when it names one, else the track). Each voice has its own
//! `Listener`, so speech reaches the model per person as `voice:<name>` on `chrome:call`.
//! Voices start as s1, s2 …; once two have spoken the model is asked to have everyone
//! introduce themselves and to name each voice (`call_name`).

use base64::Engine;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const CHANNEL: &str = "chrome:call";
/// After a second voice was heard, wait this long before asking for introductions, so the
/// request does not cut into what is being said.
const INTRO_DELAY: Duration = Duration::from_secs(6);
/// Continuous speech (100 ms pieces) while bia talks that counts as interrupting.
const BARGE_PIECES: u32 = 3;
/// bia's words count as a possible echo this long after they finished.
const ECHO_SECS: u64 = 3;
/// A voice in the middle of a phrase whose audio stopped arriving gets quiet fed, so the
/// phrase ends (the page stops sending a silent voice; a shared slot moves to someone else).
const STALLED: Duration = Duration::from_millis(1200);
/// Samples per frame towards the page (Chrome takes at most 1 MiB per frame).
const PLAY_PIECE: usize = 4800;
const NAME_MAX: usize = 40;
/// The add-on gone this long during a call: the call is over.
const GONE: Duration = Duration::from_secs(45);

/// The way to the add-on (the bridge; a fake in tests).
pub trait Link: Send + Sync {
    fn request(&self, method: &str, params: Value, wait: Duration) -> Result<Value, String>;
    fn notify(&self, method: &str, params: Value) -> Result<(), String>;
    fn connected(&self) -> bool;
}

/// Puts a line in front of the model: (identity, text) on `CHANNEL`.
pub type Say = Arc<dyn Fn(&str, &str) + Send + Sync>;

struct Voice {
    /// s<n>, numbered in the order voices were first understood; 0: nothing heard from it yet.
    n: usize,
    name: Option<String>,
    /// The language this person speaks, when the model said so (`call_name`).
    lang: Option<String>,
    listener: Option<bia_speech::Listener>,
    failed: bool,
    /// Phrases that reached the model.
    phrases: usize,
    pieces: u64,
    /// Speech was heard since the last finished phrase.
    open: bool,
    barge: u32,
    last_audio: Instant,
}

impl Voice {
    fn label(&self) -> String {
        self.name.clone().unwrap_or_else(|| format!("s{}", self.n))
    }
}

#[derive(PartialEq)]
enum Intro {
    NotYet,
    Due(Instant),
    Asked,
}

struct Call {
    tab: i64,
    url: String,
    /// The language most people in the call speak, when the model said so (`call_open`).
    lang: Option<String>,
    started: Instant,
    mic: bool,
    hearing: bool,
    gen: u64,
    speaking: bool,
    spoken: Option<(String, Instant)>,
    close_after: bool,
    voices: HashMap<String, Voice>,
    lines: Vec<(String, String)>,
    intro: Intro,
    /// What each frame of the page last reported (`call_status`).
    page: HashMap<i64, Value>,
    /// The loaded page in each frame (its own random id): another one means a reload.
    loaded: HashMap<i64, String>,
    /// Numbers given to voices so far; never reused within a call.
    numbered: usize,
    stalled: bool,
    gone_since: Option<Instant>,
}

impl Call {
    fn echo_ref(&self) -> Option<String> {
        let (text, at) = self.spoken.as_ref()?;
        (self.speaking || at.elapsed().as_secs() < ECHO_SECS).then(|| text.clone())
    }

    fn find(&self, voice: &str) -> Option<&str> {
        let want = voice.trim().trim_start_matches("voice:").to_lowercase();
        self.voices
            .iter()
            .find(|(_, v)| v.n > 0 && format!("s{}", v.n) == want)
            .or_else(|| self.voices.iter().find(|(_, v)| v.name.as_deref().is_some_and(|n| n.to_lowercase() == want)))
            .map(|(k, _)| k.as_str())
    }
}

struct Audio {
    tab: i64,
    src: String,
    rate: u32,
    pcm: Vec<i16>,
}

struct Inner {
    link: Arc<dyn Link>,
    say: Mutex<Option<Say>>,
    call: Mutex<Option<Call>>,
    audio: Mutex<mpsc::Sender<Audio>>,
    /// Where transcripts are kept (`<files>/calls`).
    files: PathBuf,
}

#[derive(Clone)]
pub struct Calls {
    inner: Arc<Inner>,
}

fn clean(name: &str) -> String {
    let s: String = name.chars().filter(|c| !c.is_control() && !matches!(c, '|' | '[' | ']')).collect();
    s.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(NAME_MAX).collect()
}

/// The optional `lang` argument: a language bia can hear.
fn heard_lang(args: &Value) -> Result<Option<String>, String> {
    let lang = args.get("lang").and_then(Value::as_str).unwrap_or("").trim().to_lowercase();
    if lang.is_empty() {
        return Ok(None);
    }
    if !bia_speech::langs::can_hear(&lang) {
        return Err(format!("lang {lang} is not a language bia can hear; use a code like en, ru, de"));
    }
    Ok(Some(lang))
}

fn fmt_dur(d: Duration) -> String {
    format!("{}:{:02}", d.as_secs() / 60, d.as_secs() % 60)
}

fn files() -> PathBuf {
    PathBuf::from(std::env::var("BIA_FILES").unwrap_or_else(|_| "/home/bia/files".into()))
}

impl Calls {
    pub fn start(link: Arc<dyn Link>) -> Self {
        Self::start_in(link, files())
    }

    fn start_in(link: Arc<dyn Link>, files: PathBuf) -> Self {
        let (tx, rx) = mpsc::channel::<Audio>();
        let calls = Calls { inner: Arc::new(Inner { link, say: Mutex::new(None), call: Mutex::new(None), audio: Mutex::new(tx), files }) };
        let worker = calls.clone();
        std::thread::Builder::new()
            .name("chrome-call-audio".into())
            .spawn(move || loop {
                match rx.recv_timeout(Duration::from_millis(300)) {
                    Ok(a) => worker.hear(a),
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
                worker.tick();
            })
            .ok();
        calls
    }

    pub fn set_say(&self, say: Say) {
        *self.inner.say.lock().unwrap() = Some(say);
    }

    fn tell(&self, identity: &str, text: &str) {
        let say = self.inner.say.lock().unwrap().clone();
        if let Some(say) = say {
            say(identity, text);
        }
    }

    fn system(&self, text: &str) {
        self.tell("system", text);
    }

    fn to_page(&self, tab: i64, msg: Value) -> Result<(), String> {
        self.inner.link.notify("voice_to_tab", json!({"tab": tab, "msg": msg}))
    }

    // ---- from the add-on -------------------------------------------------------------------

    /// A method the bridge does not know itself. None: not a call method.
    pub fn from_addon(&self, method: &str, params: &Value) -> Option<Result<Value, String>> {
        let tab = params.get("tab").and_then(Value::as_i64).unwrap_or(-1);
        match method {
            "voice_armed" => {
                let armed = self.inner.call.lock().unwrap().as_ref().is_some_and(|c| c.tab == tab);
                Some(Ok(json!({"armed": armed})))
            }
            "voice_closed" => {
                self.finish(Some(tab), "the tab was closed");
                Some(Ok(Value::Null))
            }
            "voice" => {
                let frame = params.get("frame").and_then(Value::as_i64).unwrap_or(0);
                if let Some(msg) = params.get("d").and_then(Value::as_str).and_then(|d| serde_json::from_str::<Value>(d).ok()) {
                    self.from_page(tab, frame, msg);
                }
                Some(Ok(Value::Null))
            }
            _ => None,
        }
    }

    /// A native host connected: an add-on that restarted during a call learns its tab again.
    pub fn addon_connected(&self) {
        let tab = self.inner.call.lock().unwrap().as_ref().map(|c| c.tab);
        if let Some(tab) = tab {
            let _ = self.inner.link.notify("voice_tabs", json!({"tabs": [tab]}));
        }
    }

    /// A frame of the call's tab shows another page than before (reload, or the meeting moved on):
    /// its voices are gone, and whoever speaks next is a new voice.
    fn reloaded(&self, tab: i64, frame: i64, page: &str) {
        let had = {
            let mut g = self.inner.call.lock().unwrap();
            let Some(call) = g.as_mut().filter(|c| c.tab == tab) else { return };
            match call.loaded.insert(frame, page.to_string()) {
                Some(old) if old != page => {}
                _ => return,
            }
            let own = |key: &str| if frame == 0 { !key.starts_with('f') } else { key.starts_with(&format!("f{frame}.")) };
            let had = call.voices.iter().any(|(k, v)| own(k) && v.phrases > 0);
            call.voices.retain(|k, _| !own(k));
            call.page.remove(&frame);
            call.stalled = false;
            if had && call.intro == Intro::Asked {
                call.intro = Intro::NotYet;
            }
            had
        };
        if had {
            self.system("[call page reloaded] The call's page was loaded again, so the voices start over: names given before no longer apply, and whoever speaks next gets a new number. Rejoin through the page if it asks, then have people say who they are again.");
        }
    }

    fn from_page(&self, tab: i64, frame: i64, msg: Value) {
        let kind = msg.get("t").and_then(Value::as_str).unwrap_or("");
        if let Some(page) = msg.get("page").and_then(Value::as_str).filter(|p| !p.is_empty() && p.len() <= 16) {
            self.reloaded(tab, frame, page);
        }
        if kind == "audio" {
            let Some(pcm) = msg.get("pcm").and_then(Value::as_str).and_then(|p| base64::engine::general_purpose::STANDARD.decode(p).ok()) else {
                return;
            };
            let rate = msg.get("rate").and_then(Value::as_u64).unwrap_or(16_000) as u32;
            let src = msg.get("src").and_then(Value::as_str).unwrap_or("");
            if src.is_empty() || src.len() > 32 || !(8_000..=96_000).contains(&rate) {
                return;
            }
            // two frames of one tab may both count tracks from t1
            let src = if frame == 0 { src.to_string() } else { format!("f{frame}.{src}") };
            let _ = self.inner.audio.lock().unwrap().send(Audio { tab, src, rate, pcm: bia_speech::audio::le_bytes_to_i16(&pcm) });
            return;
        }
        let mut note = None;
        let mut played = false;
        {
            let mut g = self.inner.call.lock().unwrap();
            let Some(call) = g.as_mut().filter(|c| c.tab == tab) else { return };
            match kind {
                "mic" if !call.mic => {
                    call.mic = true;
                    note = Some("[call microphone] The page took bia's voice as its microphone; chrome_call_say speaks into the call once bia has joined.");
                }
                "track" if !call.hearing && msg.get("on").and_then(Value::as_bool) == Some(true) => {
                    call.hearing = true;
                    note = Some("[call audio] bia hears the call now; what people say arrives here as voice:s1, voice:s2 …");
                }
                "state" => {
                    call.page.insert(frame, msg.clone());
                }
                "stalled" => {
                    let on = msg.get("on").and_then(Value::as_bool) == Some(true);
                    if on != call.stalled {
                        call.stalled = on;
                        note = Some(if on {
                            "[call audio stopped] The page's audio stopped running, so bia hears nobody (people may still hear bia). If it does not come back by itself in a few seconds, reload the call's page (chrome_navigate_page type=\"reload\" on its tab) and join again."
                        } else {
                            "[call audio back] The page's audio runs again; bia hears the call."
                        });
                    }
                }
                "played" => {
                    let gen = msg.get("gen").and_then(Value::as_u64).unwrap_or(0);
                    if gen == call.gen && call.speaking {
                        call.speaking = false;
                        played = true;
                    }
                }
                _ => {}
            }
        }
        if let Some(n) = note {
            self.system(n);
        }
        if played {
            self.said();
        }
    }

    /// What bia was saying has played.
    fn said(&self) {
        let closing = self.inner.call.lock().unwrap().as_ref().is_some_and(|c| c.close_after);
        if closing {
            // not on this thread: it is the one that reads the add-on's answer to the close
            let me = self.clone();
            std::thread::spawn(move || me.close_now("bia left"));
        } else {
            self.system("[you finished speaking]");
        }
    }

    // ---- hearing ---------------------------------------------------------------------------

    fn hear(&self, a: Audio) {
        let mut interrupted = false;
        let mut done = None;
        let mut failed = None;
        {
            let mut g = self.inner.call.lock().unwrap();
            let Some(call) = g.as_mut().filter(|c| c.tab == a.tab) else { return };
            let speaking = call.speaking;
            let echo = call.echo_ref();
            let start = call.lang.clone();
            let v = call.voices.entry(a.src.clone()).or_insert_with(|| Voice {
                n: 0,
                name: None,
                lang: None,
                listener: None,
                failed: false,
                phrases: 0,
                pieces: 0,
                open: false,
                barge: 0,
                last_audio: Instant::now(),
            });
            v.pieces += 1;
            v.last_audio = Instant::now();
            if v.listener.is_none() && !v.failed {
                match bia_speech::Listener::with_lang(16_000, v.lang.clone().or(start)) {
                    Ok(l) => v.listener = Some(l),
                    Err(e) => {
                        v.failed = true;
                        failed = Some(e);
                    }
                }
            }
            if let Some(listener) = v.listener.as_mut() {
                let pcm = if a.rate == 16_000 {
                    a.pcm
                } else {
                    let x = bia_speech::audio::resample(&bia_speech::audio::i16_to_f32(&a.pcm), a.rate, 16_000);
                    x.iter().map(|s| (s.clamp(-1.0, 1.0) * 32767.0) as i16).collect()
                };
                let utt = listener.push(&pcm);
                if listener.in_speech() {
                    v.open = true;
                    // talking over bia stops it, like on a phone
                    v.barge = if speaking { v.barge + 1 } else { 0 };
                    if v.barge >= BARGE_PIECES {
                        v.barge = 0;
                        interrupted = true;
                    }
                } else {
                    v.barge = 0;
                }
                if let Some(utt) = utt {
                    v.open = false;
                    done = Some((utt, echo));
                }
            }
        }
        if let Some(e) = failed {
            self.system(&format!("[call speech unavailable: {e}]"));
        }
        if interrupted && self.stop_speaking() {
            // cut off during the goodbye: bia was leaving anyway
            let closing = self.inner.call.lock().unwrap().as_ref().is_some_and(|c| c.close_after);
            if closing {
                self.close_now("bia left");
            } else {
                self.system("[interrupted: someone started talking while you were speaking]");
            }
        }
        if let Some((utt, echo)) = done {
            self.transcribe(a.tab, a.src, utt, echo);
        }
    }

    fn transcribe(&self, tab: i64, src: String, utt: bia_speech::Utterance, echo: Option<String>) {
        let me = self.clone();
        std::thread::spawn(move || match utt.transcribe_call(echo.as_deref(), None, false) {
            Ok(heard) => {
                let text = heard.text();
                if !text.is_empty() {
                    let words: Vec<&str> = heard.runs.iter().map(|r| r.text.as_str()).collect();
                    me.heard(tab, &src, &text, &words.join(" "));
                }
                if let Some(n) = heard.notice {
                    me.system(&n);
                }
            }
            Err(e) => bia_speech::diag::log("chromecall", format!("transcribe failed: {e}")),
        });
    }

    /// A phrase of one voice is ready for the model. `text` carries the `[speech <lang>]` tags.
    fn heard(&self, tab: i64, src: &str, text: &str, words: &str) {
        let mut newcomer = None;
        let label = {
            let mut g = self.inner.call.lock().unwrap();
            let Some(call) = g.as_mut().filter(|c| c.tab == tab) else { return };
            let next = call.numbered + 1;
            let Some(v) = call.voices.get_mut(src) else { return };
            if v.n == 0 {
                v.n = next;
                call.numbered = next;
            }
            v.phrases += 1;
            let label = v.label();
            let first = v.phrases == 1;
            let unnamed = v.name.is_none();
            call.lines.push((label.clone(), words.to_string()));
            let spoke = call.voices.values().filter(|v| v.phrases > 0).count();
            if spoke >= 2 && call.intro == Intro::NotYet {
                call.intro = Intro::Due(Instant::now() + INTRO_DELAY);
            } else if call.intro == Intro::Asked && first && unnamed {
                newcomer = Some(label.clone());
            }
            label
        };
        self.tell(&format!("voice:{label}"), text);
        if let Some(s) = newcomer {
            self.system(&format!(
                "[new voice {s}] Someone not heard before spoke. Ask them to introduce themselves, match them to a person in the addressbook and name the voice: chrome_call_name(voice=\"{s}\", name=\"…\")."
            ));
        }
    }

    fn tick(&self) {
        let mut ask = None;
        let mut stalled = Vec::new();
        let mut gone = false;
        {
            let mut g = self.inner.call.lock().unwrap();
            let Some(call) = g.as_mut() else { return };
            if matches!(call.intro, Intro::Due(at) if Instant::now() >= at) {
                call.intro = Intro::Asked;
                let mut open: Vec<&Voice> = call.voices.values().filter(|v| v.phrases > 0 && v.name.is_none()).collect();
                open.sort_by_key(|v| v.n);
                if !open.is_empty() {
                    let spoke = call.voices.values().filter(|v| v.phrases > 0).count();
                    let list: Vec<String> = open.iter().map(|v| v.label()).collect();
                    ask = Some(format!(
                        "[several voices] {spoke} separate voices have spoken in this call; not named yet: {}. Ask everyone to introduce themselves one at a time, with a short pause between people. Then match each voice to a person in the addressbook and name it: chrome_call_name(voice=\"{}\", name=\"…\"). Their speech then arrives as voice:<name>.",
                        list.join(", "),
                        list[0]
                    ));
                }
            }
            let echo = call.echo_ref();
            for (src, v) in call.voices.iter_mut() {
                if !v.open || v.last_audio.elapsed() < STALLED {
                    continue;
                }
                v.open = false;
                let Some(listener) = v.listener.as_mut() else { continue };
                for _ in 0..12 {
                    if let Some(utt) = listener.push(&[0i16; 1600]) {
                        stalled.push((src.clone(), utt, echo.clone()));
                        break;
                    }
                }
            }
            if self.inner.link.connected() {
                call.gone_since = None;
            } else if call.gone_since.get_or_insert_with(Instant::now).elapsed() >= GONE {
                gone = true;
            }
        }
        let tab = self.inner.call.lock().unwrap().as_ref().map(|c| c.tab);
        if let Some(a) = ask {
            self.system(&a);
        }
        for (src, utt, echo) in stalled {
            if let Some(tab) = tab {
                self.transcribe(tab, src, utt, echo);
            }
        }
        if gone {
            self.finish(None, "Chrome or its add-on stopped");
        }
    }

    // ---- speaking --------------------------------------------------------------------------

    /// Stop what bia is saying (new phrase, interruption, leaving). True: it was speaking.
    fn stop_speaking(&self) -> bool {
        let (tab, was) = {
            let mut g = self.inner.call.lock().unwrap();
            let Some(call) = g.as_mut() else { return false };
            let was = call.speaking;
            call.speaking = false;
            call.gen += 1;
            (call.tab, was)
        };
        let _ = self.to_page(tab, json!({"t": "stop"}));
        was
    }

    /// Tool `call_say`.
    pub fn say(&self, args: &Value) -> Result<String, String> {
        let text = args.get("text").and_then(Value::as_str).unwrap_or("").trim().to_string();
        let lang = args.get("lang").and_then(Value::as_str).unwrap_or("").trim().to_string();
        if text.is_empty() {
            return Err("text is empty".into());
        }
        if lang.is_empty() {
            return Err("lang is required: the language of the text, for example lang=\"en\"".into());
        }
        {
            let g = self.inner.call.lock().unwrap();
            let call = g.as_ref().ok_or("no call is open: chrome_call_open(url) first")?;
            if !call.mic {
                return Err("The page has not asked for a microphone yet, so nobody would hear this. Join the call in the page first (chrome_take_snapshot, then its Join button); chrome_call_status shows what the page reports.".into());
            }
        }
        bia_speech::tts::plan(&bia_speech::registry::current(), &text, &lang)?;
        if self.stop_speaking() {
            self.system("[voice cancelled]");
        }
        let (tab, gen) = {
            let mut g = self.inner.call.lock().unwrap();
            let call = g.as_mut().ok_or("the call ended")?;
            call.speaking = true;
            call.spoken = Some((text.clone(), Instant::now()));
            call.lines.push(("bia".into(), text.clone()));
            (call.tab, call.gen)
        };
        let me = self.clone();
        std::thread::spawn(move || {
            let live = |me: &Calls| me.inner.call.lock().unwrap().as_ref().is_some_and(|c| c.tab == tab && c.gen == gen && c.speaking);
            let mut secs = 0.0f32;
            let r = bia_speech::tts::speak(&text, &lang, &mut |chunk, rate| {
                if !live(&me) {
                    return false;
                }
                secs += chunk.len() as f32 / rate as f32;
                for piece in chunk.chunks(PLAY_PIECE) {
                    let mut bytes = Vec::with_capacity(piece.len() * 2);
                    for s in piece {
                        bytes.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
                    }
                    let pcm = base64::engine::general_purpose::STANDARD.encode(&bytes);
                    if me.to_page(tab, json!({"t": "play", "gen": gen, "rate": rate, "pcm": pcm})).is_err() {
                        return false;
                    }
                }
                true
            });
            if !live(&me) {
                return;
            }
            if let Err(e) = r {
                if let Some(c) = me.inner.call.lock().unwrap().as_mut().filter(|c| c.gen == gen) {
                    c.speaking = false;
                }
                me.system(&format!("[voice failed: {e}]"));
                return;
            }
            let _ = me.to_page(tab, json!({"t": "end", "gen": gen}));
            // the page reports when it has played; should it never do so, speaking still ends
            std::thread::sleep(Duration::from_secs_f32(secs + 6.0));
            let stuck = {
                let mut g = me.inner.call.lock().unwrap();
                match g.as_mut().filter(|c| c.tab == tab && c.gen == gen && c.speaking) {
                    Some(c) => {
                        c.speaking = false;
                        true
                    }
                    None => false,
                }
            };
            if stuck {
                let closing = me.inner.call.lock().unwrap().as_ref().is_some_and(|c| c.close_after);
                if closing {
                    me.close_now("bia left");
                } else {
                    me.system("[you finished speaking] (the page did not confirm that it played; chrome_call_status shows whether its audio is running)");
                }
            }
        });
        Ok("Speaking into the call; [you finished speaking] arrives when it has played.".into())
    }

    // ---- tools -----------------------------------------------------------------------------

    /// Tool `call_open`.
    pub fn open(&self, args: &Value) -> Result<String, String> {
        let url = args.get("url").and_then(Value::as_str).unwrap_or("").trim().to_string();
        if !(url.starts_with("https://") || url.starts_with("http://")) || url.chars().any(char::is_whitespace) {
            return Err("url must be the meeting's link, starting with https://".into());
        }
        let lang = heard_lang(args)?;
        if let Some(c) = self.inner.call.lock().unwrap().as_ref() {
            return Err(format!("A call is already open ({}). chrome_call_close it first.", c.url));
        }
        if !self.inner.link.connected() {
            return Err("Chrome's add-on is not connected: chrome_start first (chrome_status says why).".into());
        }
        bia_speech::warm();
        let r = self.inner.link.request("voice_open", json!({"url": url}), Duration::from_secs(20)).map_err(|e| {
            format!("{e}. The add-on in Chrome may be older than this function: chrome_restart updates it, then try again.")
        })?;
        let tab = r.get("tab").and_then(Value::as_i64).ok_or("the add-on did not say which tab it opened")?;
        *self.inner.call.lock().unwrap() = Some(Call {
            tab,
            url: url.clone(),
            lang,
            started: Instant::now(),
            mic: false,
            hearing: false,
            gen: 0,
            speaking: false,
            spoken: None,
            close_after: false,
            voices: HashMap::new(),
            lines: Vec::new(),
            intro: Intro::NotYet,
            page: HashMap::new(),
            loaded: HashMap::new(),
            numbered: 0,
            stalled: false,
            gone_since: None,
        });
        bia_speech::diag::log("chromecall", format!("call opened {url} tab={tab}"));
        Ok(format!(
            "Opened {url} in a new tab made for the call. Select it (chrome_list_pages, chrome_select_page) and join through the page: chrome_take_snapshot, type bia's name if it asks, click Join or Ask to join. The page gets bia's voice as its microphone and no camera. What people say arrives on {CHANNEL} as voice:s1, voice:s2 … (one per person); speak with chrome_call_say(text, lang); leave with chrome_call_close."
        ))
    }

    /// Tool `call_name`.
    pub fn name(&self, args: &Value) -> Result<String, String> {
        let voice = args.get("voice").and_then(Value::as_str).unwrap_or("");
        let name = clean(args.get("name").and_then(Value::as_str).unwrap_or(""));
        if name.is_empty() {
            return Err("name is empty".into());
        }
        let lang = heard_lang(args)?;
        let mut g = self.inner.call.lock().unwrap();
        let call = g.as_mut().ok_or("no call is open")?;
        let Some(key) = call.find(voice).map(str::to_string) else {
            let mut known: Vec<(usize, String)> = call.voices.values().filter(|v| v.n > 0).map(|v| (v.n, format!("s{}{}", v.n, v.name.as_ref().map(|n| format!(" ({n})")).unwrap_or_default()))).collect();
            known.sort();
            let known: Vec<String> = known.into_iter().map(|(_, s)| s).collect();
            return Err(format!("No voice {voice} in this call. Voices so far: {}.", if known.is_empty() { "none".into() } else { known.join(", ") }));
        };
        let v = call.voices.get_mut(&key).expect("found");
        v.name = Some(name.clone());
        let mut said = format!("s{} is {name} now; their speech arrives as voice:{name}.", v.n);
        if let Some(l) = lang {
            // a new listener that starts in their language
            v.lang = Some(l.clone());
            v.listener = None;
            v.failed = false;
            v.open = false;
            said.push_str(&format!(" Their speech is taken as {l} from now on."));
        }
        Ok(said)
    }

    /// Tool `call_status`.
    pub fn status(&self) -> Result<String, String> {
        let tab = self.inner.call.lock().unwrap().as_ref().map(|c| c.tab);
        let Some(tab) = tab else {
            return Ok(json!({"call": false}).to_string());
        };
        if self.to_page(tab, json!({"t": "stat"})).is_ok() {
            std::thread::sleep(Duration::from_millis(500));
        }
        let g = self.inner.call.lock().unwrap();
        let Some(call) = g.as_ref() else {
            return Ok(json!({"call": false}).to_string());
        };
        let mut voices: Vec<(&String, &Voice)> = call.voices.iter().collect();
        // heard voices first, in their order; then audio nobody spoke on yet
        voices.sort_by_key(|(k, v)| (v.n == 0, v.n, (*k).clone()));
        let voices: Vec<Value> = voices
            .iter()
            .map(|(key, v)| {
                json!({
                    "voice": if v.n > 0 { json!(format!("s{}", v.n)) } else { Value::Null },
                    "name": v.name,
                    "lang": v.lang,
                    // c…: a speaker the conference server names; t…/m…: an audio track; a…: one of the page's audio outputs
                    "source": key,
                    "phrases": v.phrases,
                    "seconds_received": v.pieces / 10,
                    "last_audio_seconds_ago": v.last_audio.elapsed().as_secs(),
                })
            })
            .collect();
        let mut page: Vec<(&i64, &Value)> = call.page.iter().collect();
        page.sort_by_key(|(f, _)| **f);
        Ok(json!({
            "call": true,
            "url": call.url,
            "lang": call.lang,
            "tab": call.tab,
            "minutes": call.started.elapsed().as_secs() / 60,
            "microphone_taken_by_page": call.mic,
            "hearing": call.hearing && !call.stalled,
            "page_audio_stopped": call.stalled,
            "speaking": call.speaking,
            "voices": voices,
            "page": page.iter().map(|(_, v)| (*v).clone()).collect::<Vec<_>>(),
            "addon_connected": self.inner.link.connected(),
            "speech": bia_speech::status(),
        })
        .to_string())
    }

    /// Tool `call_close`.
    pub fn close(&self) -> Result<String, String> {
        let speaking = {
            let mut g = self.inner.call.lock().unwrap();
            let call = g.as_mut().ok_or("no call is open")?;
            if call.speaking {
                call.close_after = true;
            }
            call.speaking
        };
        if speaking {
            // the goodbye plays out first; `say`'s own timer covers a page that never reports
            return Ok("Leaving as soon as what you are saying has played.".into());
        }
        self.close_now("bia left");
        Ok("Left the call and closed its tab.".into())
    }

    fn close_now(&self, why: &str) {
        let tab = self.inner.call.lock().unwrap().as_ref().map(|c| c.tab);
        let Some(tab) = tab else { return };
        self.stop_speaking();
        let _ = self.inner.link.request("voice_close", json!({"tab": tab}), Duration::from_secs(5));
        self.finish(Some(tab), why);
    }

    /// The call is over: tell the model and keep the transcript.
    fn finish(&self, tab: Option<i64>, why: &str) {
        let call = {
            let mut g = self.inner.call.lock().unwrap();
            if tab.is_some() && g.as_ref().map(|c| c.tab) != tab {
                return;
            }
            let Some(call) = g.take() else { return };
            call
        };
        let dur = fmt_dur(call.started.elapsed());
        bia_speech::diag::log("chromecall", format!("call ended {} {dur} ({why})", call.url));
        let mut line = format!("[call ended] ({dur}, {why})");
        if !call.lines.is_empty() {
            let mut text = format!("Call {} · {dur}\n", call.url);
            for (who, said) in &call.lines {
                text.push_str(&format!("\n{who}: {said}"));
            }
            text.push('\n');
            let dir = self.inner.files.join("calls");
            let path = dir.join(format!("call-{}.txt", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)));
            if std::fs::create_dir_all(&dir).is_ok() && std::fs::write(&path, text).is_ok() {
                line.push_str(&format!(" Everything said is in {}.", path.display()));
            }
        }
        self.system(&line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Fake {
        sent: Mutex<Vec<(String, Value)>>,
    }

    impl Link for Fake {
        fn request(&self, method: &str, params: Value, _wait: Duration) -> Result<Value, String> {
            self.sent.lock().unwrap().push((method.into(), params));
            Ok(json!({"tab": 7, "closed": true}))
        }
        fn notify(&self, method: &str, params: Value) -> Result<(), String> {
            self.sent.lock().unwrap().push((method.into(), params));
            Ok(())
        }
        fn connected(&self) -> bool {
            true
        }
    }

    type Lines = Arc<Mutex<Vec<(String, String)>>>;

    fn setup() -> (Calls, Arc<Fake>, Lines) {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!("bia-chromecall-{}-{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let fake = Arc::new(Fake::default());
        let calls = Calls::start_in(fake.clone(), dir);
        let lines: Lines = Arc::default();
        let sink = lines.clone();
        calls.set_say(Arc::new(move |who: &str, text: &str| sink.lock().unwrap().push((who.to_string(), text.to_string()))));
        (calls, fake, lines)
    }

    /// A voice that has been heard, without the speech models.
    fn voice(calls: &Calls, src: &str) {
        let mut g = calls.inner.call.lock().unwrap();
        let call = g.as_mut().unwrap();
        call.voices.entry(src.to_string()).or_insert_with(|| Voice {
            n: 0,
            name: None,
            lang: None,
            listener: None,
            failed: true,
            phrases: 0,
            pieces: 0,
            open: false,
            barge: 0,
            last_audio: Instant::now(),
        });
    }

    fn wait_for(lines: &Lines, what: &str) -> String {
        for _ in 0..200 {
            if let Some((_, t)) = lines.lock().unwrap().iter().find(|(_, t)| t.contains(what)) {
                return t.clone();
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("no line with {what:?}: {:?}", lines.lock().unwrap());
    }

    #[test]
    fn voices_are_introduced_and_named() {
        let (calls, fake, lines) = setup();
        assert!(calls.open(&json!({"url": "meet.google.com/abc"})).is_err(), "a link, not a host");
        assert!(calls.say(&json!({"text": "hi", "lang": "en"})).unwrap_err().contains("no call is open"));
        calls.open(&json!({"url": "https://meet.google.com/abc-defg-hij"})).unwrap();
        assert!(calls.open(&json!({"url": "https://meet.google.com/other"})).unwrap_err().contains("already open"));
        assert_eq!(fake.sent.lock().unwrap()[0].0, "voice_open");
        // only its own tab is a call tab
        assert_eq!(calls.from_addon("voice_armed", &json!({"tab": 7})).unwrap().unwrap(), json!({"armed": true}));
        assert_eq!(calls.from_addon("voice_armed", &json!({"tab": 8})).unwrap().unwrap(), json!({"armed": false}));
        assert!(calls.from_addon("passkey_get", &json!({})).is_none());

        // the page has not asked for a microphone: nobody would hear bia
        assert!(calls.say(&json!({"text": "hi", "lang": "en"})).unwrap_err().contains("microphone"));
        assert!(calls.say(&json!({"text": "hi"})).unwrap_err().contains("lang is required"));
        calls.from_addon("voice", &json!({"tab": 7, "frame": 0, "d": json!({"t": "mic"}).to_string()}));
        calls.from_addon("voice", &json!({"tab": 7, "frame": 0, "d": json!({"t": "track", "src": "t1", "on": true}).to_string()}));
        calls.from_addon("voice", &json!({"tab": 9, "frame": 0, "d": json!({"t": "mic"}).to_string()}));
        wait_for(&lines, "[call microphone]");
        wait_for(&lines, "[call audio]");
        assert_eq!(lines.lock().unwrap().len(), 2, "each said once, and not for another tab");

        // one voice: nothing to tell apart. Audio nobody spoke on takes no number.
        voice(&calls, "a-silent");
        voice(&calls, "c100");
        calls.heard(7, "c100", "[speech en] Hello everyone", "Hello everyone");
        assert_eq!(lines.lock().unwrap().last().unwrap(), &("voice:s1".to_string(), "[speech en] Hello everyone".to_string()));
        assert!(calls.inner.call.lock().unwrap().as_ref().unwrap().intro == Intro::NotYet);

        // a second voice: after a pause the model is asked to have people introduce themselves
        voice(&calls, "c200");
        calls.heard(7, "c200", "[speech en] Hi", "Hi");
        match &mut calls.inner.call.lock().unwrap().as_mut().unwrap().intro {
            Intro::Due(at) => *at = Instant::now(),
            _ => panic!("introductions not planned"),
        }
        let ask = wait_for(&lines, "[several voices]");
        assert!(ask.contains("2 separate voices") && ask.contains("s1, s2") && ask.contains("one at a time"), "{ask}");

        // named: speech arrives under the name; a name cannot pose as a system line
        assert!(calls.name(&json!({"voice": "s9", "name": "X"})).unwrap_err().contains("s1, s2"));
        assert!(calls.name(&json!({"voice": "s2", "name": "  Anna |[system]\n Lee "})).unwrap().contains("Anna system Lee"));
        calls.name(&json!({"voice": "voice:s2", "name": "Anna"})).unwrap();
        calls.heard(7, "c200", "[speech en] I am Anna", "I am Anna");
        assert_eq!(lines.lock().unwrap().last().unwrap().0, "voice:Anna");
        assert!(calls.name(&json!({"voice": "anna", "name": "Anna Lee", "lang": "xx-nope"})).unwrap_err().contains("not a language"));
        assert!(calls.name(&json!({"voice": "anna", "name": "Anna Lee", "lang": "de"})).unwrap().contains("taken as de"));
        assert_eq!(calls.inner.call.lock().unwrap().as_ref().unwrap().voices["c200"].lang.as_deref(), Some("de"));

        // someone joins later: asked for just them, once
        voice(&calls, "c300");
        calls.heard(7, "c300", "[speech de] Hallo", "Hallo");
        let late = wait_for(&lines, "[new voice s3]");
        assert!(late.contains("chrome_call_name(voice=\"s3\""), "{late}");
        calls.heard(7, "c300", "[speech de] Noch etwas", "Noch etwas");
        assert_eq!(lines.lock().unwrap().iter().filter(|(_, t)| t.contains("[new voice")).count(), 1);

        // the page's audio stops and comes back: said once each
        let page = |m: Value| calls.from_addon("voice", &json!({"tab": 7, "frame": 0, "d": m.to_string()}));
        page(json!({"t": "stalled", "on": true, "page": "p1"}));
        page(json!({"t": "stalled", "on": true, "page": "p1"}));
        wait_for(&lines, "[call audio stopped]");
        assert!(calls.status().unwrap().contains("\"page_audio_stopped\":true"));
        page(json!({"t": "stalled", "on": false, "page": "p1"}));
        wait_for(&lines, "[call audio back]");
        assert_eq!(lines.lock().unwrap().iter().filter(|(_, t)| t.contains("[call audio stopped]")).count(), 1);

        // the page is loaded again: its voices are gone, numbers go on, people are asked again
        page(json!({"t": "track", "src": "a1", "on": true, "page": "p2"}));
        wait_for(&lines, "[call page reloaded]");
        assert!(calls.name(&json!({"voice": "s1", "name": "X"})).is_err(), "old voices are gone");
        voice(&calls, "a1");
        calls.heard(7, "a1", "[speech en] Back again", "Back again");
        assert_eq!(lines.lock().unwrap().last().unwrap().0, "voice:s4");
        voice(&calls, "a2");
        calls.heard(7, "a2", "[speech en] Me too", "Me too");
        assert!(matches!(calls.inner.call.lock().unwrap().as_ref().unwrap().intro, Intro::Due(_)), "introductions are asked for again");

        // the tab closed: the call is over and what was said is kept
        calls.from_addon("voice_closed", &json!({"tab": 3}));
        assert!(calls.inner.call.lock().unwrap().is_some(), "another tab closing is not the call");
        calls.from_addon("voice_closed", &json!({"tab": 7}));
        let end = wait_for(&lines, "[call ended]");
        let path = end.split(" is in ").nth(1).unwrap().trim_end_matches('.');
        let text = std::fs::read_to_string(path).unwrap();
        assert!(text.contains("s1: Hello everyone") && text.contains("Anna: I am Anna") && text.contains("s3: Hallo"), "{text}");
        assert!(calls.close().is_err());
        assert_eq!(calls.status().unwrap(), json!({"call": false}).to_string());
    }

    #[test]
    fn played_ends_speaking_and_a_goodbye_closes() {
        let (calls, fake, lines) = setup();
        calls.open(&json!({"url": "https://example.com/room"})).unwrap();
        {
            let mut g = calls.inner.call.lock().unwrap();
            let c = g.as_mut().unwrap();
            c.mic = true;
            c.speaking = true;
            c.gen = 4;
        }
        // an old phrase's report changes nothing
        calls.from_addon("voice", &json!({"tab": 7, "d": json!({"t": "played", "gen": 3}).to_string()}));
        assert!(calls.inner.call.lock().unwrap().as_ref().unwrap().speaking);
        calls.from_addon("voice", &json!({"tab": 7, "d": json!({"t": "played", "gen": 4}).to_string()}));
        wait_for(&lines, "[you finished speaking]");

        // leaving while speaking waits for the goodbye
        calls.inner.call.lock().unwrap().as_mut().unwrap().speaking = true;
        assert!(calls.close().unwrap().contains("as soon as"));
        assert!(calls.inner.call.lock().unwrap().is_some());
        calls.from_addon("voice", &json!({"tab": 7, "d": json!({"t": "played", "gen": 4}).to_string()}));
        wait_for(&lines, "[call ended]");
        assert!(fake.sent.lock().unwrap().iter().any(|(m, p)| m == "voice_close" && p["tab"] == 7));
    }

    /// Real models: BIA_SPEECH_DIR=<speech tree> cargo test -p bia-mcp-chrome -- --ignored chrome_call
    #[test]
    #[ignore = "needs the speech models (BIA_SPEECH_DIR)"]
    fn chrome_call_roundtrip() {
        let (calls, fake, lines) = setup();
        std::env::set_var("BIA_SPEECH_STORE", std::env::temp_dir().join(format!("bia-chromecall-speech-{}", std::process::id())));
        calls.open(&json!({"url": "https://example.com/room"})).unwrap();
        let send = |src: &str, pcm: &[i16]| {
            let bytes: Vec<u8> = pcm.iter().flat_map(|s| s.to_le_bytes()).collect();
            let d = json!({"t": "audio", "src": src, "rate": 16000, "pcm": base64::engine::general_purpose::STANDARD.encode(bytes)}).to_string();
            calls.from_addon("voice", &json!({"tab": 7, "frame": 0, "d": d}));
        };
        let speak = |src: &str, text: &str| {
            let (x, rate) = bia_speech::tts::synthesize(text, "en").unwrap();
            let x16 = bia_speech::audio::resample(&x, rate, 16_000);
            let pcm: Vec<i16> = x16.iter().map(|s| (s * 32767.0) as i16).collect();
            for piece in pcm.chunks(1600) {
                send(src, piece);
            }
        };
        // two people, each on their own source; the first one's audio just stops (no trailing quiet)
        speak("c1", "Good morning, this is the weekly planning meeting.");
        let first = wait_for(&lines, "planning");
        assert!(first.starts_with("[speech en]"), "{first}");
        assert_eq!(lines.lock().unwrap().iter().find(|(_, t)| t.contains("planning")).unwrap().0, "voice:s1");
        speak("c2", "Thank you. I would like to talk about the budget.");
        for _ in 0..10 {
            send("c2", &[0i16; 1600]);
        }
        wait_for(&lines, "budget");
        assert_eq!(lines.lock().unwrap().iter().find(|(_, t)| t.contains("budget")).unwrap().0, "voice:s2");

        // bia answers: audio goes to the page in frames Chrome accepts, then the end mark
        calls.from_addon("voice", &json!({"tab": 7, "d": json!({"t": "mic"}).to_string()}));
        calls.say(&json!({"text": "Good morning. I am listening.", "lang": "en"})).unwrap();
        let t = Instant::now();
        loop {
            let sent = fake.sent.lock().unwrap();
            let msgs: Vec<&Value> = sent.iter().filter(|(m, _)| m == "voice_to_tab").map(|(_, p)| &p["msg"]).collect();
            if msgs.iter().any(|m| m["t"] == "end") {
                let plays: Vec<&&Value> = msgs.iter().filter(|m| m["t"] == "play").collect();
                assert!(!plays.is_empty());
                assert!(plays.iter().all(|m| m["pcm"].as_str().unwrap().len() < 20_000));
                break;
            }
            drop(sent);
            assert!(t.elapsed().as_secs() < 60, "no end mark");
            std::thread::sleep(Duration::from_millis(100));
        }
        calls.from_addon("voice", &json!({"tab": 7, "d": json!({"t": "played", "gen": 1}).to_string()}));
        wait_for(&lines, "[you finished speaking]");
        calls.close().unwrap();
        wait_for(&lines, "[call ended]");
    }
}
