mod ops;

use bia_mcp::{arg_str, arg_str_list, arg_str_opt, Server};
use serde_json::{json, Value};

fn opt_bool(a: &Value, k: &str) -> Option<bool> {
    a.get(k).and_then(Value::as_bool)
}

fn main() {
    let server = Server::new("lang", env!("CARGO_PKG_VERSION"))
        .tool(
            "list",
            "What bia hears and speaks now: learned languages with their voices, installed voices and speech models, settings, voice prints, disk use.",
            json!({"type": "object", "properties": {}}),
            |_, _| Ok(ops::list()),
        )
        .tool(
            "catalog",
            "Languages that can be learned (Whisper hears ~99; Piper voices speak ~42) and speech models. query (a code like es or a name like Spanish) lists that language's voices with locale, quality and size.",
            json!({"type": "object", "properties": {
                "query": {"type": "string", "description": "language code or name; omit for the full list"}
            }}),
            |_, a| ops::catalog_text(arg_str_opt(&a, "query")),
        )
        .tool(
            "install",
            "Learn a language: download and verify its voice (sha256-pinned) and enable it, so speech in it is transcribed and you can speak it (lang= on call_say / send_voice). The first non-English language also installs whisper-base (207 MB) for voice notes, and Persian (fa) its own model shenava-fa (117 MB). Do it without asking when you hear a new language ([new language heard: …]) or the user speaks or asks for one; tell the user first that you are learning it. lang may carry a region (es-MX, pt-BR, en-GB) to pick a voice of that locale. voice picks a specific voice from lang_catalog.",
            json!({"type": "object", "required": ["lang"], "properties": {
                "lang": {"type": "string", "description": "language code (es), with optional region (es-MX), or English name"},
                "voice": {"type": "string", "description": "voice id from lang_catalog (optional)"}
            }}),
            |ctx, a| ops::install(ctx, arg_str(&a, "lang")?, arg_str_opt(&a, "voice")),
        )
        .tool(
            "remove",
            "Forget a language and delete its voice files. Destructive: ask the user first.",
            json!({"type": "object", "required": ["lang"], "properties": {"lang": {"type": "string"}}}),
            |_, a| ops::remove(arg_str(&a, "lang")?),
        )
        .tool(
            "enable",
            "Turn a learned (or hear-only) language back on for transcription. Files are not downloaded; use lang_install to get a voice.",
            json!({"type": "object", "required": ["lang"], "properties": {"lang": {"type": "string"}}}),
            |_, a| ops::enable(arg_str(&a, "lang")?),
        )
        .tool(
            "disable",
            "Stop routing speech to a language (files kept) and stop new-language notices for it. Use when the user does not want a language learned.",
            json!({"type": "object", "required": ["lang"], "properties": {"lang": {"type": "string"}}}),
            |_, a| ops::disable(arg_str(&a, "lang")?),
        )
        .tool(
            "input",
            "How a language is typed on the console (fcitx5; the top bar shows the current one, a click or Ctrl+Space switches). Learning a language adds its keyboard on its own when a US keyboard cannot type its script (ru → keyboard-ru, zh → pinyin, ja → anthy, ko → hangul, vi → unikey, hi → keyboard-in …). Set it when the user wants another: a native Latin layout (de → keyboard-de, fr → keyboard-fr, es → keyboard-es, or keyboard-us-intl for accents), a variant (keyboard-ru-phonetic), \"\" for none, \"auto\" for the default.",
            json!({"type": "object", "required": ["lang", "input"], "properties": {
                "lang": {"type": "string"},
                "input": {"type": "string", "description": "keyboard-<xkb layout>[-<variant>], an input method (pinyin, anthy, hangul, unikey), \"\" or \"auto\""}
            }}),
            |_, a| ops::set_input(arg_str(&a, "lang")?, arg_str(&a, "input")?),
        )
        .tool(
            "voice",
            "Choose the voice a language is spoken with (downloads it if needed). Voice ids come from lang_catalog(query=lang).",
            json!({"type": "object", "required": ["lang", "voice"], "properties": {
                "lang": {"type": "string"},
                "voice": {"type": "string"}
            }}),
            |ctx, a| ops::set_voice(ctx, arg_str(&a, "lang")?, arg_str(&a, "voice")?),
        )
        .tool(
            "preview",
            "Make a short OGG sample of a language's voice (or of a catalog voice, downloaded if needed) under /home/bia/files/speech/previews; send it with say(files=[path]) so the user can hear it.",
            json!({"type": "object", "required": ["lang"], "properties": {
                "lang": {"type": "string"},
                "voice": {"type": "string", "description": "voice id (optional)"},
                "text": {"type": "string", "description": "what to say (optional)"}
            }}),
            |ctx, a| ops::preview(ctx, arg_str(&a, "lang")?, arg_str_opt(&a, "voice"), arg_str_opt(&a, "text")),
        )
        .tool(
            "default",
            "Set the default language: assumed for short unclear speech and at the start of each call.",
            json!({"type": "object", "required": ["lang"], "properties": {"lang": {"type": "string"}}}),
            |_, a| ops::set_default(arg_str(&a, "lang")?),
        )
        .tool(
            "settings",
            "Speech settings. denoise: clean audio (GTCRN) before language detection (default on). asr_denoised: Whisper gets the cleaned audio too (default on). split_pauses: split at short pauses to catch a language change mid-phrase (on). hallucination_filter: drop Whisper noise phrases (on). discovery: report languages heard but not learned (on). echo_filter: drop call speech repeating what you say (opt-in). speaker_check: drop speech whose voice does not match the enrolled voice print (opt-in; downloads a 29 MB model; see lang_enroll). call_model / file_model: whisper-tiny (bundled, fast), whisper-base (207 MB, better), whisper-small (639 MB, voice notes only), parakeet-v3 (487 MB; 25 European languages incl. en, de, es, fr, it, ru, uk, pl, pt: fastest and most accurate for them; installed for voice notes with the first of them learned; a voice note in another language uses the best Whisper installed) — downloaded when chosen. Voice notes are transcribed phrase by phrase, each in its own language.",
            json!({"type": "object", "properties": {
                "denoise": {"type": "boolean"},
                "asr_denoised": {"type": "boolean"},
                "split_pauses": {"type": "boolean"},
                "hallucination_filter": {"type": "boolean"},
                "discovery": {"type": "boolean"},
                "echo_filter": {"type": "boolean"},
                "speaker_check": {"type": "boolean"},
                "speaker_threshold": {"type": "number", "description": "0..1 cosine similarity to pass (default 0.45)"},
                "call_model": {"type": "string"},
                "file_model": {"type": "string"}
            }}),
            |ctx, a| {
                let keys = ["denoise", "asr_denoised", "split_pauses", "hallucination_filter", "discovery", "echo_filter", "speaker_check"];
                let bools = keys.iter().filter_map(|k| opt_bool(&a, k).map(|v| (*k, v))).collect();
                ops::settings(
                    ctx,
                    ops::SettingsArgs {
                        bools,
                        speaker_threshold: a.get("speaker_threshold").and_then(Value::as_f64).map(|f| f as f32),
                        call_model: arg_str_opt(&a, "call_model").filter(|s| !s.is_empty()).map(str::to_string),
                        file_model: arg_str_opt(&a, "file_model").filter(|s| !s.is_empty()).map(str::to_string),
                    },
                )
            },
        )
        .tool(
            "transcribe",
            "Transcribe an audio file (voice note, recording; any format ffmpeg reads) with language detection per phrase: [speech ru] … [en] …; a ? after a code marks a language not learned yet (rough). lang forces one language. model overrides the voice-note model by its name in lang_list (whisper-small, parakeet-v3, shenava-fa…).",
            json!({"type": "object", "required": ["file"], "properties": {
                "file": {"type": "string", "description": "the audio: path under /home/bia/files or absolute"},
                "lang": {"type": "string"},
                "model": {"type": "string", "description": "an installed speech model's name, e.g. whisper-small"}
            }}),
            |_, a| ops::transcribe(arg_str(&a, "file")?, arg_str_opt(&a, "lang"), arg_str_opt(&a, "model")),
        )
        .tool(
            "synthesize",
            "Speak text into an audio file under /home/bia/files/speech/out (OGG/Opus by default, or wav). lang is required; mark other-language parts inline: <lang=fr>Le Petit Bistro</lang>. To send a voice message use telegram_send_voice / whatsapp_send_voice instead.",
            json!({"type": "object", "required": ["text", "lang"], "properties": {
                "text": {"type": "string"},
                "lang": {"type": "string"},
                "format": {"type": "string", "enum": ["ogg", "wav"]}
            }}),
            |_, a| ops::synthesize(arg_str(&a, "text")?, arg_str(&a, "lang")?, arg_str_opt(&a, "format").unwrap_or("ogg")),
        )
        .tool(
            "enroll",
            "Save a person's voice print for the opt-in speaker check (drops call speech from other voices, e.g. a TV or someone nearby). key is telegram:<user id> or whatsapp:<phone> (as in their envelopes); files are recordings of only that person (voice notes), 3 s of speech or more. remove=true deletes the print.",
            json!({"type": "object", "required": ["key"], "properties": {
                "key": {"type": "string"},
                "files": {"type": "array", "items": {"type": "string"}},
                "remove": {"type": "boolean"}
            }}),
            |ctx, a| ops::enroll(ctx, arg_str(&a, "key")?, &arg_str_list(&a, "files"), opt_bool(&a, "remove").unwrap_or(false)),
        );
    server.run();
}
