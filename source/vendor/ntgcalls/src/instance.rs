// @generated from schema/ntgcalls.ntl -- DO NOT EDIT
use crate::enums::*;
use crate::error::{check, Result};
use crate::structs::*;
use crate::util::{self, FromC, ToC};
use ntgcalls_sys as sys;
use std::ffi::CString;
use std::os::raw::{c_char, c_void};

pub type UpgradeCallbackFn = dyn Fn(
    i64,
    MediaState
) + Send + Sync;
pub type StreamEndCallbackFn = dyn Fn(
    i64,
    StreamType,
    StreamDevice
) + Send + Sync;
pub type ConnectionChangeCallbackFn = dyn Fn(
    i64,
    ConnectionInfo
) + Send + Sync;
pub type FramesCallbackFn = dyn Fn(
    i64,
    StreamMode,
    StreamDevice,
    Vec<Frame>
) + Send + Sync;
pub type SignalingDataCallbackFn = dyn Fn(
    i64,
    Vec<u8>
) + Send + Sync;
pub type RemoteSourceChangeCallbackFn = dyn Fn(
    i64,
    RemoteSource
) + Send + Sync;
pub type RequestBroadcastPartCallbackFn = dyn Fn(
    i64,
    SegmentPartRequest
) + Send + Sync;
pub type RequestBroadcastTimestampCallbackFn = dyn Fn(
    i64
) + Send + Sync;
pub type RequestParticipantsCallbackFn = dyn Fn(
    i64
) + Send + Sync;
pub type OutboundBlockCallbackFn = dyn Fn(
    i64,
    Vec<u8>
) + Send + Sync;
pub type SubchainRequestCallbackFn = dyn Fn(
    i64,
    SubchainRequest
) + Send + Sync;
pub type UpdateEmojisCallbackFn = dyn Fn(
    i64,
    String
) + Send + Sync;

pub struct NTgCalls {
    handle: *mut sys::ntg_instance,
    on_upgrade_cb: Option<Box<Box<UpgradeCallbackFn>>>,
    on_stream_end_cb: Option<Box<Box<StreamEndCallbackFn>>>,
    on_connection_change_cb: Option<Box<Box<ConnectionChangeCallbackFn>>>,
    on_frames_cb: Option<Box<Box<FramesCallbackFn>>>,
    on_signaling_data_cb: Option<Box<Box<SignalingDataCallbackFn>>>,
    on_remote_source_change_cb: Option<Box<Box<RemoteSourceChangeCallbackFn>>>,
    on_request_broadcast_part_cb: Option<Box<Box<RequestBroadcastPartCallbackFn>>>,
    on_request_broadcast_timestamp_cb: Option<Box<Box<RequestBroadcastTimestampCallbackFn>>>,
    on_request_participants_cb: Option<Box<Box<RequestParticipantsCallbackFn>>>,
    on_outbound_block_cb: Option<Box<Box<OutboundBlockCallbackFn>>>,
    on_subchain_request_cb: Option<Box<Box<SubchainRequestCallbackFn>>>,
    on_update_emojis_cb: Option<Box<Box<UpdateEmojisCallbackFn>>>,
}

unsafe impl Send for NTgCalls {}

struct SendHandle(*mut sys::ntg_instance);
unsafe impl Send for SendHandle {}

/// Synchronous `send_external_frame` usable from a dedicated real-time thread
/// (bia patch). Avoids one `spawn_blocking` round-trip per 10 ms audio frame.
#[derive(Clone, Copy)]
pub struct ExternalFrameSender(*mut sys::ntg_instance);
unsafe impl Send for ExternalFrameSender {}
unsafe impl Sync for ExternalFrameSender {}

impl ExternalFrameSender {
    pub fn send(
        &self,
        chat_id: i64,
        device: StreamDevice,
        data: &[u8],
        frame_data: &FrameData,
    ) -> Result<()> {
        let mut conv = util::Conv::default();
        let frame_data_c = frame_data.to_c(&mut conv);
        check(unsafe {
            sys::ntg_send_external_frame(
                self.0,
                chat_id,
                device.into(),
                data.as_ptr(), data.len(),
                frame_data_c,
            )
        })
    }
}

impl NTgCalls {
    pub fn new() -> Self {
        Self {
            handle: unsafe { sys::ntg_instance_create() },
            on_upgrade_cb: None,
            on_stream_end_cb: None,
            on_connection_change_cb: None,
            on_frames_cb: None,
            on_signaling_data_cb: None,
            on_remote_source_change_cb: None,
            on_request_broadcast_part_cb: None,
            on_request_broadcast_timestamp_cb: None,
            on_request_participants_cb: None,
            on_outbound_block_cb: None,
            on_subchain_request_cb: None,
            on_update_emojis_cb: None,
        }
    }

    /// Raw-handle frame sender. Safety: the caller must stop using it before
    /// this `NTgCalls` is dropped (the handle is destroyed in `Drop`).
    pub unsafe fn external_frame_sender(&self) -> ExternalFrameSender {
        ExternalFrameSender(self.handle)
    }

