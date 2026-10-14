//! Long-term memory: append-only journal + time-ranged, categorized reflections.
//! Compact (in the REPL) is the only prefix rewrite; this module is the on-disk store.
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

const JOURNAL: &str = "journal.jsonl";
const REFLECTIONS: &str = "reflections.jsonl";
const STUB_NAME: &str = "reflection";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JournalMsg {
    pub id: String,
    pub role: String,
    #[serde(default)]
    pub name: String,
    pub content: String,
    #[serde(default)]
    pub call_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Record {
    pub id: String,
    pub bundle_id: String,
    /// model-chosen kind: summary, facts, decisions, … (normalized [a-z0-9_]+)
    pub category: String,
    pub depth: u32,
    pub t_start: String,
    pub t_end: String,
    pub t_start_unix: u64,
    pub t_end_unix: u64,
    pub text: String,
    #[serde(default)]
    pub message_ids: Vec<String>,
    #[serde(default)]
    pub child_ids: Vec<String>,
    #[serde(default)]
    pub created_at: String,
}

#[derive(Clone, Debug)]
pub struct Parsed {
    pub category: String,
    pub t_start: String,
    pub t_end: String,
    pub text: String,
}

pub struct Store {
    pub dir: PathBuf,
    msg_seq: u64,
    refl_seq: u64,
}

impl Store {
    pub fn open(dir: &Path) -> Store {
        let _ = std::fs::create_dir_all(dir);
        let mut s = Store {
            dir: dir.to_path_buf(),
            msg_seq: 0,
            refl_seq: 0,
        };
        s.msg_seq = scan_seq(&s.dir.join(JOURNAL), "m_");
        s.refl_seq = scan_seq(&s.dir.join(REFLECTIONS), "r_");
        s
    }

    pub fn next_msg_id(&mut self) -> String {
        self.msg_seq += 1;
        format!("m_{}", self.msg_seq)
    }

    fn next_refl_id(&mut self, cat: &str) -> String {
        self.refl_seq += 1;
        format!("r_{}_{}", self.refl_seq, cat)
    }

    fn next_bundle_id(&mut self) -> String {
        self.refl_seq += 1;
        format!("b_{}", self.refl_seq)
    }

    pub fn journal(
        &mut self,
        id: &str,
        role: &str,
        name: &str,
        content: &str,
        call_id: &str,
    ) -> Result<()> {
        append_jsonl(
            &self.dir.join(JOURNAL),
            &JournalMsg {
                id: id.into(),
                role: role.into(),
                name: name.into(),
                content: content.into(),
                call_id: call_id.into(),
            },
        )
    }

    pub fn save_records(&mut self, recs: &[Record]) -> Result<()> {
        for r in recs {
            append_jsonl(&self.dir.join(REFLECTIONS), r)?;
        }
        unsafe { libc::sync() };
        Ok(())
    }

    pub fn load_all(&self) -> Vec<Record> {
        read_jsonl(&self.dir.join(REFLECTIONS))
    }

    pub fn get(&self, id: &str) -> Option<Record> {
        self.load_all().into_iter().find(|r| r.id == id)
    }

    pub fn categories(&self) -> Vec<(String, usize)> {
        let mut map: Vec<(String, usize)> = vec![];
        for r in self.load_all() {
            if let Some(e) = map.iter_mut().find(|(c, _)| *c == r.category) {
                e.1 += 1;
            } else {
                map.push((r.category, 1));
            }
        }
        map.sort_by(|a, b| a.0.cmp(&b.0));
        map
    }

    pub fn category_names(&self) -> Vec<String> {
        self.categories().into_iter().map(|(c, _)| c).collect()
    }

    pub fn list(
        &self,
        from: Option<u64>,
        to: Option<u64>,
        category: Option<&str>,
        depth: Option<u32>,
    ) -> Vec<Record> {
        let mut v: Vec<Record> = self
            .load_all()
            .into_iter()
            .filter(|r| {
                if let Some(c) = category {
                    if r.category != c {
                        return false;
                    }
                }
                if let Some(d) = depth {
                    if r.depth != d {
                        return false;
                    }
                }
                overlap(r.t_start_unix, r.t_end_unix, from, to)
            })
            .collect();
        v.sort_by(|a, b| {
            a.t_start_unix
                .cmp(&b.t_start_unix)
                .then(a.category.cmp(&b.category))
        });
        v
    }

    pub fn journal_range(&self, from: u64, to: u64) -> Vec<JournalMsg> {
        read_jsonl::<JournalMsg>(&self.dir.join(JOURNAL))
            .into_iter()
            .filter(|m| {
                issued_unix(&m.content)
                    .map(|t| t >= from && t <= to)
                    .unwrap_or(false)
            })
            .collect()
    }

    pub fn journal_by_ids(&self, ids: &[String]) -> Vec<JournalMsg> {
        let all = read_jsonl::<JournalMsg>(&self.dir.join(JOURNAL));
        ids.iter()
            .filter_map(|id| all.iter().find(|m| m.id == *id).cloned())
            .collect()
    }

    pub fn stats(&self) -> (usize, usize) {
        (
            count_lines(&self.dir.join(JOURNAL)),
            count_lines(&self.dir.join(REFLECTIONS)),
        )
    }

    pub fn forget(&self) {
        let _ = std::fs::remove_file(self.dir.join(JOURNAL));
        let _ = std::fs::remove_file(self.dir.join(REFLECTIONS));
        unsafe { libc::sync() };
    }

    /// Seed the journal from a resumed live history that was never journaled.
    pub fn backfill(&mut self, rows: &[(String, String, String, String)]) {
        if self.dir.join(JOURNAL).exists() && count_lines(&self.dir.join(JOURNAL)) > 0 {
            return;
        }
        for (id, role, name, content) in rows {
            let _ = self.journal(id, role, name, content, "");
        }
    }

    pub fn build_records(
        &mut self,
        parsed: Vec<Parsed>,
        depth: u32,
        message_ids: Vec<String>,
        child_ids: Vec<String>,
        span_start: &str,
        span_end: &str,
    ) -> Vec<Record> {
        let bundle_id = self.next_bundle_id();
        let created = crate::clock::rfc3339_now();
        let mut out = vec![];
        for p in parsed {
            let cat = normalize_category(&p.category);
            let t_start = if crate::clock::parse_rfc3339(&p.t_start).is_some() {
                p.t_start
            } else {
                span_start.to_string()
            };
            let t_end = if crate::clock::parse_rfc3339(&p.t_end).is_some() {
                p.t_end
            } else {
                span_end.to_string()
            };
            let t_start_unix = crate::clock::parse_rfc3339(&t_start).unwrap_or(0);
            let t_end_unix = crate::clock::parse_rfc3339(&t_end).unwrap_or(t_start_unix);
            let mut text = p.text.trim().to_string();
            if !text.to_ascii_lowercase().contains("reflection ") {
                text = format!("reflection {t_start} .. {t_end}\n{text}");
            }
            if text.len() > 12 * 1024 {
                let mut end = 12 * 1024;
                while end > 0 && !text.is_char_boundary(end) {
                    end -= 1;
                }
                text = format!("{}\n...[truncated]", &text[..end]);
            }
            out.push(Record {
                id: self.next_refl_id(&cat),
                bundle_id: bundle_id.clone(),
                category: cat,
                depth,
                t_start,
                t_end,
                t_start_unix,
                t_end_unix,
                text,
                message_ids: message_ids.clone(),
                child_ids: child_ids.clone(),
                created_at: created.clone(),
            });
        }
        out
    }
}

pub fn is_stub(name: &str, content: &str) -> bool {
    if name == STUB_NAME {
        return true;
    }
    envelope_body(content).starts_with("reflection ")
}

pub fn stub_name() -> &'static str {
    STUB_NAME
}

