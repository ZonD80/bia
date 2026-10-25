//! The model writes markdown (`say` says so); chat apps do not render it. This turns it
//! into what each app understands: plain text plus formatting spans for Telegram
//! (message entities, offsets in UTF-16 units) and WhatsApp's own `*bold*` syntax.
//!
//! Supported: `**bold**`/`__bold__`, `*italic*`/`_italic_` (not inside words, so
//! `file_name` stays as it is), `~~strike~~`, `` `code` ``, fenced code blocks,
//! `[text](url)`, `# headings` (bold), `> quotes`, `-`/`*`/`+` lists (•), rules and
//! `|` tables (kept monospaced), backslash escapes. Anything else stays as written.

#[derive(Clone, Debug, PartialEq)]
pub enum Style {
    Bold,
    Italic,
    Strike,
    Code,
    /// A code block with its language (may be empty).
    Pre(String),
    Link(String),
    Quote,
}

/// A formatted range of [`Rich::text`]: `offset` and `len` count UTF-16 code units.
#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub style: Style,
    pub offset: usize,
    pub len: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rich {
    pub text: String,
    pub spans: Vec<Span>,
}

/// Plain text and formatting spans (Telegram message entities).
pub fn rich(md: &str) -> Rich {
    let mut s = RichSink::default();
    render(md.trim(), &mut s);
    let mut r = Rich { text: s.text, spans: s.spans };
    // Telegram trims trailing whitespace; keep every span inside what is left
    let keep = r.text.trim_end().len();
    r.text.truncate(keep);
    let end = utf16_len(&r.text);
    r.spans.retain_mut(|sp| {
        sp.len = sp.len.min(end.saturating_sub(sp.offset));
        sp.len > 0
    });
    r.spans.sort_by_key(|sp| sp.offset);
    r
}

/// Plain text for apps without formatting (Messenger, Instagram): the markup goes, and a
/// link keeps its address after its text (`text (url)`) unless the text is the address.
pub fn plain(md: &str) -> String {
    let r = rich(md);
    let mut links: Vec<(usize, String)> = r
        .spans
        .iter()
        .filter_map(|sp| match &sp.style {
            Style::Link(url) => Some((sp.offset + sp.len, url.clone())),
            _ => None,
        })
        .collect();
    let mut out = r.text.clone();
    // insert from the end so earlier offsets stay valid
    links.sort_by(|a, b| b.0.cmp(&a.0));
    for (end16, url) in links {
        let Some(at) = byte_at_utf16(&r.text, end16) else { continue };
        let start16 = r.spans.iter().find(|sp| sp.offset + sp.len == end16 && matches!(&sp.style, Style::Link(u) if *u == url)).map(|sp| sp.offset);
        let shown = start16.and_then(|st| byte_at_utf16(&r.text, st)).map(|st| &r.text[st..at]).unwrap_or("");
        if shown.trim() != url.trim() {
            out.insert_str(at, &format!(" ({url})"));
        }
    }
    out
}

/// Byte index of a UTF-16 offset in `s`.
fn byte_at_utf16(s: &str, off16: usize) -> Option<usize> {
    let mut n = 0;
    for (i, ch) in s.char_indices() {
        if n == off16 {
            return Some(i);
        }
        n += ch.len_utf16();
    }
    (n == off16).then_some(s.len())
}

/// WhatsApp's formatting: `*bold*`, `_italic_`, `~strike~`, `` `code` ``, ```` ``` ```` blocks,
/// `> ` quotes; links become `text (url)`.
pub fn to_whatsapp(md: &str) -> String {
    let mut s = WaSink::default();
    render(md.trim(), &mut s);
    s.out.trim_end().to_string()
}

trait Sink {
    fn text(&mut self, t: &str);
    fn open(&mut self, st: &Style);
    fn close(&mut self, st: &Style);
}

fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

#[derive(Default)]
struct RichSink {
    text: String,
    at: usize,
    open: Vec<(Style, usize)>,
    spans: Vec<Span>,
}

impl Sink for RichSink {
    fn text(&mut self, t: &str) {
        self.text.push_str(t);
        self.at += utf16_len(t);
    }
    fn open(&mut self, st: &Style) {
        self.open.push((st.clone(), self.at));
    }
    fn close(&mut self, _: &Style) {
        if let Some((style, offset)) = self.open.pop() {
            if self.at > offset {
                self.spans.push(Span { style, offset, len: self.at - offset });
            }
        }
    }
}

