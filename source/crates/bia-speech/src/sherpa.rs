//! sherpa-onnx C API (v1.13.6 c-api.h layout), dlopen'ed once from the bundled speech
//! tree (or $BIA_SHERPA_LIB). Struct layouts are checked against the C header by the
//! `layout` test (sizes and offsets measured with gcc on aarch64/x86_64 Linux).

use std::ffi::{c_char, c_void, CStr, CString};
use std::path::PathBuf;
use std::sync::OnceLock;

pub type P = *const c_char;
pub type H = *const c_void;

// ---- offline recognizer ----

#[repr(C)]
pub struct FeatureCfg {
    pub sample_rate: i32,
    pub feature_dim: i32,
}

#[repr(C)]
pub struct TransducerCfg {
    pub encoder: P,
    pub decoder: P,
    pub joiner: P,
}

#[repr(C)]
pub struct OneModel {
    pub model: P,
}

#[repr(C)]
pub struct WhisperCfg {
    pub encoder: P,
    pub decoder: P,
    pub language: P,
    pub task: P,
    pub tail_paddings: i32,
    pub enable_token_timestamps: i32,
    pub enable_segment_timestamps: i32,
}

#[repr(C)]
pub struct SenseVoiceCfg {
    pub model: P,
    pub language: P,
    pub use_itn: i32,
}

#[repr(C)]
pub struct MoonshineCfg {
    pub preprocessor: P,
    pub encoder: P,
    pub uncached_decoder: P,
    pub cached_decoder: P,
    pub merged_decoder: P,
}

#[repr(C)]
pub struct EncDec {
    pub encoder: P,
    pub decoder: P,
}

#[repr(C)]
pub struct CanaryCfg {
    pub encoder: P,
    pub decoder: P,
    pub src_lang: P,
    pub tgt_lang: P,
    pub use_pnc: i32,
}

#[repr(C)]
pub struct FunAsrNanoCfg {
    pub encoder_adaptor: P,
    pub llm: P,
    pub embedding: P,
    pub tokenizer: P,
    pub system_prompt: P,
    pub user_prompt: P,
    pub max_new_tokens: i32,
    pub temperature: f32,
    pub top_p: f32,
    pub seed: i32,
    pub language: P,
    pub itn: i32,
    pub hotwords: P,
}

#[repr(C)]
pub struct Qwen3AsrCfg {
    pub conv_frontend: P,
    pub encoder: P,
    pub decoder: P,
    pub tokenizer: P,
    pub max_total_len: i32,
    pub max_new_tokens: i32,
    pub temperature: f32,
    pub top_p: f32,
    pub seed: i32,
    pub hotwords: P,
}

#[repr(C)]
pub struct CohereCfg {
    pub encoder: P,
    pub decoder: P,
    pub language: P,
    pub use_punct: i32,
    pub use_itn: i32,
}

#[repr(C)]
pub struct OfflineModelCfg {
    pub transducer: TransducerCfg,
    pub paraformer: OneModel,
    pub nemo_ctc: OneModel,
    pub whisper: WhisperCfg,
    pub tdnn: OneModel,
    pub tokens: P,
    pub num_threads: i32,
    pub debug: i32,
    pub provider: P,
    pub model_type: P,
    pub modeling_unit: P,
    pub bpe_vocab: P,
    pub telespeech_ctc: P,
    pub sense_voice: SenseVoiceCfg,
    pub moonshine: MoonshineCfg,
    pub fire_red_asr: EncDec,
    pub dolphin: OneModel,
    pub zipformer_ctc: OneModel,
    pub canary: CanaryCfg,
    pub wenet_ctc: OneModel,
    pub omnilingual: OneModel,
    pub medasr: OneModel,
    pub funasr_nano: FunAsrNanoCfg,
    pub fire_red_asr_ctc: OneModel,
    pub qwen3_asr: Qwen3AsrCfg,
    pub cohere_transcribe: CohereCfg,
}

#[repr(C)]
pub struct LmCfg {
    pub model: P,
    pub scale: f32,
}

#[repr(C)]
pub struct HomophoneCfg {
    pub dict_dir: P,
    pub lexicon: P,
    pub rule_fsts: P,
}

