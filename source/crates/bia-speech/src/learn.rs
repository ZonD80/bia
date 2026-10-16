//! Installing languages, voices and models from the catalog (sha256-verified), shared
//! by the lang MCP (tools) and the speech users (auto-learn when a voice note or call
//! is in a language that is not learned yet).

use crate::catalog::{self, Catalog};
use crate::langs;
use crate::registry::{self, update};
use crate::store::{self, Want};

/// Voice-note model installed with the first non-English language.
pub const UPGRADE_ASR: &str = "whisper-base";

/// Progress and cancellation for long installs.
pub trait Report: Sync {
    fn progress(&self, _pct: u32, _msg: &str) {}
    fn log(&self, _msg: &str) {}
    fn cancelled(&self) -> bool {
        false
    }
}

/// Background installs: progress goes to the speech log only.
pub struct Quiet;

impl Report for Quiet {
    fn log(&self, msg: &str) {
        crate::diag::log("learn", msg);
    }
}

pub fn parse_lang(input: &str) -> Result<langs::Lang, String> {
    langs::parse(input).ok_or_else(|| format!("unknown language {input:?}; use a code such as en, ru, es, or an English name"))
}

pub fn label(code: &str) -> String {
    format!("{code} ({})", langs::name(code))
}

/// Download and register a catalog voice (no-op when installed).
pub fn ensure_voice(ctx: &dyn Report, cat: &Catalog, id: &str, pct: (u32, u32)) -> Result<bool, String> {
    if registry::current().voices.contains_key(id) {
        return Ok(false);
    }
    let cv = cat.voices.get(id).ok_or_else(|| format!("unknown voice {id}; see lang_catalog"))?;
    let models = crate::paths::models();
    let archive = models.join(format!(".{id}-{}.tar.bz2", crate::paths::stamp()));
    store::download(ctx, &Want { url: &cv.url, size: cv.size, sha256: &cv.sha256, label: id }, &archive, pct.0, pct.1)?;
    let dir = models.join(id);
    let res = store::unpack(&archive, &dir);
    let _ = std::fs::remove_file(&archive);
    res?;
    let v = store::resolve_voice(&dir, &cv.kind, &cv.langs, &cv.locale)?;
    // Load it and let it read a sentence now: a broken voice fails here, not in a call.
    let lang = cv.langs.first().cloned().unwrap_or_default();
    if let Err(e) = self_test(&v, &lang) {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(format!("voice {id} is broken: {e}"));
    }
    update(|r| {
        r.voices.insert(id.to_string(), v);
        Ok(())
    })?;
    Ok(true)
}

/// A short sentence in `code` (previews, the voice self-test).
pub fn sample_text(code: &str) -> String {
    match code {
        "ru" => "Привет! Я учусь говорить по-русски, и мне это очень нравится.",
        "uk" => "Привіт! Я вчуся говорити українською, і мені це дуже подобається.",
        "de" => "Hallo! Ich lerne gerade Deutsch, und es macht mir viel Spaß.",
        "es" => "¡Hola! Estoy aprendiendo español y me gusta mucho.",
        "fr" => "Bonjour ! J'apprends le français et j'aime beaucoup ça.",
        "it" => "Ciao! Sto imparando l'italiano e mi piace molto.",
        "pt" => "Olá! Estou aprendendo português e gosto muito.",
        "cs" => "Ahoj! Učím se česky a moc mě to baví.",
        "pl" => "Cześć! Uczę się polskiego i bardzo mi się to podoba.",
        "nl" => "Hallo! Ik leer Nederlands en ik vind het erg leuk.",
        "tr" => "Merhaba! Türkçe öğreniyorum ve bunu çok seviyorum.",
        "zh" => "你好！我正在学习中文，我很喜欢。",
        _ => "Hello! I am learning to speak, and I like it very much.",
    }
    .to_string()
}

/// Load a voice and let it read the sample sentence. Some converted models load but
/// drop most of every sentence (e.g. a text-trained Piper voice exported as espeak);
/// that shows up as far too little audio for the text.
pub fn self_test(v: &crate::registry::Voice, lang: &str) -> Result<f32, String> {
    let eng = crate::tts::Engine::load(v).map_err(|e| format!("does not load: {e}"))?;
    let text = sample_text(lang);
    let (mut n, rate) = (0usize, eng.sample_rate.max(1));
    eng.generate(&text, &mut |c, _| {
        n += c.len();
        true
    })?;
    let secs = n as f32 / rate as f32;
    let chars = text.chars().filter(|c| c.is_alphanumeric()).count() as f32;
    let per_char = if matches!(lang, "zh" | "ja" | "ko" | "yue") { 0.12 } else { 0.035 };
    if secs < chars * per_char {
        return Err(format!("it read {chars} letters in {secs:.1}s (it cuts speech short)"));
    }
    Ok(secs)
}

