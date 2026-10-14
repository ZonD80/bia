// @generated from schema/ntgcalls.ntl -- DO NOT EDIT
use ntgcalls_sys as sys;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorKind {
    Rtc,
    SdpParse,
    TransportParse,
    RtmpStreamingUnsupported,
    Connection,
    ConnectionNotFound,
    ConnectionError,
    CryptoError,
    TelegramServerError,
    RtcConnectionNeeded,
    InvalidParams,
    Signaling,
    SignalingError,
    SignalingUnsupported,
    Media,
    FileError,
    FfmpegError,
    ShellError,
    MediaDeviceError,
    Unknown(i32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub kind: ErrorKind,
    pub message: String,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.message.is_empty() {
            write!(f, "{:?}", self.kind)
        } else {
            write!(f, "{:?}: {}", self.kind, self.message)
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn check(code: sys::ntg_result) -> Result<()> {
    let kind = match code {
        sys::NTG_OK => return Ok(()),
        sys::NTG_ERR_RTC => ErrorKind::Rtc,
        sys::NTG_ERR_SDP_PARSE => ErrorKind::SdpParse,
        sys::NTG_ERR_TRANSPORT_PARSE => ErrorKind::TransportParse,
        sys::NTG_ERR_RTMP_STREAMING_UNSUPPORTED => ErrorKind::RtmpStreamingUnsupported,
        sys::NTG_ERR_CONNECTION => ErrorKind::Connection,
        sys::NTG_ERR_CONNECTION_NOT_FOUND => ErrorKind::ConnectionNotFound,
        sys::NTG_ERR_CONNECTION_ERROR => ErrorKind::ConnectionError,
        sys::NTG_ERR_CRYPTO_ERROR => ErrorKind::CryptoError,
        sys::NTG_ERR_TELEGRAM_SERVER_ERROR => ErrorKind::TelegramServerError,
        sys::NTG_ERR_RTC_CONNECTION_NEEDED => ErrorKind::RtcConnectionNeeded,
        sys::NTG_ERR_INVALID_PARAMS => ErrorKind::InvalidParams,
        sys::NTG_ERR_SIGNALING => ErrorKind::Signaling,
        sys::NTG_ERR_SIGNALING_ERROR => ErrorKind::SignalingError,
        sys::NTG_ERR_SIGNALING_UNSUPPORTED => ErrorKind::SignalingUnsupported,
        sys::NTG_ERR_MEDIA => ErrorKind::Media,
        sys::NTG_ERR_FILE_ERROR => ErrorKind::FileError,
        sys::NTG_ERR_FFMPEG_ERROR => ErrorKind::FfmpegError,
        sys::NTG_ERR_SHELL_ERROR => ErrorKind::ShellError,
        sys::NTG_ERR_MEDIA_DEVICE_ERROR => ErrorKind::MediaDeviceError,
        other => ErrorKind::Unknown(other),
    };
    Err(Error {
        kind,
        message: crate::util::cstr(unsafe { sys::ntg_last_error() }),
    })
}
