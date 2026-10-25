//! Chrome's native GTK file dialogs on :0: notice when one opens, and pick files in it.
//!
//! Chrome cancels its GTK dialog on any keyboard accept (Return, row activation, or Open on
//! a typed folder), so the dialog is always accepted with a mouse click on its default
//! button. A click on the empty action area first closes the location completion popup,
//! which otherwise eats the button click. Several files are hard-linked into a fresh folder
//! that is GTK bookmark 1; alt+1 opens it and ctrl+a selects them all.
use bia_mcp::{arg_bool, arg_str_list, Ctx};
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::{cdp, proc};

const ROLE: &str = "GtkFileChooserDialog";
const HOME: &str = "/home/bia";
const LABEL: &str = "bia-pick";
const KEEP: Duration = Duration::from_secs(24 * 3600);

#[derive(Clone)]
struct Dialog {
    id: String,
    title: String,
    page: String,
}

/// Dialogs already announced, and windows known not to be dialogs (a role never changes).
#[derive(Default)]
pub struct Seen {
    dialogs: HashSet<String>,
    others: HashSet<String>,
}

impl Seen {
    pub fn poll(&mut self, ctx: Option<&Ctx>) {
        let open = scan(&mut self.others);
        self.dialogs.retain(|id| open.iter().any(|d| &d.id == id));
        for d in open {
            if !self.dialogs.insert(d.id.clone()) {
                continue;
            }
            if let Some(ctx) = ctx {
                ctx.inbound("system", "chrome", "", &notice(&d));
            }
        }
    }
}

fn notice(d: &Dialog) -> String {
    let page = if d.page.is_empty() { String::new() } else { format!(" for {}", d.page) };
    format!(
        "[file dialog opened] {}{page}\nThe page waits until it is closed: chrome_choose_files(paths=[...]) or chrome_choose_files(cancel=true).",
        d.title
    )
}

/// Tool `choose_files`.
pub fn choose(args: &Value) -> Result<String, String> {
    let d = dialogs().pop().ok_or(
        "No file dialog is open. Click the page's upload button or drop zone first; a [file dialog opened] notice arrives when it opens.",
    )?;
    let g = Geometry::of(&d.id)?;
    x("xdotool", &["windowactivate", "--sync", &d.id])?;
    if arg_bool(args, "cancel", false) {
        x("xdotool", &["key", "Escape"])?;
        return closed(&d, "Closed the file dialog without choosing.".into());
    }
    let paths = paths(args)?;
    let save = paths.len() == 1 && (!paths[0].exists() || d.title.to_lowercase().contains("save"));
    if save {
        return save_as(&d, &g, &paths[0]);
    }
    for p in &paths {
        if !p.exists() {
            return Err(format!("{} does not exist", p.display()));
        }
        if !bia_can("-r", p) {
            return Err(format!(
                "Chrome runs as bia and cannot read {}. chown it to bia or copy it under /home/bia/files.",
                p.display()
            ));
        }
    }
    if paths.len() == 1 && paths[0].is_dir() {
        return folder(&d, &g, &paths[0]);
    }
    if let Some(dir) = paths.iter().find(|p| p.is_dir()) {
        return Err(format!("{} is a folder. Pass files, or one folder alone for a folder upload.", dir.display()));
    }
    if paths.len() == 1 {
        return single(&d, &g, &paths[0]);
    }
    several(&d, &g, &paths)
}

fn paths(args: &Value) -> Result<Vec<PathBuf>, String> {
    let mut list = arg_str_list(args, "paths");
    if list.is_empty() {
        if let Some(p) = args.get("paths").and_then(|v| v.as_str()) {
            list.push(p.to_string());
        }
    }
    let list: Vec<PathBuf> = list.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).map(PathBuf::from).collect();
    if list.is_empty() {
        return Err("paths is empty: give absolute file paths, or cancel=true".into());
    }
    if let Some(p) = list.iter().find(|p| !p.is_absolute()) {
        return Err(format!("{} is not an absolute path", p.display()));
    }
    Ok(list)
}

