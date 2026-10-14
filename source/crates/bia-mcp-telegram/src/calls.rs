//! Telegram voice calls via tgcalls (P2PCall + Calls) + bia-speech (Silero VAD,
//! Whisper with per-phrase language detection, a voice per language).

use crate::audio_out::Outbound;
use crate::client::Tg;
use crate::speech;
use crate::tts;
use ferogram::tl;
use ferogram::{Client, UpdateStream};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc as mpsc_std;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tgcalls::{
    Call, Frame, FrameData, Media, MediaDescription, P2PCall, P2PCallState, StreamDevice,
    StreamMode, VideoRotation,
};
use tokio::sync::{mpsc, oneshot};

/// Our speech counts as a possible echo this long after it finished.
const ECHO_SECS: u64 = 3;

pub enum CallCmd {
    Start {
        target: String,
        reply: oneshot::Sender<Result<String, String>>,
    },
    Accept {
        target: String,
        reply: oneshot::Sender<Result<String, String>>,
    },
    Hangup {
        target: Option<String>,
        reply: oneshot::Sender<Result<String, String>>,
    },
    Say {
        target: String,
        text: String,
        lang: String,
        reply: oneshot::Sender<Result<String, String>>,
    },
    JoinGroup {
        target: String,
        reply: oneshot::Sender<Result<String, String>>,
    },
    Status {
        reply: oneshot::Sender<String>,
    },
    /// A deferred hangup timed out (utterance `gen` never reported finished).
    ForceHangup {
        gen: u64,
    },
    /// Sent by a call's audio pacer once an utterance has fully played out.
    SpeakDone {
        pacer: u64,
        gen: u64,
        chat_id: i64,
    },
    RejectBanned {
        chat_id: i64,
    },
}

struct PendingIncoming {
    call: tl::types::PhoneCallRequested,
    user_id: i64,
    chat_id: i64,
    is_video: bool,
}

struct ActiveP2P {
    /// Paced outbound audio. Declared before `call` so it is joined before the
    /// call (and its ntgcalls handle) drops.
    audio: Option<Outbound>,
    call: P2PCall,
    chat_id: i64,
    user_id: i64,
    /// Keep signaling channel alive after connect for mid-call packets.
    sig_out_rx: mpsc::Receiver<Vec<u8>>,
}

struct ActiveGroup {
    /// See `ActiveP2P::audio`.
    audio: Option<Outbound>,
    call: Call,
    chat_id: i64,
}

/// Speech of one remote participant (SSRC).
struct SttState {
    listener: Option<bia_speech::Listener>,
    failed: bool,
}

impl SttState {
    fn new() -> Self {
        Self { listener: None, failed: false }
    }
}

/// What we are saying (or said last), for the opt-in echo filter.
fn spoken() -> &'static Mutex<Option<(String, Instant)>> {
    static S: OnceLock<Mutex<Option<(String, Instant)>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(None))
}

fn echo_ref() -> Option<String> {
    let g = spoken().lock().ok()?;
    let (text, at) = g.as_ref()?;
    let recent = Calls::global().is_speaking() || at.elapsed() < Duration::from_secs(ECHO_SECS);
    recent.then(|| text.clone())
}

/// Sync snapshot for status tool (updated from the update-loop task).
struct StatusSnap {
    pending: Vec<(i64, i64, bool)>, // chat_id, user_id, video
    active_p2p: Option<i64>,
    active_p2p_user: Option<i64>,
    active_group: Option<i64>,
    /// True while local TTS is playing into the active call.
    speaking: bool,
    /// Group-call audio SSRC → Telegram user id (Playback frames).
    ssrc_users: HashMap<i64, i64>,
}

pub struct Calls {
    snap: Mutex<StatusSnap>,
}

