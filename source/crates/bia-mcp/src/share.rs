//! Locations and contacts shared in chats (Telegram, WhatsApp): the text the model
//! reads for them, and vCards in both directions.

/// `[location] Grand Hotel, Main St 1 — 52.520008, 13.404954 (±20 m) https://…`
/// `live_secs`: a live location shared for that long. `note`: caption/comment.
pub fn location_text(
    lat: f64,
    lon: f64,
    name: &str,
    address: &str,
    accuracy_m: Option<u32>,
    live_secs: Option<u32>,
    note: &str,
) -> String {
    let mut s = String::from(if live_secs.is_some() { "[live location]" } else { "[location]" });
    let place: Vec<&str> = [name.trim(), address.trim()].into_iter().filter(|x| !x.is_empty()).collect();
    if !place.is_empty() {
        s.push(' ');
        s.push_str(&place.join(", "));
        s.push_str(" —");
    }
    s.push_str(&format!(" {lat:.6}, {lon:.6}"));
    if let Some(a) = accuracy_m.filter(|a| *a > 0) {
        s.push_str(&format!(" (±{a} m)"));
    }
    if let Some(t) = live_secs {
        s.push_str(&match t {
            0 => " (sharing live)".to_string(),
            // Telegram's "until I stop" period
            0x7fff_ffff => " (sharing live until they stop)".to_string(),
            t if t >= 3600 => format!(" (sharing live for {} h)", t / 3600),
            t => format!(" (sharing live for {} min)", t.div_ceil(60)),
        });
    }
    s.push(' ');
    s.push_str(&maps_link(lat, lon));
    let note = note.trim();
    if !note.is_empty() {
        s.push('\n');
        s.push_str(note);
    }
    s
}

pub fn maps_link(lat: f64, lon: f64) -> String {
    format!("https://maps.google.com/?q={lat:.6},{lon:.6}")
}

/// Check coordinates passed to a send tool.
pub fn check_coords(lat: f64, lon: f64) -> Result<(), String> {
    if !lat.is_finite() || !lon.is_finite() || !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return Err(format!("latitude must be -90…90 and longitude -180…180 (got {lat}, {lon})"));
    }
    Ok(())
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Card {
    pub name: String,
    pub phones: Vec<String>,
    pub emails: Vec<String>,
    pub org: String,
}

/// `[contact] Jane Doe, +49 170 1234567, jane@example.com (ACME)` plus `extra`
/// (for example the person's Telegram account).
pub fn contact_text(card: &Card, extra: &str) -> String {
    let mut parts = Vec::new();
    if !card.name.trim().is_empty() {
        parts.push(card.name.trim().to_string());
    }
    parts.extend(card.phones.iter().cloned());
    parts.extend(card.emails.iter().cloned());
    let mut s = format!("[contact] {}", if parts.is_empty() { "(empty)".into() } else { parts.join(", ") });
    if !card.org.trim().is_empty() {
        s.push_str(&format!(" ({})", card.org.trim()));
    }
    if !extra.trim().is_empty() {
        s.push_str(&format!("; {}", extra.trim()));
    }
    s
}

