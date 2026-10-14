use ntgcalls::{
    AudioDescription, ConnectionInfo, ConnectionState, DhConfig, FrameData, MediaDescription,
    MediaSource, NTgCalls, RTCServer, StreamDevice, StreamMode,
};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::error::TgCallsError;
use crate::signaling;

/// Outbound External capture (mic slot), mono 48 kHz — 10 ms = 960 bytes/frame.
fn p2p_external_capture() -> MediaDescription {
    MediaDescription {
        microphone: Some(AudioDescription {
            media_source: MediaSource::External,
            sample_rate: 48_000,
            channel_count: 1,
            input: String::new(),
            keep_open: true,
        }),
        speaker: None,
        camera: None,
        screen: None,
    }
}

/// Inbound External playback on the **microphone** slot only.
///
/// ntgcalls `StreamManager::optimize_sources` calls
/// `enable_audio_incoming()` only when `external_writers_` contains Microphone,
/// not Speaker — PLAYBACK on the speaker slot never opens the inbound RTP path
/// (see ntgcalls `stream_manager.cpp`). TxPKev / ntgcalls#44 inbound frames use
/// `mode=PLAYBACK`, `device=Microphone`.
fn p2p_external_playback() -> MediaDescription {
    MediaDescription {
        microphone: Some(AudioDescription {
            media_source: MediaSource::External,
            sample_rate: 48_000,
            channel_count: 1,
            input: String::new(),
            keep_open: true,
        }),
        speaker: None,
        camera: None,
        screen: None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum P2PCallState {
    Idle,
    Requesting,
    Exchanging,
    Connecting,
    Connected,
    Ended,
}

/// The call ended after `connect()` had already succeeded - the other side
/// hung up, missed it, or declined. `request()`/`accept()` handle discards
/// *during* setup themselves (as an `Err`); this is only for a live call.
#[derive(Debug, Clone)]
pub enum P2PEvent {
    Discarded(Option<ferogram::tl::enums::PhoneCallDiscardReason>),
}

pub struct P2PCall {
    client: ferogram::Client,
    user_id: i64,
    ntg: NTgCalls,
    state: P2PCallState,
    call_id: Option<i64>,
    call_access_hash: Option<i64>,
}

impl P2PCall {
    pub fn new(client: ferogram::Client, user_id: i64) -> Self {
        Self {
            client,
            user_id,
            ntg: NTgCalls::new(),
            state: P2PCallState::Idle,
            call_id: None,
            call_access_hash: None,
        }
    }

    /// Outgoing call: ring the user, do DH exchange, connect, then set media.
    pub async fn request(
        &mut self,
        video: bool,
        stream: &mut ferogram::UpdateStream,
    ) -> Result<(Vec<RTCServer>, Vec<String>), TgCallsError> {
        if self.state != P2PCallState::Idle {
            return Err(TgCallsError::P2PAlreadyActive);
        }
        self.state = P2PCallState::Requesting;

        let dh = signaling::get_dh_config(&self.client).await?;
        let dh_config = DhConfig {
            g: dh.g,
            p: dh.p.clone(),
            random: dh.random.clone(),
        };

        self.ntg.create_p2p_call(self.user_id).await?;
        // CAPTURE must be set immediately after create_p2p_call, BEFORE
        // init_exchange — otherwise on_frames never delivers inbound audio
        // (verified: TxPKev p2p-offline-ai-telegram-bridge / ntgcalls#44).
        self.ntg
            .set_stream_sources(self.user_id, StreamMode::Capture, &p2p_external_capture())
            .await?;

        let g_a_hash_bytes = self
            .ntg
            .init_exchange(self.user_id, &dh_config, None)
            .await?;
        debug!(
            "p2p: init_exchange done, g_a_hash {} bytes",
            g_a_hash_bytes.len()
        );

        let access_hash = signaling::get_user_access_hash(&self.client, self.user_id).await?;

        let result = self
            .client
            .invoke(&ferogram::tl::functions::phone::RequestCall {
                video,
                user_id: ferogram::tl::enums::InputUser::InputUser(
                    ferogram::tl::types::InputUser {
                        user_id: self.user_id,
                        access_hash,
                    },
                ),
                random_id: rand_i32(),
                g_a_hash: g_a_hash_bytes,
                protocol: signaling::build_protocol()?,
            })
            .await?;

        let ferogram::tl::enums::phone::PhoneCall::PhoneCall(phone_call) = result;

        let (call_id, call_access_hash) = match phone_call.phone_call {
            ferogram::tl::enums::PhoneCall::Waiting(w) => (w.id, w.access_hash),
            ferogram::tl::enums::PhoneCall::Requested(r) => (r.id, r.access_hash),
            other => {
                return Err(TgCallsError::TransportParse(format!(
                    "unexpected PhoneCall state after requestCall: {:?}",
                    other
                )))
            }
        };

        self.call_id = Some(call_id);
        self.call_access_hash = Some(call_access_hash);
        info!("p2p: call {} requested to user {}", call_id, self.user_id);

        let (g_b, versions) = self.wait_for_accept(call_id, stream).await?;

        let auth = self.ntg.exchange_keys(self.user_id, &g_b, 0).await?;
        debug!(
            "p2p: exchange_keys done, fingerprint={}",
            auth.key_fingerprint
        );

        let confirm_result = self
            .client
            .invoke(&ferogram::tl::functions::phone::ConfirmCall {
                peer: signaling::input_phone_call(call_id, call_access_hash),
                g_a: auth.g_a_or_b,
                key_fingerprint: auth.key_fingerprint,
                protocol: signaling::build_protocol()?,
            })
            .await?;

        let servers = match confirm_result {
            ferogram::tl::enums::phone::PhoneCall::PhoneCall(pc) => match pc.phone_call {
                ferogram::tl::enums::PhoneCall::PhoneCall(c) => {
                    info!(
                        "p2p: confirmCall returned phoneCall with {} connections",
                        c.connections.len()
                    );
                    connections_to_rtc(&c.connections)
                }
                other => {
                    warn!(
                        "p2p: confirmCall returned unexpected PhoneCall state: {:?}",
                        other
                    );
                    self.wait_for_confirm(call_id, stream).await?
                }
            },
        };

        info!("p2p: call confirmed with user {}", self.user_id);
        self.state = P2PCallState::Exchanging;
        Ok((servers, versions))
    }

    /// Incoming call: accept it, do DH exchange, return servers for connect().
    ///
    /// `acceptCall` often returns `phoneCallWaiting` until the caller runs
    /// `confirmCall`; that is legal (TDLib keeps listening). Pass `stream` so
    /// we can wait for the ready `phoneCall` update with connections.
    pub async fn accept(
        &mut self,
        call: ferogram::tl::types::PhoneCallRequested,
        stream: &mut ferogram::UpdateStream,
    ) -> Result<(Vec<RTCServer>, Vec<String>), TgCallsError> {
        if self.state != P2PCallState::Idle {
            return Err(TgCallsError::P2PAlreadyActive);
        }

        let dh = signaling::get_dh_config(&self.client).await?;
        let dh_config = DhConfig {
            g: dh.g,
            p: dh.p.clone(),
            random: dh.random.clone(),
        };

        self.ntg.create_p2p_call(self.user_id).await?;
        // CAPTURE must be set immediately after create_p2p_call, BEFORE
        // init_exchange — otherwise on_frames never delivers inbound audio
        // (verified: TxPKev p2p-offline-ai-telegram-bridge / ntgcalls#44).
        self.ntg
            .set_stream_sources(self.user_id, StreamMode::Capture, &p2p_external_capture())
            .await?;
        let g_b = self
            .ntg
            .init_exchange(self.user_id, &dh_config, Some(&call.g_a_hash))
            .await?;
        debug!("p2p: accept init_exchange done, g_b {} bytes", g_b.len());

        self.client
            .invoke(&ferogram::tl::functions::phone::ReceivedCall {
                peer: signaling::input_phone_call(call.id, call.access_hash),
            })
            .await?;

        let result = self
            .client
            .invoke(&ferogram::tl::functions::phone::AcceptCall {
                peer: signaling::input_phone_call(call.id, call.access_hash),
                g_b,
                protocol: signaling::build_protocol()?,
            })
            .await?;

        self.call_id = Some(call.id);
        self.call_access_hash = Some(call.access_hash);

        let ferogram::tl::enums::phone::PhoneCall::PhoneCall(phone_call) = result;

        let (servers, versions) = match phone_call.phone_call {
            ferogram::tl::enums::PhoneCall::PhoneCall(c) => {
                self.finish_accept_ready(c).await?
            }
            // Waiting: caller has not confirmCall'd yet. Accepted without
            // connections also needs the ready phoneCall update (same as TDLib
            // WaitAcceptResult → keep listening).
            ferogram::tl::enums::PhoneCall::Waiting(w) => {
                info!(
                    "p2p: acceptCall returned Waiting for call {}, waiting for ready phoneCall",
                    w.id
                );
                self.wait_for_ready_after_accept(call.id, stream).await?
            }
            ferogram::tl::enums::PhoneCall::Accepted(a) => {
                info!(
                    "p2p: acceptCall returned Accepted for call {}, waiting for ready phoneCall",
                    a.id
                );
                self.wait_for_ready_after_accept(call.id, stream).await?
            }
            other => {
                return Err(TgCallsError::TransportParse(format!(
                    "unexpected PhoneCall state after acceptCall: {:?}",
                    other
                )))
            }
        };

        self.state = P2PCallState::Exchanging;
        info!("p2p: accepted call {} from user {}", call.id, self.user_id);
        Ok((servers, versions))
    }

    /// Connect to STUN/TURN servers and wire up signaling callbacks.
    ///
    /// Returns (sig_out_rx, conn_rx) channels for the signaling pump.
    /// `sig_out_rx`: outgoing signaling bytes ntgcalls wants sent to Telegram.
    /// `conn_rx`:    connection state: `true` = ICE+DTLS connected, `false` = failed.
    pub async fn connect(
        &mut self,
        servers: &[RTCServer],
        versions: &[String],
        p2p_allowed: bool,
    ) -> Result<(mpsc::Receiver<Vec<u8>>, mpsc::Receiver<bool>), TgCallsError> {
        let (sig_out_tx, sig_out_rx) = mpsc::channel::<Vec<u8>>(64);
        let (conn_tx, conn_rx) = mpsc::channel::<bool>(8);

        self.ntg.on_signaling_data(move |_user_id, data: Vec<u8>| {
            let _ = sig_out_tx.try_send(data);
        });

        self.ntg
            .on_connection_change(move |_user_id, info: ConnectionInfo| match info.state {
                ConnectionState::Connected => {
                    let _ = conn_tx.try_send(true);
                }
                ConnectionState::Failed | ConnectionState::Timeout | ConnectionState::Closed => {
                    let _ = conn_tx.try_send(false);
                }
                ConnectionState::Connecting => {}
            });

        // custom_parameters: unused for now.
        self.ntg
            .connect_p2p(self.user_id, servers, versions, p2p_allowed, None)
            .await?;
        self.state = P2PCallState::Connecting;
        debug!("p2p: connect_p2p called with {} servers", servers.len());

        // PLAYBACK External on mic slot right after connect_p2p (TxPKev ordering).
        self.arm_inbound_playback().await?;

        Ok((sig_out_rx, conn_rx))
    }

    /// PLAYBACK External on the microphone slot + unmute/resume. Safe to call again
    /// after ICE reaches Connected (re-runs `optimize_sources`).
    pub async fn arm_inbound_playback(&self) -> Result<(), TgCallsError> {
        if matches!(self.state, P2PCallState::Idle | P2PCallState::Ended) {
            return Err(TgCallsError::P2PNotActive);
        }
        self.ntg
            .set_stream_sources(
                self.user_id,
                StreamMode::Playback,
                &p2p_external_playback(),
            )
            .await?;
        let _ = self.ntg.unmute(self.user_id).await;
        let _ = self.ntg.resume(self.user_id).await;
        debug!("p2p: PLAYBACK External (mic slot) + unmute/resume");
        Ok(())
    }

    /// Pump signaling in both directions until ICE+DTLS is up or fails.
    ///
    /// Must keep running until `conn_rx` fires - the remote's DTLS
    /// fingerprint can arrive after channel negotiation completes, so
    /// stopping early causes an ICE timeout. Returns true if connected,
    /// false if failed/timeout/discarded.
    pub async fn run_signaling(
        &mut self,
        sig_out_rx: &mut mpsc::Receiver<Vec<u8>>,
        conn_rx: &mut mpsc::Receiver<bool>,
        stream: &mut ferogram::UpdateStream,
    ) -> Result<bool, TgCallsError> {
        let call_id = self.call_id.ok_or(TgCallsError::P2PNotActive)?;
        let call_access_hash = self.call_access_hash.ok_or(TgCallsError::P2PNotActive)?;
        let peer = signaling::input_phone_call(call_id, call_access_hash);

        let client_clone = self.client.clone();
        let (out_err_tx, mut out_err_rx) = mpsc::channel::<TgCallsError>(4);
        let (out_tx, mut out_rx) = mpsc::channel::<Vec<u8>>(64);

        tokio::spawn(async move {
            while let Some(bytes) = out_rx.recv().await {
                debug!("p2p: sending {} signaling bytes to Telegram", bytes.len());
                if let Err(e) =
                    signaling::send_p2p_signaling(&client_clone, peer.clone(), &bytes).await
                {
                    let _ = out_err_tx.try_send(e);
                    break;
                }
            }
        });

        while let Ok(bytes) = sig_out_rx.try_recv() {
            let _ = out_tx.try_send(bytes);
        }

        loop {
            tokio::select! {
                biased;

                Some(bytes) = sig_out_rx.recv() => {
                    let _ = out_tx.try_send(bytes);
                }

                Some(e) = out_err_rx.recv() => {
                    return Err(e);
                }

                maybe_upd = stream.next_raw() => {
                    let upd = match maybe_upd {
                        Some(u) => u,
                        None => return Err(TgCallsError::TransportParse("client disconnected during signaling".into())),
                    };

                    match upd.constructor_id {
                        0x2661bf09 => {
                            if let ferogram::tl::enums::Update::PhoneCallSignalingData(sd) = upd.inner {
                                if sd.phone_call_id == call_id {
                                    debug!("p2p: received {} signaling bytes from Telegram", sd.data.len());
                                    self.ntg.send_signaling_data(self.user_id, &sd.data).await?;
                                }
                            }
                        }
                        0xab0f6b1e => {
                            if let ferogram::tl::enums::Update::PhoneCall(pc) = upd.inner {
                                if let ferogram::tl::enums::PhoneCall::Discarded(d) = pc.phone_call {
                                    if d.id == call_id {
                                        return Err(TgCallsError::TransportParse("call discarded during signaling".into()));
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }

                Some(connected) = conn_rx.recv() => {
                    if connected {
                        info!("p2p: ICE+DTLS connected");
                        self.state = P2PCallState::Connected;
                        while let Ok(bytes) = sig_out_rx.try_recv() {
                            let _ = out_tx.try_send(bytes);
                        }
                        return Ok(true);
                    } else {
                        warn!("p2p: WebRTC connection failed or timed out");
                        return Ok(false);
                    }
                }
            }
        }
    }

    /// Set audio/video sources. Capture can be attached before `connect()`;
    /// Playback should be attached after ICE+DTLS is up (see TxPKev / ntgcalls#44).
    pub async fn set_media(
        &self,
        stream_mode: StreamMode,
        media: &MediaDescription,
    ) -> Result<(), TgCallsError> {
        if matches!(self.state, P2PCallState::Idle | P2PCallState::Ended) {
            return Err(TgCallsError::P2PNotActive);
        }
        Ok(self
            .ntg
            .set_stream_sources(self.user_id, stream_mode, media)
            .await?)
    }

    /// Push a PCM/video frame into an External capture source.
    pub async fn send_external_frame(
        &self,
        device: StreamDevice,
        frame: &[u8],
        frame_data: &FrameData,
    ) -> Result<(), TgCallsError> {
        if matches!(self.state, P2PCallState::Idle | P2PCallState::Ended) {
            return Err(TgCallsError::P2PNotActive);
        }
        Ok(self
            .ntg
            .send_external_frame(self.user_id, device, frame, frame_data)
            .await?)
    }

    /// Synchronous frame sink for a real-time pacing thread (bia patch).
    ///
    /// # Safety
    /// Stop using the sink before this `P2PCall` is dropped.
    pub unsafe fn frame_sink(&self) -> crate::FrameSink {
        crate::FrameSink::new(self.ntg.external_frame_sender(), self.user_id)
    }

    pub async fn resume(&self) -> Result<(), TgCallsError> {
        if matches!(self.state, P2PCallState::Idle | P2PCallState::Ended) {
            return Err(TgCallsError::P2PNotActive);
        }
        let _ = self.ntg.resume(self.user_id).await?;
        Ok(())
    }

    pub async fn unmute(&self) -> Result<(), TgCallsError> {
        if matches!(self.state, P2PCallState::Idle | P2PCallState::Ended) {
            return Err(TgCallsError::P2PNotActive);
        }
        let _ = self.ntg.unmute(self.user_id).await?;
        Ok(())
    }

    /// Feed incoming signaling bytes from updatePhoneCallSignalingData
    /// manually (run_signaling already does this for you internally).
    pub async fn receive_signaling_data(&self, data: &[u8]) -> Result<(), TgCallsError> {
        if matches!(self.state, P2PCallState::Idle | P2PCallState::Ended) {
            return Err(TgCallsError::P2PNotActive);
        }
        Ok(self.ntg.send_signaling_data(self.user_id, data).await?)
    }

    /// P2P calls only ever have one peer, so both `user_id` args are the
    /// same peer this P2PCall was constructed with.
    pub async fn add_incoming_video(
        &self,
        endpoint: &str,
        ssrc_groups: &[ntgcalls::SsrcGroup],
    ) -> Result<u32, TgCallsError> {
        if matches!(self.state, P2PCallState::Idle | P2PCallState::Ended) {
            return Err(TgCallsError::P2PNotActive);
        }
        Ok(self
            .ntg
            .add_incoming_video(self.user_id, self.user_id, endpoint, ssrc_groups)
            .await?)
    }

    pub async fn remove_incoming_video(&self, endpoint: &str) -> Result<bool, TgCallsError> {
        if matches!(self.state, P2PCallState::Idle | P2PCallState::Ended) {
            return Err(TgCallsError::P2PNotActive);
        }
        Ok(self
            .ntg
            .remove_incoming_video(self.user_id, endpoint)
            .await?)
    }

    /// Registers a frame callback. Uses its own callback slot, independent
    /// of the signaling/connection callbacks `connect()` sets up.
    pub fn on_frames(
        &mut self,
        callback: impl Fn(i64, StreamMode, ntgcalls::StreamDevice, Vec<ntgcalls::Frame>)
            + Send
            + Sync
            + 'static,
    ) {
        self.ntg.on_frames(callback);
    }

    /// See the note on [`P2PCall::on_frames`].
    pub fn on_remote_source_change(
        &mut self,
        callback: impl Fn(i64, ntgcalls::RemoteSource) + Send + Sync + 'static,
    ) {
        self.ntg.on_remote_source_change(callback);
    }

    /// Re-export for callers that re-arm playback after TTS (same layout as connect).
    pub fn inbound_playback_media() -> MediaDescription {
        p2p_external_playback()
    }

    /// Hang up and discard the call.
    pub async fn end(&mut self) {
        if self.state == P2PCallState::Ended || self.state == P2PCallState::Idle {
            return;
        }
        if let Err(e) = self.ntg.stop(self.user_id).await {
            warn!("p2p: stop error (ignored): {}", e);
        }
        if let (Some(id), Some(hash)) = (self.call_id, self.call_access_hash) {
            if let Err(e) = signaling::discard_call(
                &self.client,
                signaling::input_phone_call(id, hash),
                0,
                false,
            )
            .await
            {
                warn!("p2p: discard_call error (ignored): {}", e);
            }
        }
        self.state = P2PCallState::Ended;
        info!("p2p: ended call with user {}", self.user_id);
    }

    pub fn state(&self) -> P2PCallState {
        self.state
    }

    /// Feed this raw `PhoneCall` updates from your own update loop after
    /// `connect()` has succeeded, to detect the other side hanging up on an
    /// already-live call. Returns `None` for anything not relevant to this
    /// call.
    pub fn handle_update(&mut self, call: &ferogram::tl::enums::PhoneCall) -> Option<P2PEvent> {
        let ferogram::tl::enums::PhoneCall::Discarded(d) = call else {
            return None;
        };
        if Some(d.id) != self.call_id {
            return None;
        }
        self.state = P2PCallState::Ended;
        Some(P2PEvent::Discarded(d.reason.clone()))
    }

    async fn finish_accept_ready(
        &mut self,
        c: ferogram::tl::types::PhoneCall,
    ) -> Result<(Vec<RTCServer>, Vec<String>), TgCallsError> {
        let auth = self
            .ntg
            .exchange_keys(self.user_id, &c.g_a_or_b, c.key_fingerprint)
            .await?;
        debug!(
            "p2p: accept exchange_keys done, fingerprint={}",
            auth.key_fingerprint
        );
        let servers = connections_to_rtc(&c.connections);
        let versions = extract_versions(&c.protocol);
        Ok((servers, versions))
    }

    /// After acceptCall returns Waiting/Accepted, wait for updatePhoneCall with
    /// the ready phoneCall (connections + g_a_or_b) from the caller's confirmCall.
    async fn wait_for_ready_after_accept(
        &mut self,
        call_id: i64,
        stream: &mut ferogram::UpdateStream,
    ) -> Result<(Vec<RTCServer>, Vec<String>), TgCallsError> {
        loop {
            let upd = tokio::time::timeout(std::time::Duration::from_secs(90), stream.next_raw())
                .await
                .map_err(|_| {
                    TgCallsError::TransportParse(
                        "timed out waiting for ready phoneCall after accept".into(),
                    )
                })?
                .ok_or_else(|| {
                    TgCallsError::TransportParse(
                        "client disconnected while waiting for ready phoneCall after accept"
                            .into(),
                    )
                })?;

            if upd.constructor_id != 0xab0f6b1e {
                continue;
            }

            if let ferogram::tl::enums::Update::PhoneCall(pc) = upd.inner {
                match pc.phone_call {
                    ferogram::tl::enums::PhoneCall::PhoneCall(c) if c.id == call_id => {
                        info!(
                            "p2p: got ready phoneCall for {} after accept, {} connections",
                            call_id,
                            c.connections.len()
                        );
                        return self.finish_accept_ready(c).await;
                    }
                    ferogram::tl::enums::PhoneCall::Waiting(w) if w.id == call_id => {
                        debug!("p2p: still Waiting for call {} after accept", call_id);
                    }
                    ferogram::tl::enums::PhoneCall::Accepted(a) if a.id == call_id => {
                        debug!(
                            "p2p: Accepted update for call {} after accept (waiting for ready)",
                            call_id
                        );
                    }
                    ferogram::tl::enums::PhoneCall::Discarded(d) if d.id == call_id => {
                        warn!(
                            "p2p: call {} discarded while waiting after accept, reason: {:?}",
                            call_id, d.reason
                        );
                        return Err(TgCallsError::TransportParse(format!(
                            "call discarded: {:?}",
                            d.reason
                        )));
                    }
                    ferogram::tl::enums::PhoneCall::Discarded(d) => {
                        debug!("p2p: ignoring stale Discarded for call {}", d.id);
                    }
                    other => {
                        debug!(
                            "p2p: PhoneCall update while waiting after accept: {:?}",
                            other
                        );
                    }
                }
            }
        }
    }

    async fn wait_for_accept(
        &self,
        call_id: i64,
        stream: &mut ferogram::UpdateStream,
    ) -> Result<(Vec<u8>, Vec<String>), TgCallsError> {
        loop {
            let upd = tokio::time::timeout(std::time::Duration::from_secs(90), stream.next_raw())
                .await
                .map_err(|_| {
                    TgCallsError::TransportParse("timed out waiting for remote to accept".into())
                })?
                .ok_or_else(|| {
                    TgCallsError::TransportParse(
                        "client disconnected while waiting for accept".into(),
                    )
                })?;

            if upd.constructor_id != 0xab0f6b1e {
                continue;
            }

            if let ferogram::tl::enums::Update::PhoneCall(pc) = upd.inner {
                match pc.phone_call {
                    ferogram::tl::enums::PhoneCall::Accepted(a) if a.id == call_id => {
                        let versions = extract_versions(&a.protocol);
                        info!(
                            "p2p: remote accepted call {}, {} versions",
                            call_id,
                            versions.len()
                        );
                        return Ok((a.g_b, versions));
                    }
                    ferogram::tl::enums::PhoneCall::Discarded(d) if d.id == call_id => {
                        warn!(
                            "p2p: call {} discarded while waiting for accept, reason: {:?}",
                            call_id, d.reason
                        );
                        return Err(TgCallsError::TransportParse(format!(
                            "call discarded: {:?}",
                            d.reason
                        )));
                    }
                    ferogram::tl::enums::PhoneCall::Discarded(d) => {
                        debug!("p2p: ignoring stale Discarded for call {}", d.id);
                    }
                    other => {
                        debug!(
                            "p2p: unexpected PhoneCall update while waiting for accept: {:?}",
                            other
                        );
                    }
                }
            }
        }
    }

    async fn wait_for_confirm(
        &self,
        call_id: i64,
        stream: &mut ferogram::UpdateStream,
    ) -> Result<Vec<RTCServer>, TgCallsError> {
        loop {
            let upd = tokio::time::timeout(std::time::Duration::from_secs(30), stream.next_raw())
                .await
                .map_err(|_| {
                    TgCallsError::TransportParse(
                        "timed out waiting for phoneCall after confirm".into(),
                    )
                })?
                .ok_or_else(|| {
                    TgCallsError::TransportParse(
                        "client disconnected while waiting for confirm".into(),
                    )
                })?;

            if upd.constructor_id != 0xab0f6b1e {
                continue;
            }

            if let ferogram::tl::enums::Update::PhoneCall(pc) = upd.inner {
                match pc.phone_call {
                    ferogram::tl::enums::PhoneCall::PhoneCall(c) if c.id == call_id => {
                        info!(
                            "p2p: got phoneCall for {}, {} connections",
                            call_id,
                            c.connections.len()
                        );
                        return Ok(connections_to_rtc(&c.connections));
                    }
                    ferogram::tl::enums::PhoneCall::Discarded(d) if d.id == call_id => {
                        warn!(
                            "p2p: call {} discarded after confirmCall, reason: {:?}",
                            call_id, d.reason
                        );
                        return Err(TgCallsError::TransportParse(format!(
                            "call discarded: {:?}",
                            d.reason
                        )));
                    }
                    ferogram::tl::enums::PhoneCall::Discarded(d) => {
                        debug!("p2p: ignoring stale Discarded for call {}", d.id);
                    }
                    other => {
                        debug!(
                            "p2p: unexpected PhoneCall update while waiting for confirm: {:?}",
                            other
                        );
                    }
                }
            }
        }
    }
}

impl Drop for P2PCall {
    fn drop(&mut self) {
        if self.state != P2PCallState::Idle && self.state != P2PCallState::Ended {
            warn!("p2p: P2PCall dropped without calling end()");
            let ntg = std::mem::take(&mut self.ntg);
            let user_id = self.user_id;
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                let handle2 = handle.clone();
                handle.spawn_blocking(move || {
                    handle2.block_on(async move {
                        let _ = ntg.stop(user_id).await;
                    });
                });
            }
        }
    }
}

fn connections_to_rtc(connections: &[ferogram::tl::enums::PhoneConnection]) -> Vec<RTCServer> {
    connections
        .iter()
        .map(|c| match c {
            ferogram::tl::enums::PhoneConnection::Webrtc(w) => {
                let ipv6 = if w.ipv6.is_empty() {
                    w.ip.clone()
                } else {
                    w.ipv6.clone()
                };
                RTCServer {
                    id: w.id as u64,
                    ipv4: w.ip.clone(),
                    ipv6,
                    username: w.username.clone(),
                    password: w.password.clone(),
                    port: w.port as u16,
                    turn: w.turn,
                    stun: w.stun,
                    tcp: false,
                    peer_tag: vec![],
                }
            }
            ferogram::tl::enums::PhoneConnection::PhoneConnection(c) => {
                let ipv6 = if c.ipv6.is_empty() {
                    c.ip.clone()
                } else {
                    c.ipv6.clone()
                };
                RTCServer {
                    id: c.id as u64,
                    ipv4: c.ip.clone(),
                    ipv6,
                    username: String::new(),
                    password: String::new(),
                    port: c.port as u16,
                    turn: false,
                    stun: false,
                    tcp: c.tcp,
                    peer_tag: c.peer_tag.clone(),
                }
            }
        })
        .collect()
}

fn extract_versions(protocol: &ferogram::tl::enums::PhoneCallProtocol) -> Vec<String> {
    match protocol {
        ferogram::tl::enums::PhoneCallProtocol::PhoneCallProtocol(p) => p.library_versions.clone(),
    }
}

fn rand_i32() -> i32 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::time::SystemTime;
    let mut h = DefaultHasher::new();
    SystemTime::now().hash(&mut h);
    h.finish() as i32
}