impl Calls {
    pub fn global() -> &'static Calls {
        static G: OnceLock<Calls> = OnceLock::new();
        G.get_or_init(|| Calls {
            snap: Mutex::new(StatusSnap {
                pending: Vec::new(),
                active_p2p: None,
                active_p2p_user: None,
                active_group: None,
                speaking: false,
                ssrc_users: HashMap::new(),
            }),
        })
    }

    fn set_snap(
        &self,
        pending: &HashMap<i64, PendingIncoming>,
        p2p: Option<(i64, i64)>,
        group: Option<i64>,
    ) {
        let mut g = self.snap.lock().unwrap();
        g.pending = pending
            .values()
            .map(|p| (p.chat_id, p.user_id, p.is_video))
            .collect();
        g.active_p2p = p2p.map(|(c, _)| c);
        g.active_p2p_user = p2p.map(|(_, u)| u);
        g.active_group = group;
        let clear_stt = p2p.is_none() && group.is_none();
        if clear_stt {
            g.speaking = false;
            g.ssrc_users.clear();
        }
        drop(g);
        if clear_stt {
            if let Ok(mut states) = stt_states().lock() {
                states.clear();
            }
        }
    }

    /// Active private/group call chat id, if any.
    pub fn active_chat(&self) -> Option<i64> {
        let g = self.snap.lock().unwrap();
        g.active_p2p.or(g.active_group)
    }

    pub fn is_speaking(&self) -> bool {
        self.snap.lock().unwrap().speaking
    }

    fn set_speaking(&self, on: bool) {
        if let Ok(mut g) = self.snap.lock() {
            g.speaking = on;
        }
    }

    fn set_ssrc_users(&self, map: HashMap<i64, i64>) {
        if let Ok(mut g) = self.snap.lock() {
            g.ssrc_users = map;
        }
    }

    fn user_for_ssrc(&self, ssrc: i64) -> Option<i64> {
        let g = self.snap.lock().ok()?;
        if let Some(u) = g.active_p2p_user {
            return Some(u);
        }
        g.ssrc_users.get(&ssrc).copied()
    }

    pub fn status(&self) -> String {
        let g = self.snap.lock().unwrap();
        let mut parts = Vec::new();
        parts.push(bia_speech::status());
        parts.push(format!("stack=ferogram+tgcalls/ntgcalls"));
        if g.pending.is_empty() && g.active_p2p.is_none() && g.active_group.is_none() {
            parts.push("calls=none".into());
        }
        for (chat, user, video) in &g.pending {
            parts.push(format!(
                "pending telegram:{chat} user={user} video={video}"
            ));
        }
        if let Some(chat) = g.active_p2p {
            parts.push(format!("active private telegram:{chat}"));
        }
        if let Some(chat) = g.active_group {
            parts.push(format!("active group telegram:{chat}"));
        }
        let log = bia_speech::diag::tail(12);
        if log.is_empty() {
            parts.join("  ")
        } else {
            format!("{}\nrecent speech log (files/speech/logs/speech.log):\n{log}", parts.join("  "))
        }
    }

    fn send_cmd(cmd: CallCmd) -> Result<(), String> {
        Tg::global()
            .call_tx()?
            .send(cmd)
            .map_err(|_| "call command channel closed".into())
    }

    fn sync_cmd(
        build: impl FnOnce(oneshot::Sender<Result<String, String>>) -> CallCmd,
    ) -> Result<String, String> {
        let (tx, rx) = oneshot::channel();
        Self::send_cmd(build(tx))?;
        rx.blocking_recv()
            .map_err(|_| "call command dropped".to_string())?
    }

    pub fn start(&self, target: &str) -> Result<String, String> {
        Self::sync_cmd(|reply| CallCmd::Start {
            target: target.to_string(),
            reply,
        })
    }

    pub fn accept(&self, target: &str) -> Result<String, String> {
        Self::sync_cmd(|reply| CallCmd::Accept {
            target: target.to_string(),
            reply,
        })
    }

    pub fn hangup(&self, target: Option<&str>) -> Result<String, String> {
        Self::sync_cmd(|reply| CallCmd::Hangup {
            target: target.map(str::to_string),
            reply,
        })
    }

    /// Fire-and-forget: end a deferred hangup that waited too long.
    fn force_hangup(&self, gen: u64) {
        let _ = Self::send_cmd(CallCmd::ForceHangup { gen });
    }

    pub fn say(&self, target: &str, text: &str, lang: &str) -> Result<String, String> {
        Self::sync_cmd(|reply| CallCmd::Say {
            target: target.to_string(),
            text: text.to_string(),
            lang: lang.to_string(),
            reply,
        })
    }

    pub fn join_group(&self, target: &str) -> Result<String, String> {
        Self::sync_cmd(|reply| CallCmd::JoinGroup {
            target: target.to_string(),
            reply,
        })
    }

    pub fn status_cmd(&self) -> String {
        let (tx, rx) = oneshot::channel();
        if Self::send_cmd(CallCmd::Status { reply: tx }).is_err() {
            return self.status();
        }
        rx.blocking_recv().unwrap_or_else(|_| self.status())
    }
}

pub struct CallLoopState {
    pending: HashMap<i64, PendingIncoming>,
    p2p: Option<ActiveP2P>,
    group: Option<ActiveGroup>,
    #[allow(dead_code)]
    stt: SttState,
    stt_chat: i64,
    /// Bumped on each call_say; SpeakDone with a stale gen is ignored.
    speak_gen: u64,
    /// call_hangup while speaking: hang up (this target) once the utterance has played.
    hangup_after: Option<Option<String>>,
}

/// Longest a deferred hangup waits for the current utterance to finish.
const HANGUP_WAIT_SECS: u64 = 20;

impl CallLoopState {
    pub fn new() -> Self {
        Self {
            pending: HashMap::new(),
            p2p: None,
            group: None,
            stt: SttState::new(),
            stt_chat: 0,
            speak_gen: 0,
            hangup_after: None,
        }
    }

    fn refresh_snap(&self) {
        Calls::global().set_snap(
            &self.pending,
            self.p2p.as_ref().map(|c| (c.chat_id, c.user_id)),
            self.group.as_ref().map(|c| c.chat_id),
        );
    }

