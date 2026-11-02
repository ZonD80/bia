//! memory_cut: erase one message for good, for a secret that landed in the history.
//! Its row stays as a `[cut]` tombstone: a tool result still answers its call, ids in
//! reflections still resolve, and a user row keeps its envelope so its time still counts.
//! Runs in the agent loop, which holds the live history: an MCP process could not reach it.
//! Files are replaced atomically and the old bytes are then overwritten in place.
use crate::memory;
use crate::tools::{Msg, Role};
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::io::{Seek, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

const CUT: &str = "[cut]";
/// Lines of a cut message shorter than this are not searched for in logs: too common.
const MIN_LINE: usize = 16;
const MIN_FIND: usize = 4;

/// What is left of a message: for an envelope, its head (who, where, when, id).
fn tombstone(role: &str, content: &str) -> String {
    if role == "user" {
        let body = memory::envelope_body(content);
        if body.len() < content.len() {
            return format!("{}{CUT}", &content[..content.len() - body.len()]);
        }
    }
    CUT.into()
}

fn role_str(r: &Role) -> &'static str {
    match r {
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    }
}

fn strings_of(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::String(s) => out.push(s.clone()),
        Value::Array(a) => a.iter().for_each(|x| strings_of(x, out)),
        Value::Object(o) => o.values().for_each(|x| strings_of(x, out)),
        _ => {}
    }
}

/// The lines worth searching for elsewhere, longest first (a line may contain a shorter one).
fn needles(texts: &[String]) -> Vec<String> {
    let mut v: Vec<String> = texts
        .iter()
        .flat_map(|t| t.lines())
        .map(str::trim)
        .filter(|l| l.chars().count() >= MIN_LINE)
        .map(String::from)
        .collect();
    v.sort_by(|a, b| b.len().cmp(&a.len()).then(a.cmp(b)));
    v.dedup();
    v
}

/// A needle as it looks inside a JSON string, when that differs.
fn json_form(s: &str) -> Option<String> {
    let q = serde_json::to_string(s).ok()?;
    let inner = &q[1..q.len() - 1];
    (inner != s).then(|| inner.to_string())
}

fn replace_all(hay: &[u8], needle: &[u8], with: &[u8]) -> Option<Vec<u8>> {
    let f = memchr::memmem::Finder::new(needle);
    let mut at = f.find(hay)?;
    let mut out = Vec::with_capacity(hay.len());
    let mut from = 0;
    loop {
        out.extend_from_slice(&hay[from..from + at]);
        out.extend_from_slice(with);
        from += at + needle.len();
        match f.find(&hay[from..]) {
            Some(n) => at = n,
            None => break,
        }
    }
    out.extend_from_slice(&hay[from..]);
    Some(out)
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        match e.file_type() {
            Ok(t) if t.is_dir() => walk(&p, out),
            Ok(t) if t.is_file() => out.push(p),
            _ => {}
        }
    }
}

/// Everything besides the journal and the conversation that repeats message text.
fn other_stores(files: &Path) -> Vec<PathBuf> {
    let state = files.join("state");
    let mut v = vec![
        state.join("inbox.jsonl"),
        state.join("reflections.jsonl"),
        state.join("dream.json"),
    ];
    walk(&files.join("sessions"), &mut v);
    walk(&files.join("logs").join("jobs"), &mut v);
    walk(&state.join("memory"), &mut v);
    walk(&state.join("ignored"), &mut v);
    v.retain(|p| p.is_file());
    v
}

/// Open files whose old bytes must go once they are replaced or removed.
pub fn hold(paths: &[PathBuf]) -> Vec<std::fs::File> {
    paths
        .iter()
        .filter_map(|p| std::fs::OpenOptions::new().write(true).open(p).ok())
        .collect()
}

/// Overwrite what the held files contain (works after they were renamed or unlinked).
pub fn zero(files: Vec<std::fs::File>) {
    let chunk = [0u8; 64 * 1024];
    for mut f in files {
        let mut left = f.metadata().map(|m| m.len()).unwrap_or(0);
        if f.rewind().is_err() {
            continue;
        }
        while left > 0 {
            let n = left.min(chunk.len() as u64) as usize;
            if f.write_all(&chunk[..n]).is_err() {
                break;
            }
            left -= n as u64;
        }
        let _ = f.sync_all();
    }
}