fn single(d: &Dialog, g: &Geometry, path: &Path) -> Result<String, String> {
    // ctrl+l hides an entry that already has focus; then ctrl+a and the leading "/" reopen it.
    x("xdotool", &["key", "ctrl+l"])?;
    x("xdotool", &["key", "ctrl+a"])?;
    type_path(path)?;
    g.accept()?;
    closed(d, format!("Chose {}.{}", path.display(), WAIT))
}

fn several(d: &Dialog, g: &Geometry, paths: &[PathBuf]) -> Result<String, String> {
    if !bookmarks_ready() {
        return Err(BOOKMARKS_LATE.into());
    }
    let dir = stage(paths)?;
    bookmark(&dir)?;
    sleep(Duration::from_millis(500));
    x("xdotool", &["key", "alt+1"])?;
    sleep(Duration::from_millis(800));
    x("xdotool", &["key", "ctrl+a"])?;
    sleep(Duration::from_millis(200));
    g.accept()?;
    let names: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
    closed(d, format!("Chose {} files: {}.{}", paths.len(), names.join(", "), WAIT))
}

fn folder(d: &Dialog, g: &Geometry, dir: &Path) -> Result<String, String> {
    if !bookmarks_ready() {
        return Err(BOOKMARKS_LATE.into());
    }
    bookmark(dir)?;
    sleep(Duration::from_millis(500));
    x("xdotool", &["key", "alt+1"])?;
    sleep(Duration::from_millis(800));
    g.accept()?;
    closed(d, String::new())?;
    let n = count_files(dir);
    if n == 0 {
        return Ok(format!("Chose folder {} (it has no files).", dir.display()));
    }
    if confirm_upload(&d.page)? {
        return Ok(format!(
            "Chose folder {} and confirmed Chrome's \"Upload {n} files to this site?\".{WAIT}",
            dir.display()
        ));
    }
    Ok(format!(
        "Chose folder {}. Chrome should now ask \"Upload {n} files to this site?\" inside the window; bia could not confirm it: screen_shot, then screen_click its Upload button.{WAIT}",
        dir.display()
    ))
}

/// Chrome asks before a folder upload, inside the browser window (no X window). The
/// prompt takes focus from the page, so the page reporting no focus means it is up:
/// Tab from Cancel (focused) to Upload, Space; the page gets focus back once it closes.
/// False when the prompt was not seen (the caller hands it to the model).
fn confirm_upload(page: &str) -> Result<bool, String> {
    let focus = |ws: &str| cdp::eval(ws, "document.hasFocus()").ok().and_then(|v| v.as_bool());
    // the shown tab without focus (a background tab never has it)
    let asking = |ws: &str| {
        cdp::eval(ws, "document.visibilityState == 'visible' && !document.hasFocus()").ok().and_then(|v| v.as_bool())
            == Some(true)
    };
    // Just after the file dialog closes the page has no focus either: wait until the
    // browser window is active again, then for the page to stay unfocused.
    let deadline = Instant::now() + Duration::from_secs(3);
    while x("xdotool", &["getactivewindow", "getwindowname"]).map(|n| page_of(&n) != page).unwrap_or(true) {
        if Instant::now() > deadline {
            break;
        }
        sleep(Duration::from_millis(150));
    }
    // reading a large folder delays the prompt
    let deadline = Instant::now() + Duration::from_secs(15);
    let ws = 'found: loop {
        if let Some(ws) = cdp::tabs_titled(page).into_iter().find(|ws| asking(ws)) {
            let mut held = true;
            for _ in 0..3 {
                sleep(Duration::from_millis(250));
                held &= asking(&ws);
            }
            if held {
                break 'found ws;
            }
        }
        if Instant::now() > deadline {
            return Ok(false);
        }
        sleep(Duration::from_millis(300));
    };
    x("xdotool", &["key", "Tab", "space"])?;
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        sleep(Duration::from_millis(250));
        if focus(&ws) == Some(true) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Regular files under `dir` (what Chrome counts for its prompt), up to a cap.