    pub async fn handle_cmd(
        &mut self,
        client: &Client,
        stream: &mut UpdateStream,
        cmd: CallCmd,
    ) {
        match cmd {
            CallCmd::Start { target, reply } => {
                let _ = reply.send(self.do_start(client, stream, &target).await);
            }
            CallCmd::Accept { target, reply } => {
                let _ = reply.send(self.do_accept(client, stream, &target).await);
            }
            CallCmd::Hangup { target, reply } => {
                let in_call = self.p2p.is_some() || self.group.is_some();
                if in_call && Calls::global().is_speaking() && self.hangup_after.is_none() {
                    // let the goodbye play out first ([you finished speaking] → hang up)
                    self.hangup_after = Some(target.clone());
                    let gen = self.speak_gen;
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_secs(HANGUP_WAIT_SECS));
                        Calls::global().force_hangup(gen);
                    });
                    let _ = reply.send(Ok("hanging up as soon as what you are saying has played".into()));
                } else {
                    self.hangup_after = None;
                    let _ = reply.send(self.do_hangup(client, target.as_deref()).await);
                }
            }
            CallCmd::Say { target, text, lang, reply } => {
                let _ = reply.send(self.do_say(&target, &text, &lang).await);
            }
            CallCmd::JoinGroup { target, reply } => {
                let _ = reply.send(self.do_join_group(client, &target).await);
            }
            CallCmd::Status { reply } => {
                self.refresh_snap();
                let _ = reply.send(Calls::global().status());
            }
            CallCmd::SpeakDone { pacer, gen, chat_id } => {
                let announce = self.hangup_after.is_none();
                if self.finish_speak(pacer, gen, chat_id, announce).await && !announce {
                    let target = self.hangup_after.take().flatten();
                    let _ = self.do_hangup(client, target.as_deref()).await;
                }
            }
            CallCmd::ForceHangup { gen } => {
                // the deferred hangup waited long enough
                if self.hangup_after.is_some() && gen == self.speak_gen {
                    let target = self.hangup_after.take().flatten();
                    Calls::global().set_speaking(false);
                    let _ = self.do_hangup(client, target.as_deref()).await;
                }
            }
            CallCmd::RejectBanned { chat_id } => {
                self.reject_banned(client, chat_id).await;
            }
        }
        self.refresh_snap();
    }

    pub async fn on_raw(&mut self, client: &Client, raw: ferogram::update::RawUpdate) {
        match raw.inner {
            tl::enums::Update::PhoneCall(pc) => {
                self.on_phone_call(client, pc.phone_call).await;
            }
            tl::enums::Update::PhoneCallSignalingData(sd) => {
                if let Some(active) = self.p2p.as_mut() {
                    if active.call.state() == P2PCallState::Connected {
                        let _ = active.call.receive_signaling_data(&sd.data).await;
                    }
                }
            }
            _ => {}
        }
        // Drain outbound signaling while idle in the update loop.
        if let Some(active) = self.p2p.as_mut() {
            while let Ok(bytes) = active.sig_out_rx.try_recv() {
                if let (Some(id), Some(hash)) = (
                    // access via end() path — we re-send using DiscardCall peer fields
                    // stored on P2PCall privately; use receive only for inbound.
                    // Outbound mid-call: invoke SendSignalingData if we knew call id.
                    // P2PCall keeps ids private; run_signaling owned the pump.
                    // After connect we still have sig_out_rx — forward via invoke if possible.
                    Option::<i64>::None,
                    Option::<i64>::None,
                ) {
                    let _ = (id, hash, bytes);
                }
                let _ = bytes; // mid-call outbound is rare after ICE; ntgcalls still emits
            }
        }
        self.refresh_snap();
    }

    async fn on_phone_call(&mut self, client: &Client, call: tl::enums::PhoneCall) {
        match call {
            tl::enums::PhoneCall::Requested(r) => {
                let user_id = r.admin_id;
                let chat_id = user_id;
                if Tg::global().is_banned(chat_id) {
                    let _ = discard_phone_call(client, r.id, r.access_hash).await;
                    return;
                }
                let is_video = r.video;
                self.pending.insert(
                    chat_id,
                    PendingIncoming {
                        call: r,
                        user_id,
                        chat_id,
                        is_video,
                    },
                );
                let label = format!("user:{user_id}");
                Tg::global().inbound(&label, chat_id, "", &format!("[incoming call] from {label}"));
            }
            tl::enums::PhoneCall::Discarded(d) => {
                self.pending.retain(|_, p| p.call.id != d.id);
                if let Some(active) = self.p2p.as_ref() {
                    if let Some(ev) = {
                        // handle_update needs &mut — clone reason check via id
                        let _ = active;
                        None::<()>
                    } {
                        let _ = ev;
                    }
                }
                if let Some(mut active) = self.p2p.take() {
                    // If discarded matches, end locally
                    if active.call.handle_update(&tl::enums::PhoneCall::Discarded(d.clone())).is_some()
                    {
                        let chat = active.chat_id;
                        self.stop_speaking(&mut active.audio);
                        active.call.end().await;
                        Tg::global().inbound(&format!("user:{}", active.user_id), chat, "", "[call ended]");
                        post_transcript(chat);
                    } else {
                        self.p2p = Some(active);
                    }
                }
            }
            _ => {}
        }
        self.refresh_snap();
    }

    async fn reject_banned(&mut self, client: &Client, chat_id: i64) {
        if let Some(p) = self.pending.remove(&chat_id) {
            let _ = discard_phone_call(client, p.call.id, p.call.access_hash).await;
        }
        self.refresh_snap();
    }

    async fn do_start(
        &mut self,
        client: &Client,
        stream: &mut UpdateStream,
        target: &str,
    ) -> Result<String, String> {
        if self.p2p.is_some() || self.group.is_some() {
            return Err("a call is already active".into());
        }
        let (chat_id, _title, _un) = Tg::global().resolve_target_async(target).await?;
        if chat_id <= 0 {
            return Err("call_start only works for private user chats".into());
        }
        let user_id = chat_id;
        // Warm ferogram peer cache with a real access_hash before requestCall.
        // Outbound dials need InputUser{access_hash}; inbound updates already
        // supply it, which is why receive worked and dial failed.
        {
            let peer = client
                .resolve(user_id)
                .await
                .map_err(|e| format!("resolve callee: {e}"))?;
            let input = client
                .resolve_to_input_peer(&peer)
                .await
                .map_err(|e| {
                    format!(
                        "callee access_hash missing for user {user_id} ({e}); \
                         resolve @username or exchange a message first"
                    )
                })?;
            match input {
                tl::enums::InputPeer::User(u) if u.access_hash != 0 => {
                    stt_dbg(format!(
                        "outbound callee user={user_id} access_hash=ok"
                    ));
                }
                _ => {
                    return Err(format!(
                        "callee access_hash missing for user {user_id}; \
                         resolve @username or exchange a message first"
                    ));
                }
            }
        }
        let mut call = P2PCall::new(client.clone(), user_id);
        install_p2p_stt(&mut call);

        let (servers, versions) = call
            .request(false, stream)
            .await
            .map_err(|e| format!("request call: {e}"))?;
        // Capture is already armed inside request() right after create_p2p_call.

        let (mut sig_out_rx, mut conn_rx) = call
            .connect(&servers, &versions, true)
            .await
            .map_err(|e| format!("connect: {e}"))?;

        let ok = call
            .run_signaling(&mut sig_out_rx, &mut conn_rx, stream)
            .await
            .map_err(|e| format!("signaling: {e}"))?;
        if !ok {
            call.end().await;
            return Err("WebRTC connection failed".into());
        }

        self.finish_p2p_media(&mut call, chat_id, user_id).await?;

        self.stt_chat = chat_id;
        Calls::global().set_speaking(false);
        // SAFETY: stored ahead of `call` in ActiveP2P and stopped before it drops.
        let audio = Outbound::start(unsafe { call.frame_sink() }, chat_id, speak_done(chat_id));
        tts::warm();
        self.p2p = Some(ActiveP2P {
            audio: Some(audio),
            call,
            chat_id,
            user_id,
            sig_out_rx,
        });
        self.refresh_snap();
        Tg::global().inbound(&format!("user:{user_id}"), chat_id, "", "[call ready]");
        bia_speech::calllog::start(&format!("telegram:{chat_id}"));
        Ok(format!("calling telegram:{chat_id}; wait for [call ready]"))
    }

    async fn do_accept(
        &mut self,
        client: &Client,
        stream: &mut UpdateStream,
        target: &str,
    ) -> Result<String, String> {
        if self.p2p.is_some() || self.group.is_some() {
            return Err("a call is already active".into());
        }
        let (chat_id, _, _) = Tg::global().resolve_target_async(target).await?;
        let pending = self
            .pending
            .remove(&chat_id)
            .ok_or_else(|| format!("no pending incoming call for telegram:{chat_id}"))?;
        let user_id = pending.user_id;
        let mut call = P2PCall::new(client.clone(), user_id);
        install_p2p_stt(&mut call);

        let (servers, versions) = call
            .accept(pending.call, stream)
            .await
            .map_err(|e| format!("accept: {e}"))?;
        // Capture is already armed inside accept() right after create_p2p_call.

        let (mut sig_out_rx, mut conn_rx) = call
            .connect(&servers, &versions, true)
            .await
            .map_err(|e| format!("connect: {e}"))?;

        let ok = call
            .run_signaling(&mut sig_out_rx, &mut conn_rx, stream)
            .await
            .map_err(|e| format!("signaling: {e}"))?;
        if !ok {
            call.end().await;
            return Err("WebRTC connection failed".into());
        }

        self.finish_p2p_media(&mut call, chat_id, user_id).await?;

        self.stt_chat = chat_id;
        Calls::global().set_speaking(false);
        // SAFETY: stored ahead of `call` in ActiveP2P and stopped before it drops.
        let audio = Outbound::start(unsafe { call.frame_sink() }, chat_id, speak_done(chat_id));
        tts::warm();
        self.p2p = Some(ActiveP2P {
            audio: Some(audio),
            call,
            chat_id,
            user_id,
            sig_out_rx,
        });
        self.refresh_snap();
        Tg::global().inbound(&format!("user:{user_id}"), chat_id, "", "[call ready]");
        bia_speech::calllog::start(&format!("telegram:{chat_id}"));
        Ok(format!("accepted call telegram:{chat_id}"))
    }

    /// After ICE is up: mark active chat and prime outbound silence keepalive.
    async fn finish_p2p_media(
        &self,
        call: &mut P2PCall,
        chat_id: i64,
        user_id: i64,
    ) -> Result<(), String> {
        {
            let mut g = Calls::global().snap.lock().unwrap();
            g.active_p2p = Some(chat_id);
            g.active_p2p_user = Some(user_id);
        }
        // Re-arm after ICE Connected so optimize_sources sees external_writers_[Microphone].
        if let Err(e) = call.arm_inbound_playback().await {
            stt_dbg(format!("re-arm playback after ICE: {e}"));
        }
        let fd = audio_frame_data();
        if let Err(e) = call
            .send_external_frame(StreamDevice::Microphone, &SILENCE_FRAME, &fd)
            .await
        {
            stt_dbg(format!("prime silence: {e}"));
        }
        stt_dbg(format!(
            "p2p media armed chat={chat_id} {} (ntgcalls 3.0.0; mono PLAYBACK=mic; enable_audio_incoming)",
            bia_speech::status()
        ));
        Ok(())
    }

    /// Drop queued speech and stop the pacer ahead of ending the call.
    fn stop_speaking(&mut self, audio: &mut Option<Outbound>) {
        self.speak_gen = self.speak_gen.wrapping_add(1);
        Calls::global().set_speaking(false);
        drop(audio.take());
    }

    /// A new say while speaking replaces the current utterance (the agent
    /// interrupting itself). Incoming speech never touches outbound audio.
    fn begin_speech(&mut self, chat_id: i64, text: &str, lang: &str) -> Result<(), String> {
        let audio = match (self.p2p.as_ref(), self.group.as_ref()) {
            (Some(a), _) => a.audio.as_ref(),
            (_, Some(g)) => g.audio.as_ref(),
            _ => None,
        }
        .ok_or("call audio not running")?;
        if Calls::global().is_speaking() {
            Tg::global().inbound("system", chat_id, "", "[voice cancelled]");
        }
        self.speak_gen = self.speak_gen.wrapping_add(1);
        let feed = audio.begin(self.speak_gen);
        Calls::global().set_speaking(true);
        if let Ok(mut g) = spoken().lock() {
            *g = Some((text.to_string(), Instant::now()));
        }
        bia_speech::calllog::add(&format!("telegram:{chat_id}"), "bia", text);
        tts::speak(text.to_string(), lang.to_string(), chat_id, feed);
        Ok(())
    }

    async fn do_hangup(
        &mut self,
        client: &Client,
        target: Option<&str>,
    ) -> Result<String, String> {
        let want = match target {
            Some(t) => Some(Tg::global().resolve_target_async(t).await?.0),
            None => None,
        };

        if let Some(chat_id) = want {
            if let Some(p) = self.pending.remove(&chat_id) {
                let _ = discard_phone_call(client, p.call.id, p.call.access_hash).await;
                self.refresh_snap();
                return Ok(format!("rejected incoming call telegram:{chat_id}"));
            }
        } else if self.pending.len() == 1 && self.p2p.is_none() && self.group.is_none() {
            let chat_id = *self.pending.keys().next().unwrap();
            if let Some(p) = self.pending.remove(&chat_id) {
                let _ = discard_phone_call(client, p.call.id, p.call.access_hash).await;
                self.refresh_snap();
                return Ok(format!("rejected incoming call telegram:{chat_id}"));
            }
        }

        if let Some(mut active) = self.p2p.take() {
            if want.is_some_and(|c| c != active.chat_id) {
                self.p2p = Some(active);
                return Err("chat_id does not match the active call".into());
            }
            let chat = active.chat_id;
            let user = active.user_id;
            if Calls::global().is_speaking() {
                Tg::global().inbound("system", chat, "", "[voice cancelled]");
            }
            self.stop_speaking(&mut active.audio);
            active.call.end().await;
            Tg::global().inbound(&format!("user:{user}"), chat, "", "[call ended]");
            post_transcript(chat);
            self.refresh_snap();
            return Ok(format!("hung up telegram:{chat}"));
        }

        if let Some(mut g) = self.group.take() {
            if want.is_some_and(|c| c != g.chat_id) {
                self.group = Some(g);
                return Err("chat_id does not match the active group call".into());
            }
            let chat = g.chat_id;
            self.stop_speaking(&mut g.audio);
            let _ = g.call.leave().await;
            Tg::global().inbound("group", chat, "", "[call ended]");
            post_transcript(chat);
            self.refresh_snap();
            return Ok(format!("left group call telegram:{chat}"));
        }

        Err("no active or pending call".into())
    }

    async fn do_say(&mut self, target: &str, text: &str, lang: &str) -> Result<String, String> {
        let (chat_id, _, _) = Tg::global().resolve_target_async(target).await?;
        let text = text.trim();
        if text.is_empty() {
            return Err("text is empty".into());
        }
        if self.p2p.is_none() && self.group.is_none() {
            return Err("no active call".into());
        }
        // no lang given: the caller's preferred language (addressbook)
        let lang = &bia_speech::contacts::lang_or(lang, &format!("telegram:{chat_id}"));
        // Language and voices are checked now, so a missing voice is this call's error.
        bia_speech::tts::plan(&bia_speech::registry::current(), text, lang)?;
        // Synthesis runs on the TTS thread and streams into the call's pacer;
        // [you finished speaking] arrives once the audio has played out.
        if let Some(active) = self.p2p.as_ref() {
            if active.chat_id != chat_id {
                return Err("chat_id does not match the active call".into());
            }
            self.begin_speech(chat_id, text, lang)?;
            stt_dbg(format!("tts stream chat={chat_id} gen={}", self.speak_gen));
            return Ok(format!("speaking on telegram:{chat_id}"));
        }

        if let Some(g) = self.group.as_mut() {
            if g.chat_id != chat_id {
                return Err("chat_id does not match the active group call".into());
            }
            // Ensure Capture is External mono (same contract as silence).
            let quiet = quiet_capture();
            g.call
                .set_stream_sources(StreamMode::Capture, &quiet)
                .await
                .map_err(|e| format!("group say: {e}"))?;
            self.begin_speech(chat_id, text, lang)?;
            stt_dbg(format!("tts stream (group) chat={chat_id} gen={}", self.speak_gen));
            return Ok(format!("speaking on group telegram:{chat_id}"));
        }

        Err("no active call".into())
    }

    /// The current utterance played out; true when it was the current one.
    /// `announce`: tell the model `[you finished speaking]` (not when hanging up next).
    async fn finish_speak(&mut self, pacer: u64, gen: u64, chat_id: i64, announce: bool) -> bool {
        let current = match (self.p2p.as_ref(), self.group.as_ref()) {
            (Some(a), _) => a.audio.as_ref(),
            (_, Some(g)) => g.audio.as_ref(),
            _ => None,
        };
        if current.map(Outbound::id) != Some(pacer) || gen != self.speak_gen {
            return false;
        }
        Calls::global().set_speaking(false);
        if let Some(active) = self.p2p.as_mut() {
            if active.chat_id != chat_id {
                return false;
            }
            // Capture stays External; re-arm inbound in case ntgcalls flapped.
            let _ = active.call.arm_inbound_playback().await;
            if announce {
                Tg::global().inbound("system", chat_id, "", "[you finished speaking]");
            }
            return true;
        }
        if let Some(g) = self.group.as_mut() {
            if g.chat_id != chat_id {
                return false;
            }
            let quiet = quiet_capture();
            let _ = g
                .call
                .set_stream_sources(StreamMode::Capture, &quiet)
                .await;
            let _ = g.call.record(&playback_receive()).await;
            if let Ok(parts) = g.call.get_participants().await {
                Calls::global().set_ssrc_users(ssrc_map_from_participants(&parts));
            }
            if announce {
                Tg::global().inbound("system", chat_id, "", "[you finished speaking]");
            }
            return true;
        }
        false
    }

    async fn do_join_group(&mut self, client: &Client, target: &str) -> Result<String, String> {
        if self.p2p.is_some() || self.group.is_some() {
            return Err("a call is already active".into());
        }
        let (chat_id, _, _) = Tg::global().resolve_target_async(target).await?;
        if chat_id >= 0 {
            return Err("call_join_group requires a group or channel chat_id".into());
        }
        let mut call = Call::new(client.clone(), chat_id);
        install_group_stt(&mut call);
        let quiet = quiet_capture();
        call.join(quiet)
            .await
            .map_err(|e| format!("join group call: {e}"))?;
        call.record(&playback_receive())
            .await
            .map_err(|e| format!("set group playback: {e}"))?;
        if let Ok(parts) = call.get_participants().await {
            Calls::global().set_ssrc_users(ssrc_map_from_participants(&parts));
        }
        self.stt_chat = chat_id;
        Calls::global().set_speaking(false);
        // SAFETY: stored ahead of `call` in ActiveGroup and stopped before it drops.
        let audio = Outbound::start(unsafe { call.frame_sink() }, chat_id, speak_done(chat_id));
        tts::warm();
        self.group = Some(ActiveGroup {
            audio: Some(audio),
            call,
            chat_id,
        });
        self.refresh_snap();
        Tg::global().inbound("group", chat_id, "", "[call ready]");
        bia_speech::calllog::start(&format!("telegram:{chat_id}"));
        Ok(format!("joined group call telegram:{chat_id}"))
    }
}