#[repr(C)]
pub struct OfflineRecognizerCfg {
    pub feat_config: FeatureCfg,
    pub model_config: OfflineModelCfg,
    pub lm_config: LmCfg,
    pub decoding_method: P,
    pub max_active_paths: i32,
    pub hotwords_file: P,
    pub hotwords_score: f32,
    pub rule_fsts: P,
    pub rule_fars: P,
    pub blank_penalty: f32,
    pub hr: HomophoneCfg,
}

#[repr(C)]
pub struct OfflineResult {
    pub text: P,
    pub timestamps: *const f32,
    pub count: i32,
    pub tokens: P,
    pub tokens_arr: *const P,
    pub json: P,
    pub lang: P,
    pub emotion: P,
    pub event: P,
    pub durations: *const f32,
    pub ys_log_probs: *const f32,
    pub segment_timestamps: *const f32,
    pub segment_durations: *const f32,
    pub segment_texts: P,
    pub segment_texts_arr: *const P,
    pub segment_count: i32,
}

// ---- VAD ----

#[repr(C)]
pub struct SileroCfg {
    pub model: P,
    pub threshold: f32,
    pub min_silence_duration: f32,
    pub min_speech_duration: f32,
    pub window_size: i32,
    pub max_speech_duration: f32,
}

#[repr(C)]
pub struct VadCfg {
    pub silero_vad: SileroCfg,
    pub sample_rate: i32,
    pub num_threads: i32,
    pub provider: P,
    pub debug: i32,
    pub ten_vad: SileroCfg,
}

#[repr(C)]
pub struct SpeechSegment {
    pub start: i32,
    pub samples: *const f32,
    pub n: i32,
}

// ---- spoken language identification ----

#[repr(C)]
pub struct SlidWhisperCfg {
    pub encoder: P,
    pub decoder: P,
    pub tail_paddings: i32,
}

#[repr(C)]
pub struct SlidCfg {
    pub whisper: SlidWhisperCfg,
    pub num_threads: i32,
    pub debug: i32,
    pub provider: P,
}

#[repr(C)]
pub struct SlidResult {
    pub lang: P,
}

// ---- denoiser ----

#[repr(C)]
pub struct DpdfNetCfg {
    pub model: P,
    pub attenuation_limit_db: f32,
}

#[repr(C)]
pub struct DenoiserModelCfg {
    pub gtcrn: OneModel,
    pub num_threads: i32,
    pub debug: i32,
    pub provider: P,
    pub dpdfnet: DpdfNetCfg,
}

#[repr(C)]
pub struct DenoiserCfg {
    pub model: DenoiserModelCfg,
}

#[repr(C)]
pub struct DenoisedAudio {
    pub samples: *const f32,
    pub n: i32,
    pub sample_rate: i32,
}

// ---- speaker embedding ----

#[repr(C)]
pub struct SpeakerCfg {
    pub model: P,
    pub num_threads: i32,
    pub debug: i32,
    pub provider: P,
}

// ---- TTS ----

#[repr(C)]
pub struct VitsCfg {
    pub model: P,
    pub lexicon: P,
    pub tokens: P,
    pub data_dir: P,
    pub noise_scale: f32,
    pub noise_scale_w: f32,
    pub length_scale: f32,
    pub dict_dir: P,
}

#[repr(C)]
pub struct MatchaCfg {
    pub acoustic_model: P,
    pub vocoder: P,
    pub lexicon: P,
    pub tokens: P,
    pub data_dir: P,
    pub noise_scale: f32,
    pub length_scale: f32,
    pub dict_dir: P,
}

#[repr(C)]
pub struct KokoroCfg {
    pub model: P,
    pub voices: P,
    pub tokens: P,
    pub data_dir: P,
    pub length_scale: f32,
    pub dict_dir: P,
    pub lexicon: P,
    pub lang: P,
}

#[repr(C)]
pub struct KittenCfg {
    pub model: P,
    pub voices: P,
    pub tokens: P,
    pub data_dir: P,
    pub length_scale: f32,
}

