//! WhatsApp voice calls via whatsapp-rust's `voip` media plane + sherpa STT/TTS.
//!
//! Audio contract: 16 kHz mono i16, exactly 960 samples (60 ms) per frame.
//! - Microphone: `async_channel::Receiver<Vec<i16>>` handed to `.audio(..)`; fed by the
//!   paced `Outbound` thread (TTS PCM or silence).
//! - Speaker: `async_channel::Sender<Vec<i16>>`; a `call-stt` thread drains the receiver,
//!   runs VAD and transcribes utterances into `[speech]` inbound messages.

use crate::audio_out::Outbound;
use crate::client::{key_of_jid, key_of_jid_async, Target, Wa};
use crate::tts;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;
use whatsapp_rust::async_channel::{self, Receiver, Sender};
use whatsapp_rust::prelude::*;
use whatsapp_rust::voip::{CallEvent, CallHandle};
use whatsapp_rust::wacore::types::call::{CallAction, IncomingCall};
use whatsapp_rust::wacore_binary::JidExt;

/// Our speech counts as a possible echo this long after it finished.
const ECHO_SECS: u64 = 3;

struct Pending {
    incoming: IncomingCall,
    call_id: String,
    key: String,
    label: String,
    video: bool,
    group: Option<Jid>,
}

struct Active {
    handle: CallHandle,
    id: u64,
    key: String,
    label: String,
    group: bool,
    /// Paced outbound audio (TTS + silence).
    audio: Option<Outbound>,
    /// Speaker channel; closed on teardown so the STT thread exits.
    spk: Receiver<Vec<i16>>,
    /// "[call ready]" was emitted.
    ready: Arc<AtomicBool>,
}

#[derive(Default)]
struct State {
    pending: HashMap<String, Pending>,
    active: Option<Active>,
    /// A start/accept/join is in flight.
    starting: bool,
    /// Bumped on each call_say; a SpeakDone with a stale value is ignored.
    speak_gen: u64,
    speaking: bool,
    /// call_hangup while speaking: (target, utterance) to hang up once it has played.
    hangup_after: Option<(Option<String>, u64)>,
}

/// Longest a deferred hangup waits for the current utterance to finish.
const HANGUP_WAIT_SECS: u64 = 20;

pub struct Calls {
    st: Mutex<State>,
}

type Channels = (
    Sender<Vec<i16>>,
    Receiver<Vec<i16>>,
    Sender<Vec<i16>>,
    Receiver<Vec<i16>>,
);

fn channels() -> Channels {
    let (mic_tx, mic_rx) = async_channel::bounded::<Vec<i16>>(8);
    let (spk_tx, spk_rx) = async_channel::bounded::<Vec<i16>>(64);
    (mic_tx, mic_rx, spk_tx, spk_rx)
}

fn label_of(t: &Target) -> String {
    if t.title.is_empty() || t.title == t.key || t.title == format!("+{}", t.key) {
        format!("whatsapp:{}", t.key)
    } else {
        format!("{} ({})", t.title, t.key)
    }
}

/// Notify the peer and tear the local media task down.
/// `CallHandle::hangup` is local-only; without `<terminate>` the other side stays ringing/active.
async fn signal_terminate(client: &Arc<Client>, handle: &CallHandle) {
    let call_id = handle.call_id().to_string();
    let peer = handle.peer_jid();
    let creator = handle.call_creator().clone();
    if let Err(e) = client.voip().terminate(&call_id, &peer, &creator).await {
        // Still drop our side so we never leave a zombie local call.
        handle.hangup().await;
        eprintln!("whatsapp: terminate {call_id}: {e}");
    }
}