/// The fields we care about from a vCard (2.1/3.0/4.0): FN (else N), TEL, EMAIL, ORG.
pub fn parse_vcard(vcard: &str) -> Card {
    // Unfold continuation lines (RFC 6350 3.2).
    let mut lines: Vec<String> = Vec::new();
    for raw in vcard.split('\n') {
        let raw = raw.trim_end_matches('\r');
        if (raw.starts_with(' ') || raw.starts_with('\t')) && !lines.is_empty() {
            lines.last_mut().unwrap().push_str(&raw[1..]);
        } else {
            lines.push(raw.to_string());
        }
    }
    let mut c = Card::default();
    let mut n_name = String::new();
    for l in &lines {
        let Some((head, value)) = l.split_once(':') else { continue };
        // "item1.TEL;type=CELL;waid=491701234567" → "TEL"
        let prop = head.split(';').next().unwrap_or("").rsplit('.').next().unwrap_or("").to_ascii_uppercase();
        let value = unescape(value.trim());
        if value.is_empty() {
            continue;
        }
        match prop.as_str() {
            "FN" => c.name = value,
            "N" => {
                // Family;Given;Additional;Prefix;Suffix
                let f: Vec<&str> = value.split(';').collect();
                let given = f.get(1).copied().unwrap_or("");
                let family = f.first().copied().unwrap_or("");
                n_name = [given, family].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" ");
            }
            "TEL" => {
                let v = value.trim_start_matches("tel:").to_string();
                if !c.phones.contains(&v) {
                    c.phones.push(v);
                }
            }
            "EMAIL" => {
                if !c.emails.contains(&value) {
                    c.emails.push(value);
                }
            }
            "ORG" => c.org = value.split(';').filter(|x| !x.is_empty()).collect::<Vec<_>>().join(", "),
            _ => {}
        }
    }
    if c.name.is_empty() {
        c.name = n_name;
    }
    c
}

/// A vCard 3.0 for sending. WhatsApp shows "Message" / "Add contact" when the phone
/// carries `waid` (its number in digits).
pub fn build_vcard(name: &str, phone: &str, email: &str, whatsapp_id: bool) -> String {
    let name = escape(name.trim());
    let mut v = format!("BEGIN:VCARD\r\nVERSION:3.0\r\nFN:{name}\r\nN:;{name};;;\r\n");
    let phone = phone.trim();
    if !phone.is_empty() {
        let digits: String = phone.chars().filter(|c| c.is_ascii_digit()).collect();
        if whatsapp_id && !digits.is_empty() {
            v.push_str(&format!("TEL;type=CELL;type=VOICE;waid={digits}:{phone}\r\n"));
        } else {
            v.push_str(&format!("TEL;type=CELL:{phone}\r\n"));
        }
    }
    if !email.trim().is_empty() {
        v.push_str(&format!("EMAIL:{}\r\n", escape(email.trim())));
    }
    v.push_str("END:VCARD\r\n");
    v
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace(',', "\\,").replace(';', "\\;").replace('\n', "\\n")
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n') | Some('N') => out.push(' '),
                Some(x) => out.push(x),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

// ---- live locations ------------------------------------------------------------

/// A live location someone shares in a chat.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Live {
    /// Who shares it (the name the model knows them by).
    pub who: String,
    pub lat: f64,
    pub lon: f64,
    #[serde(default)]
    pub accuracy_m: Option<u32>,
    /// Unix seconds.
    pub started: u64,
    pub updated: u64,
    /// When sharing ends by itself (None: until they stop / unknown).
    #[serde(default)]
    pub until: Option<u64>,
    /// Ended (stopped, expired or gone quiet), with when.
    #[serde(default)]
    pub ended: Option<u64>,
    /// Updates received after the first.
    #[serde(default)]
    pub updates: u64,
}

/// A live share that sends nothing for this long counts as ended (WhatsApp does not
/// say how long the sender chose; a phone that stops sharing just goes quiet).
pub const LIVE_QUIET_SECS: u64 = 20 * 60;

/// Live locations per chat. The first point of a share reaches the model; updates only
/// refresh this table; when a share ends the model hears it once (from `sweep`).
/// Kept in a JSON file so a restart does not lose it.
pub struct LiveTracker {
    path: std::path::PathBuf,
    map: std::sync::Mutex<(std::collections::BTreeMap<String, Live>, u64)>,
}

/// What `seen` did with a point.
#[derive(Debug, PartialEq)]
pub enum LiveEvent {
    /// A new share: tell the model.
    Started,
    /// An update of a running share: say nothing.
    Updated,
    /// The share was stopped by the sender: tell the model it ended.
    Stopped,
}

impl LiveTracker {
    pub fn open(path: impl Into<std::path::PathBuf>) -> Self {
        let path = path.into();
        let map = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        LiveTracker { path, map: std::sync::Mutex::new((map, 0)) }
    }

