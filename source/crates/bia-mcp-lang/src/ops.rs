//! What the lang tools do. Registry changes are read-modify-write under a file lock
//! (two installs may run at once, and the telegram/whatsapp MCPs read the file).

use bia_mcp::Ctx;
use bia_speech::catalog;
use bia_speech::langs;
use bia_speech::learn::{self, ensure_asr, ensure_extra, ensure_voice, label, parse_lang, Report};
use bia_speech::registry::{self, update};
use bia_speech::store;
use std::path::{Path, PathBuf};

/// lang tool progress goes to the agent console.
struct CtxReport<'a>(&'a Ctx);

impl Report for CtxReport<'_> {
    fn progress(&self, pct: u32, msg: &str) {
        self.0.progress(pct, msg);
    }
    fn log(&self, msg: &str) {
        self.0.log(msg);
    }
    fn cancelled(&self) -> bool {
        self.0.cancelled()
    }
}


fn files_path(p: &str) -> PathBuf {
    let path = Path::new(p);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        bia_speech::paths::files().join(path)
    }
}

/// Learn a language: enable it for hearing and install its voice.
pub fn install(ctx: &Ctx, lang: &str, voice: Option<&str>) -> Result<String, String> {
    learn::install(&CtxReport(ctx), lang, voice)
}

/// Forget a language: disable it and delete voice files no other language uses.
pub fn remove(lang: &str) -> Result<String, String> {
    let l = parse_lang(lang)?;
    let code = l.code.clone();
    let (freed, voice) = update(|r| {
        if r.default_lang == code {
            return Err(format!("{} is the default language; lang_default another one first", label(&code)));
        }
        r.enabled.retain(|x| *x != code);
        let voice = r.voice.remove(&code);
        let mut freed = 0;
        if let Some(v) = &voice {
            let used = r.voice.values().any(|x| x == v);
            if !used && r.voices.get(v).is_some_and(|x| !x.bundled) {
                if let Some(x) = r.voices.remove(v) {
                    freed = store::dir_size(Path::new(&x.dir));
                    let _ = std::fs::remove_dir_all(&x.dir);
                }
            }
        }
        Ok((freed, voice))
    })?;
    Ok(format!(
        "removed {}{}{}",
        label(&l.code),
        voice.map(|v| format!(" (voice {v})")).unwrap_or_default(),
        if freed > 0 { format!("; freed {}", catalog::mb(freed)) } else { String::new() }
    ))
}

pub fn enable(lang: &str) -> Result<String, String> {
    let l = parse_lang(lang)?;
    if !langs::can_hear(&l.code) {
        return Err(format!("{} cannot be heard (Whisper does not know it)", label(&l.code)));
    }
    let code = l.code.clone();
    let has_voice = update(|r| {
        if !r.enabled.contains(&code) {
            r.enabled.push(code.clone());
        }
        r.ignored.retain(|x| *x != code);
        Ok(r.voice.contains_key(&code))
    })?;
    Ok(format!(
        "{} enabled{}",
        label(&l.code),
        if has_voice { "" } else { " for hearing only (no voice installed: lang_install to speak it)" }
    ))
}

pub fn disable(lang: &str) -> Result<String, String> {
    let l = parse_lang(lang)?;
    let code = l.code.clone();
    update(|r| {
        if r.default_lang == code {
            return Err(format!("{} is the default language; lang_default another one first", label(&code)));
        }
        let was = r.enabled.contains(&code);
        r.enabled.retain(|x| *x != code);
        if !r.ignored.contains(&code) {
            r.ignored.push(code.clone());
        }
        Ok(if was {
            format!("{} disabled (files kept; lang_enable turns it back on); no new-language notices for it", label(&code))
        } else {
            format!("{} ignored: no new-language notices for it", label(&code))
        })
    })
}

pub fn set_voice(ctx: &Ctx, lang: &str, voice: &str) -> Result<String, String> {
    let l = parse_lang(lang)?;
    let cat = catalog::load()?;
    let cv = cat.voices.get(voice).ok_or_else(|| format!("unknown voice {voice}; lang_catalog(query=\"{}\") lists them", l.code))?;
    if !cv.langs.contains(&l.code) {
        return Err(format!("voice {voice} does not speak {}", label(&l.code)));
    }
    ensure_voice(&CtxReport(ctx), &cat, voice, (0, 100))?;
    let (code, v) = (l.code.clone(), voice.to_string());
    update(move |r| {
        r.voice.insert(code, v);
        Ok(())
    })?;
    Ok(format!("{} now speaks with {voice}", label(&l.code)))
}