pub fn ensure_asr(ctx: &dyn Report, cat: &Catalog, id: &str) -> Result<bool, String> {
    if registry::current().asr.contains_key(id) {
        return Ok(false);
    }
    let ca = cat.asr.get(id).ok_or_else(|| format!("unknown speech model {id}; known: {}", cat.asr.keys().cloned().collect::<Vec<_>>().join(", ")))?;
    let models = crate::paths::models();
    let archive = models.join(format!(".{id}-{}.tar.bz2", crate::paths::stamp()));
    store::download(ctx, &Want { url: &ca.url, size: ca.size, sha256: &ca.sha256, label: id }, &archive, 0, 90)?;
    let dir = models.join(id);
    let res = store::unpack(&archive, &dir);
    let _ = std::fs::remove_file(&archive);
    res?;
    let a = store::resolve_whisper(&dir)?;
    update(|r| {
        r.asr.insert(id.to_string(), a);
        Ok(())
    })?;
    Ok(true)
}

pub fn ensure_extra(ctx: &dyn Report, cat: &Catalog, id: &str) -> Result<(), String> {
    if registry::current().extra(id).is_some() {
        return Ok(());
    }
    let f = cat.extras.get(id).ok_or_else(|| format!("unknown model {id}"))?;
    let file = f.url.rsplit('/').next().unwrap_or(id).replace("%2B", "+");
    let dest = crate::paths::models().join(id).join(file);
    store::download(ctx, &Want { url: &f.url, size: f.size, sha256: &f.sha256, label: id }, &dest, 0, 100)?;
    update(|r| {
        r.extras.insert(id.to_string(), dest.to_string_lossy().into_owned());
        Ok(())
    })
}

