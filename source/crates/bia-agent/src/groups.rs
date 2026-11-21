//! Group members and levels, for ranking the queue (queue.rs). The chat MCPs say who is in a
//! group (`{"t":"members",…}` before its first message and after a day) and who joins or leaves
//! (`{"t":"member",…}`); mcp.rs hands both to the agent loop on channel [`CH_MEMBERS`]. Kept in
//! files/state/groups.json. A group of only root-trust people and bia ranks like root (P0);
//! any other member makes it mixed. The model can set a level by hand (`chat_level`).
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Channel of the member events inside the agent (never shown to the model as such).
pub const CH_MEMBERS: &str = "members";

/// The channel a members notice waits in: `members` for P0, `members:p<n>` below.
pub fn notice_channel(tier: u8) -> String {
    if tier == 0 {
        CH_MEMBERS.to_string()
    } else {
        format!("{CH_MEMBERS}:p{tier}")
    }
}

/// A members notice's channel, and the tier it waits in.
pub fn notice_tier(channel: &str) -> Option<u8> {
    if channel == CH_MEMBERS {
        return Some(0);
    }
    channel.strip_prefix(CH_MEMBERS)?.strip_prefix(":p")?.parse().ok().filter(|t| *t <= 3)
}

pub fn is_notice_channel(channel: &str) -> bool {
    notice_tier(channel).is_some()
}

/// Levels the model can set: auto (by members) or a fixed tier.
pub const LEVELS: [&str; 5] = ["auto", "p0", "p1", "p2", "p3"];

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Group {
    #[serde(default)]
    pub title: String,
    /// id (channel form) → name; bia left out
    #[serde(default)]
    pub members: BTreeMap<String, String>,
    /// members the platform reports (bia left out)
    #[serde(default)]
    pub count: usize,
    /// `members` is everyone
    #[serde(default)]
    pub complete: bool,
    /// "" (auto) or p0..p3, set by the model
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub level: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub why: String,
    /// Unix seconds of the last list or change
    #[serde(default)]
    pub updated: u64,
}

impl Group {
    /// Everyone in it is a root-trust person (and the list is known whole).
    pub fn all_root(&self, is_root: &dyn Fn(&str) -> bool) -> bool {
        self.complete && !self.members.is_empty() && self.members.len() == self.count && self.members.keys().all(|id| is_root(id))
    }

    /// The members that are not root-trust people.
    pub fn strangers(&self, is_root: &dyn Fn(&str) -> bool) -> Vec<(String, String)> {
        self.members.iter().filter(|(id, _)| !is_root(id)).map(|(a, b)| (a.clone(), b.clone())).collect()
    }

    /// The fixed tier the model set (0..=3), if any.
    pub fn fixed(&self) -> Option<u8> {
        self.level.strip_prefix('p').and_then(|n| n.parse().ok()).filter(|n| *n <= 3)
    }
}

#[derive(Serialize, Deserialize, Default)]
struct File {
    #[serde(default)]
    groups: BTreeMap<String, Group>,
}

pub struct Groups {
    path: PathBuf,
    map: BTreeMap<String, Group>,
}

/// What a member event did, for the notice to the model.
#[derive(Debug, PartialEq)]
pub struct Changed {
    pub channel: String,
    pub title: String,
    /// join | leave | list
    pub op: String,
    /// (id, name) of the people who came or went
    pub who: Vec<(String, String)>,
    /// a new list: the people the old one had and it has not (left while bia was away)
    pub gone: Vec<(String, String)>,
    pub by: String,
    /// the group was known before this event
    pub known: bool,
}

fn now() -> u64 {
    crate::clock::unix(crate::clock::now())
}

fn sv<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

impl Groups {
    pub fn load(path: &Path) -> Groups {
        let map = bia_mcp::statefile::read::<File>(path).map(|f| f.groups).unwrap_or_default();
        Groups { path: path.to_path_buf(), map }
    }

    /// A store that is never written.
    #[cfg(test)]
    pub fn empty() -> Groups {
        Groups { path: PathBuf::new(), map: BTreeMap::new() }
    }