#[repr(C)]
pub struct ZipvoiceCfg {
    pub tokens: P,
    pub encoder: P,
    pub decoder: P,
    pub vocoder: P,
    pub data_dir: P,
    pub lexicon: P,
    pub feat_scale: f32,
    pub t_shift: f32,
    pub target_rms: f32,
    pub guidance_scale: f32,
}

#[repr(C)]
pub struct PocketCfg {
    pub lm_flow: P,
    pub lm_main: P,
    pub encoder: P,
    pub decoder: P,
    pub text_conditioner: P,
    pub vocab_json: P,
    pub token_scores_json: P,
    pub voice_embedding_cache_capacity: i32,
}

#[repr(C)]
pub struct SupertonicCfg {
    pub duration_predictor: P,
    pub text_encoder: P,
    pub vector_estimator: P,
    pub vocoder: P,
    pub tts_json: P,
    pub unicode_indexer: P,
    pub voice_style: P,
}

#[repr(C)]
pub struct TtsModelCfg {
    pub vits: VitsCfg,
    pub num_threads: i32,
    pub debug: i32,
    pub provider: P,
    pub matcha: MatchaCfg,
    pub kokoro: KokoroCfg,
    pub kitten: KittenCfg,
    pub zipvoice: ZipvoiceCfg,
    pub pocket: PocketCfg,
    pub supertonic: SupertonicCfg,
}

#[repr(C)]
pub struct TtsCfg {
    pub model: TtsModelCfg,
    pub rule_fsts: P,
    pub max_num_sentences: i32,
    pub rule_fars: P,
    pub silence_scale: f32,
}

#[repr(C)]
pub struct GeneratedAudio {
    pub samples: *const f32,
    pub n: i32,
    pub sample_rate: i32,
}

pub type ChunkCb = extern "C" fn(*const f32, i32, *mut c_void) -> i32;

macro_rules! api {
    ($($field:ident : fn($($arg:ty),*) $(-> $ret:ty)? = $sym:literal;)*) => {
        /// Function table of libsherpa-onnx-c-api.so.
        pub struct Api {
            $(pub $field: unsafe extern "C" fn($($arg),*) $(-> $ret)?,)*
        }
        impl Api {
            unsafe fn resolve(lib: *mut c_void) -> Result<Self, String> {
                Ok(Self { $($field: std::mem::transmute::<*mut c_void, unsafe extern "C" fn($($arg),*) $(-> $ret)?>(sym(lib, $sym)?),)* })
            }
        }
    };
}