#[derive(Default)]
struct WaSink {
    out: String,
    quote: bool,
    /// where each open link's text starts in `out`
    links: Vec<usize>,
}

impl Sink for WaSink {
    fn text(&mut self, t: &str) {
        if self.quote {
            self.out.push_str(&t.replace('\n', "\n> "));
        } else {
            self.out.push_str(t);
        }
    }
    fn open(&mut self, st: &Style) {
        match st {
            Style::Bold => self.out.push('*'),
            Style::Italic => self.out.push('_'),
            Style::Strike => self.out.push('~'),
            Style::Code => self.out.push('`'),
            Style::Pre(_) => self.out.push_str("```\n"),
            Style::Link(_) => self.links.push(self.out.len()),
            Style::Quote => {
                self.out.push_str("> ");
                self.quote = true;
            }
        }
    }
    fn close(&mut self, st: &Style) {
        match st {
            Style::Bold => self.out.push('*'),
            Style::Italic => self.out.push('_'),
            Style::Strike => self.out.push('~'),
            Style::Code => self.out.push('`'),
            Style::Pre(_) => self.out.push_str("\n```"),
            Style::Link(url) => {
                let from = self.links.pop().unwrap_or(self.out.len());
                if self.out[from..].trim() != url.as_str() {
                    self.out.push_str(&format!(" ({url})"));
                }
            }
            Style::Quote => self.quote = false,
        }
    }
}

// ---- blocks ----

fn fence_of(t: &str) -> Option<&'static str> {
    if t.starts_with("```") {
        Some("```")
    } else if t.starts_with("~~~") {
        Some("~~~")
    } else {
        None
    }
}

fn is_rule(t: &str) -> bool {
    let c: Vec<char> = t.chars().filter(|c| !c.is_whitespace()).collect();
    c.len() >= 3 && ['-', '*', '_'].iter().any(|d| c.iter().all(|x| x == d))
}

fn heading(t: &str) -> Option<&str> {
    let n = t.chars().take_while(|&c| c == '#').count();
    if (1..=6).contains(&n) && t[n..].starts_with(' ') {
        Some(t[n..].trim().trim_end_matches('#').trim_end())
    } else {
        None
    }
}

fn render<S: Sink>(md: &str, s: &mut S) {
    let lines: Vec<&str> = md.lines().collect();
    let mut i = 0;
    let mut first = true;
    let mut newline = |s: &mut S| {
        if !first {
            s.text("\n");
        }
        first = false;
    };
    while i < lines.len() {
        let l = lines[i];
        let t = l.trim_start();
        if let Some(fence) = fence_of(t) {
            let lang = t[3..].trim().to_string();
            let mut j = i + 1;
            while j < lines.len() && !lines[j].trim_start().starts_with(fence) {
                j += 1;
            }
            let body = lines[i + 1..j.min(lines.len())].join("\n");
            newline(s);
            let st = Style::Pre(lang);
            s.open(&st);
            s.text(&body);
            s.close(&st);
            i = j + 1;
            continue;
        }
        if t.starts_with('|') {
            let mut j = i;
            while j < lines.len() && lines[j].trim_start().starts_with('|') {
                j += 1;
            }
            let body: Vec<&str> = lines[i..j].iter().map(|x| x.trim()).collect();
            newline(s);
            let st = Style::Pre(String::new());
            s.open(&st);
            s.text(&body.join("\n"));
            s.close(&st);
            i = j;
            continue;
        }
        if t.starts_with('>') {
            let mut j = i;
            while j < lines.len() && lines[j].trim_start().starts_with('>') {
                j += 1;
            }
            newline(s);
            s.open(&Style::Quote);
            for (k, q) in lines[i..j].iter().enumerate() {
                if k > 0 {
                    s.text("\n");
                }
                let q = q.trim_start()[1..].strip_prefix(' ').unwrap_or(&q.trim_start()[1..]);
                inline(&q.chars().collect::<Vec<_>>(), s);
            }
            s.close(&Style::Quote);
            i = j;
            continue;
        }
        newline(s);
        if let Some(h) = heading(t) {
            s.open(&Style::Bold);
            inline(&h.chars().collect::<Vec<_>>(), s);
            s.close(&Style::Bold);
        } else if is_rule(t) {
            s.text("———");
        } else if let Some(item) = ["- ", "* ", "+ "].iter().find_map(|b| t.strip_prefix(b)) {
            s.text(&format!("{}• ", &l[..l.len() - t.len()]));
            inline(&item.chars().collect::<Vec<_>>(), s);
        } else {
            inline(&l.chars().collect::<Vec<_>>(), s);
        }
        i += 1;
    }
}