/// Learn a language: enable it for hearing and install its voice.
pub fn install(ctx: &dyn Report, lang: &str, voice: Option<&str>) -> Result<String, String> {
    let l = parse_lang(lang)?;
    let cat = catalog::load()?;
    let info = cat.languages.get(&l.code).cloned().unwrap_or_default();
    let reg = registry::current();
    let voice_id = match voice.filter(|v| !v.is_empty()) {
        Some(v) => {
            let cv = cat.voices.get(v).ok_or_else(|| format!("unknown voice {v}; lang_catalog(query=\"{}\") lists them", l.code))?;
            if !cv.langs.contains(&l.code) {
                return Err(format!("voice {v} does not speak {}", label(&l.code)));
            }
            Some(v.to_string())
        }
        None => {
            let exact = l.locale().and_then(|loc| {
                info.voices
                    .iter()
                    .filter(|id| cat.voices.get(*id).is_some_and(|v| v.locale == loc && v.kind == "vits"))
                    .min_by_key(|id| quality_rank(&cat.voices[*id].quality))
                    .cloned()
            });
            exact
                .or_else(|| reg.voice.get(&l.code).cloned())
                .or_else(|| Some(info.default_voice.clone()).filter(|v| !v.is_empty()))
        }
    };
    let can_hear = langs::can_hear(&l.code);
    if !can_hear && voice_id.is_none() {
        return Err(format!("{} is not in the catalog: it can be neither heard nor spoken", label(&l.code)));
    }
    let mut downloaded = None;
    let mut skipped: Vec<String> = Vec::new();
    let mut voice_id = voice_id;
    if let Some(first) = voice_id.clone() {
        // bia chose the voice: if it turns out broken, try the language's other voices
        let mut candidates = vec![first.clone()];
        if voice.filter(|v| !v.is_empty()).is_none() {
            candidates.extend(info.voices.iter().filter(|v| **v != first).cloned());
        }
        voice_id = None;
        for v in candidates {
            ctx.progress(0, &format!("learning {}: voice {v}", label(&l.code)));
            match ensure_voice(ctx, &cat, &v, (0, 95)) {
                Ok(fresh) => {
                    if fresh {
                        downloaded = cat.voices.get(&v).map(|cv| catalog::mb(cv.size));
                    }
                    voice_id = Some(v);
                    break;
                }
                Err(e) if e.contains("is broken") => {
                    ctx.log(&e);
                    skipped.push(e);
                }
                Err(e) => return Err(e),
            }
        }
        if voice_id.is_none() && !skipped.is_empty() && !can_hear {
            return Err(skipped.join("; "));
        }
    }
    let code = l.code.clone();
    let vid = voice_id.clone();
    update(move |r| {
        if !r.enabled.contains(&code) && can_hear {
            r.enabled.push(code.clone());
        }
        r.ignored.retain(|x| *x != code);
        if let Some(v) = vid {
            r.voice.insert(code.clone(), v);
        }
        Ok(())
    })?;
    ctx.progress(100, &format!("{} ready", label(&l.code)));
    let hear = if can_hear { "hear: yes (Whisper)" } else { "hear: no (Whisper does not know it)" };
    let speak = match (&voice_id, downloaded) {
        (Some(v), Some(size)) => format!("speak: yes, voice {v} (downloaded {size})"),
        (Some(v), None) => format!("speak: yes, voice {v}"),
        (None, _) => "speak: no voice exists for this language; you can understand it but must answer in another language".into(),
    };
    let mut out = format!("learned {}: {hear}; {speak}.", label(&l.code));
    if !skipped.is_empty() {
        out.push_str(&format!(" Skipped: {}.", skipped.join("; ")));
    }
    // More than one language: voice notes get whisper-base (tiny misses words outside
    // English); calls stay on the fast bundled tiny.
    let reg = registry::current();
    if can_hear && l.code != "en" && reg.asr_file == registry::BUNDLED_ASR && !reg.asr.contains_key(UPGRADE_ASR) {
        ctx.progress(0, &format!("voice notes: {UPGRADE_ASR} for better multilingual transcripts"));
        match ensure_asr(ctx, &cat, UPGRADE_ASR).and_then(|_| {
            update(|r| {
                r.asr_file = UPGRADE_ASR.to_string();
                Ok(())
            })
        }) {
            Ok(()) => out.push_str(&format!(" Voice notes now use {UPGRADE_ASR} (more accurate outside English; calls keep whisper-tiny).")),
            Err(e) => out.push_str(&format!(" (Could not install {UPGRADE_ASR} for voice notes: {e}; voice notes keep whisper-tiny.)")),
        }
    }
    Ok(out)
}

pub fn quality_rank(q: &str) -> u8 {
    match q {
        "medium" => 0,
        "low" => 1,
        "high" => 2,
        _ => 3,
    }
}


/// Enable a language for hearing right away (no download: Whisper knows it).
/// Returns true when it was not enabled before.
pub fn enable_hearing(code: &str) -> Result<bool, String> {
    if !langs::can_hear(code) {
        return Ok(false);
    }
    update(|r| {
        r.ignored.retain(|x| x != code);
        if r.enabled.iter().any(|x| x == code) {
            return Ok(false);
        }
        r.enabled.push(code.to_string());
        Ok(true)
    })
}

/// A new language was heard (voice note or call): enable hearing now and install its
/// voice (and the better voice-note model) in the background. The returned notice
/// tells the model what happened.
pub fn auto_learn(code: &str) -> Option<String> {
    let reg = registry::current();
    if reg.ignored.iter().any(|x| x == code) || !reg.settings.discovery {
        return None;
    }
    let newly = enable_hearing(code).ok()?;
    let name = langs::name(code);
    let has_voice = reg.voice.contains_key(code);
    if !has_voice {
        let c = code.to_string();
        std::thread::spawn(move || {
            crate::diag::log("learn", format!("installing {c} in the background"));
            match install(&Quiet, &c, None) {
                Ok(s) => crate::diag::log("learn", s),
                Err(e) => crate::diag::log("learn", format!("{c}: {e}")),
            }
        });
    }
    if !newly && has_voice {
        return None;
    }
    Some(format!(
        "[new language: {code} ({name}) — learned it automatically: speech in {name} is now transcribed{}. Tell the user (in their language) that you have learned {name}. If they do not want it, lang_disable(lang=\"{code}\").]",
        if has_voice { "" } else { "; its voice (to speak it) is downloading now, a few seconds to a minute" }
    ))
}