api! {
    create_recognizer: fn(*const OfflineRecognizerCfg) -> H = "SherpaOnnxCreateOfflineRecognizer";
    recognizer_set_config: fn(H, *const OfflineRecognizerCfg) = "SherpaOnnxOfflineRecognizerSetConfig";
    destroy_recognizer: fn(H) = "SherpaOnnxDestroyOfflineRecognizer";
    create_offline_stream: fn(H) -> H = "SherpaOnnxCreateOfflineStream";
    destroy_offline_stream: fn(H) = "SherpaOnnxDestroyOfflineStream";
    accept_waveform_offline: fn(H, i32, *const f32, i32) = "SherpaOnnxAcceptWaveformOffline";
    decode_offline_stream: fn(H, H) = "SherpaOnnxDecodeOfflineStream";
    get_offline_result: fn(H) -> *const OfflineResult = "SherpaOnnxGetOfflineStreamResult";
    destroy_offline_result: fn(*const OfflineResult) = "SherpaOnnxDestroyOfflineRecognizerResult";

    create_vad: fn(*const VadCfg, f32) -> H = "SherpaOnnxCreateVoiceActivityDetector";
    destroy_vad: fn(H) = "SherpaOnnxDestroyVoiceActivityDetector";
    vad_accept: fn(H, *const f32, i32) = "SherpaOnnxVoiceActivityDetectorAcceptWaveform";
    vad_empty: fn(H) -> i32 = "SherpaOnnxVoiceActivityDetectorEmpty";
    vad_detected: fn(H) -> i32 = "SherpaOnnxVoiceActivityDetectorDetected";
    vad_front: fn(H) -> *const SpeechSegment = "SherpaOnnxVoiceActivityDetectorFront";
    vad_pop: fn(H) = "SherpaOnnxVoiceActivityDetectorPop";
    vad_flush: fn(H) = "SherpaOnnxVoiceActivityDetectorFlush";
    vad_reset: fn(H) = "SherpaOnnxVoiceActivityDetectorReset";
    destroy_segment: fn(*const SpeechSegment) = "SherpaOnnxDestroySpeechSegment";

    create_slid: fn(*const SlidCfg) -> H = "SherpaOnnxCreateSpokenLanguageIdentification";
    destroy_slid: fn(H) = "SherpaOnnxDestroySpokenLanguageIdentification";
    slid_create_stream: fn(H) -> H = "SherpaOnnxSpokenLanguageIdentificationCreateOfflineStream";
    slid_compute: fn(H, H) -> *const SlidResult = "SherpaOnnxSpokenLanguageIdentificationCompute";
    destroy_slid_result: fn(*const SlidResult) = "SherpaOnnxDestroySpokenLanguageIdentificationResult";

    create_denoiser: fn(*const DenoiserCfg) -> H = "SherpaOnnxCreateOfflineSpeechDenoiser";
    destroy_denoiser: fn(H) = "SherpaOnnxDestroyOfflineSpeechDenoiser";
    denoiser_sample_rate: fn(H) -> i32 = "SherpaOnnxOfflineSpeechDenoiserGetSampleRate";
    denoiser_run: fn(H, *const f32, i32, i32) -> *const DenoisedAudio = "SherpaOnnxOfflineSpeechDenoiserRun";
    destroy_denoised: fn(*const DenoisedAudio) = "SherpaOnnxDestroyDenoisedAudio";

    create_speaker: fn(*const SpeakerCfg) -> H = "SherpaOnnxCreateSpeakerEmbeddingExtractor";
    destroy_speaker: fn(H) = "SherpaOnnxDestroySpeakerEmbeddingExtractor";
    speaker_dim: fn(H) -> i32 = "SherpaOnnxSpeakerEmbeddingExtractorDim";
    speaker_create_stream: fn(H) -> H = "SherpaOnnxSpeakerEmbeddingExtractorCreateStream";
    speaker_is_ready: fn(H, H) -> i32 = "SherpaOnnxSpeakerEmbeddingExtractorIsReady";
    speaker_compute: fn(H, H) -> *const f32 = "SherpaOnnxSpeakerEmbeddingExtractorComputeEmbedding";
    speaker_destroy_embedding: fn(*const f32) = "SherpaOnnxSpeakerEmbeddingExtractorDestroyEmbedding";
    online_accept_waveform: fn(H, i32, *const f32, i32) = "SherpaOnnxOnlineStreamAcceptWaveform";
    online_input_finished: fn(H) = "SherpaOnnxOnlineStreamInputFinished";
    destroy_online_stream: fn(H) = "SherpaOnnxDestroyOnlineStream";

    create_tts: fn(*const TtsCfg) -> H = "SherpaOnnxCreateOfflineTts";
    destroy_tts: fn(H) = "SherpaOnnxDestroyOfflineTts";
    tts_sample_rate: fn(H) -> i32 = "SherpaOnnxOfflineTtsSampleRate";
    tts_generate: fn(H, P, i32, f32, ChunkCb, *mut c_void) -> *const GeneratedAudio = "SherpaOnnxOfflineTtsGenerateWithCallbackWithArg";
    destroy_generated: fn(*const GeneratedAudio) = "SherpaOnnxDestroyOfflineTtsGeneratedAudio";
}

fn sym(lib: *mut c_void, name: &str) -> Result<*mut c_void, String> {
    let c = CString::new(name).unwrap();
    let p = unsafe { libc::dlsym(lib, c.as_ptr()) };
    if p.is_null() {
        Err(format!("dlsym {name}: {}", dl_error()))
    } else {
        Ok(p)
    }
}

fn dl_error() -> String {
    let e = unsafe { libc::dlerror() };
    if e.is_null() {
        "unknown error".into()
    } else {
        unsafe { CStr::from_ptr(e) }.to_string_lossy().into_owned()
    }
}