// ---- inline ----

fn alnum_at(c: &[char], i: Option<usize>) -> bool {
    i.and_then(|i| c.get(i)).is_some_and(|x| x.is_alphanumeric())
}

fn space_at(c: &[char], i: usize) -> bool {
    c.get(i).is_none_or(|x| x.is_whitespace())
}

/// A run of `n` delimiters `d` at `i` (exactly `n`, or 3 for `***` = bold + italic).
fn run_at(c: &[char], i: usize, d: char, n: usize) -> bool {
    run_len(c, i, d).is_some_and(|m| m == n || m == 3)
}

/// The length of the run of `d` that starts at `i` (None inside or outside one).
fn run_len(c: &[char], i: usize, d: char) -> Option<usize> {
    if c.get(i) != Some(&d) || (i > 0 && c[i - 1] == d) {
        return None;
    }
    Some(c[i..].iter().take_while(|&&x| x == d).count())
}

/// Where the closing `n` × `d` for an opener ending before `from` starts: the end of a
/// `***` run, so the inner `*` closes first.
fn closer(c: &[char], from: usize, d: char, n: usize) -> Option<usize> {
    (from + 1..c.len()).find_map(|k| {
        let m = run_len(c, k, d).filter(|&m| m == n || m == 3)?;
        (!c[k - 1].is_whitespace() && (d != '_' || !alnum_at(c, Some(k + m)))).then_some(k + m - n)
    })
}

fn opener_ok(c: &[char], i: usize, d: char, n: usize) -> bool {
    run_at(c, i, d, n) && !space_at(c, i + n) && (d != '_' || !alnum_at(c, i.checked_sub(1)))
}

fn link_at(c: &[char], i: usize) -> Option<(usize, String, usize)> {
    let mut depth = 0;
    let mut close = None;
    for (k, &x) in c.iter().enumerate().skip(i) {
        match x {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(k);
                    break;
                }
            }
            _ => {}
        }
    }
    let close = close?;
    if c.get(close + 1) != Some(&'(') {
        return None;
    }
    let end = (close + 2..c.len()).find(|&k| c[k] == ')')?;
    let raw: String = c[close + 2..end].iter().collect();
    let url = raw.split_whitespace().next().unwrap_or("").trim_matches(['<', '>']).to_string();
    let ok = url.contains("://") || url.starts_with("mailto:") || url.starts_with("tel:");
    (ok && close > i + 1).then_some((close, url, end))
}

