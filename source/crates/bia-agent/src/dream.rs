//! Dreaming: long-term memory rebuilt by the model itself, at night.
//!
//! In the owner's quietest hours (learned from when root writes, all buses), a thread posts
//! `system|dream` tasks into the normal conversation, one at a time: reflect a past day from
//! the journal, a week from its days, a month, a year, and in the morning tell root what it
//! found. The model works them with tools (journal_day, memory_messages full, reflect_write,
//! report_write, task_done) like any task; people who write meanwhile are answered as usual.
//! When a task ends its tool traffic leaves the live context (the journal keeps it). After a
//! night the live context starts again from the calendar memory: last month, this month's
//! weeks, this week's days, today's reflections. What is true now is not a store of its own:
//! the model keeps it in blocks under categories it chooses, like everything else.
//!
//! Memory lives in generations (state/memory/gen-N): nodes (calendar reflections) and the
//! categories the model chose. `memory_rebuild` re-reflects the whole journal
//! into a new generation and switches to it when done; `rollback` goes back. The reflections
//! made when the context fills up (memory.rs) stay as they are: a daytime scratch layer.
use crate::console::{Console, Inbound};
use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const CH_DREAM: &str = "dream";

/// The tools a dream task works with: their calls and results leave the live context when
/// the task ends (the journal keeps them).
pub const DREAM_TOOLS: &[&str] = &[
    "journal_day",
    "memory_messages",
    "memory_list",
    "memory_get",
    "memory_tree",
    "memory_categories",
    "reflect_note",
    "reflect_write",
    "category_merge",
    "report_write",
    "task_done",
];

/// No nightly task starts while a person wrote less than this long ago.
const QUIET_SECS: u64 = 20 * 60;
/// An open task with no dream tool call this long is reminded; after this many reminders it closes.
const STALL_SECS: u64 = 5 * 60;
const STALL_NUDGES: u32 = 2;
/// Owner activity looked at for the quiet window.
const WINDOW_DAYS: u64 = 14;
/// Days of owner activity needed before the window is trusted (else the fallback below:
/// dreaming starts in its middle, 04:00).
const WINDOW_MIN_DAYS: u32 = 5;
/// Quiet window: the fallback when the owner's pattern is unknown.
const FALLBACK_START_MIN: u32 = 2 * 60;
const FALLBACK_END_MIN: u32 = 6 * 60;

// ---------------------------------------------------------------- calendar (local time)

/// A local calendar date.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Day {
    pub y: i64,
    pub m: u32,
    pub d: u32,
}

impl Day {
    pub fn parse(s: &str) -> Option<Day> {
        let s = s.trim();
        if s.len() != 10 || s.as_bytes()[4] != b'-' || s.as_bytes()[7] != b'-' {
            return None;
        }
        let y = s[..4].parse().ok()?;
        let m = s[5..7].parse().ok()?;
        let d = s[8..10].parse().ok()?;
        if !(1..=12).contains(&m) || d == 0 || d > crate::clock::days_in_month(y, m) {
            return None;
        }
        Some(Day { y, m, d })
    }

    fn days(self) -> i64 {
        crate::clock::days_from_civil(self.y, self.m, self.d)
    }

    fn from_days(n: i64) -> Day {
        let (y, m, d) = crate::clock::civil_from_days(n);
        Day { y, m, d }
    }

    pub fn add(self, n: i64) -> Day {
        Day::from_days(self.days() + n)
    }

    /// 0 = Monday … 6 = Sunday
    fn weekday_mon0(self) -> u32 {
        // 1970-01-01 (day 0) was a Thursday (Monday-based 3)
        ((self.days() + 3).rem_euclid(7)) as u32
    }

    pub fn monday(self) -> Day {
        self.add(-(self.weekday_mon0() as i64))
    }

    /// Local midnight of this date, as Unix seconds (the offset of that moment).
    pub fn start(self) -> u64 {
        let naive = (self.days().max(0) as u64) * 86400;
        let mut t = naive as i64 - crate::clock::offset_east_at(naive) as i64;
        // the offset can differ across the change itself: settle once more
        t = naive as i64 - crate::clock::offset_east_at(t.max(0) as u64) as i64;
        t.max(0) as u64
    }

    pub fn of(unix: u64) -> Day {
        let local = unix as i64 + crate::clock::offset_east_at(unix) as i64;
        Day::from_days(local.div_euclid(86400))
    }
}

impl std::fmt::Display for Day {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.y, self.m, self.d)
    }
}

/// Minutes since local midnight at `unix`.
fn local_minute(unix: u64) -> u32 {
    let local = unix as i64 + crate::clock::offset_east_at(unix) as i64;
    (local.rem_euclid(86400) / 60) as u32
}

fn hhmm(min: u32) -> String {
    format!("{:02}:{:02}", (min / 60) % 24, min % 60)
}

fn now() -> u64 {
    crate::clock::unix(crate::clock::now())
}

// ---------------------------------------------------------------- tasks

#[derive(Clone, Debug, PartialEq)]
pub enum Task {
    Day(Day),
    /// the Monday of the week
    Week(Day),
    Month(i64, u32),
    Year(i64),
    /// tell root about the night's report (the date the night ended)
    Morning(Day),
}

impl Task {
    pub fn key(&self) -> String {
        match self {
            Task::Day(d) => format!("day:{d}"),
            Task::Week(d) => format!("week:{d}"),
            Task::Month(y, m) => format!("month:{y:04}-{m:02}"),
            Task::Year(y) => format!("year:{y:04}"),
            Task::Morning(d) => format!("morning:{d}"),
        }
    }

    pub fn parse(key: &str) -> Option<Task> {
        let (k, v) = key.split_once(':')?;
        Some(match k {
            "day" => Task::Day(Day::parse(v)?),
            "week" => Task::Week(Day::parse(v)?),
            "month" => {
                let (y, m) = v.split_once('-')?;
                Task::Month(y.parse().ok()?, m.parse().ok()?)
            }
            "year" => Task::Year(v.parse().ok()?),
            "morning" => Task::Morning(Day::parse(v)?),
            _ => return None,
        })
    }

    fn level(&self) -> &'static str {
        match self {
            Task::Day(_) => "day",
            Task::Week(_) => "week",
            Task::Month(..) => "month",
            Task::Year(_) => "year",
            Task::Morning(_) => "morning",
        }
    }

    /// The period's name as nodes keep it: 2026-10-27, week 2026-10-26, 2026-10, 2026.
    fn period(&self) -> String {
        match self {
            Task::Day(d) | Task::Week(d) | Task::Morning(d) => d.to_string(),
            Task::Month(y, m) => format!("{y:04}-{m:02}"),
            Task::Year(y) => format!("{y:04}"),
        }
    }

    /// Local first day and the day after the last.
    fn days(&self) -> (Day, Day) {
        match *self {
            Task::Day(d) | Task::Morning(d) => (d, d.add(1)),
            Task::Week(d) => (d, d.add(7)),
            Task::Month(y, m) => {
                let first = Day { y, m, d: 1 };
                (first, first.add(crate::clock::days_in_month(y, m) as i64))
            }
            Task::Year(y) => (Day { y, m: 1, d: 1 }, Day { y: y + 1, m: 1, d: 1 }),
        }
    }

    fn span(&self) -> (u64, u64) {
        let (a, b) = self.days();
        (a.start(), b.start())
    }
}

// ---------------------------------------------------------------- store