    pub async fn create_p2p_call(
        &self,
        user_id: i64,
    ) -> Result<()> {
        let _sh = SendHandle(self.handle);
        tokio::task::spawn_blocking(move || {
            let _sh = _sh;
            let _handle = _sh.0;
        check(unsafe {
            sys::ntg_create_p2p_call(
                _handle,
                user_id,
            )
        })?;
        Ok(())
        })
        .await
        .expect("ntgcalls blocking task panicked")
    }

    pub async fn init_exchange(
        &self,
        user_id: i64,
        dh_config: &DhConfig,
        ga_hash: Option<&[u8]>,
    ) -> Result<Vec<u8>> {
        let dh_config = dh_config.clone();
        let ga_hash = ga_hash.map(|b| b.to_vec());
        let _sh = SendHandle(self.handle);
        tokio::task::spawn_blocking(move || {
            let _sh = _sh;
            let _handle = _sh.0;
        let mut conv = util::Conv::default();
        let ga_hash_ptr = ga_hash.as_ref().map_or(std::ptr::null(), |b| b.as_ptr());
        let ga_hash_len = ga_hash.as_ref().map_or(0, |b| b.len());
        let dh_config_c = dh_config.to_c(&mut conv);
        let mut out = std::ptr::null_mut();
        let mut out_len = 0;
        check(unsafe {
            sys::ntg_init_exchange(
                _handle,
                user_id,
                dh_config_c,
                ga_hash_ptr, ga_hash_len,
                &mut out, &mut out_len,
            )
        })?;
        let value = unsafe { util::bytes(out, out_len) };
        unsafe { sys::ntg_bytes_free(out as *mut _) };
        Ok(value)
        })
        .await
        .expect("ntgcalls blocking task panicked")
    }

    pub async fn exchange_keys(
        &self,
        user_id: i64,
        g_a_or_b: &[u8],
        fingerprint: i64,
    ) -> Result<AuthParams> {
        let g_a_or_b = g_a_or_b.to_vec();
        let _sh = SendHandle(self.handle);
        tokio::task::spawn_blocking(move || {
            let _sh = _sh;
            let _handle = _sh.0;
        let g_a_or_b_ptr = g_a_or_b.as_ptr();
        let g_a_or_b_len = g_a_or_b.len();
        let mut out = unsafe { std::mem::zeroed() };
        check(unsafe {
            sys::ntg_exchange_keys(
                _handle,
                user_id,
                g_a_or_b_ptr, g_a_or_b_len,
                fingerprint,
                &mut out,
            )
        })?;
        let value = unsafe { AuthParams::from_c(&out) };
        unsafe { sys::ntg_auth_params_free(&mut out) };
        Ok(value)
        })
        .await
        .expect("ntgcalls blocking task panicked")
    }

    pub async fn skip_exchange(
        &self,
        user_id: i64,
        encryption_key: &[u8],
        is_outgoing: bool,
    ) -> Result<()> {
        let encryption_key = encryption_key.to_vec();
        let _sh = SendHandle(self.handle);
        tokio::task::spawn_blocking(move || {
            let _sh = _sh;
            let _handle = _sh.0;
        let encryption_key_ptr = encryption_key.as_ptr();
        let encryption_key_len = encryption_key.len();
        check(unsafe {
            sys::ntg_skip_exchange(
                _handle,
                user_id,
                encryption_key_ptr, encryption_key_len,
                is_outgoing,
            )
        })?;
        Ok(())
        })
        .await
        .expect("ntgcalls blocking task panicked")
    }

    pub async fn connect_p2p(
        &self,
        user_id: i64,
        servers: &[RTCServer],
        versions: &[String],
        p2p_allowed: bool,
        custom_parameters: Option<&str>,
    ) -> Result<()> {
        let servers = servers.to_vec();
        let versions = versions.to_vec();
        let custom_parameters = custom_parameters.map(|s| s.to_owned());
        let _sh = SendHandle(self.handle);
        tokio::task::spawn_blocking(move || {
            let _sh = _sh;
            let _handle = _sh.0;
        let mut conv = util::Conv::default();
        let custom_parameters_c = custom_parameters.map(|s| CString::new(s).unwrap());
        let custom_parameters_ptr = custom_parameters_c.as_ref().map_or(std::ptr::null(), |c| c.as_ptr());
        let servers_c: Vec<_> = servers.iter().map(|e| e.to_c(&mut conv)).collect();
        let versions_c: Vec<_> = versions.iter().map(|e| e.to_c(&mut conv)).collect();
        check(unsafe {
            sys::ntg_connect_p2p(
                _handle,
                user_id,
                servers_c.as_ptr(), servers_c.len(),
                versions_c.as_ptr(), versions_c.len(),
                p2p_allowed,
                custom_parameters_ptr,
            )
        })?;
        Ok(())
        })
        .await
        .expect("ntgcalls blocking task panicked")
    }

    pub async fn create_call(
        &self,
        chat_id: i64,
    ) -> Result<String> {
        let _sh = SendHandle(self.handle);
        tokio::task::spawn_blocking(move || {
            let _sh = _sh;
            let _handle = _sh.0;
        let mut out = std::ptr::null_mut();
        check(unsafe {
            sys::ntg_create_call(
                _handle,
                chat_id,
                &mut out,
            )
        })?;
        Ok(util::take_string(out))
        })
        .await
        .expect("ntgcalls blocking task panicked")
    }

