//! MAX voice calls, on the Rust side: bia_max_call.py does the signalling and the WebRTC media;
//! here its audio meets bia's speech. What is heard becomes `[speech]` messages, what bia says is
//! synthesized and paced back as 60 ms frames of 16 kHz mono.

use crate::audio_out::{Feed, Outbound};
use crate::helper::{self, NORMAL, QUICK};
use crate::inbound;
use base64::Engine;
use serde_json::{json, Value};
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

const ECHO_SECS: u64 = 3;

struct Active {
    conversation: String,
    chat: String,
    ready: bool,
    audio: Option<Outbound>,
    heard: Option<mpsc::Sender<Vec<i16>>>,
    utterance: u64,
    speaking: bool,
    hang_up_after: bool,
}

fn state() -> &'static Mutex<Option<Active>> {
    static S: OnceLock<Mutex<Option<Active>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(None))
}

fn spoken() -> &'static Mutex<Option<(String, Instant)>> {
    static S: OnceLock<Mutex<Option<(String, Instant)>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(None))
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

fn channel(chat: &str) -> String {
    format!("max:{chat}")
}

fn pcm_bytes(frame: &[i16]) -> String {
    let mut b = Vec::with_capacity(frame.len() * 2);
    for x in frame {
        b.extend_from_slice(&x.to_le_bytes());
    }
    base64::engine::general_purpose::STANDARD.encode(b)
}

fn pcm_samples(text: &str) -> Vec<i16> {
    let b = base64::engine::general_purpose::STANDARD.decode(text).unwrap_or_default();
    b.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect()
}

/// A frame of what the other person says.
pub fn heard(ev: &Value) {
    if let Some(tx) = state().lock().unwrap().as_ref().and_then(|a| a.heard.clone()) {
        let _ = tx.send(pcm_samples(s(ev, "pcm")));
    }
}

/// The helper says where a call is.
pub fn changed(ev: &Value) {
    let (conversation, chat) = (s(ev, "conversation").to_string(), s(ev, "chat").to_string());
    let label = inbound::label(s(ev, "peer"), s(ev, "name"));
    let incoming = ev.get("incoming").and_then(Value::as_bool).unwrap_or(false);
    inbound::note_title(&chat, s(ev, "name"));
    match s(ev, "state") {
        "ringing" => {
            if inbound::is_banned(&chat, s(ev, "peer")) {
                std::thread::spawn(|| hangup("").map(|_| ()));
                return;
            }
            *state().lock().unwrap() = Some(new_active(&conversation, &chat));
            inbound::inbound(&label, &chat, "", &format!("[incoming call] from {label}"), 0);
        }
        "active" => mark_ready(&conversation, &chat, &label),
        "ended" | "failed" | "unsupported" => finish(ev, &conversation, &chat, &label, incoming),
        _ => {}
    }
}

fn new_active(conversation: &str, chat: &str) -> Active {
    Active {
        conversation: conversation.into(),
        chat: chat.into(),
        ready: false,
        audio: None,
        heard: None,
        utterance: 0,
        speaking: false,
        hang_up_after: false,
    }
}

fn mark_ready(conversation: &str, chat: &str, label: &str) {
    {
        let mut g = state().lock().unwrap();
        let a = g.get_or_insert_with(|| new_active(conversation, chat));
        if a.ready {
            return;
        }
        a.ready = true;
        let conv = conversation.to_string();
        a.audio = Some(Outbound::start(
            move |frame| helper::get().notify("call_pcm", json!({"pcm": pcm_bytes(&frame)})),
            chat,
            move |id, gen| played_out(&conv, id, gen),
        ));
        let (tx, rx) = mpsc::channel();
        a.heard = Some(tx);
        spawn_listener(rx, chat.to_string(), label.to_string());
    }
    crate::tts::warm();
    bia_speech::calllog::start(&channel(chat));
    inbound::inbound(label, chat, "", "[call ready]", 0);
}

