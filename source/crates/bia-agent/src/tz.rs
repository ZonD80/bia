//! IANA names and POSIX TZ strings (no tzdata on the image).
//! Offset is seconds east of UTC (RFC 3339 `+02:00`).

#[derive(Clone, Debug)]
pub struct Zone {
    /// POSIX TZ used for DST (empty = fixed offset).
    posix: String,
    /// Fallback east-of-UTC seconds when POSIX has no DST.
    fixed_east: i32,
}

pub fn resolve(name: &str) -> Result<Zone, String> {
    let raw = name.trim();
    if raw.is_empty() {
        return Err("timezone required (e.g. Europe/Warsaw, UTC+2, +02:00)".into());
    }
    let key = normalize(raw);
    if key == "utc" || key == "gmt" || key == "z" || key == "etc/utc" || key == "etc/gmt" {
        return Ok(Zone {
            posix: "UTC0".into(),
            fixed_east: 0,
        });
    }
    if let Some((_, posix)) = IANA.iter().find(|(n, _)| normalize(n) == key).copied() {
        return Ok(Zone {
            posix: posix.into(),
            fixed_east: 0,
        });
    }
    if let Some(east) = parse_offset(raw) {
        return Ok(Zone {
            posix: String::new(),
            fixed_east: east,
        });
    }
    if raw.contains(',')
        || raw.chars().any(|c| c.is_ascii_digit()) && raw.chars().any(|c| c.is_ascii_alphabetic())
    {
        return Ok(Zone {
            posix: raw.to_string(),
            fixed_east: 0,
        });
    }
    Err(format!(
        "unknown timezone {raw:?}; try UTC, UTC+2, +02:00, or an IANA name such as Europe/Warsaw, America/New_York, Asia/Tokyo"
    ))
}

pub fn offset_east(zone: &Zone, unix: u64) -> i32 {
    if zone.posix.is_empty() {
        return zone.fixed_east;
    }
    posix_offset_east(&zone.posix, unix).unwrap_or(zone.fixed_east)
}

pub fn format_offset(east: i32) -> String {
    let sign = if east >= 0 { '+' } else { '-' };
    let a = east.unsigned_abs();
    if a % 3600 == 0 {
        format!("{sign}{:02}", a / 3600)
    } else {
        format!("{sign}{:02}:{:02}", a / 3600, (a % 3600) / 60)
    }
}

fn normalize(s: &str) -> String {
    s.trim().to_ascii_lowercase().replace(' ', "_")
}

fn parse_offset(s: &str) -> Option<i32> {
    let t = s.trim();
    let t = t
        .strip_prefix("UTC")
        .or_else(|| t.strip_prefix("utc"))
        .or_else(|| t.strip_prefix("GMT"))
        .or_else(|| t.strip_prefix("gmt"))
        .unwrap_or(t);
    let t = t.trim();
    if t.is_empty() {
        return Some(0);
    }
    let (sign, rest) = match t.as_bytes().first() {
        Some(b'+') => (1i32, &t[1..]),
        Some(b'-') => (-1i32, &t[1..]),
        _ => return None,
    };
    let rest = rest.trim();
    let (h, m) = if let Some((a, b)) = rest.split_once(':') {
        (a.parse::<u32>().ok()?, b.parse::<u32>().ok()?)
    } else if rest.len() == 4 && rest.chars().all(|c| c.is_ascii_digit()) {
        (rest[..2].parse().ok()?, rest[2..].parse().ok()?)
    } else {
        (rest.parse::<u32>().ok()?, 0)
    };
    if h > 14 || m > 59 {
        return None;
    }
    Some(sign * (h as i32 * 3600 + m as i32 * 60))
}

/// POSIX: `std offset [dst [offset] [,start[/time],end[/time]]]`. Offset is west of UTC.
fn posix_offset_east(spec: &str, unix: u64) -> Option<i32> {
    let (std_off, dst_off, start, end) = parse_posix(spec)?;
    let in_dst = match (start, end) {
        (Some(s), Some(e)) => is_dst(unix, std_off, dst_off, s, e),
        _ => false,
    };
    Some(-(if in_dst { dst_off } else { std_off }))
}

#[derive(Clone, Copy)]
struct Rule {
    month: u32,
    week: u32,
    dow: u32,
    tod: u32,
}

