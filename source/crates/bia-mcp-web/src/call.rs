//! Live voice calls in the web app, over a WebSocket (`/v1/call?chat=<id>`).
//!
//! Browser → agent: binary frames of 16 kHz mono s16le (20 ms each); text JSON
//! `{t:"played", gen}` (an utterance finished playing), `{t:"mute", on}`, `{t:"hangup"}`.
//! Agent → browser: text JSON `ready`, `say {gen,text}`, `say_end {gen}`, `stop {gen}`,
//! `caption {who,text}`, `end {reason}`; binary frames `[gen u32][rate u32][s16le…]`.
//!
//! Speech goes through bia-speech (VAD, language per phrase, auto-learn) and reaches the
//! model as `[speech <lang>] …` on the chat's channel. `say` on that channel speaks
//! into the call (`speak`). bia can ring an open web app (`ring`); after the call a
//! summary with the transcript is posted to the chat.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

const RING_SECS: u64 = 30;
/// Continuous speech (in 20 ms frames) while bia talks that counts as interrupting.
const BARGE_FRAMES: u32 = 15;
/// bia's words count as a possible echo this long after they finished.
const ECHO_SECS: u64 = 3;

type Key = (String, String);

struct Call {
    identity: String,
    chat: String,
    out: Mutex<Box<dyn Write + Send>>,
    close: Box<dyn Fn() + Send + Sync>,
    gen: AtomicU64,
    speaking: AtomicBool,
    muted: AtomicBool,
    ended: AtomicBool,
    /// Hang up once what bia is saying has played (goodbye first).
    hangup_after: AtomicBool,
    started: Instant,
    lines: Mutex<Vec<(String, String)>>,
    spoken: Mutex<Option<(String, Instant)>>,
}

impl Call {
    fn send_json(&self, v: Value) {
        if let Ok(mut o) = self.out.lock() {
            let _ = crate::server::ws_send(&mut **o, 1, v.to_string().as_bytes());
        }
    }

    fn send_audio(&self, gen: u64, rate: u32, pcm: &[i16]) -> bool {
        let mut pkt = Vec::with_capacity(8 + pcm.len() * 2);
        pkt.extend_from_slice(&(gen as u32).to_le_bytes());
        pkt.extend_from_slice(&rate.to_le_bytes());
        for s in pcm {
            pkt.extend_from_slice(&s.to_le_bytes());
        }
        match self.out.lock() {
            Ok(mut o) => crate::server::ws_send(&mut **o, 2, &pkt).is_ok(),
            Err(_) => false,
        }
    }

    fn channel(&self) -> String {
        crate::chat::channel_of(&self.chat)
    }

    fn system(&self, text: &str) {
        crate::chat::agent_notice(&self.identity, &self.chat, text);
    }

    /// Stop what bia is saying (new utterance, interruption, hangup).
    fn stop_speaking(&self) -> bool {
        let was = self.speaking.swap(false, Ordering::SeqCst);
        let gen = self.gen.fetch_add(1, Ordering::SeqCst) + 1;
        self.send_json(json!({"t": "stop", "gen": gen}));
        was
    }

    fn echo_ref(&self) -> Option<String> {
        let g = self.spoken.lock().ok()?;
        let (text, at) = g.as_ref()?;
        (self.speaking.load(Ordering::SeqCst) || at.elapsed().as_secs() < ECHO_SECS)
            .then(|| text.clone())
    }
}

struct Ring {
    id: String,
    started: Instant,
}

fn calls() -> &'static Mutex<HashMap<Key, Arc<Call>>> {
    static C: OnceLock<Mutex<HashMap<Key, Arc<Call>>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

fn rings() -> &'static Mutex<HashMap<Key, Ring>> {
    static R: OnceLock<Mutex<HashMap<Key, Ring>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(HashMap::new()))
}

fn key(identity: &str, channel: &str) -> Option<Key> {
    crate::chat::chat_of(channel).map(|c| (identity.to_string(), c.to_string()))
}

fn find(identity: &str, channel: &str) -> Option<Arc<Call>> {
    let k = key(identity, channel)?;
    calls().lock().ok()?.get(&k).cloned()
}

/// A call is active on this web channel.
pub fn active(identity: &str, channel: &str) -> bool {
    find(identity, channel).is_some()
}

fn fmt_dur(d: Duration) -> String {
    let s = d.as_secs();
    format!("{}:{:02}", s / 60, s % 60)
}

