//! Whole-file JSON state (files/state/*.json) that must not be lost to a broken file.
//!
//! Every `write` replaces the file atomically (tmp, fsync, rename; keeps its mode and owner) and
//! then writes the same bytes to `<name>.bak`, the last good copy. `read` never treats a file
//! that exists but doesn't parse as "nothing saved": that file is moved aside as
//! `<name>.bad-<unix>`, the `.bak` is used (and written back), and a line goes to
//! `repairs.log` in the state folder plus stderr, so bia can tell root. Before, a truncated or
//! overwritten file loaded as empty and the next save erased what was there.
//!
//! Only a missing file counts as a first start (stores delete their files on purpose, e.g.
//! on logout; `remove` deletes the copy too). `read_restoring` also restores a missing file
//! from `.bak`, for stores that never delete theirs.

use serde::de::DeserializeOwned;
use serde::Serialize;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const REPAIRS_LOG: &str = "repairs.log";

pub fn backup_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".bak");
    PathBuf::from(s)
}

/// The parsed file; None when it is missing (or broken with no good copy).
pub fn read<T: DeserializeOwned>(path: &Path) -> Option<T> {
    load(path, false)
}

/// Like `read`, and a missing file comes back from its `.bak` too.
pub fn read_restoring<T: DeserializeOwned>(path: &Path) -> Option<T> {
    load(path, true)
}

pub fn read_or_default<T: DeserializeOwned + Default>(path: &Path) -> T {
    read(path).unwrap_or_default()
}

pub fn write_json<T: Serialize>(path: &Path, v: &T) -> std::io::Result<()> {
    write(path, &serde_json::to_vec_pretty(v).map_err(std::io::Error::other)?)
}

/// Replace the file atomically, then refresh its `.bak`.
pub fn write(path: &Path, body: &[u8]) -> std::io::Result<()> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    replace(path, body, None)?;
    // the copy gets the file's own mode and owner (passwords.json stays 0600)
    let _ = replace(&backup_path(path), body, std::fs::metadata(path).ok());
    Ok(())
}

/// Refresh the `.bak` from the file as it is now (for stores that write the file themselves).
pub fn backup(path: &Path) -> std::io::Result<()> {
    let body = std::fs::read(path)?;
    replace(&backup_path(path), &body, std::fs::metadata(path).ok())
}

/// Delete the file and its copy (a store that is reset on purpose).
pub fn remove(path: &Path) -> std::io::Result<()> {
    let _ = std::fs::remove_file(backup_path(path));
    match std::fs::remove_file(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        r => r,
    }
}

fn replace(path: &Path, body: &[u8], like: Option<std::fs::Metadata>) -> std::io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let old = like.or_else(|| std::fs::metadata(path).ok());
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(body)?;
        f.sync_all()?;
    }
    #[cfg(unix)]
    if let Some(m) = &old {
        // MCPs often run as root and write bia's files: keep the owner and mode it had
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(m.mode() & 0o7777));
        let _ = std::os::unix::fs::chown(&tmp, Some(m.uid()), Some(m.gid()));
    }
    #[cfg(not(unix))]
    let _ = old;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    #[cfg(unix)]
    if let Some(dir) = path.parent().and_then(|d| std::fs::File::open(d).ok()) {
        let _ = dir.sync_all();
    }
    Ok(())
}

fn parse<T: DeserializeOwned>(p: &Path) -> Result<Option<T>, String> {
    match std::fs::read(p) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
        Ok(b) if b.iter().all(|c| c.is_ascii_whitespace()) => Err("the file is empty".into()),
        Ok(b) => serde_json::from_slice(&b).map(Some).map_err(|e| e.to_string()),
    }
}