    pub async fn init_presentation(
        &self,
        chat_id: i64,
    ) -> Result<String> {
        let _sh = SendHandle(self.handle);
        tokio::task::spawn_blocking(move || {
            let _sh = _sh;
            let _handle = _sh.0;
        let mut out = std::ptr::null_mut();
        check(unsafe {
            sys::ntg_init_presentation(
                _handle,
                chat_id,
                &mut out,
            )
        })?;
        Ok(util::take_string(out))
        })
        .await
        .expect("ntgcalls blocking task panicked")
    }

    pub async fn init_conference(
        &self,
        chat_id: i64,
        user_id: i64,
        last_block: Option<&[u8]>,
    ) -> Result<ConferenceJoinParams> {
        let last_block = last_block.map(|b| b.to_vec());
        let _sh = SendHandle(self.handle);
        tokio::task::spawn_blocking(move || {
            let _sh = _sh;
            let _handle = _sh.0;
        let last_block_ptr = last_block.as_ref().map_or(std::ptr::null(), |b| b.as_ptr());
        let last_block_len = last_block.as_ref().map_or(0, |b| b.len());
        let mut out = unsafe { std::mem::zeroed() };
        check(unsafe {
            sys::ntg_init_conference(
                _handle,
                chat_id,
                user_id,
                last_block_ptr, last_block_len,
                &mut out,
            )
        })?;
        let value = unsafe { ConferenceJoinParams::from_c(&out) };
        unsafe { sys::ntg_conference_join_params_free(&mut out) };
        Ok(value)
        })
        .await
        .expect("ntgcalls blocking task panicked")
    }

    pub async fn connect(
        &self,
        chat_id: i64,
        params: &str,
        is_presentation: bool,
    ) -> Result<()> {
        let params = params.to_owned();
        let _sh = SendHandle(self.handle);
        tokio::task::spawn_blocking(move || {
            let _sh = _sh;
            let _handle = _sh.0;
        let params_c = CString::new(params).unwrap();
        let params_ptr = params_c.as_ptr();
        check(unsafe {
            sys::ntg_connect(
                _handle,
                chat_id,
                params_ptr,
                is_presentation,
            )
        })?;
        Ok(())
        })
        .await
        .expect("ntgcalls blocking task panicked")
    }

    pub async fn add_incoming_video(
        &self,
        chat_id: i64,
        user_id: i64,
        endpoint: &str,
        ssrc_groups: &[SsrcGroup],
    ) -> Result<u32> {
        let endpoint = endpoint.to_owned();
        let ssrc_groups = ssrc_groups.to_vec();
        let _sh = SendHandle(self.handle);
        tokio::task::spawn_blocking(move || {
            let _sh = _sh;
            let _handle = _sh.0;
        let mut conv = util::Conv::default();
        let endpoint_c = CString::new(endpoint).unwrap();
        let endpoint_ptr = endpoint_c.as_ptr();
        let ssrc_groups_c: Vec<_> = ssrc_groups.iter().map(|e| e.to_c(&mut conv)).collect();
        let mut out = Default::default();
        check(unsafe {
            sys::ntg_add_incoming_video(
                _handle,
                chat_id,
                user_id,
                endpoint_ptr,
                ssrc_groups_c.as_ptr(), ssrc_groups_c.len(),
                &mut out,
            )
        })?;
        Ok(out)
        })
        .await
        .expect("ntgcalls blocking task panicked")
    }

    pub async fn remove_incoming_video(
        &self,
        chat_id: i64,
        endpoint: &str,
    ) -> Result<bool> {
        let endpoint = endpoint.to_owned();
        let _sh = SendHandle(self.handle);
        tokio::task::spawn_blocking(move || {
            let _sh = _sh;
            let _handle = _sh.0;
        let endpoint_c = CString::new(endpoint).unwrap();
        let endpoint_ptr = endpoint_c.as_ptr();
        let mut out = Default::default();
        check(unsafe {
            sys::ntg_remove_incoming_video(
                _handle,
                chat_id,
                endpoint_ptr,
                &mut out,
            )
        })?;
        Ok(out)
        })
        .await
        .expect("ntgcalls blocking task panicked")
    }

    pub async fn set_stream_sources(
        &self,
        chat_id: i64,
        mode: StreamMode,
        media: &MediaDescription,
    ) -> Result<()> {
        let media = media.clone();
        let _sh = SendHandle(self.handle);
        tokio::task::spawn_blocking(move || {
            let _sh = _sh;
            let _handle = _sh.0;
        let mut conv = util::Conv::default();
        let media_c = media.to_c(&mut conv);
        check(unsafe {
            sys::ntg_set_stream_sources(
                _handle,
                chat_id,
                mode.into(),
                media_c,
            )
        })?;
        Ok(())
        })
        .await
        .expect("ntgcalls blocking task panicked")
    }