impl Calls {
    pub fn global() -> &'static Calls {
        static G: OnceLock<Calls> = OnceLock::new();
        G.get_or_init(|| Calls {
            st: Mutex::new(State::default()),
        })
    }

    /// Chat key of the active call, if any.
    pub fn active_key(&self) -> Option<String> {
        self.st.lock().unwrap().active.as_ref().map(|a| a.key.clone())
    }

    pub fn status_cmd(&self) -> String {
        let g = self.st.lock().unwrap();
        let mut parts = vec![
            bia_speech::status(),
            "stack=whatsapp-rust/voip (16kHz mono, 960-sample frames)".to_string(),
        ];
        if g.pending.is_empty() && g.active.is_none() && !g.starting {
            parts.push("calls=none".into());
        }
        for p in g.pending.values() {
            parts.push(format!(
                "pending whatsapp:{} from={} video={}",
                p.key, p.label, p.video
            ));
        }
        if let Some(a) = &g.active {
            parts.push(format!(
                "active {} whatsapp:{} ready={} speaking={}",
                if a.group { "group" } else { "private" },
                a.key,
                a.ready.load(Ordering::Relaxed),
                g.speaking
            ));
        }
        if g.starting {
            parts.push("starting".into());
        }
        let log = bia_speech::diag::tail(12);
        if log.is_empty() {
            parts.join("  ")
        } else {
            format!("{}\nrecent speech log (files/speech/logs/speech.log):\n{log}", parts.join("  "))
        }
    }

    fn claim(&self) -> Result<(), String> {
        let mut g = self.st.lock().unwrap();
        if g.active.is_some() || g.starting {
            return Err("a call is already active".into());
        }
        g.starting = true;
        Ok(())
    }

    fn release(&self) {
        self.st.lock().unwrap().starting = false;
    }

    // -- tool entry points (called from MCP worker threads) ----------------------

    pub fn start(&self, target: &str) -> Result<String, String> {
        let wa = Wa::global();
        let client = wa.require_ready()?;
        let t = wa.resolve_target(target)?;
        if t.jid.is_group() {
            return Err("call_start is for private chats; use call_join_group for groups".into());
        }
        self.claim()?;
        let (mic_tx, mic_rx, spk_tx, spk_rx) = channels();
        let res = wa.block_on(async {
            let voip = client.voip();
            let r = voip.call(&t.jid).audio(mic_rx, spk_tx).start().await;
            r
        });
        match res {
            Ok(handle) => {
                self.activate(handle, t.key.clone(), label_of(&t), false, mic_tx, spk_rx, false);
                Ok(format!(
                    "calling whatsapp:{}; wait for [call ready] or [call ended]",
                    t.key
                ))
            }
            Err(e) => {
                self.release();
                Err(format!("call: {e}"))
            }
        }
    }

    pub fn accept(&self, target: &str) -> Result<String, String> {
        let wa = Wa::global();
        let client = wa.require_ready()?;
        let key = wa.resolve_target(target)?.key;
        self.claim()?;
        let pending = self.st.lock().unwrap().pending.remove(&key);
        let Some(p) = pending else {
            self.release();
            return Err(format!("no pending incoming call for whatsapp:{key}"));
        };
        self.answer(wa, &client, p)
    }

    fn answer(&self, wa: &Wa, client: &Arc<Client>, p: Pending) -> Result<String, String> {
        let (mic_tx, mic_rx, spk_tx, spk_rx) = channels();
        let res = wa.block_on(async {
            let voip = client.voip();
            let r = voip.accept(&p.incoming).audio(mic_rx, spk_tx).start().await;
            r
        });
        match res {
            Ok(handle) => {
                let group = p.group.is_some();
                self.activate(handle, p.key.clone(), p.label.clone(), group, mic_tx, spk_rx, true);
                Ok(format!("accepted call whatsapp:{}", p.key))
            }
            Err(e) => {
                self.release();
                Err(format!("accept: {e}"))
            }
        }
    }

    pub fn join_group(&self, target: &str) -> Result<String, String> {
        let wa = Wa::global();
        let client = wa.require_ready()?;
        let t = wa.resolve_target(target)?;
        if !t.jid.is_group() {
            return Err("call_join_group requires a group chat_id (<id>@g.us)".into());
        }
        self.claim()?;
        // An active group call invitation for this group: answer it.
        let invite = {
            let mut g = self.st.lock().unwrap();
            let key = g
                .pending
                .values()
                .find(|p| {
                    p.key == t.key
                        || p.group.as_ref().is_some_and(|gj| key_of_jid(gj) == t.key)
                })
                .map(|p| p.key.clone());
            key.and_then(|k| g.pending.remove(&k))
        };
        if let Some(p) = invite {
            return self.answer(wa, &client, p);
        }
        // Otherwise place a group-bound call to the group's current roster.
        let (mic_tx, mic_rx, spk_tx, spk_rx) = channels();
        let res = wa.block_on(async {
            let voip = client.voip();
            let r = voip
                .group_call_by_id(&t.jid)
                .audio(mic_rx, spk_tx)
                .start()
                .await;
            r
        });
        match res {
            Ok(handle) => {
                self.activate(handle, t.key.clone(), label_of(&t), true, mic_tx, spk_rx, false);
                Ok(format!(
                    "started group call in whatsapp:{}; members are rung, wait for [call ready]",
                    t.key
                ))
            }
            Err(e) => {
                self.release();
                Err(format!(
                    "group call not available: {e} (WhatsApp only allows group calls for groups you can call, and the account must support them)"
                ))
            }
        }
    }

    pub fn hangup(&self, target: Option<&str>) -> Result<String, String> {
        let wa = Wa::global();
        let client = wa.require_ready()?;
        let want: Option<String> = match target.map(str::trim).filter(|t| !t.is_empty()) {
            Some(t) => Some(wa.resolve_target(t)?.key),
            None => None,
        };

        // Ringing incoming call: reject it.
        let pending = {
            let mut g = self.st.lock().unwrap();
            match &want {
                Some(k) => g.pending.remove(k),
                None if g.active.is_none() && g.pending.len() == 1 => {
                    let k = g.pending.keys().next().cloned();
                    k.and_then(|k| g.pending.remove(&k))
                }
                None => None,
            }
        };
        if let Some(p) = pending {
            let r = wa.block_on(async {
                let voip = client.voip();
                let r = voip.reject(&p.incoming).await;
                r
            });
            r.map_err(|e| format!("reject: {e}"))?;
            return Ok(format!("rejected incoming call whatsapp:{}", p.key));
        }

        // Speaking: let the goodbye play out first ([you finished speaking] → hang up).
        {
            let mut g = self.st.lock().unwrap();
            let ours = g.active.as_ref().is_some_and(|a| want.as_ref().is_none_or(|k| *k == a.key));
            if ours && g.speaking && g.hangup_after.is_none() {
                let utt = g.speak_gen;
                g.hangup_after = Some((want.clone(), utt));
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(HANGUP_WAIT_SECS));
                    let due = {
                        let mut g = Calls::global().st.lock().unwrap();
                        match g.hangup_after.take() {
                            Some((t, u)) if u == utt => {
                                g.speaking = false;
                                Some(t)
                            }
                            other => {
                                g.hangup_after = other;
                                None
                            }
                        }
                    };
                    if let Some(t) = due {
                        let _ = Calls::global().hangup(t.as_deref());
                    }
                });
                return Ok("hanging up as soon as what you are saying has played".into());
            }
            g.hangup_after = None;
        }

        let (active, was_speaking) = {
            let mut g = self.st.lock().unwrap();
            let was = g.speaking;
            match g.active.take() {
                Some(a) if want.as_ref().is_none_or(|k| *k == a.key) => {
                    g.speaking = false;
                    (Some(a), was)
                }
                Some(a) => {
                    g.active = Some(a);
                    return Err("chat_id does not match the active call".into());
                }
                None => (None, false),
            }
        };
        let Some(a) = active else {
            return Err("no active or pending call".into());
        };
        if was_speaking {
            wa.inbound("system", &a.key, "", "[voice cancelled]");
        }
        // hangup() is local-only; terminate sends <terminate> so the peer drops too.
        wa.block_on(signal_terminate(&client, &a.handle));
        let (key, group) = (a.key.clone(), a.group);
        self.finish_teardown(a, "[call ended]");
        Ok(format!(
            "{} whatsapp:{key}",
            if group { "left group call" } else { "hung up" }
        ))
    }

    pub fn say(&self, target: &str, text: &str, lang: &str) -> Result<String, String> {
        let wa = Wa::global();
        let key = wa.resolve_target(target)?.key;
        let text = text.trim();
        if text.is_empty() {
            return Err("text is empty".into());
        }
        if self.st.lock().unwrap().active.is_none() {
            return Err("no active call".into());
        }
        // no lang given: the caller's preferred language (addressbook)
        let lang = &bia_speech::contacts::lang_or(lang, &format!("whatsapp:{key}"));
        // Language and voices are checked now, so a missing voice is this call's error.
        bia_speech::tts::plan(&bia_speech::registry::current(), text, lang)?;
        let (feed, was_speaking) = {
            let mut g = self.st.lock().unwrap();
            match g.active.as_ref() {
                None => return Err("no active call".into()),
                Some(a) if a.key != key => {
                    return Err("chat_id does not match the active call".into())
                }
                Some(_) => {}
            }
            g.speak_gen = g.speak_gen.wrapping_add(1);
            let utt = g.speak_gen;
            let feed = match g.active.as_ref().and_then(|a| a.audio.as_ref()) {
                Some(o) => o.begin(utt),
                None => return Err("call audio not running".into()),
            };
            let was = g.speaking;
            g.speaking = true;
            (feed, was)
        };
        if was_speaking {
            wa.inbound("system", &key, "", "[voice cancelled]");
        }
        // Synthesis runs on the TTS thread and streams into the call's pacer;
        // [you finished speaking] arrives once the audio has played out.
        if let Ok(mut g) = spoken().lock() {
            *g = Some((text.to_string(), Instant::now()));
        }
        bia_speech::calllog::add(&format!("whatsapp:{key}"), "bia", text);
        tts::speak(text.to_string(), lang.to_string(), key.clone(), feed);
        Ok(format!("speaking on whatsapp:{key}"))
    }

    /// Drop everything (logout). Does not signal the peer; the session is going away.
    pub fn reset(&self) {
        let active = {
            let mut g = self.st.lock().unwrap();
            g.pending.clear();
            g.starting = false;
            g.speaking = false;
            g.active.take()
        };
        if let Some(a) = active {
            let _ = a.spk.close();
            drop(a);
        }
    }

    /// Auto-reject a ringing call from a freshly banned chat.
    pub fn reject_banned(&self, key: &str) {
        let wa = Wa::global();
        let pending = self.st.lock().unwrap().pending.remove(key);
        let (Some(p), Ok(client)) = (pending, wa.client()) else {
            return;
        };
        let r = wa.block_on(async {
            let voip = client.voip();
            let r = voip.reject(&p.incoming).await;
            r
        });
        if let Err(e) = r {
            wa.log(format!("reject banned call {key}: {e}"));
        }
    }

    // -- call lifecycle -----------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    fn activate(
        &self,
        handle: CallHandle,
        key: String,
        label: String,
        group: bool,
        mic_tx: Sender<Vec<i16>>,
        spk_rx: Receiver<Vec<i16>>,
        ready_now: bool,
    ) {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let done_key = key.clone();
        let audio = Outbound::start(mic_tx, &key, move |pacer, utt| {
            Calls::global().speak_done(pacer, utt, &done_key);
        });
        spawn_stt(spk_rx.clone(), id, key.clone(), label.clone());
        let ready = Arc::new(AtomicBool::new(ready_now));
        {
            let mut g = self.st.lock().unwrap();
            g.starting = false;
            g.speaking = false;
            g.active = Some(Active {
                handle: handle.clone(),
                id,
                key: key.clone(),
                label: label.clone(),
                group,
                audio: Some(audio),
                spk: spk_rx,
                ready,
            });
        }
        tts::warm();
        Wa::global().spawn(monitor(handle, id, key.clone()));
        if ready_now {
            Wa::global().inbound(&label, &key, "", "[call ready]");
            bia_speech::calllog::start(&format!("whatsapp:{key}"));
        }
    }

    /// First sign the peer is really on the line (answered / media flowing).
    fn mark_ready(&self, id: u64) {
        let who = {
            let g = self.st.lock().unwrap();
            match g.active.as_ref() {
                Some(a) if a.id == id && !a.ready.swap(true, Ordering::SeqCst) => {
                    Some((a.key.clone(), a.label.clone()))
                }
                _ => None,
            }
        };
        if let Some((key, label)) = who {
            Wa::global().inbound(&label, &key, "", "[call ready]");
            bia_speech::calllog::start(&format!("whatsapp:{key}"));
        }
    }

    fn mark_ready_by_call(&self, call_id: &str) {
        let id = {
            let g = self.st.lock().unwrap();
            g.active
                .as_ref()
                .filter(|a| a.handle.call_id() == call_id)
                .map(|a| a.id)
        };
        if let Some(id) = id {
            self.mark_ready(id);
        }
    }

    /// The call task finished (peer hung up, relay died, or we hung up).
    fn on_ended(&self, id: u64) {
        let taken = {
            let mut g = self.st.lock().unwrap();
            if g.active.as_ref().is_some_and(|a| a.id == id) {
                g.speaking = false;
                g.active.take()
            } else {
                None
            }
        };
        if let Some(a) = taken {
            self.finish_teardown(a, "[call ended]");
        }
    }

    fn finish_teardown(&self, a: Active, msg: &str) {
        let _ = a.spk.close();
        let (key, label) = (a.key.clone(), a.label.clone());
        // Dropping joins the pacer thread; never hold the state lock here (the
        // pacer's done callback takes it).
        drop(a);
        Wa::global().inbound(&label, &key, "", msg);
        // the transcript (both sides) goes to the chat
        std::thread::spawn(move || {
            if let Some(summary) = bia_speech::calllog::finish(&format!("whatsapp:{key}"), &label) {
                match Wa::global().send(&key, &summary, &[]) {
                    Ok(_) => bia_speech::diag::log("whatsapp", format!("transcript sent to whatsapp:{key}")),
                    Err(e) => bia_speech::diag::log("whatsapp", format!("transcript for whatsapp:{key} not sent: {e}")),
                }
            }
        });
    }

    async fn end_by_call(&self, call_id: &str, msg: &str) {
        let taken = {
            let mut g = self.st.lock().unwrap();
            if g.active.as_ref().is_some_and(|a| a.handle.call_id() == call_id) {
                g.speaking = false;
                g.active.take()
            } else {
                None
            }
        };
        if let Some(a) = taken {
            // Peer already signaled (or media died); local teardown only.
            a.handle.hangup().await;
            self.finish_teardown(a, msg);
        }
    }

    fn speak_done(&self, pacer: u64, utt: u64, key: &str) {
        let (finished, hangup) = {
            let mut g = self.st.lock().unwrap();
            let cur = g.active.as_ref().and_then(|a| a.audio.as_ref()).map(Outbound::id);
            if cur == Some(pacer) && utt == g.speak_gen {
                g.speaking = false;
                (true, g.hangup_after.take().map(|(t, _)| t))
            } else {
                (false, None)
            }
        };
        match (finished, hangup) {
            // On its own thread: tearing the call down joins this pacer thread.
            (true, Some(t)) => {
                std::thread::spawn(move || {
                    let _ = Calls::global().hangup(t.as_deref());
                });
            }
            (true, None) => Wa::global().inbound("system", key, "", "[you finished speaking]"),
            _ => {}
        }
    }

    // -- events from the bot -------------------------------------------------------

    pub async fn on_incoming(&self, client: Arc<Client>, ic: IncomingCall) {
        let wa = Wa::global();
        match &ic.action {
            CallAction::Offer {
                call_id,
                is_video,
                group_jid,
                ..
            } => {
                if ic.offline {
                    return;
                }
                let from = ic.from.to_non_ad();
                let from_key = key_of_jid_async(&client, &from).await;
                let label = match ic.notify.as_deref().map(str::trim) {
                    Some(n) if !n.is_empty() => format!("{n} ({from_key})"),
                    _ => format!("whatsapp:{from_key}"),
                };
                let group = group_jid.as_ref().map(|g| g.to_non_ad());
                let key = group
                    .as_ref()
                    .map(key_of_jid)
                    .unwrap_or_else(|| from_key.clone());
                if wa.is_banned(&key) || wa.is_banned(&from_key) {
                    let r = client.voip().reject(&ic).await;
                    if let Err(e) = r {
                        wa.log(format!("auto-reject banned call {key}: {e}"));
                    }
                    return;
                }
                {
                    let mut g = self.st.lock().unwrap();
                    g.pending.insert(
                        key.clone(),
                        Pending {
                            incoming: ic.clone(),
                            call_id: call_id.clone(),
                            key: key.clone(),
                            label: label.clone(),
                            video: *is_video,
                            group: group.clone(),
                        },
                    );
                }
                let in_group = if group.is_some() {
                    format!(" in group whatsapp:{key}")
                } else {
                    String::new()
                };
                let video = if *is_video {
                    " (video call; audio only is supported)"
                } else {
                    ""
                };
                wa.inbound(&label, &key, "", &format!("[incoming call] from {label}{in_group}{video}"));
            }
            CallAction::Accept { call_id, .. } => {
                self.mark_ready_by_call(call_id);
            }
            CallAction::Reject { call_id, .. } => {
                self.end_by_call(call_id, "[call rejected]").await;
            }
            CallAction::Terminate { call_id, .. } => {
                self.drop_pending_by_call(call_id);
                self.end_by_call(call_id, "[call ended]").await;
            }
            _ => {}
        }
    }

    pub async fn on_missed(&self, client: &Arc<Client>, from: Jid, call_id: String) {
        self.drop_pending_by_call(&call_id);
        let key = key_of_jid_async(client, &from).await;
        Wa::global().inbound(&format!("whatsapp:{key}"), &key, "", "[missed call]");
    }

    pub fn on_answered_elsewhere(&self, call_id: &str) {
        self.drop_pending_by_call(call_id);
    }

    fn drop_pending_by_call(&self, call_id: &str) {
        self.st
            .lock()
            .unwrap()
            .pending
            .retain(|_, p| p.call_id != call_id);
    }
}