/// Fires `CallCmd::SpeakDone` from the pacer thread.
fn speak_done(chat_id: i64) -> impl Fn(u64, u64) + Send + 'static {
    move |pacer, gen| {
        if let Ok(tx) = Tg::global().call_tx() {
            let _ = tx.send(CallCmd::SpeakDone { pacer, gen, chat_id });
        }
    }
}

/// Outgoing silence / TTS capture slot — mono 48 kHz (P2P External contract).
fn quiet_capture() -> MediaDescription {
    MediaDescription {
        microphone: Some(Media::external_audio(48_000, 1)),
        speaker: None,
        camera: None,
        screen: None,
    }
}

/// Incoming call audio for on_frames STT (group calls). P2P inbound uses
/// `P2PCall::arm_inbound_playback()` (PLAYBACK on microphone slot only).
fn playback_receive() -> MediaDescription {
    MediaDescription {
        microphone: Some(Media::external_audio(48_000, 1)),
        speaker: None,
        camera: None,
        screen: None,
    }
}

/// 10 ms of mono s16le @ 48 kHz = 960 bytes.
const SILENCE_FRAME: [u8; 960] = [0; 960];

fn audio_frame_data() -> FrameData {
    FrameData {
        absolute_capture_timestamp_ms: 0,
        rotation: VideoRotation::VideoRotation0,
        width: 0,
        height: 0,
    }
}