    pub async fn pause(
        &self,
        chat_id: i64,
    ) -> Result<bool> {
        let _sh = SendHandle(self.handle);
        tokio::task::spawn_blocking(move || {
            let _sh = _sh;
            let _handle = _sh.0;
        let mut out = Default::default();
        check(unsafe {
            sys::ntg_pause(
                _handle,
                chat_id,
                &mut out,
            )
        })?;
        Ok(out)
        })
        .await
        .expect("ntgcalls blocking task panicked")
    }

    pub async fn resume(
        &self,
        chat_id: i64,
    ) -> Result<bool> {
        let _sh = SendHandle(self.handle);
        tokio::task::spawn_blocking(move || {
            let _sh = _sh;
            let _handle = _sh.0;
        let mut out = Default::default();
        check(unsafe {
            sys::ntg_resume(
                _handle,
                chat_id,
                &mut out,
            )
        })?;
        Ok(out)
        })
        .await
        .expect("ntgcalls blocking task panicked")
    }

    pub async fn mute(
        &self,
        chat_id: i64,
    ) -> Result<bool> {
        let _sh = SendHandle(self.handle);
        tokio::task::spawn_blocking(move || {
            let _sh = _sh;
            let _handle = _sh.0;
        let mut out = Default::default();
        check(unsafe {
            sys::ntg_mute(
                _handle,
                chat_id,
                &mut out,
            )
        })?;
        Ok(out)
        })
        .await
        .expect("ntgcalls blocking task panicked")
    }

    pub async fn unmute(
        &self,
        chat_id: i64,
    ) -> Result<bool> {
        let _sh = SendHandle(self.handle);
        tokio::task::spawn_blocking(move || {
            let _sh = _sh;
            let _handle = _sh.0;
        let mut out = Default::default();
        check(unsafe {
            sys::ntg_unmute(
                _handle,
                chat_id,
                &mut out,
            )
        })?;
        Ok(out)
        })
        .await
        .expect("ntgcalls blocking task panicked")
    }

    pub async fn stop(
        &self,
        chat_id: i64,
    ) -> Result<()> {
        let _sh = SendHandle(self.handle);
        tokio::task::spawn_blocking(move || {
            let _sh = _sh;
            let _handle = _sh.0;
        check(unsafe {
            sys::ntg_stop(
                _handle,
                chat_id,
            )
        })?;
        Ok(())
        })
        .await
        .expect("ntgcalls blocking task panicked")
    }

    pub async fn stop_presentation(
        &self,
        chat_id: i64,
    ) -> Result<()> {
        let _sh = SendHandle(self.handle);
        tokio::task::spawn_blocking(move || {
            let _sh = _sh;
            let _handle = _sh.0;
        check(unsafe {
            sys::ntg_stop_presentation(
                _handle,
                chat_id,
            )
        })?;
        Ok(())
        })
        .await
        .expect("ntgcalls blocking task panicked")
    }

    pub async fn get_emojis_fingerprint(
        &self,
        chat_id: i64,
    ) -> Result<String> {
        let _sh = SendHandle(self.handle);
        tokio::task::spawn_blocking(move || {
            let _sh = _sh;
            let _handle = _sh.0;
        let mut out = std::ptr::null_mut();
        check(unsafe {
            sys::ntg_get_emojis_fingerprint(
                _handle,
                chat_id,
                &mut out,
            )
        })?;
        Ok(util::take_string(out))
        })
        .await
        .expect("ntgcalls blocking task panicked")
    }

    pub async fn time(
        &self,
        chat_id: i64,
        mode: StreamMode,
    ) -> Result<u64> {
        let _sh = SendHandle(self.handle);
        tokio::task::spawn_blocking(move || {
            let _sh = _sh;
            let _handle = _sh.0;
        let mut out = Default::default();
        check(unsafe {
            sys::ntg_time(
                _handle,
                chat_id,
                mode.into(),
                &mut out,
            )
        })?;
        Ok(out)
        })
        .await
        .expect("ntgcalls blocking task panicked")
    }

    pub async fn get_state(
        &self,
        chat_id: i64,
    ) -> Result<MediaState> {
        let _sh = SendHandle(self.handle);
        tokio::task::spawn_blocking(move || {
            let _sh = _sh;
            let _handle = _sh.0;
        let mut out = unsafe { std::mem::zeroed() };
        check(unsafe {
            sys::ntg_get_state(
                _handle,
                chat_id,
                &mut out,
            )
        })?;
        let value = unsafe { MediaState::from_c(&out) };
        unsafe { sys::ntg_media_state_free(&mut out) };
        Ok(value)
        })
        .await
        .expect("ntgcalls blocking task panicked")
    }

    pub async fn get_call_type(
        &self,
        chat_id: i64,
    ) -> Result<CallType> {
        let _sh = SendHandle(self.handle);
        tokio::task::spawn_blocking(move || {
            let _sh = _sh;
            let _handle = _sh.0;
        let mut out = unsafe { std::mem::zeroed() };
        check(unsafe {
            sys::ntg_get_call_type(
                _handle,
                chat_id,
                &mut out,
            )
        })?;
        Ok(out.into())
        })
        .await
        .expect("ntgcalls blocking task panicked")
    }

