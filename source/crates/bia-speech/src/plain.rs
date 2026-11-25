//! Markdown → speech. The model writes Markdown out of habit (bold, lists, links, code); a
//! voice reading it says "asterisk asterisk" or a whole URL. `speakable` keeps the words and
//! drops the markup: emphasis and code marks go, links keep their text, URLs become their
//! host, code blocks, rules and table separators are skipped, and headings, list items and
//! table rows end with a pause.

/// The text of `md` as it should be spoken.
pub fn speakable(md: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut fence: Option<&str> = None;
    let mut skipped_code = false;
    for raw in md.lines() {
        let t = raw.trim();
        if let Some(f) = fence {
            if t.starts_with(f) {
                fence = None;
            }
            continue;
        }
        if t.starts_with("```") || t.starts_with("~~~") {
            fence = Some(&t[..3]);
            skipped_code = true;
            continue;
        }
        if t.is_empty() {
            if out.last().is_some_and(|l| !l.is_empty()) {
                out.push(String::new());
            }
            continue;
        }
        if is_rule(t) || is_table_sep(t) || is_ref_def(t) {
            continue;
        }
        let mut l = t;
        while let Some(r) = l.strip_prefix('>') {
            l = r.trim_start();
        }
        let mut pause = false;
        let hashes = l.chars().take_while(|&c| c == '#').count();
        if (1..=6).contains(&hashes) && (l.len() == hashes || l[hashes..].starts_with(' ')) {
            l = l[hashes..].trim().trim_end_matches('#').trim_end();
            pause = true;
        }
        for m in ["- [ ] ", "- [x] ", "- [X] ", "* [ ] ", "* [x] ", "- ", "* ", "+ ", "• "] {
            if let Some(r) = l.strip_prefix(m) {
                l = r.trim_start();
                pause = true;
                break;
            }
        }
        let digits = l.chars().take_while(|c| c.is_ascii_digit()).count();
        if digits > 0 && digits < 4 && (l[digits..].starts_with(". ") || l[digits..].starts_with(") ")) {
            l = l[digits + 2..].trim_start();
            pause = true;
        }
        let mut s = if l.starts_with('|') && l.len() > 1 {
            pause = true;
            l.trim_matches('|').split('|').map(|c| inline(c.trim())).filter(|c| !c.is_empty()).collect::<Vec<_>>().join(", ")
        } else {
            inline(l)
        };
        if s.trim().is_empty() {
            continue;
        }
        if pause && !s.trim_end().ends_with(|c: char| ".!?…:;,。！？".contains(c)) {
            s.push('.');
        }
        out.push(s);
    }
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    let text = out.join("\n");
    // a reply that is nothing but code still says something
    if text.trim().is_empty() && skipped_code {
        return md.replace("```", "").replace("~~~", "").trim().to_string();
    }
    text
}

fn is_rule(t: &str) -> bool {
    let c: String = t.chars().filter(|c| !c.is_whitespace()).collect();
    c.len() >= 3 && (c.chars().all(|x| x == '-') || c.chars().all(|x| x == '*') || c.chars().all(|x| x == '_'))
}

fn is_table_sep(t: &str) -> bool {
    t.contains('-') && t.contains('|') && t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ' | '\t'))
}

/// `[1]: https://…` link reference definitions.
fn is_ref_def(t: &str) -> bool {
    t.starts_with('[') && t.find("]:").is_some_and(|i| !t[1..i].contains(']'))
}

fn word(c: Option<char>) -> bool {
    c.is_some_and(|c| c.is_alphanumeric())
}

