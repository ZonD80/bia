//! bia's ears and voice in phone calls, as in WhatsApp/Telegram calls. Android 8.1 cannot
//! record calls through its API, and its audio HAL sends Telephony Tx to a dead proxy, so this
//! drives the Qualcomm sound card directly (root in the chroot sees /dev/snd):
//! - hearing: "MultiMedia1 Mixer VOC_REC_DL" + arecord on hw:0,0 (the caller's side) into
//!   bia-speech; utterances arrive as text on the phone:<number> chat;
//! - speaking: "Incall_Music Audio Mixer MultiMedia9" + aplay on plughw:0,27 (into the uplink);
//! - only bia talks and listens: the phone's microphones (DEC1-5 Volume) and earpiece/speaker
//!   (RX1-3 Digital Volume) go to 0 for the call and are restored after it.
//! (Found on a Mi A2, sdm660; both Incall_Music mixers reach the call, but not together.)

use bia_mcp::{members, Ctx};
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const RATE: u32 = 16_000;
/// Our own speech counts as a possible echo this long after it was played.
const ECHO_SECS: u64 = 3;
const CARD: &str = "0";
const CAPTURE_PCM: &str = "hw:0,0"; // MultiMedia1
const PLAY_PCM: &str = "plughw:0,27"; // MultiMedia9
const REC_MIXER: &str = "MultiMedia1 Mixer VOC_REC_DL";
const UL_MIXER: &str = "MultiMedia1 Mixer VOC_REC_UL";
const MUSIC_MIXER: &str = "Incall_Music Audio Mixer MultiMedia9";
const MUTED: &[&str] = &[
    "DEC1 Volume", "DEC2 Volume", "DEC3 Volume", "DEC4 Volume", "DEC5 Volume",
    "RX1 Digital Volume", "RX2 Digital Volume", "RX3 Digital Volume",
];

/// The number of the call in progress (connected), if any.
fn active() -> &'static Mutex<Option<String>> {
    static A: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    A.get_or_init(|| Mutex::new(None))
}
pub fn active_number() -> Option<String> {
    active().lock().unwrap().clone()
}

/// What bia said in calls recently: (chat, text, when) — echo filter and SMS dedup.
fn spoken() -> &'static Mutex<Vec<(String, String, Instant)>> {
    static S: OnceLock<Mutex<Vec<(String, String, Instant)>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(Vec::new()))
}
fn note_spoken(chat: &str, text: &str) {
    let mut s = spoken().lock().unwrap();
    s.retain(|(_, _, t)| t.elapsed() < Duration::from_secs(600));
    s.push((chat.to_string(), text.to_string(), Instant::now()));
}
/// True when `text` was spoken into the call with `chat` in the last 10 minutes (the agent's
/// text copy of it must then not go out as an SMS).
pub fn was_spoken(chat: &str, text: &str) -> bool {
    spoken().lock().unwrap().iter().any(|(c, t, w)| c == chat && t.trim() == text.trim() && w.elapsed() < Duration::from_secs(600))
}
fn echo_ref() -> Option<String> {
    let s = spoken().lock().unwrap();
    s.iter().rev().find(|(_, _, w)| w.elapsed() < Duration::from_secs(ECHO_SECS + 30)).map(|(_, t, _)| t.clone())
}

fn log(msg: impl AsRef<str>) {
    bia_speech::diag::log("phone", msg);
}

fn amixer_set(name: &str, value: &str) -> bool {
    Command::new("amixer").args(["-q", "-c", CARD, "cset", &format!("name={name}"), value])
        .stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
}
fn amixer_get(name: &str) -> Option<String> {
    let o = Command::new("amixer").args(["-c", CARD, "cget", &format!("name={name}")]).stderr(Stdio::null()).output().ok()?;
    let t = String::from_utf8_lossy(&o.stdout);
    t.lines().find_map(|l| l.trim().strip_prefix(": values=").map(str::to_string))
}

/// Levels saved at the start of the call, restored at its end.
fn saved() -> &'static Mutex<Vec<(String, String)>> {
    static S: OnceLock<Mutex<Vec<(String, String)>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(Vec::new()))
}
static RECORDER: Mutex<Option<u32>> = Mutex::new(None);
static PLAYER: Mutex<Option<u32>> = Mutex::new(None);