async fn discard_phone_call(client: &Client, id: i64, access_hash: i64) -> Result<(), String> {
    client
        .invoke(&tl::functions::phone::DiscardCall {
            video: false,
            peer: tl::enums::InputPhoneCall::InputPhoneCall(tl::types::InputPhoneCall {
                id,
                access_hash,
            }),
            duration: 0,
            reason: tl::enums::PhoneCallDiscardReason::Hangup,
            connection_id: 0,
        })
        .await
        .map_err(|e| format!("discard: {e}"))?;
    Ok(())
}

/// Call speech diagnostics → files/speech/logs/speech.log (tail in call_status).
fn stt_dbg(msg: impl Into<String>) {
    bia_speech::diag::log("telegram", msg.into());
}

fn mode_name(mode: StreamMode) -> &'static str {
    match mode {
        StreamMode::Capture => "Capture",
        StreamMode::Playback => "Playback",
    }
}

fn device_name(device: StreamDevice) -> &'static str {
    match device {
        StreamDevice::Microphone => "Microphone",
        StreamDevice::Speaker => "Speaker",
        StreamDevice::Camera => "Camera",
        StreamDevice::Screen => "Screen",
    }
}

type FrameBatch = (StreamMode, StreamDevice, Vec<Frame>);

/// ntgcalls invokes on_frames on its own media thread; only hand the batch
/// off here so VAD, locks and logging never stall it while we are speaking.
fn queue_frames(mode: StreamMode, device: StreamDevice, frames: Vec<Frame>) {
    static TX: OnceLock<mpsc_std::Sender<FrameBatch>> = OnceLock::new();
    let tx = TX.get_or_init(|| {
        let (tx, rx) = mpsc_std::channel::<FrameBatch>();
        std::thread::Builder::new()
            .name("call-stt".into())
            .spawn(move || {
                for (mode, device, frames) in rx {
                    forward_playback_frames(mode, device, frames);
                }
            })
            .expect("spawn call-stt thread");
        tx
    });
    let _ = tx.send((mode, device, frames));
}