pub fn stub_field(content: &str, key: &str) -> Option<String> {
    let body = envelope_body(content);
    let k = format!("{key}=");
    let i = body.find(&k)?;
    let rest = &body[i + k.len()..];
    let end = rest
        .find(|c: char| c.is_whitespace() || c == ']' || c == '[')
        .unwrap_or(rest.len());
    let v = rest[..end].trim().trim_matches(',');
    if v.is_empty() {
        None
    } else {
        Some(v.to_string())
    }
}

pub fn stub_id(content: &str) -> Option<String> {
    stub_field(content, "id")
}

pub fn stub_depth(content: &str) -> u32 {
    stub_field(content, "depth")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

pub fn stub_body(rec: &Record) -> String {
    format!(
        "reflection {} .. {} [id={} category={} depth={} bundle={}]\n{}",
        rec.t_start, rec.t_end, rec.id, rec.category, rec.depth, rec.bundle_id, rec.text
    )
}

pub fn envelope_body(content: &str) -> &str {
    // identity|channel|issued|delivered|id|message
    let mut n = 0;
    for (i, c) in content.char_indices() {
        if c == '|' {
            n += 1;
            if n == 5 {
                return &content[i + 1..];
            }
        }
    }
    content
}

pub fn issued_of(content: &str) -> Option<String> {
    content.split('|').nth(2).map(|s| s.to_string())
}

pub fn issued_unix(content: &str) -> Option<u64> {
    issued_of(content).and_then(|s| crate::clock::parse_rfc3339(&s))
}

pub fn normalize_category(s: &str) -> String {
    let mut o = String::new();
    for c in s.trim().to_ascii_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            o.push(c);
        } else if c == ' ' || c == '-' || c == '/' {
            if !o.ends_with('_') && !o.is_empty() {
                o.push('_');
            }
        }
    }
    let o = o.trim_matches('_').to_string();
    if o.is_empty() {
        "summary".into()
    } else {
        o.chars().take(32).collect()
    }
}