fn count_files(dir: &Path) -> usize {
    let mut n = 0;
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            match e.file_type() {
                Ok(t) if t.is_dir() => todo.push(e.path()),
                Ok(t) if t.is_file() => n += 1,
                _ => {}
            }
            if n >= 100_000 {
                return n;
            }
        }
    }
    n
}

fn save_as(d: &Dialog, g: &Geometry, path: &Path) -> Result<String, String> {
    let parent = path.parent().filter(|p| p.is_dir()).ok_or_else(|| format!("{} has no existing folder", path.display()))?;
    if !bia_can("-w", parent) {
        return Err(format!("Chrome runs as bia and cannot write in {}", parent.display()));
    }
    x("xdotool", &["key", "ctrl+a"])?;
    type_path(path)?;
    g.accept()?;
    closed(d, format!("Saving to {}. A replace question may follow if it exists.", path.display()))
}

const WAIT: &str = " The site may still be uploading: wait until the page shows the file (or its progress ends) before sending the form.";

const BOOKMARKS_LATE: &str = "This dialog opened before bia prepared Chrome's file bookmarks. chrome_choose_files(cancel=true), click the upload button again, then retry.";

fn type_path(path: &Path) -> Result<(), String> {
    let s = path.to_str().ok_or("path is not UTF-8")?;
    x("xdotool", &["type", "--delay", "12", "--", s])?;
    sleep(Duration::from_millis(400));
    Ok(())
}

/// Wait for the dialog window to go away.
fn closed(d: &Dialog, ok: String) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(4);
    while Instant::now() < deadline {
        if !dialogs().iter().any(|o| o.id == d.id) {
            return Ok(ok);
        }
        sleep(Duration::from_millis(150));
    }
    Err(format!(
        "The file dialog \"{}\" is still open (a path may be wrong, or GTK asks something). screen_shot to see it; screen tools can finish, or chrome_choose_files(cancel=true).",
        d.title
    ))
}

struct Geometry {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

impl Geometry {
    fn of(id: &str) -> Result<Self, String> {
        let info = x("xwininfo", &["-id", id])?;
        let field = |name: &str| -> Result<i32, String> {
            info.lines()
                .find_map(|l| l.trim().strip_prefix(name))
                .and_then(|v| v.trim().parse().ok())
                .ok_or_else(|| format!("xwininfo: no {name}"))
        };
        Ok(Geometry {
            x: field("Absolute upper-left X:")?,
            y: field("Absolute upper-left Y:")?,
            w: field("Width:")?,
            h: field("Height:")?,
        })
    }

    /// Click the empty left of the button row (drops a completion popup), then the
    /// default button (Open, Save, Upload) at the bottom right.
    fn accept(&self) -> Result<(), String> {
        self.click(self.x + 15, self.y + self.h - 15)?;
        sleep(Duration::from_millis(300));
        self.click(self.x + self.w - 40, self.y + self.h - 26)
    }