fn stt_states() -> &'static Mutex<HashMap<i64, SttState>> {
    static STATES: OnceLock<Mutex<HashMap<i64, SttState>>> = OnceLock::new();
    STATES.get_or_init(|| Mutex::new(HashMap::new()))
}

struct SttDbgCounters {
    callbacks: AtomicU64,
    playback_bytes: AtomicU64,
    peak_rms: AtomicU64,
    last_summary: Mutex<Instant>,
    logged_first: AtomicU64,
}

fn stt_dbg_counters() -> &'static SttDbgCounters {
    static C: OnceLock<SttDbgCounters> = OnceLock::new();
    C.get_or_init(|| SttDbgCounters {
        callbacks: AtomicU64::new(0),
        playback_bytes: AtomicU64::new(0),
        peak_rms: AtomicU64::new(0),
        last_summary: Mutex::new(Instant::now()),
        logged_first: AtomicU64::new(0),
    })
}

fn forward_playback_frames(mode: StreamMode, device: StreamDevice, frames: Vec<Frame>) {
    let n = frames.len();
    let bytes: usize = frames.iter().map(|f| f.data.len()).sum();
    let ssrcs: Vec<(i64, usize)> = {
        let mut m: HashMap<i64, usize> = HashMap::new();
        for f in &frames {
            *m.entry(f.ssrc).or_default() += f.data.len();
        }
        m.into_iter().collect()
    };
    let max_rms = frames
        .iter()
        .filter(|f| !f.data.is_empty())
        .map(|f| speech::pcm16_rms(&f.data))
        .max()
        .unwrap_or(0);

    let c = stt_dbg_counters();
    let cb = c.callbacks.fetch_add(1, Ordering::Relaxed) + 1;
    if c.logged_first.fetch_add(1, Ordering::Relaxed) < 8 {
        stt_dbg(format!(
            "frame#{cb} mode={} device={} frames={n} bytes={bytes} max_rms={max_rms} ssrcs={ssrcs:?}",
            mode_name(mode),
            device_name(device),
        ));
    }
    if mode == StreamMode::Playback {
        c.playback_bytes.fetch_add(bytes as u64, Ordering::Relaxed);
        let _ = c.peak_rms.fetch_max(max_rms as u64, Ordering::Relaxed);
    }
    if let Ok(mut last) = c.last_summary.lock() {
        if last.elapsed() >= Duration::from_secs(2) {
            *last = Instant::now();
            let pb = c.playback_bytes.swap(0, Ordering::Relaxed);
            let peak = c.peak_rms.swap(0, Ordering::Relaxed);
            let snap = Calls::global().active_chat();
            stt_dbg(format!(
                "2s: callbacks={cb} playback_bytes={pb} peak_rms={peak} active_chat={snap:?} mode_now={} device_now={}",
                mode_name(mode),
                device_name(device),
            ));
        }
    }

    // Playback only — Capture is our TTS/mic and must not feed STT.
    if mode != StreamMode::Playback {
        return;
    }
    if device != StreamDevice::Speaker && device != StreamDevice::Microphone {
        return;
    }
    let mut by_ssrc: HashMap<i64, Vec<u8>> = HashMap::new();
    for f in frames {
        if f.data.is_empty() {
            continue;
        }
        by_ssrc
            .entry(f.ssrc)
            .or_default()
            .extend_from_slice(&f.data);
    }
    if by_ssrc.is_empty() {
        return;
    }
    // VAD runs on the call-stt thread (see queue_frames), never on ntgcalls'
    // media thread or the current-thread update runtime.
    let Ok(mut states) = stt_states().lock() else {
        return;
    };
    for (ssrc, pcm) in by_ssrc {
        let stt = states.entry(ssrc).or_insert_with(SttState::new);
        feed_stt(stt, ssrc, &pcm);
    }
}