fn played_out(conversation: &str, id: u64, gen: u64) {
    let (chat, hang) = {
        let mut g = state().lock().unwrap();
        let Some(a) = g.as_mut().filter(|a| a.conversation == conversation) else { return };
        if a.audio.as_ref().map(Outbound::id) != Some(id) || a.utterance != gen {
            return;
        }
        a.speaking = false;
        (a.chat.clone(), a.hang_up_after)
    };
    if hang {
        std::thread::spawn(|| {
            let _ = helper::get().call("call_hangup", json!({}), NORMAL);
        });
    } else {
        inbound::inbound("system", &chat, "", "[you finished speaking]", 0);
    }
}

fn finish(ev: &Value, conversation: &str, chat: &str, label: &str, incoming: bool) {
    let was = {
        let mut g = state().lock().unwrap();
        if g.as_ref().is_some_and(|a| a.conversation == conversation) { g.take() } else { None }
    };
    let accepted = ev.get("accepted").and_then(Value::as_bool).unwrap_or(false);
    let reason = s(ev, "reason");
    let msg = match reason {
        "ANOTHER_DEVICE" => "[call ended: answered on another device]".to_string(),
        "REJECTED" if incoming => "[call rejected]".to_string(),
        _ if incoming && !accepted => "[missed call]".to_string(),
        "" | "HUNGUP" => "[call ended]".to_string(),
        r => format!("[call ended: {}]", r.to_lowercase().replace('_', " ")),
    };
    // Dropping the pacer joins its thread; the state lock is free by now.
    let ready = was.as_ref().is_some_and(|a| a.ready);
    drop(was);
    inbound::inbound(label, chat, "", &msg, 0);
    if ready {
        let (chat, label) = (chat.to_string(), label.to_string());
        std::thread::spawn(move || {
            if let Some(summary) = bia_speech::calllog::finish(&channel(&chat), &label) {
                let _ = helper::get().call("send", json!({"chat": chat, "text": summary}), NORMAL);
            }
        });
    }
}

fn spawn_listener(rx: mpsc::Receiver<Vec<i16>>, chat: String, label: String) {
    let _ = std::thread::Builder::new().name("call-stt".into()).spawn(move || {
        bia_speech::warm();
        let mut listener = match bia_speech::Listener::with_lang(16_000, bia_speech::contacts::lang_for(&channel(&chat))) {
            Ok(l) => Some(l),
            Err(e) => {
                inbound::log(&format!("speech unavailable on this call: {e}"));
                None
            }
        };
        while let Ok(frame) = rx.recv() {
            let Some(l) = listener.as_mut() else { continue };
            if let Some(utt) = l.push(&frame) {
                transcribe(utt, chat.clone(), label.clone());
            }
        }
    });
}

fn transcribe(utt: bia_speech::Utterance, chat: String, label: String) {
    let echo = spoken()
        .lock()
        .ok()
        .and_then(|g| g.as_ref().filter(|(_, at)| at.elapsed().as_secs() < 60 + ECHO_SECS).map(|(t, _)| t.clone()));
    std::thread::spawn(move || {
        let speaker = channel(&chat);
        match utt.transcribe(echo.as_deref(), Some(&speaker)) {
            Ok(heard) => {
                let text = heard.text();
                if !text.is_empty() {
                    let words: Vec<&str> = heard.runs.iter().map(|r| r.text.as_str()).collect();
                    bia_speech::calllog::add(&speaker, &label, &words.join(" "));
                    inbound::inbound(&label, &chat, "", &text, 0);
                }
                if let Some(n) = heard.notice {
                    inbound::inbound("system", &chat, "", &n, 0);
                }
            }
            Err(e) => inbound::log(&format!("speech: {e}")),
        }
    });
}

// --- tools ---