/// A call connected: from now on only bia hears and speaks.
pub fn started(ctx: &Ctx, number: &str) {
    *active().lock().unwrap() = Some(number.to_string());
    RINGING.store(false, std::sync::atomic::Ordering::Relaxed);
    {
        let mut sv = saved().lock().unwrap();
        if sv.is_empty() {
            for c in MUTED {
                if let Some(v) = amixer_get(c) {
                    sv.push((c.to_string(), v));
                }
            }
        }
    }
    for c in MUTED {
        amixer_set(c, "0");
    }
    amixer_set(REC_MIXER, "1");
    amixer_set(MUSIC_MIXER, "1");
    // Debugging only, opt in: with files/phone/debug-call-uplink present, what this phone
    // sends (the uplink) is mixed into what bia records, to hear what the caller hears.
    if crate::files_dir().join("phone/debug-call-uplink").exists() {
        amixer_set(UL_MIXER, "1");
        log("debug: recording the uplink too");
    }
    log(format!("call with {number}: phone mic and earpiece muted, in-call record and music on"));
    // normally loaded while it rang; a call that connected without ringing here loads now
    if plan().lock().unwrap().as_ref().map(|p| p.number.as_str()) != Some(number) {
        preparing(number, None);
    }
    let ctx = ctx.clone();
    let num = number.to_string();
    let _ = std::thread::Builder::new().name("call-listen".into()).spawn(move || listen(ctx, num));
}

/// The coming (or current) call; its models are loaded for the languages [`preparing`]
/// picked: the one bia fixed, else the person's addressbook language, else the default.
pub struct Plan {
    pub number: String,
    /// bia fixed the call's language: every phrase is heard in it, no detection.
    pub forced: Option<String>,
}

pub fn plan() -> &'static Mutex<Option<Plan>> {
    static P: Mutex<Option<Plan>> = Mutex::new(None);
    &P
}

/// The phone rings or bia is about to dial `number`: load the speech models and voice of
/// the call's language now, so the first words are heard and answered without the load on
/// top (5-10 s on a phone). `lang` fixes the call's language. The languages it loads.
pub fn preparing(number: &str, lang: Option<&str>) -> Vec<String> {
    let lang = lang.filter(|l| !l.trim().is_empty());
    let langs = bia_speech::call_langs(&format!("phone:{number}"), lang);
    let forced = lang.and_then(|_| langs.first().cloned());
    *plan().lock().unwrap() = Some(Plan { number: number.to_string(), forced: forced.clone() });
    let (lock, _) = &*WARM;
    *lock.lock().unwrap() += 1;
    let list = langs.clone();
    let spawned = std::thread::Builder::new().name("call-warm".into()).spawn(move || {
        let t = Instant::now();
        bia_speech::warm_call(&list, forced.is_some());
        log(format!(
            "call models for {} loaded in {:.1}s{}",
            if list.is_empty() { "any language".to_string() } else { list.join(", ") },
            t.elapsed().as_secs_f64(),
            if forced.is_some() { " (language fixed)" } else { "" }
        ));
        let (lock, cv) = &*WARM;
        *lock.lock().unwrap() -= 1;
        cv.notify_all();
        make_acks(&list);
    });
    if spawned.is_err() {
        *WARM.0.lock().unwrap() -= 1;
    }
    langs
}

// ---- acknowledgments ----

/// Language -> (phrase, call audio) of each acknowledgment, made while the phone rings.
fn ack_audio() -> &'static Mutex<std::collections::HashMap<String, Vec<(String, Vec<u8>)>>> {
    static A: OnceLock<Mutex<std::collections::HashMap<String, Vec<(String, Vec<u8>)>>>> = OnceLock::new();
    A.get_or_init(Default::default)
}

/// Call audio (telephone band, 16 kHz) of each acknowledgment of `lang` in its voice now.
fn ack_list(lang: &str) -> Vec<(String, Vec<u8>)> {
    let mut list = Vec::new();
    for p in bia_speech::acks::phrases(lang) {
        let mut band = Telephone::new();
        let mut pcm: Vec<i16> = Vec::new();
        let ok = bia_speech::tts::speak(&p, lang, &mut |chunk, rate| {
            pcm.extend(band.run(&resample(chunk, rate)));
            true
        });
        if ok.is_ok() && !pcm.is_empty() {
            list.push((p, pcm.iter().flat_map(|s| s.to_le_bytes()).collect()));
        }
    }
    list
}

/// Make the acknowledgments of the call's languages (while the phone rings); another
/// language heard in the call gets its own when it first comes up.
fn make_acks(langs: &[String]) {
    let t = Instant::now();
    let mut n = 0;
    for l in langs {
        let list = ack_list(l);
        n += list.len();
        ack_audio().lock().unwrap().insert(l.clone(), list);
    }
    log(format!("{n} acknowledgments made in {:.1}s", t.elapsed().as_secs_f64()));
}