    fn save(&self) {
        if self.path.as_os_str().is_empty() {
            return;
        }
        let _ = bia_mcp::statefile::write_json(&self.path, &File { groups: self.map.clone() });
    }

    pub fn get(&self, channel: &str) -> Option<&Group> {
        self.map.get(channel)
    }

    /// Apply a `members` or `member` event from a chat MCP. None when it was not one.
    pub fn apply(&mut self, ev: &Value) -> Option<Changed> {
        let channel = sv(ev, "channel").to_string();
        if channel.is_empty() {
            return None;
        }
        let known = self.map.contains_key(&channel);
        let g = self.map.entry(channel.clone()).or_default();
        let title = sv(ev, "title");
        if !title.is_empty() {
            g.title = title.to_string();
        }
        g.updated = now();
        let out = match sv(ev, "t") {
            "members" => {
                let list: BTreeMap<String, String> = ev
                    .get("members")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter(|m| !sv(m, "id").is_empty())
                            .map(|m| (sv(m, "id").to_string(), sv(m, "name").to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                // people in the new list that the old one did not have (joins missed while off)
                let who: Vec<(String, String)> = if known {
                    list.iter().filter(|(id, _)| !g.members.contains_key(*id)).map(|(a, b)| (a.clone(), b.clone())).collect()
                } else {
                    vec![]
                };
                let complete = ev.get("complete").and_then(Value::as_bool).unwrap_or(false);
                // people the old list had and the new one has not: only when both are whole
                let gone: Vec<(String, String)> = if known && complete && g.complete {
                    g.members.iter().filter(|(id, _)| !list.contains_key(*id)).map(|(a, b)| (a.clone(), b.clone())).collect()
                } else {
                    vec![]
                };
                g.count = ev.get("count").and_then(Value::as_u64).map(|n| n as usize).unwrap_or(list.len()).max(list.len());
                g.complete = complete;
                g.members = list;
                Changed { channel: channel.clone(), title: g.title.clone(), op: "list".into(), who, gone, by: String::new(), known }
            }
            "member" => {
                let id = sv(ev, "id").to_string();
                let name = sv(ev, "name").to_string();
                let op = sv(ev, "op").to_string();
                if id.is_empty() {
                    return None;
                }
                match op.as_str() {
                    "join" => {
                        if g.members.insert(id.clone(), name.clone()).is_none() {
                            g.count += 1;
                        }
                        // a list that was never fetched stays unknown (mixed)
                        if !known {
                            g.complete = false;
                        }
                    }
                    "leave" => {
                        if g.members.remove(&id).is_some() || !g.complete {
                            g.count = g.count.saturating_sub(1);
                        }
                    }
                    _ => return None,
                }
                Changed { channel: channel.clone(), title: g.title.clone(), op, who: vec![(id, name)], gone: vec![], by: sv(ev, "by").to_string(), known }
            }
            _ => return None,
        };
        self.save();
        Some(out)
    }

    /// chat_level: auto, or a fixed tier p0..p3 for any chat (a group or a direct chat).
    pub fn set_level(&mut self, chat: &str, level: &str, why: &str) -> Result<String> {
        let chat = chat.trim();
        let level = level.trim().to_lowercase();
        if chat.is_empty() {
            bail!("chat is empty: pass the channel, e.g. telegram:-1001234567890");
        }
        if !LEVELS.contains(&level.as_str()) {
            bail!("level must be one of {}", LEVELS.join(", "));
        }
        let g = self.map.entry(chat.to_string()).or_default();
        g.level = if level == "auto" { String::new() } else { level.clone() };
        g.why = if level == "auto" { String::new() } else { why.trim().to_string() };
        self.save();
        Ok(format!("{chat}: level {level}"))
    }

    pub fn level_of(&self, chat: &str) -> Option<u8> {
        self.map.get(chat).and_then(Group::fixed)
    }

    /// chat_levels: the groups known, their level and who is in them.
    pub fn list(&self, is_root: &dyn Fn(&str) -> bool) -> String {
        if self.map.is_empty() {
            return "no groups known yet (a group is listed after its first message)".into();
        }
        let mut out = vec![];
        for (ch, g) in &self.map {
            let auto = if g.members.is_empty() && g.count == 0 {
                "direct chat or not listed yet".to_string()
            } else if g.all_root(is_root) {
                "all members root: P0".to_string()
            } else {
                let s = g.strangers(is_root);
                let names: Vec<String> = s.iter().take(5).map(|(id, n)| if n.is_empty() { id.clone() } else { format!("{n} ({id})") }).collect();
                format!(
                    "mixed: root P1, replies and mentions P2, others P3; {} member(s){}{}",
                    g.count,
                    if g.complete { "" } else { ", list partial" },
                    if names.is_empty() { String::new() } else { format!("; not root: {}", names.join(", ")) }
                )
            };
            let level = if g.level.is_empty() { "auto".to_string() } else { format!("{} (set{})", g.level, if g.why.is_empty() { String::new() } else { format!(": {}", g.why) }) };
            let title = if g.title.is_empty() { String::new() } else { format!(" \"{}\"", g.title) };
            out.push(format!("{ch}{title}: level {level}; {auto}"));
        }
        out.join("\n")
    }
}

/// The notice for the model after a member event, with the tier it waits in: every join and
/// leave is told. A stranger joining a group that ranked as root (all members root, or set to
/// p0) is P0 with the ways out; joins and leaves that keep or make a group root are P1; the
/// comings and goings of mixed groups are P3, like their talk. `was_root`: how the group ranked
/// before the event. None for a first list (nothing came or went).
pub fn notice(c: &Changed, g: &Group, was_root: bool, is_root: &dyn Fn(&str) -> bool) -> Option<(String, u8)> {
    let title = if c.title.is_empty() { c.channel.clone() } else { format!("{} ({})", c.title, c.channel) };
    let fixed_p0 = g.fixed() == Some(0);
    let root_group = was_root || fixed_p0;
    let now_root = g.all_root(is_root);
    let person = |(id, name): &(String, String)| if name.is_empty() { id.clone() } else { format!("{name} ({id})") };
    let names = |v: &[&(String, String)]| v.iter().map(|w| person(w)).collect::<Vec<_>>().join(", ");
    let by = if c.by.is_empty() || c.who.iter().any(|w| w.0 == c.by) { String::new() } else { format!(" (by {})", c.by) };
    let away = c.op == "list";
    let mut lines: Vec<String> = vec![];
    let mut tier = 3u8;
    let (joined, left): (Vec<&(String, String)>, Vec<&(String, String)>) = match c.op.as_str() {
        "join" => (c.who.iter().collect(), vec![]),
        "leave" => (vec![], c.who.iter().collect()),
        "list" => (c.who.iter().collect(), c.gone.iter().collect()),
        _ => return None,
    };
    if !joined.is_empty() {
        let roots: Vec<&(String, String)> = joined.iter().copied().filter(|w| is_root(&w.0)).collect();
        let others: Vec<&(String, String)> = joined.iter().copied().filter(|w| !is_root(&w.0)).collect();
        let verb = if away { "joined while you were away:" } else { "joined" };
        if root_group && !others.is_empty() {
            let first = &others[0].0;
            let head = format!("[members] {} {verb} {title}{by}: not a root contact.", names(&others));
            let tail = if fixed_p0 {
                format!(" You set this group to p0, so it stays P0; chat_level(chat={}, level=auto) ranks it by its members again (mixed: root P1, replies and mentions P2, others P3).", c.channel)
            } else {
                format!(
                    " The group no longer ranks as root: it is mixed now (root P1, replies and mentions P2, others P3). If they are a root person: addressbook_upsert their contact with trust root and channels [{first}] (the group is root again); or keep the group at P0 anyway with chat_level(chat={}, level=p0).",
                    c.channel
                )
            };
            lines.push(head + &tail);
            tier = 0;
            if !roots.is_empty() {
                lines.push(format!("[members] {} {verb} {title} too: a root contact.", names(&roots)));
            }
        } else if root_group {
            lines.push(format!("[members] {} {verb} {title}{by}: a root contact; the group keeps ranking as root (P0).", names(&roots)));
            tier = tier.min(1);
        } else {
            lines.push(format!("[members] {} {verb} {title}{by}.", names(&joined)));
        }
    }
    if !left.is_empty() {
        let verb = if away { "left while you were away:" } else { "left" };
        if !was_root && now_root {
            lines.push(format!("[members] {} {verb} {title}{by}: everyone in it is a root contact now; the group ranks as root (P0) again.", names(&left)));
            tier = tier.min(1);
        } else if root_group {
            lines.push(format!("[members] {} {verb} {title}{by}; the group keeps ranking as root (P0).", names(&left)));
            tier = tier.min(1);
        } else {
            lines.push(format!("[members] {} {verb} {title}{by}.", names(&left)));
        }
    }
    if lines.is_empty() {
        return None;
    }
    Some((lines.join("\n"), tier))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn roots(id: &str) -> bool {
        id == "telegram:1" || id == "telegram:2"
    }

    fn snap(members: &[(&str, &str)], complete: bool) -> Value {
        json!({"t": "members", "channel": "telegram:-5", "title": "Family", "count": members.len(), "complete": complete,
               "members": members.iter().map(|(i, n)| json!({"id": i, "name": n})).collect::<Vec<_>>()})
    }

    #[test]
    fn a_group_of_only_root_people_ranks_as_root_until_a_stranger_joins() {
        let mut g = Groups::empty();
        let c = g.apply(&snap(&[("telegram:1", "Root"), ("telegram:2", "Wife")], true)).unwrap();
        assert!(!c.known && c.who.is_empty());
        let grp = g.get("telegram:-5").unwrap().clone();
        assert!(grp.all_root(&roots));
        // a stranger joins: demoted, and the model is told with the id and both ways out
        let was = grp.all_root(&roots);
        let c = g.apply(&json!({"t": "member", "channel": "telegram:-5", "op": "join", "id": "telegram:7", "name": "Ann"})).unwrap();
        let grp = g.get("telegram:-5").unwrap().clone();
        assert!(!grp.all_root(&roots));
        let (n, tier) = notice(&c, &grp, was, &roots).unwrap();
        assert_eq!(tier, 0);
        assert!(n.contains("Ann (telegram:7)") && n.contains("channels [telegram:7]") && n.contains("level=p0"), "{n}");
        // she leaves: root again, said once
        let c = g.apply(&json!({"t": "member", "channel": "telegram:-5", "op": "leave", "id": "telegram:7", "name": "Ann"})).unwrap();
        let grp = g.get("telegram:-5").unwrap().clone();
        assert!(grp.all_root(&roots));
        let (n, tier) = notice(&c, &grp, false, &roots).unwrap();
        assert!(tier == 1 && n.contains("root (P0) again"), "{n}");
        // a root contact leaves: still root, told at P1
        let was = grp.all_root(&roots);
        let c = g.apply(&json!({"t": "member", "channel": "telegram:-5", "op": "leave", "id": "telegram:2", "name": "Wife"})).unwrap();
        let grp = g.get("telegram:-5").unwrap().clone();
        let (n, tier) = notice(&c, &grp, was, &roots).unwrap();
        assert!(tier == 1 && n.contains("Wife (telegram:2) left") && n.contains("keeps ranking as root"), "{n}");
    }

    #[test]
    fn a_root_contact_joining_keeps_the_level_and_mixed_groups_are_told_at_p3() {
        let mut g = Groups::empty();
        g.apply(&snap(&[("telegram:1", "Root")], true));
        let c = g.apply(&json!({"t": "member", "channel": "telegram:-5", "op": "join", "id": "telegram:2", "name": "Wife"})).unwrap();
        let grp = g.get("telegram:-5").unwrap().clone();
        assert!(grp.all_root(&roots));
        let (n, tier) = notice(&c, &grp, true, &roots).unwrap();
        assert!(tier == 1 && n.contains("keeps ranking as root"), "{n}");
        // a mixed group: every join and leave is told, at P3 like its talk
        let mut g = Groups::empty();
        g.apply(&snap(&[("telegram:1", "Root"), ("telegram:8", "Bob")], true));
        let c = g.apply(&json!({"t": "member", "channel": "telegram:-5", "op": "join", "id": "telegram:9", "name": "Eve"})).unwrap();
        let grp = g.get("telegram:-5").unwrap().clone();
        let (n, tier) = notice(&c, &grp, false, &roots).unwrap();
        assert!(tier == 3 && n == "[members] Eve (telegram:9) joined Family (telegram:-5).", "{n}");
        let c = g.apply(&json!({"t": "member", "channel": "telegram:-5", "op": "leave", "id": "telegram:8", "name": "Bob", "by": "telegram:1"})).unwrap();
        let grp = g.get("telegram:-5").unwrap().clone();
        let (n, tier) = notice(&c, &grp, false, &roots).unwrap();
        assert!(tier == 3 && n.contains("Bob (telegram:8) left") && n.contains("(by telegram:1)"), "{n}");
        // a first list says nothing
        let mut g = Groups::empty();
        let c = g.apply(&snap(&[("telegram:8", "Bob")], true)).unwrap();
        let grp = g.get("telegram:-5").unwrap().clone();
        assert!(notice(&c, &grp, false, &roots).is_none());
    }

    #[test]
    fn partial_and_unknown_lists_are_mixed() {
        let mut g = Groups::empty();
        g.apply(&snap(&[("telegram:1", "Root")], false));
        assert!(!g.get("telegram:-5").unwrap().all_root(&roots));
        // a join into a group never listed: still unknown
        let mut g = Groups::empty();
        g.apply(&json!({"t": "member", "channel": "telegram:-9", "op": "join", "id": "telegram:1"}));
        assert!(!g.get("telegram:-9").unwrap().all_root(&roots));
    }

    #[test]
    fn a_new_list_reports_who_came_while_bia_was_away() {
        let mut g = Groups::empty();
        g.apply(&snap(&[("telegram:1", "Root")], true));
        let c = g.apply(&snap(&[("telegram:1", "Root"), ("telegram:7", "Ann")], true)).unwrap();
        assert_eq!(c.who, vec![("telegram:7".to_string(), "Ann".to_string())]);
        let grp = g.get("telegram:-5").unwrap().clone();
        let (n, tier) = notice(&c, &grp, true, &roots).unwrap();
        assert!(tier == 0 && n.contains("while you were away"), "{n}");
        // and who left meanwhile, when both lists are whole
        let c = g.apply(&snap(&[("telegram:1", "Root")], true)).unwrap();
        assert_eq!(c.gone, vec![("telegram:7".to_string(), "Ann".to_string())]);
        let grp = g.get("telegram:-5").unwrap().clone();
        let (n, tier) = notice(&c, &grp, false, &roots).unwrap();
        assert!(tier == 1 && n.contains("left while you were away") && n.contains("root (P0) again"), "{n}");
    }

    #[test]
    fn levels_set_by_the_model() {
        let mut g = Groups::empty();
        assert!(g.set_level("telegram:-5", "p9", "").is_err());
        g.set_level("telegram:-5", "p0", "family").unwrap();
        assert_eq!(g.level_of("telegram:-5"), Some(0));
        assert!(g.list(&roots).contains("p0 (set: family)"));
        g.set_level("telegram:-5", "auto", "").unwrap();
        assert_eq!(g.level_of("telegram:-5"), None);
        // a fixed p0 group: a stranger joining is reported, the level stays
        g.set_level("telegram:-5", "p0", "").unwrap();
        g.apply(&snap(&[("telegram:1", "Root")], true));
        let c = g.apply(&json!({"t": "member", "channel": "telegram:-5", "op": "join", "id": "telegram:7", "name": "Ann"})).unwrap();
        let grp = g.get("telegram:-5").unwrap().clone();
        let (n, _) = notice(&c, &grp, true, &roots).unwrap();
        assert!(n.contains("stays P0") && n.contains("level=auto"), "{n}");
    }
}