/// Serve one call WebSocket until it closes. `recv` reads the next frame.
pub fn serve(
    out: Box<dyn Write + Send>,
    close: Box<dyn Fn() + Send + Sync>,
    recv: &mut dyn FnMut() -> std::io::Result<(u8, Vec<u8>)>,
    identity: &str,
    chat: &str,
    ring_id: &str,
) {
    let k = (identity.to_string(), chat.to_string());
    if let Some(old) = calls().lock().ok().and_then(|c| c.get(&k).cloned()) {
        hang_up(&old, "replaced by a new call");
    }
    let answered = rings()
        .lock()
        .ok()
        .and_then(|mut r| r.remove(&k))
        .is_some_and(|r| r.id == ring_id || ring_id.is_empty());
    if answered {
        crate::chat::emit_event(identity, json!({"t": "ring_end", "chat": chat}));
    }
    let call = Arc::new(Call {
        identity: identity.to_string(),
        chat: chat.to_string(),
        out: Mutex::new(out),
        close,
        gen: AtomicU64::new(0),
        speaking: AtomicBool::new(false),
        muted: AtomicBool::new(false),
        ended: AtomicBool::new(false),
        hangup_after: AtomicBool::new(false),
        started: Instant::now(),
        lines: Mutex::new(Vec::new()),
        spoken: Mutex::new(None),
    });
    if let Ok(mut c) = calls().lock() {
        c.insert(k.clone(), call.clone());
    }
    bia_speech::warm();
    let mut listener = match bia_speech::Listener::with_lang(
        16_000,
        bia_speech::contacts::lang_for_identity(identity),
    ) {
        Ok(l) => l,
        Err(e) => {
            call.send_json(json!({"t": "end", "reason": format!("speech unavailable: {e}")}));
            call.system(&format!("[call failed: speech unavailable: {e}]"));
            finish(&call, None);
            return;
        }
    };
    call.send_json(json!({"t": "ready"}));
    crate::chat::emit_event(identity, json!({"t": "call", "chat": chat, "on": true}));
    call.system(if answered {
        "[call started] (they answered your call)"
    } else {
        "[call started] (they called you)"
    });
    bia_speech::diag::log(
        "webcall",
        format!("call started {identity}@{}", call.channel()),
    );
    let mut barge = 0u32;
    loop {
        match recv() {
            Ok((2, payload)) => {
                if call.muted.load(Ordering::Relaxed) {
                    continue;
                }
                let pcm = bia_speech::audio::le_bytes_to_i16(&payload);
                let utt = listener.push(&pcm);
                // Talking over bia stops its playback (like a phone call).
                if call.speaking.load(Ordering::SeqCst) && listener.in_speech() {
                    barge += 1;
                    if barge >= BARGE_FRAMES {
                        barge = 0;
                        if call.stop_speaking() {
                            call.system(
                                "[interrupted: they started talking while you were speaking]",
                            );
                        }
                    }
                } else {
                    barge = 0;
                }
                if let Some(utt) = utt {
                    transcribe(call.clone(), utt);
                }
            }
            Ok((1, payload)) => {
                let Ok(v) = serde_json::from_slice::<Value>(&payload) else {
                    continue;
                };
                match v.get("t").and_then(|t| t.as_str()).unwrap_or("") {
                    "played" => {
                        let gen = v.get("gen").and_then(|g| g.as_u64()).unwrap_or(0);
                        if gen == call.gen.load(Ordering::SeqCst)
                            && call.speaking.swap(false, Ordering::SeqCst)
                        {
                            if call.hangup_after.load(Ordering::SeqCst) {
                                hang_up(&call, "bia hung up");
                            } else {
                                call.system("[you finished speaking]");
                            }
                        }
                    }
                    "mute" => call.muted.store(
                        v.get("on").and_then(|x| x.as_bool()).unwrap_or(false),
                        Ordering::Relaxed,
                    ),
                    "hangup" => break,
                    "ping" => call.send_json(json!({"t": "pong"})),
                    _ => {}
                }
            }
            Ok((9, payload)) => {
                if let Ok(mut o) = call.out.lock() {
                    let _ = crate::server::ws_send(&mut **o, 10, &payload);
                }
            }
            Ok((8, _)) | Err(_) => break,
            Ok(_) => {}
        }
        if call.ended.load(Ordering::SeqCst) {
            break;
        }
    }
    finish(&call, Some("they hung up"));
}