pub fn parse_blocks(text: &str, span_start: &str, span_end: &str) -> Vec<Parsed> {
    let t = text.replace("<|im_end|>", "");
    let t = match t.rfind("</think>") {
        Some(i) => t[i + 8..].to_string(),
        None => t,
    };
    let mut out = vec![];
    let mut rest = t.as_str();
    while let Some(i) = rest.find("REFLECTION") {
        rest = rest[i + "REFLECTION".len()..].trim_start();
        let mut category = "summary".to_string();
        let mut t_start = span_start.to_string();
        let mut t_end = span_end.to_string();
        if let Some(nl) = rest.find('\n') {
            let first = rest[..nl].trim();
            if let Some(c) = header_val(first, "category") {
                category = c;
            }
            if let Some(s) = header_val(first, "start") {
                t_start = s;
            }
            if let Some(e) = header_val(first, "end") {
                t_end = e;
            }
            rest = rest.get(nl + 1..).unwrap_or("");
        }
        loop {
            let line_end = rest.find('\n').unwrap_or(rest.len());
            let line = rest[..line_end].trim();
            if let Some(s) = line.strip_prefix("start=") {
                t_start = s.trim().to_string();
                rest = rest.get(line_end + 1..).unwrap_or("");
                continue;
            }
            if let Some(s) = line.strip_prefix("end=") {
                t_end = s.trim().to_string();
                rest = rest.get(line_end + 1..).unwrap_or("");
                continue;
            }
            if let Some(s) = line.strip_prefix("category=") {
                category = s.trim().to_string();
                rest = rest.get(line_end + 1..).unwrap_or("");
                continue;
            }
            break;
        }
        let (body, next) = match rest.find("\nEND") {
            Some(j) => (&rest[..j], rest.get(j + 4..).unwrap_or("")),
            None => match rest.find("\nREFLECTION") {
                Some(j) => (&rest[..j], &rest[j..]),
                None => (rest, ""),
            },
        };
        let body = body.trim();
        if !body.is_empty() {
            out.push(Parsed {
                category,
                t_start,
                t_end,
                text: body.to_string(),
            });
        }
        rest = next;
    }
    out
}

pub fn mechanical(
    span_start: &str,
    span_end: &str,
    n_msgs: usize,
    identities: &[String],
    tools: &[String],
    stub_cats: &[String],
) -> Vec<Parsed> {
    let who = if identities.is_empty() {
        "none".into()
    } else {
        identities.join(", ")
    };
    let tools_s = if tools.is_empty() {
        "none".into()
    } else {
        tools.join(", ")
    };
    let mut blocks = vec![Parsed {
        category: "summary".into(),
        t_start: span_start.into(),
        t_end: span_end.into(),
        text: format!(
            "reflection {span_start} .. {span_end}\n{n_msgs} messages. identities: {who}. tools: {tools_s}.{}",
            if stub_cats.is_empty() { String::new() } else { format!(" rolled-up categories: {}", stub_cats.join(", ")) }
        ),
    }];
    if !tools.is_empty() || !identities.is_empty() {
        blocks.push(Parsed {
            category: "facts".into(),
            t_start: span_start.into(),
            t_end: span_end.into(),
            text: format!(
                "reflection {span_start} .. {span_end}\nidentities: {who}\ntools used: {tools_s}"
            ),
        });
    }
    blocks
}