/// A calendar reflection: one block (category) of a day, week, month or year.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub level: String,
    pub period: String,
    pub t_start: u64,
    pub t_end: u64,
    pub category: String,
    /// the block that says the period in short (the live context and lists show it)
    #[serde(default)]
    pub overview: bool,
    pub text: String,
    #[serde(default)]
    pub sources: Vec<String>,
    #[serde(default)]
    pub created_at: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Category {
    #[serde(default)]
    pub meaning: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merged_into: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct GenMeta {
    id: u32,
    /// the first local day this generation covers
    start_day: String,
    created_at: String,
}

/// One generation of the calendar memory on disk.
struct Gen {
    dir: PathBuf,
    meta: GenMeta,
}

impl Gen {
    fn open(root: &Path, id: u32) -> Option<Gen> {
        let dir = root.join(format!("gen-{id}"));
        let meta: GenMeta = serde_json::from_slice(&std::fs::read(dir.join("meta.json")).ok()?).ok()?;
        Some(Gen { dir, meta })
    }

    fn create(root: &Path, id: u32, start_day: Day) -> Result<Gen> {
        let dir = root.join(format!("gen-{id}"));
        std::fs::create_dir_all(&dir)?;
        let meta = GenMeta { id, start_day: start_day.to_string(), created_at: crate::clock::rfc3339_now() };
        write_atomic(&dir.join("meta.json"), &serde_json::to_vec_pretty(&meta)?)?;
        Ok(Gen { dir, meta })
    }

    fn start_day(&self) -> Day {
        Day::parse(&self.meta.start_day).unwrap_or(Day::of(now()))
    }

    fn nodes(&self) -> Vec<Node> {
        read_jsonl(&self.dir.join("nodes.jsonl"))
    }

    fn add_node(&self, n: &Node) -> Result<()> {
        append_jsonl(&self.dir.join("nodes.jsonl"), n)
    }

    fn categories(&self) -> BTreeMap<String, Category> {
        std::fs::read(self.dir.join("categories.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    fn save_categories(&self, c: &BTreeMap<String, Category>) -> Result<()> {
        write_atomic(&self.dir.join("categories.json"), &serde_json::to_vec_pretty(c)?)
    }

    /// Periods with nothing to reflect (no messages): never asked again.
    fn empty(&self) -> BTreeSet<String> {
        std::fs::read(self.dir.join("empty.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    fn mark_empty(&self, key: &str) -> Result<()> {
        let mut e = self.empty();
        e.insert(key.to_string());
        write_atomic(&self.dir.join("empty.json"), &serde_json::to_vec(&e)?)
    }

    /// Periods (`level:period`) that have an overview.
    fn done(&self) -> BTreeSet<String> {
        self.nodes().into_iter().filter(|n| n.overview).map(|n| format!("{}:{}", n.level, n.period)).collect()
    }
}

// ---------------------------------------------------------------- state

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Settings {
    #[serde(default = "yes")]
    enabled: bool,
    /// HH:MM local: dream then instead of in the middle of the learned quiet window
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fixed_time: Option<String>,
}

fn yes() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Settings { enabled: true, fixed_time: None }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Window {
    /// minutes since local midnight; the window may wrap past midnight
    start: u32,
    end: u32,
    /// days with owner activity in the last WINDOW_DAYS
    days: u32,
    /// how many of them were quiet over the whole window
    quiet_days: u32,
    fallback: bool,
    /// local date it was computed
    computed: String,
}

impl Window {
    fn len(&self) -> u32 {
        (self.end + 1440 - self.start) % 1440
    }
    fn middle(&self) -> u32 {
        (self.start + self.len() / 2) % 1440
    }
    /// `min` (minutes since midnight) inside [from, end)
    fn contains_from(&self, from: u32, min: u32) -> bool {
        let span = (self.end + 1440 - from) % 1440;
        (min + 1440 - from) % 1440 < span
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Open {
    key: String,
    gen: u32,
    posted_at: u64,
    #[serde(default)]
    notes: Vec<String>,
    #[serde(default)]
    overview: bool,
    #[serde(default)]
    writes: u32,
    #[serde(default)]
    fails: u32,
    /// the last dream tool call for it
    #[serde(default)]
    touched: u64,
    /// reminders after it stalled
    #[serde(default)]
    nudges: u32,
    /// the morning report this task writes to (night runs)
    #[serde(default)]
    report_day: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Rebuild {
    gen: u32,
    paused: bool,
    started_at: String,
    total: usize,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Report {
    /// local date of the morning it is for
    day: String,
    path: String,
    /// when the night's work ended (root writing after this gets it)
    night_end: u64,
    /// minute of the window's end that day (deliver by then)
    due_min: u32,
    posted: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct State {
    #[serde(default)]
    settings: Settings,
    #[serde(default)]
    current_gen: u32,
    #[serde(default)]
    previous_gen: Option<u32>,
    /// the morning (local date) whose night is done
    #[serde(default)]
    night_done: String,
    /// a night (or /dream now) is running: the morning it is for
    #[serde(default)]
    night: Option<String>,
    /// started with dream now: no waiting for quiet (it still goes one task at a time)
    #[serde(default)]
    night_now: bool,
    #[serde(default)]
    open: Option<Open>,
    #[serde(default)]
    rebuild: Option<Rebuild>,
    #[serde(default)]
    report: Option<Report>,
    #[serde(default)]
    spine: bool,
    #[serde(default)]
    window: Option<Window>,
    #[serde(default)]
    seq: u64,
    /// when the last rebuild estimate was shown (start needs root's yes after it)
    #[serde(default)]
    estimate_at: u64,
}

/// What a finished task leaves in the live context.
pub struct Finished {
    pub key: String,
    pub note: String,
}

pub struct Dream {
    state_dir: PathBuf,
    files_dir: PathBuf,
    root: PathBuf,
    st: Mutex<State>,
    /// last human message (any) and last message from the owner (root trust)
    last_human: AtomicU64,
    last_root: AtomicU64,
    /// the live context: its size and use (tokens), for page sizes and estimates
    ctx_total: AtomicUsize,
    ctx_used: AtomicUsize,
    finished: Mutex<Option<Finished>>,
    /// the live context filled up during the open task: the loop re-posts it
    reset: AtomicBool,
    /// the last human inbound this turn was root's
    turn_root: AtomicBool,
}

impl Dream {
    pub fn open(files_dir: &Path) -> Arc<Dream> {
        let state_dir = files_dir.join("state");
        let root = state_dir.join("memory");
        let _ = std::fs::create_dir_all(&root);
        let mut st: State = std::fs::read(root.join("dream.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        migrate_facts(&root);
        if st.current_gen == 0 || Gen::open(&root, st.current_gen).is_none() {
            // the first generation starts today: days before it come with memory_rebuild
            let id = st.current_gen.max(1);
            let _ = Gen::create(&root, id, Day::of(now()));
            st.current_gen = id;
        }
        let d = Arc::new(Dream {
            state_dir,
            files_dir: files_dir.to_path_buf(),
            root,
            st: Mutex::new(st),
            last_human: AtomicU64::new(0),
            last_root: AtomicU64::new(0),
            ctx_total: AtomicUsize::new(0),
            ctx_used: AtomicUsize::new(0),
            finished: Mutex::new(None),
            reset: AtomicBool::new(false),
            turn_root: AtomicBool::new(false),
        });
        d.save(&d.st.lock().unwrap());
        d
    }

    fn save(&self, st: &State) {
        if let Ok(b) = serde_json::to_vec_pretty(st) {
            let _ = write_atomic(&self.root.join("dream.json"), &b);
        }
    }

    fn gen(&self, id: u32) -> Option<Gen> {
        Gen::open(&self.root, id)
    }

    fn current(&self) -> Option<Gen> {
        let id = self.st.lock().unwrap().current_gen;
        self.gen(id)
    }

    /// The loop tells who wrote: any person, and whether it was the owner.
    pub fn note_human(&self, root: bool) {
        let t = now();
        self.last_human.store(t, Ordering::SeqCst);
        if root {
            self.last_root.store(t, Ordering::SeqCst);
        }
        self.turn_root.store(root, Ordering::SeqCst);
    }

    /// A dream tool was called: the open task is alive.
    fn touch(&self) {
        let mut st = self.st.lock().unwrap();
        if let Some(o) = st.open.as_mut() {
            o.touched = now();
        }
    }

    pub fn set_ctx(&self, total: usize, used: usize) {
        self.ctx_total.store(total, Ordering::SeqCst);
        self.ctx_used.store(used, Ordering::SeqCst);
    }

    /// A page of the journal digest: about a quarter of the context (characters).
    fn page_chars(&self) -> usize {
        let t = self.ctx_total.load(Ordering::SeqCst);
        if t == 0 { 24_000 } else { t.clamp(8_000, 200_000) }
    }

    #[cfg(test)]
    pub fn open_task(&self) -> Option<String> {
        self.st.lock().unwrap().open.as_ref().map(|o| o.key.clone())
    }

    /// The task just ended (task_done passed): the loop removes its traffic.
    pub fn take_finished(&self) -> Option<Finished> {
        self.finished.lock().unwrap().take()
    }

    /// The live context filled up during the open task: its traffic goes, and the task is
    /// posted again with the notes the model kept.
    pub fn reset_open(&self) -> Option<String> {
        let st = self.st.lock().unwrap();
        let key = st.open.as_ref()?.key.clone();
        self.reset.store(true, Ordering::SeqCst);
        Some(key)
    }

    /// The live context should start again from the calendar memory (after a night, a
    /// rebuild switch or a rollback): its stubs, oldest first. None while a task is open.
    pub fn take_spine(&self) -> Option<Vec<String>> {
        let mut st = self.st.lock().unwrap();
        if !st.spine || st.open.is_some() {
            return None;
        }
        st.spine = false;
        self.save(&st);
        drop(st);
        let g = self.current()?;
        let stubs = spine(&g, now(), self.page_chars() / 2);
        (!stubs.is_empty()).then_some(stubs)
    }
}

// ---------------------------------------------------------------- the owner's quiet window

/// Owner activity: journal messages from root (console, web) or a chat linked to a contact
/// with root trust (telegram, whatsapp, messenger, instagram).
fn owner_times(state_dir: &Path, since: u64) -> Vec<u64> {
    let ab = crate::addressbook::AddressBook::load(&state_dir.join("addressbook.json"));
    let st = crate::memory::Store::open(state_dir);
    let mut out = vec![];
    for m in st.journal_all() {
        if m.role != "user" {
            continue;
        }
        let mut f = m.content.splitn(6, '|');
        let (Some(identity), Some(channel), Some(issued)) = (f.next(), f.next(), f.next()) else { continue };
        let Some(t) = crate::clock::parse_rfc3339(issued) else { continue };
        if t < since || identity == crate::tools::ID_SYSTEM {
            continue;
        }
        let root = identity == crate::tools::ID_ROOT
            || ab.find_for(identity, channel).is_some_and(|c| c.trust == "root");
        if root {
            out.push(t);
        }
    }
    out
}

/// The longest stretch of the day (15-minute slots, wrapping midnight) in which the owner
/// wrote on at most one day in ten.
fn quiet_window(times: &[u64], today: Day) -> Window {
    const SLOTS: usize = 96;
    let mut per_slot = vec![BTreeSet::new(); SLOTS];
    let mut days = BTreeSet::new();
    for &t in times {
        let day = Day::of(t);
        days.insert(day);
        per_slot[(local_minute(t) / 15) as usize % SLOTS].insert(day);
    }
    let n = days.len() as u32;
    let fallback = Window {
        start: FALLBACK_START_MIN,
        end: FALLBACK_END_MIN,
        days: n,
        quiet_days: 0,
        fallback: true,
        computed: today.to_string(),
    };
    if n < WINDOW_MIN_DAYS {
        return fallback;
    }
    let allowed = (n / 10).max(1);
    let quiet: Vec<bool> = per_slot.iter().map(|s| s.len() as u32 <= allowed).collect();
    if quiet.iter().all(|q| *q) {
        return fallback;
    }
    // longest circular run of quiet slots
    let (mut best_start, mut best_len) = (0usize, 0usize);
    for start in 0..SLOTS {
        if !quiet[start] || quiet[(start + SLOTS - 1) % SLOTS] {
            continue;
        }
        let mut len = 0;
        while len < SLOTS && quiet[(start + len) % SLOTS] {
            len += 1;
        }
        if len > best_len {
            best_start = start;
            best_len = len;
        }
    }
    if best_len < 8 {
        // under two hours of quiet anywhere: no pattern to trust
        return fallback;
    }
    let start = (best_start * 15) as u32;
    let end = (((best_start + best_len) % SLOTS) * 15) as u32;
    let quiet_days = days
        .iter()
        .filter(|d| {
            times.iter().all(|&t| {
                Day::of(t) != **d || {
                    let m = local_minute(t);
                    !Window { start, end, ..Default::default() }.contains_from(start, m)
                }
            })
        })
        .count() as u32;
    Window { start, end, days: n, quiet_days, fallback: false, computed: today.to_string() }
}

impl Dream {
    fn window(&self, st: &mut State) -> Window {
        let today = Day::of(now()).to_string();
        let fresh = st.window.as_ref().is_some_and(|w| w.computed == today);
        if !fresh {
            let since = now().saturating_sub(WINDOW_DAYS * 86400);
            st.window = Some(quiet_window(&owner_times(&self.state_dir, since), Day::of(now())));
        }
        let mut w = st.window.clone().unwrap_or_default();
        if let Some(f) = st.settings.fixed_time.as_deref().and_then(parse_hhmm) {
            // a fixed time: dream then; the morning report is due four hours later
            w.start = (f + 1440 - 60) % 1440;
            w.end = (f + 240) % 1440;
        }
        w
    }

    /// When tonight's dream starts (minutes since midnight).
    fn dream_minute(st: &State, w: &Window) -> u32 {
        st.settings.fixed_time.as_deref().and_then(parse_hhmm).unwrap_or_else(|| w.middle())
    }
}

fn parse_hhmm(s: &str) -> Option<u32> {
    let (h, m) = s.trim().split_once(':')?;
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

// ---------------------------------------------------------------- the journal digest

struct Row {
    id: String,
    line: String,
}

/// The journal of [from, to) as lines: what people and bia said in full, tool results cut
/// (head and tail, with the id that memory_messages gives in full); earlier dream work and
/// system notices about it are left out.
fn digest(state_dir: &Path, from: u64, to: u64) -> Vec<Row> {
    let st = crate::memory::Store::open(state_dir);
    let mut t = 0u64;
    let mut out = vec![];
    let mut in_dream = false;
    for m in st.journal_all() {
        if m.role == "user" {
            if let Some(x) = crate::memory::issued_unix(&m.content) {
                t = x;
            }
            let mut f = m.content.splitn(6, '|');
            let identity = f.next().unwrap_or("");
            let channel = f.next().unwrap_or("");
            let body = m.content.splitn(6, '|').nth(5).unwrap_or(&m.content);
            in_dream = channel == CH_DREAM;
            if t < from || t >= to || in_dream || crate::memory::is_stub(&m.name, &m.content) {
                continue;
            }
            let when = crate::clock::format_hms(t, crate::clock::offset_east_at(t));
            out.push(Row { id: m.id.clone(), line: format!("[{} {} {identity}@{channel}] {}", m.id, &when[..5], cut(body, 4000, &m.id)) });
            continue;
        }
        if t < from || t >= to {
            continue;
        }
        match m.role.as_str() {
            "assistant" => {
                let text = m.content.trim();
                if !text.is_empty() && !in_dream {
                    out.push(Row { id: m.id.clone(), line: format!("[{} bia] {}", m.id, cut(text, 4000, &m.id)) });
                }
            }
            "tool" => {
                if DREAM_TOOLS.contains(&m.name.as_str()) {
                    continue;
                }
                out.push(Row { id: m.id.clone(), line: format!("[{} → {}] {}", m.id, m.name, cut(m.content.trim(), 500, &m.id)) });
            }
            _ => {}
        }
    }
    out
}

/// Long text: its head and tail, and how to get it all.
fn cut(s: &str, max: usize, id: &str) -> String {
    let s = redact(s);
    let n = s.chars().count();
    if n <= max {
        return s.replace('\n', " ⏎ ");
    }
    let head: String = s.chars().take(max * 2 / 3).collect();
    let tail: String = s.chars().skip(n - max / 3).collect();
    format!(
        "{} … [{} characters cut; memory_messages(ids=[\"{id}\"], full=true)] … {}",
        head.replace('\n', " ⏎ "),
        n - max,
        tail.replace('\n', " ⏎ ")
    )
}

/// Keys, tokens and passwords that look like ones are masked before memory keeps text.
pub fn redact(s: &str) -> String {
    const PREFIXES: &[&str] = &["sk-or-v1-", "sk-ant-", "sk-proj-", "sk-", "xai-", "ghp_", "gho_", "github_pat_", "AKIA", "AIza", "xoxb-", "xoxp-", "glpat-"];
    let mut out = String::with_capacity(s.len());
    let b = s.as_bytes();
    let mut i = 0;
    let tokenish = |c: u8| c.is_ascii_alphanumeric() || c == b'-' || c == b'_' || c == b'.';
    while i < b.len() {
        if let Some(p) = PREFIXES.iter().find(|p| s[i..].starts_with(**p) && (i == 0 || !tokenish(b[i - 1]))) {
            let mut j = i + p.len();
            while j < b.len() && tokenish(b[j]) {
                j += 1;
            }
            if j - i >= p.len() + 16 {
                out.push_str(p);
                out.push_str("[redacted]");
                i = j;
                continue;
            }
        }
        if s[i..].starts_with("-----BEGIN") && s[i..].contains("PRIVATE KEY-----") {
            if let Some(e) = s[i..].find("-----END") {
                let end = s[i + e..].find("-----\n").map(|k| i + e + k + 5).unwrap_or(b.len()).min(b.len());
                out.push_str("[private key redacted]");
                i = end;
                continue;
            }
        }
        // JWTs (eyJ….….…)
        if s[i..].starts_with("eyJ") && (i == 0 || !tokenish(b[i - 1])) {
            let mut j = i;
            while j < b.len() && tokenish(b[j]) {
                j += 1;
            }
            if j - i > 60 && s[i..j].matches('.').count() >= 2 {
                out.push_str("[token redacted]");
                i = j;
                continue;
            }
        }
        let ch = s[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Rows split into pages of about `chars` characters.
fn pages(rows: &[Row], chars: usize) -> Vec<(usize, usize)> {
    let mut out = vec![];
    let (mut start, mut used) = (0usize, 0usize);
    for (i, r) in rows.iter().enumerate() {
        let n = r.line.len() + 1;
        if used > 0 && used + n > chars {
            out.push((start, i));
            start = i;
            used = 0;
        }
        used += n;
    }
    if start < rows.len() {
        out.push((start, rows.len()));
    }
    out
}

// ---------------------------------------------------------------- what is due

impl Dream {
    /// The next task of a generation, in dependency order: the oldest day without a
    /// reflection, then weeks whose days are done, months, years. `upto` is the first day
    /// not to do (today).
    fn next_task(&self, g: &Gen, upto: Day) -> Option<Task> {
        let done = g.done();
        let empty = g.empty();
        let has = |t: &Task| done.contains(&format!("{}:{}", t.level(), t.period())) || empty.contains(&t.key());
        let first = g.start_day();
        // the journal's first day: a rebuild starts there, not at an empty gen start
        let mut d = first;
        while d < upto {
            let t = Task::Day(d);
            if !has(&t) {
                let (a, b) = t.span();
                if digest(&self.state_dir, a, b).is_empty() {
                    let _ = g.mark_empty(&t.key());
                } else {
                    return Some(t);
                }
            }
            d = d.add(1);
        }
        let day_done = |d: Day| done.contains(&format!("day:{d}"));
        // weeks that ended before `upto`
        let mut w = first.monday();
        while w.add(7) <= upto {
            let t = Task::Week(w);
            if !has(&t) && (0..7).any(|i| day_done(w.add(i))) {
                return Some(t);
            }
            w = w.add(7);
        }
        // months that ended
        let (mut y, mut m) = (first.y, first.m);
        loop {
            let t = Task::Month(y, m);
            if t.days().1 > upto {
                break;
            }
            let (a, b) = t.days();
            if !has(&t) && done.iter().any(|k| k.strip_prefix("day:").and_then(Day::parse).is_some_and(|d| d >= a && d < b)) {
                return Some(t);
            }
            if m == 12 {
                y += 1;
                m = 1;
            } else {
                m += 1;
            }
        }
        // years that ended
        let mut y = first.y;
        while (Day { y: y + 1, m: 1, d: 1 }) <= upto {
            let t = Task::Year(y);
            if !has(&t) && done.iter().any(|k| k.starts_with(&format!("month:{y:04}-"))) {
                return Some(t);
            }
            y += 1;
        }
        None
    }

    /// How many tasks a generation still has (for progress lines).
    fn remaining(&self, g: &Gen, upto: Day) -> usize {
        let done = g.done();
        let empty = g.empty();
        let mut n = 0;
        let mut d = g.start_day();
        while d < upto {
            if !done.contains(&format!("day:{d}")) && !empty.contains(&format!("day:{d}")) {
                n += 1;
            }
            d = d.add(1);
        }
        n
    }
}

// ---------------------------------------------------------------- task messages

fn tz_label() -> String {
    crate::clock::timezone_name()
}

impl Dream {
    fn task_text(&self, g: &Gen, t: &Task, notes: &[String], rebuild: bool) -> String {
        let key = t.key();
        let common = "People may write while you do this: answer them as usual (if someone wants something long, you may say you are sorting your memory and will be back shortly), then continue here. \
Do not change settings, schedules or software during this work: proposals go to report_write(section=\"Needs your yes\"). Never copy secrets (keys, passwords, codes) into memory. \
Long work is paused now and then: for this task nobody needs a status, so do not message anyone about it; just go on.";
        let notes_s = if notes.is_empty() {
            String::new()
        } else {
            format!("\nThe context filled up before; your notes so far:\n- {}\n", notes.join("\n- "))
        };
        let report = if rebuild {
            "This is a rebuild of all memory: no morning report; leave report_write out."
        } else {
            "Ideas worth telling the owner in the morning (something to improve, a routine to take off them, a problem you noticed in your own work): report_write(section, text). Only when there is something worth saying; otherwise write nothing."
        };
        match t {
            Task::Day(d) => {
                let (a, b) = t.span();
                let rows = digest(&self.state_dir, a, b);
                let n_pages = pages(&rows, self.page_chars()).len().max(1);
                format!(
                    "[dream task {key}] Reflect the day {d} ({tz}, local midnight to midnight) into long-term memory: {} messages, {n_pages} page(s).\n\
1. Read it: journal_day(page=1 … {n_pages}). Tool outputs there are cut; memory_messages(ids=[…], full=true) gives anything in full. Older memory: memory_list, memory_get.\n\
2. Write it with reflect_write: one block with overview=true (the day in short: what happened and what mattered), then the blocks you find worth keeping, each under a category you choose (memory_categories lists them with their meaning; reuse one when it fits, give meaning for a new one). Cite message ids in sources.\n\
3. What is true now (people, accounts, rules, preferences, promises) goes in a block too, under a category you choose. When something changed, write the new state and what it replaced (was …, now … since …): later periods are read first, so the newest block wins. A conflict you cannot settle: write both and report_write(section=\"Questions\", …).\n\
4. {report}\n\
{common}{notes_s}\nThen task_done.",
                    rows.len(),
                    tz = tz_label()
                )
            }
            Task::Week(_) | Task::Month(..) | Task::Year(_) => {
                let (a, b) = t.days();
                let child_level = match t {
                    Task::Week(_) | Task::Month(..) => "day",
                    _ => "month",
                };
                let children: Vec<String> = g
                    .nodes()
                    .into_iter()
                    .filter(|n| n.overview && n.level == child_level)
                    .filter(|n| Day::parse(&format!("{}-01", n.period)).or(Day::parse(&n.period)).is_some_and(|d| d >= a && d < b))
                    .map(|n| format!("{} {} ({}): {}", n.period, n.id, n.category, one_line(&n.text, 600)))
                    .collect();
                let what = match t {
                    Task::Week(d) => format!("the week {d} – {}", d.add(6)),
                    Task::Month(y, m) => format!("the month {y:04}-{m:02}"),
                    Task::Year(y) => format!("the year {y:04}"),
                    _ => unreachable!(),
                };
                format!(
                    "[dream task {key}] Reflect {what} ({tz}) from its {child_level}s:\n{}\n\
memory_get(id) reads any of them in full (other categories too: memory_list), memory_messages the raw messages when a summary seems wrong or thin. \
Write it with reflect_write: overview=true for the period in short, then the blocks worth keeping (your categories; category_merge when two turned out to be the same), including what holds true across the period or changed in it, if the days missed it. {report}\n\
{common}{notes_s}\nThen task_done.",
                    if children.is_empty() { "(no reflections below it)".into() } else { children.join("\n") },
                    tz = tz_label()
                )
            }
            Task::Morning(d) => {
                let path = self.report_path(*d);
                format!(
                    "[dream task {key}] Good morning task: last night you wrote {path}. Tell root about it in a few lines, in their language, on the chat they use most, with the file attached (say files=[\"{path}\"]). Then task_done.",
                    path = path.display()
                )
            }
        }
    }

    fn report_path(&self, d: Day) -> PathBuf {
        self.files_dir.join("reports").join(format!("morning-{d}.md"))
    }
}

fn one_line(s: &str, max: usize) -> String {
    let body = s.trim().replace('\n', " ");
    if body.chars().count() <= max {
        body
    } else {
        format!("{}…", body.chars().take(max).collect::<String>())
    }
}

// ---------------------------------------------------------------- the scheduler

pub fn start(d: Arc<Dream>, con: Arc<Console>) {
    let _ = std::thread::Builder::new().name("dream".into()).spawn(move || loop {
        std::thread::sleep(Duration::from_secs(20));
        if let Some(text) = d.tick() {
            con.push_inbound(
                Inbound {
                    identity: crate::tools::ID_SYSTEM.into(),
                    channel: CH_DREAM.into(),
                    issued: crate::clock::now(),
                    id: String::new(),
                    text,
                }
                .ensure_id(),
            );
        }
    });
}

impl Dream {
    /// What to post now, if anything (one task at a time).
    pub fn tick(&self) -> Option<String> {
        let mut st = self.st.lock().unwrap();
        let t = now();
        let today = Day::of(t);
        let quiet_for = t.saturating_sub(self.last_human.load(Ordering::SeqCst));

        // the open task again, after the live context filled up
        if self.reset.swap(false, Ordering::SeqCst) {
            let o = st.open.clone()?;
            let g = self.gen(o.gen)?;
            let task = Task::parse(&o.key)?;
            let rebuild = st.rebuild.as_ref().is_some_and(|r| r.gen == o.gen);
            if let Some(op) = st.open.as_mut() {
                op.posted_at = t;
            }
            self.save(&st);
            return Some(self.task_text(&g, &task, &o.notes, rebuild));
        }
        // the open task stalled (the model went idle without task_done): remind it, then close it
        if let Some(o) = st.open.clone() {
            if t.saturating_sub(o.posted_at.max(o.touched)) < STALL_SECS || quiet_for < 120 {
                return None;
            }
            let g = self.gen(o.gen)?;
            let task = Task::parse(&o.key)?;
            if o.nudges >= STALL_NUDGES {
                if !o.overview && !matches!(task, Task::Morning(_)) {
                    let _ = auto_overview(&g, &task);
                }
                st.open = None;
                self.save(&st);
                *self.finished.lock().unwrap() = Some(Finished { key: task.key(), note: "closed: no progress".into() });
                return None;
            }
            let rebuild = st.rebuild.as_ref().is_some_and(|r| r.gen == o.gen);
            if let Some(op) = st.open.as_mut() {
                op.nudges += 1;
                op.touched = t;
            }
            self.save(&st);
            return Some(format!(
                "[dream task {}] is still open: no dream tool was called for it for a while. Go on with it, and end it with task_done(note=…) (call task_done by name, not inside discover's arguments).\n\n{}",
                o.key,
                self.task_text(&g, &task, &o.notes, rebuild)
            ));
        }

        // a rebuild runs now, one task after another
        if let Some(r) = st.rebuild.clone() {
            if r.paused {
                return None;
            }
            let g = self.gen(r.gen)?;
            match self.next_task(&g, today) {
                Some(task) => return Some(self.post(&mut st, &g, task, true, String::new())),
                None => {
                    // done up to yesterday: switch to it
                    st.previous_gen = Some(st.current_gen);
                    st.current_gen = r.gen;
                    st.rebuild = None;
                    st.spine = true;
                    st.night_done = today.to_string();
                    self.save(&st);
                    return Some(format!(
                        "memory_rebuild finished: all memory up to {} is reflected again (generation {}); you think from it now. Tell root it is done. If it is worse than before, memory_rebuild(action=\"rollback\") goes back.",
                        today.add(-1),
                        r.gen
                    ));
                }
            }
        }

        let w = self.window(&mut st);
        let minute = local_minute(t);

        // the morning: tell root about the night's report
        if let Some(rep) = st.report.clone() {
            let due = (self.last_root.load(Ordering::SeqCst) > rep.night_end) || (rep.day == today.to_string() && minute >= rep.due_min && minute < rep.due_min + 180);
            if !rep.posted && due && Path::new(&rep.path).exists() {
                let day = Day::parse(&rep.day).unwrap_or(today);
                let g = self.gen(st.current_gen)?;
                if let Some(r) = st.report.as_mut() {
                    r.posted = true;
                }
                return Some(self.post(&mut st, &g, Task::Morning(day), false, String::new()));
            }
            if rep.posted || Day::parse(&rep.day).is_some_and(|d| d < today.add(-1)) {
                st.report = None;
                self.save(&st);
            }
        }

        // a night in progress: the next task when nobody is talking
        if let Some(morning) = st.night.clone() {
            if !st.night_now && quiet_for < QUIET_SECS {
                return None;
            }
            let g = self.gen(st.current_gen)?;
            match self.next_task(&g, today) {
                Some(task) => return Some(self.post(&mut st, &g, task, false, morning)),
                None => {
                    st.night = None;
                    st.night_now = false;
                    st.night_done = morning.clone();
                    st.spine = true;
                    let path = self.report_path(Day::parse(&morning).unwrap_or(today));
                    if path.exists() {
                        st.report = Some(Report {
                            day: morning,
                            path: path.display().to_string(),
                            night_end: t,
                            due_min: w.end,
                            posted: false,
                        });
                    }
                    self.save(&st);
                    return None;
                }
            }
        }

        // tonight's dream: in the window, from its middle (or the fixed time) on
        if !st.settings.enabled {
            return None;
        }
        let from = Self::dream_minute(&st, &w);
        if !w.contains_from(from, minute) {
            return None;
        }
        // the morning this night is for: in the evening part of a window that runs past
        // midnight it is tomorrow, after midnight (or in a window that does not wrap) today
        let evening = w.end < w.start && minute >= w.start;
        let morning = if evening { today.add(1) } else { today };
        if st.night_done == morning.to_string() || quiet_for < QUIET_SECS {
            return None;
        }
        let g = self.gen(st.current_gen)?;
        match self.next_task(&g, today) {
            Some(task) => {
                st.night = Some(morning.to_string());
                Some(self.post(&mut st, &g, task, false, morning.to_string()))
            }
            None => {
                st.night_done = morning.to_string();
                self.save(&st);
                None
            }
        }
    }

    fn post(&self, st: &mut State, g: &Gen, task: Task, rebuild: bool, report_day: String) -> String {
        st.open = Some(Open {
            key: task.key(),
            gen: g.meta.id,
            posted_at: now(),
            notes: vec![],
            overview: false,
            writes: 0,
            fails: 0,
            touched: 0,
            nudges: 0,
            report_day,
        });
        self.save(st);
        self.task_text(g, &task, &[], rebuild)
    }

    /// `/dream now`: start a night's work at once (outside the window).
    pub fn start_now(&self) -> String {
        let mut st = self.st.lock().unwrap();
        if st.rebuild.is_some() {
            return "a memory rebuild is running; it covers this".into();
        }
        let today = Day::of(now());
        let morning = today.to_string();
        // say so when there is nothing to do (right after a rebuild, every day is reflected)
        if let Some(g) = self.gen(st.current_gen) {
            if self.next_task(&g, today).is_none() {
                return format!(
                    "nothing to dream about: every day up to {} is reflected in generation {} (the next night picks up today)",
                    today.add(-1),
                    g.meta.id
                );
            }
        }
        st.night_done = String::new();
        st.night = Some(morning);
        st.night_now = true;
        self.save(&st);
        "dreaming starts in a moment: the tasks follow as [dream task …] messages".into()
    }
}

// ---------------------------------------------------------------- the live context after a night

/// Stubs for the live context: last month, this month's weeks, this week's
/// days (each its overview), oldest first. Today's reflections stay as they are.
fn spine(g: &Gen, t: u64, max_chars: usize) -> Vec<String> {
    let today = Day::of(t);
    let nodes = g.nodes();
    let over = |level: &str, period: &str| nodes.iter().rev().find(|n| n.overview && n.level == level && n.period == period).cloned();
    let mut out = vec![];
    let month_first = Day { y: today.y, m: today.m, d: 1 };
    let last_month = month_first.add(-1);
    if let Some(n) = over("month", &format!("{:04}-{:02}", last_month.y, last_month.m)) {
        out.push(n);
    }
    let this_monday = today.monday();
    let mut w = month_first.monday();
    while w < this_monday {
        if let Some(n) = over("week", &w.to_string()) {
            out.push(n);
        } else {
            // a week without its own reflection yet: its days
            for i in 0..7 {
                let d = w.add(i);
                if d >= month_first.add(-7) && d < this_monday {
                    if let Some(n) = over("day", &d.to_string()) {
                        out.push(n);
                    }
                }
            }
        }
        w = w.add(7);
    }
    let mut d = this_monday.max(month_first.add(-7));
    while d < today {
        if let Some(n) = over("day", &d.to_string()) {
            out.push(n);
        }
        d = d.add(1);
    }
    let mut stubs: Vec<String> = vec![];
    let mut used = 0usize;
    for n in out {
        let depth = match n.level.as_str() {
            "day" => 1,
            "week" => 2,
            "month" => 3,
            _ => 4,
        };
        let s = format!(
            "reflection {} .. {} [id={} category={} depth={depth} bundle=calendar]\n{} {}: {}",
            crate::clock::rfc3339_unix(n.t_start),
            crate::clock::rfc3339_unix(n.t_end),
            n.id,
            n.category,
            n.level,
            n.period,
            n.text
        );
        if used + s.len() > max_chars {
            break;
        }
        used += s.len();
        stubs.push(s);
    }
    stubs
}

/// 1.7.0 kept a facts ledger (gen-N/facts.json) beside the reflections. What is true now is
/// a block like any other since: each generation's current facts become one `facts` block on
/// its newest reflected day, and the file is kept as facts.json.migrated.
fn migrate_facts(root: &Path) {
    let Ok(rd) = std::fs::read_dir(root) else { return };
    for e in rd.flatten() {
        let dir = e.path();
        let file = dir.join("facts.json");
        let Some(id) = e.file_name().to_str().and_then(|n| n.strip_prefix("gen-")).and_then(|n| n.parse().ok()) else { continue };
        if !file.is_file() {
            continue;
        }
        let Some(g) = Gen::open(root, id) else { continue };
        let facts: Vec<Value> = std::fs::read(&file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let current: Vec<&Value> = facts.iter().filter(|f| f["status"].as_str() == Some("current")).collect();
        let day = g.nodes().into_iter().filter(|n| n.overview && n.level == "day").map(|n| n.period).max();
        if let (false, Some(day)) = (current.is_empty(), day.as_deref().and_then(Day::parse)) {
            let mut by: BTreeMap<String, Vec<String>> = BTreeMap::new();
            for f in &current {
                let since = f["since"].as_str().unwrap_or("");
                let text = f["text"].as_str().unwrap_or("").trim();
                by.entry(f["topic"].as_str().unwrap_or("other").to_string())
                    .or_default()
                    .push(if since.is_empty() { text.to_string() } else { format!("{text} (since {since})") });
            }
            let text: String = by.iter().map(|(t, v)| format!("{t}: {}\n", v.join("; "))).collect();
            let task = Task::Day(day);
            let (t_start, t_end) = task.span();
            let sources = current.iter().flat_map(|f| f["sources"].as_array().cloned().unwrap_or_default()).filter_map(|v| v.as_str().map(String::from)).collect();
            let _ = g.add_node(&Node {
                id: format!("n_facts_{}", day.to_string().replace('-', "")),
                level: "day".into(),
                period: task.period(),
                t_start,
                t_end,
                category: "facts".into(),
                overview: false,
                text: format!("What was true by {day} (carried over from the facts list of bia 1.7.0):\n{text}"),
                sources,
                created_at: crate::clock::rfc3339_now(),
            });
            let mut cats = g.categories();
            cats.entry("facts".into()).or_insert_with(|| Category { meaning: "what is true now: people, accounts, rules, preferences, promises".into(), merged_into: None });
            let _ = g.save_categories(&cats);
        }
        let _ = std::fs::rename(&file, dir.join("facts.json.migrated"));
    }
}

// ---------------------------------------------------------------- tools

/// The dream tools and the calendar side of the memory tools. `None`: not a dream tool.
/// A minimal overview for a period the model left without one, so the night goes on.
fn auto_overview(g: &Gen, task: &Task) -> Result<()> {
    let (t_start, t_end) = task.span();
    g.add_node(&Node {
        id: format!("n_auto_{}", task.key().replace(':', "_")),
        level: task.level().into(),
        period: task.period(),
        t_start,
        t_end,
        category: "summary".into(),
        overview: true,
        text: "(no reflection was written for this period)".into(),
        sources: vec![],
        created_at: crate::clock::rfc3339_now(),
    })
}

pub fn tool(d: &Dream, name: &str, args: &Value) -> Option<Result<String>> {
    if matches!(name, "journal_day" | "reflect_note" | "reflect_write" | "category_merge" | "report_write") {
        d.touch();
    }
    Some(match name {
        "journal_day" => d.t_journal_day(args),
        "reflect_note" => d.t_note(args),
        "reflect_write" => d.t_write(args),
        "category_merge" => d.t_merge(args),
        "report_write" => d.t_report(args),
        "task_done" => d.t_done(args),
        "memory_rebuild" => d.t_rebuild(args),
        "dream" => d.t_dream(args),
        _ => return None,
    })
}

fn s<'a>(a: &'a Value, k: &str) -> &'a str {
    a.get(k).and_then(|v| v.as_str()).unwrap_or("").trim()
}

fn list(a: &Value, k: &str) -> Vec<String> {
    a.get(k)
        .and_then(|v| v.as_array())
        .map(|x| x.iter().filter_map(|v| v.as_str().map(|s| s.trim().to_string())).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default()
}

impl Dream {
    fn open_or_err(&self) -> Result<(Open, Gen, Task)> {
        let st = self.st.lock().unwrap();
        let o = st.open.clone().ok_or_else(|| anyhow!("no dream task is open (this tool works only inside a [dream task …])"))?;
        let g = self.gen(o.gen).ok_or_else(|| anyhow!("memory generation {} is missing", o.gen))?;
        let t = Task::parse(&o.key).ok_or_else(|| anyhow!("bad task {}", o.key))?;
        Ok((o, g, t))
    }

    fn t_journal_day(&self, a: &Value) -> Result<String> {
        let day = match Day::parse(s(a, "date")) {
            Some(d) => d,
            None => match self.open_or_err() {
                Ok((_, _, Task::Day(d))) => d,
                _ => bail!("date is required (YYYY-MM-DD, local)"),
            },
        };
        let (from, to) = Task::Day(day).span();
        let rows = digest(&self.state_dir, from, to);
        if rows.is_empty() {
            return Ok(format!("day {day}: no messages"));
        }
        let pg = pages(&rows, self.page_chars());
        let n = a.get("page").and_then(|v| v.as_u64()).unwrap_or(1).max(1) as usize;
        let Some(&(i, j)) = pg.get(n - 1) else {
            bail!("day {day} has {} page(s)", pg.len());
        };
        let mut out = format!("day {day} ({}), page {n} of {}, messages {} – {}:\n", tz_label(), pg.len(), rows[i].id, rows[j - 1].id);
        for r in &rows[i..j] {
            out.push_str(&r.line);
            out.push('\n');
        }
        if n < pg.len() {
            out.push_str(&format!("… page {} next\n", n + 1));
        }
        Ok(out)
    }


    fn t_note(&self, a: &Value) -> Result<String> {
        let text = redact(s(a, "text"));
        if text.is_empty() {
            bail!("text is required");
        }
        let mut st = self.st.lock().unwrap();
        let o = st.open.as_mut().ok_or_else(|| anyhow!("no dream task is open"))?;
        o.notes.push(one_line(&text, 2000));
        self.save(&st);
        Ok("noted (kept if the context fills up and this task is posted again)".into())
    }

    fn t_write(&self, a: &Value) -> Result<String> {
        let (o, g, task) = self.open_or_err()?;
        if matches!(task, Task::Morning(_)) {
            bail!("the morning task only tells root about the report");
        }
        let text = redact(s(a, "text"));
        if text.is_empty() {
            bail!("text is required");
        }
        let cat = crate::memory::normalize_category(s(a, "category"));
        if cat.is_empty() {
            bail!("category is required (memory_categories lists yours; a new one needs meaning)");
        }
        let mut cats = g.categories();
        let cat = match cats.get(&cat).and_then(|c| c.merged_into.clone()) {
            Some(into) => into,
            None => cat,
        };
        if !cats.contains_key(&cat) {
            let meaning = s(a, "meaning");
            if meaning.is_empty() {
                bail!("{cat} is a new category: pass meaning (one line: what it keeps), or use one of: {}", cats.keys().cloned().collect::<Vec<_>>().join(", "));
            }
            cats.insert(cat.clone(), Category { meaning: meaning.to_string(), merged_into: None });
            g.save_categories(&cats)?;
        }
        let overview = a.get("overview").and_then(|v| v.as_bool()).unwrap_or(false);
        let (t_start, t_end) = task.span();
        let id = {
            let mut st = self.st.lock().unwrap();
            st.seq += 1;
            let id = format!("n_{}_{}", st.seq, cat);
            if let Some(op) = st.open.as_mut() {
                op.writes += 1;
                op.overview |= overview;
            }
            self.save(&st);
            id
        };
        let sources: Vec<String> = list(a, "sources").into_iter().filter(|x| x.starts_with("m_") || x.starts_with("n_") || x.starts_with("r_")).collect();
        g.add_node(&Node {
            id: id.clone(),
            level: task.level().into(),
            period: task.period(),
            t_start,
            t_end,
            category: cat.clone(),
            overview,
            text,
            sources,
            created_at: crate::clock::rfc3339_now(),
        })?;
        let _ = o;
        Ok(format!("saved {id} ({} {}, {cat}{})", task.level(), task.period(), if overview { ", overview" } else { "" }))
    }

    fn t_merge(&self, a: &Value) -> Result<String> {
        let (_, g, _) = self.open_or_err()?;
        let from = crate::memory::normalize_category(s(a, "from"));
        let into = crate::memory::normalize_category(s(a, "into"));
        if from.is_empty() || into.is_empty() || from == into {
            bail!("from and into are two different categories");
        }
        let mut cats = g.categories();
        if !cats.contains_key(&from) {
            bail!("no category {from}");
        }
        let meaning = s(a, "meaning");
        let entry = cats.entry(into.clone()).or_default();
        if !meaning.is_empty() {
            entry.meaning = meaning.to_string();
        }
        if let Some(c) = cats.get_mut(&from) {
            c.merged_into = Some(into.clone());
        }
        g.save_categories(&cats)?;
        Ok(format!("{from} is now kept as {into} (older blocks keep their label and point there)"))
    }

    fn t_report(&self, a: &Value) -> Result<String> {
        let (o, _, task) = self.open_or_err()?;
        if self.st.lock().unwrap().rebuild.as_ref().is_some_and(|r| r.gen == o.gen) {
            bail!("a rebuild writes no morning report");
        }
        if matches!(task, Task::Morning(_)) {
            bail!("the report is already written");
        }
        let section = s(a, "section");
        let text = redact(s(a, "text"));
        if section.is_empty() || text.is_empty() {
            bail!("section and text are required");
        }
        let day = Day::parse(&o.report_day).unwrap_or(Day::of(now()));
        let path = self.report_path(day);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut body = std::fs::read_to_string(&path).unwrap_or_default();
        if body.is_empty() {
            body = format!("# Morning {day}\n");
        }
        let head = format!("\n## {section}\n");
        if let Some(i) = body.find(&head) {
            let after = i + head.len();
            let end = body[after..].find("\n## ").map(|k| after + k).unwrap_or(body.len());
            body.insert_str(end, &format!("\n{text}\n"));
        } else {
            body.push_str(&format!("{head}\n{text}\n"));
        }
        write_atomic(&path, body.as_bytes())?;
        own_like_parent(&path, &self.files_dir);
        Ok(format!("added to {} ({section})", path.display()))
    }

    fn t_done(&self, a: &Value) -> Result<String> {
        let (o, g, task) = self.open_or_err()?;
        if !matches!(task, Task::Morning(_)) && !o.overview {
            let fails = {
                let mut st = self.st.lock().unwrap();
                let f = st.open.as_mut().map(|o| {
                    o.fails += 1;
                    o.fails
                });
                self.save(&st);
                f.unwrap_or(0)
            };
            if fails < 3 {
                bail!("not done: write the period in short first: reflect_write(overview=true, category=…, text=…)");
            }
            // three tries without an overview: a minimal one, so the night goes on
            auto_overview(&g, &task)?;
        }
        let note = one_line(s(a, "note"), 300);
        let mut st = self.st.lock().unwrap();
        st.open = None;
        self.save(&st);
        drop(st);
        *self.finished.lock().unwrap() = Some(Finished {
            key: task.key(),
            note: if note.is_empty() { String::new() } else { note.clone() },
        });
        Ok(format!("{} done", task.key()))
    }

    fn t_dream(&self, a: &Value) -> Result<String> {
        match s(a, "action") {
            "" | "status" => Ok(self.status()),
            "now" => Ok(self.start_now()),
            "on" | "off" => {
                let mut st = self.st.lock().unwrap();
                st.settings.enabled = s(a, "action") == "on";
                self.save(&st);
                Ok(format!("dreaming {}", if st.settings.enabled { "on" } else { "off" }))
            }
            "time" => {
                let v = s(a, "time");
                let mut st = self.st.lock().unwrap();
                st.settings.fixed_time = if v.is_empty() || v == "auto" {
                    None
                } else {
                    parse_hhmm(v).ok_or_else(|| anyhow!("time is HH:MM (local) or auto"))?;
                    Some(v.to_string())
                };
                self.save(&st);
                Ok(match &st.settings.fixed_time {
                    Some(t) => format!("dreaming at {t} local"),
                    None => "dreaming in the middle of root's quietest hours".into(),
                })
            }
            x => bail!("action is status, now, on, off or time (not {x})"),
        }
    }

    pub fn status(&self) -> String {
        let mut st = self.st.lock().unwrap();
        let w = self.window(&mut st);
        let at = Self::dream_minute(&st, &w);
        let mut out = vec![];
        out.push(if !st.settings.enabled {
            "dreaming: off".to_string()
        } else if st.settings.fixed_time.is_some() {
            format!("dreaming: at {} (fixed)", hhmm(at))
        } else if w.fallback {
            format!("dreaming: at {} (not enough of root's activity yet to learn the quiet hours: {} day(s))", hhmm(at), w.days)
        } else {
            format!(
                "dreaming: at {} (root is quiet {}–{} on {} of {} days)",
                hhmm(at),
                hhmm(w.start),
                hhmm(w.end),
                w.quiet_days,
                w.days
            )
        });
        if let Some(o) = &st.open {
            out.push(format!("working on: {} (since {})", o.key, crate::clock::rfc3339_unix(o.posted_at)));
        }
        if let Some(n) = &st.night {
            out.push(format!("night running (morning {n})"));
        }
        if let Some(r) = &st.rebuild {
            let left = self.gen(r.gen).map(|g| self.remaining(&g, Day::of(now()))).unwrap_or(0);
            out.push(format!(
                "memory rebuild: generation {}, {} of {} days done{}",
                r.gen,
                r.total.saturating_sub(left),
                r.total,
                if r.paused { " (paused)" } else { "" }
            ));
        }
        if let Some(g) = self.gen(st.current_gen) {
            let nodes = g.nodes();
            let days = nodes.iter().filter(|n| n.overview && n.level == "day").count();
            out.push(format!("memory: generation {}, {days} day(s) reflected since {}, {} categories", g.meta.id, g.meta.start_day, g.categories().len()));
        }
        out.join("\n")
    }

    // ---- rebuild

    fn first_journal_day(&self) -> Option<Day> {
        let st = crate::memory::Store::open(&self.state_dir);
        st.journal_all().iter().find_map(|m| crate::memory::issued_unix(&m.content)).map(Day::of)
    }

    fn t_rebuild(&self, a: &Value) -> Result<String> {
        match s(a, "action") {
            "" | "estimate" => self.rebuild_estimate(),
            "start" => self.rebuild_start(),
            "status" => Ok(self.status()),
            "pause" | "resume" => {
                let mut st = self.st.lock().unwrap();
                let r = st.rebuild.as_mut().ok_or_else(|| anyhow!("no rebuild is running"))?;
                r.paused = s(a, "action") == "pause";
                let p = r.paused;
                self.save(&st);
                Ok(if p { "rebuild paused" } else { "rebuild goes on" }.into())
            }
            "rollback" => {
                let mut st = self.st.lock().unwrap();
                if st.rebuild.is_some() {
                    bail!("a rebuild is running: pause it, or let it finish first");
                }
                let prev = st.previous_gen.ok_or_else(|| anyhow!("there is no earlier generation to go back to"))?;
                st.previous_gen = Some(st.current_gen);
                st.current_gen = prev;
                st.spine = true;
                self.save(&st);
                Ok(format!("memory is generation {prev} again (the other one is kept: rollback again to return)"))
            }
            x => bail!("action is estimate, start, status, pause, resume or rollback (not {x})"),
        }
    }

    fn rebuild_estimate(&self) -> Result<String> {
        let first = self.first_journal_day().ok_or_else(|| anyhow!("the journal is empty: nothing to rebuild"))?;
        let today = Day::of(now());
        let (mut days, mut chars) = (0usize, 0usize);
        let mut d = first;
        while d < today {
            let (a, b) = Task::Day(d).span();
            let rows = digest(&self.state_dir, a, b);
            if !rows.is_empty() {
                days += 1;
                chars += rows.iter().map(|r| r.line.len() + 1).sum::<usize>();
            }
            d = d.add(1);
        }
        let weeks = ((today.days() - first.days()) / 7).max(0) as usize;
        let months = (((today.y - first.y) * 12 + today.m as i64 - first.m as i64).max(0)) as usize;
        let tasks = days + weeks + months;
        // each tool round sends the live context again; ~6 rounds a task
        let base = self.ctx_used.load(Ordering::SeqCst).max(6000);
        let tokens = chars / 4 + tasks * 6 * base;
        let mut st = self.st.lock().unwrap();
        st.estimate_at = now();
        self.save(&st);
        Ok(format!(
            "rebuild estimate: the journal goes back to {first}; {days} day(s) with messages, about {weeks} week(s) and {months} month(s): about {tasks} tasks. \
About {} input tokens in all ({} of journal text; the rest is your context sent again with each tool round), plus output. \
It runs now, one task after another, between conversations; status, pause and resume work while it runs, and when it reaches yesterday memory switches to it (rollback goes back). \
Tell root these numbers (and what they cost with your brain) and start it only after root says yes: memory_rebuild(action=\"start\").",
            fmt_tokens(tokens),
            fmt_tokens(chars / 4)
        ))
    }

    fn rebuild_start(&self) -> Result<String> {
        let mut st = self.st.lock().unwrap();
        if st.rebuild.is_some() {
            bail!("a rebuild is already running (status, pause, resume)");
        }
        if st.estimate_at == 0 || self.last_root.load(Ordering::SeqCst) <= st.estimate_at || !self.turn_root.load(Ordering::SeqCst) {
            bail!("root has to agree first: show them memory_rebuild(action=\"estimate\") and start it after their yes");
        }
        let first = self.first_journal_day().ok_or_else(|| anyhow!("the journal is empty"))?;
        let id = (1..).find(|i| !self.root.join(format!("gen-{i}")).exists()).unwrap_or(st.current_gen + 1);
        let g = Gen::create(&self.root, id, first)?;
        let total = self.remaining(&g, Day::of(now()));
        st.rebuild = Some(Rebuild { gen: id, paused: false, started_at: crate::clock::rfc3339_now(), total });
        st.night = None;
        self.save(&st);
        Ok(format!("rebuild started: generation {id} from {first}, {total} day(s); tasks follow as [dream task …] messages"))
    }
}

fn fmt_tokens(n: usize) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1e6)
    } else if n >= 1000 {
        format!("{}k", n / 1000)
    } else {
        n.to_string()
    }
}

// ---------------------------------------------------------------- the calendar in memory_list / memory_get

/// Calendar reflections overlapping [from, to] for memory_list (overviews first).
pub fn list_nodes(state_dir: &Path, from: Option<u64>, to: Option<u64>, category: Option<&str>) -> Vec<String> {
    let root = state_dir.join("memory");
    let st: State = std::fs::read(root.join("dream.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let Some(g) = Gen::open(&root, st.current_gen) else { return vec![] };
    let mut nodes: Vec<Node> = g
        .nodes()
        .into_iter()
        .filter(|n| from.is_none_or(|f| n.t_end > f) && to.is_none_or(|t| n.t_start <= t))
        .filter(|n| category.is_none_or(|c| n.category == c))
        .collect();
    nodes.sort_by(|a, b| a.t_start.cmp(&b.t_start).then(b.overview.cmp(&a.overview)));
    nodes
        .into_iter()
        .map(|n| format!("{}  {} {}  {}{}\n  {}", n.id, n.level, n.period, n.category, if n.overview { " (overview)" } else { "" }, one_line(&n.text, 180)))
        .collect()
}

pub fn get_node(state_dir: &Path, id: &str) -> Option<String> {
    let root = state_dir.join("memory");
    let st: State = std::fs::read(root.join("dream.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let mut gens: Vec<u32> = vec![st.current_gen];
    gens.extend(st.previous_gen);
    for gid in gens {
        if let Some(g) = Gen::open(&root, gid) {
            if let Some(n) = g.nodes().into_iter().find(|n| n.id == id) {
                let siblings: Vec<String> = g
                    .nodes()
                    .into_iter()
                    .filter(|x| x.level == n.level && x.period == n.period && x.id != n.id)
                    .map(|x| format!("{} ({})", x.id, x.category))
                    .collect();
                return Some(format!(
                    "{} {} {} ({}{}), generation {gid}\nsources: {}\nsame period: {}\n\n{}",
                    n.id,
                    n.level,
                    n.period,
                    n.category,
                    if n.overview { ", overview" } else { "" },
                    if n.sources.is_empty() { "-".into() } else { n.sources.join(", ") },
                    if siblings.is_empty() { "-".into() } else { siblings.join(", ") },
                    n.text
                ));
            }
        }
    }
    None
}

/// Categories with their meaning and use (memory_categories).
pub fn categories_text(state_dir: &Path) -> String {
    let root = state_dir.join("memory");
    let st: State = std::fs::read(root.join("dream.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let Some(g) = Gen::open(&root, st.current_gen) else { return String::new() };
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for n in g.nodes() {
        *counts.entry(n.category).or_default() += 1;
    }
    g.categories()
        .into_iter()
        .map(|(name, c)| match c.merged_into {
            Some(into) => format!("{name} → kept as {into}"),
            None => format!("{name}  {}  — {}", counts.get(&name).copied().unwrap_or(0), c.meaning),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------- files

fn read_jsonl<T: serde::de::DeserializeOwned>(p: &Path) -> Vec<T> {
    std::fs::read_to_string(p)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

fn append_jsonl<T: Serialize>(p: &Path, v: &T) -> Result<()> {
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(p)?;
    writeln!(f, "{}", serde_json::to_string(v)?)?;
    f.sync_all()?;
    Ok(())
}

fn write_atomic(p: &Path, b: &[u8]) -> Result<()> {
    let tmp = p.with_extension("tmp");
    std::fs::write(&tmp, b)?;
    std::fs::rename(&tmp, p)?;
    Ok(())
}

/// Files bia reads (reports) belong to the account that owns files/.
fn own_like_parent(p: &Path, files: &Path) {
    use std::os::unix::fs::MetadataExt;
    if let Ok(m) = std::fs::metadata(files) {
        let _ = std::os::unix::fs::chown(p, Some(m.uid()), Some(m.gid()));
        if let Some(d) = p.parent() {
            let _ = std::os::unix::fs::chown(d, Some(m.uid()), Some(m.gid()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn days_and_weeks() {
        let d = Day::parse("2026-10-28").unwrap();
        assert_eq!(d.add(4).to_string(), "2026-11-01");
        assert_eq!(d.monday().to_string(), "2026-10-26");
        assert_eq!(Day::parse("2026-11-01").unwrap().monday().to_string(), "2026-10-26");
        assert_eq!(Day::parse("2026-02-30"), None);
        assert_eq!(Task::parse("month:2026-10").unwrap().days().1.to_string(), "2026-11-01");
        assert_eq!(Task::parse("week:2026-10-26").unwrap().key(), "week:2026-10-26");
        assert_eq!(Task::Year(2026).days().1.to_string(), "2027-01-01");
    }

    #[test]
    fn quiet_window_finds_the_night() {
        // root writes 07:00–20:45 (UTC, the default timezone) on 10 days
        let day0 = Day::parse("2026-10-01").unwrap().start();
        let mut times = vec![];
        for d in 0..10 {
            for h in 7..21 {
                times.push(day0 + d * 86400 + h * 3600 + 15 * 60);
                times.push(day0 + d * 86400 + h * 3600 + 45 * 60);
            }
        }
        let w = quiet_window(&times, Day::parse("2026-10-11").unwrap());
        assert!(!w.fallback);
        // the first message is at 07:15 and the last at 20:45: quiet from 21:00 to 07:15
        assert_eq!((hhmm(w.start), hhmm(w.end)), ("21:00".into(), "07:15".into()));
        assert_eq!(hhmm(w.middle()), "02:07");
        assert!(w.contains_from(w.middle(), 3 * 60));
        assert!(!w.contains_from(w.middle(), 12 * 60));
        assert_eq!(w.quiet_days, 10);
        // too little activity: 03:00-ish fallback
        let few = quiet_window(&times[..20], Day::parse("2026-10-11").unwrap());
        assert!(few.fallback);
    }

    #[test]
    fn secrets_are_masked() {
        let r = redact("key sk-or-v1-0123456789abcdef0123456789abcdef and ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ012345 ok");
        assert!(!r.contains("0123456789abcdef0123"), "{r}");
        assert!(r.contains("sk-or-v1-[redacted]") && r.contains("ghp_[redacted]") && r.ends_with(" ok"));
        assert_eq!(redact("the sk-word is fine"), "the sk-word is fine");
        let k = redact("x\n-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----\ny");
        assert!(k.contains("[private key redacted]") && !k.contains("abc"), "{k}");
        assert_eq!(redact("привет"), "привет");
    }

    #[test]
    fn cut_keeps_head_and_tail() {
        let long = "a".repeat(3000) + &"z".repeat(3000);
        let c = cut(&long, 600, "m_9");
        assert!(c.starts_with("aaaa") && c.ends_with("zzzz") && c.contains("m_9"));
        assert!(c.len() < 900);
    }

    #[test]
    fn pages_split_rows() {
        let rows: Vec<Row> = (0..10).map(|i| Row { id: format!("m_{i}"), line: "x".repeat(99) }).collect();
        assert_eq!(pages(&rows, 300), vec![(0, 3), (3, 6), (6, 9), (9, 10)]);
        assert_eq!(pages(&rows, 100_000), vec![(0, 10)]);
    }

    fn tmp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("bia-dream-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(p.join("state")).unwrap();
        p
    }

    /// Journal rows for a day: a user message per hour.
    fn journal(files: &Path, day: &str, n: u64) {
        let mut st = crate::memory::Store::open(&files.join("state"));
        let d0 = Day::parse(day).unwrap().start();
        for h in 0..n {
            let id = st.next_msg_id();
            let issued = crate::clock::rfc3339_unix(d0 + 3600 * (h + 8));
            st.journal(&id, "user", "", &format!("root|console|{issued}|{issued}|b{h}|hello {h}"), "").unwrap();
            let id = st.next_msg_id();
            st.journal(&id, "assistant", "", &format!("answer {h}"), "").unwrap();
            let id = st.next_msg_id();
            st.journal(&id, "tool", "bash", &"out".repeat(1000), "c1").unwrap();
        }
    }

    #[test]
    fn a_whole_rebuild_in_order() {
        let files = tmp("rebuild");
        journal(&files, "2026-10-01", 3);
        journal(&files, "2026-10-03", 2);
        let d = Dream::open(&files);
        let g = Gen::create(&d.root, 2, Day::parse("2026-10-01").unwrap()).unwrap();
        let upto = Day::parse("2026-11-03").unwrap();
        // days with messages first (the empty 2nd is skipped and remembered)
        assert_eq!(d.next_task(&g, upto), Some(Task::Day(Day::parse("2026-10-01").unwrap())));
        let rows = digest(&d.state_dir, Day::parse("2026-10-01").unwrap().start(), Day::parse("2026-10-02").unwrap().start());
        assert_eq!(rows.len(), 9);
        assert!(rows[2].line.contains("memory_messages(ids=[\"m_3\"], full=true)"), "{}", rows[2].line);
        let write = |t: &Task| {
            let (a, b) = t.span();
            g.add_node(&Node { id: format!("n_{}", t.key()), level: t.level().into(), period: t.period(), t_start: a, t_end: b, category: "summary".into(), overview: true, text: t.key(), sources: vec![], created_at: String::new() }).unwrap();
        };
        let mut seen = vec![];
        while let Some(t) = d.next_task(&g, upto) {
            seen.push(t.key());
            write(&t);
            assert!(seen.len() < 20, "{seen:?}");
        }
        assert_eq!(seen, ["day:2026-10-01", "day:2026-10-03", "week:2026-09-28", "month:2026-10"]);
        assert!(g.empty().contains("day:2026-10-02"));
        let _ = std::fs::remove_dir_all(&files);
    }

    #[test]
    fn tools_need_an_open_task_and_an_overview() {
        let files = tmp("tools");
        journal(&files, "2026-10-01", 1);
        let d = Dream::open(&files);
        assert!(d.t_write(&json!({"text": "x", "category": "summary"})).is_err());
        {
            let mut st = d.st.lock().unwrap();
            let g = d.gen(st.current_gen).unwrap();
            let _ = d.post(&mut st, &g, Task::Day(Day::parse("2026-10-01").unwrap()), false, "2026-10-02".into());
        }
        // a new category needs its meaning
        assert!(d.t_write(&json!({"text": "x", "category": "garden"})).is_err());
        assert!(d.t_done(&json!({})).is_err(), "no overview yet");
        d.t_write(&json!({"text": "the day", "category": "summary", "meaning": "the period in short", "overview": true, "sources": ["m_1"]})).unwrap();
        assert!(super::tool(&d, "facts_update", &json!({})).is_none(), "no separate facts store");
        d.t_report(&json!({"section": "For you", "text": "a gym reminder on Mondays"})).unwrap();
        d.t_report(&json!({"section": "For you", "text": "second idea"})).unwrap();
        let rep = std::fs::read_to_string(d.report_path(Day::parse("2026-10-02").unwrap())).unwrap();
        assert_eq!(rep.matches("## For you").count(), 1, "{rep}");
        assert!(rep.contains("gym") && rep.contains("second idea"));
        assert!(d.t_done(&json!({"note": "ok"})).unwrap().contains("day:2026-10-01 done"));
        assert_eq!(d.take_finished().unwrap().key, "day:2026-10-01");
        assert!(d.open_task().is_none());
        let _ = std::fs::remove_dir_all(&files);
    }

    #[test]
    fn dream_now_posts_the_first_task() {
        let files = tmp("now");
        journal(&files, "2026-10-01", 1);
        let d = Dream::open(&files);
        Gen::create(&d.root, 1, Day::parse("2026-10-01").unwrap()).unwrap();
        d.note_human(true); // someone just wrote: "now" does not wait for quiet
        assert!(d.start_now().contains("starts in a moment"));
        let text = d.tick().expect("a task");
        assert!(text.contains("day:2026-10-01"), "{text}");
        assert_eq!(d.open_task().as_deref(), Some("day:2026-10-01"));
        assert!(d.tick().is_none(), "one task at a time");
        let _ = std::fs::remove_dir_all(&files);
    }

    #[test]
    fn a_stalled_task_is_reminded_then_closed() {
        let files = tmp("stall");
        journal(&files, "2026-10-01", 1);
        let d = Dream::open(&files);
        Gen::create(&d.root, 1, Day::parse("2026-10-01").unwrap()).unwrap();
        d.start_now();
        assert!(d.tick().unwrap().contains("[dream task day:2026-10-01]"));
        assert!(d.tick().is_none(), "not stalled yet");
        let age = |d: &Dream| {
            let mut st = d.st.lock().unwrap();
            let o = st.open.as_mut().unwrap();
            o.posted_at -= STALL_SECS + 1;
            o.touched = o.touched.saturating_sub(STALL_SECS + 1);
        };
        for _ in 0..STALL_NUDGES {
            age(&d);
            let r = d.tick().expect("a reminder");
            assert!(r.starts_with("[dream task day:2026-10-01] is still open"), "{r}");
        }
        age(&d);
        assert!(d.tick().is_none());
        assert!(d.open_task().is_none(), "closed");
        assert_eq!(d.take_finished().unwrap().key, "day:2026-10-01");
        let g = d.gen(1).unwrap();
        assert!(g.nodes().iter().any(|n| n.overview && n.period == "2026-10-01"));
        let _ = std::fs::remove_dir_all(&files);
    }

    #[test]
    fn a_facts_ledger_becomes_a_facts_block() {
        let files = tmp("facts");
        let root = files.join("state/memory");
        let g = Gen::create(&root, 1, Day::parse("2026-10-01").unwrap()).unwrap();
        let (a, b) = Task::Day(Day::parse("2026-10-02").unwrap()).span();
        g.add_node(&Node { id: "n_1".into(), level: "day".into(), period: "2026-10-02".into(), t_start: a, t_end: b, category: "summary".into(), overview: true, text: "the day".into(), sources: vec![], created_at: String::new() }).unwrap();
        std::fs::write(g.dir.join("facts.json"), serde_json::to_vec(&json!([
            {"id": "f_1", "topic": "gym", "text": "gym on Tuesday", "since": "2026-10-01", "status": "replaced"},
            {"id": "f_2", "topic": "gym", "text": "gym on Thursday", "since": "2026-10-02", "status": "current", "sources": ["m_7"]},
            {"id": "f_3", "topic": "dentist", "text": "Dr. Ruiz", "since": "", "status": "current"}
        ])).unwrap()).unwrap();
        let _d = Dream::open(&files);
        let block = g.nodes().into_iter().find(|n| n.category == "facts").expect("a facts block");
        assert_eq!(block.period, "2026-10-02");
        assert!(block.text.contains("gym: gym on Thursday (since 2026-10-02)") && block.text.contains("dentist: Dr. Ruiz") && !block.text.contains("Tuesday"), "{}", block.text);
        assert_eq!(block.sources, ["m_7"]);
        assert!(g.categories().contains_key("facts"));
        assert!(!g.dir.join("facts.json").exists() && g.dir.join("facts.json.migrated").exists());
        let _ = std::fs::remove_dir_all(&files);
    }

    #[test]
    fn rebuild_needs_roots_yes_after_the_estimate() {
        let files = tmp("yes");
        journal(&files, "2026-10-01", 1);
        let d = Dream::open(&files);
        assert!(d.rebuild_start().is_err(), "no estimate shown");
        let e = d.rebuild_estimate().unwrap();
        assert!(e.contains("1 day(s) with messages"), "{e}");
        d.note_human(false);
        assert!(d.rebuild_start().is_err(), "not root");
        // root answers after the estimate (a later second)
        d.st.lock().unwrap().estimate_at -= 2;
        d.note_human(true);
        assert!(d.rebuild_start().unwrap().contains("rebuild started"));
        assert!(d.t_rebuild(&json!({"action": "rollback"})).is_err(), "rollback while running");
        let _ = std::fs::remove_dir_all(&files);
    }
}
