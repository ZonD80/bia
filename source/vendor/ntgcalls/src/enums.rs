// @generated from schema/ntgcalls.ntl -- DO NOT EDIT
use ntgcalls_sys as sys;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaSource {
    Unknown,
    File,
    Shell,
    Ffmpeg,
    Device,
    Desktop,
    External,
}

impl From<sys::ntg_media_source> for MediaSource {
    fn from(c: sys::ntg_media_source) -> Self {
        match c {
            sys::ntg_media_source::NTG_MEDIA_SOURCE_UNKNOWN => Self::Unknown,
            sys::ntg_media_source::NTG_MEDIA_SOURCE_FILE => Self::File,
            sys::ntg_media_source::NTG_MEDIA_SOURCE_SHELL => Self::Shell,
            sys::ntg_media_source::NTG_MEDIA_SOURCE_FFMPEG => Self::Ffmpeg,
            sys::ntg_media_source::NTG_MEDIA_SOURCE_DEVICE => Self::Device,
            sys::ntg_media_source::NTG_MEDIA_SOURCE_DESKTOP => Self::Desktop,
            sys::ntg_media_source::NTG_MEDIA_SOURCE_EXTERNAL => Self::External,
        }
    }
}

impl From<MediaSource> for sys::ntg_media_source {
    fn from(v: MediaSource) -> Self {
        match v {
            MediaSource::Unknown => Self::NTG_MEDIA_SOURCE_UNKNOWN,
            MediaSource::File => Self::NTG_MEDIA_SOURCE_FILE,
            MediaSource::Shell => Self::NTG_MEDIA_SOURCE_SHELL,
            MediaSource::Ffmpeg => Self::NTG_MEDIA_SOURCE_FFMPEG,
            MediaSource::Device => Self::NTG_MEDIA_SOURCE_DEVICE,
            MediaSource::Desktop => Self::NTG_MEDIA_SOURCE_DESKTOP,
            MediaSource::External => Self::NTG_MEDIA_SOURCE_EXTERNAL,
        }
    }
}

impl crate::util::FromC for MediaSource {
    type C = sys::ntg_media_source;
    unsafe fn from_c(c: &Self::C) -> Self {
        (*c).into()
    }
}

