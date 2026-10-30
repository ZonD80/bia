//! Unanswered human messages. Each human inbound stays open until the model closes it
//! with `answered=[ids]` on say/react. A per-channel backoff decides when an
//! `unanswered:` notice nudges the model; a channel that ignores 5 nudges gets one
//! automatic "still working on it" and is muted until the user writes again or a job
//! event arrives. Persisted to state/unanswered.json.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const TICK_SECS: u64 = 15;
const MAX_INTERVAL_SECS: u64 = 600;
const MAX_SILENT_NUDGES: u32 = 5;
pub const FALLBACK_TEXT: &str = "Still working on it.";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub id: String,
    pub channel: String,
    pub identity: String,
    pub received: u64,
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
}

/// What the REPL should do on a check.
#[derive(Debug, Default, PartialEq)]
pub struct Due {
    /// open entries on channels whose nudge is due (oldest first)
    pub nudge: Vec<Entry>,
    /// last reply per nudged channel (0 = none), parallel to the channels in `nudge`
    pub last_reply: Vec<(String, u64)>,
    /// (identity, channel) that get the one automatic fallback and are now muted
    pub fallback: Vec<(String, String)>,
}

pub struct Set {
    path: Option<PathBuf>,
    open: Vec<Entry>,
    chans: Vec<Chan>,
}

fn interval(step: u32) -> u64 {
    TICK_SECS
        .saturating_mul(1u64 << step.min(16))
        .min(MAX_INTERVAL_SECS)
}

impl Set {
    pub fn load(state_dir: &Path) -> Set {
        let path = state_dir.join("unanswered.json");
        let f = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<File>(&b).ok())
            .unwrap_or_default();
        // The physical console is not tracked (older state may still list it).
        let mut set = Set {
            path: Some(path),
            open: f.open,
            chans: f.chans,
        };
        set.open.retain(|e| e.channel != "console");
        set.prune();
        set
    }

    #[cfg(test)]
    fn mem() -> Set {
        Set {
            path: None,
            open: vec![],
            chans: vec![],
        }
    }

    fn persist(&self) {
        let Some(path) = &self.path else { return };
        if let Some(p) = path.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        let f = File {
            open: self.open.clone(),
            chans: self.chans.clone(),
        };
        let tmp = path.with_extension("json.tmp");
        if let Ok(buf) = serde_json::to_vec_pretty(&f) {
            if std::fs::write(&tmp, buf).is_ok() {
                let _ = std::fs::rename(&tmp, path);
            }
        }
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
    pub fn add(&mut self, id: &str, channel: &str, identity: &str, now: u64) {
        if !self.open.iter().any(|e| e.id == id && e.channel == channel) {
            self.open.push(Entry {
                id: id.into(),
                channel: channel.into(),
                identity: identity.into(),
                received: now,
            });
        }
        let c = self.chan(channel);
        c.step = 0;
        c.silent = 0;
        c.muted = false;
        c.next_due = now + TICK_SECS;
        self.persist();
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
            let c = self.chan(channel);
            c.last_reply = now;
            c.silent = 0;
            c.muted = false;
            c.next_due = now + interval(c.step);
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
        for c in self.chans.iter_mut().filter(|c| c.muted) {
            c.muted = false;
            c.step = 0;
            c.silent = 0;
            c.next_due = now + TICK_SECS;
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
            .map(|c| c.next_due.saturating_sub(now))
            .min()
    }

    /// Collect due nudges and fallbacks, advancing the backoff of every channel involved.
    pub fn check(&mut self, now: u64) -> Due {
        let mut due = Due::default();
        let mut nudged: Vec<String> = vec![];
        for c in self.chans.iter_mut() {
            if c.muted || c.next_due > now {
                continue;
            }
            let Some(first) = self.open.iter().find(|e| e.channel == c.channel) else {
                continue;
            };
            if c.silent >= MAX_SILENT_NUDGES {
                c.muted = true;
                due.fallback
                    .push((first.identity.clone(), c.channel.clone()));
                continue;
            }
            c.silent += 1;
            c.step = c.step.saturating_add(1);
            c.next_due = now + interval(c.step);
            due.last_reply.push((c.channel.clone(), c.last_reply));
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

/// Body of the `unanswered:` system notice.
pub fn notice(due: &Due, jobs: &str, now: u64) -> String {
    let mut s = String::from("unanswered:\n");
    for e in &due.nudge {
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
    s.push_str(&format!(
        "running jobs: {}\n",
        if jobs.is_empty() { "none" } else { jobs }
    ));
    s.push_str("Use each channel exactly as listed. Not answered yet: reply with say. Still working: post a short status. Already answered and done: close it with say(channel, answered=[id]) and no message, which sends nothing. Do not repeat an answer.");
    s
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
        assert!(s.check(110).nudge.is_empty());
        let d = s.check(115);
        assert_eq!(ids(&d.nudge), ["1"]);
        // step 1 → 30s
        assert!(s.check(140).nudge.is_empty());
        assert_eq!(ids(&s.check(145).nudge), ["1"]);
        // step 2 → 60s; a status reply postpones by the current interval, no reset
        s.on_reply("telegram:5", &[], 150);
        assert!(s.check(205).nudge.is_empty());
        assert_eq!(ids(&s.check(210).nudge), ["1"]);
    }

    #[test]
    fn interval_caps() {
        assert_eq!(interval(0), 15);
        assert_eq!(interval(3), 120);
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
        assert_eq!(d.fallback, [("bob".to_string(), "whatsapp:9".to_string())]);
        assert_eq!(s.muted_len(), 1);
        // muted: nothing more, ever, without an event
        assert_eq!(s.check(t + 100_000), Due::default());
        assert_eq!(s.secs_until_due(t), None);
        // a job event wakes it from 15s
        s.wake(t + 10);
        assert_eq!(s.check(t + 25).nudge.len(), 1);
        // and a new user message resets it too
        let mut s2 = Set::mem();
        s2.add("1", "web", "root", 0);
        s2.chans[0].muted = true;
        s2.add("2", "web", "root", 50);
        assert_eq!(ids(&s2.check(65).nudge), ["1", "2"]);
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
        // web reset by "c" at 5 → due at 20; telegram due at 18
        let d = s.check(20);
        assert_eq!(ids(&d.nudge), ["a", "b", "c"]);
        let n = notice(&d, "job:7 bash 10s", 20);
        assert!(n.starts_with("unanswered:\n"));
        assert!(n.contains("channel=telegram:1 id=b from alice, open 17s, no reply yet"));
        assert!(n.contains("running jobs: job:7 bash 10s"));
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