fn load<T: DeserializeOwned>(path: &Path, restore_missing: bool) -> Option<T> {
    let why = match parse::<T>(path) {
        Ok(Some(v)) => return Some(v),
        Ok(None) if !restore_missing => return None,
        Ok(None) => None,
        // permission or I/O trouble: leave the file alone, it may read fine next time
        Err(e) if path.exists() && std::fs::read(path).is_err() => {
            eprintln!("{}: {e}", path.display());
            return None;
        }
        Err(e) => Some(e),
    };
    let mut what = match &why {
        Some(e) => {
            let t = now();
            let aside = (0..)
                .map(|n| PathBuf::from(if n == 0 { format!("{}.bad-{t}", path.display()) } else { format!("{}.bad-{t}-{n}", path.display()) }))
                .find(|p| !p.exists())
                .unwrap_or_default();
            let kept = std::fs::rename(path, &aside).is_ok();
            format!(
                "{} could not be read ({e}){}",
                path.display(),
                if kept { format!(", kept as {}", aside.display()) } else { String::new() }
            )
        }
        None => format!("{} was missing", path.display()),
    };
    let bak = backup_path(path);
    let restored = match parse::<T>(&bak) {
        Ok(Some(v)) => {
            // put the good copy back so the next save starts from it
            let _ = std::fs::read(&bak).map(|b| replace(path, &b, std::fs::metadata(&bak).ok()));
            what.push_str(&format!("; restored from {}", bak.display()));
            Some(v)
        }
        _ if why.is_none() => return None, // missing, and nothing to restore: a first start
        _ => {
            what.push_str("; no good copy, started empty");
            None
        }
    };
    note(path, &what);
    restored
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// The `state` folder above the file (else its own folder).
fn state_dir(path: &Path) -> PathBuf {
    path.ancestors()
        .skip(1)
        .find(|a| a.file_name().is_some_and(|n| n == "state"))
        .or(path.parent())
        .map(Path::to_path_buf)
        .unwrap_or_default()
}

fn note(path: &Path, what: &str) {
    eprintln!("state: {what}");
    let log = state_dir(path).join(REPAIRS_LOG);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&log) {
        let _ = writeln!(f, "{} {}", now(), what.replace('\n', " "));
    }
}

/// repairs.log lines after `offset` bytes → (lines, new offset). For the agent's notices.
pub fn repairs_since(state: &Path, offset: u64) -> (Vec<String>, u64) {
    let Ok(b) = std::fs::read(state.join(REPAIRS_LOG)) else { return (vec![], 0) };
    let len = b.len() as u64;
    let from = if offset > len { 0 } else { offset as usize };
    let text = String::from_utf8_lossy(&b[from..]);
    let lines = text
        .lines()
        .filter_map(|l| l.split_once(' ').map(|(_, w)| w.trim().to_string()))
        .filter(|l| !l.is_empty())
        .collect();
    (lines, len)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Serialize, Deserialize, Default, Debug, PartialEq)]
    struct Book {
        items: Vec<String>,
    }

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bia-statefile-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("state")).unwrap();
        d
    }

    fn bad_files(d: &Path) -> usize {
        std::fs::read_dir(d).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().contains(".bad-")).count()
    }

    #[test]
    fn a_broken_file_comes_back_from_the_copy() {
        let d = dir("broken");
        let p = d.join("state/book.json");
        assert_eq!(read::<Book>(&p), None);
        write_json(&p, &Book { items: vec!["a".into(), "b".into()] }).unwrap();
        assert!(backup_path(&p).is_file());

        for junk in [&br#"{"NewTabPage":{},"profile":{"exit_type":"Normal"}}"#[..], b"", b"{\"items\":[\"a\""] {
            std::fs::write(&p, junk).unwrap();
            let b: Book = read(&p).unwrap();
            assert_eq!(b.items, ["a", "b"]);
            assert_eq!(read::<Book>(&p).unwrap().items.len(), 2, "written back");
        }
        assert_eq!(bad_files(&d.join("state")), 3);
        let (lines, off) = repairs_since(&d.join("state"), 0);
        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains("could not be read") && lines[0].contains("restored from"), "{}", lines[0]);
        assert_eq!(repairs_since(&d.join("state"), off).0.len(), 0);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn missing_is_a_first_start_unless_restoring() {
        let d = dir("missing");
        let p = d.join("state/book.json");
        write_json(&p, &Book { items: vec!["x".into()] }).unwrap();
        std::fs::remove_file(&p).unwrap();
        assert_eq!(read::<Book>(&p), None);
        assert_eq!(read_restoring::<Book>(&p).unwrap().items, ["x"]);
        assert!(p.is_file());
        remove(&p).unwrap();
        assert!(!p.exists() && !backup_path(&p).exists());
        assert_eq!(read_restoring::<Book>(&p), None);
        assert!(repairs_since(&d.join("state"), 0).0.len() == 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn broken_with_no_copy_is_kept_aside() {
        let d = dir("nocopy");
        let p = d.join("state/sub/book.json");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, b"garbage").unwrap();
        assert_eq!(read::<Book>(&p), None);
        assert!(!p.exists());
        assert_eq!(bad_files(p.parent().unwrap()), 1);
        let (lines, _) = repairs_since(&d.join("state"), 0); // logged in state/, not state/sub/
        assert!(lines[0].contains("no good copy"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[cfg(unix)]
    #[test]
    fn keeps_mode() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let d = dir("mode");
        let p = d.join("state/secret.json");
        write_json(&p, &Book::default()).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
        write_json(&p, &Book { items: vec!["s".into()] }).unwrap();
        assert_eq!(std::fs::metadata(&p).unwrap().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(backup_path(&p)).unwrap().mode() & 0o777, 0o600);
        let _ = std::fs::remove_dir_all(&d);
    }
}
