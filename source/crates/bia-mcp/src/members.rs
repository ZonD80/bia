//! Group members for the agent's queue ranking. A chat MCP tells the agent who is in a group
//! (`{"t":"members",…}`) before that group's first message and after a day, and who joined or
//! left (`{"t":"member",…}`). Ids are in channel form: `telegram:<user id>`, the same string
//! as the person's direct chat with bia, so the agent finds them in its contacts.
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Groups bigger than this are never all root people: their list is not fetched whole.
pub const MAX_LISTED: usize = 20;
/// How long a member list is trusted before it is fetched again (joins missed while off).
pub const REFRESH: Duration = Duration::from_secs(24 * 3600);

/// Who wrote a message: their id in channel form and whether the chat is a group.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct From {
    pub sender: String,
    pub group: bool,
}

impl From {
    pub fn new(sender: impl Into<String>, group: bool) -> From {
        From { sender: sender.into(), group }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Member {
    /// channel form: telegram:123
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub channel: String,
    pub title: String,
    pub members: Vec<Member>,
    /// how many members the platform reports (bia left out); 0 when unknown
    pub count: usize,
    /// `members` is everyone (else a part of a big group, or unknown)
    pub complete: bool,
}

impl Snapshot {
    /// A snapshot from a listed set: bia's own id is left out; more than [`MAX_LISTED`]
    /// people (or `count` above it) make it partial.
    pub fn new(channel: &str, title: &str, mut members: Vec<Member>, me: &str, count: usize) -> Snapshot {
        members.retain(|m| !m.id.is_empty() && m.id != me);
        members.dedup_by(|a, b| a.id == b.id);
        let count = count.max(members.len());
        let complete = count <= MAX_LISTED && members.len() == count;
        if members.len() > MAX_LISTED {
            members.truncate(MAX_LISTED);
        }
        Snapshot { channel: channel.into(), title: title.into(), members, count, complete }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "t": "members",
            "channel": self.channel,
            "title": self.title,
            "members": self.members.iter().map(|m| json!({"id": m.id, "name": m.name})).collect::<Vec<_>>(),
            "count": self.count,
            "complete": self.complete,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub channel: String,
    pub title: String,
    /// join | leave
    pub op: String,
    pub member: Member,
    /// who did it when it was someone else (an admin adding or removing), channel form
    pub by: String,
}

impl Change {
    pub fn join(channel: &str, title: &str, id: &str, name: &str, by: &str) -> Change {
        Change { channel: channel.into(), title: title.into(), op: "join".into(), member: Member { id: id.into(), name: name.into() }, by: by.into() }
    }
    pub fn leave(channel: &str, title: &str, id: &str, name: &str, by: &str) -> Change {
        Change { channel: channel.into(), title: title.into(), op: "leave".into(), member: Member { id: id.into(), name: name.into() }, by: by.into() }
    }
    pub fn to_json(&self) -> Value {
        let mut v = json!({
            "t": "member",
            "channel": self.channel,
            "title": self.title,
            "op": self.op,
            "id": self.member.id,
            "name": self.member.name,
        });
        if !self.by.is_empty() && self.by != self.member.id {
            v["by"] = json!(self.by);
        }
        v
    }
}

/// Which groups had their member list sent in this run, and when.
#[derive(Default)]
pub struct Tracker {
    sent: Mutex<HashMap<String, Instant>>,
}

impl Tracker {
    /// The group's list should be sent now (never in this run, or older than [`REFRESH`]).
    /// Marks it sent: a failed fetch is not retried until the next refresh.
    pub fn due(&self, channel: &str) -> bool {
        let mut m = self.sent.lock().unwrap_or_else(|e| e.into_inner());
        match m.get(channel) {
            Some(t) if t.elapsed() < REFRESH => false,
            _ => {
                m.insert(channel.to_string(), Instant::now());
                true
            }
        }
    }
    /// Fetch the list again at the group's next message (after a catch-up: joins were missed).
    pub fn forget(&self, channel: &str) {
        self.sent.lock().unwrap_or_else(|e| e.into_inner()).remove(channel);
    }
    pub fn forget_all(&self) {
        self.sent.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
}

/// One tracker per MCP process.
pub fn tracker() -> &'static Tracker {
    static T: std::sync::OnceLock<Tracker> = std::sync::OnceLock::new();
    T.get_or_init(Tracker::default)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(id: &str) -> Member {
        Member { id: id.into(), name: id.into() }
    }

    #[test]
    fn snapshots_leave_bia_out_and_mark_big_groups_partial() {
        let s = Snapshot::new("telegram:-5", "Family", vec![m("telegram:1"), m("telegram:2"), m("telegram:9")], "telegram:9", 0);
        assert_eq!(s.members.len(), 2);
        assert!(s.complete);
        assert_eq!(s.count, 2);
        let big: Vec<Member> = (0..30).map(|i| m(&format!("telegram:{i}"))).collect();
        let s = Snapshot::new("telegram:-6", "Big", big, "", 0);
        assert!(!s.complete);
        assert_eq!(s.members.len(), MAX_LISTED);
        // a short list from a big group (the platform says how many): partial
        let s = Snapshot::new("telegram:-7", "", vec![m("telegram:1")], "", 300);
        assert!(!s.complete);
        assert_eq!(s.count, 300);
        let v = s.to_json();
        assert_eq!(v["t"], "members");
        assert_eq!(v["complete"], false);
    }

    #[test]
    fn changes_name_who_did_them_only_when_someone_else() {
        let v = Change::join("telegram:-5", "Family", "telegram:3", "Ann", "telegram:3").to_json();
        assert_eq!(v["op"], "join");
        assert!(v.get("by").is_none());
        let v = Change::leave("telegram:-5", "Family", "telegram:3", "Ann", "telegram:1").to_json();
        assert_eq!(v["by"], "telegram:1");
    }

    #[test]
    fn the_tracker_sends_a_list_once_until_forgotten() {
        let t = Tracker::default();
        assert!(t.due("x"));
        assert!(!t.due("x"));
        t.forget("x");
        assert!(t.due("x"));
    }
}
