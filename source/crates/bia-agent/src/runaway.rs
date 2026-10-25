//! Runaway generation guard: a model stuck in a loop (the same paragraph again and
//! again, or endless made-up `<invoke><parameter>…` blocks in its thoughts) keeps
//! streaming until the token limit. Any model, any stream: the tail of the output is
//! checked for repetition, and the caller stops the stream and nudges the model.
use std::collections::HashSet;

/// Substring length compared (bytes).
const GRAM: usize = 24;
/// The tail that must be mostly repeats before the stream counts as a loop.
const TAIL: usize = 4096;
/// How far back a repeat may come from (tail included).
const WINDOW: usize = 16384;
/// Re-check after this many new bytes.
const STEP: usize = 512;
/// Share of the tail's substrings already seen earlier in the window (whitespace runs
/// collapsed). Measured on this repo's docs and sources: at most about 0.5 (boilerplate
/// code); an exact loop is near 1.0, loops of near-copies (tool-call markup with
/// different values each time) 0.67 and up.
const LIMIT: f64 = 0.6;
/// Consecutive checks at or above LIMIT before it counts (a repetitive stretch of
/// ordinary code passes; a loop stays).
const HOLD: u32 = 2;

/// One stream (thoughts and text are fed separately).
#[derive(Default)]
struct Stream {
    buf: Vec<u8>,
    /// Bytes ever streamed (buf keeps only the last WINDOW).
    total: usize,
    checked: usize,
    over: u32,
}

impl Stream {
    /// The repeating tail when the stream has turned into a loop.
    fn push(&mut self, piece: &str) -> Option<String> {
        for &b in piece.as_bytes() {
            if b.is_ascii_whitespace() {
                if self.buf.last() != Some(&b' ') {
                    self.buf.push(b' ');
                }
            } else {
                self.buf.push(b);
            }
        }
        self.total += piece.len();
        if self.buf.len() > WINDOW {
            let cut = self.buf.len() - WINDOW;
            self.buf.drain(..cut);
        }
        if self.buf.len() < TAIL + GRAM || self.total - self.checked < STEP {
            return None;
        }
        self.checked = self.total;
        if repeat_share(&self.buf, TAIL) < LIMIT {
            self.over = 0;
            return None;
        }
        self.over += 1;
        (self.over >= HOLD).then(|| {
            let tail = &self.buf[self.buf.len() - 300..];
            String::from_utf8_lossy(tail).trim_start_matches('\u{fffd}').to_string()
        })
    }
}

/// Share of the last `tail` bytes' GRAM-substrings that already occurred earlier in `buf`.
fn repeat_share(buf: &[u8], tail: usize) -> f64 {
    let start = buf.len().saturating_sub(tail);
    let mut seen: HashSet<&[u8]> = HashSet::with_capacity(buf.len());
    let (mut hits, mut n) = (0usize, 0usize);
    for i in 0..=buf.len() - GRAM {
        let g = &buf[i..i + GRAM];
        if i >= start {
            n += 1;
            if seen.contains(g) {
                hits += 1;
            }
        }
        seen.insert(g);
    }
    if n == 0 {
        0.0
    } else {
        hits as f64 / n as f64
    }
}

/// Watches one reply. `push` returns false once the reply is looping; `tripped` keeps
/// the repeating tail for the note to the model.
#[derive(Default)]
pub struct Guard {
    thought: Stream,
    text: Stream,
    tripped: Option<String>,
}

impl Guard {
    pub fn push(&mut self, thought: bool, piece: &str) -> bool {
        if self.tripped.is_some() {
            return false;
        }
        let s = if thought { &mut self.thought } else { &mut self.text };
        self.tripped = s.push(piece);
        self.tripped.is_none()
    }

    pub fn tripped(&self) -> Option<&str> {
        self.tripped.as_deref()
    }
}

/// A reply with no tool calls whose text is one written out: `say|web|answered=[…]`
/// (the envelope format copied from the history), `say(…)` or tool-call markup. Nothing
/// ran, so the model is told. The name of the function it meant, or the markup.
pub fn fake_call(text: &str, known: &[String]) -> Option<String> {
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let line = line.trim_start_matches(['`', '*', '-', '>', ' ']);
        for tag in ["<invoke", "<tool_call", "<function_call", "<parameter name="] {
            if line.contains(tag) {
                return Some(tag.trim_start_matches('<').to_string());
            }
        }
        let name: String = line.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
        let rest = line[name.len()..].trim_start();
        if !name.is_empty() && (rest.starts_with('|') || rest.starts_with('(')) && known.iter().any(|k| *k == name) {
            return Some(name);
        }
    }
    None
}

/// What the model reads after writing a call as text.
pub fn fake_notice(name: &str) -> String {
    format!(
        "nothing ran: your last reply wrote {name} as text instead of calling it. \
         Make it a real tool call (e.g. say(channel=…, message=…, answered=[…])). \
         Messages you meant to answer or close are still open."
    )
}

/// Retries after a loop within one turn before the turn stops.
pub const RETRIES: u32 = 2;

/// What the model reads after its reply was cut.
pub fn notice(tail: &str) -> String {
    let tail: String = tail.chars().rev().take(200).collect::<Vec<_>>().into_iter().rev().collect();
    format!(
        "your last reply was stopped: it kept repeating itself (it ended with: {:?}). \
         Do not repeat it. Call tools only as real tool calls, never as <invoke>/<parameter> \
         text in thoughts or replies. If you are stuck, say so in one short message.",
        tail.trim()
    )
}

