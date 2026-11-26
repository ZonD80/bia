//! On-device speech shared by the telegram, whatsapp and lang MCPs.
//!
//! sherpa-onnx (dlopen'ed C API) runs everything in-process; models stay loaded while
//! in use and are unloaded after a few idle minutes (`idle`):
//! Silero VAD, GTCRN noise suppression, Whisper language identification and
//! transcription, Piper/Kokoro voices. Languages, voices and settings come from the
//! registry that bia-mcp-lang manages.

pub mod acks;
pub mod audio;
pub mod calllog;
pub mod catalog;
pub mod contacts;
pub mod diag;
pub mod filter;
pub mod idle;
pub mod input;
pub mod langs;
pub mod learn;
pub mod listen;
pub mod models;
pub mod paths;
pub mod pipeline;
pub mod plain;
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

/// [`warm`] (call models and voices), waiting for it.
pub fn warm_wait() {
    models::warm_now();
    tts::warm_now();
}

/// Load the call models now (as a ringing call does), waiting for them.
pub fn warm_call_models() {
    models::warm_now();
}

/// The languages a call on `channel` (phone:+34…) is expected in: `lang` when bia fixed
/// it for the call, else the person's preferred language from the addressbook, else the
/// default language (someone bia does not know yet).
pub fn call_langs(channel: &str, lang: Option<&str>) -> Vec<String> {
    let reg = registry::current();
    let pick = lang
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .or_else(|| contacts::lang_for(channel))
        .or_else(|| Some(reg.default_lang.clone()));
    pick.and_then(|l| langs::parse(&l)).map(|l| vec![l.code]).filter(|v| !v[0].is_empty()).unwrap_or_default()
}

/// A language bia fixes for a call: one it can hear and speak. Its code (es-MX → es).
pub fn check_call_lang(lang: &str) -> Result<String, String> {
    let l = langs::parse(lang).ok_or_else(|| format!("unknown language {lang:?}"))?;
    if !langs::can_hear(&l.code) {
        return Err(format!("{} can not be heard in a call: no speech model knows it", l.code));
    }
    if registry::current().voice_for(&l).is_none() {
        return Err(format!("no voice speaks {} yet: lang_install(lang=\"{}\") first", l.code, l.code));
    }
    Ok(l.code)
}

/// Load what a call in `langs` needs (speech models and voices), waiting for them.
pub fn warm_call(langs: &[String], forced: bool) {
    models::warm_call(langs, forced);
    tts::warm_langs(langs);
}

/// Unload the models nobody uses right now (see [`idle::unload_now`]).
pub fn unload_now() -> usize {
    idle::unload_now()
}