pub fn instruction(span_start: &str, span_end: &str, known: &[String], rollup: bool) -> String {
    let known_s = if known.is_empty() {
        "summary, facts, decisions, people, open_questions, lessons".into()
    } else {
        known.join(", ")
    };
    let kind = if rollup {
        "The messages above are earlier reflections (not raw events). Write a higher-level memory of that period."
    } else {
        "The conversation above (from the period) is what you are consolidating."
    };
    format!(
        "REFLECT. Do not call tools. Do not speak to root.\n\
{kind}\n\
Period: {span_start} .. {span_end}\n\
Write 2 or more blocks in this exact form:\n\
REFLECTION category=summary\nstart={span_start}\nend={span_end}\nreflection {span_start} .. {span_end}\n<dense text>\nEND\n\
REFLECTION category=facts\nstart={span_start}\nend={span_end}\nreflection {span_start} .. {span_end}\n<durable facts, settings, identities, decisions>\nEND\n\
Category is a short lowercase name you choose. Known categories: {known_s}\n\
Reuse a known category when it fits. Invent a new one when you need a new kind of memory.\n\
Always include summary. Include facts if anything should survive as a fact. Add more blocks when useful (decisions, people, open_questions, lessons, …).\n\
Each block body must start with: reflection {span_start} .. {span_end}\n\
Be brief and precise."
    )
}

pub fn parse_query_time(s: &str, end_of_day: bool) -> Option<u64> {
    let s = s.trim();
    if s.len() == 10 && s.as_bytes().get(4) == Some(&b'-') {
        let t = crate::clock::parse_rfc3339(s)?;
        return Some(if end_of_day { t + 86400 - 1 } else { t });
    }
    crate::clock::parse_rfc3339(s)
}

pub fn tool_list(dir: &Path, args: &Value) -> Result<String> {
    let st = Store::open(dir);
    let from = args
        .get("from")
        .and_then(|v| v.as_str())
        .and_then(|s| parse_query_time(s, false));
    let to = args
        .get("to")
        .and_then(|v| v.as_str())
        .and_then(|s| parse_query_time(s, true));
    let cat = args
        .get("category")
        .and_then(|v| v.as_str())
        .map(normalize_category);
    let depth = args.get("depth").and_then(|v| v.as_u64()).map(|n| n as u32);
    let recs = st.list(from, to, cat.as_deref(), depth);
    if recs.is_empty() {
        return Ok("(no reflections in that range)".into());
    }
    let mut lines = vec![];
    for r in recs {
        let preview: String = r.text.chars().take(180).collect();
        lines.push(format!(
            "{}  {}  depth={}  {} .. {}  bundle={}\n  {}",
            r.id,
            r.category,
            r.depth,
            r.t_start,
            r.t_end,
            r.bundle_id,
            preview.replace('\n', " ")
        ));
    }
    Ok(lines.join("\n"))
}

pub fn tool_get(dir: &Path, args: &Value) -> Result<String> {
    let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("").trim();
    if id.is_empty() {
        bail!("id required");
    }
    let st = Store::open(dir);
    let Some(r) = st.get(id) else {
        bail!("no reflection {id}")
    };
    let sibs: Vec<String> = st
        .load_all()
        .into_iter()
        .filter(|x| x.bundle_id == r.bundle_id && x.id != r.id)
        .map(|x| format!("{} ({})", x.id, x.category))
        .collect();
    Ok(format!(
        "id: {}\nbundle: {}\ncategory: {}\ndepth: {}\nperiod: {} .. {}\nchildren: {}\nsiblings: {}\nmessages: {}\n\n{}",
        r.id,
        r.bundle_id,
        r.category,
        r.depth,
        r.t_start,
        r.t_end,
        if r.child_ids.is_empty() { "—".into() } else { r.child_ids.join(", ") },
        if sibs.is_empty() { "—".into() } else { sibs.join(", ") },
        if r.message_ids.is_empty() { "—".into() } else { r.message_ids.join(", ") },
        r.text
    ))
}