/// A language learned (or its phrases written) during the call: make its audio in the
/// background; the next phrase in it gets one.
fn remake_acks(lang: &str) {
    static BUSY: Mutex<Vec<String>> = Mutex::new(Vec::new());
    {
        let mut b = BUSY.lock().unwrap();
        if b.iter().any(|l| l == lang) {
            return;
        }
        b.push(lang.to_string());
    }
    let l = lang.to_string();
    let _ = std::thread::Builder::new().name("call-acks".into()).spawn(move || {
        let list = ack_list(&l);
        log(format!("{} acknowledgments in {l} made", list.len()));
        ack_audio().lock().unwrap().insert(l.clone(), list);
        BUSY.lock().unwrap().retain(|x| *x != l);
    });
}

/// Words in a phrase worth acknowledging: a "hello" or "thanks" is answered quickly
/// enough without one.
const ACK_MIN_WORDS: usize = 3;

/// The caller said `text` in `lang`: say a short acknowledgment at once, unless bia is
/// speaking already. A different one than last time.
fn acknowledge(ctx: &Ctx, chat: &str, lang: &str, text: &str) {
    if text.split_whitespace().count() < ACK_MIN_WORDS || UNSPOKEN.load(std::sync::atomic::Ordering::SeqCst) > 0 {
        return;
    }
    static LAST: Mutex<usize> = Mutex::new(usize::MAX);
    let pick = {
        let all = ack_audio().lock().unwrap();
        // made for other phrases (bia wrote new ones), or not yet (learned mid-call)
        let want = bia_speech::acks::phrases(lang);
        let have: Vec<&str> = all.get(lang).map(|l| l.iter().map(|(p, _)| p.as_str()).collect()).unwrap_or_default();
        if have != want.iter().map(String::as_str).collect::<Vec<_>>() {
            drop(all);
            if !want.is_empty() {
                remake_acks(lang);
            }
            return;
        }
        let Some(list) = all.get(lang).filter(|l| !l.is_empty()) else { return };
        let mut last = LAST.lock().unwrap();
        let n = list.len();
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos() as usize).unwrap_or(0);
        let mut i = seed % n;
        if i == *last && n > 1 {
            i = (i + 1) % n;
        }
        *last = i;
        list[i].clone()
    };
    note_spoken(chat, &pick.0);
    UNSPOKEN.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let gen = SAY_GEN.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    let job = Say { chat: chat.into(), text: pick.0, lang: lang.into(), ctx: ctx.clone(), gen, ack: Some(pick.1) };
    if speaker().send(job).is_err() {
        UNSPOKEN.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Loads of call models still running (the coming call is ready at 0).
static WARM: std::sync::LazyLock<(Mutex<usize>, std::sync::Condvar)> = std::sync::LazyLock::new(|| (Mutex::new(0), std::sync::Condvar::new()));

/// Wait (at most `max`) until the models [`preparing`] loads are in: an incoming call is
/// answered, and an outgoing one dialled, only then, so the first words are heard at once.
/// Seconds waited.
pub fn wait_ready(max: Duration) -> f64 {
    let t = Instant::now();
    let (lock, cv) = &*WARM;
    let g = lock.lock().unwrap();
    let _ = cv.wait_timeout_while(g, max, |loading| *loading > 0);
    t.elapsed().as_secs_f64()
}

/// The models of a call are freed soon after it: they take hundreds of MB of a phone's RAM
/// (Parakeet alone ~700 MB). Not while another call is on or ringing.
fn unload_after_call() {
    let _ = std::thread::Builder::new().name("call-unload".into()).spawn(|| {
        // a reply still being spoken when they hung up stops by itself first
        std::thread::sleep(Duration::from_secs(10));
        if active_number().is_none() && !RINGING.load(std::sync::atomic::Ordering::Relaxed) {
            let n = bia_speech::unload_now();
            log(format!("call over: {n} speech model(s) unloaded"));
        }
    });
}

/// A call is ringing in or out (models being loaded for it).
pub static RINGING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The call ended: give the phone back its microphone and earpiece.
pub fn ended() {
    let was = active().lock().unwrap().take();
    for p in [&RECORDER, &PLAYER] {
        if let Some(pid) = p.lock().unwrap().take() {
            let _ = Command::new("kill").arg(pid.to_string()).status();
        }
    }
    amixer_set(REC_MIXER, "0");
    amixer_set(MUSIC_MIXER, "0");
    amixer_set(UL_MIXER, "0");
    for (c, v) in saved().lock().unwrap().drain(..) {
        amixer_set(&c, &v);
    }
    if was.is_some() {
        log("call ended: phone mic and earpiece restored");
    }
    *plan().lock().unwrap() = None;
    RINGING.store(false, std::sync::atomic::Ordering::Relaxed);
    unload_after_call();
}

fn listen(ctx: Ctx, num: String) {
    let channel = format!("phone:{num}");
    let forced = plan().lock().unwrap().as_ref().filter(|p| p.number == num).and_then(|p| p.forced.clone());
    let mut listener = match bia_speech::Listener::with_lang(RATE, bia_speech::contacts::lang_for(&channel)) {
        Ok(l) => l,
        Err(e) => {
            ctx.inbound("system", &channel, "", &format!("[call] bia can not hear this call: speech unavailable ({e})"));
            return;
        }
    };
    if let Some(f) = &forced {
        match listener.force_lang(f) {
            Ok(()) => log(format!("the call is heard in {f} only")),
            Err(e) => log(format!("call language {f} not fixed: {e}")),
        }
    }
    // phone audio: narrowband with dropouts; 0.7 s split sentences mid-way
    listener.set_gap(1.2);
    let mut child = match Command::new("arecord")
        .args(["-q", "-D", CAPTURE_PCM, "-f", "S16_LE", "-r", "16000", "-c", "1", "-t", "raw"])
        .stdout(Stdio::piped()).stderr(Stdio::null()).spawn()
    {
        Ok(c) => c,
        Err(e) => {
            ctx.inbound("system", &channel, "", &format!("[call] bia can not hear this call: arecord: {e}"));
            return;
        }
    };
    *RECORDER.lock().unwrap() = Some(child.id());
    let mut out = child.stdout.take().unwrap();
    log(format!("listening to the call with {num}"));
    // Debugging only, opt in: with files/phone/debug-call-audio present the caller's audio
    // (16 kHz mono s16le) is kept in files/speech/logs/ for `bia-mcp-phone --replay`.
    let mut dump = debug_dump(&num);
    let mut buf = vec![0u8; 1920]; // 60 ms
    let mut have = 0usize;
    let (mut frames, mut peak) = (0u64, 0i32);
    while active_number().as_deref() == Some(num.as_str()) {
        match out.read(&mut buf[have..]) {
            Ok(0) => break,
            Ok(n) => {
                have += n;
                if have < buf.len() {
                    continue;
                }
                have = 0;
                if let Some(f) = dump.as_mut() {
                    let _ = f.write_all(&buf);
                }
                let frame: Vec<i16> = buf.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
                frames += 1;
                peak = peak.max(frame.iter().map(|x| (*x as i32).abs()).max().unwrap_or(0));
                if frames % 100 == 0 {
                    log(format!("call {num}: {frames} frames, peak {peak}"));
                    peak = 0;
                }
                if let Some(utt) = listener.push(&frame) {
                    log(format!("utterance {:.2}s", utt.secs()));
                    let (ctx, channel, num) = (ctx.clone(), channel.clone(), num.clone());
                    std::thread::spawn(move || match utt.transcribe(echo_ref().as_deref(), Some(&channel)) {
                        Ok(heard) => {
                            let text = heard.text();
                            log(format!("heard: {text}"));
                            if let Some(run) = heard.runs.iter().rev().find(|r| r.installed && !r.text.trim().is_empty()) {
                                let words: String = heard.runs.iter().map(|r| r.text.as_str()).collect::<Vec<_>>().join(" ");
                                acknowledge(&ctx, &num, &run.lang, &words);
                            }
                            if !text.is_empty() {
                                bia_speech::calllog::add(&channel, &num, &text);
                                let from = members::From { sender: channel.clone(), ..Default::default() };
                                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
                                ctx.inbound_by(&num, &channel, "", &text, now, &from);
                            }
                            if let Some(n) = heard.notice {
                                ctx.inbound("system", &channel, "", &n);
                            }
                        }
                        Err(e) => log(format!("transcribe failed: {e}")),
                    });
                }
            }
            Err(_) => break,
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    log(format!("stopped listening to {num} after {frames} frames"));
}

fn debug_dump(num: &str) -> Option<std::fs::File> {
    let files = crate::files_dir();
    if !files.join("phone/debug-call-audio").exists() {
        return None;
    }
    let dir = files.join("speech/logs");
    let _ = std::fs::create_dir_all(&dir);
    let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let p = dir.join(format!("call-{}-{at}.pcm", num.trim_start_matches('+')));
    log(format!("debug: keeping the call audio in {}", p.display()));
    std::fs::File::create(p).ok()
}

/// `bia-mcp-phone --replay <file.pcm> [gap]`: run recorded call audio (16 kHz s16le) through
/// the call listener and recogniser exactly as in a call, printing each utterance.
pub fn replay(path: &str, gap: f32) -> Result<(), String> {
    let raw = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    // as when the phone rings: the call models load before anyone speaks
    bia_speech::warm_call_models();
    let mut listener = bia_speech::Listener::with_lang(RATE, None)?;
    // BIA_REPLAY_LANG=es: as a call whose language bia fixed
    if let Ok(l) = std::env::var("BIA_REPLAY_LANG") {
        listener.force_lang(&l)?;
    }
    listener.set_gap(gap);
    let mut fed = 0usize;
    // BIA_REPLAY_REALTIME=1: feed at the pace of a call (a streaming model decodes along)
    let realtime = std::env::var("BIA_REPLAY_REALTIME").is_ok_and(|v| v == "1");
    let t0 = std::time::Instant::now();
    // like a call: each utterance is transcribed on its own thread, in order
    let mut prev: Option<std::thread::JoinHandle<()>> = None;
    let mut take = |u: bia_speech::Utterance, at: usize| {
        let before = prev.take();
        prev = Some(std::thread::spawn(move || {
            let secs = u.secs();
            let t = std::time::Instant::now();
            if let Some(b) = before {
                let _ = b.join();
            }
            let r = u.transcribe(None, Some("replay"));
            let ms = t.elapsed().as_millis();
            match r {
                Ok(h) => println!("{:7.2}s  utterance {secs:.2}s  text in {ms} ms  {}", at as f32 / RATE as f32, h.text()),
                Err(e) => println!("{:7.2}s  utterance {secs:.2}s  failed: {e}", at as f32 / RATE as f32),
            }
        }));
    };
    for chunk in raw.chunks(1920) {
        if realtime {
            let due = std::time::Duration::from_secs_f64(fed as f64 / RATE as f64);
            if let Some(w) = due.checked_sub(t0.elapsed()) {
                std::thread::sleep(w);
            }
        }
        let frame: Vec<i16> = chunk.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
        fed += frame.len();
        if let Some(u) = listener.push(&frame) {
            take(u, fed);
        }
    }
    // trailing silence so the last utterance completes
    let quiet = vec![0i16; 960];
    for _ in 0..200 {
        fed += quiet.len();
        if let Some(u) = listener.push(&quiet) {
            take(u, fed);
            break;
        }
    }
    drop(take);
    if let Some(p) = prev {
        let _ = p.join();
    }
    Ok(())
}

// ---- speaking ----

struct Say {
    chat: String,
    text: String,
    lang: String,
    ctx: Ctx,
    /// Its place in [`SAY_GEN`]: a newer reply replaces it, even mid-sentence.
    gen: u64,
    /// An acknowledgment ("Got it."): made ahead (`audio`), never cut by the reply after it.
    ack: Option<Vec<u8>>,
}

/// Number of the latest reply queued for the call.
static SAY_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn superseded(gen: u64) -> bool {
    SAY_GEN.load(std::sync::atomic::Ordering::SeqCst) != gen
}

fn speaker() -> &'static mpsc::Sender<Say> {
    static TX: OnceLock<mpsc::Sender<Say>> = OnceLock::new();
    TX.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Say>();
        let _ = std::thread::Builder::new().name("call-say".into()).spawn(move || {
            while let Ok(job) = rx.recv() {
                let _done = Spoken;
                if active_number().as_deref() != Some(job.chat.as_str()) || superseded(job.gen) {
                    continue; // hung up, or a newer reply came before this one started
                }
                // an acknowledgment plays to the end; the reply waits for it (it is short)
                let cuttable = job.ack.is_none();
                // stream: aplay starts with the first sentence while the rest is synthesized
                let mut player = match start_player() {
                    Ok(p) => p,
                    Err(e) => { log(format!("aplay: {e}")); continue; }
                };
                *PLAYER.lock().unwrap() = Some(player.id());
                let stdin = player.stdin.take().unwrap();
                // The writer keeps the line fed: while the next sentence is still being
                // made, it writes silence at the pace of real time instead of letting the
                // sound card run dry (an underrun is a click or a burst of noise in the
                // call; a long sentence after a short one used to leave a 1.5 s hole).
                let (audio_tx, audio_rx) = mpsc::channel::<Vec<u8>>();
                let writer = std::thread::Builder::new().name("call-play".into()).spawn(move || feed(stdin, audio_rx));
                // aplay's own warnings (underrun!!!) into the speech log
                if let Some(err) = player.stderr.take() {
                    let _ = std::thread::Builder::new().name("aplay-err".into()).spawn(move || {
                        use std::io::BufRead;
                        for line in std::io::BufReader::new(err).lines().map_while(Result::ok) {
                            if !line.trim().is_empty() && !line.starts_with("Playing raw data") {
                                log(format!("aplay: {}", line.trim()));
                            }
                        }
                    });
                }
                note_spoken(&job.chat, &job.text);
                let t0 = Instant::now();
                let mut first: Option<f64> = None;
                let mut secs = 0f64;
                let chat = job.chat.clone();
                let mut band = Telephone::new();
                let gen = job.gen;
                let r = if let Some(a) = job.ack {
                    secs = a.len() as f64 / 2.0 / RATE as f64;
                    first = Some(0.0);
                    let _ = audio_tx.send(a);
                    Ok(bia_speech::tts::Spoken { first_ms: 0, samples: 0 })
                } else { bia_speech::tts::speak(&job.text, &job.lang, &mut |chunk, rate| {
                    if active_number().as_deref() != Some(chat.as_str()) || superseded(gen) {
                        return false; // they hung up, or bia says something new: stop making it
                    }
                    let pcm = band.run(&resample(chunk, rate));
                    secs += pcm.len() as f64 / RATE as f64;
                    first.get_or_insert(t0.elapsed().as_secs_f64());
                    let bytes: Vec<u8> = pcm.iter().flat_map(|s| s.to_le_bytes()).collect();
                    audio_tx.send(bytes).is_ok()
                }) };
                let made = t0.elapsed().as_secs_f64();
                drop(audio_tx);
                // switch to the newer reply at once: what is still queued in the player goes
                // too (checked again while it plays out)
                let cut = loop {
                    if cuttable && superseded(gen) {
                        let _ = player.kill();
                        break true;
                    }
                    if writer.as_ref().map_or(true, |w| w.is_finished()) && player.try_wait().ok().flatten().is_some() {
                        break false;
                    }
                    std::thread::sleep(Duration::from_millis(30));
                };
                if let Ok(w) = writer {
                    let _ = w.join();
                }
                let _ = player.wait();
                *PLAYER.lock().unwrap() = None;
                if cut {
                    log(format!("cut after {secs:.1}s made: bia said something new"));
                    continue;
                }
                log(format!("spoke {secs:.1}s into the call; first sound after {:.2}s, all made in {made:.1}s", first.unwrap_or(0.0)));
                if let Err(e) = r {
                    log(format!("voice failed: {e}"));
                    job.ctx.inbound("system", &format!("phone:{}", job.chat), "", &format!("[voice failed: {e}]"));
                } else if cuttable && !superseded(gen) && active_number().as_deref() == Some(job.chat.as_str()) {
                    // what bia said has played to the end (as on the other buses); the
                    // line's own acknowledgments are not bia's words and say nothing
                    job.ctx.inbound("system", &format!("phone:{}", job.chat), "", "[you finished speaking]");
                }
            }
        });
        tx
    })
}