fn inline<S: Sink>(c: &[char], s: &mut S) {
    let mut lit = String::new();
    let flush = |lit: &mut String, s: &mut S| {
        if !lit.is_empty() {
            s.text(lit);
            lit.clear();
        }
    };
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        if ch == '\\' && c.get(i + 1).is_some_and(|x| x.is_ascii_punctuation()) {
            lit.push(c[i + 1]);
            i += 2;
            continue;
        }
        if ch == '`' {
            let n = c[i..].iter().take_while(|&&x| x == '`').count();
            if let Some(j) = (i + n..c.len()).find(|&k| run_len(c, k, '`') == Some(n)) {
                flush(&mut lit, s);
                let body: String = c[i + n..j].iter().collect();
                s.open(&Style::Code);
                s.text(&body);
                s.close(&Style::Code);
                i = j + n;
                continue;
            }
            lit.extend(&c[i..i + n]);
            i += n;
            continue;
        }
        if ch == '[' {
            if let Some((close, url, end)) = link_at(c, i) {
                flush(&mut lit, s);
                let st = Style::Link(url);
                s.open(&st);
                inline(&c[i + 1..close], s);
                s.close(&st);
                i = end + 1;
                continue;
            }
        }
        let styled = [('*', 2, Style::Bold), ('_', 2, Style::Bold), ('~', 2, Style::Strike), ('*', 1, Style::Italic), ('_', 1, Style::Italic)]
            .into_iter()
            .find_map(|(d, n, st)| {
                (ch == d && opener_ok(c, i, d, n)).then(|| closer(c, i + n, d, n).map(|j| (n, j, st))).flatten()
            });
        if let Some((n, j, st)) = styled {
            flush(&mut lit, s);
            s.open(&st);
            inline(&c[i + n..j], s);
            s.close(&st);
            i = j + n;
            continue;
        }
        lit.push(ch);
        i += 1;
    }
    flush(&mut lit, s);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sp(style: Style, offset: usize, len: usize) -> Span {
        Span { style, offset, len }
    }

    #[test]
    fn plain_keeps_link_addresses() {
        assert_eq!(
            plain("**Hi** 👋 see [the docs](https://x.io/p) and [https://y.io](https://y.io)"),
            "Hi 👋 see the docs (https://x.io/p) and https://y.io"
        );
        assert_eq!(plain("- one\n- two"), "• one\n• two");
    }

    #[test]
    fn telegram_spans() {
        let r = rich("**Hi** _there_, see `a_b` and [docs](https://x.io/p)");
        assert_eq!(r.text, "Hi there, see a_b and docs");
        assert_eq!(
            r.spans,
            vec![sp(Style::Bold, 0, 2), sp(Style::Italic, 3, 5), sp(Style::Code, 14, 3), sp(Style::Link("https://x.io/p".into()), 22, 4)]
        );
        // nesting, strike, headings, lists, quotes, code blocks
        let r = rich("# Plan\n- **one *two***\n* ~~old~~\n> said\n> twice\n```rust\nlet x = 1;\n```");
        assert_eq!(r.text, "Plan\n• one two\n• old\nsaid\ntwice\nlet x = 1;");
        assert!(r.spans.contains(&sp(Style::Bold, 0, 4)));
        assert!(r.spans.contains(&sp(Style::Bold, 7, 7)));
        assert!(r.spans.contains(&sp(Style::Italic, 11, 3)));
        assert!(r.spans.contains(&sp(Style::Strike, 17, 3)));
        assert!(r.spans.contains(&sp(Style::Quote, 21, 10)));
        assert!(r.spans.contains(&sp(Style::Pre("rust".into()), 32, 10)));
    }

    #[test]
    fn leaves_plain_text_alone() {
        for t in ["file_name and my_var_2", "2 * 3 * 4", "price: 5*", "a ** b", "under_score_", "[not a link](just words)", "C:\\path\\to"] {
            let r = rich(t);
            assert_eq!(r.text, t, "{t}");
            assert!(r.spans.is_empty(), "{t}: {:?}", r.spans);
        }
        assert_eq!(rich(r"\*not italic\*").text, "*not italic*");
        let r = rich("***both***");
        assert_eq!(r.text, "both");
        assert!(r.spans.contains(&sp(Style::Bold, 0, 4)) && r.spans.contains(&sp(Style::Italic, 0, 4)), "{:?}", r.spans);
    }

    #[test]
    fn bold_around_code() {
        let r = rich("**Квота (через API, `drive/v3/about`):**");
        assert_eq!(r.text, "Квота (через API, drive/v3/about):");
        assert!(r.spans.contains(&sp(Style::Bold, 0, 34)) && r.spans.contains(&sp(Style::Code, 18, 14)), "{:?}", r.spans);
        assert_eq!(to_whatsapp("**Квота (`drive/v3/about`):**"), "*Квота (`drive/v3/about`):*");
    }

    #[test]
    fn utf16_offsets() {
        let r = rich("🙂 **ok**");
        assert_eq!(r.text, "🙂 ok");
        assert_eq!(r.spans, vec![sp(Style::Bold, 3, 2)]);
    }

    #[test]
    fn whatsapp() {
        assert_eq!(to_whatsapp("**Hi** _there_ ~~x~~ `c`"), "*Hi* _there_ ~x~ `c`");
        assert_eq!(to_whatsapp("## Title\n- a\n- b"), "*Title*\n• a\n• b");
        assert_eq!(to_whatsapp("[docs](https://x.io) and [https://y.io](https://y.io)"), "docs (https://x.io) and https://y.io");
        assert_eq!(to_whatsapp("> one\n> two"), "> one\n> two");
        assert_eq!(to_whatsapp("```\ncode *x*\n```"), "```\ncode *x*\n```");
        assert_eq!(to_whatsapp("file_name stays"), "file_name stays");
    }
}
