//! The waiting queue: which messages go to the model now, so a long queue never fills the
//! context (the model would only reflect, turn after turn, and never work).
//!
//! Messages are ranked (tiers): P0 root, root-trust people in direct chats, groups of only root
//! people, console, system notices; P1 other contacts, job results, schedules, a root-trust
//! person in a mixed group; P2 strangers in direct chats, replies to bia and mentions in mixed
//! groups; P3 the rest of mixed groups. The model can fix a chat's tier (`chat_level`).
//!
//! What fits goes in, most important first, round-robin over chats inside a tier; in the
//! conversation they keep each chat's order and the most important chat comes last (nearest
//! the reply). What does not fit waits; the model is told how much waits and where
//! (`[queue] …`) and can take more (`queue_take`). A body too long for the context is cut and
//! kept whole in a file (envelope, id and times unchanged).
use crate::console::Inbound;
use std::collections::{BTreeMap, HashMap};

/// The channel of the `[queue]` notices.
pub const CH_QUEUE: &str = "queue";

/// Tiers: 0 is the most important.
pub type Tier = u8;
pub const TIERS: Tier = 4;

/// A message body bigger than 1/CUT_SHARE of the context is cut.
pub const CUT_SHARE: usize = 8;
/// Share of the context kept free for the model's own work once reflections loop (%).
pub const LOOP_RESERVE_PCT: usize = 20;
/// The model is told again how the queue stands at most this often (seconds), unless new
/// chats start waiting.
pub const NOTICE_EVERY_SECS: u64 = 600;

/// Tokens a message takes in the context: its whole envelope, counted with fewer chars per
/// token than history_token_estimate (non-Latin text costs more), so it errs on the safe side.
pub fn tokens(ib: &Inbound) -> usize {
    let head = ib.identity.len() + ib.channel.len() + ib.id.len() + 48;
    (head + ib.text.chars().count()).div_ceil(3) + 8
}

/// What ranking needs to know about people and chats.
pub struct Ranker<'a> {
    pub book: &'a crate::addressbook::AddressBook,
    pub groups: &'a crate::groups::Groups,
    /// bia's own name (a mention of it in a group ranks like a reply)
    pub me: &'a str,
}

/// The message answers one of bia's, or names bia, or woke an ignored chat.
pub fn calls_bia(text: &str, me: &str) -> bool {
    if text.contains(": this message woke you") {
        return true;
    }
    if text.lines().any(|l| {
        l.trim_start().strip_prefix("[reply to ").and_then(|r| r.split_once(' ')).map(|(_, rest)| rest.starts_with("from you,") || rest.starts_with("from you:") || rest.starts_with("from you]")).unwrap_or(false)
    }) {
        return true;
    }
    let me = me.trim().to_lowercase();
    if me.chars().count() < 2 {
        return false;
    }
    let low = text.to_lowercase();
    let mut from = 0;
    while let Some(i) = low[from..].find(&me) {
        let (a, b) = (from + i, from + i + me.len());
        let before = low[..a].chars().next_back().map_or(true, |c| !c.is_alphanumeric());
        let after = low[b..].chars().next().map_or(true, |c| !c.is_alphanumeric());
        if before && after {
            return true;
        }
        from = b;
    }
    false
}

impl Ranker<'_> {
    fn root_id(&self, id: &str) -> bool {
        !id.is_empty() && self.book.is_root_id(id)
    }

    pub fn tier(&self, ib: &Inbound) -> Tier {
        use crate::tools::{CH_CONSOLE, CH_SCHEDULE, ID_ROOT, ID_SYSTEM};
        if ib.channel.starts_with(crate::jobs::PREFIX) || ib.channel == CH_SCHEDULE {
            return 1;
        }
        if ib.identity == ID_SYSTEM {
            if let Some(t) = crate::groups::notice_tier(&ib.channel) {
                return t;
            }
        }
        if ib.identity == ID_SYSTEM || ib.identity == ID_ROOT || ib.channel == CH_CONSOLE {
            return 0;
        }
        let fixed = self.groups.level_of(&ib.channel);
        if ib.group || self.groups.get(&ib.channel).is_some_and(|g| g.count > 0) {
            let base = match fixed {
                Some(t) => t,
                None if self.groups.get(&ib.channel).is_some_and(|g| g.all_root(&|id| self.root_id(id))) => 0,
                None => 3,
            };
            let by_sender = if self.root_id(&ib.sender) {
                1
            } else if calls_bia(&ib.text, self.me) {
                2
            } else {
                3
            };
            return base.min(by_sender);
        }
        if let Some(t) = fixed {
            return t;
        }
        let contact = self.book.find_for(&ib.identity, &ib.channel).or_else(|| self.book.find_by_id(&ib.sender));
        match contact {
            Some(c) if c.trust == "root" => 0,
            Some(_) => 1,
            None => 2,
        }
    }
}