fn install_p2p_stt(call: &mut P2PCall) {
    stt_dbg("install_p2p_stt: on_frames + remote_source registered (before create_p2p_call)");
    reset_stt();
    stt_dbg_counters().logged_first.store(0, Ordering::Relaxed);
    call.on_frames(move |_chat, mode, device, frames| {
        queue_frames(mode, device, frames);
    });
    call.on_remote_source_change(move |chat, src| {
        static N: AtomicU64 = AtomicU64::new(0);
        if N.fetch_add(1, Ordering::Relaxed) < 12 {
            stt_dbg(format!(
                "remote_source chat={chat} ssrc={} device={:?} state={:?}",
                src.ssrc, src.device, src.state
            ));
        }
    });
}

fn install_group_stt(call: &mut Call) {
    stt_dbg("install_group_stt: on_frames registered");
    reset_stt();
    stt_dbg_counters().logged_first.store(0, Ordering::Relaxed);
    call.on_frames(move |_chat, mode, device, frames| {
        queue_frames(mode, device, frames);
    });
}

fn ssrc_map_from_participants(
    parts: &[ferogram::tl::types::GroupCallParticipant],
) -> HashMap<i64, i64> {
    let mut map = HashMap::new();
    for p in parts {
        if p.is_self || p.left {
            continue;
        }
        let uid = match &p.peer {
            ferogram::tl::enums::Peer::User(u) => u.user_id,
            _ => continue,
        };
        if p.source != 0 {
            map.insert(i64::from(p.source), uid);
        }
    }
    map
}

