//! Calendar for the agent: tasks and reminders persisted under /home/bia/files/state.
//! Due items are pushed as `system|schedule` inbound messages, including catch-up after boot.
use crate::clock;
use crate::console::{Console, Inbound};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const GRACE_SECS: u64 = 90;
const MAX_FIRE_PER_ITEM: usize = 14;
const MIN_EVERY_SECS: u64 = 60;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    /// `task` or `reminder`
    #[serde(default = "default_kind")]
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub body: String,
    /// next due unix (UTC)
    pub at: u64,
    /// `none`, `hourly`, `daily`, `weekly`, `monthly`, `yearly`, `weekdays`, `weekends`, or `30m` / `2h`
    #[serde(default)]
    pub repeat: String,
    #[serde(default = "enabled_true")]
    pub enabled: bool,
    #[serde(default)]
    pub created: u64,
    /// Program to run when due. Empty means deliver the text only.
    #[serde(default)]
    pub command: String,
    /// Unix account the program runs as (`bia`, `root`, …).
    #[serde(default)]
    pub user: String,
    /// When set, a system message is sent only if the program output matches.
    #[serde(default)]
    pub pattern: String,
    /// Five crontab fields in the machine timezone. Overrides `at` / `repeat`.
    #[serde(default)]
    pub cron: String,
}

fn default_kind() -> String {
    "task".into()
}
fn enabled_true() -> bool {
    true
}

#[derive(Serialize, Deserialize, Default)]
struct File {
    #[serde(default)]
    items: Vec<Item>,
}

pub struct Store {
    path: PathBuf,
    items: Vec<Item>,
}

pub struct Fired {
    pub at: u64,
    pub text: String,
}

enum Due {
    Notice(Fired),
    Run(Run),
}

struct Run {
    id: String,
    at: u64,
    missed: bool,
    kind: String,
    title: String,
    body: String,
    command: String,
    user: String,
    pattern: String,
    log_path: PathBuf,
}

#[derive(Clone, Debug)]
enum Repeat {
    None,
    Hourly,
    Daily,
    Weekly,
    Monthly,
    Yearly,
    Weekdays,
    Weekends,
    Every(u64),
}

impl Repeat {
    fn parse(s: &str) -> Result<Repeat> {
        let s = s.trim().to_ascii_lowercase();
        if s.is_empty() || s == "none" || s == "once" || s == "off" {
            return Ok(Repeat::None);
        }
        Ok(match s.as_str() {
            "hourly" | "hour" => Repeat::Hourly,
            "daily" | "day" => Repeat::Daily,
            "weekly" | "week" => Repeat::Weekly,
            "monthly" | "month" => Repeat::Monthly,
            "yearly" | "year" | "annually" | "annual" => Repeat::Yearly,
            "weekdays" | "weekday" | "workdays" => Repeat::Weekdays,
            "weekends" | "weekend" => Repeat::Weekends,
            other => Repeat::Every(parse_every(other)?),
        })
    }

    fn label(&self) -> String {
        match self {
            Repeat::None => "none".into(),
            Repeat::Hourly => "hourly".into(),
            Repeat::Daily => "daily".into(),
            Repeat::Weekly => "weekly".into(),
            Repeat::Monthly => "monthly".into(),
            Repeat::Yearly => "yearly".into(),
            Repeat::Weekdays => "weekdays".into(),
            Repeat::Weekends => "weekends".into(),
            Repeat::Every(s) => format_every(*s),
        }
    }

    fn next(&self, at: u64) -> u64 {
        match self {
            Repeat::None => at,
            Repeat::Hourly => at.saturating_add(3600),
            Repeat::Daily => at.saturating_add(86400),
            Repeat::Weekly => at.saturating_add(86400 * 7),
            Repeat::Monthly => clock::add_months(at, 1),
            Repeat::Yearly => clock::add_years(at, 1),
            Repeat::Weekdays => next_matching(at, true),
            Repeat::Weekends => next_matching(at, false),
            Repeat::Every(s) => at.saturating_add(*s),
        }
    }