/// One waiting message as the planner sees it.
#[derive(Clone, Debug)]
pub struct Item {
    pub tier: Tier,
    pub chat: String,
    pub issued: u64,
    pub tokens: usize,
}

/// The order messages are offered in: tier first; inside a tier one message per chat in turn
/// (the chat waiting longest first), each chat oldest first. A flooding group cannot starve a
/// direct chat of the same tier.
pub fn offer_order(items: &[Item]) -> Vec<usize> {
    let mut out = Vec::with_capacity(items.len());
    for tier in 0..TIERS {
        // chat → its messages of this tier, oldest first
        let mut chats: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (i, it) in items.iter().enumerate().filter(|(_, it)| it.tier == tier) {
            chats.entry(it.chat.clone()).or_default().push(i);
        }
        let mut lanes: Vec<Vec<usize>> = chats.into_values().collect();
        for l in lanes.iter_mut() {
            l.sort_by_key(|&i| (items[i].issued, i));
        }
        lanes.sort_by_key(|l| (items[l[0]].issued, l[0]));
        let mut k = 0;
        loop {
            let mut any = false;
            for l in &lanes {
                if let Some(&i) = l.get(k) {
                    out.push(i);
                    any = true;
                }
            }
            if !any {
                break;
            }
            k += 1;
        }
    }
    out
}

/// Which messages go in now: in [`offer_order`] while their tokens fit in `budget`, stopping at
/// the first that does not (a smaller, less important one never jumps ahead of it). The first
/// one always goes in, so the queue moves even when the context is tight (bodies are cut to
/// fit beforehand).
pub fn pick(items: &[Item], budget: usize) -> Vec<usize> {
    let mut used = 0;
    let mut out = vec![];
    for i in offer_order(items) {
        let t = items[i].tokens;
        if !out.is_empty() && used + t > budget {
            break;
        }
        used += t;
        out.push(i);
    }
    out
}

/// The order picked messages enter the conversation: each chat's messages together and in
/// their own order; the chats from least to most important (ties: the one waiting longest
/// first), so the most important is last, next to where the model starts answering.
pub fn delivery_order(items: &[Item], picked: &[usize]) -> Vec<usize> {
    let mut chats: Vec<(String, Vec<usize>)> = vec![];
    let mut at: HashMap<String, usize> = HashMap::new();
    for &i in picked {
        let c = &items[i].chat;
        match at.get(c) {
            Some(&k) => chats[k].1.push(i),
            None => {
                at.insert(c.clone(), chats.len());
                chats.push((c.clone(), vec![i]));
            }
        }
    }
    for (_, l) in chats.iter_mut() {
        l.sort_by_key(|&i| (items[i].issued, i));
    }
    chats.sort_by_key(|(_, l)| {
        let best = l.iter().map(|&i| items[i].tier).min().unwrap_or(TIERS);
        (std::cmp::Reverse(best), items[l[0]].issued)
    });
    chats.into_iter().flat_map(|(_, l)| l).collect()
}

