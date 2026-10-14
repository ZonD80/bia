//! Language codes: Whisper's table (what can be heard) plus normalisation of what the
//! model passes as `lang` ("ru", "ru-RU", "ru_RU", "Russian").

pub const WHISPER: &[(&str, &str)] = &[
    ("en", "English"), ("zh", "Chinese"), ("de", "German"), ("es", "Spanish"), ("ru", "Russian"),
    ("ko", "Korean"), ("fr", "French"), ("ja", "Japanese"), ("pt", "Portuguese"), ("tr", "Turkish"),
    ("pl", "Polish"), ("ca", "Catalan"), ("nl", "Dutch"), ("ar", "Arabic"), ("sv", "Swedish"),
    ("it", "Italian"), ("id", "Indonesian"), ("hi", "Hindi"), ("fi", "Finnish"), ("vi", "Vietnamese"),
    ("he", "Hebrew"), ("uk", "Ukrainian"), ("el", "Greek"), ("ms", "Malay"), ("cs", "Czech"),
    ("ro", "Romanian"), ("da", "Danish"), ("hu", "Hungarian"), ("ta", "Tamil"), ("no", "Norwegian"),
    ("th", "Thai"), ("ur", "Urdu"), ("hr", "Croatian"), ("bg", "Bulgarian"), ("lt", "Lithuanian"),
    ("la", "Latin"), ("mi", "Maori"), ("ml", "Malayalam"), ("cy", "Welsh"), ("sk", "Slovak"),
    ("te", "Telugu"), ("fa", "Persian"), ("lv", "Latvian"), ("bn", "Bengali"), ("sr", "Serbian"),
    ("az", "Azerbaijani"), ("sl", "Slovenian"), ("kn", "Kannada"), ("et", "Estonian"),
    ("mk", "Macedonian"), ("br", "Breton"), ("eu", "Basque"), ("is", "Icelandic"), ("hy", "Armenian"),
    ("ne", "Nepali"), ("mn", "Mongolian"), ("bs", "Bosnian"), ("kk", "Kazakh"), ("sq", "Albanian"),
    ("sw", "Swahili"), ("gl", "Galician"), ("mr", "Marathi"), ("pa", "Punjabi"), ("si", "Sinhala"),
    ("km", "Khmer"), ("sn", "Shona"), ("yo", "Yoruba"), ("so", "Somali"), ("af", "Afrikaans"),
    ("oc", "Occitan"), ("ka", "Georgian"), ("be", "Belarusian"), ("tg", "Tajik"), ("sd", "Sindhi"),
    ("gu", "Gujarati"), ("am", "Amharic"), ("yi", "Yiddish"), ("lo", "Lao"), ("uz", "Uzbek"),
    ("fo", "Faroese"), ("ht", "Haitian Creole"), ("ps", "Pashto"), ("tk", "Turkmen"),
    ("nn", "Nynorsk"), ("mt", "Maltese"), ("sa", "Sanskrit"), ("lb", "Luxembourgish"),
    ("my", "Myanmar"), ("bo", "Tibetan"), ("tl", "Tagalog"), ("mg", "Malagasy"), ("as", "Assamese"),
    ("tt", "Tatar"), ("haw", "Hawaiian"), ("ln", "Lingala"), ("ha", "Hausa"), ("ba", "Bashkir"),
    ("jw", "Javanese"), ("su", "Sundanese"),
];

const EXTRA: &[(&str, &str)] = &[("ku", "Kurdish")];

pub fn name(code: &str) -> String {
    WHISPER
        .iter()
        .chain(EXTRA)
        .find(|(c, _)| *c == code)
        .map(|(_, n)| n.to_string())
        .unwrap_or_else(|| code.to_string())
}

pub fn can_hear(code: &str) -> bool {
    WHISPER.iter().any(|(c, _)| *c == code)
}

/// A requested language: base code plus an optional region ("es-MX" → es, Some("MX")).
#[derive(Clone, Debug, PartialEq)]
pub struct Lang {
    pub code: String,
    pub region: Option<String>,
}

impl Lang {
    pub fn locale(&self) -> Option<String> {
        self.region.as_ref().map(|r| format!("{}_{r}", self.code))
    }
}

/// Parse "ru", "RU", "ru-RU", "ru_RU" or an English name ("Russian").
pub fn parse(input: &str) -> Option<Lang> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }
    let (base, region) = match s.split_once(['-', '_']) {
        Some((b, r)) if !r.is_empty() => (b, Some(r.to_ascii_uppercase())),
        _ => (s, None),
    };
    let lower = base.to_ascii_lowercase();
    let known = |c: &str| WHISPER.iter().chain(EXTRA).any(|(k, _)| *k == c);
    if known(&lower) {
        return Some(Lang { code: lower, region });
    }
    WHISPER
        .iter()
        .chain(EXTRA)
        .find(|(_, n)| n.eq_ignore_ascii_case(s))
        .map(|(c, _)| Lang { code: c.to_string(), region: None })
}

/// A piece of text to speak in one language.
#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub lang: String,
    pub text: String,
}

/// Split `text` by inline `<lang=xx>…</lang>` spans; everything else is in `base`.
/// Unknown span codes are an error, so a typo never silently falls back.
pub fn spans(text: &str, base: &str) -> Result<Vec<Span>, String> {
    let mut out: Vec<Span> = Vec::new();
    let mut push = |lang: &str, t: &str| {
        let t = t.trim();
        if t.is_empty() {
            return;
        }
        match out.last_mut() {
            Some(last) if last.lang == lang => {
                last.text.push(' ');
                last.text.push_str(t);
            }
            _ => out.push(Span { lang: lang.to_string(), text: t.to_string() }),
        }
    };
    let mut rest = text;
    while let Some(i) = rest.find("<lang=") {
        push(base, &rest[..i]);
        let after = &rest[i + 6..];
        let Some(close) = after.find('>') else {
            return Err("unclosed <lang=…> tag".into());
        };
        let code = after[..close].trim().trim_matches(['"', '\'']);
        let lang = parse(code).ok_or_else(|| format!("unknown language in <lang={code}>"))?;
        let body = &after[close + 1..];
        let end = body.find("</lang>").ok_or("missing </lang>")?;
        push(&lang.code, &body[..end]);
        rest = &body[end + 7..];
    }
    push(base, rest);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_forms() {
        assert_eq!(parse("ru").unwrap().code, "ru");
        assert_eq!(parse("RU").unwrap().code, "ru");
        let l = parse("es-mx").unwrap();
        assert_eq!((l.code.as_str(), l.locale()), ("es", Some("es_MX".into())));
        assert_eq!(parse("pt_BR").unwrap().locale(), Some("pt_BR".into()));
        assert_eq!(parse("Russian").unwrap().code, "ru");
        assert_eq!(parse("ku").unwrap().code, "ku");
        assert!(parse("xx").is_none());
        assert!(parse("").is_none());
    }

    #[test]
    fn spans_split_and_merge() {
        let s = spans("Book it at <lang=fr>Le Petit Bistro</lang> tomorrow", "en").unwrap();
        assert_eq!(
            s,
            vec![
                Span { lang: "en".into(), text: "Book it at".into() },
                Span { lang: "fr".into(), text: "Le Petit Bistro".into() },
                Span { lang: "en".into(), text: "tomorrow".into() },
            ]
        );
        let s = spans("<lang=en>hello</lang> world", "en").unwrap();
        assert_eq!(s, vec![Span { lang: "en".into(), text: "hello world".into() }]);
        assert!(spans("<lang=zz>x</lang>", "en").is_err());
        assert!(spans("<lang=fr>x", "en").is_err());
    }
}