    /// A live point from `chat`. `until`: end of the share when known. `stopped`: the
    /// sender ended it with this point.
    #[allow(clippy::too_many_arguments)]
    pub fn seen(&self, chat: &str, who: &str, lat: f64, lon: f64, accuracy_m: Option<u32>, until: Option<u64>, stopped: bool, now: u64) -> LiveEvent {
        let mut g = self.map.lock().unwrap_or_else(|e| e.into_inner());
        let running = g.0.get(chat).is_some_and(|l| l.ended.is_none());
        let ev = if running {
            let l = g.0.get_mut(chat).unwrap();
            l.lat = lat;
            l.lon = lon;
            l.accuracy_m = accuracy_m.or(l.accuracy_m);
            l.updated = now;
            l.updates += 1;
            if until.is_some() {
                l.until = until;
            }
            if stopped {
                l.ended = Some(now);
                LiveEvent::Stopped
            } else {
                LiveEvent::Updated
            }
        } else {
            g.0.insert(
                chat.to_string(),
                Live { who: who.to_string(), lat, lon, accuracy_m, started: now, updated: now, until, ended: stopped.then_some(now), updates: 0 },
            );
            LiveEvent::Started
        };
        // Updates may come every few seconds: write at most every 30 s (and on start/stop).
        let flush = ev != LiveEvent::Updated || now >= g.1 + 30;
        if flush {
            g.1 = now;
            Self::save(&self.path, &g.0);
        }
        ev
    }

    /// Shares that ended by time or silence since the last sweep (marked ended, returned
    /// once). Call every few seconds; `now` in Unix seconds.
    pub fn sweep(&self, now: u64) -> Vec<(String, Live)> {
        let mut g = self.map.lock().unwrap_or_else(|e| e.into_inner());
        let mut out = Vec::new();
        for (k, l) in g.0.iter_mut() {
            if l.ended.is_some() {
                continue;
            }
            let expired = l.until.is_some_and(|u| now >= u);
            let quiet = now >= l.updated + LIVE_QUIET_SECS;
            if expired || quiet {
                l.ended = Some(if expired { l.until.unwrap_or(now).min(now) } else { now });
                out.push((k.clone(), l.clone()));
            }
        }
        // Forget shares that ended more than a week ago.
        g.0.retain(|_, l| l.ended.is_none_or(|e| now < e + 7 * 24 * 3600));
        if !out.is_empty() || now >= g.1 + 30 {
            g.1 = now;
            Self::save(&self.path, &g.0);
        }
        out
    }

    pub fn get(&self, chat: &str) -> Option<Live> {
        self.map.lock().unwrap_or_else(|e| e.into_inner()).0.get(chat).cloned()
    }

    /// Every share in a chat: the chat itself (one-to-one) or `chat/sender` keys (groups),
    /// running ones first, newest first.
    pub fn for_chat(&self, chat: &str) -> Vec<Live> {
        let g = self.map.lock().unwrap_or_else(|e| e.into_inner());
        let prefix = format!("{chat}/");
        let mut v: Vec<Live> = g.0.iter().filter(|(k, _)| *k == chat || k.starts_with(&prefix)).map(|(_, l)| l.clone()).collect();
        v.sort_by_key(|l| (l.ended.is_some(), std::cmp::Reverse(l.updated)));
        v
    }
}

/// Tracker key: the chat, or `chat/sender` in a group (several people may share).
pub fn live_key(chat: &str, sender: &str, group: bool) -> String {
    if group {
        format!("{chat}/{sender}")
    } else {
        chat.to_string()
    }
}

/// The chat part of a tracker key.
pub fn live_chat(key: &str) -> &str {
    key.split('/').next().unwrap_or(key)
}

impl LiveTracker {
    /// Tool answer for a chat: every share in it, or None.
    pub fn status_for_chat(&self, chat: &str, now: u64) -> Option<String> {
        let all = self.for_chat(chat);
        (!all.is_empty()).then(|| all.iter().map(|l| live_status(l, now)).collect::<Vec<_>>().join("\n"))
    }

