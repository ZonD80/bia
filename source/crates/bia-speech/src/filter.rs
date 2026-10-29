//! Post-transcription filters: Whisper hallucinations and our own voice echoed back.

fn words(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect()
}

/// Phrases Whisper produces from noise or silence (it was trained on subtitled video).
const PHRASES: &[&str] = &[
    "thank you for watching",
    "thanks for watching",
    "please subscribe",
    "like and subscribe",
    "subscribe to my channel",
    "subtitles by the amara org community",
    "transcribed by",
    "продолжение следует",
    "спасибо за просмотр",
    "субтитры сделал",
    "субтитры создавал",
    "редактор субтитров",
    "untertitel im auftrag des zdf",
    "untertitelung des zdf",
    "sous titrage",
    "sottotitoli creati dalla comunità amara org",
    "ご視聴ありがとうございました",
    "字幕",
];

/// `text` without [..] and (..) parts; an unclosed bracket runs to the end.
fn strip_brackets(t: &str) -> String {
    let mut out = String::new();
    let mut close: Option<char> = None;
    for c in t.chars() {
        match (close, c) {
            (None, '[') => close = Some(']'),
            (None, '(') => close = Some(')'),
            (None, _) => out.push(c),
            (Some(x), _) if c == x => close = None,
            _ => {}
        }
    }
    out
}

/// Whisper's sound-event captions are not words: "[MUSIC PLAYING]", "[NON-ENGLISH
/// SPEECH]", "(speaking in foreign language)", "[INAUDIBLE]", "♪". Returns the text
/// without them and whether any were there. A bracket that holds ordinary words
/// ("call me (after eight)") stays.
pub fn strip_captions(text: &str) -> (String, bool) {
    const EVENTS: &[&str] = &[
        "music", "inaudible", "speech", "speaking", "foreign", "language", "applause", "laugh", "laughs",
        "laughing", "laughter", "noise", "silence", "blank_audio", "blank", "audio", "unintelligible",
        "crosstalk", "indistinct", "cough", "coughing", "sigh", "sighs", "breathing", "background",
        "playing", "upbeat", "sound", "sounds", "static", "beep", "клацание", "музыка", "неразборчиво",
        "смех", "musik", "música", "musique",
    ];
    let is_caption = |inner: &str| {
        let w = words(inner);
        let shouting = inner.chars().any(|c| c.is_alphabetic()) && !inner.chars().any(|c| c.is_lowercase());
        w.is_empty() || shouting || w.iter().any(|x| EVENTS.contains(&x.as_str()))
    };
    let mut out = String::new();
    let mut found = false;
    let mut rest = text;
    while let Some(i) = rest.find(['[', '(']) {
        let close = if rest[i..].starts_with('[') { ']' } else { ')' };
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        // an unclosed bracket runs to the end
        let (inner, next) = match after.find(close) {
            Some(j) => (&after[..j], &after[j + 1..]),
            None => (after, ""),
        };
        if is_caption(inner) {
            found = true;
        } else {
            out.push_str(&rest[i..rest.len() - next.len()]);
        }
        rest = next;
    }
    out.push_str(rest);
    let notes: String = out.chars().filter(|c| !matches!(c, '♪' | '♫')).collect();
    found |= notes.len() != out.len();
    let clean = notes.split_whitespace().collect::<Vec<_>>().join(" ");
    (clean.trim_matches(|c: char| c == '-' || c.is_whitespace()).to_string(), found)
}

/// Whisper decoder loops ("azért azért azért …", "and then and then and then …"): a
/// word or phrase of up to 3 words repeated 4+ times in a row is kept once. Returns
/// the text and whether a loop was cut.
pub fn collapse_loops(text: &str) -> (String, bool) {
    let toks: Vec<&str> = text.split_whitespace().collect();
    let norm: Vec<String> = toks
        .iter()
        .map(|t| t.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect())
        .collect();
    let mut keep = Vec::new();
    let mut cut = false;
    let mut i = 0;
    'scan: while i < toks.len() {
        for n in 1..=3 {
            let same = |k: usize| i + (k + 1) * n <= toks.len() && (0..n).all(|j| !norm[i + j].is_empty() && norm[i + j] == norm[i + k * n + j]);
            let mut reps = 1;
            while same(reps) {
                reps += 1;
            }
            if reps >= 4 {
                keep.extend_from_slice(&toks[i..i + n]);
                i += reps * n;
                cut = true;
                continue 'scan;
            }
        }
        keep.push(toks[i]);
        i += 1;
    }
    (keep.join(" "), cut)
}