/// Cut a body that alone would take more than 1/[`CUT_SHARE`] of the context. The whole text
/// goes to `dir/<id>.txt`; the body keeps its head and says where the rest is. Envelope, id
/// and times stay. Returns whether it was cut.
pub fn cut_long(ib: &mut Inbound, ctx: usize, dir: &std::path::Path, files: &std::path::Path) -> bool {
    if ctx == 0 || tokens(ib) <= ctx / CUT_SHARE {
        return false;
    }
    let keep_chars = (ctx / (CUT_SHARE * 2)) * 3;
    let total = ib.text.chars().count();
    if total <= keep_chars {
        return false;
    }
    let _ = std::fs::create_dir_all(dir);
    let safe: String = ib.id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    let path = dir.join(format!("{}.txt", if safe.is_empty() { crate::console::mint_msg_id() } else { safe }));
    let saved = std::fs::write(&path, &ib.text).is_ok();
    let shown = path.strip_prefix(files).map(|p| p.display().to_string()).unwrap_or_else(|_| path.display().to_string());
    let head: String = ib.text.chars().take(keep_chars).collect();
    ib.text = if saved {
        format!("{head}\n[cut: the message is {total} characters, too long for your context; this is its start. The whole text is in files/{shown}: read it in parts with bash (head, sed -n, grep)]")
    } else {
        format!("{head}\n[cut: the message is {total} characters, too long for your context; this is its start (the whole text could not be saved)]")
    };
    true
}

/// One chat's waiting messages, for the notice.
#[derive(Clone, Debug)]
pub struct Waiting {
    pub chat: String,
    pub who: String,
    pub count: usize,
    pub oldest: u64,
    pub tier: Tier,
}

/// Group the waiting messages by chat, most important and longest waiting first.
pub fn waiting(pending: &[(Tier, &Inbound)]) -> Vec<Waiting> {
    let mut by: BTreeMap<String, Waiting> = BTreeMap::new();
    for (t, ib) in pending {
        let issued = crate::clock::unix(ib.issued);
        let w = by.entry(ib.channel.clone()).or_insert_with(|| Waiting { chat: ib.channel.clone(), who: ib.identity.clone(), count: 0, oldest: issued, tier: *t });
        w.count += 1;
        w.oldest = w.oldest.min(issued);
        w.tier = w.tier.min(*t);
        if ib.group {
            w.who.clear();
        }
    }
    let mut v: Vec<Waiting> = by.into_values().collect();
    v.sort_by_key(|w| (w.tier, w.oldest));
    v
}

fn mins(now: u64, t: u64) -> String {
    let m = now.saturating_sub(t) / 60;
    if m < 1 {
        "under a minute".into()
    } else if m < 120 {
        format!("{m} min")
    } else {
        format!("{} h", m / 60)
    }
}