    fn snap(&self, mut at: u64) -> u64 {
        match self {
            Repeat::Weekdays => {
                while is_weekend(at) {
                    at = at.saturating_add(86400);
                }
                at
            }
            Repeat::Weekends => {
                while !is_weekend(at) {
                    at = at.saturating_add(86400);
                }
                at
            }
            _ => at,
        }
    }
}

fn is_weekend(at: u64) -> bool {
    matches!(clock::weekday(at), 0 | 6)
}

fn next_matching(from: u64, weekdays: bool) -> u64 {
    let mut t = from.saturating_add(86400);
    for _ in 0..8 {
        if is_weekend(t) != weekdays {
            return t;
        }
        t = t.saturating_add(86400);
    }
    t
}

fn parse_every(s: &str) -> Result<u64> {
    let s = s.trim();
    let i = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    if i == 0 {
        bail!("repeat must be none, hourly, daily, weekly, monthly, yearly, weekdays, weekends, or a duration like 30m / 2h");
    }
    let n: u64 = s[..i]
        .parse()
        .map_err(|_| anyhow::anyhow!("bad repeat duration"))?;
    let unit = s[i..].trim();
    let secs = match unit {
        "" | "s" | "sec" | "secs" | "second" | "seconds" => n,
        "m" | "min" | "mins" | "minute" | "minutes" => n.saturating_mul(60),
        "h" | "hr" | "hrs" | "hour" | "hours" => n.saturating_mul(3600),
        "d" | "day" | "days" => n.saturating_mul(86400),
        "w" | "week" | "weeks" => n.saturating_mul(86400 * 7),
        _ => bail!("unknown repeat {s}"),
    };
    if secs < MIN_EVERY_SECS {
        bail!("repeat duration must be at least 60 seconds");
    }
    Ok(secs)
}

fn format_every(secs: u64) -> String {
    if secs.is_multiple_of(86400) {
        format!("{}d", secs / 86400)
    } else if secs.is_multiple_of(3600) {
        format!("{}h", secs / 3600)
    } else if secs.is_multiple_of(60) {
        format!("{}m", secs / 60)
    } else {
        format!("{secs}s")
    }
}

fn parse_kind(s: &str) -> Result<String> {
    match s.trim().to_ascii_lowercase().as_str() {
        "" | "task" | "todo" | "event" => Ok("task".into()),
        "reminder" | "remind" | "alarm" => Ok("reminder".into()),
        other => bail!("kind must be task or reminder, not {other}"),
    }
}

fn new_id() -> String {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("s{n:x}")
}