    fn save(path: &std::path::Path, map: &std::collections::BTreeMap<String, Live>) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = path.with_extension("tmp");
        if let Ok(b) = serde_json::to_vec_pretty(map) {
            if std::fs::write(&tmp, b).is_ok() {
                let _ = std::fs::rename(&tmp, path);
            }
        }
    }
}

fn ago(secs: u64) -> String {
    match secs {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", secs / 60),
        _ => format!("{} h {} min ago", secs / 3600, secs % 3600 / 60),
    }
}

fn clock(unix: u64) -> String {
    format!("{:02}:{:02} UTC", unix % 86400 / 3600, unix % 3600 / 60)
}

/// The tool answer for "where is X now".
pub fn live_status(l: &Live, now: u64) -> String {
    let acc = l.accuracy_m.filter(|a| *a > 0).map(|a| format!(" (±{a} m)")).unwrap_or_default();
    let state = match (l.ended, l.until) {
        (Some(e), _) => format!("sharing ended {}", ago(now.saturating_sub(e))),
        (None, Some(u)) if u > now => format!("still sharing, until {}", clock(u)),
        (None, _) => "still sharing".into(),
    };
    format!(
        "{} — {:.6}, {:.6}{acc}, updated {} ({} updates since {}); {state}. {}",
        l.who,
        l.lat,
        l.lon,
        ago(now.saturating_sub(l.updated)),
        l.updates,
        clock(l.started),
        maps_link(l.lat, l.lon)
    )
}

/// The one line the model gets when a share ends.
pub fn live_ended_text(l: &Live, now: u64) -> String {
    format!(
        "[live location ended] {} — last seen {:.6}, {:.6}, {}. {}",
        l.who,
        l.lat,
        l.lon,
        ago(now.saturating_sub(l.updated)),
        maps_link(l.lat, l.lon)
    )
}

// ---- edited messages --------------------------------------------------------------

/// `2026-10-13 14:02 UTC` (civil date from Unix seconds, no time-zone database).
pub fn utc(unix: i64) -> String {
    let (year, month, day) = civil(unix.div_euclid(86_400));
    let t = unix.rem_euclid(86_400);
    format!("{year}-{month:02}-{day:02} {:02}:{:02} UTC", t / 3600, t % 3600 / 60)
}

/// `2026-10-13T14:02:05Z`, the envelope's timestamp format (history lines use it too).
pub fn rfc3339(unix: i64) -> String {
    let (year, month, day) = civil(unix.div_euclid(86_400));
    let t = unix.rem_euclid(86_400);
    format!("{year}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", t / 3600, t % 3600 / 60, t % 60)
}

/// Days since 1970-01-01 → (year, month, day) (Howard Hinnant's algorithm).
fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// What the model gets when someone edits a message: when it was sent and edited, the
/// old text (None when bia never saw it) and the new one.
pub fn edit_text(id: &str, sent: Option<i64>, edited: i64, old: Option<&str>, new: &str) -> String {
    let edited_s = utc(edited);
    let when = match sent {
        Some(s) => {
            let sent_s = utc(s);
            // same day: only the time of the edit
            let short = if sent_s[..10] == edited_s[..10] { edited_s[11..].to_string() } else { edited_s.clone() };
            format!("sent {sent_s}, edited {short}")
        }
        None => format!("edited {edited_s}"),
    };
    let was = match old {
        Some(o) if !o.trim().is_empty() => o.to_string(),
        Some(_) => "(no text)".into(),
        None => "(not known: sent before bia saw this chat)".into(),
    };
    let now = if new.trim().is_empty() { "(no text)" } else { new };
    format!("[edited {id}] {when}\nwas: {was}\nnow: {now}")
}

/// Last known text of recent messages (Telegram does not send the old text with an
/// edit). Bounded; saved by `flush` when changed.
pub struct TextMemory {
    path: std::path::PathBuf,
    max: usize,
    inner: std::sync::Mutex<(std::collections::VecDeque<(String, i64, String)>, bool)>,
}