fn parse_posix(spec: &str) -> Option<(i32, i32, Option<Rule>, Option<Rule>)> {
    let spec = spec.trim();
    let (std_off, rest) = take_offset_after_name(spec, true)?;
    let rest = rest.trim_start();
    if rest.is_empty() {
        return Some((std_off, std_off, None, None));
    }
    let (dst_off, rest) = if rest.starts_with(',') {
        (std_off - 3600, rest)
    } else {
        let after_name = skip_tzname(rest);
        if let Some((off, n)) = parse_posix_offset(after_name) {
            (off, &after_name[n..])
        } else {
            (std_off - 3600, after_name)
        }
    };
    let rest = rest.trim_start();
    if rest.is_empty() {
        return Some((std_off, dst_off, None, None));
    }
    let rest = rest.strip_prefix(',')?;
    let (a, b) = rest.split_once(',')?;
    Some((std_off, dst_off, Some(parse_rule(a)?), Some(parse_rule(b)?)))
}

fn skip_tzname(s: &str) -> &str {
    let mut i = 0;
    let b = s.as_bytes();
    if i < b.len() && b[i] == b'<' {
        i += 1;
        while i < b.len() && b[i] != b'>' {
            i += 1;
        }
        i += 1;
    } else {
        while i < b.len() && (b[i].is_ascii_alphabetic() || b[i] == b'_' || b[i] == b'/') {
            i += 1;
        }
    }
    &s[i.min(s.len())..]
}

fn take_offset_after_name(s: &str, offset_required: bool) -> Option<(i32, &str)> {
    let rest = skip_tzname(s);
    if let Some((off, n)) = parse_posix_offset(rest) {
        Some((off, &rest[n..]))
    } else if offset_required {
        None
    } else {
        Some((0, rest))
    }
}

/// POSIX offset: `[+|-]hh[:mm[:ss]]`, positive is west of UTC.
fn parse_posix_offset(s: &str) -> Option<(i32, usize)> {
    let mut i = 0;
    let mut sign = 1i32;
    let b = s.as_bytes();
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        if b[i] == b'-' {
            sign = -1;
        }
        i += 1;
    }
    if i >= b.len() || !b[i].is_ascii_digit() {
        return None;
    }
    let start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let h: i32 = s[start..i].parse().ok()?;
    let mut m = 0i32;
    let mut sec = 0i32;
    if i < b.len() && b[i] == b':' {
        i += 1;
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        m = s[start..i].parse().ok()?;
        if i < b.len() && b[i] == b':' {
            i += 1;
            let start = i;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            sec = s[start..i].parse().ok()?;
        }
    }
    Some((sign * (h * 3600 + m * 60 + sec), i))
}

fn parse_rule(s: &str) -> Option<Rule> {
    let s = s.trim();
    let (body, tod) = if let Some((a, b)) = s.split_once('/') {
        (a, parse_tod(b).unwrap_or(2 * 3600))
    } else {
        (s, 2 * 3600)
    };
    let body = body.strip_prefix('M')?;
    let mut p = body.split('.');
    Some(Rule {
        month: p.next()?.parse().ok()?,
        week: p.next()?.parse().ok()?,
        dow: p.next()?.parse().ok()?,
        tod,
    })
}

fn parse_tod(s: &str) -> Option<u32> {
    let mut p = s.split(':');
    let h: u32 = p.next()?.parse().ok()?;
    let m: u32 = p.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    let sec: u32 = p.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    Some(h * 3600 + m * 60 + sec)
}

fn is_dst(unix: u64, std_off: i32, dst_off: i32, start: Rule, end: Rule) -> bool {
    let y = crate::clock::year_of(unix);
    let start_u = rule_unix(y, start, std_off);
    let end_u = rule_unix(y, end, dst_off);
    if start_u <= end_u {
        unix >= start_u && unix < end_u
    } else {
        unix >= start_u || unix < end_u
    }
}

fn rule_unix(year: i64, r: Rule, offset_west: i32) -> u64 {
    let dim = crate::clock::days_in_month(year, r.month);
    let mut day = if r.week == 5 {
        dim
    } else {
        1 + (r.week.saturating_sub(1)) * 7
    };
    day = day.min(dim);
    let mut t = crate::clock::unix_civil(year, r.month, day, r.tod);
    let want = r.dow;
    for _ in 0..7 {
        if crate::clock::weekday(t) == want {
            break;
        }
        if r.week == 5 {
            t = t.saturating_sub(86400);
        } else {
            t = t.saturating_add(86400);
        }
    }
    // POSIX time is local; convert to UTC by adding west-of-UTC offset.
    ((t as i64) + offset_west as i64).max(0) as u64
}

