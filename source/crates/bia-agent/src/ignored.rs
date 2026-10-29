//! Ignored chats (chat_ignore): a busy group whose talk is not for the model. Its
//! messages are kept here (`state/ignored/<chat>.jsonl`), not in the context, the
//! journal or the inbox, so they cost nothing and do not reach dreams; the chat stays
//! unread. The rules (`state/ignore.json`, `bia_mcp::ignore`) are checked by the chat
//! MCPs after a voice note is transcribed: a wake pattern or a reply to bia delivers the
//! message with the few ignored ones before it. The model sees how many it missed and
//! takes in as many as it needs with chat_restore, newest first, deeper each call; what
//! it restores is journaled like any message.
use crate::console::Inbound;
use anyhow::{bail, Result};
use bia_mcp::ignore::{self, Rule};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Channel of restored messages and of the notes about ignored chats.
pub const CH_IGNORED: &str = "ignored";
/// How the notes about ignored chats begin (dreams leave them out).
pub const NOTE_HEAD: &str = "[ignored chats, kept from you:";
/// Buses whose chats can be ignored (never the console, web chats or bia's own channels).
const BUSES: &[&str] = &["telegram:", "whatsapp:", "messenger:", "instagram:"];
/// Ignored messages shown with one that wakes the model.
const WOKE_CONTEXT: usize = 5;
/// chat_restore: messages per call by default, and the most.
const RESTORE_DEFAULT: usize = 50;
const RESTORE_MAX: usize = 500;
/// A restored message longer than this is cut (the chat's history tool has it whole).
const CLIP: usize = 600;
/// A chat's file is cut to its newest half past this size.
const MAX_FILE: u64 = 16 << 20;
/// How often the model is told about new ignored messages, at most.
const NOTE_GAP_SECS: u64 = 15 * 60;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Row {
    /// the chat (file names cannot be turned back into channels)
    #[serde(default)]
    ch: String,
    id: String,
    identity: String,
    issued: u64,
    text: String,
    #[serde(default)]
    restored: bool,
}

#[derive(Default)]
struct Counts {
    /// ignored since the model was last told
    new: u64,
    woke: u64,
    first_new: u64,
}

struct State {
    dir: PathBuf,
    counts: BTreeMap<String, Counts>,
    last_note: u64,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

/// `state` is files/state (the rules file is found through BIA_FILES, like the MCPs do).
pub fn init(state: &std::path::Path) {
    *STATE.lock().unwrap_or_else(|e| e.into_inner()) =
        Some(State { dir: state.join("ignored"), counts: BTreeMap::new(), last_note: now() });
}

fn with<T>(f: impl FnOnce(&mut State) -> T) -> Option<T> {
    STATE.lock().unwrap_or_else(|e| e.into_inner()).as_mut().map(f)
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn can_ignore(channel: &str) -> bool {
    BUSES.iter().any(|b| channel.strip_prefix(b).is_some_and(|rest| !rest.is_empty()))
}

fn file(dir: &std::path::Path, channel: &str) -> PathBuf {
    let name: String = channel.chars().map(|c| if c.is_ascii_alphanumeric() || "-_.@".contains(c) { c } else { '_' }).collect();
    dir.join(format!("{name}.jsonl"))
}

fn read(dir: &std::path::Path, channel: &str) -> Vec<Row> {
    std::fs::read_to_string(file(dir, channel))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

fn write(dir: &std::path::Path, channel: &str, rows: &[Row]) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let p = file(dir, channel);
    let tmp = p.with_extension("jsonl.tmp");
    let body: String = rows.iter().filter_map(|r| serde_json::to_string(r).ok()).map(|l| l + "\n").collect();
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, &p)?;
    Ok(())
}

/// An ignored message: kept for chat_restore, counted for the model's next note.
pub fn keep(ib: &Inbound) {
    with(|st| {
        let row = Row { ch: ib.channel.clone(), id: ib.id.clone(), identity: ib.identity.clone(), issued: secs(ib.issued), text: ib.text.clone(), restored: false };
        let p = file(&st.dir, &ib.channel);
        let _ = std::fs::create_dir_all(&st.dir);
        if let Ok(line) = serde_json::to_string(&row) {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
                let _ = writeln!(f, "{line}");
            }
        }
        if std::fs::metadata(&p).map(|m| m.len() > MAX_FILE).unwrap_or(false) {
            let rows = read(&st.dir, &ib.channel);
            let _ = write(&st.dir, &ib.channel, &rows[rows.len() / 2..]);
        }
        let c = st.counts.entry(ib.channel.clone()).or_default();
        if c.new == 0 {
            c.first_new = row.issued;
        }
        c.new += 1;
    });
}

fn line(r: &Row) -> String {
    let text = r.text.split_whitespace().collect::<Vec<_>>().join(" ");
    let text = if text.chars().count() > CLIP { text.chars().take(CLIP).collect::<String>() + "…" } else { text };
    format!("[{} id {}] {}: {text}", hm(r.issued), r.id, r.identity)
}