fn transcribe(call: Arc<Call>, utt: bia_speech::Utterance) {
    let echo = call.echo_ref();
    std::thread::spawn(move || {
        let speaker = format!("web:{}", call.identity);
        match utt.transcribe_call(echo.as_deref(), Some(&speaker), true) {
            Ok(heard) => {
                let text = heard.text();
                if !text.is_empty() && !call.ended.load(Ordering::SeqCst) {
                    let words: Vec<&str> = heard.runs.iter().map(|r| r.text.as_str()).collect();
                    let words = words.join(" ");
                    call.send_json(json!({"t": "caption", "who": "you", "text": words}));
                    if let Ok(mut l) = call.lines.lock() {
                        l.push((call.identity.clone(), words));
                    }
                    crate::chat::agent_speech(&call.identity, &call.chat, &text);
                }
                if let Some(n) = heard.notice {
                    call.system(&n);
                }
            }
            Err(e) => bia_speech::diag::log("webcall", format!("transcribe failed: {e}")),
        }
    });
}

/// Speak `text` (in `lang`) into the active call on this channel.
pub fn speak(identity: &str, channel: &str, text: &str, lang: &str) -> Result<String, String> {
    let call = find(identity, channel).ok_or("no active web call on this channel")?;
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("text is empty".into());
    }
    // no lang given: the person's preferred language (addressbook)
    let preferred = bia_speech::contacts::lang_for_identity(identity).unwrap_or_default();
    let lang = if lang.trim().is_empty() {
        preferred.as_str()
    } else {
        lang
    };
    bia_speech::tts::plan(&bia_speech::registry::current(), &text, lang)?;
    if call.stop_speaking() {
        call.system("[voice cancelled]");
    }
    let gen = call.gen.load(Ordering::SeqCst);
    call.speaking.store(true, Ordering::SeqCst);
    if let Ok(mut s) = call.spoken.lock() {
        *s = Some((text.clone(), Instant::now()));
    }
    if let Ok(mut l) = call.lines.lock() {
        l.push(("bia".into(), text.clone()));
    }
    call.send_json(json!({"t": "say", "gen": gen, "text": text}));
    let lang = lang.to_string();
    std::thread::spawn(move || {
        let live =
            |c: &Call| c.gen.load(Ordering::SeqCst) == gen && !c.ended.load(Ordering::SeqCst);
        let r = bia_speech::tts::speak(&text, &lang, &mut |chunk, rate| {
            if !live(&call) {
                return false;
            }
            let pcm: Vec<i16> = chunk
                .iter()
                .map(|s| (s.clamp(-1.0, 1.0) * 32767.0) as i16)
                .collect();
            call.send_audio(gen, rate, &pcm)
        });
        if !live(&call) {
            return;
        }
        match r {
            Ok(_) => call.send_json(json!({"t": "say_end", "gen": gen})),
            Err(e) => {
                call.speaking.store(false, Ordering::SeqCst);
                call.system(&format!("[voice failed: {e}]"));
            }
        }
    });
    Ok("spoken in the web call; [you finished speaking] arrives when it has played".into())
}

fn hang_up(call: &Call, reason: &str) {
    call.stop_speaking();
    call.send_json(json!({"t": "end", "reason": reason}));
    call.ended.store(true, Ordering::SeqCst);
    (call.close)();
}

/// bia withdraws its own ringing call before it was answered.
pub fn cancel_ring(identity: &str, channel: &str) -> Result<String, String> {
    let k = key(identity, channel).ok_or_else(|| format!("{channel} is not a web chat"))?;
    if rings().lock().ok().and_then(|mut r| r.remove(&k)).is_some() {
        crate::chat::emit_event(identity, json!({"t": "ring_end", "chat": k.1}));
        return Ok(format!(
            "call to {channel} cancelled before it was answered"
        ));
    }
    if find(identity, channel).is_some() {
        return Err("the call was already answered; end it with web_call_hangup".into());
    }
    Err(format!("not ringing on {channel}"))
}

/// bia ends the call.
pub fn hangup(identity: &str, channel: &str) -> Result<String, String> {
    if let Some(k) = key(identity, channel) {
        if rings().lock().ok().and_then(|mut r| r.remove(&k)).is_some() {
            crate::chat::emit_event(identity, json!({"t": "ring_end", "chat": k.1}));
            return Ok("stopped ringing".into());
        }
    }
    let call = find(identity, channel).ok_or("no active web call on this channel")?;
    if call.speaking.load(Ordering::SeqCst) {
        // let the goodbye play out; hang up when it has (or after 20 s at most)
        call.hangup_after.store(true, Ordering::SeqCst);
        let c = call.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(20));
            if !c.ended.load(Ordering::SeqCst) {
                hang_up(&c, "bia hung up");
            }
        });
        return Ok(format!(
            "hanging up {channel} as soon as what you are saying has played"
        ));
    }
    hang_up(&call, "bia hung up");
    Ok(format!("hung up {channel}"))
}