    pub async fn get_connection_mode(
        &self,
        chat_id: i64,
    ) -> Result<ConnectionMode> {
        let _sh = SendHandle(self.handle);
        tokio::task::spawn_blocking(move || {
            let _sh = _sh;
            let _handle = _sh.0;
        let mut out = unsafe { std::mem::zeroed() };
        check(unsafe {
            sys::ntg_get_connection_mode(
                _handle,
                chat_id,
                &mut out,
            )
        })?;
        Ok(out.into())
        })
        .await
        .expect("ntgcalls blocking task panicked")
    }

    pub async fn cpu_usage(
        &self,
    ) -> Result<f64> {
        let _sh = SendHandle(self.handle);
        tokio::task::spawn_blocking(move || {
            let _sh = _sh;
            let _handle = _sh.0;
        let mut out = Default::default();
        check(unsafe {
            sys::ntg_cpu_usage(
                _handle,
                &mut out,
            )
        })?;
        Ok(out)
        })
        .await
        .expect("ntgcalls blocking task panicked")
    }

    pub fn ping(
    ) -> Result<String> {
        let mut out = std::ptr::null_mut();
        check(unsafe {
            sys::ntg_ping(
                &mut out,
            )
        })?;
        Ok(util::take_string(out))
    }

    pub fn get_media_devices(
    ) -> Result<MediaDevices> {
        let mut out = unsafe { std::mem::zeroed() };
        check(unsafe {
            sys::ntg_get_media_devices(
                &mut out,
            )
        })?;
        let value = unsafe { MediaDevices::from_c(&out) };
        unsafe { sys::ntg_media_devices_free(&mut out) };
        Ok(value)
    }

    pub fn get_protocol(
    ) -> Result<Protocol> {
        let mut out = unsafe { std::mem::zeroed() };
        check(unsafe {
            sys::ntg_get_protocol(
                &mut out,
            )
        })?;
        let value = unsafe { Protocol::from_c(&out) };
        unsafe { sys::ntg_protocol_free(&mut out) };
        Ok(value)
    }

    pub fn enable_glib_loop(
        enable: bool,
    ) -> Result<()> {
        check(unsafe {
            sys::ntg_enable_glib_loop(
                enable,
            )
        })?;
        Ok(())
    }

    pub async fn send_broadcast_timestamp(
        &self,
        chat_id: i64,
        timestamp: i64,
    ) -> Result<()> {
        let _sh = SendHandle(self.handle);
        tokio::task::spawn_blocking(move || {
            let _sh = _sh;
            let _handle = _sh.0;
        check(unsafe {
            sys::ntg_send_broadcast_timestamp(
                _handle,
                chat_id,
                timestamp,
            )
        })?;
        Ok(())
        })
        .await
        .expect("ntgcalls blocking task panicked")
    }

    pub async fn send_broadcast_part(
        &self,
        chat_id: i64,
        segment_id: i64,
        part_id: i32,
        status: MediaSegmentPartStatus,
        quality_update: bool,
        data: Option<&[u8]>,
    ) -> Result<()> {
        let data = data.map(|b| b.to_vec());
        let _sh = SendHandle(self.handle);
        tokio::task::spawn_blocking(move || {
            let _sh = _sh;
            let _handle = _sh.0;
        let data_ptr = data.as_ref().map_or(std::ptr::null(), |b| b.as_ptr());
        let data_len = data.as_ref().map_or(0, |b| b.len());
        check(unsafe {
            sys::ntg_send_broadcast_part(
                _handle,
                chat_id,
                segment_id,
                part_id,
                status.into(),
                quality_update,
                data_ptr, data_len,
            )
        })?;
        Ok(())
        })
        .await
        .expect("ntgcalls blocking task panicked")
    }

    pub async fn send_signaling_data(
        &self,
        chat_id: i64,
        msg_key: &[u8],
    ) -> Result<()> {
        let msg_key = msg_key.to_vec();
        let _sh = SendHandle(self.handle);
        tokio::task::spawn_blocking(move || {
            let _sh = _sh;
            let _handle = _sh.0;
        let msg_key_ptr = msg_key.as_ptr();
        let msg_key_len = msg_key.len();
        check(unsafe {
            sys::ntg_send_signaling_data(
                _handle,
                chat_id,
                msg_key_ptr, msg_key_len,
            )
        })?;
        Ok(())
        })
        .await
        .expect("ntgcalls blocking task panicked")
    }

    pub async fn send_external_frame(
        &self,
        chat_id: i64,
        device: StreamDevice,
        data: &[u8],
        frame_data: &FrameData,
    ) -> Result<()> {
        let data = data.to_vec();
        let frame_data = frame_data.clone();
        let _sh = SendHandle(self.handle);
        tokio::task::spawn_blocking(move || {
            let _sh = _sh;
            let _handle = _sh.0;
        let mut conv = util::Conv::default();
        let data_ptr = data.as_ptr();
        let data_len = data.len();
        let frame_data_c = frame_data.to_c(&mut conv);
        check(unsafe {
            sys::ntg_send_external_frame(
                _handle,
                chat_id,
                device.into(),
                data_ptr, data_len,
                frame_data_c,
            )
        })?;
        Ok(())
        })
        .await
        .expect("ntgcalls blocking task panicked")
    }

