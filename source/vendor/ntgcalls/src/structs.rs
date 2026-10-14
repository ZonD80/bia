// @generated from schema/ntgcalls.ntl -- DO NOT EDIT
#![allow(dead_code)]
use crate::enums::*;
use crate::util::{self, Conv, FromC, ToC};
use ntgcalls_sys as sys;

#[derive(Debug, Clone)]
pub struct ConnectionInfo {
    pub state: ConnectionState,
    pub kind: ConnectionKind,
}

impl FromC for ConnectionInfo {
    type C = sys::ntg_connection_info;
    unsafe fn from_c(c: &Self::C) -> Self {
        Self {
            state: FromC::from_c(&c.state),
            kind: FromC::from_c(&c.kind),
        }
    }
}

impl ToC for ConnectionInfo {
    type C = sys::ntg_connection_info;
    fn to_c(&self, conv: &mut Conv) -> Self::C {
        let mut out: Self::C = unsafe { std::mem::zeroed() };
        out.state = self.state.to_c(conv);
        out.kind = self.kind.to_c(conv);
        out
    }
}

#[derive(Debug, Clone)]
pub struct RemoteSource {
    pub ssrc: u32,
    pub state: StreamStatus,
    pub device: StreamDevice,
}

impl FromC for RemoteSource {
    type C = sys::ntg_remote_source;
    unsafe fn from_c(c: &Self::C) -> Self {
        Self {
            ssrc: FromC::from_c(&c.ssrc),
            state: FromC::from_c(&c.state),
            device: FromC::from_c(&c.device),
        }
    }
}

impl ToC for RemoteSource {
    type C = sys::ntg_remote_source;
    fn to_c(&self, conv: &mut Conv) -> Self::C {
        let mut out: Self::C = unsafe { std::mem::zeroed() };
        out.ssrc = self.ssrc.to_c(conv);
        out.state = self.state.to_c(conv);
        out.device = self.device.to_c(conv);
        out
    }
}

#[derive(Debug, Clone)]
pub struct SubchainRequest {
    pub subchain: i32,
    pub height: i32,
    pub limit: i32,
}

impl FromC for SubchainRequest {
    type C = sys::ntg_subchain_request;
    unsafe fn from_c(c: &Self::C) -> Self {
        Self {
            subchain: FromC::from_c(&c.subchain),
            height: FromC::from_c(&c.height),
            limit: FromC::from_c(&c.limit),
        }
    }
}

impl ToC for SubchainRequest {
    type C = sys::ntg_subchain_request;
    fn to_c(&self, conv: &mut Conv) -> Self::C {
        let mut out: Self::C = unsafe { std::mem::zeroed() };
        out.subchain = self.subchain.to_c(conv);
        out.height = self.height.to_c(conv);
        out.limit = self.limit.to_c(conv);
        out
    }
}

#[derive(Debug, Clone)]
pub struct AudioDescription {
    pub media_source: MediaSource,
    pub sample_rate: u32,
    pub channel_count: u8,
    pub input: String,
    pub keep_open: bool,
}

impl FromC for AudioDescription {
    type C = sys::ntg_audio_description;
    unsafe fn from_c(c: &Self::C) -> Self {
        Self {
            media_source: FromC::from_c(&c.media_source),
            sample_rate: FromC::from_c(&c.sample_rate),
            channel_count: FromC::from_c(&c.channel_count),
            input: util::cstr(c.input),
            keep_open: FromC::from_c(&c.keep_open),
        }
    }
}

impl ToC for AudioDescription {
    type C = sys::ntg_audio_description;
    fn to_c(&self, conv: &mut Conv) -> Self::C {
        let mut out: Self::C = unsafe { std::mem::zeroed() };
        out.media_source = self.media_source.to_c(conv);
        out.sample_rate = self.sample_rate.to_c(conv);
        out.channel_count = self.channel_count.to_c(conv);
        out.input = conv.cstr(&self.input);
        out.keep_open = self.keep_open.to_c(conv);
        out
    }
}

#[derive(Debug, Clone)]
pub struct CallInfo {
    pub playback: StreamStatus,
    pub capture: StreamStatus,
}

impl FromC for CallInfo {
    type C = sys::ntg_call_info;
    unsafe fn from_c(c: &Self::C) -> Self {
        Self {
            playback: FromC::from_c(&c.playback),
            capture: FromC::from_c(&c.capture),
        }
    }
}

impl ToC for CallInfo {
    type C = sys::ntg_call_info;
    fn to_c(&self, conv: &mut Conv) -> Self::C {
        let mut out: Self::C = unsafe { std::mem::zeroed() };
        out.playback = self.playback.to_c(conv);
        out.capture = self.capture.to_c(conv);
        out
    }
}