pub fn set_default(lang: &str) -> Result<String, String> {
    let l = parse_lang(lang)?;
    let code = l.code.clone();
    update(|r| {
        if !r.enabled.contains(&code) {
            return Err(format!("{} is not installed; lang_install it first", label(&code)));
        }
        r.default_lang = code.clone();
        Ok(())
    })?;
    Ok(format!(
        "default language: {} — speak it to root from now on (it is written into your instructions at the next start)",
        label(&l.code)
    ))
}

/// How a language is typed on the console: an fcitx5 layout or input method, "" for
/// none (the US keyboard), "auto" for bia's choice.
pub fn set_input(lang: &str, input: &str) -> Result<String, String> {
    let l = parse_lang(lang)?;
    let code = l.code.clone();
    let input = input.trim();
    if !input.is_empty() && input != "auto" {
        bia_speech::input::check(input)?;
    }
    let r = update(|r| {
        match input {
            "auto" => r.input.remove(&code),
            i => r.input.insert(code.clone(), i.to_string()),
        };
        Ok(r.clone())
    })?;
    let now = bia_speech::input::for_lang(&r, &code);
    let learned = r.enabled.contains(&code) || r.voice.contains_key(&code) || r.default_lang == code;
    Ok(format!(
        "{} types with {}{}",
        label(&code),
        now.as_deref().unwrap_or("the US keyboard"),
        if learned { "; the console's input list is updated (top bar, Ctrl+Space)" } else { " once it is learned" }
    ))
}

pub fn preview(ctx: &Ctx, lang: &str, voice: Option<&str>, text: Option<&str>) -> Result<String, String> {
    let l = parse_lang(lang)?;
    let mut speak_lang = l.code.clone();
    if let Some(v) = voice.filter(|v| !v.is_empty()) {
        let cat = catalog::load()?;
        ensure_voice(&CtxReport(ctx), &cat, v, (0, 90))?;
        // Speak with exactly this voice: its locale picks it in voice_for.
        let reg = registry::current();
        let loc = reg.voices.get(v).map(|x| x.locale.clone()).unwrap_or_default();
        if reg.voice.get(&l.code).map(String::as_str) != Some(v) {
            if loc.is_empty() {
                return Err(format!("preview of {v} needs it set: lang_voice(lang=\"{}\", voice=\"{v}\")", l.code));
            }
            speak_lang = loc.replace('_', "-");
        }
    }
    let text = text
        .filter(|t| !t.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| learn::sample_text(&l.code));
    let dir = bia_speech::paths::files().join("speech/previews");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let out = dir.join(format!("{}-{}.ogg", l.code, bia_speech::paths::stamp()));
    let (secs, _) = bia_speech::tts::voice_note(&text, &speak_lang, &out)?;
    Ok(format!("preview: {} ({secs:.1}s). Send it with say(files=[that path]).", rel(&out)))
}


fn rel(p: &Path) -> String {
    p.strip_prefix(bia_speech::paths::files()).map(|r| r.to_string_lossy().into_owned()).unwrap_or_else(|_| p.to_string_lossy().into_owned())
}

pub fn transcribe(file: &str, lang: Option<&str>, model: Option<&str>) -> Result<String, String> {
    let path = files_path(file);
    if !path.exists() {
        return Err(format!("{} not found", path.display()));
    }
    let forced = match lang.filter(|l| !l.is_empty()) {
        Some(l) => {
            let p = parse_lang(l)?;
            if !langs::can_hear(&p.code) {
                return Err(format!("{} cannot be transcribed (Whisper does not know it)", label(&p.code)));
            }
            Some(p.code)
        }
        None => None,
    };
    let heard = bia_speech::transcribe_file(&path, forced.as_deref(), model.filter(|m| !m.is_empty()))?;
    let mut out = heard.text();
    if out.is_empty() {
        out = format!("[no speech found in {:.1}s of audio]", heard.secs);
    }
    if let Some(l) = heard.new_lang {
        out.push('\n');
        out.push_str(&bia_speech::discovery_notice(&l, Some(file)));
    }
    Ok(out)
}

pub fn synthesize(text: &str, lang: &str, format: &str) -> Result<String, String> {
    let dir = bia_speech::paths::files().join("speech/out");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stamp = bia_speech::paths::stamp();
    if format == "wav" {
        let (samples, rate) = bia_speech::tts::synthesize(text, lang)?;
        let out = dir.join(format!("speech-{stamp}.wav"));
        write_wav(&out, &samples, rate)?;
        return Ok(format!("{} ({:.1}s)", rel(&out), samples.len() as f32 / rate.max(1) as f32));
    }
    let out = dir.join(format!("speech-{stamp}.ogg"));
    let (secs, _) = bia_speech::tts::voice_note(text, lang, &out)?;
    Ok(format!("{} ({secs:.1}s, OGG/Opus)", rel(&out)))
}