/// `10-29 14:05` in the machine's timezone.
fn hm(t: u64) -> String {
    let (min, hour, day, month, _) = crate::clock::shifted_fields(t, crate::clock::offset_east_at(t));
    format!("{month:02}-{day:02} {hour:02}:{min:02}")
}

/// A message that woke the model in an ignored chat: what woke it and the ignored
/// messages just before it go in front (those then count as restored).
pub fn woke(mut ib: Inbound, why: &str) -> Inbound {
    let head = with(|st| {
        st.counts.entry(ib.channel.clone()).or_default().woke += 1;
        let mut rows = read(&st.dir, &ib.channel);
        let open: Vec<usize> = (0..rows.len()).filter(|&i| !rows[i].restored).collect();
        let shown: Vec<usize> = open.iter().rev().take(WOKE_CONTEXT).rev().copied().collect();
        let mut head = format!("[ignored chat {}: this message woke you ({why}); {} ignored message(s) not shown", ib.channel, open.len());
        if !shown.is_empty() {
            head.push_str(&format!(", the last {}:", shown.len()));
            for &i in &shown {
                head.push_str("\n  ");
                head.push_str(&line(&rows[i]));
                rows[i].restored = true;
            }
            let _ = write(&st.dir, &ib.channel, &rows);
        }
        head.push_str("\nchat_restore gives more; chat_unignore stops ignoring]");
        head
    });
    if let Some(h) = head {
        ib.text = format!("{h}\n{}", ib.text);
    }
    ib
}

/// chat_ignore
pub fn ignore(chat: &str, wake: &[String], why: &str) -> Result<String> {
    if !can_ignore(chat) {
        bail!("{chat} cannot be ignored: only telegram, whatsapp, messenger and instagram chats (a chat id as channels lists it, e.g. telegram:-1001234567890)");
    }
    for p in wake {
        ignore::compile(p).map_err(anyhow::Error::msg)?;
    }
    let mut rules = ignore::load();
    let again = rules.chats.contains_key(chat);
    let since = rules.chats.get(chat).map(|r| r.since).unwrap_or_else(now);
    rules.chats.insert(chat.to_string(), Rule { since, wake: wake.to_vec(), note: why.to_string(), title: String::new() });
    ignore::save(&rules)?;
    let wakes = if wake.is_empty() { "only replies to your messages".to_string() } else { format!("replies to your messages and {}", wake.join(", ")) };
    Ok(format!(
        "{} {chat}: its messages are kept from you and it stays unread. Wakes you: {wakes} (checked on voice transcripts too). You are told how many you miss; chat_restore(chat) shows them.",
        if again { "Still ignoring" } else { "Ignoring" }
    ))
}

/// chat_unignore
pub fn unignore(chat: &str) -> Result<String> {
    let mut rules = ignore::load();
    let was = rules.chats.remove(chat).is_some();
    ignore::save(&rules)?;
    let open = with(|st| read(&st.dir, chat).iter().filter(|r| !r.restored).count()).unwrap_or(0);
    Ok(match (was, open) {
        (false, _) => format!("{chat} was not ignored"),
        (true, 0) => format!("{chat}: its messages reach you again"),
        (true, n) => format!("{chat}: its messages reach you again. {n} ignored message(s) were not shown; chat_restore(chat) shows them, newest first."),
    })
}

/// chat_restore: the newest `count` ignored messages not shown yet, oldest first, as one
/// message for the model (journaled like any other). Returns that text and a summary.
pub fn restore(chat: &str, count: Option<usize>) -> Result<(String, String)> {
    let n = count.unwrap_or(RESTORE_DEFAULT).clamp(1, RESTORE_MAX);
    let out = with(|st| -> Result<(String, String)> {
        let mut rows = read(&st.dir, chat);
        let open: Vec<usize> = (0..rows.len()).filter(|&i| !rows[i].restored).collect();
        if open.is_empty() {
            bail!("no ignored messages from {chat} left to show{}", if rows.is_empty() { "" } else { " (all shown already; the chat's history tool goes further back)" });
        }
        let take: Vec<usize> = open.iter().rev().take(n).rev().copied().collect();
        let older = open.len() - take.len();
        let (from, to) = (rows[take[0]].issued, rows[*take.last().unwrap()].issued);
        let mut text = format!("[restored from ignored chat {chat}: {} message(s), {}–{}, oldest first", take.len(), hm(from), hm(to));
        text.push_str(&if older > 0 { format!("; {older} older ones still not shown (chat_restore again)]") } else { "]".to_string() });
        for &i in &take {
            text.push('\n');
            text.push_str(&line(&rows[i]));
            rows[i].restored = true;
        }
        write(&st.dir, chat, &rows)?;
        let summary = format!("restored {} message(s) from {chat} ({}–{}); they follow as a system message{}", take.len(), hm(from), hm(to), if older > 0 { format!("; {older} older ones left") } else { String::new() });
        Ok((text, summary))
    });
    out.unwrap_or_else(|| bail!("ignored chats are not set up"))
}