pub fn status() -> Result<String, String> {
    let r = helper::get().call("call_status", json!({}), QUICK)?;
    let mut lines = Vec::new();
    if !s(&r, "problem").is_empty() {
        lines.push(format!("calls cannot work here: {}", s(&r, "problem")));
    }
    let n = |k: &str| r.get(k).and_then(Value::as_i64).unwrap_or(0);
    if !s(&r, "state").is_empty() {
        lines.push(format!(
            "call {} with {} on max:{} ({}): heard {} frames (peak {}), sent {} frames",
            s(&r, "state"),
            inbound::label(s(&r, "peer"), s(&r, "name")),
            s(&r, "chat"),
            if r.get("incoming").and_then(Value::as_bool).unwrap_or(false) { "incoming" } else { "outgoing" },
            n("frames_in"),
            n("peak_in"),
            n("frames_out"),
        ));
    } else {
        lines.push("no call".to_string());
        if n("last_frames_in") > 0 || n("last_frames_out") > 0 {
            lines.push(format!("last call: heard {} frames (peak {}), sent {} frames", n("last_frames_in"), n("last_peak_in"), n("last_frames_out")));
        }
    }
    lines.push(bia_speech::status());
    Ok(lines.join("\n"))
}

pub fn start(chat: &str) -> Result<String, String> {
    let r = helper::get().call("call_start", json!({"chat": chat}), NORMAL)?;
    Ok(format!("calling {}; wait for [call ready] or [call ended]", inbound::format_chat(s(&r, "chat"), s(&r, "name"))))
}

pub fn accept(chat: &str) -> Result<String, String> {
    let r = helper::get().call("call_accept", json!({"chat": chat}), NORMAL)?;
    Ok(format!("answering max:{}; wait for [call ready]", s(&r, "chat")))
}

/// End the call, or let a goodbye that is still being spoken finish first.
pub fn hangup(chat: &str) -> Result<String, String> {
    {
        let mut g = state().lock().unwrap();
        if let Some(a) = g.as_mut().filter(|a| a.speaking && (chat.is_empty() || a.chat == chat)) {
            a.hang_up_after = true;
            return Ok("hanging up when the current sentence has been spoken".into());
        }
    }
    let params = if chat.is_empty() { json!({}) } else { json!({"chat": chat}) };
    let r = helper::get().call("call_hangup", params, NORMAL)?;
    Ok(format!("{} max:{}", if s(&r, "reason") == "REJECTED" { "rejected" } else { "hung up" }, s(&r, "chat")))
}

pub fn say(chat: &str, text: &str, lang: &str) -> Result<String, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("text is empty".into());
    }
    if state().lock().unwrap().as_ref().is_none_or(|a| !a.ready) {
        return Err("no active call".into());
    }
    let lang = bia_speech::contacts::lang_or(lang, &channel(chat));
    bia_speech::tts::plan(&bia_speech::registry::current(), text, &lang)?;
    let (feed, was_speaking): (Feed, bool) = {
        let mut g = state().lock().unwrap();
        let a = g.as_mut().filter(|a| a.ready).ok_or("no active call")?;
        if a.chat != chat {
            return Err("chat_id does not match the active call".into());
        }
        a.utterance = a.utterance.wrapping_add(1);
        let was = a.speaking;
        a.speaking = true;
        let audio = a.audio.as_ref().ok_or("call audio not running")?;
        (audio.begin(a.utterance), was)
    };
    if was_speaking {
        inbound::inbound("system", chat, "", "[voice cancelled]", 0);
    }
    if let Ok(mut g) = spoken().lock() {
        *g = Some((text.to_string(), Instant::now()));
    }
    bia_speech::calllog::add(&channel(chat), "bia", text);
    crate::tts::speak(text.to_string(), lang, chat.to_string(), feed);
    Ok(format!("speaking on max:{chat}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm_round_trip() {
        let frame = vec![0i16, 1, -1, i16::MAX, i16::MIN];
        assert_eq!(pcm_samples(&pcm_bytes(&frame)), frame);
        assert_eq!(crate::audio_out::FRAME_SAMPLES, 960);
    }
}