/// True when `text` (from `secs` seconds of speech) looks like a hallucination.
pub fn is_hallucination(text: &str, secs: f32) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return true;
    }
    // Sound-event captions: [Music], (applause), ♪♪, *laughs*
    let stripped: String = t
        .chars()
        .filter(|c| !matches!(c, '♪' | '♫' | '*' | '.' | '…' | ' '))
        .collect();
    if stripped.is_empty() {
        return true;
    }
    // Nothing but captions in brackets ("[Music]", "(applause)", or cut off: "[Music").
    if strip_brackets(t).chars().all(|c| !c.is_alphanumeric()) {
        return true;
    }
    let w = words(t);
    if w.is_empty() {
        return true;
    }
    if w.len() == 1 && w[0] == "you" {
        return true;
    }
    let joined = w.join(" ");
    if PHRASES.iter().any(|p| joined.contains(p)) {
        return true;
    }
    // Decoder loops: the same few words over and over.
    if w.len() >= 12 {
        let mut uniq = w.clone();
        uniq.sort();
        uniq.dedup();
        if (uniq.len() as f32) < w.len() as f32 * 0.25 {
            return true;
        }
    }
    // Far more text than the audio could hold.
    secs > 0.0 && t.chars().count() as f32 / secs > 40.0
}

/// True when `heard` mostly repeats `spoken` (our TTS leaking back into the call).
pub fn is_echo(heard: &str, spoken: &str) -> bool {
    let h = words(heard);
    if h.len() < 2 {
        return false;
    }
    let s = words(spoken);
    let hit = h.iter().filter(|w| s.contains(w)).count();
    hit as f32 / h.len() as f32 >= 0.6
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hallucinations() {
        assert!(is_hallucination("Thank you for watching!", 2.0));
        assert!(is_hallucination(" you", 1.0));
        assert!(is_hallucination("[Music]", 3.0));
        assert!(is_hallucination("[Music", 3.0));
        assert!(is_hallucination("(applause) [laughs]", 3.0));
        assert!(!is_hallucination("Call me (after eight)", 2.0));
        assert!(is_hallucination("♪ ♪", 3.0));
        assert!(is_hallucination("Продолжение следует...", 2.0));
        assert!(is_hallucination(&"the the the the the the the the the the the the".to_string(), 5.0));
        assert!(is_hallucination(&"a".repeat(200), 1.0));
        assert!(!is_hallucination("Thank you.", 1.0));
        assert!(!is_hallucination("Can you call me back at eight?", 2.5));
        assert!(!is_hallucination("Привет, я буду в восемь", 2.0));
    }

    #[test]
    fn loops() {
        let (t, cut) = collapse_loops("és azért azért azért azért azért azért az");
        assert_eq!((t.as_str(), cut), ("és azért az", true));
        assert_eq!(collapse_loops("and then and then and then and then we left").0, "and then we left");
        assert_eq!(collapse_loops("no, no, no, I said").0, "no, no, no, I said");
        assert_eq!(collapse_loops("Привет, как дела?"), ("Привет, как дела?".into(), false));
    }

    #[test]
    fn captions() {
        assert_eq!(
            strip_captions("[NON-ENGLISH SPEECH] or This time it's much slower, right?"),
            ("or This time it's much slower, right?".into(), true)
        );
        assert_eq!(strip_captions("[INAUDIBLE] [MUSIC PLAYING]"), (String::new(), true));
        assert_eq!(strip_captions("(speaking in foreign language) ok"), ("ok".into(), true));
        assert_eq!(strip_captions("♪ la la ♪"), ("la la".into(), true));
        assert_eq!(strip_captions("Call me (after eight)"), ("Call me (after eight)".into(), false));
        assert_eq!(strip_captions("Привет, как дела?"), ("Привет, как дела?".into(), false));
        assert_eq!(strip_captions("hi [Music"), ("hi".into(), true));
    }

    #[test]
    fn echo() {
        assert!(is_echo("I will call you at eight", "Sure, I will call you at eight tomorrow."));
        assert!(!is_echo("no, nine please", "Sure, I will call you at eight tomorrow."));
        assert!(!is_echo("eight", "eight"));
    }
}
