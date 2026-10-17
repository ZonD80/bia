//! Release notes (RELEASE_NOTES.md at the repo root, built in). On the first boot of a new
//! version the boot notice carries every release since the version that ran before: what
//! changed, for the user, and the steps bia does itself.
use std::path::Path;

const NOTES: &str = include_str!("../../../../RELEASE_NOTES.md");

#[derive(Debug, Default, PartialEq)]
pub struct Release {
    pub version: String,
    pub changes: Vec<String>,
    pub tasks: Vec<String>,
}

/// `## <version>` sections with `### Changes` and `### For bia` bullet lists.
pub fn parse(md: &str) -> Vec<Release> {
    let mut out: Vec<Release> = vec![];
    let mut list: Option<bool> = None; // Some(true) = tasks
    for line in md.lines() {
        let l = line.trim_end();
        if let Some(v) = l.strip_prefix("## ") {
            out.push(Release {
                version: v.trim().to_string(),
                ..Default::default()
            });
            list = None;
        } else if let Some(h) = l.strip_prefix("### ") {
            list = match h.trim().to_lowercase().as_str() {
                "changes" => Some(false),
                "for bia" => Some(true),
                _ => None,
            };
        } else if let (Some(item), Some(tasks), Some(r)) =
            (l.strip_prefix("- "), list, out.last_mut())
        {
            if tasks {
                r.tasks.push(item.trim().to_string());
            } else {
                r.changes.push(item.trim().to_string());
            }
        } else if let (Some(tasks), Some(r)) = (list, out.last_mut()) {
            // a wrapped bullet continues the previous item
            let more = l.trim();
            let v = if tasks { &mut r.tasks } else { &mut r.changes };
            if !more.is_empty() {
                if let Some(last) = v.last_mut() {
                    last.push(' ');
                    last.push_str(more);
                }
            }
        }
    }
    out
}

/// `1.10.2` → (1, 10, 2); anything unparsable sorts first.
fn key(v: &str) -> (u32, u32, u32) {
    let mut it = v
        .trim()
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .map(|s| s.parse().unwrap_or(0));
    (
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
    )
}

/// Releases after `prev` up to and including `cur`, oldest first. Without `prev` (a machine
/// from before versions were recorded) only `cur` itself.
pub fn between(all: Vec<Release>, prev: Option<&str>, cur: &str) -> Vec<Release> {
    let (lo, hi) = (prev.map(key), key(cur));
    let mut v: Vec<Release> = all
        .into_iter()
        .filter(|r| {
            let k = key(&r.version);
            k <= hi && lo.map_or(k == hi, |lo| k > lo)
        })
        .collect();
    v.sort_by_key(|r| key(&r.version));
    v
}

/// The version that ran on the previous boot (files/state/version).
pub fn last_version(state: &Path) -> Option<String> {
    std::fs::read_to_string(state.join("version"))
        .ok()
        .map(|s| s.trim().trim_start_matches("bia ").to_string())
        .filter(|s| !s.is_empty())
}

pub fn record(state: &Path, cur: &str) {
    let _ = std::fs::write(state.join("version"), format!("{cur}\n"));
}

/// The part of the boot notice about an update, or None when there is nothing to tell.
pub fn notice(prev: Option<&str>, cur: &str) -> Option<String> {
    notice_from(NOTES, prev, cur)
}

fn notice_from(md: &str, prev: Option<&str>, cur: &str) -> Option<String> {
    if prev.is_some_and(|p| key(p) >= key(cur)) {
        return None;
    }
    let rel = between(parse(md), prev, cur);
    if rel.is_empty() {
        return None;
    }
    let mut s = match prev {
        Some(p) => format!("bia was updated from {p} to {cur}. Release notes:"),
        None => format!("bia was updated to {cur}. Release notes:"),
    };
    let mut tasks = vec![];
    for r in &rel {
        s.push_str(&format!("\n{}:", r.version));
        for c in &r.changes {
            s.push_str(&format!("\n- {c}"));
        }
        tasks.extend(r.tasks.iter().map(|t| format!("({}) {t}", r.version)));
    }
    if !tasks.is_empty() {
        s.push_str("\nUpdate steps for you (part of this update; do them now, without asking):");
        for t in &tasks {
            s.push_str(&format!("\n- {t}"));
        }
    }
    Some(s)
}

/// What the model is asked to do with the notice.
pub fn instruction(has_tasks: bool) -> &'static str {
    if has_tasks {
        "Tell root in a few short lines, in their language, what changed in this update. Then do the update steps yourself (enable and call the tools they name), and finally tell root what you did and anything that needs them. Then say you are ready for commands."
    } else {
        "Tell root in a few short lines, in their language, what changed in this update, then say you are ready for commands. Do not call tools for this notice."
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MD: &str = "# notes\n\nintro\n\n## 1.2.0\n\n### Changes\n- big thing\n\n## 1.1.1\n\n### Changes\n- web is a bus\n  like telegram\n- groups\n\n### For bia\n- link certificates\n\n## 1.1.0\n\n### Changes\n- old\n";

    #[test]
    fn parses_sections_and_wrapped_items() {
        let r = parse(MD);
        assert_eq!(r.len(), 3);
        assert_eq!(r[1].version, "1.1.1");
        assert_eq!(r[1].changes, vec!["web is a bus like telegram", "groups"]);
        assert_eq!(r[1].tasks, vec!["link certificates"]);
        assert!(r[0].tasks.is_empty());
    }

    #[test]
    fn picks_the_releases_since_the_previous_version() {
        let v = |p: Option<&str>, c: &str| -> Vec<String> {
            between(parse(MD), p, c)
                .into_iter()
                .map(|r| r.version)
                .collect()
        };
        assert_eq!(v(Some("1.1.0"), "1.2.0"), vec!["1.1.1", "1.2.0"]);
        assert_eq!(v(Some("1.1.0"), "1.1.1"), vec!["1.1.1"]);
        assert_eq!(v(None, "1.1.1"), vec!["1.1.1"]);
        assert_eq!(v(Some("1.1.1"), "1.1.1"), Vec::<String>::new());
        assert!(key("1.10.0") > key("1.9.9"));
    }

    #[test]
    fn notice_lists_changes_then_steps() {
        let n = notice_from(MD, Some("1.1.0"), "1.2.0").unwrap();
        assert!(n.starts_with("bia was updated from 1.1.0 to 1.2.0."), "{n}");
        let (a, b) = (
            n.find("web is a bus").unwrap(),
            n.find("Update steps").unwrap(),
        );
        assert!(a < b && n.contains("- (1.1.1) link certificates"), "{n}");
        assert!(!n.contains("old"), "{n}");
        assert!(notice_from(MD, Some("1.2.0"), "1.2.0").is_none());
        assert!(
            notice_from(MD, Some("1.3.0"), "1.2.0").is_none(),
            "downgrade: nothing to tell"
        );
    }

    #[test]
    fn the_built_in_notes_cover_this_version() {
        let all = parse(NOTES);
        let cur = env!("CARGO_PKG_VERSION");
        let r = all
            .iter()
            .find(|r| r.version == cur)
            .unwrap_or_else(|| panic!("RELEASE_NOTES.md has no section for {cur}"));
        assert!(!r.changes.is_empty(), "{cur} has no Changes");
    }

    #[test]
    fn version_file_round_trip() {
        let dir = std::env::temp_dir().join(format!("bia-release-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(last_version(&dir), None);
        record(&dir, "1.1.1");
        assert_eq!(last_version(&dir).as_deref(), Some("1.1.1"));
    }
}