/// Replace a file atomically, same owner and mode, then overwrite its old bytes.
fn rewrite(path: &Path, data: &[u8]) -> Result<()> {
    let meta = std::fs::metadata(path)?;
    let mut name = path.as_os_str().to_owned();
    name.push(".cut.tmp");
    let tmp = PathBuf::from(name);
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.set_permissions(std::fs::Permissions::from_mode(meta.mode() & 0o7777))?;
        f.sync_all()?;
    }
    // the agent is root: without this a rewritten file falls out of bia's backups
    let _ = std::os::unix::fs::chown(&tmp, Some(meta.uid()), Some(meta.gid()));
    let old = hold(&[path.to_path_buf()]);
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    zero(old);
    Ok(())
}

/// Replace the needles in the other stores; returns the files changed, relative to `files`.
fn scrub(files: &Path, needles: &[String]) -> Result<Vec<String>> {
    let mut changed = vec![];
    for p in other_stores(files) {
        let Ok(mut data) = std::fs::read(&p) else { continue };
        let mut hit = false;
        for n in needles {
            let escaped = json_form(n);
            for form in std::iter::once(n.as_str()).chain(escaped.as_deref()) {
                if let Some(next) = replace_all(&data, form.as_bytes(), CUT.as_bytes()) {
                    data = next;
                    hit = true;
                }
            }
        }
        if hit {
            rewrite(&p, &data)?;
            changed.push(rel(files, &p));
        }
    }
    Ok(changed)
}

fn rel(files: &Path, p: &Path) -> String {
    p.strip_prefix(files).unwrap_or(p).display().to_string()
}

/// The log of the job whose result this envelope carries (PTY output differs from it line by line).
fn job_log(files: &Path, content: &str) -> Option<PathBuf> {
    let mut f = content.split('|');
    let (who, channel) = (f.next()?, f.next()?);
    let id = channel.strip_prefix("job:")?;
    if who != "system" || id.is_empty() || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let p = files.join("logs").join("jobs").join(format!("{id}.log"));
    p.is_file().then_some(p)
}

fn is_cut_call(name: &str, args: &Value) -> bool {
    crate::tools::Tools::expand_call(name, args).0 == "memory_cut"
}

/// The ids of messages holding any needle: journal first, then rows only the live context has.
/// memory_cut's own calls do not count: their arguments are what was searched for.
fn holders(state: &Path, history: &[Msg], needles: &[String], skip: &str) -> Vec<(String, String)> {
    let finders: Vec<_> = needles.iter().map(|n| memchr::memmem::Finder::new(n.as_bytes())).collect();
    let has = |t: &str| finders.iter().any(|f| f.find(t.as_bytes()).is_some());
    let mut out: Vec<(String, String)> = vec![];
    let mut add = |id: &str, label: String| {
        if id != skip && !out.iter().any(|(i, _)| i == id) {
            out.push((id.to_string(), label));
        }
    };
    for m in memory::Store::open(state).journal_all() {
        if has(&m.content) {
            add(&m.id, format!("{} {}", m.role, m.name).trim().to_string());
        }
    }
    for m in history {
        let mut texts = vec![m.content.clone()];
        for c in &m.tool_calls {
            if !is_cut_call(&c.name, &c.args) {
                strings_of(&c.args, &mut texts);
            }
        }
        if texts.iter().any(|t| has(t)) {
            add(&m.id, format!("{} {}", role_str(&m.role), m.name).trim().to_string());
        }
    }
    out
}

/// memory_cut(find=…): which messages and files hold a fragment. Never echoes their text.
pub fn find(files: &Path, history: &[Msg], fragment: &str) -> Result<String> {
    let fragment = fragment.trim();
    if fragment.chars().count() < MIN_FIND {
        bail!("find needs at least {MIN_FIND} characters");
    }
    let needle = vec![fragment.to_string()];
    let rows = holders(&files.join("state"), history, &needle, "");
    let escaped = json_form(fragment);
    let in_files: Vec<String> = other_stores(files)
        .iter()
        .filter(|p| {
            std::fs::read(p).is_ok_and(|d| {
                std::iter::once(fragment)
                    .chain(escaped.as_deref())
                    .any(|n| memchr::memmem::find(&d, n.as_bytes()).is_some())
            })
        })
        .map(|p| rel(files, p))
        .collect();
    if rows.is_empty() && in_files.is_empty() {
        return Ok("nothing holds it".into());
    }
    let mut lines: Vec<String> = rows.iter().map(|(id, what)| format!("{id} {what}")).collect();
    if !in_files.is_empty() {
        lines.push(format!("also in files: {}", in_files.join(", ")));
    }
    lines.push("memory_cut(id=…) erases one message and its lines in those files.".into());
    Ok(lines.join("\n"))
}