/// Common IANA names → POSIX TZ. EU: last Sunday March 02:00 / last Sunday October 03:00.
const IANA: &[(&str, &str)] = &[
    ("UTC", "UTC0"),
    ("Etc/UTC", "UTC0"),
    ("Etc/GMT", "GMT0"),
    ("Europe/London", "GMT0BST,M3.5.0/1,M10.5.0/2"),
    ("Europe/Dublin", "IST-1GMT0,M10.5.0,M3.5.0/1"),
    ("Europe/Lisbon", "WET0WEST,M3.5.0/1,M10.5.0/2"),
    ("Europe/Paris", "CET-1CEST,M3.5.0/2,M10.5.0/3"),
    ("Europe/Berlin", "CET-1CEST,M3.5.0/2,M10.5.0/3"),
    ("Europe/Warsaw", "CET-1CEST,M3.5.0/2,M10.5.0/3"),
    ("Europe/Prague", "CET-1CEST,M3.5.0/2,M10.5.0/3"),
    ("Europe/Vienna", "CET-1CEST,M3.5.0/2,M10.5.0/3"),
    ("Europe/Rome", "CET-1CEST,M3.5.0/2,M10.5.0/3"),
    ("Europe/Madrid", "CET-1CEST,M3.5.0/2,M10.5.0/3"),
    ("Europe/Amsterdam", "CET-1CEST,M3.5.0/2,M10.5.0/3"),
    ("Europe/Brussels", "CET-1CEST,M3.5.0/2,M10.5.0/3"),
    ("Europe/Stockholm", "CET-1CEST,M3.5.0/2,M10.5.0/3"),
    ("Europe/Oslo", "CET-1CEST,M3.5.0/2,M10.5.0/3"),
    ("Europe/Copenhagen", "CET-1CEST,M3.5.0/2,M10.5.0/3"),
    ("Europe/Zurich", "CET-1CEST,M3.5.0/2,M10.5.0/3"),
    ("Europe/Budapest", "CET-1CEST,M3.5.0/2,M10.5.0/3"),
    ("Europe/Helsinki", "EET-2EEST,M3.5.0/3,M10.5.0/4"),
    ("Europe/Athens", "EET-2EEST,M3.5.0/3,M10.5.0/4"),
    ("Europe/Bucharest", "EET-2EEST,M3.5.0/3,M10.5.0/4"),
    ("Europe/Kyiv", "EET-2EEST,M3.5.0/3,M10.5.0/4"),
    ("Europe/Kiev", "EET-2EEST,M3.5.0/3,M10.5.0/4"),
    ("Europe/Riga", "EET-2EEST,M3.5.0/3,M10.5.0/4"),
    ("Europe/Moscow", "MSK-3"),
    ("Europe/Istanbul", "TRT-3"),
    ("America/New_York", "EST5EDT,M3.2.0,M11.1.0"),
    ("America/Chicago", "CST6CDT,M3.2.0,M11.1.0"),
    ("America/Denver", "MST7MDT,M3.2.0,M11.1.0"),
    ("America/Los_Angeles", "PST8PDT,M3.2.0,M11.1.0"),
    ("America/Phoenix", "MST7"),
    ("America/Anchorage", "AKST9AKDT,M3.2.0,M11.1.0"),
    ("America/Honolulu", "HST10"),
    ("America/Toronto", "EST5EDT,M3.2.0,M11.1.0"),
    ("America/Vancouver", "PST8PDT,M3.2.0,M11.1.0"),
    ("America/Sao_Paulo", "BRT3"),
    ("America/Mexico_City", "CST6"),
    ("America/Buenos_Aires", "ART3"),
    ("America/Argentina/Buenos_Aires", "ART3"),
    ("Asia/Tokyo", "JST-9"),
    ("Asia/Seoul", "KST-9"),
    ("Asia/Shanghai", "CST-8"),
    ("Asia/Hong_Kong", "HKT-8"),
    ("Asia/Singapore", "SGT-8"),
    ("Asia/Taipei", "CST-8"),
    ("Asia/Bangkok", "ICT-7"),
    ("Asia/Jakarta", "WIB-7"),
    ("Asia/Kolkata", "IST-5:30"),
    ("Asia/Calcutta", "IST-5:30"),
    ("Asia/Dubai", "GST-4"),
    ("Asia/Jerusalem", "IST-2IDT,M3.4.4/26,M10.5.0"),
    ("Australia/Sydney", "AEST-10AEDT,M10.1.0,M4.1.0/3"),
    ("Australia/Melbourne", "AEST-10AEDT,M10.1.0,M4.1.0/3"),
    ("Australia/Perth", "AWST-8"),
    ("Pacific/Auckland", "NZST-12NZDT,M9.5.0,M4.1.0/3"),
    ("Africa/Cairo", "EET-2"),
    ("Africa/Johannesburg", "SAST-2"),
    ("Africa/Lagos", "WAT-1"),
    ("Africa/Nairobi", "EAT-3"),
];