/// Remove the call, tell the model, and post the transcript to the chat.
fn finish(call: &Arc<Call>, reason: Option<&str>) {
    let k = (call.identity.clone(), call.chat.clone());
    let removed = calls()
        .lock()
        .ok()
        .and_then(|mut c| match c.get(&k) {
            Some(x) if Arc::ptr_eq(x, call) => c.remove(&k),
            _ => None,
        })
        .is_some();
    call.ended.store(true, Ordering::SeqCst);
    (call.close)();
    if !removed {
        return;
    }
    let dur = fmt_dur(call.started.elapsed());
    crate::chat::emit_event(
        &call.identity,
        json!({"t": "call", "chat": call.chat, "on": false}),
    );
    let why = reason.map(|r| format!(", {r}")).unwrap_or_default();
    call.system(&format!("[call ended] ({dur}{why})"));
    bia_speech::diag::log(
        "webcall",
        format!("call ended {}@{} {dur}", call.identity, call.channel()),
    );
    let lines = call.lines.lock().map(|l| l.clone()).unwrap_or_default();
    let mut summary = format!("📞 Call · {dur}");
    if lines.is_empty() {
        summary.push_str("\n\n_(nothing was said)_");
    } else {
        summary.push('\n');
        for (who, text) in lines {
            summary.push_str(&format!("\n**{who}:** {text}"));
        }
    }
    let _ = crate::chat::post_bia(&call.identity, &call.chat, &summary);
}

/// bia calls an open web app (an incoming-call screen with ringing).
pub fn ring(identity: &str, channel: &str) -> Result<String, String> {
    let k = key(identity, channel).ok_or_else(|| format!("{channel} is not a web chat"))?;
    if !crate::chat::has_client(identity) {
        return Err(format!(
            "no web app is open for {identity}; send a message instead"
        ));
    }
    if find(identity, channel).is_some() {
        return Err("already in a call on this channel".into());
    }
    crate::chat::ensure_chat(identity, channel)?;
    let id = crate::bus::mint_msg_id();
    if let Ok(mut r) = rings().lock() {
        r.insert(
            k.clone(),
            Ring {
                id: id.clone(),
                started: Instant::now(),
            },
        );
    }
    crate::chat::emit_event(identity, json!({"t": "ring", "chat": k.1, "id": id}));
    let (ident, chat, rid) = (identity.to_string(), k.1.clone(), id);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(RING_SECS));
        let k = (ident.clone(), chat.clone());
        let expired = rings()
            .lock()
            .ok()
            .and_then(|mut r| match r.get(&k) {
                Some(x) if x.id == rid && x.started.elapsed().as_secs() >= RING_SECS => {
                    r.remove(&k)
                }
                _ => None,
            })
            .is_some();
        if expired {
            crate::chat::emit_event(&ident, json!({"t": "ring_end", "chat": chat}));
            crate::chat::agent_notice(&ident, &chat, "[no answer]");
        }
    });
    Ok(format!(
        "ringing {channel}; [call started] arrives when they answer, [call declined] or [no answer] (after {RING_SECS}s) otherwise"
    ))
}