    fn click(&self, cx: i32, cy: i32) -> Result<(), String> {
        x("xdotool", &["mousemove", "--sync", &cx.to_string(), &cy.to_string(), "click", "1"]).map(|_| ())
    }
}

/// Open GTK file dialogs, oldest first.
fn dialogs() -> Vec<Dialog> {
    scan(&mut HashSet::new())
}

/// Like `dialogs`, skipping ids in `others` and adding the non-dialog windows it finds.
fn scan(others: &mut HashSet<String>) -> Vec<Dialog> {
    let Ok(root) = x("xprop", &["-root", "_NET_CLIENT_LIST"]) else {
        return Vec::new();
    };
    let Some((_, ids)) = root.split_once('#') else {
        return Vec::new();
    };
    let ids: Vec<&str> = ids.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
    others.retain(|o| ids.contains(&o.as_str()));
    let mut out = Vec::new();
    for id in ids {
        if others.contains(id) {
            continue;
        }
        let Ok(props) = x("xprop", &["-id", id, "WM_WINDOW_ROLE", "_NET_WM_NAME", "WM_TRANSIENT_FOR"]) else {
            continue;
        };
        if prop_str(&props, "WM_WINDOW_ROLE").as_deref() != Some(ROLE) {
            others.insert(id.to_string());
            continue;
        }
        let page = props
            .lines()
            .find(|l| l.starts_with("WM_TRANSIENT_FOR"))
            .and_then(|l| l.rsplit_once('#'))
            .and_then(|(_, p)| x("xprop", &["-id", p.trim(), "_NET_WM_NAME"]).ok())
            .and_then(|p| prop_str(&p, "_NET_WM_NAME"))
            .map(|t| page_of(&t))
            .unwrap_or_default();
        out.push(Dialog {
            id: id.to_string(),
            title: prop_str(&props, "_NET_WM_NAME").unwrap_or_else(|| "File dialog".into()),
            page,
        });
    }
    out
}

/// `NAME(TYPE) = "value"` → value.
/// The page title in a browser window's title.
fn page_of(window: &str) -> String {
    let t = window.trim();
    t.strip_suffix(" - Google Chrome").or_else(|| t.strip_suffix(" - Chromium")).unwrap_or(t).to_string()
}

fn prop_str(props: &str, name: &str) -> Option<String> {
    let line = props.lines().find(|l| l.starts_with(&format!("{name}(")))?;
    let (_, v) = line.split_once(" = ")?;
    let v = v.trim();
    let v = v.strip_prefix('"').and_then(|v| v.strip_suffix('"')).unwrap_or(v);
    Some(v.replace("\\\"", "\"").replace("\\\\", "\\"))
}

fn bookmarks_path() -> PathBuf {
    Path::new(HOME).join(".config/gtk-3.0/bookmarks")
}

/// GTK reloads the bookmarks file live only when it existed as the dialog opened, so it is
/// created before Chrome starts.
pub fn prepare() {
    let path = bookmarks_path();
    if path.exists() {
        return;
    }
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if fs::write(&path, "").is_ok() {
        chown_bia(&Path::new(HOME).join(".config/gtk-3.0"));
        chown_bia(&path);
    }
}

fn bookmarks_ready() -> bool {
    let ready = bookmarks_path().exists();
    prepare();
    ready
}

/// Put `dir` first in GTK's bookmarks (alt+1), dropping bia's earlier entry.
fn bookmark(dir: &Path) -> Result<(), String> {
    let path = bookmarks_path();
    let old = fs::read_to_string(&path).unwrap_or_default();
    let mut text = format!("{} {LABEL}\n", file_uri(dir));
    for line in old.lines().filter(|l| !l.trim().is_empty() && !l.ends_with(&format!(" {LABEL}"))) {
        text.push_str(line);
        text.push('\n');
    }
    let tmp = path.with_extension("bia-tmp");
    fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    chown_bia(&tmp);
    fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))
}

fn file_uri(path: &Path) -> String {
    let mut out = String::from("file://");
    for b in path.to_string_lossy().bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Hard-link (or copy) the files into a fresh folder so ctrl+a selects exactly them.
/// Folders stay a day: sites may read a file only when the form is sent.
fn stage(paths: &[PathBuf]) -> Result<PathBuf, String> {
    let mut names = HashSet::new();
    for p in paths {
        let name = p.file_name().ok_or_else(|| format!("{} has no file name", p.display()))?;
        if !names.insert(name.to_os_string()) {
            return Err(format!("two files are named {}; the dialog needs distinct names", name.to_string_lossy()));
        }
    }
    let root = pick_root();
    prune(&root);
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let dir = root.join(stamp.to_string());
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for p in paths {
        let to = dir.join(p.file_name().unwrap_or_default());
        if fs::hard_link(p, &to).is_err() {
            fs::copy(p, &to).map_err(|e| format!("copy {}: {e}", p.display()))?;
            chown_bia(&to);
        }
    }
    for d in [root.parent().unwrap_or(&root), &root, &dir] {
        chown_bia(d);
    }
    Ok(dir)
}

fn pick_root() -> PathBuf {
    let files = std::env::var("BIA_FILES").unwrap_or_else(|_| format!("{HOME}/files"));
    PathBuf::from(files).join("state").join("chrome").join("pick")
}

fn prune(root: &Path) {
    let Ok(rd) = fs::read_dir(root) else { return };
    for e in rd.flatten() {
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > KEEP);
        if old {
            let _ = fs::remove_dir_all(e.path());
        }
    }
}