/// Surfaces terminal relay failures and notices when the call task is over.
async fn monitor(handle: CallHandle, id: u64, key: String) {
    let events = handle.events();
    loop {
        tokio::select! {
            ev = events.recv() => match ev {
                Ok(CallEvent::RelayAllocateFailed(code)) => {
                    Wa::global().inbound("system", &key, "", &format!("[call failed: relay allocate error {code}]"));
                }
                Ok(CallEvent::RelayAllocateTimedOut) => {
                    Wa::global().inbound("system", &key, "", "[call failed: relay did not answer]");
                }
                Ok(CallEvent::RelayReconnectTimedOut) => {
                    Wa::global().inbound("system", &key, "", "[call failed: relay reconnect timed out]");
                }
                Ok(_) => {}
                Err(_) => break,
            },
            _ = handle.wait_ended() => break,
        }
    }
    Calls::global().on_ended(id);
}

// ---------------------------------------------------------------------------
// Speaker side: VAD + STT
// ---------------------------------------------------------------------------

/// What we are saying (or said last), for the opt-in echo filter.
fn spoken() -> &'static Mutex<Option<(String, Instant)>> {
    static S: OnceLock<Mutex<Option<(String, Instant)>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(None))
}

fn echo_ref() -> Option<String> {
    let speaking = Calls::global().st.lock().map(|g| g.speaking).unwrap_or(false);
    let g = spoken().lock().ok()?;
    let (text, at) = g.as_ref()?;
    (speaking || at.elapsed().as_secs() < ECHO_SECS).then(|| text.clone())
}