/// After a call: send the transcript (both sides) to the chat.
fn post_transcript(chat: i64) {
    std::thread::spawn(move || {
        let target = chat.to_string();
        let name = Tg::global()
            .resolve_target(&target)
            .map(|(_, title, user)| if title.is_empty() { user } else { title })
            .unwrap_or_else(|_| "them".into());
        if let Some(summary) = bia_speech::calllog::finish(&format!("telegram:{chat}"), &name) {
            match Tg::global().send(&target, &summary, &[]) {
                Ok(_) => stt_dbg(format!("transcript sent to telegram:{chat}")),
                Err(e) => stt_dbg(format!("transcript for telegram:{chat} not sent: {e}")),
            }
        }
    });
}

/// New call: drop the listeners (and language memory) of the previous one.
fn reset_stt() {
    if let Ok(mut st) = stt_states().lock() {
        st.clear();
    }
    bia_speech::warm();
}

fn feed_stt(stt: &mut SttState, ssrc: i64, pcm: &[u8]) {
    let (chat, p2p) = {
        let g = Calls::global().snap.lock().ok();
        let Some(g) = g else {
            stt_dbg("feed: snap lock failed");
            return;
        };
        match (g.active_p2p, g.active_group) {
            (Some(c), _) => (c, true),
            (_, Some(c)) => (c, false),
            _ => {
                static NO_CHAT: AtomicU64 = AtomicU64::new(0);
                if NO_CHAT.fetch_add(1, Ordering::Relaxed) < 3 {
                    stt_dbg(format!("feed: no active_chat in snap (ssrc={ssrc} bytes={})", pcm.len()));
                }
                return;
            }
        }
    };
    if stt.listener.is_none() && !stt.failed {
        match bia_speech::Listener::with_lang(48_000, bia_speech::contacts::lang_for(&format!("telegram:{chat}"))) {
            Ok(l) => {
                stt_dbg(format!("listener ready ssrc={ssrc} chat={chat} (48 kHz mono)"));
                stt.listener = Some(l)
            }
            Err(e) => {
                stt.failed = true;
                Tg::global().log(format!("speech unavailable on this call: {e}"));
                return;
            }
        }
    }
    let Some(listener) = stt.listener.as_mut() else { return };
    let samples: Vec<i16> = pcm.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect();
    let Some(utt) = listener.push(&samples) else { return };
    stt_dbg(format!("utterance ssrc={ssrc} chat={chat} {:.2}s of speech", utt.secs()));
    let user = Calls::global().user_for_ssrc(ssrc).unwrap_or(ssrc);
    let echo = echo_ref();
    std::thread::spawn(move || {
        let secs = utt.secs();
        let key = format!("telegram:{user}");
        match utt.transcribe(echo.as_deref(), Some(&key)) {
            Ok(heard) => {
                for d in &heard.dropped {
                    stt_dbg(format!("dropped chat={chat} user={user}: {d}"));
                }
                let text = heard.text();
                if !text.is_empty() {
                    let words: Vec<&str> = heard.runs.iter().map(|r| r.text.as_str()).collect();
                    let who = if p2p { bia_speech::calllog::PEER.to_string() } else { format!("user:{user}") };
                    bia_speech::calllog::add(&format!("telegram:{chat}"), &who, &words.join(" "));
                    Tg::global().inbound(&format!("user:{user}"), chat, "", &text);
                } else {
                    stt_dbg(format!("nothing to deliver chat={chat} user={user} ({secs:.1}s)"));
                }
                if let Some(n) = heard.notice {
                    Tg::global().inbound("system", chat, "", &n);
                }
            }
            Err(e) => {
                stt_dbg(format!("transcript err chat={chat} {secs:.1}s: {e}"));
                Tg::global().log(format!("speech: {e}"));
            }
        }
    });
}

#[allow(dead_code)]
fn _wav_path() -> PathBuf {
    Tg::files_dir().join("tmp")
}