    pub async fn update_audio_ssrc_mappings(
        &self,
        chat_id: i64,
        ssrc_groups: &[SsrcMapping],
    ) -> Result<()> {
        let ssrc_groups = ssrc_groups.to_vec();
        let _sh = SendHandle(self.handle);
        tokio::task::spawn_blocking(move || {
            let _sh = _sh;
            let _handle = _sh.0;
        let mut conv = util::Conv::default();
        let ssrc_groups_c: Vec<_> = ssrc_groups.iter().map(|e| e.to_c(&mut conv)).collect();
        check(unsafe {
            sys::ntg_update_audio_ssrc_mappings(
                _handle,
                chat_id,
                ssrc_groups_c.as_ptr(), ssrc_groups_c.len(),
            )
        })?;
        Ok(())
        })
        .await
        .expect("ntgcalls blocking task panicked")
    }

    pub async fn apply_blocks(
        &self,
        chat_id: i64,
        subchain: i32,
        next_offset: i32,
        blocks: &[Vec<u8>],
        from_short_poll: bool,
    ) -> Result<()> {
        let blocks = blocks.to_vec();
        let _sh = SendHandle(self.handle);
        tokio::task::spawn_blocking(move || {
            let _sh = _sh;
            let _handle = _sh.0;
        let mut conv = util::Conv::default();
        let blocks_c: Vec<_> = blocks.iter().map(|e| e.to_c(&mut conv)).collect();
        check(unsafe {
            sys::ntg_apply_blocks(
                _handle,
                chat_id,
                subchain,
                next_offset,
                blocks_c.as_ptr(), blocks_c.len(),
                from_short_poll,
            )
        })?;
        Ok(())
        })
        .await
        .expect("ntgcalls blocking task panicked")
    }

    pub async fn finish_subchain_request(
        &self,
        chat_id: i64,
        subchain: i32,
    ) -> Result<()> {
        let _sh = SendHandle(self.handle);
        tokio::task::spawn_blocking(move || {
            let _sh = _sh;
            let _handle = _sh.0;
        check(unsafe {
            sys::ntg_finish_subchain_request(
                _handle,
                chat_id,
                subchain,
            )
        })?;
        Ok(())
        })
        .await
        .expect("ntgcalls blocking task panicked")
    }

    pub async fn calls(
        &self,
    ) -> Result<std::collections::HashMap<i64, CallInfo>> {
        let _sh = SendHandle(self.handle);
        tokio::task::spawn_blocking(move || {
            let _sh = _sh;
            let _handle = _sh.0;
        let mut out = std::ptr::null_mut();
        let mut out_len = 0;
        check(unsafe {
            sys::ntg_calls(
                _handle,
                &mut out, &mut out_len,
            )
        })?;
        let value = if out.is_null() {
            std::collections::HashMap::new()
        } else {
            let map = unsafe {
                std::slice::from_raw_parts(out, out_len)
                    .iter()
                    .map(|e| (e.key, FromC::from_c(&e.value)))
                    .collect()
            };
            unsafe { sys::ntg_call_info_entry_free(out, out_len) };
            map
        };
        Ok(value)
        })
        .await
        .expect("ntgcalls blocking task panicked")
    }

    pub fn on_upgrade(
        &mut self,
        callback: impl Fn(
            i64,
            MediaState
        ) + Send + Sync + 'static,
    ) {
        let boxed: Box<Box<UpgradeCallbackFn>> = Box::new(Box::new(callback));
        let user_data = (&*boxed as *const Box<UpgradeCallbackFn>) as *mut c_void;
        unsafe {
            sys::ntg_on_upgrade_callback(self.handle, Some(upgrade_callback_trampoline), user_data);
        }
        self.on_upgrade_cb = Some(boxed);
    }

    pub fn on_stream_end(
        &mut self,
        callback: impl Fn(
            i64,
            StreamType,
            StreamDevice
        ) + Send + Sync + 'static,
    ) {
        let boxed: Box<Box<StreamEndCallbackFn>> = Box::new(Box::new(callback));
        let user_data = (&*boxed as *const Box<StreamEndCallbackFn>) as *mut c_void;
        unsafe {
            sys::ntg_on_stream_end_callback(self.handle, Some(stream_end_callback_trampoline), user_data);
        }
        self.on_stream_end_cb = Some(boxed);
    }

    pub fn on_connection_change(
        &mut self,
        callback: impl Fn(
            i64,
            ConnectionInfo
        ) + Send + Sync + 'static,
    ) {
        let boxed: Box<Box<ConnectionChangeCallbackFn>> = Box::new(Box::new(callback));
        let user_data = (&*boxed as *const Box<ConnectionChangeCallbackFn>) as *mut c_void;
        unsafe {
            sys::ntg_on_connection_change_callback(self.handle, Some(connection_change_callback_trampoline), user_data);
        }
        self.on_connection_change_cb = Some(boxed);
    }