impl Store {
    pub fn load(path: &Path) -> Store {
        let items = std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice::<File>(&b).ok())
            .map(|f| f.items)
            .unwrap_or_default();
        Store {
            path: path.to_path_buf(),
            items,
        }
    }

    fn save(&self) -> Result<()> {
        if let Some(p) = self.path.parent() {
            std::fs::create_dir_all(p)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        {
            use std::io::Write;
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(&serde_json::to_vec_pretty(&File {
                items: self.items.clone(),
            })?)?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, &self.path)
            .or_else(|_| std::fs::copy(&tmp, &self.path).map(|_| ()))?;
        let _ = std::fs::remove_file(&tmp);
        unsafe { libc::sync() };
        Ok(())
    }

    pub fn enabled_count(&self) -> usize {
        self.items.iter().filter(|i| i.enabled).count()
    }

    pub fn format_list(&self) -> String {
        if self.items.is_empty() {
            return "(no schedules)".into();
        }
        let mut rows: Vec<&Item> = self.items.iter().collect();
        rows.sort_by_key(|i| i.at);
        rows.iter()
            .map(|i| {
                let pause = if i.enabled { "" } else { "  paused" };
                let body = if i.body.trim().is_empty() {
                    String::new()
                } else {
                    let first = i.body.lines().next().unwrap_or("").trim();
                    format!("\n  {first}")
                };
                format!(
                    "{}  {}  {}  next {}{}  {}{extra}{body}",
                    i.id,
                    i.kind,
                    schedule_label(i),
                    clock::rfc3339_unix(i.at),
                    pause,
                    i.title,
                    extra = action_note(i),
                    body = body,
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn create(&mut self, spec: &NewItem<'_>) -> Result<Item> {
        let title = spec.title.trim();
        if title.is_empty() {
            bail!("title required");
        }
        let user = default_user(spec.command, spec.user);
        check_action(spec.command, &user, spec.pattern)?;
        let now = clock::unix(clock::now());
        let cron = spec.cron.trim();
        let (when, repeat, cron_stored) = if !cron.is_empty() {
            let expr = Cron::parse(cron)?;
            (
                expr.next_after(now, clock::offset_east_now())?,
                String::new(),
                expr.source,
            )
        } else {
            let mut when = clock::parse_when(spec.at, now).ok_or_else(|| {
                anyhow::anyhow!("cannot parse at={}; use RFC 3339, YYYY-MM-DD HH:MM, monday 09:00, in 10m, or cron", spec.at)
            })?;
            let rep = Repeat::parse(spec.repeat)?;
            when = rep.snap(when);
            if when <= now {
                if matches!(rep, Repeat::None) {
                    bail!(
                        "time is in the past; use a future datetime, a relative 'in 10m', or cron"
                    );
                }
                while when <= now {
                    let n = rep.next(when);
                    if n <= when {
                        when = now + 1;
                        break;
                    }
                    when = n;
                }
            }
            (when, rep.label(), String::new())
        };
        let item = Item {
            id: new_id(),
            kind: parse_kind(spec.kind)?,
            title: title.to_string(),
            body: spec.body.to_string(),
            at: when,
            repeat,
            enabled: true,
            created: now,
            command: spec.command.trim().to_string(),
            user,
            pattern: spec.pattern.trim().to_string(),
            cron: cron_stored,
        };
        self.items.push(item.clone());
        self.save()?;
        Ok(item)
    }

    pub fn update(&mut self, id: &str, spec: &UpdateItem<'_>) -> Result<Item> {
        let now = clock::unix(clock::now());
        let Some(item) = self.items.iter_mut().find(|i| i.id == id) else {
            bail!("no schedule {id}");
        };
        if let Some(t) = spec.title {
            let t = t.trim();
            if t.is_empty() {
                bail!("title required");
            }
            item.title = t.to_string();
        }
        if let Some(b) = spec.body {
            item.body = b.to_string();
        }
        if let Some(k) = spec.kind {
            item.kind = parse_kind(k)?;
        }
        if let Some(c) = spec.command {
            item.command = c.trim().to_string();
        }
        if let Some(u) = spec.user {
            item.user = u.trim().to_string();
        }
        if let Some(p) = spec.pattern {
            item.pattern = p.trim().to_string();
        }
        if !item.command.is_empty() && item.user.is_empty() {
            item.user = "bia".into();
        }
        check_action(&item.command, &item.user, &item.pattern)?;
        if let Some(c) = spec.cron {
            let c = c.trim();
            if c.is_empty() {
                item.cron.clear();
            } else {
                let expr = Cron::parse(c)?;
                item.at = expr.next_after(now, clock::offset_east_now())?;
                item.cron = expr.source;
                item.repeat.clear();
            }
        }
        if item.cron.is_empty() {
            if let Some(r) = spec.repeat {
                item.repeat = Repeat::parse(r)?.label();
            }
            if let Some(a) = spec.at {
                let rep = Repeat::parse(&item.repeat)?;
                let mut when = clock::parse_when(a, now)
                    .ok_or_else(|| anyhow::anyhow!("cannot parse at={a:?}"))?;
                when = rep.snap(when);
                if when <= now && matches!(rep, Repeat::None) {
                    bail!("time is in the past");
                }
                while when <= now && !matches!(rep, Repeat::None) {
                    let n = rep.next(when);
                    if n <= when {
                        when = now + 1;
                        break;
                    }
                    when = n;
                }
                item.at = when;
            }
        }
        if let Some(e) = spec.enabled {
            item.enabled = e;
        }
        let out = item.clone();
        self.save()?;
        Ok(out)
    }

    pub fn delete(&mut self, id: &str) -> Result<bool> {
        let n = self.items.len();
        self.items.retain(|i| i.id != id);
        if self.items.len() == n {
            return Ok(false);
        }
        self.save()?;
        Ok(true)
    }

    /// Fire every due occurrence. One-shots are removed; repeats skip forward past now.
    /// A command runs once per due item (missed slots are not replayed). After a long
    /// downtime a text item delivers at most `MAX_FIRE_PER_ITEM` occurrences, plus a summary.
    fn consume_due(&mut self, now: u64, busy: &HashSet<String>) -> Vec<Due> {
        let mut due = Vec::new();
        let mut keep = Vec::new();
        let mut changed = false;
        let east = clock::offset_east_now();
        for mut item in std::mem::take(&mut self.items) {
            if !item.enabled || item.at > now {
                keep.push(item);
                continue;
            }
            if !item.command.is_empty() && busy.contains(&item.id) {
                keep.push(item);
                continue;
            }
            changed = true;
            let rep = Repeat::parse(&item.repeat).unwrap_or(Repeat::None);
            if !item.command.is_empty() {
                let missed = now.saturating_sub(item.at) > GRACE_SECS;
                let run = Run {
                    id: item.id.clone(),
                    at: item.at,
                    missed,
                    kind: item.kind.clone(),
                    title: item.title.clone(),
                    body: item.body.clone(),
                    command: item.command.clone(),
                    user: item.user.clone(),
                    pattern: item.pattern.clone(),
                    log_path: self.log_path(&item.id, item.at),
                };
                if item.cron.is_empty() && matches!(rep, Repeat::None) {
                    due.push(Due::Run(run));
                    continue;
                }
                item.at = if !item.cron.is_empty() {
                    Cron::parse(&item.cron)
                        .and_then(|c| c.next_after(now, east))
                        .unwrap_or(now.saturating_add(3600))
                } else {
                    next_future(item.at, now, &rep)
                };
                due.push(Due::Run(run));
                keep.push(item);
                continue;
            }
            if matches!(rep, Repeat::None) {
                due.push(Due::Notice(fire_msg(
                    &item,
                    item.at,
                    now.saturating_sub(item.at) > GRACE_SECS,
                )));
                continue;
            }
            let mut occ = item.at;
            let mut n = 0usize;
            let mut skipped = 0u64;
            let mut guard = 0u32;
            while occ <= now {
                if n < MAX_FIRE_PER_ITEM {
                    due.push(Due::Notice(fire_msg(
                        &item,
                        occ,
                        now.saturating_sub(occ) > GRACE_SECS,
                    )));
                    n += 1;
                } else {
                    skipped += 1;
                }
                let next = rep.next(occ);
                if next <= occ {
                    occ = now.saturating_add(1);
                    break;
                }
                occ = next;
                guard += 1;
                if guard > 50_000 {
                    let (jump, extra) = jump_future(occ, now, &rep);
                    skipped += extra;
                    occ = jump;
                    break;
                }
            }
            if skipped > 0 {
                due.push(Due::Notice(Fired {
                    at: now,
                    text: format!(
                        "Missed {} additional {} occurrence(s) of \"{}\" during downtime; next is {}.",
                        skipped,
                        item.repeat,
                        item.title,
                        clock::rfc3339_unix(occ)
                    ),
                }));
            }
            item.at = occ;
            keep.push(item);
        }
        self.items = keep;
        if changed {
            let _ = self.save();
        }
        due
    }

    fn log_path(&self, id: &str, at: u64) -> PathBuf {
        let dir = self
            .path
            .parent()
            .unwrap_or(Path::new("."))
            .join("schedule-logs");
        dir.join(format!("{id}-{at}.log"))
    }
}

fn jump_future(mut occ: u64, now: u64, rep: &Repeat) -> (u64, u64) {
    let step = match rep {
        Repeat::Hourly => 3600,
        Repeat::Daily => 86400,
        Repeat::Weekly => 86400 * 7,
        Repeat::Every(s) => *s,
        _ => {
            let mut extra = 0u64;
            while occ <= now {
                let n = rep.next(occ);
                if n <= occ {
                    return (now.saturating_add(1), extra);
                }
                occ = n;
                extra += 1;
                if extra > 20_000 {
                    return (now.saturating_add(1), extra);
                }
            }
            return (occ, extra);
        }
    };
    if step == 0 || occ > now {
        return (occ, 0);
    }
    let extra = (now - occ) / step + 1;
    (occ.saturating_add(extra * step), extra)
}

fn fire_msg(item: &Item, at: u64, missed: bool) -> Fired {
    let label = if item.kind == "reminder" {
        "Reminder"
    } else {
        "Scheduled task"
    };
    let when = if missed {
        format!(
            "{} (missed; due while the system was off or busy)",
            clock::rfc3339_unix(at)
        )
    } else {
        clock::rfc3339_unix(at)
    };
    let mut text = format!("{label}: {}\nWhen: {when}", item.title);
    if !item.body.trim().is_empty() {
        text.push('\n');
        text.push_str(item.body.trim_end());
    }
    Fired { at, text }
}

fn push_fired(con: &Console, ev: Fired) {
    let issued = UNIX_EPOCH + Duration::from_secs(ev.at);
    con.push_inbound(
        Inbound {
            identity: "system".into(),
            channel: "schedule".into(),
            issued,
            id: String::new(),
            text: ev.text,
        }
        .ensure_id(),
    );
}

/// Catch up immediately, then poll about once a second.
/// Command tasks run outside the store lock. A run still in progress is not started again.
pub fn start(store: Arc<Mutex<Store>>, con: Arc<Console>) {
    let busy: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
    dispatch(&store, &con, &busy);
    let _ = std::thread::Builder::new()
        .name("schedules".into())
        .spawn(move || loop {
            dispatch(&store, &con, &busy);
            std::thread::sleep(Duration::from_secs(1));
        });
}

fn dispatch(store: &Arc<Mutex<Store>>, con: &Arc<Console>, busy: &Arc<Mutex<HashSet<String>>>) {
    let due = {
        let running = busy.lock().unwrap_or_else(|e| e.into_inner());
        let mut st = store.lock().unwrap_or_else(|e| e.into_inner());
        st.consume_due(clock::unix(clock::now()), &running)
    };
    for item in due {
        match item {
            Due::Notice(ev) => push_fired(con, ev),
            Due::Run(run) => {
                let id = run.id.clone();
                busy.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(id.clone());
                let busy_thread = busy.clone();
                let con = con.clone();
                let spawned = std::thread::Builder::new()
                    .name(format!("sched-{id}"))
                    .spawn(move || {
                        if let Some(ev) = execute(&run) {
                            push_fired(&con, ev);
                        }
                        busy_thread
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .remove(&run.id);
                    });
                if spawned.is_err() {
                    busy.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
                }
            }
        }
    }
}

pub fn describe(item: &Item) -> String {
    format!(
        "ok: {}  {}  {}  next {}  {}{}",
        item.id,
        item.kind,
        schedule_label(item),
        clock::rfc3339_unix(item.at),
        item.title,
        action_note(item),
    )
}

pub struct NewItem<'a> {
    pub title: &'a str,
    pub body: &'a str,
    pub at: &'a str,
    pub kind: &'a str,
    pub repeat: &'a str,
    pub cron: &'a str,
    pub command: &'a str,
    pub user: &'a str,
    pub pattern: &'a str,
}

pub struct UpdateItem<'a> {
    pub title: Option<&'a str>,
    pub body: Option<&'a str>,
    pub at: Option<&'a str>,
    pub kind: Option<&'a str>,
    pub repeat: Option<&'a str>,
    pub cron: Option<&'a str>,
    pub command: Option<&'a str>,
    pub user: Option<&'a str>,
    pub pattern: Option<&'a str>,
    pub enabled: Option<bool>,
}

fn schedule_label(item: &Item) -> String {
    if !item.cron.is_empty() {
        item.cron.clone()
    } else if item.repeat.is_empty() {
        "none".into()
    } else {
        item.repeat.clone()
    }
}

fn action_note(item: &Item) -> String {
    if item.command.is_empty() {
        return String::new();
    }
    let cmd = item.command.lines().next().unwrap_or("").trim();
    let cmd = if cmd.chars().count() > 80 {
        format!("{}…", cmd.chars().take(80).collect::<String>())
    } else {
        cmd.to_string()
    };
    let silent = if item.pattern.is_empty() {
        String::new()
    } else {
        format!("  silent unless /{}/", item.pattern)
    };
    format!("\n  as {}: {cmd}{silent}", item.user)
}

fn next_future(mut occ: u64, now: u64, rep: &Repeat) -> u64 {
    let mut guard = 0u32;
    while occ <= now {
        let n = rep.next(occ);
        if n <= occ {
            return now.saturating_add(1);
        }
        occ = n;
        guard += 1;
        if guard > 50_000 {
            return jump_future(occ, now, rep).0;
        }
    }
    occ
}

fn default_user(command: &str, user: &str) -> String {
    let user = user.trim();
    if !command.trim().is_empty() && user.is_empty() {
        "bia".into()
    } else {
        user.to_string()
    }
}

fn check_action(command: &str, user: &str, pattern: &str) -> Result<()> {
    let command = command.trim();
    let user = user.trim();
    let pattern = pattern.trim();
    if command.is_empty() {
        if !user.is_empty() || !pattern.is_empty() {
            bail!("user and match need a command");
        }
        return Ok(());
    }
    if command.len() > 16_000 {
        bail!("command is too long");
    }
    account_ok(user)?;
    if !pattern.is_empty() {
        if pattern.len() > 2_000 {
            bail!("match expression is too long");
        }
        regex::Regex::new(pattern).map_err(|e| anyhow::anyhow!("bad match expression: {e}"))?;
    }
    Ok(())
}

fn account_ok(name: &str) -> Result<()> {
    if name.len() > 32
        || name.starts_with('-')
        || !name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        bail!("user must be a unix account name");
    }
    #[cfg(target_os = "linux")]
    {
        let c = std::ffi::CString::new(name).map_err(|_| anyhow::anyhow!("bad user name"))?;
        if unsafe { libc::getpwnam(c.as_ptr()) }.is_null() {
            bail!("no such user {name}");
        }
    }
    Ok(())
}

fn output_notifies(pattern: &str, output: &str) -> Result<bool> {
    if pattern.is_empty() {
        return Ok(true);
    }
    let re =
        regex::Regex::new(pattern).map_err(|e| anyhow::anyhow!("bad match expression: {e}"))?;
    Ok(re.is_match(output))
}

const RUN_TIMEOUT: Duration = Duration::from_secs(300);
const MATCH_CAP: usize = 8 * 1024 * 1024;

fn execute(run: &Run) -> Option<Fired> {
    let fail = |why: String| -> Option<Fired> {
        Some(Fired {
            at: run.at,
            text: format!("{}\n{why}", task_text(run)),
        })
    };
    if let Some(parent) = run.log_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return fail(format!("could not create the log directory: {e}"));
        }
    }
    let mut log = match std::fs::File::create(&run.log_path) {
        Ok(f) => f,
        Err(e) => return fail(format!("could not create the log: {e}")),
    };
    let stdout = match log.try_clone() {
        Ok(f) => Stdio::from(f),
        Err(e) => return fail(format!("could not open the log: {e}")),
    };
    let stderr = match log.try_clone() {
        Ok(f) => Stdio::from(f),
        Err(e) => return fail(format!("could not open the log: {e}")),
    };
    let mut child = match spawn_as(&run.user, &run.command, stdout, stderr) {
        Ok(c) => c,
        Err(e) => return fail(format!("could not run as {}: {e}", run.user)),
    };
    let started = Instant::now();
    let waited = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Ok(st),
            Ok(None) if started.elapsed() >= RUN_TIMEOUT => {
                kill_group(child.id());
                let _ = child.wait();
                break Err(true);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(200)),
            Err(_) => break Err(false),
        }
    };
    let (code, timed_out) = match waited {
        Ok(st) => (st.code(), false),
        Err(timed_out) => (None, timed_out),
    };
    let output = read_capped(&run.log_path, MATCH_CAP);
    let _ = writeln!(
        log,
        "\nexit {}",
        code.map(|c| c.to_string())
            .unwrap_or_else(|| "signal".into())
    );
    let _ = log.flush();
    let log_line = format!("Log: {}", run.log_path.display());
    if timed_out {
        return Some(Fired {
            at: run.at,
            text: format!(
                "{}\n{log_line}\nThe program was stopped after 300s.",
                task_text(run)
            ),
        });
    }
    match output_notifies(&run.pattern, &output) {
        Ok(true) => Some(Fired {
            at: run.at,
            text: format!("{}\n{log_line}", task_text(run)),
        }),
        Ok(false) => None,
        Err(e) => Some(Fired {
            at: run.at,
            text: format!("{}\n{log_line}\n{e}", task_text(run)),
        }),
    }
}