/// The owner's language (chosen when the stick was flashed): the default for speech,
/// learned for hearing at once, and its voice installed in the background, retried
/// every minute for a day while there is no network yet.
pub fn adopt_home(lang: &str) -> Result<String, String> {
    let l = parse_lang(lang)?;
    let code = l.code.clone();
    let hear = langs::can_hear(&code);
    update(|r| {
        r.default_lang = code.clone();
        if hear && !r.enabled.contains(&code) {
            r.enabled.push(code.clone());
        }
        r.ignored.retain(|x| *x != code);
        Ok(())
    })?;
    if registry::current().voice.contains_key(&code) || code == "en" {
        return Ok(format!("home language {}", label(&code)));
    }
    install_in_background(vec![lang.trim().to_string()]);
    Ok(format!("home language {}; its voice is downloading in the background", label(&code)))
}

/// Install languages one after another in a background thread, retrying each every minute
/// for a day (the network may not be up yet). Progress goes to the speech log.
fn install_in_background(langs: Vec<String>) {
    if langs.is_empty() {
        return;
    }
    std::thread::spawn(move || {
        for want in langs {
            for _ in 0..24 * 60 {
                match install(&Quiet, &want, None) {
                    Ok(s) => {
                        crate::diag::log("learn", s);
                        break;
                    }
                    Err(e) => crate::diag::log("learn", format!("{want}: {e}; retrying in 1 min")),
                }
                std::thread::sleep(std::time::Duration::from_secs(60));
            }
        }
    });
}

/// Languages whose voice should be on disk but is not: the home language's voice when an
/// interrupted download (power cut, fold, flight) never registered it, and registered voices
/// whose files are gone.
pub fn unfinished(r: &registry::Registry, has_voice: impl Fn(&str) -> bool) -> Vec<String> {
    let mut todo: Vec<String> = r
        .voice
        .iter()
        .filter(|(_, id)| {
            r.voices
                .get(*id)
                .is_none_or(|v| !v.bundled && !std::path::Path::new(&v.dir).is_dir())
        })
        .map(|(l, _)| l.clone())
        .collect();
    let home = &r.default_lang;
    if !home.is_empty() && home != "en" && !r.voice.contains_key(home) && has_voice(home) && !todo.contains(home) {
        todo.insert(0, home.clone());
    }
    todo
}

/// At boot (also the first after an unfold or a landing): finish downloads that were cut
/// off, so learned languages speak again without being learned anew.
pub fn resume() -> Option<String> {
    let cat = catalog::load().ok();
    let todo = unfinished(&registry::current(), |l| {
        cat.as_ref()
            .and_then(|c| c.languages.get(l))
            .is_some_and(|i| !i.voices.is_empty() || !i.default_voice.is_empty())
    });
    if todo.is_empty() {
        return None;
    }
    let msg = format!(
        "speech: finishing voices in the background: {}",
        todo.iter().map(|l| label(l)).collect::<Vec<_>>().join(", ")
    );
    install_in_background(todo);
    Some(msg)
}

#[cfg(test)]
mod resume_tests {
    use super::*;
    use crate::registry::{Registry, Voice};

    #[test]
    fn finds_cut_off_voices() {
        let d = std::env::temp_dir().join(format!("bia-resume-{}", std::process::id()));
        std::fs::create_dir_all(d.join("de")).unwrap();
        let mut r = Registry::default();
        let voice = |dir: &std::path::Path, bundled| Voice { dir: dir.display().to_string(), bundled, ..Default::default() };
        r.voices.insert("v-de".into(), voice(&d.join("de"), false));
        r.voices.insert("v-fr".into(), voice(&d.join("fr-gone"), false));
        r.voices.insert("v-en".into(), voice(std::path::Path::new("/nonexistent"), true));
        for (l, v) in [("de", "v-de"), ("fr", "v-fr"), ("en", "v-en"), ("es", "v-unknown")] {
            r.voice.insert(l.into(), v.into());
        }
        r.default_lang = "ru".into();
        // ru first (the home language, never registered), then the registered ones whose files are gone
        assert_eq!(unfinished(&r, |_| true), ["ru", "es", "fr"]);
        // a text-only home language has nothing to download
        assert_eq!(unfinished(&r, |_| false), ["es", "fr"]);
        r.default_lang = "en".into();
        assert_eq!(unfinished(&r, |_| true), ["es", "fr"]);
        let _ = std::fs::remove_dir_all(&d);
    }
}
