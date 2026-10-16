//! On-device speech shared by the telegram, whatsapp and lang MCPs.
//!
//! sherpa-onnx (dlopen'ed C API) runs everything in-process with models kept loaded:
//! Silero VAD, GTCRN noise suppression, Whisper language identification and
//! transcription, Piper/Kokoro voices. Languages, voices and settings come from the
//! registry that bia-mcp-lang manages.

pub mod audio;
pub mod calllog;
pub mod catalog;
pub mod contacts;
pub mod diag;
pub mod filter;
pub mod input;
pub mod langs;
pub mod learn;
pub mod listen;
pub mod models;
pub mod paths;
pub mod pipeline;
pub mod registry;
pub mod sherpa;
pub mod store;
pub mod tts;

pub use listen::{Listener, Utterance};
pub use pipeline::{discovery_notice, transcribe_file, transcribe_note, transcribe_note_from, Heard};

/// One line for status tools.
pub fn status() -> String {
    let reg = registry::current();
    let lib = match sherpa::api() {
        Ok(_) => "ok".to_string(),
        Err(e) => e,
    };
    let voices: Vec<String> = reg
        .enabled
        .iter()
        .map(|l| match reg.voice.get(l) {
            Some(v) => format!("{l}:{v}"),
            None => format!("{l}:no-voice"),
        })
        .collect();
    format!(
        "speech: sherpa={lib} hear+speak=[{}] call_model={} file_model={} vad={} denoise={}",
        voices.join(" "),
        reg.asr_call,
        reg.asr_file,
        reg.extra("silero-vad").is_some(),
        reg.settings.denoise && reg.extra("gtcrn").is_some(),
    )
}

/// Load call models and voices in the background (call start / MCP start).
pub fn warm() {
    models::warm();
    tts::warm();
}