fn task_text(run: &Run) -> String {
    let item = Item {
        id: run.id.clone(),
        kind: run.kind.clone(),
        title: run.title.clone(),
        body: run.body.clone(),
        at: run.at,
        repeat: String::new(),
        enabled: true,
        created: 0,
        command: String::new(),
        user: String::new(),
        pattern: String::new(),
        cron: String::new(),
    };
    fire_msg(&item, run.at, run.missed).text
}

fn read_capped(path: &Path, cap: usize) -> String {
    use std::io::Read;
    let Ok(f) = std::fs::File::open(path) else {
        return String::new();
    };
    let mut buf = Vec::new();
    let _ = f.take(cap as u64).read_to_end(&mut buf);
    String::from_utf8_lossy(&buf).into_owned()
}

fn kill_group(pid: u32) {
    unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
}

fn spawn_as(
    user: &str,
    command: &str,
    stdout: Stdio,
    stderr: Stdio,
) -> std::io::Result<std::process::Child> {
    let runuser = ["/usr/sbin/runuser", "/sbin/runuser", "/bin/runuser"]
        .into_iter()
        .find(|p| Path::new(p).is_file());
    let mut cmd = if let Some(bin) = runuser {
        let mut c = Command::new(bin);
        c.args(["-u", user, "--", "/bin/sh", "-c", command]);
        c
    } else {
        let mut c = Command::new("su");
        c.args(["-s", "/bin/sh", user, "-c", command]);
        c
    };
    cmd.stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr)
        .process_group(0);
    cmd.spawn()
}