pub fn lib_path() -> PathBuf {
    std::env::var("BIA_SHERPA_LIB")
        .map(PathBuf::from)
        .unwrap_or_else(|_| crate::paths::bundled().join("lib/libsherpa-onnx-c-api.so"))
}

/// The process-wide function table; the library is never dlclose'd.
pub fn api() -> Result<&'static Api, String> {
    static API: OnceLock<Result<Api, String>> = OnceLock::new();
    API.get_or_init(|| {
        let path = lib_path();
        if !path.exists() {
            return Err(format!("{} missing", path.display()));
        }
        let c_path = CString::new(path.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
        let lib = unsafe { libc::dlopen(c_path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
        if lib.is_null() {
            return Err(format!("dlopen {}: {}", path.display(), dl_error()));
        }
        unsafe { Api::resolve(lib) }
    })
    .as_ref()
    .map_err(Clone::clone)
}

/// Owns the C strings a config points into; they must outlive the create call.
#[derive(Default)]
pub struct Strs(Vec<CString>);

impl Strs {
    pub fn p(&mut self, s: &str) -> P {
        let c = CString::new(s.replace('\0', " ")).unwrap_or_default();
        let p = c.as_ptr();
        self.0.push(c);
        p
    }
}

pub fn cstr_to_string(p: P) -> String {
    if p.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
    }
}

#[cfg(test)]
mod layout {
    use super::*;
    use std::mem::{offset_of, size_of};

    /// Values printed by a gcc build of c-api.h v1.13.6 (LP64).
    #[test]
    #[cfg(target_pointer_width = "64")]
    fn matches_c_header() {
        assert_eq!(size_of::<OfflineRecognizerCfg>(), 608);
        assert_eq!(size_of::<OfflineModelCfg>(), 504);
        assert_eq!(offset_of!(OfflineRecognizerCfg, model_config), 8);
        assert_eq!(offset_of!(OfflineRecognizerCfg, lm_config), 512);
        assert_eq!(offset_of!(OfflineRecognizerCfg, decoding_method), 528);
        assert_eq!(offset_of!(OfflineRecognizerCfg, hr), 584);
        assert_eq!(offset_of!(OfflineModelCfg, whisper), 40);
        assert_eq!(offset_of!(OfflineModelCfg, tokens), 96);
        assert_eq!(offset_of!(OfflineModelCfg, num_threads), 104);
        assert_eq!(offset_of!(OfflineModelCfg, provider), 112);
        assert_eq!(offset_of!(OfflineModelCfg, sense_voice), 152);
        assert_eq!(offset_of!(OfflineModelCfg, funasr_nano), 312);
        assert_eq!(offset_of!(OfflineModelCfg, qwen3_asr), 408);
        assert_eq!(offset_of!(OfflineModelCfg, cohere_transcribe), 472);
        assert_eq!(size_of::<OfflineResult>(), 128);
        assert_eq!(offset_of!(OfflineResult, lang), 48);
        assert_eq!(offset_of!(OfflineResult, ys_log_probs), 80);
        assert_eq!(offset_of!(OfflineResult, segment_count), 120);
        assert_eq!(size_of::<VadCfg>(), 88);
        assert_eq!(offset_of!(VadCfg, sample_rate), 32);
        assert_eq!(offset_of!(VadCfg, ten_vad), 56);
        assert_eq!(size_of::<SpeechSegment>(), 24);
        assert_eq!(size_of::<SlidCfg>(), 40);
        assert_eq!(offset_of!(SlidCfg, provider), 32);
        assert_eq!(size_of::<DenoiserCfg>(), 40);
        assert_eq!(offset_of!(DenoiserModelCfg, dpdfnet), 24);
        assert_eq!(size_of::<DenoisedAudio>(), 16);
        assert_eq!(size_of::<SpeakerCfg>(), 24);
        assert_eq!(size_of::<TtsCfg>(), 448);
        assert_eq!(offset_of!(TtsCfg, silence_scale), 440);
        assert_eq!(size_of::<TtsModelCfg>(), 416);
        assert_eq!(offset_of!(TtsModelCfg, kokoro), 128);
        assert_eq!(offset_of!(TtsModelCfg, supertonic), 360);
    }
}
