//! Wall-clock helpers without a time crate: RFC 3339 in UTC, plus timezone / NTP / RTC.
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static CFG: OnceLock<Mutex<ClockCfg>> = OnceLock::new();
static NTP_STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn cfg_lock() -> std::sync::MutexGuard<'static, ClockCfg> {
    CFG.get_or_init(|| Mutex::new(ClockCfg::default()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

pub fn now() -> SystemTime {
    SystemTime::now()
}

pub fn unix(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `2026-09-03T12:34:56Z`
pub fn rfc3339(t: SystemTime) -> String {
    let secs = unix(t);
    let days = (secs / 86400) as i64;
    let (h, m, s) = ((secs % 86400) / 3600, (secs % 3600) / 60, secs % 60);
    let (y, mo, d) = civil_from_days(days);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

pub fn rfc3339_now() -> String {
    rfc3339(now())
}

pub fn rfc3339_unix(secs: u64) -> String {
    rfc3339(UNIX_EPOCH + Duration::from_secs(secs))
}

/// `12:34:56Z` or local `14:34:56+02`
pub fn hms_now() -> String {
    format_hms(unix(now()), offset_east_now())
}

pub fn offset_east_now() -> i32 {
    cfg_lock().offset_east(unix(now()))
}

/// The machine timezone's offset east of UTC (seconds) at `unix`.
pub fn offset_east_at(unix: u64) -> i32 {
    cfg_lock().offset_east(unix)
}

/// The machine timezone's name (IANA or UTC+N), as configured.
pub fn timezone_name() -> String {
    cfg_lock().timezone.clone()
}

pub fn format_hms(unix_secs: u64, east: i32) -> String {
    let local = (unix_secs as i64 + east as i64).max(0) as u64;
    let t = local % 86400;
    let off = crate::tz::format_offset(east);
    let z = if east == 0 { "Z".into() } else { off };
    format!("{:02}:{:02}:{:02}{z}", t / 3600, (t % 3600) / 60, t % 60)
}

pub fn year_of(unix: u64) -> i64 {
    civil_from_days((unix / 86400) as i64).0
}

pub fn unix_civil(y: i64, m: u32, d: u32, tod: u32) -> u64 {
    (days_from_civil(y, m, d) as u64)
        .saturating_mul(86400)
        .saturating_add(tod as u64)
}

/// yyyymmdd-hhmmss for file names
pub fn stamp_now() -> String {
    let secs = unix(now());
    let (y, mo, d) = civil_from_days((secs / 86400) as i64);
    format!(
        "{y:04}{mo:02}{d:02}-{:02}{:02}{:02}",
        (secs % 86400) / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

pub fn ago(from_unix: u64) -> String {
    let secs = unix(now()).saturating_sub(from_unix);
    if secs < 120 {
        format!("{secs}s")
    } else if secs < 7200 {
        format!("{}min", secs / 60)
    } else if secs < 172800 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86400)
    }
}

/// Unix weekday: 0 = Sunday … 6 = Saturday (1970-01-01 was Thursday).
pub fn weekday(unix: u64) -> u32 {
    ((unix / 86400) as u32 + 4) % 7
}

/// minute, hour, day, month, weekday of `unix` shifted by `east` seconds.
pub fn shifted_fields(unix: u64, east: i32) -> (u32, u32, u32, u32, u32) {
    let local = (unix as i64 + east as i64).max(0) as u64;
    let (_y, month, day) = civil_from_days((local / 86400) as i64);
    let tod = local % 86400;
    (
        (tod % 3600 / 60) as u32,
        (tod / 3600) as u32,
        day,
        month,
        weekday(local),
    )
}

pub fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

/// Add calendar months, clamping the day (Jan 31 + 1 month → Feb 28/29).
pub fn add_months(unix: u64, months: i32) -> u64 {
    let time = unix % 86400;
    let (mut y, mut m, d) = civil_from_days((unix / 86400) as i64);
    let mut mm = m as i32 + months;
    while mm > 12 {
        mm -= 12;
        y += 1;
    }
    while mm < 1 {
        mm += 12;
        y -= 1;
    }
    m = mm as u32;
    let d = d.min(days_in_month(y, m));
    (days_from_civil(y, m, d) as u64) * 86400 + time
}

pub fn add_years(unix: u64, years: i32) -> u64 {
    add_months(unix, years * 12)
}

/// RFC 3339, `YYYY-MM-DD HH:MM[:SS]`, `YYYY-MM-DD`, weekday (`monday 09:00`), or relative `in 10m`.
pub fn parse_when(s: &str, now: u64) -> Option<u64> {
    let s = s.trim();
    if let Some(t) = parse_relative(s, now) {
        return Some(t);
    }
    if let Some(t) = parse_weekday_when(s, now) {
        return Some(t);
    }
    let s = s.trim_end_matches('Z').trim_end_matches("+00:00");
    parse_rfc3339(&s.replace(' ', "T"))
}

fn parse_hms(s: &str) -> Option<u64> {
    let rest = s.trim().trim_end_matches('Z').trim_end_matches('z');
    let mut parts = rest.split(':');
    let h: u64 = parts.next()?.parse().ok()?;
    let m: u64 = parts.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    let sec: u64 = parts.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    if h > 23 || m > 59 || sec > 60 {
        None
    } else {
        Some(h * 3600 + m * 60 + sec)
    }
}

fn parse_weekday_when(s: &str, now: u64) -> Option<u64> {
    let s = s.trim().to_ascii_lowercase();
    let (name, rest) = match s.split_once(char::is_whitespace) {
        Some((n, r)) => (n, r.trim()),
        None => (s.as_str(), ""),
    };
    let want = match name {
        "sun" | "sunday" => 0,
        "mon" | "monday" => 1,
        "tue" | "tues" | "tuesday" => 2,
        "wed" | "wednesday" => 3,
        "thu" | "thur" | "thurs" | "thursday" => 4,
        "fri" | "friday" => 5,
        "sat" | "saturday" => 6,
        _ => return None,
    };
    let tod = if rest.is_empty() { 0 } else { parse_hms(rest)? };
    let today = now / 86400 * 86400;
    let add = (want + 7 - weekday(now)) % 7;
    let mut t = today + add as u64 * 86400 + tod;
    if t <= now {
        t += 7 * 86400;
    }
    Some(t)
}

fn parse_relative(s: &str, now: u64) -> Option<u64> {
    let s = s.trim().to_ascii_lowercase();
    let s = s
        .strip_prefix("in ")
        .or_else(|| s.strip_prefix("in"))
        .unwrap_or(&s)
        .trim();
    let i = s.find(|c: char| !c.is_ascii_digit())?;
    if i == 0 {
        return None;
    }
    let n: u64 = s[..i].parse().ok()?;
    let unit = s[i..].trim();
    let secs = match unit {
        "s" | "sec" | "secs" | "second" | "seconds" => n,
        "m" | "min" | "mins" | "minute" | "minutes" => n.saturating_mul(60),
        "h" | "hr" | "hrs" | "hour" | "hours" => n.saturating_mul(3600),
        "d" | "day" | "days" => n.saturating_mul(86400),
        "w" | "week" | "weeks" => n.saturating_mul(86400 * 7),
        _ => return None,
    };
    Some(now.saturating_add(secs))
}

/// Parse `YYYY-MM-DDTHH:MM:SSZ` or `YYYY-MM-DD` (UTC midnight). Optional `+HH:MM` / `-HH:MM`.
pub fn parse_rfc3339(s: &str) -> Option<u64> {
    let s = s.trim();
    let (date, time) = if let Some(i) = s.find('T') {
        let rest = s[i + 1..].trim_end_matches('Z');
        let rest = rest.split('.').next().unwrap_or(rest);
        let (rest, off) = split_offset(rest);
        let mut parts = rest.split(':');
        let h: u64 = parts.next()?.parse().ok()?;
        let m: u64 = parts.next()?.parse().ok()?;
        let sec: u64 = parts.next().and_then(|x| x.parse().ok()).unwrap_or(0);
        if h > 23 || m > 59 || sec > 60 {
            return None;
        }
        (&s[..i], (h * 3600 + m * 60 + sec) as i64 - off)
    } else {
        (s, 0)
    };
    let mut dp = date.split('-');
    let y: i64 = dp.next()?.parse().ok()?;
    let mo: u32 = dp.next()?.parse().ok()?;
    let d: u32 = dp.next()?.parse().ok()?;
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    let days = days_from_civil(y, mo, d);
    Some((days * 86400 + time).max(0) as u64)
}

fn split_offset(rest: &str) -> (&str, i64) {
    if let Some(i) = rest.rfind(['+', '-']) {
        if i == 0 {
            return (rest, 0);
        }
        let sign = if rest.as_bytes()[i] == b'+' { 1i64 } else { -1 };
        let off = rest[i + 1..].trim_end_matches('Z');
        let (h, m) = if let Some((a, b)) = off.split_once(':') {
            (a.parse::<i64>().unwrap_or(0), b.parse::<i64>().unwrap_or(0))
        } else if off.len() == 4 {
            (off[..2].parse().unwrap_or(0), off[2..].parse().unwrap_or(0))
        } else {
            (off.parse().unwrap_or(0), 0)
        };
        return (&rest[..i], sign * (h * 3600 + m * 60));
    }
    if rest.ends_with("+00:00") {
        return (rest.trim_end_matches("+00:00"), 0);
    }
    (rest, 0)
}

pub(crate) fn days_from_civil(mut y: i64, m: u32, d: u32) -> i64 {
    y -= if m <= 2 { 1 } else { 0 };
    let era = y.div_euclid(400);
    let yoe = (y - era * 400) as u64;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp as u64 + 2) / 5 + d as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe as i64 - 719468
}

pub(crate) fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClockCfg {
    #[serde(default = "default_tz")]
    pub timezone: String,
    #[serde(default = "default_true")]
    pub ntp: bool,
    #[serde(default = "default_servers")]
    pub servers: Vec<String>,
    #[serde(default)]
    pub last_sync: Option<u64>,
    #[serde(default)]
    pub last_offset_ms: Option<i64>,
    #[serde(skip)]
    path: Option<PathBuf>,
}

fn default_tz() -> String {
    "UTC".into()
}
fn default_true() -> bool {
    true
}
fn default_servers() -> Vec<String> {
    vec![
        "time.cloudflare.com".into(),
        "time.google.com".into(),
        "pool.ntp.org".into(),
    ]
}

impl Default for ClockCfg {
    fn default() -> Self {
        ClockCfg {
            timezone: default_tz(),
            ntp: true,
            servers: default_servers(),
            last_sync: None,
            last_offset_ms: None,
            path: None,
        }
    }
}

impl ClockCfg {
    fn offset_east(&self, unix: u64) -> i32 {
        crate::tz::resolve(&self.timezone)
            .map(|z| crate::tz::offset_east(&z, unix))
            .unwrap_or(0)
    }

    fn save(&self) -> Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = path.with_extension("json.tmp");
        {
            use std::io::Write;
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(&serde_json::to_vec_pretty(self)?)?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, path).or_else(|_| std::fs::copy(&tmp, path).map(|_| ()))?;
        let _ = std::fs::remove_file(&tmp);
        Ok(())
    }
}

pub fn restore(path: &Path) {
    let loaded = std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice::<ClockCfg>(&b).ok())
        .unwrap_or_default();
    {
        let mut c = cfg_lock();
        *c = loaded;
        c.path = Some(path.to_path_buf());
        if c.servers.is_empty() {
            c.servers = default_servers();
        }
    }
    start_ntp_thread();
}

pub fn snapshot() -> ClockCfg {
    cfg_lock().clone()
}

fn apply_unix(sec: i64, write_rtc: bool) -> Result<()> {
    if sec < 0 {
        bail!("time before 1970");
    }
    let ts = libc::timespec {
        tv_sec: sec as _,
        tv_nsec: 0,
    };
    if unsafe { libc::clock_settime(libc::CLOCK_REALTIME, &ts) } != 0 {
        bail!("clock_settime: {}", std::io::Error::last_os_error());
    }
    if write_rtc {
        let _ = rtc_set(sec as u64);
    }
    Ok(())
}

fn rtc_set(unix: u64) -> Result<()> {
    #[repr(C)]
    struct RtcTime {
        tm_sec: i32,
        tm_min: i32,
        tm_hour: i32,
        tm_mday: i32,
        tm_mon: i32,
        tm_year: i32,
        tm_wday: i32,
        tm_yday: i32,
        tm_isdst: i32,
    }
    const RTC_SET_TIME: libc::c_ulong = 0x4024_700a;
    let (y, mo, d) = civil_from_days((unix / 86400) as i64);
    let tod = unix % 86400;
    let rt = RtcTime {
        tm_sec: (tod % 60) as i32,
        tm_min: ((tod % 3600) / 60) as i32,
        tm_hour: (tod / 3600) as i32,
        tm_mday: d as i32,
        tm_mon: mo as i32 - 1,
        tm_year: (y - 1900) as i32,
        tm_wday: 0,
        tm_yday: 0,
        tm_isdst: 0,
    };
    for p in ["/dev/rtc0", "/dev/rtc"] {
        if let Ok(f) = std::fs::OpenOptions::new().write(true).open(p) {
            use std::os::unix::io::AsRawFd;
            if unsafe { libc::ioctl(f.as_raw_fd(), RTC_SET_TIME as _, &rt) } == 0 {
                return Ok(());
            }
        }
    }
    bail!("no rtc")
}

fn start_ntp_thread() {
    use std::sync::atomic::Ordering;
    if NTP_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("ntp".into())
        .spawn(|| loop {
            if cfg_lock().ntp {
                let wait = match ntp_once() {
                    Ok(_) => Duration::from_secs(64),
                    Err(_) => Duration::from_secs(8),
                };
                std::thread::sleep(wait);
            } else {
                std::thread::sleep(Duration::from_secs(4));
            }
        });
}

fn ntp_once() -> Result<String> {
    let servers = cfg_lock().servers.clone();
    let mut last = anyhow::anyhow!("no ntp servers");
    for host in &servers {
        match sntp(host) {
            Ok((unix, offset_ms)) => {
                apply_unix(unix, true)?;
                {
                    let mut c = cfg_lock();
                    c.last_sync = Some(unix as u64);
                    c.last_offset_ms = Some(offset_ms);
                    let _ = c.save();
                }
                return Ok(format!(
                    "synced from {host}, offset {offset_ms} ms, {}",
                    rfc3339_unix(unix as u64)
                ));
            }
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// SNTP (RFC 4330) over UDP/123. Returns (unix seconds, offset ms).
fn sntp(host: &str) -> Result<(i64, i64)> {
    use std::net::{ToSocketAddrs, UdpSocket};
    let addr = (host.trim(), 123u16)
        .to_socket_addrs()
        .map_err(|e| anyhow::anyhow!("{host}: {e}"))?
        .next()
        .ok_or_else(|| anyhow::anyhow!("{host}: no address"))?;
    let sock = UdpSocket::bind("0.0.0.0:0")?;
    sock.set_read_timeout(Some(Duration::from_secs(3)))?;
    sock.connect(addr)?;
    let mut pkt = [0u8; 48];
    pkt[0] = 0x23; // VN=4, mode=client
    let t0 = unix(now()) as i64;
    sock.send(&pkt)?;
    let n = sock.recv(&mut pkt)?;
    if n < 48 {
        bail!("{host}: short reply");
    }
    let mode = pkt[0] & 0x7;
    if mode != 4 {
        bail!("{host}: not a server reply");
    }
    let ntp_sec = u32::from_be_bytes(pkt[40..44].try_into().unwrap());
    let frac = u32::from_be_bytes(pkt[44..48].try_into().unwrap());
    let mut unix_sec = ntp_sec as i64 - 2_208_988_800;
    if unix_sec < 0 {
        unix_sec += 1i64 << 32;
    }
    if !(1_600_000_000..=4_000_000_000).contains(&unix_sec) {
        bail!("{host}: implausible time {unix_sec}");
    }
    let ms = (frac as u64 * 1000 / (1u64 << 32)) as i64;
    let t1 = unix(now()) as i64;
    let offset_ms = (unix_sec - t0) * 1000 + ms - (t1 - t0) * 500;
    Ok((unix_sec, offset_ms))
}