struct Cron {
    source: String,
    minute: Field,
    hour: Field,
    dom: Field,
    month: Field,
    dow: Field,
}

#[derive(Clone)]
struct Field {
    any: bool,
    vals: Vec<u32>,
}

impl Field {
    fn hit(&self, n: u32) -> bool {
        self.any || self.vals.binary_search(&n).is_ok()
    }

    fn hit_dow(&self, n: u32) -> bool {
        self.hit(n) || (n == 0 && self.hit(7))
    }
}

impl Cron {
    fn parse(s: &str) -> Result<Cron> {
        let fields: Vec<&str> = s.split_whitespace().collect();
        if fields.len() != 5 {
            bail!("cron needs 5 fields: minute hour day-of-month month day-of-week");
        }
        let minute = parse_field(fields[0], 0, 59, &[])?;
        let hour = parse_field(fields[1], 0, 23, &[])?;
        let dom = parse_field(fields[2], 1, 31, &[])?;
        let month = parse_field(fields[3], 1, 12, MONTHS)?;
        let dow = parse_field(fields[4], 0, 7, DOW)?;
        Ok(Cron {
            source: fields.join(" "),
            minute,
            hour,
            dom,
            month,
            dow,
        })
    }

    fn matches(&self, unix: u64, east: i32) -> bool {
        let (minute, hour, day, month, dow) = clock::shifted_fields(unix, east);
        if !self.minute.hit(minute) || !self.hour.hit(hour) || !self.month.hit(month) {
            return false;
        }
        let dom_ok = self.dom.hit(day);
        let dow_ok = self.dow.hit_dow(dow);
        if !self.dom.any && !self.dow.any {
            dom_ok || dow_ok
        } else {
            dom_ok && dow_ok
        }
    }