/// Replies queued for the call and not yet played to the end.
static UNSPOKEN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Marks a queued reply done however its job ends.
struct Spoken;

impl Drop for Spoken {
    fn drop(&mut self) {
        UNSPOKEN.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Wait (at most `max`) until everything bia said into the call has been played, so a
/// goodbye followed at once by a hang-up is heard. Seconds waited.
pub fn wait_spoken(max: Duration) -> f64 {
    let t = Instant::now();
    while UNSPOKEN.load(std::sync::atomic::Ordering::SeqCst) > 0 && active_number().is_some() && t.elapsed() < max {
        std::thread::sleep(Duration::from_millis(50));
    }
    t.elapsed().as_secs_f64()
}

/// Write the audio as it comes. Silence goes in only when what was written is about to
/// run out (less than `LOW` ahead of real time), so the sound card never runs dry
/// between sentences, and a streaming voice (Phonon: 80 ms pieces arriving at about the
/// pace they play) gets no holes in the middle of its words. Ends when the speech is done.
/// Speech is held until this much is ready (at the start, and again after the voice fell
/// behind), so a moment of slow synthesis is one short pause, not a stutter.
const PREBUFFER_SECS: f64 = 0.3;

fn feed(mut out: std::process::ChildStdin, rx: mpsc::Receiver<Vec<u8>>) {
    const STEP: Duration = Duration::from_millis(30);
    let quiet = vec![0u8; (RATE as usize * 30 / 1000) * 2];
    let mut clock = Lead::default();
    let mut held: Vec<u8> = Vec::new();
    let mut holding = true;
    let write = |out: &mut std::process::ChildStdin, b: &[u8], clock: &mut Lead| {
        clock.wrote(b.len() / 2, Instant::now());
        out.write_all(b).is_ok()
    };
    loop {
        let ok = match rx.recv_timeout(STEP) {
            Ok(b) if holding => {
                held.extend_from_slice(&b);
                if held.len() as f64 / 2.0 / RATE as f64 >= PREBUFFER_SECS {
                    holding = false;
                    write(&mut out, &std::mem::take(&mut held), &mut clock)
                } else {
                    true
                }
            }
            Ok(b) => write(&mut out, &b, &mut clock),
            Err(mpsc::RecvTimeoutError::Timeout) if clock.needs_fill(Instant::now()) => {
                // ran dry (between sentences, or the voice is behind): fill, and gather
                // a little again before speaking on
                holding = true;
                write(&mut out, &quiet, &mut clock)
            }
            Err(mpsc::RecvTimeoutError::Timeout) => true,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = out.write_all(&held);
                break;
            }
        };
        if !ok {
            break;
        }
    }
}

/// How far the audio written to the player runs ahead of real time (a lower bound: the
/// card starts a little after the first write).
#[derive(Default)]
struct Lead {
    start: Option<Instant>,
    samples: usize,
}

impl Lead {
    /// Below this much audio ahead, silence is written to keep the card fed.
    const LOW: f64 = 0.15;

