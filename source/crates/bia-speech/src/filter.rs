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
    fn echo() {
        assert!(is_echo("I will call you at eight", "Sure, I will call you at eight tomorrow."));
        assert!(!is_echo("no, nine please", "Sure, I will call you at eight tomorrow."));
        assert!(!is_echo("eight", "eight"));
    }
}