fn is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

/// `test <flag> path` as the user Chrome runs as.
fn bia_can(flag: &str, path: &Path) -> bool {
    let mut cmd = if is_root() {
        let mut c = Command::new("runuser");
        c.args(["-u", "bia", "--", "test"]);
        c
    } else {
        Command::new("test")
    };
    cmd.arg(flag).arg(path).status().map(|s| s.success()).unwrap_or(false)
}

fn chown_bia(path: &Path) {
    if is_root() {
        let _ = Command::new("chown").arg("bia:bia").arg(path).status();
    }
}

fn x(bin: &str, args: &[&str]) -> Result<String, String> {
    let display = std::env::var("DISPLAY").unwrap_or_else(|_| proc::DISPLAY.into());
    let xauth = std::env::var("XAUTHORITY").unwrap_or_else(|_| {
        let home = format!("{HOME}/.Xauthority");
        if Path::new(&home).is_file() { home } else { "/run/bia/Xauthority".into() }
    });
    let out = Command::new(bin)
        .args(args)
        .env("DISPLAY", display)
        .env("XAUTHORITY", xauth)
        .output()
        .map_err(|e| format!("{bin}: {e}"))?;
    if !out.status.success() {
        return Err(format!("{bin} failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prop_values() {
        let p = "WM_WINDOW_ROLE(STRING) = \"GtkFileChooserDialog\"\n_NET_WM_NAME(UTF8_STRING) = \"Open \\\"x\\\" Files\"\nWM_TRANSIENT_FOR(WINDOW): window id # 0x400002\n";
        assert_eq!(prop_str(p, "WM_WINDOW_ROLE").as_deref(), Some(ROLE));
        assert_eq!(prop_str(p, "_NET_WM_NAME").as_deref(), Some("Open \"x\" Files"));
        assert_eq!(prop_str(p, "WM_CLASS"), None);
    }

    #[test]
    fn page_titles() {
        assert_eq!(page_of("Upload - Google Chrome"), "Upload");
        assert_eq!(page_of("m:a.txt - Chromium\n"), "m:a.txt");
        assert_eq!(page_of("Plain"), "Plain");
    }

    #[test]
    fn uri_escapes() {
        assert_eq!(file_uri(Path::new("/home/bia/a b/ü.txt")), "file:///home/bia/a%20b/%C3%BC.txt");
    }

    /// Needs X on $DISPLAY with one file dialog open; run by tests/integration/chrome-dialog-test.sh.
    #[test]
    #[ignore]
    fn seen_from_env() {
        let mut seen = Seen::default();
        seen.poll(None);
        seen.poll(None);
        println!("seen: {} dialog(s), {} other window(s), {:?}", seen.dialogs.len(), seen.others.len(), dialogs().first().map(notice));
        assert_eq!(seen.dialogs.len(), 1);
        assert!(!seen.others.is_empty());
    }

    /// Needs X on $DISPLAY with Chrome showing a file dialog; run by
    /// tests/integration/chrome-dialog-test.sh.
    #[test]
    #[ignore]
    fn choose_from_env() {
        let paths: Vec<String> = std::env::var("BIA_DIALOG_PATHS").unwrap().split(':').map(String::from).collect();
        let cancel = std::env::var("BIA_DIALOG_CANCEL").is_ok();
        let r = choose(&serde_json::json!({"paths": paths, "cancel": cancel}));
        println!("choose: {r:?}");
        r.unwrap();
    }

    /// Prints what the folder-upload confirmation sees over CDP (BIA_CDP_TITLE = tab title).
    #[test]
    #[ignore]
    fn cdp_from_env() {
        let title = std::env::var("BIA_CDP_TITLE").unwrap_or_default();
        println!("cdp: list {:?}", crate::watch::json_list().map(|v| v.to_string()));
        for ws in cdp::tabs_titled(&title) {
            println!("cdp: {ws} focus {:?}", cdp::eval(&ws, "[document.visibilityState, document.hasFocus()]"));
        }
    }
}