impl crate::util::ToC for MediaSource {
    type C = sys::ntg_media_source;
    fn to_c(&self, _conv: &mut crate::util::Conv) -> Self::C {
        (*self).into()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamType {
    Audio,
    Video,
}

impl From<sys::ntg_stream_type> for StreamType {
    fn from(c: sys::ntg_stream_type) -> Self {
        match c {
            sys::ntg_stream_type::NTG_STREAM_TYPE_AUDIO => Self::Audio,
            sys::ntg_stream_type::NTG_STREAM_TYPE_VIDEO => Self::Video,
        }
    }
}

impl From<StreamType> for sys::ntg_stream_type {
    fn from(v: StreamType) -> Self {
        match v {
            StreamType::Audio => Self::NTG_STREAM_TYPE_AUDIO,
            StreamType::Video => Self::NTG_STREAM_TYPE_VIDEO,
        }
    }
}

impl crate::util::FromC for StreamType {
    type C = sys::ntg_stream_type;
    unsafe fn from_c(c: &Self::C) -> Self {
        (*c).into()
    }
}

impl crate::util::ToC for StreamType {
    type C = sys::ntg_stream_type;
    fn to_c(&self, _conv: &mut crate::util::Conv) -> Self::C {
        (*self).into()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamStatus {
    Active,
    Paused,
    Idling,
}

impl From<sys::ntg_stream_status> for StreamStatus {
    fn from(c: sys::ntg_stream_status) -> Self {
        match c {
            sys::ntg_stream_status::NTG_STREAM_STATUS_ACTIVE => Self::Active,
            sys::ntg_stream_status::NTG_STREAM_STATUS_PAUSED => Self::Paused,
            sys::ntg_stream_status::NTG_STREAM_STATUS_IDLING => Self::Idling,
        }
    }
}

impl From<StreamStatus> for sys::ntg_stream_status {
    fn from(v: StreamStatus) -> Self {
        match v {
            StreamStatus::Active => Self::NTG_STREAM_STATUS_ACTIVE,
            StreamStatus::Paused => Self::NTG_STREAM_STATUS_PAUSED,
            StreamStatus::Idling => Self::NTG_STREAM_STATUS_IDLING,
        }
    }
}

impl crate::util::FromC for StreamStatus {
    type C = sys::ntg_stream_status;
    unsafe fn from_c(c: &Self::C) -> Self {
        (*c).into()
    }
}

impl crate::util::ToC for StreamStatus {
    type C = sys::ntg_stream_status;
    fn to_c(&self, _conv: &mut crate::util::Conv) -> Self::C {
        (*self).into()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamMode {
    Capture,
    Playback,
}

impl From<sys::ntg_stream_mode> for StreamMode {
    fn from(c: sys::ntg_stream_mode) -> Self {
        match c {
            sys::ntg_stream_mode::NTG_STREAM_MODE_CAPTURE => Self::Capture,
            sys::ntg_stream_mode::NTG_STREAM_MODE_PLAYBACK => Self::Playback,
        }
    }
}

impl From<StreamMode> for sys::ntg_stream_mode {
    fn from(v: StreamMode) -> Self {
        match v {
            StreamMode::Capture => Self::NTG_STREAM_MODE_CAPTURE,
            StreamMode::Playback => Self::NTG_STREAM_MODE_PLAYBACK,
        }
    }
}

impl crate::util::FromC for StreamMode {
    type C = sys::ntg_stream_mode;
    unsafe fn from_c(c: &Self::C) -> Self {
        (*c).into()
    }
}

impl crate::util::ToC for StreamMode {
    type C = sys::ntg_stream_mode;
    fn to_c(&self, _conv: &mut crate::util::Conv) -> Self::C {
        (*self).into()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamDevice {
    Microphone,
    Speaker,
    Camera,
    Screen,
}

impl From<sys::ntg_stream_device> for StreamDevice {
    fn from(c: sys::ntg_stream_device) -> Self {
        match c {
            sys::ntg_stream_device::NTG_STREAM_DEVICE_MICROPHONE => Self::Microphone,
            sys::ntg_stream_device::NTG_STREAM_DEVICE_SPEAKER => Self::Speaker,
            sys::ntg_stream_device::NTG_STREAM_DEVICE_CAMERA => Self::Camera,
            sys::ntg_stream_device::NTG_STREAM_DEVICE_SCREEN => Self::Screen,
        }
    }
}

impl From<StreamDevice> for sys::ntg_stream_device {
    fn from(v: StreamDevice) -> Self {
        match v {
            StreamDevice::Microphone => Self::NTG_STREAM_DEVICE_MICROPHONE,
            StreamDevice::Speaker => Self::NTG_STREAM_DEVICE_SPEAKER,
            StreamDevice::Camera => Self::NTG_STREAM_DEVICE_CAMERA,
            StreamDevice::Screen => Self::NTG_STREAM_DEVICE_SCREEN,
        }
    }
}

impl crate::util::FromC for StreamDevice {
    type C = sys::ntg_stream_device;
    unsafe fn from_c(c: &Self::C) -> Self {
        (*c).into()
    }
}

impl crate::util::ToC for StreamDevice {
    type C = sys::ntg_stream_device;
    fn to_c(&self, _conv: &mut crate::util::Conv) -> Self::C {
        (*self).into()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Connecting,
    Connected,
    Failed,
    Timeout,
    Closed,
}

impl From<sys::ntg_connection_state> for ConnectionState {
    fn from(c: sys::ntg_connection_state) -> Self {
        match c {
            sys::ntg_connection_state::NTG_CONNECTION_STATE_CONNECTING => Self::Connecting,
            sys::ntg_connection_state::NTG_CONNECTION_STATE_CONNECTED => Self::Connected,
            sys::ntg_connection_state::NTG_CONNECTION_STATE_FAILED => Self::Failed,
            sys::ntg_connection_state::NTG_CONNECTION_STATE_TIMEOUT => Self::Timeout,
            sys::ntg_connection_state::NTG_CONNECTION_STATE_CLOSED => Self::Closed,
        }
    }
}

impl From<ConnectionState> for sys::ntg_connection_state {
    fn from(v: ConnectionState) -> Self {
        match v {
            ConnectionState::Connecting => Self::NTG_CONNECTION_STATE_CONNECTING,
            ConnectionState::Connected => Self::NTG_CONNECTION_STATE_CONNECTED,
            ConnectionState::Failed => Self::NTG_CONNECTION_STATE_FAILED,
            ConnectionState::Timeout => Self::NTG_CONNECTION_STATE_TIMEOUT,
            ConnectionState::Closed => Self::NTG_CONNECTION_STATE_CLOSED,
        }
    }
}

impl crate::util::FromC for ConnectionState {
    type C = sys::ntg_connection_state;
    unsafe fn from_c(c: &Self::C) -> Self {
        (*c).into()
    }
}

impl crate::util::ToC for ConnectionState {
    type C = sys::ntg_connection_state;
    fn to_c(&self, _conv: &mut crate::util::Conv) -> Self::C {
        (*self).into()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionKind {
    Normal,
    Presentation,
}

impl From<sys::ntg_connection_kind> for ConnectionKind {
    fn from(c: sys::ntg_connection_kind) -> Self {
        match c {
            sys::ntg_connection_kind::NTG_CONNECTION_KIND_NORMAL => Self::Normal,
            sys::ntg_connection_kind::NTG_CONNECTION_KIND_PRESENTATION => Self::Presentation,
        }
    }
}

impl From<ConnectionKind> for sys::ntg_connection_kind {
    fn from(v: ConnectionKind) -> Self {
        match v {
            ConnectionKind::Normal => Self::NTG_CONNECTION_KIND_NORMAL,
            ConnectionKind::Presentation => Self::NTG_CONNECTION_KIND_PRESENTATION,
        }
    }
}

impl crate::util::FromC for ConnectionKind {
    type C = sys::ntg_connection_kind;
    unsafe fn from_c(c: &Self::C) -> Self {
        (*c).into()
    }
}

impl crate::util::ToC for ConnectionKind {
    type C = sys::ntg_connection_kind;
    fn to_c(&self, _conv: &mut crate::util::Conv) -> Self::C {
        (*self).into()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallType {
    Group,
    Outgoing,
    Incoming,
    P2p,
    Conference,
}

impl From<sys::ntg_call_type> for CallType {
    fn from(c: sys::ntg_call_type) -> Self {
        match c {
            sys::ntg_call_type::NTG_CALL_TYPE_GROUP => Self::Group,
            sys::ntg_call_type::NTG_CALL_TYPE_OUTGOING => Self::Outgoing,
            sys::ntg_call_type::NTG_CALL_TYPE_INCOMING => Self::Incoming,
            sys::ntg_call_type::NTG_CALL_TYPE_P2P => Self::P2p,
            sys::ntg_call_type::NTG_CALL_TYPE_CONFERENCE => Self::Conference,
        }
    }
}

impl From<CallType> for sys::ntg_call_type {
    fn from(v: CallType) -> Self {
        match v {
            CallType::Group => Self::NTG_CALL_TYPE_GROUP,
            CallType::Outgoing => Self::NTG_CALL_TYPE_OUTGOING,
            CallType::Incoming => Self::NTG_CALL_TYPE_INCOMING,
            CallType::P2p => Self::NTG_CALL_TYPE_P2P,
            CallType::Conference => Self::NTG_CALL_TYPE_CONFERENCE,
        }
    }
}

impl crate::util::FromC for CallType {
    type C = sys::ntg_call_type;
    unsafe fn from_c(c: &Self::C) -> Self {
        (*c).into()
    }
}

impl crate::util::ToC for CallType {
    type C = sys::ntg_call_type;
    fn to_c(&self, _conv: &mut crate::util::Conv) -> Self::C {
        (*self).into()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaSegmentQuality {
    None,
    Thumbnail,
    Medium,
    Full,
}

impl From<sys::ntg_media_segment_quality> for MediaSegmentQuality {
    fn from(c: sys::ntg_media_segment_quality) -> Self {
        match c {
            sys::ntg_media_segment_quality::NTG_MEDIA_SEGMENT_QUALITY_NONE => Self::None,
            sys::ntg_media_segment_quality::NTG_MEDIA_SEGMENT_QUALITY_THUMBNAIL => Self::Thumbnail,
            sys::ntg_media_segment_quality::NTG_MEDIA_SEGMENT_QUALITY_MEDIUM => Self::Medium,
            sys::ntg_media_segment_quality::NTG_MEDIA_SEGMENT_QUALITY_FULL => Self::Full,
        }
    }
}

impl From<MediaSegmentQuality> for sys::ntg_media_segment_quality {
    fn from(v: MediaSegmentQuality) -> Self {
        match v {
            MediaSegmentQuality::None => Self::NTG_MEDIA_SEGMENT_QUALITY_NONE,
            MediaSegmentQuality::Thumbnail => Self::NTG_MEDIA_SEGMENT_QUALITY_THUMBNAIL,
            MediaSegmentQuality::Medium => Self::NTG_MEDIA_SEGMENT_QUALITY_MEDIUM,
            MediaSegmentQuality::Full => Self::NTG_MEDIA_SEGMENT_QUALITY_FULL,
        }
    }
}

impl crate::util::FromC for MediaSegmentQuality {
    type C = sys::ntg_media_segment_quality;
    unsafe fn from_c(c: &Self::C) -> Self {
        (*c).into()
    }
}

impl crate::util::ToC for MediaSegmentQuality {
    type C = sys::ntg_media_segment_quality;
    fn to_c(&self, _conv: &mut crate::util::Conv) -> Self::C {
        (*self).into()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaSegmentPartStatus {
    NotReady,
    ResyncNeeded,
    Downloading,
    Success,
}

impl From<sys::ntg_media_segment_part_status> for MediaSegmentPartStatus {
    fn from(c: sys::ntg_media_segment_part_status) -> Self {
        match c {
            sys::ntg_media_segment_part_status::NTG_MEDIA_SEGMENT_PART_STATUS_NOT_READY => Self::NotReady,
            sys::ntg_media_segment_part_status::NTG_MEDIA_SEGMENT_PART_STATUS_RESYNC_NEEDED => Self::ResyncNeeded,
            sys::ntg_media_segment_part_status::NTG_MEDIA_SEGMENT_PART_STATUS_DOWNLOADING => Self::Downloading,
            sys::ntg_media_segment_part_status::NTG_MEDIA_SEGMENT_PART_STATUS_SUCCESS => Self::Success,
        }
    }
}

impl From<MediaSegmentPartStatus> for sys::ntg_media_segment_part_status {
    fn from(v: MediaSegmentPartStatus) -> Self {
        match v {
            MediaSegmentPartStatus::NotReady => Self::NTG_MEDIA_SEGMENT_PART_STATUS_NOT_READY,
            MediaSegmentPartStatus::ResyncNeeded => Self::NTG_MEDIA_SEGMENT_PART_STATUS_RESYNC_NEEDED,
            MediaSegmentPartStatus::Downloading => Self::NTG_MEDIA_SEGMENT_PART_STATUS_DOWNLOADING,
            MediaSegmentPartStatus::Success => Self::NTG_MEDIA_SEGMENT_PART_STATUS_SUCCESS,
        }
    }
}

impl crate::util::FromC for MediaSegmentPartStatus {
    type C = sys::ntg_media_segment_part_status;
    unsafe fn from_c(c: &Self::C) -> Self {
        (*c).into()
    }
}

impl crate::util::ToC for MediaSegmentPartStatus {
    type C = sys::ntg_media_segment_part_status;
    fn to_c(&self, _conv: &mut crate::util::Conv) -> Self::C {
        (*self).into()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionMode {
    None,
    Rtc,
    Stream,
    Rtmp,
}

impl From<sys::ntg_connection_mode> for ConnectionMode {
    fn from(c: sys::ntg_connection_mode) -> Self {
        match c {
            sys::ntg_connection_mode::NTG_CONNECTION_MODE_NONE => Self::None,
            sys::ntg_connection_mode::NTG_CONNECTION_MODE_RTC => Self::Rtc,
            sys::ntg_connection_mode::NTG_CONNECTION_MODE_STREAM => Self::Stream,
            sys::ntg_connection_mode::NTG_CONNECTION_MODE_RTMP => Self::Rtmp,
        }
    }
}

impl From<ConnectionMode> for sys::ntg_connection_mode {
    fn from(v: ConnectionMode) -> Self {
        match v {
            ConnectionMode::None => Self::NTG_CONNECTION_MODE_NONE,
            ConnectionMode::Rtc => Self::NTG_CONNECTION_MODE_RTC,
            ConnectionMode::Stream => Self::NTG_CONNECTION_MODE_STREAM,
            ConnectionMode::Rtmp => Self::NTG_CONNECTION_MODE_RTMP,
        }
    }
}

impl crate::util::FromC for ConnectionMode {
    type C = sys::ntg_connection_mode;
    unsafe fn from_c(c: &Self::C) -> Self {
        (*c).into()
    }
}

impl crate::util::ToC for ConnectionMode {
    type C = sys::ntg_connection_mode;
    fn to_c(&self, _conv: &mut crate::util::Conv) -> Self::C {
        (*self).into()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoRotation {
    VideoRotation0,
    VideoRotation90,
    VideoRotation180,
    VideoRotation270,
}

impl From<sys::ntg_video_rotation> for VideoRotation {
    fn from(c: sys::ntg_video_rotation) -> Self {
        match c {
            sys::ntg_video_rotation::NTG_VIDEO_ROTATION_VIDEO_ROTATION_0 => Self::VideoRotation0,
            sys::ntg_video_rotation::NTG_VIDEO_ROTATION_VIDEO_ROTATION_90 => Self::VideoRotation90,
            sys::ntg_video_rotation::NTG_VIDEO_ROTATION_VIDEO_ROTATION_180 => Self::VideoRotation180,
            sys::ntg_video_rotation::NTG_VIDEO_ROTATION_VIDEO_ROTATION_270 => Self::VideoRotation270,
        }
    }
}

impl From<VideoRotation> for sys::ntg_video_rotation {
    fn from(v: VideoRotation) -> Self {
        match v {
            VideoRotation::VideoRotation0 => Self::NTG_VIDEO_ROTATION_VIDEO_ROTATION_0,
            VideoRotation::VideoRotation90 => Self::NTG_VIDEO_ROTATION_VIDEO_ROTATION_90,
            VideoRotation::VideoRotation180 => Self::NTG_VIDEO_ROTATION_VIDEO_ROTATION_180,
            VideoRotation::VideoRotation270 => Self::NTG_VIDEO_ROTATION_VIDEO_ROTATION_270,
        }
    }
}

impl crate::util::FromC for VideoRotation {
    type C = sys::ntg_video_rotation;
    unsafe fn from_c(c: &Self::C) -> Self {
        (*c).into()
    }
}

impl crate::util::ToC for VideoRotation {
    type C = sys::ntg_video_rotation;
    fn to_c(&self, _conv: &mut crate::util::Conv) -> Self::C {
        (*self).into()
    }
}