impl TextMemory {
    pub fn open(path: impl Into<std::path::PathBuf>, max: usize) -> Self {
        let path = path.into();
        let v: std::collections::VecDeque<(String, i64, String)> = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        TextMemory { path, max, inner: std::sync::Mutex::new((v, false)) }
    }

    /// Remember (or replace) the text of message `key` sent at `sent`.
    pub fn put(&self, key: &str, sent: i64, text: &str) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(e) = g.0.iter_mut().find(|e| e.0 == key) {
            e.2 = text.to_string();
        } else {
            g.0.push_back((key.to_string(), sent, text.to_string()));
            while g.0.len() > self.max {
                g.0.pop_front();
            }
        }
        g.1 = true;
    }

    /// (sent, text) of message `key`.
    pub fn get(&self, key: &str) -> Option<(i64, String)> {
        let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.0.iter().find(|e| e.0 == key).map(|e| (e.1, e.2.clone()))
    }

    pub fn flush(&self) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if !g.1 {
            return;
        }
        g.1 = false;
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = self.path.with_extension("tmp");
        if let Ok(b) = serde_json::to_vec(&g.0) {
            if std::fs::write(&tmp, b).is_ok() {
                let _ = std::fs::rename(&tmp, &self.path);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn location_lines() {
        assert_eq!(
            location_text(52.520008, 13.404954, "Grand Hotel", "Main St 1", Some(20), None, ""),
            "[location] Grand Hotel, Main St 1 — 52.520008, 13.404954 (±20 m) https://maps.google.com/?q=52.520008,13.404954"
        );
        assert_eq!(
            location_text(-33.8688, 151.2093, "", "", None, Some(900), "meet here"),
            "[live location] -33.868800, 151.209300 (sharing live for 15 min) https://maps.google.com/?q=-33.868800,151.209300\nmeet here"
        );
        assert!(location_text(1.0, 2.0, "", "", None, Some(0x7fff_ffff), "").contains("until they stop"));
        assert!(check_coords(91.0, 0.0).is_err());
        assert!(check_coords(f64::NAN, 0.0).is_err());
        assert!(check_coords(-90.0, 180.0).is_ok());
    }

    #[test]
    fn vcard_round_trip() {
        let v = build_vcard("Jane Doe", "+49 170 1234567", "jane@example.com", true);
        assert!(v.contains("waid=491701234567:+49 170 1234567"));
        let c = parse_vcard(&v);
        assert_eq!(c.name, "Jane Doe");
        assert_eq!(c.phones, vec!["+49 170 1234567"]);
        assert_eq!(c.emails, vec!["jane@example.com"]);
        assert_eq!(
            contact_text(&c, "Telegram user 42"),
            "[contact] Jane Doe, +49 170 1234567, jane@example.com; Telegram user 42"
        );
    }

    #[test]
    fn vcard_from_phones() {
        // iPhone-style, folded line, item-prefixed TEL, N without FN
        let v = "BEGIN:VCARD\r\nVERSION:3.0\r\nN:Doe;John;;;\r\nitem1.TEL;type=CELL;waid=15551234567:+1 555-123-\r\n 4567\r\nORG:ACME\\, Inc.;Sales\r\nEND:VCARD";
        let c = parse_vcard(v);
        assert_eq!(c.name, "John Doe");
        assert_eq!(c.phones, vec!["+1 555-123-4567"]);
        assert_eq!(c.org, "ACME, Inc., Sales");
        assert_eq!(contact_text(&c, ""), "[contact] John Doe, +1 555-123-4567 (ACME, Inc., Sales)");
        assert_eq!(contact_text(&Card::default(), ""), "[contact] (empty)");
    }

    #[test]
    fn live_tracker_start_update_end() {
        let dir = std::env::temp_dir().join(format!("bia-live-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("live.json");
        let t = LiveTracker::open(&path);
        assert_eq!(t.seen("c1", "Lena", 52.5, 13.4, Some(10), Some(1000 + 3600), false, 1000), LiveEvent::Started);
        assert_eq!(t.seen("c1", "Lena", 52.51, 13.41, None, None, false, 1010), LiveEvent::Updated);
        let l = t.get("c1").unwrap();
        assert_eq!((l.lat, l.updates, l.accuracy_m, l.until), (52.51, 1, Some(10), Some(4600)));
        assert!(live_status(&l, 1070).contains("Lena — 52.510000, 13.410000 (±10 m), updated 1 min ago (1 updates since 00:16 UTC); still sharing, until 01:16 UTC"));
        // nothing ended yet
        assert!(t.sweep(2000).is_empty());
        // the share runs out
        let ended = t.sweep(4600);
        assert_eq!(ended.len(), 1);
        assert!(live_ended_text(&ended[0].1, 4600).starts_with("[live location ended] Lena — last seen 52.510000, 13.410000"));
        assert!(t.sweep(4700).is_empty(), "reported once");
        // a new share in the same chat starts fresh
        assert_eq!(t.seen("c1", "Lena", 1.0, 2.0, None, None, false, 5000), LiveEvent::Started);
        // quiet share (WhatsApp: no end time) ends after LIVE_QUIET_SECS
        assert!(t.sweep(5000 + LIVE_QUIET_SECS - 1).is_empty());
        assert_eq!(t.sweep(5000 + LIVE_QUIET_SECS).len(), 1);
        // the sender stops it
        assert_eq!(t.seen("c2", "Max", 1.0, 2.0, None, None, false, 6000), LiveEvent::Started);
        assert_eq!(t.seen("c2", "Max", 1.5, 2.5, None, None, true, 6100), LiveEvent::Stopped);
        assert!(t.sweep(6200).is_empty(), "stopped shares are not reported again");
        // groups: one share per sender, listed together
        t.seen(&live_key("g", "a", true), "A", 1.0, 1.0, None, None, false, 7000);
        t.seen(&live_key("g", "b", true), "B", 2.0, 2.0, None, None, false, 7001);
        assert_eq!(t.for_chat("g").iter().map(|l| l.who.as_str()).collect::<Vec<_>>(), vec!["B", "A"]);
        assert_eq!(live_chat("g/b"), "g");
        assert!(t.status_for_chat("nobody", 7002).is_none());
        // survives a restart
        let t2 = LiveTracker::open(&path);
        assert_eq!(t2.get("c2").unwrap().lat, 1.5);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn edits_show_old_new_and_times() {
        assert_eq!(utc(1_791_900_000), "2026-10-13 14:00 UTC");
        assert_eq!(rfc3339(1_791_900_005), "2026-10-13T14:00:05Z");
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(
            edit_text("77", Some(1_791_900_000), 1_791_900_300, Some("see you at 5"), "see you at 6"),
            "[edited 77] sent 2026-10-13 14:00 UTC, edited 14:05 UTC\nwas: see you at 5\nnow: see you at 6"
        );
        assert_eq!(
            edit_text("8", Some(1_791_900_000), 1_791_900_000 + 86_400, Some("a"), "b"),
            "[edited 8] sent 2026-10-13 14:00 UTC, edited 2026-10-14 14:00 UTC\nwas: a\nnow: b"
        );
        assert!(edit_text("9", None, 1_791_900_000, None, "x").contains("was: (not known"));
        let dir = std::env::temp_dir().join(format!("bia-texts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let m = TextMemory::open(dir.join("t.json"), 2);
        m.put("c:1", 10, "one");
        m.put("c:2", 20, "two");
        m.put("c:1", 10, "one (edited)");
        m.put("c:3", 30, "three");
        assert_eq!(m.get("c:1"), None, "oldest dropped past max");
        assert_eq!(m.get("c:3"), Some((30, "three".into())));
        m.flush();
        assert_eq!(TextMemory::open(dir.join("t.json"), 2).get("c:2"), Some((20, "two".into())));
        let _ = std::fs::remove_dir_all(dir);
    }
}