fn write_wav(path: &Path, samples: &[f32], rate: u32) -> Result<(), String> {
    let data: Vec<u8> = samples
        .iter()
        .flat_map(|s| ((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes())
        .collect();
    let mut h = Vec::with_capacity(44 + data.len());
    h.extend_from_slice(b"RIFF");
    h.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    h.extend_from_slice(b"WAVEfmt ");
    h.extend_from_slice(&16u32.to_le_bytes());
    h.extend_from_slice(&1u16.to_le_bytes());
    h.extend_from_slice(&1u16.to_le_bytes());
    h.extend_from_slice(&rate.to_le_bytes());
    h.extend_from_slice(&(rate * 2).to_le_bytes());
    h.extend_from_slice(&2u16.to_le_bytes());
    h.extend_from_slice(&16u16.to_le_bytes());
    h.extend_from_slice(b"data");
    h.extend_from_slice(&(data.len() as u32).to_le_bytes());
    h.extend_from_slice(&data);
    std::fs::write(path, h).map_err(|e| e.to_string())
}

pub struct SettingsArgs {
    pub bools: Vec<(&'static str, bool)>,
    pub speaker_threshold: Option<f32>,
    pub call_model: Option<String>,
    pub file_model: Option<String>,
}

pub fn settings(ctx: &Ctx, a: SettingsArgs) -> Result<String, String> {
    let cat = catalog::load()?;
    for m in [&a.call_model, &a.file_model].into_iter().flatten() {
        ensure_asr(&CtxReport(ctx), &cat, m)?;
    }
    if a.bools.iter().any(|(k, v)| *k == "speaker_check" && *v) {
        ensure_extra(&CtxReport(ctx), &cat, "speaker-campplus")?;
    }
    update(|r| {
        let s = &mut r.settings;
        for (k, v) in &a.bools {
            match *k {
                "denoise" => s.denoise = *v,
                "asr_denoised" => s.asr_denoised = *v,
                "split_pauses" => s.split_pauses = *v,
                "hallucination_filter" => s.hallucination_filter = *v,
                "discovery" => s.discovery = *v,
                "echo_filter" => s.echo_filter = *v,
                "speaker_check" => s.speaker_check = *v,
                _ => {}
            }
        }
        if let Some(t) = a.speaker_threshold {
            s.speaker_threshold = t.clamp(0.0, 1.0);
        }
        if let Some(m) = &a.call_model {
            r.asr_call = m.clone();
        }
        if let Some(m) = &a.file_model {
            r.asr_file = m.clone();
        }
        Ok(())
    })?;
    let r = registry::current();
    Ok(format!(
        "settings: {}; call_model={} file_model={}",
        serde_json::to_string(&r.settings).unwrap_or_default(),
        r.asr_call,
        r.asr_file
    ))
}

pub fn enroll(ctx: &Ctx, key: &str, files: &[String], remove: bool) -> Result<String, String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("key is required: telegram:<user id> or whatsapp:<phone>".into());
    }
    if remove {
        let k = key.to_string();
        let had = update(move |r| Ok(r.speakers.remove(&k).is_some()))?;
        return Ok(if had { format!("voice print for {key} removed") } else { format!("no voice print for {key}") });
    }
    if files.is_empty() {
        return Err("files: one or more recordings (voice notes) of this person, 3 s of speech or more".into());
    }
    let cat = catalog::load()?;
    ensure_extra(&CtxReport(ctx), &cat, "speaker-campplus")?;
    let mut sum: Vec<f32> = Vec::new();
    for f in files {
        let v = bia_speech::pipeline::voice_print(&files_path(f))?;
        if sum.is_empty() {
            sum = vec![0.0; v.len()];
        }
        for (s, x) in sum.iter_mut().zip(&v) {
            *s += x;
        }
    }
    let n = files.len() as f32;
    let avg: Vec<f32> = sum.into_iter().map(|x| x / n).collect();
    let k = key.to_string();
    update(move |r| {
        r.speakers.insert(k, avg);
        Ok(())
    })?;
    let on = registry::current().settings.speaker_check;
    Ok(format!(
        "voice print for {key} saved from {} recording(s).{}",
        files.len(),
        if on { "" } else { " The speaker check is off: lang_settings(speaker_check=true) turns it on." }
    ))
}

pub fn list() -> String {
    let r = registry::current();
    let mut out = format!("default language: {}\nlearned (heard and spoken):\n", label(&r.default_lang));
    for l in &r.enabled {
        let v = r.voice.get(l).map(String::as_str).unwrap_or("no voice: hear only");
        let typed = bia_speech::input::for_lang(&r, l).map(|i| format!(", typed with {i}")).unwrap_or_default();
        out.push_str(&format!("  {} — voice {v}{typed}\n", label(l)));
    }
    let speak_only: Vec<String> = r.voice.keys().filter(|l| !r.enabled.contains(l)).map(|l| label(l)).collect();
    if !speak_only.is_empty() {
        out.push_str(&format!("voice installed but disabled: {}\n", speak_only.join(", ")));
    }
    if !r.ignored.is_empty() {
        out.push_str(&format!("ignored (no new-language notices): {}\n", r.ignored.join(", ")));
    }
    out.push_str("installed voices:\n");
    for (id, v) in &r.voices {
        let size = if v.bundled { "bundled".to_string() } else { catalog::mb(store::dir_size(Path::new(&v.dir))) };
        out.push_str(&format!("  {id} ({}, {size})\n", v.langs.join("/")));
    }
    out.push_str("speech models:\n");
    for (id, a) in &r.asr {
        let size = if a.bundled { "bundled".to_string() } else { catalog::mb(store::dir_size(Path::new(&a.dir))) };
        let mut uses = vec![];
        if r.asr_call == *id {
            uses.push("calls");
        }
        if r.asr_file == *id {
            uses.push("voice notes");
        }
        out.push_str(&format!("  {id} ({size}){}\n", if uses.is_empty() { String::new() } else { format!(" — used for {}", uses.join(" and ")) }));
    }
    out.push_str(&format!("settings: {}\n", serde_json::to_string(&r.settings).unwrap_or_default()));
    if !r.speakers.is_empty() {
        out.push_str(&format!("voice prints: {}\n", r.speakers.keys().cloned().collect::<Vec<_>>().join(", ")));
    }
    out.push_str(&format!(
        "disk: {} in {}\n{}",
        catalog::mb(store::dir_size(&bia_speech::paths::models())),
        bia_speech::paths::models().display(),
        bia_speech::status()
    ));
    out
}

pub fn catalog_text(query: Option<&str>) -> Result<String, String> {
    let cat = catalog::load()?;
    let reg = registry::current();
    let q = query.map(str::trim).filter(|q| !q.is_empty());
    let mark = |code: &str| if reg.is_enabled(code) { " [learned]" } else { "" };
    if let Some(q) = q {
        let code = langs::parse(q).map(|l| l.code);
        let hits: Vec<(&String, &catalog::LangInfo)> = cat
            .languages
            .iter()
            .filter(|(c, i)| Some(c.as_str()) == code.as_deref() || i.name.to_lowercase().contains(&q.to_lowercase()))
            .collect();
        if hits.is_empty() {
            return Ok(format!("no language matches {q:?}"));
        }
        let mut out = String::new();
        for (c, i) in hits {
            out.push_str(&format!(
                "{c} {}{} — hear: {}; voices: {}\n",
                i.name,
                mark(c),
                if i.hear { "yes" } else { "no" },
                if i.voices.is_empty() { "none (cannot be spoken)".to_string() } else { i.voices.len().to_string() }
            ));
            for v in &i.voices {
                let cv = &cat.voices[v];
                let mut flags = vec![];
                if *v == i.default_voice {
                    flags.push("default");
                }
                if reg.voices.contains_key(v) {
                    flags.push("installed");
                }
                if reg.voice.get(c) == Some(v) {
                    flags.push("in use");
                }
                out.push_str(&format!(
                    "  {v} — {} {}, {}{}{}\n",
                    cv.locale,
                    cv.quality,
                    catalog::mb(cv.size),
                    if cv.note.is_empty() { String::new() } else { format!(", {}", cv.note) },
                    if flags.is_empty() { String::new() } else { format!(" [{}]", flags.join(", ")) }
                ));
            }
        }
        return Ok(out);
    }
    let mut out = String::from("languages (code name — hear/speak):\n");
    for (c, i) in &cat.languages {
        let speak = if i.voices.is_empty() { "hear only" } else if i.hear { "hear+speak" } else { "speak only" };
        out.push_str(&format!("{c} {} — {speak}{}\n", i.name, mark(c)));
    }
    out.push_str("speech models: ");
    out.push_str(
        &cat.asr
            .iter()
            .map(|(id, a)| format!("{id} ({}, {})", catalog::mb(a.size), a.note))
            .collect::<Vec<_>>()
            .join("; "),
    );
    out.push_str("\nlang_catalog(query=\"<code or name>\") lists the voices of a language.");
    Ok(out)
}