    pub fn on_frames(
        &mut self,
        callback: impl Fn(
            i64,
            StreamMode,
            StreamDevice,
            Vec<Frame>
        ) + Send + Sync + 'static,
    ) {
        let boxed: Box<Box<FramesCallbackFn>> = Box::new(Box::new(callback));
        let user_data = (&*boxed as *const Box<FramesCallbackFn>) as *mut c_void;
        unsafe {
            sys::ntg_on_frames_callback(self.handle, Some(frames_callback_trampoline), user_data);
        }
        self.on_frames_cb = Some(boxed);
    }

    pub fn on_signaling_data(
        &mut self,
        callback: impl Fn(
            i64,
            Vec<u8>
        ) + Send + Sync + 'static,
    ) {
        let boxed: Box<Box<SignalingDataCallbackFn>> = Box::new(Box::new(callback));
        let user_data = (&*boxed as *const Box<SignalingDataCallbackFn>) as *mut c_void;
        unsafe {
            sys::ntg_on_signaling_data_callback(self.handle, Some(signaling_data_callback_trampoline), user_data);
        }
        self.on_signaling_data_cb = Some(boxed);
    }

    pub fn on_remote_source_change(
        &mut self,
        callback: impl Fn(
            i64,
            RemoteSource
        ) + Send + Sync + 'static,
    ) {
        let boxed: Box<Box<RemoteSourceChangeCallbackFn>> = Box::new(Box::new(callback));
        let user_data = (&*boxed as *const Box<RemoteSourceChangeCallbackFn>) as *mut c_void;
        unsafe {
            sys::ntg_on_remote_source_change_callback(self.handle, Some(remote_source_change_callback_trampoline), user_data);
        }
        self.on_remote_source_change_cb = Some(boxed);
    }

    pub fn on_request_broadcast_part(
        &mut self,
        callback: impl Fn(
            i64,
            SegmentPartRequest
        ) + Send + Sync + 'static,
    ) {
        let boxed: Box<Box<RequestBroadcastPartCallbackFn>> = Box::new(Box::new(callback));
        let user_data = (&*boxed as *const Box<RequestBroadcastPartCallbackFn>) as *mut c_void;
        unsafe {
            sys::ntg_on_request_broadcast_part_callback(self.handle, Some(request_broadcast_part_callback_trampoline), user_data);
        }
        self.on_request_broadcast_part_cb = Some(boxed);
    }

    pub fn on_request_broadcast_timestamp(
        &mut self,
        callback: impl Fn(
            i64
        ) + Send + Sync + 'static,
    ) {
        let boxed: Box<Box<RequestBroadcastTimestampCallbackFn>> = Box::new(Box::new(callback));
        let user_data = (&*boxed as *const Box<RequestBroadcastTimestampCallbackFn>) as *mut c_void;
        unsafe {
            sys::ntg_on_request_broadcast_timestamp_callback(self.handle, Some(request_broadcast_timestamp_callback_trampoline), user_data);
        }
        self.on_request_broadcast_timestamp_cb = Some(boxed);
    }

    pub fn on_request_participants(
        &mut self,
        callback: impl Fn(
            i64
        ) + Send + Sync + 'static,
    ) {
        let boxed: Box<Box<RequestParticipantsCallbackFn>> = Box::new(Box::new(callback));
        let user_data = (&*boxed as *const Box<RequestParticipantsCallbackFn>) as *mut c_void;
        unsafe {
            sys::ntg_on_request_participants_callback(self.handle, Some(request_participants_callback_trampoline), user_data);
        }
        self.on_request_participants_cb = Some(boxed);
    }

    pub fn on_outbound_block(
        &mut self,
        callback: impl Fn(
            i64,
            Vec<u8>
        ) + Send + Sync + 'static,
    ) {
        let boxed: Box<Box<OutboundBlockCallbackFn>> = Box::new(Box::new(callback));
        let user_data = (&*boxed as *const Box<OutboundBlockCallbackFn>) as *mut c_void;
        unsafe {
            sys::ntg_on_outbound_block_callback(self.handle, Some(outbound_block_callback_trampoline), user_data);
        }
        self.on_outbound_block_cb = Some(boxed);
    }

    pub fn on_subchain_request(
        &mut self,
        callback: impl Fn(
            i64,
            SubchainRequest
        ) + Send + Sync + 'static,
    ) {
        let boxed: Box<Box<SubchainRequestCallbackFn>> = Box::new(Box::new(callback));
        let user_data = (&*boxed as *const Box<SubchainRequestCallbackFn>) as *mut c_void;
        unsafe {
            sys::ntg_on_subchain_request_callback(self.handle, Some(subchain_request_callback_trampoline), user_data);
        }
        self.on_subchain_request_cb = Some(boxed);
    }

    pub fn on_update_emojis(
        &mut self,
        callback: impl Fn(
            i64,
            String
        ) + Send + Sync + 'static,
    ) {
        let boxed: Box<Box<UpdateEmojisCallbackFn>> = Box::new(Box::new(callback));
        let user_data = (&*boxed as *const Box<UpdateEmojisCallbackFn>) as *mut c_void;
        unsafe {
            sys::ntg_on_update_emojis_callback(self.handle, Some(update_emojis_callback_trampoline), user_data);
        }
        self.on_update_emojis_cb = Some(boxed);
    }

}

impl Drop for NTgCalls {
    fn drop(&mut self) {
        unsafe { sys::ntg_instance_destroy(self.handle) };
    }
}