fn spawn_stt(rx: Receiver<Vec<i16>>, id: u64, key: String, label: String) {
    let _ = std::thread::Builder::new()
        .name("call-stt".into())
        .spawn(move || {
            bia_speech::warm();
            let mut listener = match bia_speech::Listener::with_lang(16_000, bia_speech::contacts::lang_for(&format!("whatsapp:{key}"))) {
                Ok(l) => {
                    bia_speech::diag::log("whatsapp", format!("listener ready call={id} chat={key} (16 kHz mono)"));
                    Some(l)
                }
                Err(e) => {
                    bia_speech::diag::log("whatsapp", format!("speech unavailable call={id}: {e}"));
                    Wa::global().log(format!("speech unavailable on this call: {e}"));
                    None
                }
            };
            let (mut frames, mut peak, mut since) = (0u64, 0i32, Instant::now());
            let mut first = true;
            while let Ok(frame) = rx.recv_blocking() {
                if first {
                    first = false;
                    // Media only flows once the other side is connected.
                    Calls::global().mark_ready(id);
                }
                frames += 1;
                peak = peak.max(frame.iter().map(|s| (*s as i32).abs()).max().unwrap_or(0));
                if frames == 1 || since.elapsed().as_secs() >= 5 {
                    bia_speech::diag::log("whatsapp", format!("call={id} frames={frames} samples/frame={} peak={peak}", frame.len()));
                    since = Instant::now();
                    peak = 0;
                }
                let Some(l) = listener.as_mut() else { continue };
                if let Some(utt) = l.push(&frame) {
                    bia_speech::diag::log("whatsapp", format!("utterance call={id} {:.2}s of speech", utt.secs()));
                    transcribe(utt, key.clone(), label.clone());
                }
            }
        });
}

fn transcribe(utt: bia_speech::Utterance, key: String, label: String) {
    let echo = echo_ref();
    std::thread::spawn(move || {
        let speaker = format!("whatsapp:{key}");
        match utt.transcribe(echo.as_deref(), Some(&speaker)) {
            Ok(heard) => {
                let text = heard.text();
                if !text.is_empty() {
                    let words: Vec<&str> = heard.runs.iter().map(|r| r.text.as_str()).collect();
                    bia_speech::calllog::add(&format!("whatsapp:{key}"), &label, &words.join(" "));
                    Wa::global().inbound(&label, &key, "", &text);
                }
                if let Some(n) = heard.notice {
                    Wa::global().inbound("system", &key, "", &n);
                }
            }
            Err(e) => {
                bia_speech::diag::log("whatsapp", format!("transcribe failed: {e}"));
                Wa::global().log(format!("speech: {e}"))
            }
        }
    });
}