/// The user declined bia's call.
pub fn decline(identity: &str, chat: &str) {
    let k = (identity.to_string(), chat.to_string());
    if rings().lock().ok().and_then(|mut r| r.remove(&k)).is_some() {
        crate::chat::emit_event(identity, json!({"t": "ring_end", "chat": chat}));
        crate::chat::agent_notice(identity, chat, "[call declined]");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::Bus;
    use std::sync::mpsc;

    #[derive(Clone, Default)]
    struct Sink(Arc<Mutex<Vec<u8>>>);

    impl Write for Sink {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// Server→browser frames (unmasked) written so far.
    fn frames(buf: &[u8]) -> Vec<(u8, Vec<u8>)> {
        let mut out = Vec::new();
        let mut i = 0;
        while i + 2 <= buf.len() {
            let op = buf[i] & 0x0f;
            let mut len = (buf[i + 1] & 0x7f) as usize;
            i += 2;
            if len == 126 {
                len = u16::from_be_bytes([buf[i], buf[i + 1]]) as usize;
                i += 2;
            } else if len == 127 {
                len = u64::from_be_bytes(buf[i..i + 8].try_into().unwrap()) as usize;
                i += 8;
            }
            out.push((op, buf[i..i + len].to_vec()));
            i += len;
        }
        out
    }

    fn next_web(con: &Bus) -> crate::bus::Inbound {
        loop {
            let ib = con.next_held(Duration::from_secs(60)).expect("inbound");
            if crate::chat::is_web(&ib.channel) {
                return ib;
            }
        }
    }

    /// Real models: BIA_SPEECH_DIR=<speech tree> cargo test -p bia-mcp-web -- --ignored web_call
    #[test]
    #[ignore = "needs the speech models (BIA_SPEECH_DIR)"]
    fn web_call_roundtrip() {
        let dir = std::env::temp_dir().join(format!("bia-webcall-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("BIA_SPEECH_STORE", dir.join("speech"));
        let con = Arc::new(Bus::new(dir.clone()));
        crate::chat::init(&dir, con.clone());
        crate::chat::ensure_chat("root", "web:test").unwrap();

        let sink = Sink::default();
        let (tx, rx) = mpsc::channel::<(u8, Vec<u8>)>();
        let out = sink.clone();
        let server = std::thread::spawn(move || {
            let mut recv = || rx.recv().map_err(|_| std::io::Error::other("closed"));
            serve(
                Box::new(out),
                Box::new(|| {}),
                &mut recv,
                "root",
                "test",
                "",
            );
        });
        let ib = next_web(&con);
        assert!(ib.text.starts_with("[call started]"), "{}", ib.text);

        // the caller speaks (synthesized), then is quiet
        let (x, rate) =
            bia_speech::tts::synthesize("Hello bia. What time is it in Madrid right now?", "en")
                .unwrap();
        let x16 = bia_speech::audio::resample(&x, rate, 16_000);
        let pcm: Vec<i16> = x16.iter().map(|s| (s * 32767.0) as i16).collect();
        for f in pcm
            .chunks(320)
            .chain(std::iter::repeat_n([0i16; 320].as_slice(), 75))
        {
            tx.send((2, f.iter().flat_map(|s| s.to_le_bytes()).collect()))
                .unwrap();
        }
        // one or two utterances, depending on how long the pause after "Hello bia." is
        let mut heard = String::new();
        for _ in 0..3 {
            let ib = next_web(&con);
            eprintln!("model got {}@{}: {}", ib.identity, ib.channel, ib.text);
            assert_eq!(
                (ib.identity.as_str(), ib.channel.as_str()),
                ("root", "web:test")
            );
            assert!(ib.text.starts_with("[speech en]"), "{}", ib.text);
            heard.push_str(&ib.text);
            if heard.to_lowercase().contains("madrid") {
                break;
            }
        }
        assert!(heard.to_lowercase().contains("madrid"), "{heard}");

        // bia answers: audio streams to the browser, then say_end
        assert!(
            speak("root", "web:test", "It is nine in the evening.", "").is_err(),
            "lang required"
        );
        speak("root", "web:test", "It is nine in the evening.", "en").unwrap();
        let t = Instant::now();
        let gen = loop {
            let fr = frames(&sink.0.lock().unwrap());
            let end = fr.iter().find_map(|(op, p)| {
                let v: Value = (*op == 1)
                    .then(|| serde_json::from_slice(p).ok())
                    .flatten()?;
                (v["t"] == "say_end").then(|| v["gen"].as_u64().unwrap())
            });
            if let Some(g) = end {
                let audio: usize = fr
                    .iter()
                    .filter(|(op, _)| *op == 2)
                    .map(|(_, p)| (p.len() - 8) / 2)
                    .sum();
                let caps: Vec<String> = fr
                    .iter()
                    .filter(|(op, _)| *op == 1)
                    .map(|(_, p)| String::from_utf8_lossy(p).into_owned())
                    .collect();
                eprintln!("browser got {audio} samples of speech; events {caps:?}");
                assert!(audio > 8000, "audio streamed");
                break g;
            }
            assert!(t.elapsed().as_secs() < 30, "no say_end");
            std::thread::sleep(Duration::from_millis(100));
        };
        tx.send((
            1,
            json!({"t": "played", "gen": gen}).to_string().into_bytes(),
        ))
        .unwrap();
        let ib = next_web(&con);
        assert!(
            ib.text.starts_with("[you finished speaking]"),
            "{}",
            ib.text
        );

        // bia hangs up: the model hears it ended, the chat gets the transcript
        hangup("root", "web:test").unwrap();
        drop(tx);
        server.join().unwrap();
        let ib = next_web(&con);
        assert!(ib.text.starts_with("[call ended]"), "{}", ib.text);
        let h = crate::chat::chat_text("root", "test");
        eprintln!("chat:\n{h}");
        assert!(
            h.contains("📞 Call")
                && h.to_lowercase().contains("madrid")
                && h.contains("nine in the evening")
        );
        assert!(!active("root", "web:test"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
