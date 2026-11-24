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
    log(format!("call with {number}: phone mic and earpiece muted, in-call record and music on"));
    bia_speech::warm();
    let ctx = ctx.clone();
    let num = number.to_string();
    let _ = std::thread::Builder::new().name("call-listen".into()).spawn(move || listen(ctx, num));
}

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
    for (c, v) in saved().lock().unwrap().drain(..) {
        amixer_set(&c, &v);
    }
    if was.is_some() {
        log("call ended: phone mic and earpiece restored");
    }
}

fn listen(ctx: Ctx, num: String) {
    let channel = format!("phone:{num}");
    let mut listener = match bia_speech::Listener::with_lang(RATE, bia_speech::contacts::lang_for(&channel)) {
        Ok(l) => l,
        Err(e) => {
            ctx.inbound("system", &channel, "", &format!("[call] bia can not hear this call: speech unavailable ({e})"));
            return;
        }
    };
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

// ---- speaking ----

struct Say {
    chat: String,
    text: String,
    lang: String,
    ctx: Ctx,
}

fn speaker() -> &'static mpsc::Sender<Say> {
    static TX: OnceLock<mpsc::Sender<Say>> = OnceLock::new();
    TX.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Say>();
        let _ = std::thread::Builder::new().name("call-say".into()).spawn(move || {
            while let Ok(job) = rx.recv() {
                if active_number().as_deref() != Some(job.chat.as_str()) {
                    continue;
                }
                // stream: aplay starts with the first sentence while the rest is synthesized
                let mut player = match Command::new("aplay")
                    .args(["-q", "-D", PLAY_PCM, "-f", "S16_LE", "-r", "16000", "-c", "1", "-t", "raw"])
                    .stdin(Stdio::piped()).stderr(Stdio::null()).spawn()
                {
                    Ok(p) => p,
                    Err(e) => { log(format!("aplay: {e}")); continue; }
                };
                *PLAYER.lock().unwrap() = Some(player.id());
                let mut stdin = player.stdin.take().unwrap();
                note_spoken(&job.chat, &job.text);
                let t0 = Instant::now();
                let mut first: Option<f64> = None;
                let mut secs = 0f64;
                let chat = job.chat.clone();
                let r = bia_speech::tts::speak(&job.text, &job.lang, &mut |chunk, rate| {
                    if active_number().as_deref() != Some(chat.as_str()) {
                        return false; // they hung up: stop making it
                    }
                    let pcm = resample(chunk, rate);
                    secs += pcm.len() as f64 / RATE as f64;
                    first.get_or_insert(t0.elapsed().as_secs_f64());
                    let bytes: Vec<u8> = pcm.iter().flat_map(|s| s.to_le_bytes()).collect();
                    stdin.write_all(&bytes).is_ok()
                });
                drop(stdin);
                let _ = player.wait();
                *PLAYER.lock().unwrap() = None;
                log(format!("spoke {secs:.1}s into the call; first sound after {:.2}s of synthesis", first.unwrap_or(0.0)));
                if let Err(e) = r {
                    log(format!("voice failed: {e}"));
                    job.ctx.inbound("system", &format!("phone:{}", job.chat), "", &format!("[voice failed: {e}]"));
                }
            }
        });
        tx
    })
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
    let _ = speaker().send(Say { chat: chat.into(), text: text.into(), lang: lang.into(), ctx: ctx.clone() });
    Ok(format!("speaking in the call with {chat}"))
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