    fn next_after(&self, after: u64, east: i32) -> Result<u64> {
        let local = (after as i64 + east as i64).max(0) as u64;
        let mut utc = ((local / 60 + 1) * 60) as i64 - east as i64;
        if utc < 0 {
            utc = 0;
        }
        let mut utc = utc as u64;
        for _ in 0..(366 * 24 * 60) {
            if self.matches(utc, east) {
                return Ok(utc);
            }
            utc = utc.saturating_add(60);
        }
        bail!("cron {} does not match in the next year", self.source);
    }
}

const MONTHS: &[(&str, u32)] = &[
    ("jan", 1),
    ("feb", 2),
    ("mar", 3),
    ("apr", 4),
    ("may", 5),
    ("jun", 6),
    ("jul", 7),
    ("aug", 8),
    ("sep", 9),
    ("oct", 10),
    ("nov", 11),
    ("dec", 12),
];
const DOW: &[(&str, u32)] = &[
    ("sun", 0),
    ("mon", 1),
    ("tue", 2),
    ("wed", 3),
    ("thu", 4),
    ("fri", 5),
    ("sat", 6),
];

fn parse_field(spec: &str, min: u32, max: u32, names: &[(&str, u32)]) -> Result<Field> {
    if spec == "*" {
        return Ok(Field {
            any: true,
            vals: Vec::new(),
        });
    }
    let mut vals = Vec::new();
    for part in spec.split(',') {
        if part.is_empty() {
            bail!("bad cron field {spec}");
        }
        let (range, step) = match part.split_once('/') {
            Some((r, s)) => {
                let step: u32 = s
                    .parse()
                    .map_err(|_| anyhow::anyhow!("bad cron step {s}"))?;
                if step == 0 {
                    bail!("cron step must be at least 1");
                }
                (r, step)
            }
            None => (part, 1u32),
        };
        let (start, end) = if range == "*" {
            (min, max)
        } else if let Some((a, b)) = range.split_once('-') {
            (field_num(a, names)?, field_num(b, names)?)
        } else {
            let n = field_num(range, names)?;
            (n, n)
        };
        if start < min || end > max || start > end {
            bail!("cron value {range} is outside {min}-{max}");
        }
        let mut n = start;
        while n <= end {
            vals.push(n);
            n = n.saturating_add(step);
            if step == 0 {
                break;
            }
        }
    }
    vals.sort_unstable();
    vals.dedup();
    if vals.is_empty() {
        bail!("cron field {spec} matches nothing");
    }
    Ok(Field { any: false, vals })
}