/// `[queue] …`: how much waits and where, which chats started waiting since the last notice
/// (they may need to hear that bia is busy: only the model talks to people), and how to take
/// more. `reserve`: the loop guard is on (a share of the context is kept free).
pub fn notice(w: &[Waiting], newly: &[String], now: u64, reserve: bool) -> String {
    let total: usize = w.iter().map(|x| x.count).sum();
    let oldest = w.iter().map(|x| x.oldest).min().unwrap_or(now);
    let mut lines = vec![format!(
        "[queue] {total} message(s) from {} chat(s) are still waiting (the oldest for {}): your context cannot take them all at once. They come to you by themselves, most important first (P0 root and root-only groups, P1 contacts, P2 strangers and replies or mentions of you in groups, P3 other group talk).",
        w.len(),
        mins(now, oldest)
    )];
    for x in w.iter().take(12) {
        let who = if x.who.is_empty() { String::new() } else { format!(" {}", x.who) };
        lines.push(format!("- {}{who}: {} waiting, P{}, oldest {} ago", x.chat, x.count, x.tier, mins(now, x.oldest)));
    }
    if w.len() > 12 {
        lines.push(format!("- … and {} more chat(s)", w.len() - 12));
    }
    if !newly.is_empty() {
        lines.push(format!(
            "Started waiting: {}. If someone there needs an answer soon, tell them briefly that you are busy and will reply; do not answer each message yet.",
            newly.join(", ")
        ));
    }
    lines.push("queue_take(chat, count) brings a chat's waiting messages now (as many as your context takes); queue_take() without a chat takes the most important.".into());
    if reserve {
        lines.push(format!("Reflections kept coming while the queue was long, so {LOOP_RESERVE_PCT}% of your context is now kept free for your own work until the queue is empty."));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn it(tier: Tier, chat: &str, issued: u64, tokens: usize) -> Item {
        Item { tier, chat: chat.into(), issued, tokens }
    }

    #[test]
    fn tiers_first_then_round_robin_over_chats() {
        let items = vec![
            it(3, "group", 1, 10),
            it(3, "group", 2, 10),
            it(3, "group", 3, 10),
            it(3, "dm", 5, 10),
            it(0, "root", 9, 10),
            it(1, "friend", 4, 10),
        ];
        // root first, then the friend, then the group and the dm take turns
        assert_eq!(offer_order(&items), vec![4, 5, 0, 3, 1, 2]);
    }

    #[test]
    fn pick_stops_at_the_first_that_does_not_fit_but_always_takes_one() {
        let items = vec![it(0, "a", 1, 50), it(1, "b", 2, 30), it(2, "c", 3, 5)];
        assert_eq!(pick(&items, 60), vec![0]);
        assert_eq!(pick(&items, 85), vec![0, 1, 2]);
        assert_eq!(pick(&items, 10), vec![0]);
    }

    #[test]
    fn delivery_keeps_chat_order_and_puts_the_most_important_chat_last() {
        let items = vec![it(0, "root", 9, 1), it(3, "group", 2, 1), it(3, "group", 1, 1), it(1, "friend", 5, 1)];
        let picked = pick(&items, 100);
        let order = delivery_order(&items, &picked);
        assert_eq!(order, vec![2, 1, 3, 0]);
    }

    fn ib(identity: &str, channel: &str, sender: &str, group: bool, text: &str) -> Inbound {
        let mut i = Inbound::console(text);
        i.identity = identity.into();
        i.channel = channel.into();
        i.sender = sender.into();
        i.group = group;
        i
    }

    #[test]
    fn ranks_people_and_groups() {
        let dir = std::env::temp_dir().join(format!("bia-queue-rank-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("addressbook.json");
        std::fs::write(&path, r#"{"contacts":[
            {"id":"boss","name":"Boss","trust":"root","channels":["telegram:1"]},
            {"id":"wife","name":"Wife","trust":"root","channels":["telegram:2"]},
            {"id":"friend","name":"Friend","trust":"normal","channels":["telegram:5"]},
            {"id":"insta","name":"Insta","trust":"root","channels":["instagram:t77"],"ids":["instagram:777"]}]}"#).unwrap();
        let book = crate::addressbook::AddressBook::load(&path);
        let mut groups = crate::groups::Groups::empty();
        let snap = |ch: &str, ids: &[&str]| serde_json::json!({"t": "members", "channel": ch, "count": ids.len(), "complete": true,
            "members": ids.iter().map(|i| serde_json::json!({"id": i, "name": ""})).collect::<Vec<_>>()});
        groups.apply(&snap("telegram:-100", &["telegram:1", "telegram:2"]));
        groups.apply(&snap("telegram:-200", &["telegram:1", "telegram:8"]));
        let rk = Ranker { book: &book, groups: &groups, me: "Milo" };
        // root and system
        assert_eq!(rk.tier(&Inbound::console("hi")), 0);
        assert_eq!(rk.tier(&ib("system", "members", "", false, "[members] …")), 0);
        assert_eq!(rk.tier(&ib("system", "members:p3", "", false, "[members] Eve joined")), 3);
        assert_eq!(rk.tier(&ib("system", "members:p1", "", false, "[members] Wife left")), 1);
        // direct chats: root-trust contact, contact, stranger
        assert_eq!(rk.tier(&ib("Boss (1)", "telegram:1", "telegram:1", false, "hi")), 0);
        assert_eq!(rk.tier(&ib("Friend (5)", "telegram:5", "telegram:5", false, "hi")), 1);
        assert_eq!(rk.tier(&ib("Who (7)", "telegram:7", "telegram:7", false, "hi")), 2);
        // a root person known by their platform id (the direct chat has another id)
        assert_eq!(rk.tier(&ib("Insta", "instagram:t99", "instagram:777", true, "hi")), 1);
        // a group of only root people
        assert_eq!(rk.tier(&ib("Wife (2)", "telegram:-100", "telegram:2", true, "dinner?")), 0);
        // a mixed group: root sender P1, a reply or a mention P2, the rest P3
        assert_eq!(rk.tier(&ib("Boss (1)", "telegram:-200", "telegram:1", true, "prices")), 1);
        assert_eq!(rk.tier(&ib("T (8)", "telegram:-200", "telegram:8", true, "[reply to 5 from you: \"ok\"]\nsure?")), 2);
        assert_eq!(rk.tier(&ib("T (8)", "telegram:-200", "telegram:8", true, "Milo, look")), 2);
        assert_eq!(rk.tier(&ib("T (8)", "telegram:-200", "telegram:8", true, "talk")), 3);
        // a group never listed counts as mixed even when its channel was linked to root
        assert_eq!(rk.tier(&ib("T (8)", "telegram:-300", "telegram:8", true, "talk")), 3);
        // the model's level holds, but a root sender still ranks at least P1
        groups.set_level("telegram:-200", "p2", "").unwrap();
        let rk = Ranker { book: &book, groups: &groups, me: "Milo" };
        assert_eq!(rk.tier(&ib("T (8)", "telegram:-200", "telegram:8", true, "talk")), 2);
        assert_eq!(rk.tier(&ib("Boss (1)", "telegram:-200", "telegram:1", true, "prices")), 1);
        groups.set_level("telegram:7", "p0", "").unwrap();
        let rk = Ranker { book: &book, groups: &groups, me: "Milo" };
        assert_eq!(rk.tier(&ib("Who (7)", "telegram:7", "telegram:7", false, "hi")), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn replies_mentions_and_wakes_call_bia() {
        assert!(calls_bia("[reply to 55 from you: \"ok\"]\nreally?", "Milo"));
        assert!(calls_bia("[reply to 55 from you, quoting: \"x\"]\nhm", ""));
        assert!(!calls_bia("[reply to 55 from Ann: \"ok\"]\nreally?", "Milo"));
        assert!(calls_bia("what does milo think?", "Milo"));
        assert!(!calls_bia("milonga tonight", "Milo"));
        assert!(calls_bia("[ignored chat telegram:-5: this message woke you (wake pattern)]", ""));
    }

    #[test]
    fn long_bodies_are_cut_and_kept_whole() {
        let dir = std::env::temp_dir().join(format!("bia-queue-cut-{}", std::process::id()));
        let files = dir.clone();
        let mut ib = Inbound::console("x".repeat(40_000));
        ib.id = "m1".into();
        assert!(cut_long(&mut ib, 8192, &dir.join("inbox/long"), &files));
        assert!(tokens(&ib) <= 8192 / CUT_SHARE, "{}", tokens(&ib));
        assert!(ib.text.contains("files/inbox/long/m1.txt"), "{}", &ib.text[ib.text.len() - 200..]);
        assert_eq!(std::fs::read_to_string(dir.join("inbox/long/m1.txt")).unwrap().len(), 40_000);
        let mut short = Inbound::console("hello");
        assert!(!cut_long(&mut short, 8192, &dir, &files));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_notice_names_chats_newly_waiting_and_how_to_take_more() {
        let w = vec![Waiting { chat: "telegram:-5".into(), who: String::new(), count: 20, oldest: 1000, tier: 3 }, Waiting { chat: "telegram:7".into(), who: "Ann (@ann, 7)".into(), count: 2, oldest: 1500, tier: 1 }];
        let n = notice(&w, &["telegram:7".into()], 1000 + 23 * 60, true);
        assert!(n.contains("22 message(s) from 2 chat(s)") && n.contains("23 min"), "{n}");
        assert!(n.contains("Started waiting: telegram:7") && n.contains("queue_take") && n.contains("20%"), "{n}");
    }
}