/// chat_ignored: chats ignored now, and others with ignored messages not shown.
pub fn list() -> String {
    let rules = ignore::load();
    let rows = with(|st| {
        let mut chans: Vec<String> = rules.chats.keys().cloned().collect();
        // chats no longer ignored with messages still kept
        if let Ok(rd) = std::fs::read_dir(&st.dir) {
            for e in rd.flatten().filter(|e| e.path().extension().is_some_and(|x| x == "jsonl")) {
                let first = std::fs::read_to_string(e.path()).ok().and_then(|t| t.lines().next().and_then(|l| serde_json::from_str::<Row>(l).ok()));
                chans.extend(first.map(|r| r.ch).filter(|c| !c.is_empty()));
            }
        }
        chans.sort();
        chans.dedup();
        chans
            .iter()
            .map(|c| {
                let all = read(&st.dir, c);
                let open = all.iter().filter(|r| !r.restored).count();
                match rules.chats.get(c) {
                    Some(r) => format!(
                        "{c}  ignored since {}  {open} not shown ({} kept)  wake: {}{}",
                        hm(r.since),
                        all.len(),
                        if r.wake.is_empty() { "replies to you".to_string() } else { format!("replies to you, {}", r.wake.join(", ")) },
                        if r.note.is_empty() { String::new() } else { format!("  ({})", r.note) }
                    ),
                    None if open == 0 => String::new(),
                    None => format!("{c}  not ignored now  {open} not shown"),
                }
            })
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
    })
    .unwrap_or_default();
    if rows.is_empty() {
        "no chat is ignored".into()
    } else {
        rows.join("\n")
    }
}

/// For the model's next turn, at most every quarter of an hour: how many messages it
/// missed in ignored chats since it was last told. None when there is nothing new.
pub fn note() -> Option<String> {
    with(|st| {
        let t = now();
        if t.saturating_sub(st.last_note) < NOTE_GAP_SECS || st.counts.values().all(|c| c.new == 0) {
            return None;
        }
        st.last_note = t;
        let parts: Vec<String> = st
            .counts
            .iter_mut()
            .filter(|(_, c)| c.new > 0)
            .map(|(ch, c)| {
                let s = format!(
                    "{ch} {} new since {}{}",
                    c.new,
                    hm(c.first_new),
                    if c.woke > 0 { format!(" ({} woke you)", c.woke) } else { String::new() }
                );
                *c = Counts::default();
                s
            })
            .collect();
        Some(format!("{NOTE_HEAD} {}. chat_restore(chat) shows them; chat_unignore(chat) stops ignoring]", parts.join("; ")))
    })
    .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ib(chan: &str, id: &str, text: &str, t: u64) -> Inbound {
        Inbound { identity: "Anna (@anna)".into(), channel: chan.into(), issued: UNIX_EPOCH + std::time::Duration::from_secs(t), id: id.into(), text: text.into() }
    }

    #[test]
    fn keep_wake_restore() {
        let dir = std::env::temp_dir().join(format!("bia-ignored-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::env::set_var("BIA_FILES", &dir);
        init(&dir.join("state"));
        let g = "telegram:-1001";
        assert!(can_ignore(g) && !can_ignore("web:c1") && !can_ignore("console") && !can_ignore("telegram:"));
        assert!(ignore("console", &[], "").is_err());
        assert!(ignore(g, &["(bad".into()], "").is_err());
        assert!(ignore(g, &[r"\bfred\b".into()], "crypto talk").unwrap().starts_with("Ignoring"));
        assert!(ignore::load().chats.contains_key(g));
        for i in 0..12 {
            keep(&ib(g, &i.to_string(), &format!("msg {i}"), 1_000 + i));
        }
        // a wake-up shows the last five before it
        let w = woke(ib(g, "12", "fred, look", 1_100), "wake pattern fred");
        assert!(w.text.contains("12 ignored message(s) not shown, the last 5:"), "{}", w.text);
        assert!(w.text.contains("msg 7") && w.text.contains("msg 11") && !w.text.contains("msg 6") && w.text.ends_with("fred, look"));
        // restore goes deeper each call, newest first
        let (t, s) = restore(g, Some(4)).unwrap();
        assert!(t.contains("msg 3") && t.contains("msg 6") && !t.contains("msg 7") && !t.contains("msg 2"), "{t}");
        assert!(t.contains("3 older ones still not shown") && s.contains("3 older ones left"));
        let (t, _) = restore(g, None).unwrap();
        assert!(t.contains("msg 0") && t.contains("msg 2") && !t.contains("older"));
        assert!(restore(g, None).is_err());
        // the note: counts once, then quiet
        with(|st| st.last_note = 0);
        let n = note().unwrap();
        assert!(n.contains(&format!("{g} 12 new")) && n.contains("(1 woke you)"), "{n}");
        assert!(note().is_none());
        assert!(list().contains("crypto talk"));
        assert!(unignore(g).unwrap().contains("reach you again"));
        assert!(!ignore::load().chats.contains_key(g));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