fn field_num(tok: &str, names: &[(&str, u32)]) -> Result<u32> {
    if let Ok(n) = tok.parse::<u32>() {
        return Ok(n);
    }
    let t = tok.to_ascii_lowercase();
    names
        .iter()
        .find(|(n, _)| *n == t)
        .map(|(_, v)| *v)
        .ok_or_else(|| anyhow::anyhow!("bad cron value {tok}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cron_next_matches() {
        let daily = Cron::parse("0 0 * * *").unwrap();
        assert_eq!(daily.next_after(0, 0).unwrap(), 86400);
        let quarter = Cron::parse("*/15 * * * *").unwrap();
        assert_eq!(quarter.next_after(0, 0).unwrap(), 900);
        let monday = Cron::parse("0 9 * * 1").unwrap();
        assert_eq!(monday.next_after(0, 0).unwrap(), 4 * 86400 + 9 * 3600);
        let either = Cron::parse("0 0 1 * 1").unwrap();
        assert_eq!(either.next_after(0, 0).unwrap(), 4 * 86400);
        assert!(Cron::parse("* * *").is_err());
        assert!(Cron::parse("60 * * * *").is_err());
    }

    #[test]
    fn silent_unless_output_matches() {
        assert!(output_notifies("", "nothing to report").unwrap());
        assert!(output_notifies("new mail", "2 new mail").unwrap());
        assert!(!output_notifies("new mail", "no new emails").unwrap());
        assert!(output_notifies("(?i)inbox", "Inbox: 1").unwrap());
    }

    #[test]
    fn create_cron_command() {
        let dir = std::env::temp_dir().join(format!("bia-sched-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut st = Store::load(&dir.join("schedules.json"));
        let item = st
            .create(&NewItem {
                title: "mail",
                body: "check the inbox",
                at: "",
                kind: "task",
                repeat: "",
                cron: "0 9 * * 1-5",
                command: "/usr/local/bin/mail-check",
                user: "root",
                pattern: "new mail",
            })
            .unwrap();
        assert_eq!(item.cron, "0 9 * * 1-5");
        assert_eq!(item.user, "root");
        assert!(item.at > 0);
        let listed = st.format_list();
        assert!(listed.contains("silent unless /new mail/"));
        assert!(listed.contains("as root"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
