//! Typing for learned languages: fcitx5's input list (the kitty console) follows the
//! language registry. A language whose script a US keyboard cannot type gets its layout
//! or input method; `lang_input` overrides that (a native Latin layout, or none). The
//! first item stays the plain US keyboard. Items nobody manages (added by hand in
//! fcitx5) are kept.

use crate::registry::Registry;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub const BASE: &str = "keyboard-us";

/// Language → fcitx5 item. keyboard-<xkb layout>[-<variant>], or an engine.
const DEFAULTS: &[(&str, &str)] = &[
    ("am", "keyboard-et"),
    ("ar", "keyboard-ara"),
    ("as", "keyboard-in-asm-kagapa"),
    ("ba", "keyboard-ru-bak"),
    ("be", "keyboard-by"),
    ("bg", "keyboard-bg"),
    ("bn", "keyboard-bd-probhat"),
    ("bo", "keyboard-cn-tib"),
    ("el", "keyboard-gr"),
    ("fa", "keyboard-ir"),
    ("gu", "keyboard-in-guj"),
    ("he", "keyboard-il"),
    ("hi", "keyboard-in"),
    ("hy", "keyboard-am"),
    // Not mozc: its server refuses to run as root, and the console session is root.
    ("ja", "anthy"),
    ("ka", "keyboard-ge"),
    ("kk", "keyboard-kz"),
    ("km", "keyboard-kh"),
    ("kn", "keyboard-in-kan"),
    ("ko", "hangul"),
    ("lo", "keyboard-la"),
    ("mk", "keyboard-mk"),
    ("ml", "keyboard-in-mal"),
    ("mn", "keyboard-mn"),
    ("mr", "keyboard-in-marathi"),
    ("my", "keyboard-mm"),
    ("ne", "keyboard-np"),
    ("pa", "keyboard-in-guru"),
    ("ps", "keyboard-af-ps"),
    ("ru", "keyboard-ru"),
    ("sa", "keyboard-in-san-kagapa"),
    ("sd", "keyboard-pk-snd"),
    ("si", "keyboard-lk"),
    ("sr", "keyboard-rs"),
    ("ta", "keyboard-in-tam"),
    ("te", "keyboard-in-tel"),
    ("tg", "keyboard-tj"),
    ("th", "keyboard-th"),
    ("tt", "keyboard-ru-tt"),
    ("uk", "keyboard-ua"),
    ("ur", "keyboard-pk"),
    ("vi", "unikey"),
    // No Yiddish layout: Hebrew script.
    ("yi", "keyboard-il"),
    ("zh", "pinyin"),
];

pub fn default_for(code: &str) -> Option<&'static str> {
    DEFAULTS.iter().find(|(c, _)| *c == code).map(|(_, i)| *i)
}

/// What `code` types with: its override ("" = none) or the default.
pub fn for_lang(r: &Registry, code: &str) -> Option<String> {
    match r.input.get(code) {
        Some(i) => Some(i.clone()).filter(|i| !i.is_empty()),
        None => default_for(code).map(str::to_string),
    }
}

/// Whether fcitx5 here has `item`: an xkb layout[-variant] from evdev.lst, or an input
/// method engine with its file in /usr/share/fcitx5/inputmethod. Unknown when neither
/// list exists (accepted).
pub fn check(item: &str) -> Result<(), String> {
    if let Some(kb) = item.strip_prefix("keyboard-") {
        let Ok(lst) = std::fs::read_to_string("/usr/share/X11/xkb/rules/evdev.lst") else {
            return Ok(());
        };
        let (layout, variant) = kb.split_once('-').unwrap_or((kb, ""));
        let mut section = "";
        let found = lst.lines().any(|l| {
            if let Some(h) = l.strip_prefix("! ") {
                section = h;
                return false;
            }
            let mut w = l.split_whitespace();
            match (section, w.next(), w.next()) {
                ("layout", Some(n), _) => variant.is_empty() && n == layout,
                ("variant", Some(n), Some(of)) => n == variant && of == format!("{layout}:"),
                _ => false,
            }
        });
        return if found { Ok(()) } else { Err(format!("no keyboard layout {item} (xkb {layout}{})", if variant.is_empty() { String::new() } else { format!("({variant})") })) };
    }
    let dir = Path::new("/usr/share/fcitx5/inputmethod");
    if !dir.is_dir() || dir.join(format!("{item}.conf")).is_file() {
        return Ok(());
    }
    let mut have: Vec<String> = std::fs::read_dir(dir)
        .map(|d| d.flatten().filter_map(|e| e.file_name().to_str()?.strip_suffix(".conf").map(str::to_string)).collect())
        .unwrap_or_default();
    have.sort();
    Err(format!("no input method {item}; installed: {}, or keyboard-<layout>[-<variant>]", have.join(", ")))
}

