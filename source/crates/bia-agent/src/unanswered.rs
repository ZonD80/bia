//! Unanswered human messages. Each human inbound stays open until the model closes it
//! with `answered=[ids]` on say/react. A per-channel backoff decides when an
//! `unanswered:` notice nudges the model; a channel that ignores 5 nudges gets one
//! automatic "still working on it" and is muted until the user writes again or a job
//! event arrives. Persisted to state/unanswered.json.
//!
//! A chat in a voice call is nudged every [`CALL_SECS`] without backoff, and gets the
//! spoken fallback after one ignored nudge: silence on a call reads as a dropped line.
//! Jobs carry the messages they work for (`for_ids`, jobs.rs); the notice lists them
//! under each message, so the model knows which person waits for which task.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const TICK_SECS: u64 = 20;
/// Nudge interval of a chat in a voice call (fixed, no backoff).
pub const CALL_SECS: u64 = 10;
const MAX_INTERVAL_SECS: u64 = 600;
const MAX_SILENT_NUDGES: u32 = 5;
/// Ignored nudges before a call gets the spoken fallback.
const MAX_SILENT_CALL: u32 = 1;
/// Longest bia is taken to be speaking into a call without [you finished speaking]
/// (an event lost with a crashed MCP must not silence the reminders for good).
pub const SPEAK_CAP_SECS: u64 = 60;
pub const FALLBACK_TEXT: &str = "Still working on it.";