pub fn tool_tree(dir: &Path, args: &Value) -> Result<String> {
    let st = Store::open(dir);
    let from = args
        .get("from")
        .and_then(|v| v.as_str())
        .and_then(|s| parse_query_time(s, false));
    let to = args
        .get("to")
        .and_then(|v| v.as_str())
        .and_then(|s| parse_query_time(s, true));
    let recs = st.list(from, to, None, None);
    if recs.is_empty() {
        return Ok("(empty)".into());
    }
    let mut lines = vec![];
    for r in &recs {
        lines.push(format!(
            "{}{} [{}] {} .. {}  {}",
            "  ".repeat(r.depth as usize),
            r.id,
            r.category,
            r.t_start,
            r.t_end,
            r.bundle_id
        ));
    }
    Ok(lines.join("\n"))
}

pub fn tool_messages(dir: &Path, args: &Value) -> Result<String> {
    let st = Store::open(dir);
    let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("").trim();
    let rows = if !id.is_empty() {
        let r = st
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("no reflection {id}"))?;
        if !r.message_ids.is_empty() {
            st.journal_by_ids(&r.message_ids)
        } else if !r.child_ids.is_empty() {
            return Ok(format!(
                "depth {} reflection; dive children: {}",
                r.depth,
                r.child_ids.join(", ")
            ));
        } else {
            st.journal_range(r.t_start_unix, r.t_end_unix)
        }
    } else {
        let from = args
            .get("from")
            .and_then(|v| v.as_str())
            .and_then(|s| parse_query_time(s, false));
        let to = args
            .get("to")
            .and_then(|v| v.as_str())
            .and_then(|s| parse_query_time(s, true));
        let (from, to) = match (from, to) {
            (Some(a), Some(b)) => (a, b),
            _ => bail!("id or from+to required"),
        };
        st.journal_range(from, to)
    };
    if rows.is_empty() {
        return Ok("(no messages)".into());
    }
    let mut lines = vec![];
    for m in rows.into_iter().take(80) {
        let preview: String = m.content.chars().take(240).collect();
        lines.push(format!(
            "{} {} {}",
            m.id,
            m.role,
            preview.replace('\n', " ")
        ));
    }
    Ok(lines.join("\n"))
}

pub fn tool_categories(dir: &Path) -> Result<String> {
    let st = Store::open(dir);
    let cats = st.categories();
    if cats.is_empty() {
        return Ok("(none yet; categories appear when you reflect)".into());
    }
    Ok(cats
        .into_iter()
        .map(|(c, n)| format!("{c}  {n}"))
        .collect::<Vec<_>>()
        .join("\n"))
}

fn overlap(rs: u64, re: u64, from: Option<u64>, to: Option<u64>) -> bool {
    let from = from.unwrap_or(0);
    let to = to.unwrap_or(u64::MAX);
    rs <= to && from <= re
}

fn header_val(header: &str, key: &str) -> Option<String> {
    let k = format!("{key}=");
    let rest = header.split_whitespace().find(|p| p.starts_with(&k))?;
    Some(rest[k.len()..].trim_matches('"').trim().to_string())
}

fn append_jsonl<T: Serialize>(path: &Path, v: &T) -> Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    serde_json::to_writer(&mut f, v)?;
    f.write_all(b"\n")?;
    f.flush()?;
    Ok(())
}

fn read_jsonl<T: for<'de> Deserialize<'de>>(path: &Path) -> Vec<T> {
    let Ok(f) = std::fs::File::open(path) else {
        return vec![];
    };
    BufReader::new(f)
        .lines()
        .filter_map(|l| l.ok())
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(&l).ok())
        .collect()
}

fn count_lines(path: &Path) -> usize {
    let Ok(f) = std::fs::File::open(path) else {
        return 0;
    };
    BufReader::new(f)
        .lines()
        .filter_map(|l| l.ok())
        .filter(|l| !l.trim().is_empty())
        .count()
}

fn scan_seq(path: &Path, prefix: &str) -> u64 {
    let mut max = 0u64;
    for line in read_jsonl::<Value>(path) {
        if let Some(id) = line.get("id").and_then(|x| x.as_str()) {
            if let Some(rest) = id.strip_prefix(prefix) {
                let n = rest
                    .split('_')
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                if n > max {
                    max = n;
                }
            }
        }
    }
    max
}