#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub name: String,
    pub metadata: String,
}

impl FromC for DeviceInfo {
    type C = sys::ntg_device_info;
    unsafe fn from_c(c: &Self::C) -> Self {
        Self {
            name: util::cstr(c.name),
            metadata: util::cstr(c.metadata),
        }
    }
}

impl ToC for DeviceInfo {
    type C = sys::ntg_device_info;
    fn to_c(&self, conv: &mut Conv) -> Self::C {
        let mut out: Self::C = unsafe { std::mem::zeroed() };
        out.name = conv.cstr(&self.name);
        out.metadata = conv.cstr(&self.metadata);
        out
    }
}

#[derive(Debug, Clone)]
pub struct MediaDevices {
    pub microphone: Vec<DeviceInfo>,
    pub speaker: Vec<DeviceInfo>,
    pub camera: Vec<DeviceInfo>,
    pub screen: Vec<DeviceInfo>,
}

impl FromC for MediaDevices {
    type C = sys::ntg_media_devices;
    unsafe fn from_c(c: &Self::C) -> Self {
        Self {
            microphone: util::vec_from_c(c.microphone, c.microphone_len),
            speaker: util::vec_from_c(c.speaker, c.speaker_len),
            camera: util::vec_from_c(c.camera, c.camera_len),
            screen: util::vec_from_c(c.screen, c.screen_len),
        }
    }
}

impl ToC for MediaDevices {
    type C = sys::ntg_media_devices;
    fn to_c(&self, conv: &mut Conv) -> Self::C {
        let mut out: Self::C = unsafe { std::mem::zeroed() };
        let microphone_tmp: Vec<_> =
            self.microphone.iter().map(|e| e.to_c(conv)).collect();
        let (microphone_ptr, microphone_len) = conv.vec(microphone_tmp);
        out.microphone = microphone_ptr;
        out.microphone_len = microphone_len;
        let speaker_tmp: Vec<_> =
            self.speaker.iter().map(|e| e.to_c(conv)).collect();
        let (speaker_ptr, speaker_len) = conv.vec(speaker_tmp);
        out.speaker = speaker_ptr;
        out.speaker_len = speaker_len;
        let camera_tmp: Vec<_> =
            self.camera.iter().map(|e| e.to_c(conv)).collect();
        let (camera_ptr, camera_len) = conv.vec(camera_tmp);
        out.camera = camera_ptr;
        out.camera_len = camera_len;
        let screen_tmp: Vec<_> =
            self.screen.iter().map(|e| e.to_c(conv)).collect();
        let (screen_ptr, screen_len) = conv.vec(screen_tmp);
        out.screen = screen_ptr;
        out.screen_len = screen_len;
        out
    }
}

#[derive(Debug, Clone)]
pub struct MediaState {
    pub muted: bool,
    pub video_paused: bool,
    pub video_stopped: bool,
    pub presentation_paused: bool,
    pub presentation_stopped: bool,
}

impl FromC for MediaState {
    type C = sys::ntg_media_state;
    unsafe fn from_c(c: &Self::C) -> Self {
        Self {
            muted: FromC::from_c(&c.muted),
            video_paused: FromC::from_c(&c.video_paused),
            video_stopped: FromC::from_c(&c.video_stopped),
            presentation_paused: FromC::from_c(&c.presentation_paused),
            presentation_stopped: FromC::from_c(&c.presentation_stopped),
        }
    }
}

impl ToC for MediaState {
    type C = sys::ntg_media_state;
    fn to_c(&self, conv: &mut Conv) -> Self::C {
        let mut out: Self::C = unsafe { std::mem::zeroed() };
        out.muted = self.muted.to_c(conv);
        out.video_paused = self.video_paused.to_c(conv);
        out.video_stopped = self.video_stopped.to_c(conv);
        out.presentation_paused = self.presentation_paused.to_c(conv);
        out.presentation_stopped = self.presentation_stopped.to_c(conv);
        out
    }
}

#[derive(Debug, Clone)]
pub struct VideoDescription {
    pub media_source: MediaSource,
    pub width: i16,
    pub height: i16,
    pub fps: u8,
    pub input: String,
    pub keep_open: bool,
}

impl FromC for VideoDescription {
    type C = sys::ntg_video_description;
    unsafe fn from_c(c: &Self::C) -> Self {
        Self {
            media_source: FromC::from_c(&c.media_source),
            width: FromC::from_c(&c.width),
            height: FromC::from_c(&c.height),
            fps: FromC::from_c(&c.fps),
            input: util::cstr(c.input),
            keep_open: FromC::from_c(&c.keep_open),
        }
    }
}