/// "Still working on it." in `lang` (its first part, e.g. es-ES → es); English otherwise.
pub fn fallback_text(lang: &str) -> &'static str {
    let l = lang.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase();
    match l.as_str() {
        "ru" => "Ещё работаю над этим.",
        "uk" => "Ще працюю над цим.",
        "es" => "Sigo trabajando en ello.",
        "ca" => "Encara hi estic treballant.",
        "fr" => "J'y travaille encore.",
        "de" => "Ich arbeite noch daran.",
        "pt" => "Ainda estou trabalhando nisso.",
        "it" => "Ci sto ancora lavorando.",
        _ => FALLBACK_TEXT,
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub id: String,
    pub channel: String,
    pub identity: String,
    pub received: u64,
    /// sent in a group chat (statuses there name who they are for)
    #[serde(default)]
    pub group: bool,
    /// language of a transcribed phrase ([speech xx]), for a spoken fallback
    #[serde(default)]
    pub lang: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
struct Chan {
    channel: String,
    /// last time the model said or reacted anything on this channel (0 = never)
    last_reply: u64,
    next_due: u64,
    /// backoff step: interval is TICK_SECS * 2^step, capped
    step: u32,
    /// nudges since the last reply on this channel
    silent: u32,
    muted: bool,
}

#[derive(Serialize, Deserialize, Default)]
struct File {
    #[serde(default)]
    open: Vec<Entry>,
    #[serde(default)]
    chans: Vec<Chan>,
    #[serde(default)]
    calls: Vec<String>,
}

/// The automatic status for a chat that ignored its nudges.
#[derive(Debug, Clone, PartialEq)]
pub struct Fallback {
    /// identity of the oldest open message (the one say goes to)
    pub identity: String,
    pub channel: String,
    /// in a group: everyone with an open message there, oldest first (named in the text)
    pub names: Vec<String>,
    /// language of the newest transcribed phrase ("" = not known)
    pub lang: String,
    pub call: bool,
}

impl Fallback {
    /// The text to send: in the chat's language, naming the people in a group.
    pub fn text(&self) -> String {
        let t = fallback_text(&self.lang);
        if self.names.is_empty() {
            return t.to_string();
        }
        let who: Vec<String> = self.names.iter().map(|n| format!("@{n}")).collect();
        format!("{}, {}{}", who.join(" "), t[..1].to_lowercase(), &t[1..])
    }
}

/// A running job and the open message it works for (built from jobs.rs for the notice).
#[derive(Debug, Clone, PartialEq)]
pub struct Work {
    pub channel: String,
    pub id: String,
    /// e.g. "job:0a12 chrome_read, running 25s"
    pub job: String,
}

/// What the REPL should do on a check.
#[derive(Debug, Default, PartialEq)]
pub struct Due {
    /// open entries on channels whose nudge is due (oldest first)
    pub nudge: Vec<Entry>,
    /// last reply per nudged channel (0 = none), parallel to the channels in `nudge`
    pub last_reply: Vec<(String, u64)>,
    /// chats that get the one automatic fallback and are now muted
    pub fallback: Vec<Fallback>,
    /// nudged channels that are in a voice call
    pub calls: Vec<String>,
}

pub struct Set {
    path: Option<PathBuf>,
    open: Vec<Entry>,
    chans: Vec<Chan>,
    /// channels in a voice call now
    calls: Vec<String>,
    /// (channel, since): bia is speaking into that call; nothing is asked or said there
    /// until it finished (not persisted: a restart ends the speech anyway)
    speaking: Vec<(String, u64)>,
}

fn interval(step: u32) -> u64 {
    TICK_SECS
        .saturating_mul(1u64 << step.min(16))
        .min(MAX_INTERVAL_SECS)
}

impl Set {
    pub fn load(state_dir: &Path) -> Set {
        let path = state_dir.join("unanswered.json");
        let f: File = bia_mcp::statefile::read_or_default(&path);
        // The physical console is not tracked (older state may still list it).
        let mut set = Set {
            path: Some(path),
            open: f.open,
            chans: f.chans,
            calls: f.calls,
            speaking: vec![],
        };
        set.open.retain(|e| e.channel != "console");
        set.prune();
        set
    }

    #[cfg(test)]
    pub(crate) fn mem() -> Set {
        Set {
            path: None,
            open: vec![],
            chans: vec![],
            calls: vec![],
            speaking: vec![],
        }
    }

    fn persist(&self) {
        let Some(path) = &self.path else { return };
        let f = File {
            open: self.open.clone(),
            chans: self.chans.clone(),
            calls: self.calls.clone(),
        };
        let _ = bia_mcp::statefile::write_json(path, &f);
    }

    fn chan(&mut self, channel: &str) -> &mut Chan {
        if let Some(i) = self.chans.iter().position(|c| c.channel == channel) {
            return &mut self.chans[i];
        }
        self.chans.push(Chan {
            channel: channel.into(),
            ..Default::default()
        });
        self.chans.last_mut().unwrap()
    }

    /// Drop channel state for channels without open messages.
    fn prune(&mut self) {
        let open = &self.open;
        self.chans
            .retain(|c| open.iter().any(|e| e.channel == c.channel));
    }

    pub fn in_call(&self, channel: &str) -> bool {
        self.calls.iter().any(|c| c == channel)
    }

    /// The first wait of a channel (after a new message or a job event).
    fn first_secs(&self, channel: &str) -> u64 {
        if self.in_call(channel) {
            CALL_SECS
        } else {
            TICK_SECS
        }
    }

    /// The wait after a nudge or a status at backoff `step`.
    fn next_secs(&self, channel: &str, step: u32) -> u64 {
        if self.in_call(channel) {
            CALL_SECS
        } else {
            interval(step)
        }
    }

    /// A voice call on `channel` started (`on`) or ended. While it lasts, open messages
    /// there are nudged every CALL_SECS.
    pub fn set_call(&mut self, channel: &str, on: bool, now: u64) {
        let was = self.in_call(channel);
        if on == was {
            return;
        }
        if on {
            self.calls.push(channel.into());
            if let Some(c) = self.chans.iter_mut().find(|c| c.channel == channel) {
                c.muted = false;
                c.silent = 0;
                c.next_due = c.next_due.min(now + CALL_SECS);
            }
        } else {
            self.calls.retain(|c| c != channel);
            self.speaking.retain(|(c, _)| c != channel);
        }
        self.persist();
    }

    /// bia started speaking into the call on `channel` (a say, or the spoken fallback).
    pub fn speaking_started(&mut self, channel: &str, now: u64) {
        if !self.in_call(channel) {
            return;
        }
        self.speaking.retain(|(c, _)| c != channel);
        self.speaking.push((channel.into(), now));
    }

    /// What bia said into the call on `channel` has played (or was cut): the wait for the
    /// next status starts now, as if it had just replied.
    pub fn speaking_done(&mut self, channel: &str, now: u64) {
        let was = self.speaking.iter().any(|(c, _)| c == channel);
        self.speaking.retain(|(c, _)| c != channel);
        if was {
            if let Some(c) = self.chans.iter_mut().find(|c| c.channel == channel) {
                c.last_reply = now;
                c.next_due = c.next_due.max(now + CALL_SECS);
            }
            self.persist();
        }
    }

    /// bia is speaking into the call on `channel` now (for at most SPEAK_CAP_SECS).
    pub fn is_speaking(&self, channel: &str, now: u64) -> bool {
        self.speaking.iter().any(|(c, t)| c == channel && now.saturating_sub(*t) < SPEAK_CAP_SECS)
    }

    /// A job result for people on `channel`: may it ask the model for a status now? Only
    /// when the chat's reminder is due (not while bia speaks into its call); asking then
    /// counts as that reminder, so several results and the notice ask once. Returns how
    /// long they have heard nothing.
    pub fn claim_status(&mut self, channel: &str, now: u64) -> Option<u64> {
        if self.is_speaking(channel, now) {
            return None;
        }
        let call = self.in_call(channel);
        let oldest = self.open.iter().filter(|e| e.channel == channel).map(|e| e.received).min()?;
        let c = self.chans.iter_mut().find(|c| c.channel == channel)?;
        if c.muted || c.next_due > now {
            return None;
        }
        let waited = now.saturating_sub(oldest.max(c.last_reply));
        c.silent += 1;
        if !call {
            c.step = c.step.saturating_add(1);
        }
        c.next_due = now + if call { CALL_SECS } else { interval(c.step) };
        self.persist();
        Some(waited)
    }

    /// Open messages of `identity` on `channel`, oldest first (what a call made right
    /// after their message works for).
    pub fn open_of(&self, identity: &str, channel: &str) -> Vec<Entry> {
        self.open
            .iter()
            .filter(|e| e.channel == channel && e.identity == identity)
            .cloned()
            .collect()
    }

    /// The open message `id` names: on `prefer` (the channel of the turn) if there, else
    /// the only one with that id anywhere (platform ids repeat across chats).
    pub fn find(&self, id: &str, prefer: &str) -> Option<Entry> {
        let id = id.trim();
        if let Some(e) = self.open.iter().find(|e| e.id == id && e.channel == prefer) {
            return Some(e.clone());
        }
        let hits: Vec<&Entry> = self.open.iter().filter(|e| e.id == id).collect();
        (hits.len() == 1).then(|| hits[0].clone())
    }

    /// When bia last said anything on `channel` (0 = never, or nothing open there).
    #[cfg(test)]
    pub fn last_reply(&self, channel: &str) -> u64 {
        self.chans.iter().find(|c| c.channel == channel).map(|c| c.last_reply).unwrap_or(0)
    }

    pub fn len(&self) -> usize {
        self.open.len()
    }

    pub fn muted_len(&self) -> usize {
        self.open
            .iter()
            .filter(|e| self.chans.iter().any(|c| c.channel == e.channel && c.muted))
            .count()
    }

    /// A human message arrived: open it and restart the channel's backoff.
    #[cfg(test)]
    pub fn add(&mut self, id: &str, channel: &str, identity: &str, now: u64) {
        self.add_msg(id, channel, identity, now, false, "");
    }

    /// [`Set::add`] with what the fallback needs: sent in a group, the phrase's language.
    pub fn add_msg(&mut self, id: &str, channel: &str, identity: &str, now: u64, group: bool, lang: &str) {
        // someone else already waits here: their status stays due when it was (a busy group
        // must not push it back); the same person writing on restarts the wait
        let others_wait = self.open.iter().any(|e| e.channel == channel && e.identity != identity);
        if !self.open.iter().any(|e| e.id == id && e.channel == channel) {
            self.open.push(Entry {
                id: id.into(),
                channel: channel.into(),
                identity: identity.into(),
                received: now,
                group,
                lang: lang.into(),
            });
        }
        let first = self.first_secs(channel);
        let c = self.chan(channel);
        let keep = others_wait && !c.muted && c.next_due > now;
        c.step = 0;
        c.silent = 0;
        c.muted = false;
        c.next_due = if keep { c.next_due.min(now + first) } else { now + first };
        self.persist();
    }

    /// The automatic fallback went out on `channel`: it counts as bia speaking there (the
    /// chat stays muted until a new message or a job event).
    pub fn on_fallback(&mut self, channel: &str, now: u64) {
        if let Some(c) = self.chans.iter_mut().find(|c| c.channel == channel) {
            c.last_reply = now;
            self.persist();
        }
    }

    /// The model said or reacted on `channel`. Closes `answered` ids; the rest stay open
    /// and the next nudge moves out by the current interval (no reset to 15s).
    /// Returns the ids that were closed.
    pub fn on_reply(&mut self, channel: &str, answered: &[String], now: u64) -> Vec<String> {
        let mut closed = vec![];
        for id in answered {
            let id = id.trim();
            if id.is_empty() {
                continue;
            }
            // Prefer the reply's channel (platform ids repeat across chats); else a unique match.
            let pos = self
                .open
                .iter()
                .position(|e| e.id == id && e.channel == channel)
                .or_else(|| {
                    let hits: Vec<usize> = self
                        .open
                        .iter()
                        .enumerate()
                        .filter(|(_, e)| e.id == id)
                        .map(|(i, _)| i)
                        .collect();
                    (hits.len() == 1).then(|| hits[0])
                });
            if let Some(i) = pos {
                closed.push(self.open.remove(i).id);
            }
        }
        let touched = self.open.iter().any(|e| e.channel == channel);
        if touched {
            let step = self.chan(channel).step;
            let wait = self.next_secs(channel, step);
            let c = self.chan(channel);
            c.last_reply = now;
            c.silent = 0;
            c.muted = false;
            c.next_due = now + wait;
        }
        if touched || !closed.is_empty() {
            self.prune();
            self.persist();
        }
        closed
    }

    /// Other chats on the same bus with open messages, when `channel` itself has none
    /// (a reply that probably went to the wrong chat, e.g. telegram instead of whatsapp).
    pub fn has_open(&self, channel: &str) -> bool {
        self.open.iter().any(|e| e.channel == channel)
    }

    pub fn open_elsewhere(&self, channel: &str) -> Vec<String> {
        if self.open.iter().any(|e| e.channel == channel) {
            return vec![];
        }
        let bus = |c: &str| c.split(':').next().unwrap_or(c).to_string();
        let mut out: Vec<String> = vec![];
        for e in &self.open {
            if bus(&e.channel) == bus(channel) && !out.contains(&e.channel) {
                out.push(e.channel.clone());
            }
        }
        out
    }

    /// A job finished or reported stale?: muted channels get nudged again from 15s.
    pub fn wake(&mut self, now: u64) {
        let mut changed = false;
        let calls = self.calls.clone();
        for c in self.chans.iter_mut().filter(|c| c.muted) {
            c.muted = false;
            c.step = 0;
            c.silent = 0;
            c.next_due = now + if calls.contains(&c.channel) { CALL_SECS } else { TICK_SECS };
            changed = true;
        }
        if changed {
            self.persist();
        }
    }

    /// Seconds until the earliest due channel (None when nothing can come due).
    pub fn secs_until_due(&self, now: u64) -> Option<u64> {
        self.chans
            .iter()
            .filter(|c| !c.muted && self.open.iter().any(|e| e.channel == c.channel))
            .map(|c| {
                // while bia speaks there: look again when the cap would run out (a
                // [you finished speaking] wakes the loop by itself)
                match self.speaking.iter().find(|(ch, _)| *ch == c.channel) {
                    Some((_, t)) if now.saturating_sub(*t) < SPEAK_CAP_SECS => (t + SPEAK_CAP_SECS).saturating_sub(now).max(1),
                    _ => c.next_due.saturating_sub(now),
                }
            })
            .min()
    }

    /// Collect due nudges and fallbacks, advancing the backoff of every channel involved.
    pub fn check(&mut self, now: u64) -> Due {
        let mut due = Due::default();
        let mut nudged: Vec<String> = vec![];
        let speaking: Vec<String> = self
            .speaking
            .iter()
            .filter(|(_, t)| now.saturating_sub(*t) < SPEAK_CAP_SECS)
            .map(|(c, _)| c.clone())
            .collect();
        for c in self.chans.iter_mut() {
            if c.muted || c.next_due > now {
                continue;
            }
            // bia is speaking into this call: no reminder and no fallback over its words;
            // the wait starts again when it finished (speaking_done)
            if speaking.contains(&c.channel) {
                continue;
            }
            let mine: Vec<&Entry> = self.open.iter().filter(|e| e.channel == c.channel).collect();
            let Some(first) = mine.first() else {
                continue;
            };
            let call = self.calls.contains(&c.channel);
            if c.silent >= if call { MAX_SILENT_CALL } else { MAX_SILENT_NUDGES } {
                c.muted = true;
                let mut names: Vec<String> = vec![];
                if mine.iter().any(|e| e.group) {
                    for e in &mine {
                        if !names.contains(&e.identity) {
                            names.push(e.identity.clone());
                        }
                    }
                }
                let lang = mine.iter().rev().find(|e| !e.lang.is_empty()).map(|e| e.lang.clone()).unwrap_or_default();
                due.fallback.push(Fallback {
                    identity: first.identity.clone(),
                    channel: c.channel.clone(),
                    names,
                    lang,
                    call,
                });
                continue;
            }
            c.silent += 1;
            if !call {
                c.step = c.step.saturating_add(1);
            }
            c.next_due = now + if call { CALL_SECS } else { interval(c.step) };
            due.last_reply.push((c.channel.clone(), c.last_reply));
            if call {
                due.calls.push(c.channel.clone());
            }
            nudged.push(c.channel.clone());
        }
        due.nudge = self
            .open
            .iter()
            .filter(|e| nudged.contains(&e.channel))
            .cloned()
            .collect();
        due.nudge.sort_by_key(|e| e.received);
        if !due.nudge.is_empty() || !due.fallback.is_empty() {
            self.persist();
        }
        due
    }

    pub fn clear(&mut self) {
        self.open.clear();
        self.chans.clear();
        self.persist();
    }

    /// One line per open message (for /unanswered).
    pub fn format_list(&self, now: u64) -> String {
        if self.open.is_empty() {
            return "(no unanswered messages)".into();
        }
        self.open
            .iter()
            .map(|e| {
                let c = self.chans.iter().find(|c| c.channel == e.channel);
                let muted = c.map(|c| c.muted).unwrap_or(false);
                let next = c.map(|c| c.next_due.saturating_sub(now)).unwrap_or(0);
                format!(
                    "{} #{} {}, open {}{}",
                    e.channel,
                    e.id,
                    e.identity,
                    ago(now.saturating_sub(e.received)),
                    if muted {
                        ", muted".to_string()
                    } else {
                        format!(", next nudge in {next}s")
                    }
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

pub fn ago(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else {
        format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60)
    }
}

/// Body of the `unanswered:` system notice. One block per chat (channel): its open
/// messages with who sent them and the jobs working for each, then one status call for
/// the whole chat (a group hears one status for everyone, not one per person) and the
/// close call per person.
pub fn notice(due: &Due, jobs: &str, work: &[Work], now: u64) -> String {
    let mut s = String::from("unanswered:\n");
    // chats in order of their oldest message; the close call is spelled out because
    // models did not link the listed ids to say(answered=…) on their own
    let mut chans: Vec<&str> = Vec::new();
    for e in &due.nudge {
        if !chans.contains(&e.channel.as_str()) {
            chans.push(e.channel.as_str());
        }
    }
    for channel in chans {
        let mine: Vec<&Entry> = due.nudge.iter().filter(|e| e.channel == channel).collect();
        for e in &mine {
            notice_line(&mut s, due, e, now);
            for w in work.iter().filter(|w| w.channel == e.channel && w.id == e.id) {
                s.push_str(&format!("    working for it: {}\n", w.job));
            }
        }
        let mut people: Vec<&str> = vec![];
        for e in &mine {
            if !people.contains(&e.identity.as_str()) {
                people.push(e.identity.as_str());
            }
        }
        let call = due.calls.iter().any(|c| c == channel);
        let status = if call {
            "<one short spoken sentence: what you are doing now>"
        } else if people.len() > 1 {
            "<one short status for everyone waiting here, @Name each>"
        } else {
            "<short status>"
        };
        s.push_str(&format!(
            "  → still working: say(identity={:?}, channel={channel:?}, message=\"{status}\"){}\n",
            people[0],
            if call { " — in a voice call: say it now, every pause feels like a dropped line" } else { "" }
        ));
        for who in people {
            let ids: Vec<String> = mine.iter().filter(|e| e.identity == who).map(|e| format!("{:?}", e.id)).collect();
            s.push_str(&format!(
                "  → done: say(identity={who:?}, channel={channel:?}, message=\"<answer>\", answered=[{}])\n",
                ids.join(", ")
            ));
        }
    }
    s.push_str(&format!(
        "running jobs: {}\n",
        if jobs.is_empty() { "none" } else { jobs }
    ));
    s.push_str("Use each channel exactly as listed. Task still running: send the → still working call (it keeps the messages open). Task done: send the → done call. Already answered and done: the → done call without message closes them and sends nothing. Do not repeat an answer. Tag each function call with the ids it works for: for_ids=[…].");
    s
}

fn notice_line(s: &mut String, due: &Due, e: &Entry, now: u64) {
    let last = due
        .last_reply
        .iter()
        .find(|(ch, _)| *ch == e.channel)
        .map(|(_, t)| *t)
        .unwrap_or(0);
    let status = if last == 0 || last < e.received {
        "no reply yet".to_string()
    } else {
        format!("last status {} ago", ago(now.saturating_sub(last)))
    };
    s.push_str(&format!(
        "  channel={} id={} from {}, open {}, {status}\n",
        e.channel,
        e.id,
        e.identity,
        ago(now.saturating_sub(e.received))
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[Entry]) -> Vec<&str> {
        v.iter().map(|e| e.id.as_str()).collect()
    }

    #[test]
    fn nudge_backoff_and_reply_postpones() {
        let mut s = Set::mem();
        s.add("1", "telegram:5", "alice", 100);
        assert!(s.check(115).nudge.is_empty());
        let d = s.check(120);
        assert_eq!(ids(&d.nudge), ["1"]);
        // step 1 → 40s
        assert!(s.check(155).nudge.is_empty());
        assert_eq!(ids(&s.check(160).nudge), ["1"]);
        // step 2 → 80s; a status reply postpones by the current interval, no reset
        s.on_reply("telegram:5", &[], 170);
        assert!(s.check(245).nudge.is_empty());
        assert_eq!(ids(&s.check(250).nudge), ["1"]);
    }

    #[test]
    fn interval_caps() {
        assert_eq!(interval(0), 20);
        assert_eq!(interval(3), 160);
        assert_eq!(interval(10), MAX_INTERVAL_SECS);
        assert_eq!(interval(40), MAX_INTERVAL_SECS);
    }

    #[test]
    fn answered_closes() {
        let mut s = Set::mem();
        s.add("1", "web", "root", 0);
        s.add("2", "web", "root", 1);
        assert_eq!(s.on_reply("web", &["1".into()], 5), ["1"]);
        assert_eq!(s.len(), 1);
        assert_eq!(s.on_reply("web", &["2".into()], 6), ["2"]);
        assert!(s.open.is_empty());
        assert!(s.chans.is_empty());
        assert!(s.check(1000).nudge.is_empty());
    }

    #[test]
    fn answered_prefers_reply_channel_then_unique() {
        let mut s = Set::mem();
        s.add("7", "telegram:1", "a", 0);
        s.add("7", "telegram:2", "b", 0);
        s.add("9", "web", "root", 0);
        assert_eq!(s.on_reply("telegram:2", &["7".into()], 1), ["7"]);
        assert!(s.open.iter().any(|e| e.channel == "telegram:1"));
        // ambiguous ids elsewhere are not guessed; a unique one is closed
        s.add("7", "telegram:3", "c", 0);
        assert!(s.on_reply("console", &["7".into()], 2).is_empty());
        assert_eq!(s.on_reply("console", &["9".into()], 2), ["9"]);
    }

    #[test]
    fn fallback_once_then_muted_until_user_or_job() {
        let mut s = Set::mem();
        s.add("1", "whatsapp:9", "bob", 0);
        for _ in 0..MAX_SILENT_NUDGES {
            let t = s.chans[0].next_due;
            assert_eq!(s.check(t).nudge.len(), 1);
        }
        let t = s.chans[0].next_due;
        let d = s.check(t);
        assert!(d.nudge.is_empty());
        assert_eq!(
            d.fallback,
            [Fallback { identity: "bob".into(), channel: "whatsapp:9".into(), names: vec![], lang: String::new(), call: false }]
        );
        assert_eq!(d.fallback[0].text(), FALLBACK_TEXT);
        assert_eq!(s.muted_len(), 1);
        // muted: nothing more, ever, without an event
        assert_eq!(s.check(t + 100_000), Due::default());
        assert_eq!(s.secs_until_due(t), None);
        // a job event wakes it from 20s
        s.wake(t + 10);
        assert!(s.check(t + 29).nudge.is_empty());
        assert_eq!(s.check(t + 30).nudge.len(), 1);
        // and a new user message resets it too
        let mut s2 = Set::mem();
        s2.add("1", "web", "root", 0);
        s2.chans[0].muted = true;
        s2.add("2", "web", "root", 50);
        assert_eq!(ids(&s2.check(70).nudge), ["1", "2"]);
    }

    #[test]
    fn replies_keep_fallback_away() {
        let mut s = Set::mem();
        s.add("1", "web", "root", 0);
        let mut t = 0;
        for _ in 0..20 {
            t = s.chans[0].next_due;
            let d = s.check(t);
            assert!(d.fallback.is_empty());
            assert_eq!(d.nudge.len(), 1);
            s.on_reply("web", &[], t + 1);
        }
        // backoff reached the cap
        assert_eq!(s.chans[0].next_due - (t + 1), MAX_INTERVAL_SECS);
    }

    #[test]
    fn one_notice_for_many_channels() {
        let mut s = Set::mem();
        s.add("a", "web", "root", 0);
        s.add("b", "telegram:1", "alice", 3);
        s.add("c", "web", "root", 5);
        // web reset by "c" at 5 → due at 25; telegram due at 23
        let d = s.check(25);
        assert_eq!(ids(&d.nudge), ["a", "b", "c"]);
        let n = notice(&d, "job:7 bash 10s", &[], 25);
        assert!(n.starts_with("unanswered:\n"));
        assert!(n.contains("channel=telegram:1 id=b from alice, open 22s, no reply yet"));
        assert!(n.contains("running jobs: job:7 bash 10s"));
        // web's two messages are grouped under one close call, before telegram's
        assert!(n.contains(
            "  → still working: say(identity=\"root\", channel=\"web\", message=\"<short status>\")\n  → done: say(identity=\"root\", channel=\"web\", message=\"<answer>\", answered=[\"a\", \"c\"])\n"
        ));
        assert!(n.contains(
            "  → done: say(identity=\"alice\", channel=\"telegram:1\", message=\"<answer>\", answered=[\"b\"])\n"
        ));
        assert!(n.find("channel=web id=c").unwrap() < n.find("channel=telegram:1 id=b").unwrap());
        assert!(n.contains("for_ids=[…]"));
    }

    /// Two people waiting in one group: each message shows the job working for it, the
    /// chat gets one status (for both), and each person their own close call.
    #[test]
    fn group_notice_one_status_for_everyone() {
        let mut s = Set::mem();
        s.add_msg("m12", "telegram:-100", "Anna", 0, true, "");
        s.add_msg("m15", "telegram:-100", "Ben", 2, true, "");
        let d = s.check(22);
        assert_eq!(ids(&d.nudge), ["m12", "m15"]);
        let work = [
            Work { channel: "telegram:-100".into(), id: "m12".into(), job: "job:0a12 chrome_read, running 20s".into() },
            Work { channel: "telegram:-100".into(), id: "m15".into(), job: "job:0a13 bash, running 18s".into() },
            Work { channel: "telegram:-100".into(), id: "m15".into(), job: "job:0a14 bash, running 3s".into() },
        ];
        let n = notice(&d, "", &work, 22);
        assert_eq!(n.matches("→ still working: say(").count(), 1, "{n}");
        assert!(n.contains("message=\"<one short status for everyone waiting here, @Name each>\""), "{n}");
        assert!(n.contains("id=m12 from Anna, open 22s, no reply yet\n    working for it: job:0a12 chrome_read, running 20s\n"), "{n}");
        assert!(n.contains("id=m15 from Ben, open 20s, no reply yet\n    working for it: job:0a13 bash, running 18s\n    working for it: job:0a14 bash, running 3s\n"), "{n}");
        assert!(n.contains("say(identity=\"Anna\", channel=\"telegram:-100\", message=\"<answer>\", answered=[\"m12\"])"), "{n}");
        assert!(n.contains("say(identity=\"Ben\", channel=\"telegram:-100\", message=\"<answer>\", answered=[\"m15\"])"), "{n}");
        assert!(n.contains("running jobs: none"));
    }

    /// In a group, someone else writing does not push back a status already due; the same
    /// person writing on does restart the wait.
    #[test]
    fn others_writing_do_not_delay_a_status() {
        let mut s = Set::mem();
        s.add_msg("a", "telegram:-1", "Anna", 0, true, "");
        s.add_msg("b", "telegram:-1", "Ben", 4, true, "");
        assert_eq!(ids(&s.check(20).nudge), ["a", "b"]);
        let mut s = Set::mem();
        s.add_msg("a", "telegram:-1", "Anna", 0, true, "");
        s.add_msg("a2", "telegram:-1", "Anna", 4, true, "");
        assert!(s.check(20).nudge.is_empty());
        assert_eq!(s.check(24).nudge.len(), 2);
    }

    #[test]
    fn a_fallback_counts_as_speaking() {
        let mut s = Set::mem();
        s.set_call("phone:+1", true, 0);
        s.add_msg("x", "phone:+1", "alex", 0, false, "en");
        s.check(10);
        let d = s.check(20);
        assert_eq!(d.fallback.len(), 1);
        s.on_fallback("phone:+1", 20);
        assert_eq!(s.last_reply("phone:+1"), 20);
        assert_eq!(s.muted_len(), 1);
    }

    /// While bia speaks into a call: no reminder, no fallback; it waits for the end of the
    /// speech, then 10s; a lost end event stops blocking after the cap.
    #[test]
    fn speaking_holds_reminders_and_fallback() {
        let mut s = Set::mem();
        s.set_call("phone:+1", true, 0);
        s.add_msg("p1", "phone:+1", "alex", 0, false, "es");
        assert_eq!(s.check(10).nudge.len(), 1);
        // the model answers the reminder with a long spoken status
        s.on_reply("phone:+1", &[], 12);
        s.speaking_started("phone:+1", 12);
        // its next reminder (22) and the fallback after it would fall inside the speech
        let d = s.check(30);
        assert!(d.nudge.is_empty() && d.fallback.is_empty(), "{d:?}");
        assert_eq!(s.secs_until_due(30), Some(42), "looks again when the cap runs out");
        s.speaking_done("phone:+1", 35);
        assert!(s.check(44).nudge.is_empty());
        assert_eq!(s.check(45).nudge.len(), 1);
        // a speech whose end never comes stops blocking after the cap
        s.speaking_started("phone:+1", 50);
        assert!(s.is_speaking("phone:+1", 50 + SPEAK_CAP_SECS - 1));
        assert!(!s.is_speaking("phone:+1", 50 + SPEAK_CAP_SECS));
        // the call ends: no speaking state left
        s.speaking_started("phone:+1", 200);
        s.set_call("phone:+1", false, 201);
        assert!(!s.is_speaking("phone:+1", 202));
    }

    /// Speaking is only tracked in a call; claim_status asks once per due reminder.
    #[test]
    fn claim_status_is_the_reminder() {
        let mut s = Set::mem();
        s.speaking_started("telegram:5", 0);
        assert!(!s.is_speaking("telegram:5", 1), "not in a call");
        s.add("1", "telegram:5", "bob", 0);
        assert_eq!(s.claim_status("telegram:5", 19), None);
        assert_eq!(s.claim_status("telegram:5", 20), Some(20));
        assert_eq!(s.claim_status("telegram:5", 21), None);
        assert!(s.check(59).nudge.is_empty());
        assert_eq!(s.check(60).nudge.len(), 1, "then the backoff goes on (40s)");
    }

    /// One person, several messages: one block, one status, one close call for all.
    #[test]
    fn burst_from_one_person_is_one_block() {
        let mut s = Set::mem();
        s.add("1", "whatsapp:7", "bob", 0);
        s.add("2", "whatsapp:7", "bob", 1);
        s.add("3", "whatsapp:7", "bob", 2);
        let d = s.check(22);
        let n = notice(&d, "", &[], 22);
        assert_eq!(n.matches("→ still working: say(").count(), 1);
        assert_eq!(n.matches("→ done: say(").count(), 1);
        assert!(n.contains("answered=[\"1\", \"2\", \"3\"]"));
        assert_eq!(s.open_of("bob", "whatsapp:7").len(), 3);
    }

    #[test]
    fn call_nudges_every_10s_then_speaks_the_fallback() {
        let mut s = Set::mem();
        s.set_call("phone:+34600", true, 0);
        s.add_msg("", "phone:+34600", "alex", 100, false, "es");
        assert!(s.check(109).nudge.is_empty());
        let d = s.check(110);
        assert_eq!(d.nudge.len(), 1);
        assert_eq!(d.calls, ["phone:+34600"]);
        let n = notice(&d, "", &[], 110);
        assert!(n.contains("<one short spoken sentence: what you are doing now>"), "{n}");
        assert!(n.contains("in a voice call"), "{n}");
        // the model said a status: the next nudge is 10s after it, no backoff
        s.on_reply("phone:+34600", &[], 112);
        assert!(s.check(121).nudge.is_empty());
        assert_eq!(s.check(122).nudge.len(), 1);
        // ignored once: the fallback is spoken in the caller's language
        let d = s.check(132);
        assert!(d.nudge.is_empty());
        assert_eq!(d.fallback.len(), 1);
        assert!(d.fallback[0].call);
        assert_eq!(d.fallback[0].lang, "es");
        assert_eq!(d.fallback[0].text(), "Sigo trabajando en ello.");
        // the next phrase they say starts over at 10s
        s.add_msg("", "phone:+34600", "alex", 140, false, "es");
        assert_eq!(s.check(150).nudge.len(), 1);
    }

    #[test]
    fn call_start_pulls_a_due_chat_forward_and_end_restores_backoff() {
        let mut s = Set::mem();
        s.add("1", "telegram:5", "alice", 0);
        assert_eq!(s.secs_until_due(0), Some(20));
        s.set_call("telegram:5", true, 2);
        assert_eq!(s.secs_until_due(2), Some(10));
        assert!(s.in_call("telegram:5"));
        s.set_call("telegram:5", false, 5);
        assert!(!s.in_call("telegram:5"));
        // a status after the call: back to the chat backoff
        s.on_reply("telegram:5", &[], 30);
        assert_eq!(s.secs_until_due(30), Some(20));
    }

    #[test]
    fn group_fallback_names_everyone_waiting() {
        let mut s = Set::mem();
        s.add_msg("m1", "whatsapp:g1", "Anna", 0, true, "");
        s.add_msg("m2", "whatsapp:g1", "Ben", 1, true, "");
        s.add_msg("m3", "whatsapp:g1", "Anna", 2, true, "");
        let mut d = Due::default();
        for _ in 0..=MAX_SILENT_NUDGES {
            let t = s.chans[0].next_due;
            d = s.check(t);
        }
        assert_eq!(d.fallback.len(), 1);
        assert_eq!(d.fallback[0].names, ["Anna", "Ben"]);
        assert_eq!(d.fallback[0].text(), "@Anna @Ben, still working on it.");
    }

    #[test]
    fn fallback_texts() {
        assert_eq!(fallback_text("ru"), "Ещё работаю над этим.");
        assert_eq!(fallback_text("es-ES"), "Sigo trabajando en ello.");
        assert_eq!(fallback_text("xx"), FALLBACK_TEXT);
        assert_eq!(fallback_text(""), FALLBACK_TEXT);
    }

    #[test]
    fn find_prefers_the_turns_chat() {
        let mut s = Set::mem();
        s.add("7", "telegram:1", "a", 0);
        s.add("7", "telegram:2", "b", 0);
        s.add("9", "web", "root", 0);
        assert_eq!(s.find("7", "telegram:2").unwrap().identity, "b");
        assert!(s.find("7", "web").is_none(), "ambiguous elsewhere: not guessed");
        assert_eq!(s.find("9", "telegram:1").unwrap().channel, "web");
        assert!(s.find("nope", "web").is_none());
    }

    #[test]
    fn calls_persist() {
        let dir = std::env::temp_dir().join(format!("bia-unanswered-calls-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        {
            let mut s = Set::load(&dir);
            s.set_call("phone:+1", true, 0);
            s.add_msg("x", "telegram:3", "alice", 1, true, "ru");
        }
        let s = Set::load(&dir);
        assert!(s.in_call("phone:+1"));
        assert!(s.open[0].group && s.open[0].lang == "ru");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wrong_chat_hint_same_bus_only() {
        let mut s = Set::mem();
        s.add("b1", "web:c26", "root", 0);
        s.add("9", "telegram:5", "alice", 0);
        assert_eq!(s.open_elsewhere("web"), ["web:c26"]);
        assert!(s.open_elsewhere("web:c26").is_empty());
        assert!(s.open_elsewhere("whatsapp:1").is_empty());
    }

    #[test]
    fn answering_custom_chat_is_not_a_wrong_chat() {
        // An old message is still open on the general chat; the user writes in a
        // custom chat and the model answers it there, closing it. The hint must be
        // taken before closing (as note_reply does), else it says "reply on web".
        let mut s = Set::mem();
        s.add("old", "web", "root", 0);
        s.add("b7", "web:c26", "root", 5);
        let before = s.open_elsewhere("web:c26");
        s.on_reply("web:c26", &["b7".into()], 6);
        assert!(before.is_empty(), "web:c26 had its own open message");
        assert_eq!(
            s.open_elsewhere("web:c26"),
            ["web"],
            "what the old order wrongly reported"
        );
        assert!(s.has_open("web") && !s.has_open("web:c26"));
    }

    #[test]
    fn persists() {
        let dir = std::env::temp_dir().join(format!("bia-unanswered-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        {
            let mut s = Set::load(&dir);
            s.add("1", "web", "root", 10);
            s.add("2", "telegram:3", "alice", 11);
            s.on_reply("web", &["1".into()], 12);
        }
        let s = Set::load(&dir);
        assert_eq!(ids(&s.open), ["2"]);
        assert_eq!(s.chans.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