impl Default for NTgCalls {
    fn default() -> Self {
        Self::new()
    }
}

unsafe extern "C" fn upgrade_callback_trampoline(
    _handle: *mut sys::ntg_instance,
    chat_id: i64,
    state: sys::ntg_media_state,
    user_data: *mut c_void,
) {
    let callback = &*(user_data as *const Box<UpgradeCallbackFn>);
    let state = MediaState::from_c(&state);
    callback(
        chat_id,
        state
    );
}

unsafe extern "C" fn stream_end_callback_trampoline(
    _handle: *mut sys::ntg_instance,
    chat_id: i64,
    r#type: sys::ntg_stream_type,
    device: sys::ntg_stream_device,
    user_data: *mut c_void,
) {
    let callback = &*(user_data as *const Box<StreamEndCallbackFn>);
    let r#type: StreamType = r#type.into();
    let device: StreamDevice = device.into();
    callback(
        chat_id,
        r#type,
        device
    );
}

unsafe extern "C" fn connection_change_callback_trampoline(
    _handle: *mut sys::ntg_instance,
    chat_id: i64,
    state: sys::ntg_connection_info,
    user_data: *mut c_void,
) {
    let callback = &*(user_data as *const Box<ConnectionChangeCallbackFn>);
    let state = ConnectionInfo::from_c(&state);
    callback(
        chat_id,
        state
    );
}

unsafe extern "C" fn frames_callback_trampoline(
    _handle: *mut sys::ntg_instance,
    chat_id: i64,
    mode: sys::ntg_stream_mode,
    device: sys::ntg_stream_device,
    frames: *const sys::ntg_frame, frames_len: usize,
    user_data: *mut c_void,
) {
    let callback = &*(user_data as *const Box<FramesCallbackFn>);
    let mode: StreamMode = mode.into();
    let device: StreamDevice = device.into();
    let frames = util::vec_from_c(frames as *mut _, frames_len);
    callback(
        chat_id,
        mode,
        device,
        frames
    );
}

unsafe extern "C" fn signaling_data_callback_trampoline(
    _handle: *mut sys::ntg_instance,
    chat_id: i64,
    data: *const u8, data_len: usize,
    user_data: *mut c_void,
) {
    let callback = &*(user_data as *const Box<SignalingDataCallbackFn>);
    let data = util::bytes(data as *mut u8, data_len);
    callback(
        chat_id,
        data
    );
}

unsafe extern "C" fn remote_source_change_callback_trampoline(
    _handle: *mut sys::ntg_instance,
    chat_id: i64,
    state: sys::ntg_remote_source,
    user_data: *mut c_void,
) {
    let callback = &*(user_data as *const Box<RemoteSourceChangeCallbackFn>);
    let state = RemoteSource::from_c(&state);
    callback(
        chat_id,
        state
    );
}

unsafe extern "C" fn request_broadcast_part_callback_trampoline(
    _handle: *mut sys::ntg_instance,
    chat_id: i64,
    request: sys::ntg_segment_part_request,
    user_data: *mut c_void,
) {
    let callback = &*(user_data as *const Box<RequestBroadcastPartCallbackFn>);
    let request = SegmentPartRequest::from_c(&request);
    callback(
        chat_id,
        request
    );
}

unsafe extern "C" fn request_broadcast_timestamp_callback_trampoline(
    _handle: *mut sys::ntg_instance,
    chat_id: i64,
    user_data: *mut c_void,
) {
    let callback = &*(user_data as *const Box<RequestBroadcastTimestampCallbackFn>);
    callback(
        chat_id
    );
}

unsafe extern "C" fn request_participants_callback_trampoline(
    _handle: *mut sys::ntg_instance,
    chat_id: i64,
    user_data: *mut c_void,
) {
    let callback = &*(user_data as *const Box<RequestParticipantsCallbackFn>);
    callback(
        chat_id
    );
}

unsafe extern "C" fn outbound_block_callback_trampoline(
    _handle: *mut sys::ntg_instance,
    chat_id: i64,
    block: *const u8, block_len: usize,
    user_data: *mut c_void,
) {
    let callback = &*(user_data as *const Box<OutboundBlockCallbackFn>);
    let block = util::bytes(block as *mut u8, block_len);
    callback(
        chat_id,
        block
    );
}

unsafe extern "C" fn subchain_request_callback_trampoline(
    _handle: *mut sys::ntg_instance,
    chat_id: i64,
    subchain_request: sys::ntg_subchain_request,
    user_data: *mut c_void,
) {
    let callback = &*(user_data as *const Box<SubchainRequestCallbackFn>);
    let subchain_request = SubchainRequest::from_c(&subchain_request);
    callback(
        chat_id,
        subchain_request
    );
}

unsafe extern "C" fn update_emojis_callback_trampoline(
    _handle: *mut sys::ntg_instance,
    chat_id: i64,
    emojis: *const c_char,
    user_data: *mut c_void,
) {
    let callback = &*(user_data as *const Box<UpdateEmojisCallbackFn>);
    let emojis = util::cstr(emojis);
    callback(
        chat_id,
        emojis
    );
}