    fn wrote(&mut self, samples: usize, now: Instant) {
        self.start.get_or_insert(now);
        self.samples += samples;
    }

    fn ahead(&self, now: Instant) -> f64 {
        let Some(s) = self.start else { return 0.0 };
        self.samples as f64 / RATE as f64 - now.duration_since(s).as_secs_f64()
    }

    fn needs_fill(&self, now: Instant) -> bool {
        self.ahead(now) < Self::LOW
    }
}

/// aplay into the call. First with half a second of buffer: synthesis comes in bursts
/// and shares the CPU with the call's recognition, and a dry buffer is a click in the
/// caller's ear. The call device only opens during a call and its limits are not
/// documented: when it refuses those settings, aplay exits at once and the defaults
/// that always worked are used instead.
fn start_player() -> std::io::Result<std::process::Child> {
    let base = ["-D", PLAY_PCM, "-f", "S16_LE", "-r", "16000", "-c", "1", "-t", "raw"];
    let spawn = |extra: &[&str]| Command::new("aplay").args(base).args(extra).stdin(Stdio::piped()).stderr(Stdio::piped()).spawn();
    let mut p = spawn(&["--buffer-time=500000"])?;
    std::thread::sleep(Duration::from_millis(150));
    if let Ok(Some(st)) = p.try_wait() {
        let mut why = String::new();
        if let Some(mut e) = p.stderr.take() {
            let _ = e.read_to_string(&mut why);
        }
        log(format!("aplay refused a 0.5 s buffer ({st}: {}); default buffer", why.lines().find(|l| l.contains("Unable")).unwrap_or("").trim()));
        return spawn(&[]);
    }
    Ok(p)
}

// Queue `text` to be spoken into the active call with `chat`.
pub fn say(ctx: &Ctx, chat: &str, text: &str, lang: &str) -> Result<String, String> {
    match active_number() {
        Some(n) if n == chat => {}
        Some(n) => return Err(format!("no active call with {chat} (the call is with {n})")),
        None => return Err("no active call".into()),
    }
    // counts as spoken at once, so the agent's text copy right after is not sent as SMS
    note_spoken(chat, text);
    UNSPOKEN.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    // a new reply replaces the one playing or waiting (bia changed its mind, or answers
    // what the caller just said): it starts as soon as the old one stops
    let gen = SAY_GEN.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    // no language given: the one fixed for the call
    let lang = match lang.trim() {
        "" => plan().lock().unwrap().as_ref().and_then(|p| p.forced.clone()).unwrap_or_default(),
        l => l.to_string(),
    };
    if speaker().send(Say { chat: chat.into(), text: text.into(), lang, ctx: ctx.clone(), gen, ack: None }).is_err() {
        UNSPOKEN.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
    Ok(format!("speaking in the call with {chat}"))
}

/// bia's voice made fit for a phone line: a call here is narrowband (the caller's audio
/// stops at 3.9 kHz) and the sound card folds what lies above 4 kHz back down instead
/// of filtering it, so the 4-8 kHz of a 16 kHz voice (every "s") came out as short
/// bursts of high noise. A low-pass FIR (Blackman windowed sinc, 3.6 kHz, 101 taps) keeps
/// the telephone band; each piece fades in and out over 5 ms so it starts without a pop.
struct Telephone {
    taps: Vec<f32>,
    hist: Vec<f32>,
}

impl Telephone {
    const CUT_HZ: f32 = 3_600.0;
    const TAPS: usize = 101;
    const FADE: usize = 80; // 5 ms at 16 kHz

    fn new() -> Self {
        let n = Self::TAPS;
        let fc = Self::CUT_HZ / RATE as f32;
        let m = (n - 1) as f32;
        let mut taps: Vec<f32> = (0..n)
            .map(|i| {
                let x = i as f32 - m / 2.0;
                let sinc = if x == 0.0 { 2.0 * fc } else { (2.0 * std::f32::consts::PI * fc * x).sin() / (std::f32::consts::PI * x) };
                let w = 0.42 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / m).cos() + 0.08 * (4.0 * std::f32::consts::PI * i as f32 / m).cos();
                sinc * w
            })
            .collect();
        let sum: f32 = taps.iter().sum();
        taps.iter_mut().for_each(|t| *t /= sum);
        Self { taps, hist: vec![0.0; n - 1] }
    }

    /// Filter one piece of 16 kHz audio (the filter carries over between pieces).
    fn run(&mut self, pcm: &[i16]) -> Vec<i16> {
        let mut x = std::mem::take(&mut self.hist);
        x.extend(pcm.iter().map(|&s| s as f32));
        let n = self.taps.len();
        let mut out: Vec<i16> = (0..pcm.len())
            .map(|k| {
                let acc: f32 = self.taps.iter().zip(&x[k..k + n]).map(|(t, v)| t * v).sum();
                acc.clamp(-32768.0, 32767.0) as i16
            })
            .collect();
        self.hist = x[x.len() - (n - 1)..].to_vec();
        let f = Self::FADE.min(out.len() / 2);
        for i in 0..f {
            let g = i as f32 / f as f32;
            out[i] = (out[i] as f32 * g) as i16;
            let j = out.len() - 1 - i;
            out[j] = (out[j] as f32 * g) as i16;
        }
        out
    }
}

/// Any rate → 16 kHz i16 (linear; speech only).
fn resample(samples: &[f32], from: u32) -> Vec<i16> {
    let conv = |x: f32| (x.clamp(-1.0, 1.0) * 32767.0) as i16;
    if samples.is_empty() {
        return Vec::new();
    }
    if from == RATE || from == 0 {
        return samples.iter().copied().map(conv).collect();
    }
    let n = (samples.len() as u64 * RATE as u64 / from as u64) as usize;
    (0..n)
        .map(|k| {
            let pos = k as f64 * from as f64 / RATE as f64;
            let i = pos.floor() as usize;
            let t = (pos - i as f64) as f32;
            let a = samples[i.min(samples.len() - 1)];
            let b = samples[(i + 1).min(samples.len() - 1)];
            conv(a + (b - a) * t)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_only_when_the_speech_runs_out() {
        let t = Instant::now();
        let ms = |n: u64| t + Duration::from_millis(n);
        let mut l = Lead::default();
        assert!(l.needs_fill(t)); // nothing yet: keep the card fed
        // a voice sending 80 ms pieces every 70 ms, with half a second written first
        l.wrote(RATE as usize / 2, t);
        let mut now = 0;
        for _ in 0..20 {
            now += 70;
            assert!(!l.needs_fill(ms(now)), "no hole in the middle of speech at {now} ms");
            l.wrote(RATE as usize * 80 / 1000, ms(now));
        }
        // the voice stops: silence once less than LOW is left
        let left = (l.ahead(ms(now)) * 1000.0) as u64;
        assert!(!l.needs_fill(ms(now + left - 200)));
        assert!(l.needs_fill(ms(now + left - 100)));
    }

    fn tone(hz: f32, n: usize) -> Vec<i16> {
        (0..n).map(|i| ((2.0 * std::f32::consts::PI * hz * i as f32 / RATE as f32).sin() * 10_000.0) as i16).collect()
    }
    fn rms(x: &[i16]) -> f32 {
        (x.iter().map(|&v| (v as f32).powi(2)).sum::<f32>() / x.len() as f32).sqrt()
    }

    #[test]
    fn the_phone_band_passes_and_the_top_is_cut() {
        // middle of a 1 s piece, away from the fades and the filter's start
        let mid = |x: Vec<i16>| x[2_000..14_000].to_vec();
        let speech = mid(Telephone::new().run(&tone(1_000.0, 16_000)));
        let hiss = mid(Telephone::new().run(&tone(6_000.0, 16_000)));
        assert!(rms(&speech) > 6_500.0, "1 kHz kept: {}", rms(&speech));
        assert!(rms(&hiss) < 70.0, "6 kHz removed: {}", rms(&hiss));
    }

    #[test]
    fn pieces_join_without_a_step() {
        // the filter continues across pieces; only the 5 ms fades touch the edges
        let mut t = Telephone::new();
        let a = t.run(&tone(500.0, 4_000));
        let b = t.run(&tone(500.0, 4_000));
        assert_eq!(a[0], 0);
        assert_eq!(*b.last().unwrap(), 0);
        assert_eq!(a.len() + b.len(), 8_000);
    }
}