/// Top-bar label: the script's own sign for CJK, else the language code.
fn label(code: &str) -> String {
    match code {
        "zh" => "中".into(),
        "ja" => "あ".into(),
        "ko" => "한".into(),
        _ => code.to_ascii_uppercase(),
    }
}

/// Inputs `r` needs, default language first: (fcitx5 item, label).
pub fn wanted(r: &Registry) -> Vec<(String, String)> {
    let langs = std::iter::once(&r.default_lang).chain(&r.enabled).chain(r.voice.keys());
    let mut out: Vec<(String, String)> = Vec::new();
    for code in langs {
        if let Some(item) = for_lang(r, code) {
            if item != BASE && !out.iter().any(|(i, _)| *i == item) {
                out.push((item, label(code)));
            }
        }
    }
    out
}

/// Items this module may remove: every default plus any override, old or new.
fn managed(old: Option<&Registry>, new: &Registry) -> BTreeSet<String> {
    let mut m: BTreeSet<String> = DEFAULTS.iter().map(|(_, i)| i.to_string()).collect();
    for r in old.into_iter().chain([new]) {
        m.extend(r.input.values().filter(|i| !i.is_empty() && *i != BASE).cloned());
    }
    m
}

/// True when a registry change can change the input list.
pub fn affects(old: &Registry, new: &Registry) -> bool {
    old.input != new.input || wanted(old) != wanted(new)
}

/// fcitx5's config directory (or $BIA_FCITX5_DIR). bia-session creates it. Unit tests
/// never reach a real one.
pub fn dir() -> PathBuf {
    let real = if cfg!(test) { "/nonexistent/fcitx5" } else { "/root/.config/fcitx5" };
    std::env::var("BIA_FCITX5_DIR").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(real))
}

/// Bring fcitx5's profile in line with the registry and reload fcitx5. `old` names the
/// overrides of the previous registry, so an item one of them added is dropped too.
/// Does nothing without the config directory (no GUI session).
pub fn sync(old: Option<&Registry>, new: &Registry) {
    let d = dir();
    if !d.is_dir() {
        return;
    }
    let want = wanted(new);
    match apply(&d, &want, &managed(old, new)) {
        Ok(true) => {
            let text = std::fs::read_to_string(d.join("profile")).unwrap_or_default();
            reload(&item_names(&text));
        }
        Ok(false) => {}
        Err(e) => crate::diag::log("input", e),
    }
}

/// Group 0's items in a profile, in order.
fn item_names(profile: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut item = false;
    for l in profile.lines().map(str::trim) {
        if l.starts_with('[') {
            item = l.strip_prefix("[Groups/0/Items/").is_some_and(|r| r.trim_end_matches(']').parse::<u32>().is_ok());
        } else if let Some(n) = l.strip_prefix("Name=").filter(|_| item) {
            out.push(n.to_string());
        }
    }
    out
}

/// Rewrite `profile` and `bia-labels` (read by /usr/local/sbin/bia-im). True when the
/// profile changed.
pub fn apply(dir: &Path, want: &[(String, String)], managed: &BTreeSet<String>) -> Result<bool, String> {
    let path = dir.join("profile");
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    let new = profile(&old, want, managed);
    let mut labels = format!("{BASE} EN\n");
    for (item, l) in want {
        labels.push_str(&format!("{item} {l}\n"));
    }
    write_if_changed(&dir.join("bia-labels"), &labels)?;
    write_if_changed(&path, &new)
}