impl ToC for VideoDescription {
    type C = sys::ntg_video_description;
    fn to_c(&self, conv: &mut Conv) -> Self::C {
        let mut out: Self::C = unsafe { std::mem::zeroed() };
        out.media_source = self.media_source.to_c(conv);
        out.width = self.width.to_c(conv);
        out.height = self.height.to_c(conv);
        out.fps = self.fps.to_c(conv);
        out.input = conv.cstr(&self.input);
        out.keep_open = self.keep_open.to_c(conv);
        out
    }
}

#[derive(Debug, Clone)]
pub struct MediaDescription {
    pub microphone: Option<AudioDescription>,
    pub speaker: Option<AudioDescription>,
    pub camera: Option<VideoDescription>,
    pub screen: Option<VideoDescription>,
}

impl FromC for MediaDescription {
    type C = sys::ntg_media_description;
    unsafe fn from_c(c: &Self::C) -> Self {
        Self {
            microphone: util::opt_from_c(c.microphone),
            speaker: util::opt_from_c(c.speaker),
            camera: util::opt_from_c(c.camera),
            screen: util::opt_from_c(c.screen),
        }
    }
}

impl ToC for MediaDescription {
    type C = sys::ntg_media_description;
    fn to_c(&self, conv: &mut Conv) -> Self::C {
        let mut out: Self::C = unsafe { std::mem::zeroed() };
        out.microphone = match &self.microphone {
            Some(v) => {
                let c = v.to_c(conv);
                conv.boxed(c)
            }
            None => std::ptr::null_mut(),
        };
        out.speaker = match &self.speaker {
            Some(v) => {
                let c = v.to_c(conv);
                conv.boxed(c)
            }
            None => std::ptr::null_mut(),
        };
        out.camera = match &self.camera {
            Some(v) => {
                let c = v.to_c(conv);
                conv.boxed(c)
            }
            None => std::ptr::null_mut(),
        };
        out.screen = match &self.screen {
            Some(v) => {
                let c = v.to_c(conv);
                conv.boxed(c)
            }
            None => std::ptr::null_mut(),
        };
        out
    }
}

#[derive(Debug, Clone)]
pub struct FrameData {
    pub absolute_capture_timestamp_ms: i64,
    pub rotation: VideoRotation,
    pub width: u16,
    pub height: u16,
}

impl FromC for FrameData {
    type C = sys::ntg_frame_data;
    unsafe fn from_c(c: &Self::C) -> Self {
        Self {
            absolute_capture_timestamp_ms: FromC::from_c(&c.absolute_capture_timestamp_ms),
            rotation: FromC::from_c(&c.rotation),
            width: FromC::from_c(&c.width),
            height: FromC::from_c(&c.height),
        }
    }
}

impl ToC for FrameData {
    type C = sys::ntg_frame_data;
    fn to_c(&self, conv: &mut Conv) -> Self::C {
        let mut out: Self::C = unsafe { std::mem::zeroed() };
        out.absolute_capture_timestamp_ms = self.absolute_capture_timestamp_ms.to_c(conv);
        out.rotation = self.rotation.to_c(conv);
        out.width = self.width.to_c(conv);
        out.height = self.height.to_c(conv);
        out
    }
}

#[derive(Debug, Clone)]
pub struct SsrcGroup {
    pub semantics: String,
    pub ssrcs: Vec<u32>,
}

impl FromC for SsrcGroup {
    type C = sys::ntg_ssrc_group;
    unsafe fn from_c(c: &Self::C) -> Self {
        Self {
            semantics: util::cstr(c.semantics),
            ssrcs: util::vec_from_c(c.ssrcs, c.ssrcs_len),
        }
    }
}

impl ToC for SsrcGroup {
    type C = sys::ntg_ssrc_group;
    fn to_c(&self, conv: &mut Conv) -> Self::C {
        let mut out: Self::C = unsafe { std::mem::zeroed() };
        out.semantics = conv.cstr(&self.semantics);
        let ssrcs_tmp: Vec<_> =
            self.ssrcs.iter().map(|e| e.to_c(conv)).collect();
        let (ssrcs_ptr, ssrcs_len) = conv.vec(ssrcs_tmp);
        out.ssrcs = ssrcs_ptr;
        out.ssrcs_len = ssrcs_len;
        out
    }
}

#[derive(Debug, Clone)]
pub struct Frame {
    pub ssrc: i64,
    pub data: Vec<u8>,
    pub frame_data: FrameData,
}