#[derive(Debug)]
pub struct Report {
    pub id: String,
    pub what: String,
    pub files: Vec<String>,
    pub also: Vec<String>,
}

impl Report {
    pub fn text(&self) -> String {
        let mut s = format!(
            "cut {} ({}): erased from the live context, journal.jsonl and conversation.json with its backup; a [cut] line stays in its place.",
            self.id, self.what
        );
        if !self.files.is_empty() {
            s.push_str(&format!(" Also erased its lines in: {}.", self.files.join(", ")));
        }
        if !self.also.is_empty() {
            s.push_str(&format!(" The same lines are still in {}: cut each of them too.", self.also.join(", ")));
        }
        s.push_str(&format!(" Not touched: lines under {MIN_LINE} characters in those files, chat platforms, backups and files outside the history. Whatever reached a cloud model or a chat stays out there: if this was a secret, replace it."));
        s
    }
}

/// Cut message `id` from the history and the journal, and its lines from the other stores.
/// The caller saves the conversation.
pub fn cut_message(files: &Path, history: &mut [Msg], id: &str) -> Result<Report> {
    let state = files.join("state");
    let journal = state.join(memory::JOURNAL);
    let raw = std::fs::read_to_string(&journal).unwrap_or_default();
    let mut texts = vec![];
    let mut what = None;
    let mut rows: Vec<String> = Vec::new();
    let mut in_journal = false;
    let mut job_source = None;
    for line in raw.lines() {
        if let Ok(mut v) = serde_json::from_str::<Value>(line) {
            if v["id"].as_str() == Some(id) {
                let role = v["role"].as_str().unwrap_or("").to_string();
                let content = v["content"].as_str().unwrap_or("").to_string();
                what = Some(format!("{role} {}", v["name"].as_str().unwrap_or("")).trim().to_string());
                let stone = tombstone(&role, &content);
                if stone != content {
                    in_journal = true;
                    job_source = Some(content.clone());
                    texts.push(memory::envelope_body(&content).to_string());
                    v["content"] = json!(stone);
                    rows.push(v.to_string());
                    continue;
                }
            }
        }
        rows.push(line.to_string());
    }
    let mut in_live = false;
    for m in history.iter_mut().filter(|m| m.id == id) {
        let role = role_str(&m.role);
        what = Some(format!("{role} {}", m.name).trim().to_string());
        let stone = tombstone(role, &m.content);
        let has_args = m.tool_calls.iter().any(|c| c.args != json!({}));
        if stone != m.content || has_args {
            in_live = true;
            texts.push(memory::envelope_body(&m.content).to_string());
            job_source.get_or_insert_with(|| m.content.clone());
            for c in &mut m.tool_calls {
                strings_of(&c.args, &mut texts);
                c.args = json!({}); // the call and its result stay paired
            }
            m.content = stone;
        }
    }
    let Some(what) = what else { bail!("no message {id} (ids look like m_123; find=<fragment> lists them)") };
    if !in_journal && !in_live {
        bail!("{id} is already cut");
    }
    if in_journal {
        let mut body = rows.join("\n");
        body.push('\n');
        rewrite(&journal, body.as_bytes())?;
    }
    let needles = needles(&texts);
    let mut changed = scrub(files, &needles)?;
    if let Some(log) = job_source.as_deref().and_then(|c| job_log(files, c)) {
        rewrite(&log, format!("{CUT}\n").as_bytes())?;
        let name = rel(files, &log);
        if !changed.contains(&name) {
            changed.push(name);
        }
    }
    let also = holders(&state, history, &needles, id).into_iter().map(|(i, _)| i).collect();
    Ok(Report { id: id.to_string(), what, files: changed, also })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ToolCall;

    const KEY: &str = "b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQ";

    fn setup(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bia-cut-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("state")).unwrap();
        std::fs::create_dir_all(d.join("logs/jobs")).unwrap();
        std::fs::create_dir_all(d.join("sessions")).unwrap();
        d
    }

    fn journal(d: &Path, rows: &[(&str, &str, &str)]) {
        let mut st = memory::Store::open(&d.join("state"));
        for (id, role, content) in rows {
            st.journal(id, role, "", content, "").unwrap();
        }
    }

    fn row(d: &Path, id: &str) -> String {
        memory::Store::open(&d.join("state")).journal_by_ids(&[id.to_string()])[0].content.clone()
    }

    fn user(id: &str, content: &str) -> Msg {
        let mut m = Msg::user(content.to_string());
        m.id = id.into();
        m
    }

    #[test]
    fn keeps_the_envelope_head_and_the_time() {
        let d = setup("head");
        let msg = format!("root|telegram:1|2026-11-01T22:23:59Z|2026-11-01T22:24:00Z|m9|{KEY}");
        journal(&d, &[("m_1", "user", &msg), ("m_2", "assistant", "fine")]);
        let mut live = vec![user("m_1", &msg)];
        let r = cut_message(&d, &mut live, "m_1").unwrap();
        let head = "root|telegram:1|2026-11-01T22:23:59Z|2026-11-01T22:24:00Z|m9|[cut]";
        assert_eq!(live[0].content, head);
        assert_eq!(row(&d, "m_1"), head);
        assert_eq!(row(&d, "m_2"), "fine", "other rows stay as they were");
        assert_eq!(memory::issued_of(&row(&d, "m_1")).as_deref(), Some("2026-11-01T22:23:59Z"));
        assert!(!r.text().contains(KEY));
        assert!(!std::fs::read_to_string(d.join("state/journal.jsonl")).unwrap().contains(KEY));
    }

    #[test]
    fn a_call_keeps_its_pairing_and_loses_its_arguments() {
        let d = setup("call");
        let call = ToolCall {
            id: "c1".into(),
            name: "bash".into(),
            args: json!({"command": format!("echo {KEY} > f")}),
            ..Default::default()
        };
        let mut a = Msg::assistant(format!("writing {KEY}"), vec![call]);
        a.id = "m_1".into();
        journal(&d, &[("m_1", "assistant", &a.content)]);
        std::fs::write(d.join("logs/jobs/00aa.log"), format!("$ echo {KEY} > f\n")).unwrap();
        let mut live = vec![a];
        cut_message(&d, &mut live, "m_1").unwrap();
        assert_eq!(live[0].content, CUT);
        assert_eq!(live[0].tool_calls[0].id, "c1");
        assert_eq!(live[0].tool_calls[0].name, "bash");
        assert_eq!(live[0].tool_calls[0].args, json!({}));
        assert!(!std::fs::read_to_string(d.join("logs/jobs/00aa.log")).unwrap().contains(KEY));
    }

    #[test]
    fn scrubs_other_stores_keeps_owner_and_mode_and_leaves_short_lines() {
        let d = setup("scrub");
        let line = format!("-----KEY \"{KEY}\"");
        let body = format!("ok\n{line}\nbye");
        journal(&d, &[("m_1", "tool", &body)]);
        let session = d.join("sessions/session-1.md");
        std::fs::write(&session, format!("### tool bash\nok\n{line}\nbye\n")).unwrap();
        std::fs::set_permissions(&session, std::fs::Permissions::from_mode(0o640)).unwrap();
        // JSON stores hold the same line escaped
        let refl = serde_json::to_string(&json!({"text": format!("seen {line}")})).unwrap();
        std::fs::write(d.join("state/reflections.jsonl"), format!("{refl}\n")).unwrap();
        let clean = d.join("logs/jobs/00bb.log");
        std::fs::write(&clean, "nothing here\n").unwrap();
        let before = std::fs::metadata(&clean).unwrap().modified().unwrap();

        let r = cut_message(&d, &mut [], "m_1").unwrap();

        let s = std::fs::read_to_string(&session).unwrap();
        assert!(!s.contains(KEY), "{s}");
        assert!(s.contains("### tool bash\nok\n"), "short lines stay: {s}");
        assert_eq!(std::fs::metadata(&session).unwrap().mode() & 0o777, 0o640);
        assert!(!std::fs::read_to_string(d.join("state/reflections.jsonl")).unwrap().contains(KEY));
        assert_eq!(std::fs::metadata(&clean).unwrap().modified().unwrap(), before, "untouched");
        assert!(r.files.contains(&"sessions/session-1.md".to_string()), "{:?}", r.files);
        let left: Vec<_> = std::fs::read_dir(d.join("sessions")).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(left.len(), 1, "no temp files: {left:?}");
    }

    #[test]
    fn a_job_result_takes_its_log_along() {
        let d = setup("job");
        let msg = format!("system|job:00cc|2026-11-01T22:23:59Z|2026-11-01T22:24:00Z|m9|ok\r\n{KEY}");
        journal(&d, &[("m_1", "user", &msg)]);
        let log = d.join("logs/jobs/00cc.log");
        std::fs::write(&log, format!("$ cat f\n\x1b[0m{}\x1b[0m\n", &KEY[..10])).unwrap();
        cut_message(&d, &mut [], "m_1").unwrap();
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "[cut]\n");
    }

    #[test]
    fn a_job_id_from_a_person_does_not_reach_a_file() {
        let d = setup("jobid");
        std::fs::write(d.join("logs/jobs/00cc.log"), "x\n").unwrap();
        assert!(job_log(&d, "alice|job:00cc|t|t|m|hi").is_none());
        assert!(job_log(&d, "system|job:../x|t|t|m|hi").is_none());
        assert!(job_log(&d, "system|job:00cc|t|t|m|hi").is_some());
    }

    #[test]
    fn reports_the_other_messages_holding_the_same_lines() {
        let d = setup("also");
        journal(&d, &[("m_1", "tool", KEY), ("m_2", "tool", KEY), ("m_3", "tool", "other text of some length")]);
        let r = cut_message(&d, &mut [], "m_1").unwrap();
        assert_eq!(r.also, vec!["m_2".to_string()]);
        assert_eq!(row(&d, "m_2"), KEY, "one message at a time");
        assert!(r.text().contains("m_2"));
    }

    #[test]
    fn find_lists_ids_without_the_text_and_skips_its_own_calls() {
        let d = setup("find");
        journal(&d, &[("m_1", "tool", KEY), ("m_2", "tool", "unrelated")]);
        std::fs::write(d.join("sessions/session-1.md"), KEY).unwrap();
        let call = ToolCall {
            id: "c".into(),
            name: "memory_cut".into(),
            args: json!({"find": "b3BlbnNzaC"}),
            ..Default::default()
        };
        let mut caller = Msg::assistant(String::new(), vec![call]);
        caller.id = "m_3".into();
        let out = find(&d, &[caller], "b3BlbnNzaC").unwrap();
        assert!(out.contains("m_1"), "{out}");
        assert!(!out.contains("m_2") && !out.contains("m_3"), "{out}");
        assert!(out.contains("sessions/session-1.md"), "{out}");
        assert!(!out.contains(KEY), "{out}");
        assert_eq!(find(&d, &[], "no such words").unwrap(), "nothing holds it");
        assert!(find(&d, &[], "ab").is_err());
    }

    #[test]
    fn unknown_and_repeated_ids_are_refused() {
        let d = setup("again");
        journal(&d, &[("m_1", "tool", "something to erase here")]);
        assert!(cut_message(&d, &mut [], "m_404").unwrap_err().to_string().contains("no message"));
        cut_message(&d, &mut [], "m_1").unwrap();
        assert!(cut_message(&d, &mut [], "m_1").unwrap_err().to_string().contains("already cut"));
    }

    #[test]
    fn the_saved_conversation_and_its_backup_lose_the_message() {
        let d = setup("saved");
        let path = d.join("state/conversation.json");
        let bak = path.with_extension("json.bak");
        journal(&d, &[("m_1", "tool", KEY)]);
        let mut live = vec![user("m_1", KEY), user("m_2", "stays")];
        crate::state::save(&path, "m", &live, vec![], vec![]).unwrap();
        crate::state::save(&path, "m", &live, vec![], vec![]).unwrap();
        assert!(std::fs::read_to_string(&bak).unwrap().contains(KEY));
        let (a, b) = (d.join("old-a"), d.join("old-b"));
        std::fs::hard_link(&path, &a).unwrap();
        std::fs::hard_link(&bak, &b).unwrap();

        // the order Repl::cut_call runs them in
        let old = hold(&[path.clone(), bak.clone()]);
        cut_message(&d, &mut live, "m_1").unwrap();
        crate::state::save(&path, "m", &live, vec![], vec![]).unwrap();
        crate::state::save(&path, "m", &live, vec![], vec![]).unwrap();
        zero(old);

        for f in [&path, &bak] {
            assert!(!std::fs::read_to_string(f).unwrap().contains(KEY), "{}", f.display());
        }
        for f in [&a, &b] {
            assert!(std::fs::read(f).unwrap().iter().all(|x| *x == 0), "{}", f.display());
        }
        let (_, h, note) = crate::state::load(&path).unwrap();
        assert!(note.is_none());
        assert_eq!(h.len(), 2);
        assert_eq!(h[1].content, "stays");
    }

    #[test]
    fn zero_overwrites_a_replaced_file() {
        let d = setup("zero");
        let p = d.join("f");
        std::fs::write(&p, KEY).unwrap();
        let hard = d.join("old-inode");
        std::fs::hard_link(&p, &hard).unwrap();
        rewrite(&p, b"new").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"new");
        assert_eq!(std::fs::read(&hard).unwrap(), vec![0u8; KEY.len()], "old bytes gone");
    }
}