fn write_if_changed(path: &Path, text: &str) -> Result<bool, String> {
    if std::fs::read_to_string(path).is_ok_and(|t| t == text) {
        return Ok(false);
    }
    let tmp = path.with_extension("bia-tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("rename {}: {e}", path.display()))?;
    Ok(true)
}

/// fcitx5 profile text with group 0's items replaced: the base keyboard first, items
/// nobody manages and wanted ones in their old order, then new wanted ones.
fn profile(old: &str, want: &[(String, String)], managed: &BTreeSet<String>) -> String {
    // (header, body lines); the header of lines before any section is "".
    let mut sections: Vec<(String, Vec<String>)> = vec![(String::new(), Vec::new())];
    for line in old.lines() {
        let t = line.trim();
        if t.starts_with('[') && t.ends_with(']') {
            sections.push((t[1..t.len() - 1].to_string(), Vec::new()));
        } else if let Some(s) = sections.last_mut() {
            s.1.push(line.to_string());
        }
    }
    let is_item = |h: &str| h.strip_prefix("Groups/0/Items/").is_some_and(|n| n.parse::<u32>().is_ok());
    let item_name = |body: &[String]| {
        body.iter()
            .find_map(|l| l.trim().strip_prefix("Name="))
            .unwrap_or("")
            .to_string()
    };
    let mut items: Vec<(String, Vec<String>)> = sections
        .iter()
        .filter(|(h, _)| is_item(h))
        .map(|(_, b)| (item_name(b), b.clone()))
        .filter(|(n, _)| !n.is_empty())
        .filter(|(n, _)| *n == BASE || !managed.contains(n) || want.iter().any(|(w, _)| w == n))
        .collect();
    let fresh = |n: &str| (n.to_string(), vec![format!("Name={n}"), "Layout=".to_string(), String::new()]);
    if !items.iter().any(|(n, _)| n == BASE) {
        items.insert(0, fresh(BASE));
    }
    for (w, _) in want {
        if !items.iter().any(|(n, _)| n == w) {
            items.push(fresh(w));
        }
    }
    let default_im = want.first().map_or(BASE, |(w, _)| w.as_str());

    let group = |out: &mut String, body: Option<&Vec<String>>| {
        out.push_str("[Groups/0]\n");
        let mut has_default = false;
        for l in body.into_iter().flatten() {
            if l.trim().starts_with("DefaultIM=") {
                out.push_str(&format!("DefaultIM={default_im}\n"));
                has_default = true;
            } else {
                out.push_str(l);
                out.push('\n');
            }
        }
        if body.is_none() {
            out.push_str("# Group Name\nName=Default\n# Layout\nDefault Layout=us\n");
        }
        if !has_default {
            out.push_str(&format!("# Default Input Method\nDefaultIM={default_im}\n\n"));
        }
        for (i, (_, b)) in items.iter().enumerate() {
            out.push_str(&format!("[Groups/0/Items/{i}]\n"));
            for l in b {
                out.push_str(l);
                out.push('\n');
            }
        }
    };
    let mut out = String::new();
    let mut wrote_group = false;
    for (h, body) in &sections {
        if h.is_empty() {
            for l in body {
                out.push_str(l);
                out.push('\n');
            }
        } else if h == "Groups/0" {
            group(&mut out, Some(body));
            wrote_group = true;
        } else if !is_item(h) {
            out.push_str(&format!("[{h}]\n"));
            for l in body {
                out.push_str(l);
                out.push('\n');
            }
        }
    }
    if !wrote_group {
        let mut head = String::new();
        group(&mut head, None);
        out = head + "\n" + &out;
        if !sections.iter().any(|(h, _)| h == "GroupOrder") {
            out.push_str("[GroupOrder]\n0=Default\n\n");
        }
    }
    out
}

/// Hand the new list to a running fcitx5 (it lives on bia-session's bus). Its reload
/// (`fcitx5-remote -r`) does not re-read the profile, so the group is set over DBus;
/// fcitx5 then saves the same list itself.
fn reload(items: &[String]) {
    let bus = "/run/bia/session-bus";
    if !Path::new(bus).exists() || items.is_empty() {
        return;
    }
    let mut args: Vec<String> = vec![format!("--address=unix:path={bus}"), "--timeout=5".into()];
    args.extend(
        ["call", "org.fcitx.Fcitx5", "/controller", "org.fcitx.Fcitx.Controller1", "SetInputMethodGroupInfo", "ssa(ss)", "Default", "us"]
            .map(String::from),
    );
    args.push(items.len().to_string());
    for i in items {
        args.push(i.clone());
        args.push(String::new());
    }
    let r = std::process::Command::new("busctl")
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output();
    match r {
        Ok(o) if o.status.success() => {}
        Ok(o) => crate::diag::log("input", format!("fcitx5 update: {}", String::from_utf8_lossy(&o.stderr).trim())),
        Err(e) => crate::diag::log("input", format!("busctl: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(p: &str) -> Vec<String> {
        p.lines().filter_map(|l| l.strip_prefix("Name=")).filter(|n| *n != "Default").map(str::to_string).collect()
    }

    #[test]
    fn follows_the_registry() {
        let mut r = Registry::default();
        assert!(wanted(&r).is_empty(), "English needs nothing beyond the base keyboard");
        r.default_lang = "ru".into();
        r.enabled = vec!["ru".into(), "en".into(), "zh".into(), "de".into()];
        r.voice.insert("ja".into(), "x".into());
        let w: Vec<String> = wanted(&r).into_iter().map(|(i, l)| format!("{i}={l}")).collect();
        assert_eq!(w, ["keyboard-ru=RU", "pinyin=中", "anthy=あ"]);
        r.input.insert("de".into(), "keyboard-de".into());
        r.input.insert("zh".into(), String::new());
        let w: Vec<String> = wanted(&r).into_iter().map(|(i, _)| i).collect();
        assert_eq!(w, ["keyboard-ru", "keyboard-de", "anthy"]);
    }

    #[test]
    fn rewrites_the_profile() {
        let shipped = "[Groups/0]\n# Group Name\nName=Default\n# Layout\nDefault Layout=us\n# Default Input Method\nDefaultIM=pinyin\n\n[Groups/0/Items/0]\nName=keyboard-us\nLayout=\n\n[Groups/0/Items/1]\nName=hangul\nLayout=\n\n[Groups/0/Items/2]\nName=keyboard-us-dvorak\nLayout=\n\n[GroupOrder]\n0=Default\n\n";
        let mut r = Registry::default();
        r.enabled = vec!["en".into(), "ru".into()];
        let want = wanted(&r);
        let p = profile(shipped, &want, &managed(None, &r));
        assert_eq!(names(&p), ["keyboard-us", "keyboard-us-dvorak", "keyboard-ru"], "hangul dropped, hand-added kept");
        assert_eq!(item_names(&p), names(&p));
        assert!(p.contains("DefaultIM=keyboard-ru\n"));
        assert!(p.contains("[Groups/0/Items/2]\nName=keyboard-ru\n"));
        assert!(p.contains("[GroupOrder]\n0=Default"));
        assert_eq!(profile(&p, &want, &managed(None, &r)), p, "stable");

        // An override's item goes when the override does.
        let mut old = r.clone();
        old.input.insert("de".into(), "keyboard-de".into());
        old.enabled.push("de".into());
        let p2 = profile(&p, &wanted(&old), &managed(None, &old));
        assert!(names(&p2).contains(&"keyboard-de".to_string()));
        let p3 = profile(&p2, &want, &managed(Some(&old), &r));
        assert_eq!(names(&p3), ["keyboard-us", "keyboard-us-dvorak", "keyboard-ru"]);

        let empty = profile("", &want, &managed(None, &r));
        assert_eq!(names(&empty), ["keyboard-us", "keyboard-ru"]);
        assert!(empty.starts_with("[Groups/0]\n") && empty.contains("[GroupOrder]\n0=Default"));
    }

    /// Needs xkb-data and the fcitx5 engines of the image (rootfs.sh) installed.
    #[test]
    #[ignore]
    fn defaults_exist() {
        assert!(check("keyboard-de").is_ok() && check("keyboard-ru-phonetic").is_ok());
        assert!(check("keyboard-xx").is_err() && check("keyboard-ru-nope").is_err() && check("nope").is_err());
        let bad: Vec<String> = DEFAULTS.iter().filter_map(|(c, i)| check(i).err().map(|e| format!("{c}: {e}"))).collect();
        assert!(bad.is_empty(), "{bad:#?}");
    }

    #[test]
    fn apply_writes_labels_and_reports_changes() {
        let d = std::env::temp_dir().join(format!("bia-input-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let mut r = Registry::default();
        r.enabled = vec!["en".into(), "ko".into()];
        let want = wanted(&r);
        assert!(apply(&d, &want, &managed(None, &r)).unwrap());
        assert!(!apply(&d, &want, &managed(None, &r)).unwrap());
        assert_eq!(std::fs::read_to_string(d.join("bia-labels")).unwrap(), "keyboard-us EN\nhangul 한\n");
        let _ = std::fs::remove_dir_all(&d);
    }
}