/// Inline Markdown of one line → plain words.
fn inline(s: &str) -> String {
    let c: Vec<char> = s.chars().collect();
    let mut o = String::with_capacity(s.len());
    let mut i = 0;
    let at = |i: usize| c.get(i).copied();
    let starts = |i: usize, p: &str| p.chars().enumerate().all(|(k, ch)| c.get(i + k) == Some(&ch));
    while i < c.len() {
        let ch = c[i];
        match ch {
            '\\' if at(i + 1).is_some_and(|n| n.is_ascii_punctuation()) => {
                o.push(c[i + 1]);
                i += 2;
                continue;
            }
            '`' => {
                let n = c[i..].iter().take_while(|&&x| x == '`').count();
                let close = (i + n..c.len()).find(|&j| c[j..].iter().take_while(|&&x| x == '`').count() == n);
                if let Some(j) = close {
                    o.extend(&c[i + n..j]);
                    i = j + n;
                } else {
                    i += n;
                }
                continue;
            }
            '!' if at(i + 1) == Some('[') => {
                i += 1;
                continue;
            }
            '[' => {
                // [text](url) / [text][ref] → text; [^1] footnote → nothing
                if let Some(e) = (i + 1..c.len()).find(|&j| c[j] == ']') {
                    if at(i + 1) == Some('^') {
                        i = e + 1;
                        continue;
                    }
                    let after = at(e + 1);
                    let close = match after {
                        Some('(') => (e + 2..c.len()).find(|&j| c[j] == ')'),
                        Some('[') => (e + 2..c.len()).find(|&j| c[j] == ']'),
                        _ => None,
                    };
                    if let Some(k) = close {
                        let text: String = c[i + 1..e].iter().collect();
                        let target: String = c[e + 2..k].iter().collect();
                        let shown = inline(&text);
                        o.push_str(if shown.trim().is_empty() && after == Some('(') { host(&target) } else { shown }.as_str());
                        i = k + 1;
                        continue;
                    }
                }
            }
            '<' => {
                if let Some(e) = (i + 1..c.len().min(i + 300)).find(|&j| c[j] == '>') {
                    let inner: String = c[i + 1..e].iter().collect();
                    if inner.starts_with("http://") || inner.starts_with("https://") || inner.starts_with("mailto:") {
                        o.push_str(&host(&inner));
                        i = e + 1;
                        continue;
                    }
                    let tag = inner.trim_start_matches('/');
                    // <lang=ru>…</lang> picks the voice (langs::spans): it stays
                    if inner.starts_with("lang=") || inner == "/lang" {
                        o.extend(&c[i..=e]);
                        i = e + 1;
                        continue;
                    }
                    if tag.chars().next().is_some_and(|x| x.is_ascii_alphabetic()) && !inner.contains(char::is_whitespace) || tag.starts_with("br") {
                        if tag.starts_with("br") || tag.starts_with('p') {
                            o.push(' ');
                        }
                        i = e + 1;
                        continue;
                    }
                }
            }
            'h' if (starts(i, "http://") || starts(i, "https://")) && !word(i.checked_sub(1).and_then(at)) => {
                let e = (i..c.len()).find(|&j| c[j].is_whitespace()).unwrap_or(c.len());
                let mut url: String = c[i..e].iter().collect();
                let mut tail = String::new();
                while url.ends_with(|x: char| ".,;:!?)".contains(x)) {
                    tail.insert(0, url.pop().unwrap());
                }
                o.push_str(&host(&url));
                o.push_str(&tail);
                i = e;
                continue;
            }
            '*' | '_' | '~' => {
                let n = c[i..].iter().take_while(|&&x| x == ch).count();
                let prev = i.checked_sub(1).and_then(at);
                let next = at(i + n);
                let markup = match (ch, n) {
                    ('~', 1) => false,
                    ('_', _) => !(word(prev) && word(next)),
                    (_, 1) => !(prev.is_some_and(|p| !p.is_whitespace()) && next.is_some_and(|x| !x.is_whitespace()) && word(prev) && word(next))
                        && (next.is_some_and(|x| !x.is_whitespace()) || prev.is_some_and(|p| !p.is_whitespace())),
                    _ => true,
                };
                if markup {
                    i += n;
                    continue;
                }
                o.extend(&c[i..i + n]);
                i += n;
                continue;
            }
            _ => {}
        }
        o.push(ch);
        i += 1;
    }
    // collapse the spaces the removals left
    let mut r = String::with_capacity(o.len());
    for w in o.split(' ').filter(|w| !w.is_empty()) {
        if !r.is_empty() {
            r.push(' ');
        }
        r.push_str(w);
    }
    r
}

/// https://www.example.com/a/b?c → example.com (a voice reading a whole URL helps nobody).
fn host(url: &str) -> String {
    let u = url.trim();
    if let Some(m) = u.strip_prefix("mailto:") {
        return m.to_string();
    }
    let rest = u.split("://").nth(1).unwrap_or(u);
    let h = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let h = h.rsplit('@').next().unwrap_or(h);
    h.strip_prefix("www.").unwrap_or(h).to_string()
}

#[cfg(test)]
mod tests {
    use super::speakable;

    #[test]
    fn emphasis_code_links() {
        assert_eq!(speakable("This is **very** important and *quite* _nice_."), "This is very important and quite nice.");
        assert_eq!(speakable("Run `ls -la` now"), "Run ls -la now");
        assert_eq!(speakable("See [the docs](https://ownbia.com/docs) or https://www.example.com/a?b=1."), "See the docs or example.com.");
        assert_eq!(speakable("~~old~~ new"), "old new");
        assert_eq!(speakable("![a cat](cat.png)"), "a cat");
        assert_eq!(speakable("write to <mailto:a@b.c> or <https://x.org/p>"), "write to a@b.c or x.org");
        assert_eq!(speakable("line<br>next"), "line next");
        assert_eq!(speakable("Say <lang=ru>**привет**</lang> now"), "Say <lang=ru>привет</lang> now");
    }

    #[test]
    fn keeps_plain_symbols() {
        assert_eq!(speakable("snake_case_name stays"), "snake_case_name stays");
        assert_eq!(speakable("2 * 3 = 6 and 5*4"), "2 * 3 = 6 and 5*4");
        assert_eq!(speakable("about ~5 minutes"), "about ~5 minutes");
        assert_eq!(speakable("Привет, **мир**!"), "Привет, мир!");
        assert_eq!(speakable("a < b > c"), "a < b > c");
        assert_eq!(speakable("\\*not bold\\*"), "*not bold*");
    }

    #[test]
    fn blocks() {
        let md = "# Plan\n\n1. Buy milk\n2. Call **Ann**\n- [x] done item\n\n> quoted words\n\n---\n\n```bash\nrm -rf /tmp/x\n```\nAfter code.";
        assert_eq!(speakable(md), "Plan.\n\nBuy milk.\nCall Ann.\ndone item.\n\nquoted words\n\nAfter code.");
    }

    #[test]
    fn tables_and_refs() {
        let md = "| City | Temp |\n|---|:---:|\n| Madrid | 21° |\n\n[1]: https://example.com\nText[^1] here.";
        assert_eq!(speakable(md), "City, Temp.\nMadrid, 21°.\n\nText here.");
    }

    #[test]
    fn only_code_is_still_said() {
        assert_eq!(speakable("```\necho hi\n```"), "echo hi");
    }
}