impl FromC for Frame {
    type C = sys::ntg_frame;
    unsafe fn from_c(c: &Self::C) -> Self {
        Self {
            ssrc: FromC::from_c(&c.ssrc),
            data: util::bytes(c.data, c.data_len),
            frame_data: FromC::from_c(&c.frame_data),
        }
    }
}

impl ToC for Frame {
    type C = sys::ntg_frame;
    fn to_c(&self, conv: &mut Conv) -> Self::C {
        let mut out: Self::C = unsafe { std::mem::zeroed() };
        out.ssrc = self.ssrc.to_c(conv);
        out.data = self.data.as_ptr() as *mut u8;
        out.data_len = self.data.len();
        out.frame_data = self.frame_data.to_c(conv);
        out
    }
}

#[derive(Debug, Clone)]
pub struct SegmentPartRequest {
    pub segment_id: i64,
    pub part_id: i32,
    pub limit: i32,
    pub timestamp: i64,
    pub quality_update: bool,
    pub channel_id: i32,
    pub quality: MediaSegmentQuality,
}

impl FromC for SegmentPartRequest {
    type C = sys::ntg_segment_part_request;
    unsafe fn from_c(c: &Self::C) -> Self {
        Self {
            segment_id: FromC::from_c(&c.segment_id),
            part_id: FromC::from_c(&c.part_id),
            limit: FromC::from_c(&c.limit),
            timestamp: FromC::from_c(&c.timestamp),
            quality_update: FromC::from_c(&c.quality_update),
            channel_id: FromC::from_c(&c.channel_id),
            quality: FromC::from_c(&c.quality),
        }
    }
}

impl ToC for SegmentPartRequest {
    type C = sys::ntg_segment_part_request;
    fn to_c(&self, conv: &mut Conv) -> Self::C {
        let mut out: Self::C = unsafe { std::mem::zeroed() };
        out.segment_id = self.segment_id.to_c(conv);
        out.part_id = self.part_id.to_c(conv);
        out.limit = self.limit.to_c(conv);
        out.timestamp = self.timestamp.to_c(conv);
        out.quality_update = self.quality_update.to_c(conv);
        out.channel_id = self.channel_id.to_c(conv);
        out.quality = self.quality.to_c(conv);
        out
    }
}

#[derive(Debug, Clone)]
pub struct SsrcMapping {
    pub user_id: i64,
    pub ssrc: i32,
}

impl FromC for SsrcMapping {
    type C = sys::ntg_ssrc_mapping;
    unsafe fn from_c(c: &Self::C) -> Self {
        Self {
            user_id: FromC::from_c(&c.user_id),
            ssrc: FromC::from_c(&c.ssrc),
        }
    }
}

impl ToC for SsrcMapping {
    type C = sys::ntg_ssrc_mapping;
    fn to_c(&self, conv: &mut Conv) -> Self::C {
        let mut out: Self::C = unsafe { std::mem::zeroed() };
        out.user_id = self.user_id.to_c(conv);
        out.ssrc = self.ssrc.to_c(conv);
        out
    }
}

#[derive(Debug, Clone)]
pub struct AuthParams {
    pub key_fingerprint: i64,
    pub g_a_or_b: Vec<u8>,
}

impl FromC for AuthParams {
    type C = sys::ntg_auth_params;
    unsafe fn from_c(c: &Self::C) -> Self {
        Self {
            key_fingerprint: FromC::from_c(&c.key_fingerprint),
            g_a_or_b: util::bytes(c.g_a_or_b, c.g_a_or_b_len),
        }
    }
}

impl ToC for AuthParams {
    type C = sys::ntg_auth_params;
    fn to_c(&self, conv: &mut Conv) -> Self::C {
        let mut out: Self::C = unsafe { std::mem::zeroed() };
        out.key_fingerprint = self.key_fingerprint.to_c(conv);
        out.g_a_or_b = self.g_a_or_b.as_ptr() as *mut u8;
        out.g_a_or_b_len = self.g_a_or_b.len();
        out
    }
}

#[derive(Debug, Clone)]
pub struct ConferenceJoinParams {
    pub payload: String,
    pub public_key: Vec<u8>,
    pub block: Vec<u8>,
}

impl FromC for ConferenceJoinParams {
    type C = sys::ntg_conference_join_params;
    unsafe fn from_c(c: &Self::C) -> Self {
        Self {
            payload: util::cstr(c.payload),
            public_key: util::bytes(c.public_key, c.public_key_len),
            block: util::bytes(c.block, c.block_len),
        }
    }
}