/// `text` without its looping end: the part before the repeating window, shortened.
pub fn cut(text: &str) -> String {
    let keep = text.len().saturating_sub(TAIL).min(2000);
    let mut end = keep;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let head = text[..end].trim_end();
    if head.is_empty() {
        "[reply cut: it was repeating]".into()
    } else {
        format!("{head}\n[reply cut: it was repeating]")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(g: &mut Guard, thought: bool, s: &str) -> bool {
        // stream in small pieces, like an API does
        let mut rest = s;
        while !rest.is_empty() {
            let mut n = rest.len().min(17);
            while !rest.is_char_boundary(n) {
                n += 1;
            }
            if !g.push(thought, &rest[..n]) {
                return false;
            }
            rest = &rest[n..];
        }
        true
    }

    #[test]
    fn exact_loop_trips() {
        let mut g = Guard::default();
        let para = "I should check the update log first, then answer the user about progress.\n";
        assert!(!feed(&mut g, false, &para.repeat(200)));
        assert!(g.tripped().unwrap().contains("update log"));
    }

    #[test]
    fn varied_markup_loop_trips_in_thoughts() {
        let mut g = Guard::default();
        let mut s = String::new();
        for i in 0..300 {
            s.push_str(&format!(
                "<invoke name=\"bash\">\n<parameter name=\"command\">ls -la /bia-update/{i}</parameter>\n<parameter name=\"timeout_secs\">{}</parameter>\n</invoke>\n",
                i * 7
            ));
        }
        assert!(!feed(&mut g, true, &s));
        // the text stream is untouched by a looping thought stream until then
        assert!(g.tripped().is_some());
    }

    #[test]
    fn chatty_markup_loop_trips() {
        let mut g = Guard::default();
        let mut s = String::new();
        for i in 0..300 {
            s.push_str(&format!(
                "Let me check the job.\n<invoke name=\"job_logs\">\n<parameter name=\"id\">{i:04x}</parameter>\n<parameter name=\"tail\">{}</parameter>\n</invoke>\nThe job is still running, I will wait.\n",
                20 + i % 5
            ));
        }
        assert!(!feed(&mut g, true, &s));
    }

    #[test]
    fn real_sources_pass() {
        for src in [
            include_str!("repl.rs"),
            include_str!("remote.rs"),
            include_str!("tools.rs"),
            include_str!("../../../../ARCHITECTURE.md"),
        ] {
            let mut g = Guard::default();
            assert!(feed(&mut g, false, src), "tripped: {:?}", g.tripped());
        }
    }

    #[test]
    fn prose_and_code_pass() {
        let mut g = Guard::default();
        // varied prose: every sentence different
        let mut s = String::new();
        for i in 0..400 {
            s.push_str(&format!(
                "Step {i}: the {} reading was {} at {}:{:02}, so the {} value moves to {}. ",
                ["first", "second", "third", "next"][i % 4],
                i * 37 % 101,
                i % 24,
                i * 13 % 60,
                ["upper", "lower", "middle"][i % 3],
                i * i % 997
            ));
        }
        assert!(feed(&mut g, false, &s), "prose tripped");
        // a long code listing with shared boilerplate but different bodies
        let mut code = String::new();
        for i in 0..200 {
            code.push_str(&format!(
                "fn handler_{i}(req: &Request) -> Response {{\n    let v = req.get(\"k{i}\").unwrap_or({});\n    Response::ok(v * {} + {})\n}}\n\n",
                i * 3, i % 11, i * i
            ));
        }
        let mut g = Guard::default();
        assert!(feed(&mut g, false, &code), "code tripped");
    }

    #[test]
    fn short_replies_never_checked() {
        let mut g = Guard::default();
        assert!(feed(&mut g, false, &"ok ".repeat(1000)));
    }

    #[test]
    fn calls_written_as_text() {
        let known: Vec<String> = ["say", "discover", "bash"].iter().map(|s| s.to_string()).collect();
        assert_eq!(fake_call("say|web|answered=[b731eaf330007,b732474b0000c]", &known).as_deref(), Some("say"));
        assert_eq!(fake_call("I will reply.\nsay(channel=\"web\", message=\"hi\")", &known).as_deref(), Some("say"));
        assert_eq!(fake_call("<invoke name=\"say\">", &known).as_deref(), Some("invoke"));
        assert_eq!(fake_call("`bash(command=\"ls\")`", &known).as_deref(), Some("bash"));
        // ordinary notes stay notes
        assert!(fake_call("(no action — already closed)", &known).is_none());
        assert!(fake_call("The say tool sends messages; bash runs commands.", &known).is_none());
        assert!(fake_call("unknown_fn(1)", &known).is_none());
    }

    #[test]
    fn cut_keeps_the_head() {
        let text = format!("Here is the plan.{}", "loop ".repeat(2000));
        let c = cut(&text);
        assert!(c.starts_with("Here is the plan."));
        assert!(c.ends_with("[reply cut: it was repeating]"));
        assert!(cut(&"x".repeat(100)).starts_with("[reply cut"));
    }
}