impl ToC for ConferenceJoinParams {
    type C = sys::ntg_conference_join_params;
    fn to_c(&self, conv: &mut Conv) -> Self::C {
        let mut out: Self::C = unsafe { std::mem::zeroed() };
        out.payload = conv.cstr(&self.payload);
        out.public_key = self.public_key.as_ptr() as *mut u8;
        out.public_key_len = self.public_key.len();
        out.block = self.block.as_ptr() as *mut u8;
        out.block_len = self.block.len();
        out
    }
}

#[derive(Debug, Clone)]
pub struct DhConfig {
    pub g: i32,
    pub p: Vec<u8>,
    pub random: Vec<u8>,
}

impl FromC for DhConfig {
    type C = sys::ntg_dh_config;
    unsafe fn from_c(c: &Self::C) -> Self {
        Self {
            g: FromC::from_c(&c.g),
            p: util::bytes(c.p, c.p_len),
            random: util::bytes(c.random, c.random_len),
        }
    }
}

impl ToC for DhConfig {
    type C = sys::ntg_dh_config;
    fn to_c(&self, conv: &mut Conv) -> Self::C {
        let mut out: Self::C = unsafe { std::mem::zeroed() };
        out.g = self.g.to_c(conv);
        out.p = self.p.as_ptr() as *mut u8;
        out.p_len = self.p.len();
        out.random = self.random.as_ptr() as *mut u8;
        out.random_len = self.random.len();
        out
    }
}

#[derive(Debug, Clone)]
pub struct Protocol {
    pub min_layer: i32,
    pub max_layer: i32,
    pub udp_p2p: bool,
    pub udp_reflector: bool,
    pub library_versions: Vec<String>,
}

impl FromC for Protocol {
    type C = sys::ntg_protocol;
    unsafe fn from_c(c: &Self::C) -> Self {
        Self {
            min_layer: FromC::from_c(&c.min_layer),
            max_layer: FromC::from_c(&c.max_layer),
            udp_p2p: FromC::from_c(&c.udp_p2p),
            udp_reflector: FromC::from_c(&c.udp_reflector),
            library_versions: util::vec_from_c(c.library_versions, c.library_versions_len),
        }
    }
}

impl ToC for Protocol {
    type C = sys::ntg_protocol;
    fn to_c(&self, conv: &mut Conv) -> Self::C {
        let mut out: Self::C = unsafe { std::mem::zeroed() };
        out.min_layer = self.min_layer.to_c(conv);
        out.max_layer = self.max_layer.to_c(conv);
        out.udp_p2p = self.udp_p2p.to_c(conv);
        out.udp_reflector = self.udp_reflector.to_c(conv);
        let library_versions_tmp: Vec<_> =
            self.library_versions.iter().map(|e| e.to_c(conv)).collect();
        let (library_versions_ptr, library_versions_len) = conv.vec(library_versions_tmp);
        out.library_versions = library_versions_ptr;
        out.library_versions_len = library_versions_len;
        out
    }
}

#[derive(Debug, Clone)]
pub struct RTCServer {
    pub id: u64,
    pub ipv4: String,
    pub ipv6: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub turn: bool,
    pub stun: bool,
    pub tcp: bool,
    pub peer_tag: Vec<u8>,
}

impl FromC for RTCServer {
    type C = sys::ntg_rtc_server;
    unsafe fn from_c(c: &Self::C) -> Self {
        Self {
            id: FromC::from_c(&c.id),
            ipv4: util::cstr(c.ipv4),
            ipv6: util::cstr(c.ipv6),
            port: FromC::from_c(&c.port),
            username: util::cstr(c.username),
            password: util::cstr(c.password),
            turn: FromC::from_c(&c.turn),
            stun: FromC::from_c(&c.stun),
            tcp: FromC::from_c(&c.tcp),
            peer_tag: util::bytes(c.peer_tag, c.peer_tag_len),
        }
    }
}

impl ToC for RTCServer {
    type C = sys::ntg_rtc_server;
    fn to_c(&self, conv: &mut Conv) -> Self::C {
        let mut out: Self::C = unsafe { std::mem::zeroed() };
        out.id = self.id.to_c(conv);
        out.ipv4 = conv.cstr(&self.ipv4);
        out.ipv6 = conv.cstr(&self.ipv6);
        out.port = self.port.to_c(conv);
        out.username = conv.cstr(&self.username);
        out.password = conv.cstr(&self.password);
        out.turn = self.turn.to_c(conv);
        out.stun = self.stun.to_c(conv);
        out.tcp = self.tcp.to_c(conv);
        out.peer_tag = self.peer_tag.as_ptr() as *mut u8;
        out.peer_tag_len = self.peer_tag.len();
        out
    }
}

