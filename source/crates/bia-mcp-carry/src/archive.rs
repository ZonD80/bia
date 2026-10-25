//! Fold this machine's choices into a gzip tar. Unfold applies that archive on any architecture.
use bia_mcp::Ctx;
use serde_json::{json, Value};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const HOME: &str = "/home/bia";

const STATE_SKIP: &[&str] = &["boot_notice", "last_exit", "screen", "chrome", "dl", "hitl"];
const MCP_ARCHS: &[&str] = &["aarch64", "x86_64"];
/// Fold + markers for OS update/rescue/morph — outside /home/bia so the OS copy never rewrites it.
const UPDATE_DIR: &str = "/bia-update";
const CHROME_SKIP: &[&str] = &[
    "Cache",
    "Code Cache",
    "GPUCache",
    "ShaderCache",
    "GrShaderCache",
    "Crash Reports",
    "BrowserMetrics",
    "Safe Browsing",
    "optimization_guide_hint_cache",
    "optimization_guide_model_store",
    "GraphiteDawnCache",
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Restore,
    Land,
    /// Same machine after its OS was replaced by a newer image. Keeps identity, skips mcp.json.
    Update,
}

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Where the uncompressed copy of a fold/unfold lives: next to the archive (same disk,
/// e.g. /bia-update), or /var/tmp when the archive sits inside the home being folded.
/// Never /tmp: on Debian it is RAM, and the copy can be as large as the model.
fn scratch_dir(near: &Path, home: &Path, tag: &str) -> PathBuf {
    let name = format!(".bia-carry-{tag}-{}", std::process::id());
    if near.as_os_str().is_empty() || near.starts_with(home) {
        Path::new("/var/tmp").join(name)
    } else {
        near.join(name)
    }
}

fn files_root() -> PathBuf {
    PathBuf::from(std::env::var("BIA_FILES").unwrap_or_else(|_| format!("{HOME}/files")))
}

/// Fold and unfold read and write all of /home/bia, /etc and dpkg, root's 0600 files too;
/// as another user tar would leave those out without a word.
fn require_root(what: &str) -> Result<(), String> {
    if unsafe { libc::geteuid() } == 0 {
        Ok(())
    } else {
        Err(format!("{what} must run as root: another user cannot read every file under /home/bia. As user bia (schedules by default) run it with sudo."))
    }
}

fn rel_ok(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && !path.is_absolute()
        && path.components().all(|c| matches!(c, Component::Normal(_)))
}

pub fn fold(ctx: &Ctx, path: Option<&str>, force: bool) -> Result<String, String> {
    let dest = match path.map(str::trim).filter(|s| !s.is_empty()) {
        Some(p) => under_files(Path::new(p))?,
        None => {
            let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            under_files(Path::new(&format!("carry/fold-{secs}.tar.gz")))?
        }
    };
    let packed = fold_shared(ctx, Path::new("/"), &dest, force, true, Some("carry_fold"))?;
    ctx.log(format!("fold: {}", packed.path.display()));
    Ok(format!(
        "{}\n{} bytes gzip, {} bytes when unfolded\nThe archive holds /home/bia, including keys, passwords, and sessions.",
        packed.path.display(),
        packed.gzip_len,
        packed.unpacked
    ))
}

/// Write an archive at an absolute path (used when flying; the file is not a user backup).
pub fn fold_into(ctx: &Ctx, dest: &Path) -> Result<u64, String> {
    Ok(fold_shared(ctx, Path::new("/"), dest, false, false, None)?.gzip_len)
}

pub fn unfold(ctx: &Ctx, path: &str, mode: Mode, force: bool) -> Result<String, String> {
    let src = under_files(Path::new(path.trim()))?;
    if !force {
        let Some(need) = unfolded_size(&src) else {
            return Err("this is an older backup, so the unpacked size is not in the file name and free space cannot be checked. It can still be unfolded. Ask the user to confirm, then call carry_unfold again with force=true only after they confirm.".into());
        };
        let home = Path::new(HOME);
        if !home.exists() {
            fs::create_dir_all(home).map_err(|e| e.to_string())?;
        }
        let near = src.parent().unwrap_or_else(|| Path::new("/var/tmp"));
        guard_space("unfold", need, &scratch_dir(near, home, "out"), home, false, Some("carry_unfold"))?;
    }
    unfold_file(ctx, &src, mode)
}

pub fn finish_update(ctx: &Ctx, tar: &Path, stash: &Path) -> Result<String, String> {
    unfold_inner(ctx, tar, Mode::Update, Some(stash))
}

pub fn unfold_file(ctx: &Ctx, src: &Path, mode: Mode) -> Result<String, String> {
    unfold_inner(ctx, src, mode, None)
}

fn unfold_inner(ctx: &Ctx, src: &Path, mode: Mode, stash: Option<&Path>) -> Result<String, String> {
    require_root("unfold")?;
    let len = fs::metadata(src).map(|m| m.len()).map_err(|e| format!("{}: {e}", src.display()))?;
    ctx.log(format!("unfold: reading {} ({} bytes)", src.display(), len));
    let near = src.parent().unwrap_or_else(|| Path::new("/var/tmp"));
    let stage = Scratch(scratch_dir(near, Path::new(HOME), "out"));
    let _ = fs::remove_dir_all(&stage.0);
    fs::create_dir_all(&stage.0).map_err(|e| e.to_string())?;
    tar_extract(src, &stage.0)?;
    let manifest: Value = {
        let p = stage.0.join("manifest.json");
        let bytes = fs::read(&p).map_err(|e| format!("manifest.json: {e}"))?;
        serde_json::from_slice(&bytes).map_err(|e| format!("manifest.json: {e}"))?
    };
    if manifest.get("format").and_then(|v| v.as_str()) != Some("bia-carry") {
        return Err("not a carry archive".into());
    }
    let packages = manifest_packages(&manifest);
    ctx.log(format!("unfold: {} manually installed packages", packages.len()));
    if mode == Mode::Update {
        let etc = stage.0.join("etc");
        if etc.is_dir() {
            apply_etc(&etc, Mode::Update)?;
            ctx.log("unfold: restored this machine's network before installing packages");
            let _ = Command::new("ifup").args(["-a"]).status();
        }
    }
    let debs_dir = {
        let staged = stage.0.join(crate::debs::STORE_REL);
        if staged.is_dir() {
            staged
        } else {
            Path::new("/").join(crate::debs::STORE_REL)
        }
    };
    let failed_names = apt_install(ctx, &packages, &debs_dir);
    let failed: Vec<String> = failed_names
        .iter()
        .map(|n| {
            packages
                .iter()
                .find(|p| &p.name == n)
                .map(pkg_label)
                .unwrap_or_else(|| n.clone())
        })
        .collect();
    ctx.log("unfold: copying files");
    apply_tree(ctx, &stage.0, mode)?;
    if mode == Mode::Update {
        if let Some(dir) = stash {
            restore_stashed_mcps(ctx, dir)?;
        }
    }
    let host = rewrite_native_host();
    ctx.log(&host);
    let _ = Command::new("chown")
        .args(["-R", "bia:bia", HOME])
        .status();
    let mut missing_bins = Vec::new();
    if mode != Mode::Update {
        let staged = stage.0.join("mcps");
        let live = PathBuf::from(format!("{HOME}/MCPs"));
        for miss in copy_arch_bins(&staged, &live) {
            ctx.log(format!("unfold: {miss}"));
            missing_bins.push(miss);
        }
    }
    for gap in missing_mcp_bins(&PathBuf::from(format!("{HOME}/MCPs"))) {
        ctx.log(&gap);
        if !missing_bins.iter().any(|s| s == &gap) {
            missing_bins.push(gap);
        }
    }
    let custom = if mode == Mode::Update {
        fs::read_to_string(Path::new(UPDATE_DIR).join("lead"))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| s.contains("this boot restored"))
    } else {
        None
    };
    let base = custom.unwrap_or_else(|| match mode {
        Mode::Land => "this boot landed from another machine".into(),
        Mode::Restore => "this boot restored from a carry archive".into(),
        Mode::Update => "this boot restored this machine after updating from a newer image".into(),
    });
    let base = if mode == Mode::Update {
        format!("{base} (log: /bia-update/update.log)")
    } else {
        base
    };
    let notice = boot_notice(&base, &failed, &missing_bins);
    ctx.log(&notice);
    let _ = std::fs::remove_file("/etc/bia-land-mode");
    // The extracted tree can be large and may sit in /bia-update; remove it while
    // nothing is racing the shutdown.
    drop(stage);
    if mode == Mode::Land {
        // The flown-in archive sits on the disk (/var/tmp), not in RAM.
        let _ = fs::remove_file(src);
    }
    if mode == Mode::Update {
        // The agent is restored: drop the fold and the stash now (a delete after the
        // reboot is spawned races shutdown and left /bia-update behind), and turn
        // pending into done. bia-update removes the rest on the next boot.
        let dir = Path::new(UPDATE_DIR);
        let _ = fs::remove_file(src);
        if let Some(stash) = stash {
            let _ = fs::remove_dir_all(stash);
        }
        let _ = fs::rename(dir.join("pending"), dir.join("done"));
        ctx.log("unfold: update finished; /bia-update (except update.log) is removed on the next boot");
    }
    ctx.log("unfold: rebooting");
    reboot(&notice)?;
    Ok(notice)
}

pub fn fold_root(ctx: &Ctx, root: &Path, dest: &Path) -> Result<u64, String> {
    ctx.log(format!("fold: agent on {}", root.display()));
    Ok(fold_shared(ctx, root, dest, false, false, None)?.gzip_len)
}

struct Packed {
    path: PathBuf,
    gzip_len: u64,
    unpacked: u64,
}

fn fold_shared(ctx: &Ctx, base: &Path, dest: &Path, force: bool, stamp: bool, tool: Option<&str>) -> Result<Packed, String> {
    require_root("fold")?;
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let home = base.join("home/bia");
    let dest_dir = dest.parent().unwrap_or_else(|| Path::new("/"));
    if home.is_dir() {
        // Only the live system can ask apt which installed versions a repository has;
        // for another root (rescue folds the disk from the flash) keep what is there.
        let live = base == Path::new("/");
        let mut manual = manual_at(base);
        let mut from_repo = std::collections::HashMap::new();
        if live {
            attach_origins(&mut manual);
            let names: Vec<String> = crate::debs::list(&base.join(crate::debs::STORE_REL))
                .into_iter()
                .map(|d| d.name)
                .collect();
            if !names.is_empty() {
                if let Ok(out) = Command::new("apt-cache").arg("policy").args(&names).output() {
                    from_repo = origins_from_policy(&String::from_utf8_lossy(&out.stdout));
                }
            }
        }
        let custom: Vec<(String, String, String)> = manual
            .into_iter()
            .filter(|p| live && p.origin.is_empty())
            .map(|p| (p.name, p.version, p.arch))
            .collect();
        crate::debs::refresh(ctx, base, &custom, &from_repo);
    }
    ctx.log(format!("fold: listing {}", home.display()));
    let mut members = Members::default();
    if home.is_dir() {
        members.walk(base, Path::new("home/bia"), Some(dest), &|_| true)?;
    }
    ctx.log("fold: listing /etc selections");
    etc_members(base, &mut members)?;
    ctx.log(format!("fold: {} files, {} bytes", members.files, members.bytes));
    if !force && members.bytes > 0 {
        space_short("fold", &dir_ready(dest_dir)?, members.bytes, 1, tool, "")?;
    }
    // Only the manifest is written outside the archive; everything else streams from `base`.
    let meta = Scratch(scratch_dir(dest_dir, &home, "meta"));
    let _ = fs::remove_dir_all(&meta.0);
    fs::create_dir_all(&meta.0).map_err(|e| e.to_string())?;
    ctx.log("fold: recording packages and MCP binaries");
    let manifest_len = write_manifest(&meta.0.join("manifest.json"), base)?;
    let list = meta.0.join("members");
    members.write_list(&list)?;
    let unpacked = members.bytes + manifest_len;
    ctx.log(format!("fold: compressing {unpacked} bytes into {}", dest.display()));
    tar_create(&meta.0, base, &list, dest)?;
    let path = if stamp {
        let named = named_with_unfolded(dest, unpacked);
        if named != dest {
            let _ = fs::remove_file(&named);
            fs::rename(dest, &named).map_err(|e| format!("{}: {e}", named.display()))?;
        }
        named
    } else {
        dest.to_path_buf()
    };
    let gzip_len = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    ctx.log(format!("fold: done — {gzip_len} bytes gzip, {unpacked} bytes when unfolded → {}", path.display()));
    Ok(Packed { path, gzip_len, unpacked })
}

/// Archive members as paths relative to the folded root (`home/bia/…`, `etc/…`).
/// Directories are listed too, so tar runs with --no-recursion and empty folders survive.
#[derive(Default)]
struct Members {
    list: Vec<PathBuf>,
    files: u64,
    bytes: u64,
}

impl Members {
    /// Add `rel` (a directory under `base`) and everything below it. `keep` sees the
    /// root-relative path of each regular file; `exclude` (the archive itself) is skipped.
    fn walk(&mut self, base: &Path, rel: &Path, exclude: Option<&Path>, keep: &dyn Fn(&str) -> bool) -> Result<(), String> {
        self.list.push(rel.to_path_buf());
        let mut stack = vec![rel.to_path_buf()];
        while let Some(cur) = stack.pop() {
            let dir = base.join(&cur);
            let rd = fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            for ent in rd {
                let ent = ent.map_err(|e| e.to_string())?;
                let name = ent.file_name();
                if !rel_ok(Path::new(&name)) {
                    continue;
                }
                let from = ent.path();
                if exclude.is_some_and(|p| same_file(&from, p)) {
                    continue;
                }
                let r = cur.join(&name);
                let meta = fs::symlink_metadata(&from).map_err(|e| e.to_string())?;
                if meta.is_dir() {
                    self.list.push(r.clone());
                    stack.push(r);
                } else if meta.file_type().is_symlink() {
                    self.list.push(r);
                } else if meta.is_file() && keep(&r.to_string_lossy()) {
                    self.files += 1;
                    self.bytes += meta.len();
                    self.list.push(r);
                }
            }
        }
        Ok(())
    }

    fn file(&mut self, base: &Path, rel: &str) {
        if let Ok(m) = fs::metadata(base.join(rel)) {
            self.files += 1;
            self.bytes += m.len();
            self.list.push(PathBuf::from(rel));
        }
    }

    fn write_list(&self, path: &Path) -> Result<(), String> {
        use std::os::unix::ffi::OsStrExt;
        let mut buf = Vec::new();
        for p in &self.list {
            buf.extend_from_slice(p.as_os_str().as_bytes());
            buf.push(0);
        }
        fs::write(path, buf).map_err(|e| format!("{}: {e}", path.display()))
    }
}

fn write_manifest(path: &Path, base: &Path) -> Result<u64, String> {
    let mcps = base.join("home/bia/MCPs");
    let manifest = json!({
        "format": "bia-carry",
        "version": 1,
        "source_arch": arch_at(base),
        "debian": debian_codename_at(base),
        "packages": package_records_at(base),
        "mcp_bins": mcp_bins_at(&mcps),
    });
    let bytes = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;
    fs::write(path, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(bytes.len() as u64)
}
fn same_file(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y)
}

fn text_file_busy(err: &std::io::Error) -> bool {
    matches!(
        err.raw_os_error(),
        Some(code) if code == libc::ETXTBSY as i32 || code == libc::EBUSY as i32
    ) || {
        let s = err.to_string().to_ascii_lowercase();
        s.contains("text file busy") || s.contains("device or resource busy")
    }
}

/// Copy one file. If the destination is open or a running binary, unlink it first
/// (the process keeps the old inode) and copy again. Skip only if that still fails.
fn copy_file(ctx: Option<&Ctx>, from: &Path, to: &Path) -> Result<(), String> {
    match fs::copy(from, to) {
        Ok(_) => return Ok(()),
        Err(e) if text_file_busy(&e) => {
            log_busy(ctx, &format!("replace busy {} ({e})", to.display()));
        }
        Err(e) => return Err(format!("{}: {e}", to.display())),
    }
    match fs::remove_file(to) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            log_busy(ctx, &format!("skip busy {} (could not unlink: {e})", to.display()));
            return Ok(());
        }
    }
    match fs::copy(from, to) {
        Ok(_) => {
            log_busy(ctx, &format!("replaced {}", to.display()));
            Ok(())
        }
        Err(e) if text_file_busy(&e) => {
            log_busy(ctx, &format!("skip busy {} after unlink ({e})", to.display()));
            Ok(())
        }
        Err(e) => Err(format!("{}: {e}", to.display())),
    }
}

fn log_busy(ctx: Option<&Ctx>, msg: &str) {
    if let Some(ctx) = ctx {
        ctx.log(format!("unfold: {msg}"));
    } else {
        eprintln!("unfold: {msg}");
    }
}

/// Copy a tree, keeping symlinks as symlinks. `exclude` is one absolute file to leave out
/// (the archive being written, when it already sits under the home directory).
/// `expect_bytes` is the measured source size for progress (0 skips percent).
fn copy_any(ctx: &Ctx, label: &str, src: &Path, dst: &Path, exclude: Option<&Path>, expect_bytes: u64) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| format!("{}: {e}", dst.display()))?;
    let mut stack = vec![(src.to_path_buf(), dst.to_path_buf(), true)];
    let mut files = 0u64;
    let mut bytes = 0u64;
    let mut last_report = 0u64;
    while let Some((cur, dest, top)) = stack.pop() {
        if top && cur != src {
            if let Ok(rel) = cur.strip_prefix(src) {
                ctx.log(format!("{label}:   {}", rel.display()));
            }
        }
        let rd = fs::read_dir(&cur).map_err(|e| format!("{}: {e}", cur.display()))?;
        let mut kids: Vec<_> = rd.collect();
        // Process later entries first so pop() walks left-to-right for top-level names.
        kids.reverse();
        for ent in kids {
            let ent = ent.map_err(|e| e.to_string())?;
            let name = ent.file_name();
            let name_s = name.to_string_lossy();
            if !rel_ok(Path::new(&*name_s)) {
                continue;
            }
            let from = ent.path();
            if exclude.is_some_and(|p| same_file(&from, p)) {
                ctx.log(format!("{label}:   skip {}", from.display()));
                continue;
            }
            let to = dest.join(&name);
            let meta = fs::symlink_metadata(&from).map_err(|e| e.to_string())?;
            if meta.file_type().is_symlink() {
                let target = fs::read_link(&from).map_err(|e| e.to_string())?;
                let _ = fs::remove_file(&to);
                std::os::unix::fs::symlink(&target, &to).map_err(|e| format!("{}: {e}", to.display()))?;
                continue;
            }
            if meta.is_dir() {
                fs::create_dir_all(&to).map_err(|e| e.to_string())?;
                stack.push((from, to, cur == src));
            } else if meta.is_file() {
                if let Some(parent) = to.parent() {
                    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                let n = meta.len();
                if n >= 32 * 1024 * 1024 {
                    if let Ok(rel) = from.strip_prefix(src) {
                        ctx.log(format!("{label}:   copy {} ({n} bytes)", rel.display()));
                    } else {
                        ctx.log(format!("{label}:   copy {} ({n} bytes)", from.display()));
                    }
                }
                copy_file(Some(ctx), &from, &to)?;
                files = files.saturating_add(1);
                bytes = bytes.saturating_add(n);
                if bytes.saturating_sub(last_report) >= 32 * 1024 * 1024 {
                    last_report = bytes;
                    if expect_bytes > 0 {
                        let pct = ((bytes.min(expect_bytes) * 100) / expect_bytes) as u32;
                        ctx.progress(pct, format!("{label}: {files} files, {bytes} / {expect_bytes} bytes"));
                    } else {
                        ctx.log(format!("{label}: {files} files, {bytes} bytes"));
                    }
                }
            }
        }
    }
    if expect_bytes > 0 {
        ctx.progress(100, format!("{label}: {files} files, {bytes} bytes packed"));
    } else {
        ctx.log(format!("{label}: {files} files, {bytes} bytes"));
    }
    Ok(())
}

/// Live MCP programs already on this machine (`bin`, `mcp.json`, `native-host`).
/// Unfold/update restore `/home/bia` from an archive over them, then put these back so
/// a backup cannot replace newer system binaries that already exist.
fn keep_image_mcp_programs() -> Result<Scratch, String> {
    let dir = std::env::temp_dir().join(format!("bia-mcp-keep-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mcps = PathBuf::from(format!("{HOME}/MCPs"));
    if let Ok(rd) = fs::read_dir(&mcps) {
        for ent in rd.flatten() {
            if !ent.path().is_dir() {
                continue;
            }
            let name = ent.file_name();
            let name_s = name.to_string_lossy();
            if !rel_ok(Path::new(&*name_s)) {
                continue;
            }
            let dst = dir.join(&name);
            for keep in ["bin", "mcp.json", "native-host"] {
                let from = ent.path().join(keep);
                if !from.exists() {
                    continue;
                }
                let to = dst.join(keep);
                if from.is_dir() {
                    copy_tree(None, &from, &to, &|_| false)?;
                } else if from.is_file() {
                    fs::create_dir_all(&dst).map_err(|e| e.to_string())?;
                    copy_file(None, &from, &to)?;
                }
            }
        }
    }
    Ok(Scratch(dir))
}

fn put_back_mcp_programs(ctx: &Ctx, stash: &Path) -> Result<(), String> {
    let dest_root = PathBuf::from(format!("{HOME}/MCPs"));
    let Ok(rd) = fs::read_dir(stash) else {
        return Ok(());
    };
    for ent in rd.flatten() {
        if !ent.path().is_dir() {
            continue;
        }
        let name = ent.file_name();
        let dst = dest_root.join(&name);
        fs::create_dir_all(&dst).map_err(|e| e.to_string())?;
        for keep in ["bin", "mcp.json", "native-host"] {
            let from = ent.path().join(keep);
            if !from.exists() {
                continue;
            }
            let to = dst.join(keep);
            if from.is_dir() {
                let _ = fs::remove_dir_all(&to);
                copy_tree(Some(ctx), &from, &to, &|_| false)?;
            } else if from.is_file() {
                copy_file(Some(ctx), &from, &to)?;
            }
        }
    }
    Ok(())
}
/// This machine's choices under /etc: network, identity, Chrome policy, local units and
/// SSH host keys. Files the image itself ships (`/etc/bia-shipped`) are left out.
fn etc_members(base: &Path, m: &mut Members) -> Result<(), String> {
    let shipped = shipped_set_at(base);
    let specs = [
        "timezone",
        "hostname",
        "machine-id",
        "network/interfaces",
        "network/interfaces.d",
        "wpa_supplicant.conf",
        "wpa_supplicant",
        "opt/chrome",
        "opt/google/chrome",
        "systemd/system",
    ];
    for rel in specs {
        let rel = format!("etc/{rel}");
        let src = base.join(&rel);
        if src.is_dir() {
            let prune = rel == "etc/systemd/system";
            m.walk(base, Path::new(&rel), None, &|r| !(prune && shipped.contains(r)))?;
        } else if src.exists() && !shipped.contains(&rel) {
            m.file(base, &rel);
        }
    }
    if let Ok(rd) = fs::read_dir(base.join("etc/ssh")) {
        for ent in rd.flatten() {
            let name = ent.file_name().to_string_lossy().to_string();
            if name.starts_with("ssh_host_") && ent.path().is_file() {
                m.file(base, &format!("etc/ssh/{name}"));
            }
        }
    }
    Ok(())
}
fn apply_tree(ctx: &Ctx, stage: &Path, mode: Mode) -> Result<(), String> {
    let packed = stage.join("home/bia");
    if packed.is_dir() {
        // Keep the image's MCP programs: archive binaries may be from another release,
        // and Update has already put the image's binaries in place.
        let keep = keep_image_mcp_programs()?;
        ctx.log(format!("unfold: restoring {}", HOME));
        copy_any(ctx, "unfold", &packed, Path::new(HOME), None, 0)?;
        put_back_mcp_programs(ctx, &keep.0)?;
    } else {
        let toml = stage.join("bia.toml");
        if toml.is_file() {
            copy_file(Some(ctx), &toml, Path::new(&format!("{HOME}/bia.toml")))?;
        }
        let state = stage.join("files/state");
        if state.is_dir() {
            copy_tree(Some(ctx), &state, &files_root().join("state"), &|n| STATE_SKIP.contains(&n))?;
        }
        let mcps = stage.join("mcps");
        if mcps.is_dir() {
            let skip_json = mode == Mode::Update;
            copy_tree(Some(ctx), &mcps, &PathBuf::from(format!("{HOME}/MCPs")), &|n| {
                n == "bin" || n == "native-host" || (skip_json && n == "mcp.json")
            })?;
        }
        let chrome = stage.join("chrome-profile");
        if chrome.is_dir() {
            let dst = PathBuf::from(format!("{HOME}/.config/bia-chrome"));
            // Older archives used the default profile path; migrate on next chrome start either way.
            let legacy = PathBuf::from(format!("{HOME}/.config/google-chrome"));
            if !dst.exists() && legacy.is_dir() {
                let _ = fs::rename(&legacy, &dst);
            }
            fs::create_dir_all(&dst).map_err(|e| e.to_string())?;
            copy_tree(Some(ctx), &chrome, &dst, &|n| CHROME_SKIP.contains(&n))?;
        }
    }
    let etc = stage.join("etc");
    if etc.is_dir() {
        apply_etc(&etc, mode)?;
    }
    Ok(())
}

fn apply_etc(src: &Path, mode: Mode) -> Result<(), String> {
    let mut stack = vec![src.to_path_buf()];
    while let Some(cur) = stack.pop() {
        let rd = match fs::read_dir(&cur) {
            Ok(rd) => rd,
            Err(e) => return Err(e.to_string()),
        };
        for ent in rd {
            let ent = ent.map_err(|e| e.to_string())?;
            let path = ent.path();
            let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            if meta.file_type().is_symlink() {
                continue;
            }
            let rel = path.strip_prefix(src).unwrap_or(&path);
            if !rel_ok(rel) {
                continue;
            }
            let rel_s = rel.to_string_lossy();
            if mode == Mode::Land && land_skips_etc(&rel_s) {
                continue;
            }
            let dst = Path::new("/etc").join(rel);
            if meta.is_dir() {
                fs::create_dir_all(&dst).map_err(|e| e.to_string())?;
                stack.push(path);
            } else if meta.is_file() {
                if let Some(parent) = dst.parent() {
                    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                copy_file(None, &path, &dst)?;
            }
        }
    }
    Ok(())
}

fn land_skips_etc(rel: &str) -> bool {
    rel == "hostname"
        || rel == "machine-id"
        || rel.starts_with("network/")
        || rel.starts_with("wpa_supplicant")
        || rel.starts_with("ssh/")
}

fn restore_stashed_mcps(ctx: &Ctx, stash: &Path) -> Result<(), String> {
    restore_stashed_mcps_into(ctx, stash, &PathBuf::from(format!("{HOME}/MCPs")))
}

/// Put local MCPs (not shipped by the new image) back from the `cp -a` stash.
/// Symlinks are kept: a Python .venv is bin/python -> /usr/bin/python3 and lib64 -> lib.
fn restore_stashed_mcps_into(ctx: &Ctx, stash: &Path, dest_root: &Path) -> Result<(), String> {
    if !stash.is_dir() {
        return Ok(());
    }
    fs::create_dir_all(dest_root).map_err(|e| e.to_string())?;
    for ent in fs::read_dir(stash).map_err(|e| e.to_string())? {
        let ent = ent.map_err(|e| e.to_string())?;
        let from = ent.path();
        if !from.is_dir() {
            continue;
        }
        let name = ent.file_name();
        let name_s = name.to_string_lossy();
        if !rel_ok(Path::new(&*name_s)) {
            continue;
        }
        let dst = dest_root.join(&name);
        ctx.log(format!("unfold: restoring local MCP {name_s}"));
        let _ = fs::remove_dir_all(&dst);
        copy_any(ctx, "unfold", &from, &dst, None, 0)?;
    }
    Ok(())
}

fn boot_notice(base: &str, failed: &[String], missing_bins: &[String]) -> String {
    let mut notice = base.to_string();
    if !failed.is_empty() {
        notice.push_str(". packages that did not install: ");
        notice.push_str(&failed.join("; "));
        notice.push_str(". Search the internet for each of these and download the .deb for that version and architecture");
    }
    if !missing_bins.is_empty() {
        notice.push_str(". binaries that were not copied: ");
        notice.push_str(&missing_bins.join(", "));
    }
    if !failed.is_empty() || !missing_bins.is_empty() {
        notice.push_str(". Repair these yourself now, then tell the user what you fixed and what still fails. Ask the user first only before a destructive step (removing or overwriting existing files, packages or binaries, up- or downgrading installed packages, disk or partition changes, reboot or poweroff).");
    }
    notice
}

/// Copy `bin/aarch64` and `bin/x86_64` from each MCP. A missing architecture is skipped.
/// Files that already exist on the destination are left alone (do not replace system bins).
/// A copy that fails is returned as a note; other MCPs are still copied.
fn copy_arch_bins(src_root: &Path, dst_root: &Path) -> Vec<String> {
    let mut missed = Vec::new();
    let Ok(rd) = fs::read_dir(src_root) else {
        return missed;
    };
    for ent in rd.flatten() {
        if !ent.path().is_dir() {
            continue;
        }
        let name = ent.file_name();
        let name_s = name.to_string_lossy();
        if !rel_ok(Path::new(&*name_s)) {
            continue;
        }
        for arch in MCP_ARCHS {
            let from = ent.path().join("bin").join(arch);
            if !from.is_dir() {
                continue;
            }
            let to = dst_root.join(&name).join("bin").join(arch);
            if let Err(e) = copy_tree_skip_existing(None, &from, &to) {
                missed.push(format!("{name_s} bin/{arch}: {e}"));
                continue;
            }
            mark_exec(&to);
        }
    }
    missed
}

/// Like `copy_tree`, but never overwrites a file that already exists at the destination.
fn copy_tree_skip_existing(ctx: Option<&Ctx>, src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    let rd = fs::read_dir(src).map_err(|e| e.to_string())?;
    for ent in rd {
        let ent = ent.map_err(|e| e.to_string())?;
        let from = ent.path();
        let name = ent.file_name();
        let name_s = name.to_string_lossy();
        if !rel_ok(Path::new(&*name_s)) {
            continue;
        }
        let to = dst.join(&name);
        let meta = fs::symlink_metadata(&from).map_err(|e| e.to_string())?;
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            copy_tree_skip_existing(ctx, &from, &to)?;
        } else if meta.is_file() {
            if to.is_file() {
                continue;
            }
            copy_file(ctx, &from, &to)?;
        }
    }
    Ok(())
}

fn mark_exec(dir: &Path) {
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    for ent in rd.flatten() {
        let path = ent.path();
        if path.is_dir() {
            mark_exec(&path);
            continue;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o755));
        }
    }
}

/// `mcp.json` command paths for aarch64 and x86_64 that are not files on disk.
fn missing_mcp_bins(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(root) else {
        return out;
    };
    for ent in rd.flatten() {
        if !ent.path().is_dir() {
            continue;
        }
        let name = ent.file_name();
        let name_s = name.to_string_lossy();
        if !rel_ok(Path::new(&*name_s)) {
            continue;
        }
        let Ok(bytes) = fs::read(ent.path().join("mcp.json")) else {
            continue;
        };
        let Ok(v) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        let Some(cmd) = v.get("command").and_then(|c| c.as_object()) else {
            continue;
        };
        for arch in MCP_ARCHS {
            let Some(rel) = cmd.get(*arch).and_then(|x| x.as_str()) else {
                continue;
            };
            let rel_path = Path::new(rel);
            if !rel_ok(rel_path) {
                continue;
            }
            if !ent.path().join(rel_path).is_file() {
                out.push(format!("{name_s} {rel}"));
            }
        }
    }
    out.sort();
    out
}

fn rewrite_native_host() -> String {
    let arch = std::env::consts::ARCH;
    let bin = format!("{HOME}/MCPs/chrome/bin/{arch}/bia-mcp-chrome");
    let dir = PathBuf::from(format!("{HOME}/MCPs/chrome"));
    if !dir.is_dir() {
        return "chrome: no MCP folder".into();
    }
    if !Path::new(&bin).is_file() {
        return format!("chrome native-host left as it is: no {bin}");
    }
    let wrapper = dir.join("native-host");
    let script = format!("#!/bin/sh\nexec '{bin}' --native-host\n");
    if let Err(e) = fs::write(&wrapper, script) {
        return format!("native-host: {e}");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755));
    }
    "chrome native-host rewritten for this architecture".into()
}

fn reboot(notice: &str) -> Result<(), String> {
    let dir = files_root().join("state");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let notice_path = dir.join("boot_notice");
    fs::write(&notice_path, notice).map_err(|e| format!("boot notice: {e}"))?;
    let _ = fs::write(dir.join("last_exit"), "reboot");
    let _ = Command::new("sync").status();
    match Command::new("systemctl").arg("reboot").spawn() {
        Ok(_) => Ok(()),
        Err(e) => {
            let _ = fs::remove_file(&notice_path);
            Err(format!("reboot: {e}"))
        }
    }
}

/// Reinstall the folded machine's manual packages. A kept .deb (see `debs`) is used for
/// each package that is not installed yet, plus kept dependencies that are missing, so
/// exact versions come back even without a network; the rest install by name.
/// Returns the names that did not install.
fn apt_install(ctx: &Ctx, pkgs: &[ManualPkg], debs_dir: &Path) -> Vec<String> {
    if pkgs.is_empty() {
        return vec![];
    }
    let arch = Command::new("dpkg")
        .arg("--print-architecture")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    let live = crate::debs::installed_at(Path::new("/"));
    let is_installed = |n: &str| live.iter().any(|(name, _, _)| name == n);
    let usable: Vec<crate::debs::Deb> = crate::debs::list(debs_dir)
        .into_iter()
        .filter(|d| d.arch == "all" || d.arch == arch)
        .collect();
    // (package name, apt-get argument)
    let mut items: Vec<(String, String)> = Vec::new();
    for p in pkgs {
        // Same image on both sides is the usual case: nothing to fetch.
        if is_installed(&p.name) {
            continue;
        }
        let deb = usable
            .iter()
            .find(|d| d.name == p.name && (p.version.is_empty() || d.version == p.version));
        let arg = deb.map(|d| d.path.to_string_lossy().into_owned()).unwrap_or_else(|| p.name.clone());
        items.push((p.name.clone(), arg));
    }
    let mut deps: Vec<&crate::debs::Deb> = Vec::new();
    for d in &usable {
        if !is_installed(&d.name)
            && !pkgs.iter().any(|p| p.name == d.name)
            && !deps.iter().any(|x| x.name == d.name)
        {
            deps.push(d);
        }
    }
    let dep_args: Vec<String> = deps.iter().map(|d| d.path.to_string_lossy().into_owned()).collect();
    if items.is_empty() && dep_args.is_empty() {
        ctx.log(format!("unfold: all {} manual packages are already installed", pkgs.len()));
        return vec![];
    }
    let local = items.iter().filter(|(n, a)| a != n).count();
    ctx.log(format!(
        "unfold: {} manual packages to install ({local} from kept .debs, {} kept dependencies)",
        items.len(),
        dep_args.len()
    ));
    ctx.log("unfold: apt-get update");
    // Offline (no route, no DNS) must fail, not hang the unfold; a slow link still gets time.
    let update = run_bounded(
        Command::new("apt-get")
            .args(["update", "-qq", "-o", "Acquire::Retries=1", "-o", "Acquire::http::Timeout=20", "-o", "Acquire::https::Timeout=20"])
            .env("DEBIAN_FRONTEND", "noninteractive")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null()),
        Duration::from_secs(900),
    );
    if !update {
        if local == 0 {
            ctx.log("unfold: apt-get update failed; package names were not installed");
            return items.into_iter().map(|(n, _)| n).collect();
        }
        ctx.log("unfold: apt-get update failed; installing from kept .debs, names may fail");
    }
    let mark_deps_auto = || {
        if !deps.is_empty() {
            let names: Vec<&str> = deps.iter().map(|d| d.name.as_str()).collect();
            let _ = Command::new("apt-mark").arg("auto").args(&names).output();
        }
    };
    let mut all: Vec<String> = items.iter().map(|(_, a)| a.clone()).collect();
    all.extend(dep_args.iter().cloned());
    if apt_get_install(ctx, &all, "unfold: apt") {
        mark_deps_auto();
        return vec![];
    }
    let mut failed = clear_broken(ctx);
    let n = items.len();
    for (i, (name, arg)) in items.iter().enumerate() {
        if failed.contains(name) {
            continue;
        }
        // the batch already installed most of them
        if crate::debs::installed_at(Path::new("/")).iter().any(|(p, _, _)| p == name) {
            continue;
        }
        ctx.log(format!("unfold: apt-get install {name} ({}/{n})", i + 1));
        let mut one = vec![arg.clone()];
        one.extend(dep_args.iter().cloned());
        if !apt_get_install(ctx, &one, &format!("unfold: apt {name}")) {
            for b in clear_broken(ctx) {
                if !failed.contains(&b) {
                    failed.push(b);
                }
            }
            let ok = crate::debs::installed_at(Path::new("/")).iter().any(|(p, _, _)| p == name);
            if !ok && !failed.contains(name) {
                failed.push(name.clone());
                ctx.log(format!("unfold: {name} did not install"));
            }
        }
    }
    mark_deps_auto();
    failed
}

/// Run to completion, or kill it after `limit`. True when it exited 0.
fn run_bounded(cmd: &mut Command, limit: Duration) -> bool {
    let Ok(mut child) = cmd.spawn() else { return false };
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(st)) => return st.success(),
            Ok(None) if start.elapsed() < limit => std::thread::sleep(Duration::from_millis(500)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

/// `apt-get install` with live progress from APT::Status-Fd (download ~30%, dpkg ~70%).
/// The status goes to its own pipe on fd 3: with Status-Fd=1 apt hides stdout from
/// dpkg, so maintainer scripts that print (google-chrome-stable's postinst runs sed)
/// failed with "couldn't close stdout: Bad file descriptor".
fn apt_get_install(ctx: &Ctx, args: &[String], label: &str) -> bool {
    use std::io::BufRead;
    use std::os::fd::FromRawFd;
    use std::os::unix::process::CommandExt;
    let mut fds = [0 as libc::c_int; 2];
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return false;
    }
    let (rd, wr) = (fds[0], fds[1]);
    let mut cmd = Command::new("apt-get");
    cmd.args(["-q", "-y", "--no-install-recommends", "-o", "APT::Status-Fd=3", "-o", "Dpkg::Use-Pty=0"])
        .arg("install")
        .args(args)
        .env("DEBIAN_FRONTEND", "noninteractive")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped());
    unsafe {
        // dup2 clears close-on-exec on fd 3 only; the original pipe ends stay closed.
        cmd.pre_exec(move || {
            if libc::dup2(wr, 3) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = cmd.spawn();
    unsafe { libc::close(wr) };
    let status = unsafe { std::fs::File::from_raw_fd(rd) };
    let Ok(mut child) = child else {
        return false;
    };
    // apt's normal output (and maintainer scripts'): errors and warnings to the log.
    let out_reader = child.stdout.take().map(|out| {
        let ctx = ctx.clone();
        let label = label.to_string();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(out).lines().map_while(Result::ok) {
                let t = line.trim();
                if t.starts_with("E:") || t.starts_with("W:") || t.starts_with("dpkg: error") {
                    ctx.log(format!("{label}: {t}"));
                }
            }
        })
    });
    let mut last: Option<u32> = None;
    for line in std::io::BufReader::new(status).lines().map_while(Result::ok) {
        if let Some((pct, msg)) = apt_status(&line) {
            if last.map_or(true, |l| pct > l) {
                last = Some(pct);
                ctx.progress(pct, format!("{label}: {msg}"));
            }
        } else if line.starts_with("pmerror:") {
            ctx.log(format!("{label}: {}", line.trim()));
        }
    }
    if let Some(t) = out_reader {
        let _ = t.join();
    }
    child.wait().map(|s| s.success()).unwrap_or(false)
}

/// Packages dpkg left half-installed or half-configured (their scripts failed).
fn broken_packages() -> Vec<String> {
    let out = Command::new("dpkg-query")
        .args(["-W", "-f", "${Package}\t${db:Status-Abbrev}\n"])
        .output();
    let Ok(out) = out else { return vec![] };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let (name, st) = l.split_once('\t')?;
            let st = st.trim();
            // want=install but not fully installed (U unpacked, F half-configured,
            // H half-installed, W/t triggers awaited/pending are fine)
            let bad = st.starts_with('i') && matches!(st.chars().nth(1), Some('U' | 'F' | 'H'));
            bad.then(|| name.to_string())
        })
        .collect()
}

/// A package whose maintainer script fails blocks every later apt-get (apt tries to
/// configure it each time). Retry once; if it still fails, remove it so the rest can
/// install, and return it to be reported as not installed.
fn clear_broken(ctx: &Ctx) -> Vec<String> {
    if broken_packages().is_empty() {
        return vec![];
    }
    let _ = Command::new("dpkg")
        .args(["--configure", "-a"])
        .env("DEBIAN_FRONTEND", "noninteractive")
        .stdin(std::process::Stdio::null())
        .output();
    let still = broken_packages();
    for p in &still {
        ctx.log(format!("unfold: {p}: its install script fails; removing it so the other packages can install"));
        let _ = Command::new("dpkg")
            .args(["--remove", "--force-remove-reinstreq", "--force-depends", p])
            .stdin(std::process::Stdio::null())
            .output();
    }
    still
}

/// Overall percent and message from an APT::Status-Fd line.
/// The id field may contain a colon (`libc6:arm64`), so the percent is the first numeric field after it.
fn apt_status(line: &str) -> Option<(u32, String)> {
    let parts: Vec<&str> = line.split(':').collect();
    let kind = *parts.first()?;
    let k = (2..parts.len()).find(|&i| parts[i].trim().parse::<f64>().is_ok())?;
    let pct: f64 = parts[k].trim().parse().ok()?;
    let msg = parts[k + 1..].join(":").trim().to_string();
    let pct = pct.clamp(0.0, 100.0);
    let overall = match kind {
        "dlstatus" => pct * 0.3,
        "pmstatus" => 30.0 + pct * 0.7,
        _ => return None,
    };
    Some((overall as u32, msg))
}

#[derive(Clone)]
struct ManualPkg {
    name: String,
    version: String,
    arch: String,
    origin: String,
}

fn pkg_label(p: &ManualPkg) -> String {
    let mut s = p.name.clone();
    if !p.version.is_empty() {
        s.push(' ');
        s.push_str(&p.version);
    }
    if !p.arch.is_empty() {
        s.push(' ');
        s.push_str(&p.arch);
    }
    if !p.origin.is_empty() {
        s.push_str(" from ");
        s.push_str(&p.origin);
    }
    s
}

fn manual_at(base: &Path) -> Vec<ManualPkg> {
    if base == Path::new("/") {
        live_manual_packages()
    } else {
        let status = fs::read_to_string(base.join("var/lib/dpkg/status")).unwrap_or_default();
        let extended = fs::read_to_string(base.join("var/lib/apt/extended_states")).unwrap_or_default();
        manual_packages(&status, &extended)
    }
}

fn package_records_at(base: &Path) -> Value {
    let pkgs = if base == Path::new("/") {
        let mut pkgs = live_manual_packages();
        attach_origins(&mut pkgs);
        pkgs
    } else {
        let status = fs::read_to_string(base.join("var/lib/dpkg/status")).unwrap_or_default();
        let extended = fs::read_to_string(base.join("var/lib/apt/extended_states")).unwrap_or_default();
        manual_packages(&status, &extended)
    };
    Value::Array(
        pkgs.into_iter()
            .map(|p| {
                json!({
                    "name": p.name,
                    "version": p.version,
                    "arch": p.arch,
                    "origin": p.origin,
                })
            })
            .collect(),
    )
}

fn manifest_packages(manifest: &Value) -> Vec<ManualPkg> {
    let Some(items) = manifest.get("packages").and_then(|v| v.as_array()) else {
        return vec![];
    };
    items
        .iter()
        .filter_map(|item| {
            if let Some(name) = item.as_str() {
                if pkg_ok(name) && !skip_pkg(name) {
                    return Some(ManualPkg {
                        name: name.to_string(),
                        version: String::new(),
                        arch: String::new(),
                        origin: String::new(),
                    });
                }
                return None;
            }
            let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("");
            if !pkg_ok(name) || skip_pkg(name) {
                return None;
            }
            Some(ManualPkg {
                name: name.to_string(),
                version: item.get("version").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                arch: item.get("arch").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                origin: item.get("origin").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            })
        })
        .collect()
}

fn live_manual_packages() -> Vec<ManualPkg> {
    let installed = dpkg_query_records();
    let manual = apt_manual();
    let src: Vec<ManualPkg> = if manual.is_empty() {
        installed
    } else {
        installed
            .into_iter()
            .filter(|p| manual.iter().any(|n| n == &p.name))
            .collect()
    };
    src.into_iter().filter(|p| pkg_ok(&p.name) && !skip_pkg(&p.name)).collect()
}

fn dpkg_query_records() -> Vec<ManualPkg> {
    let out = Command::new("dpkg-query")
        .args(["-W", "-f", "${Package}\t${Version}\t${Architecture}\t${db:Status-Status}\n"])
        .output();
    let Ok(out) = out else {
        return vec![];
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let mut f = line.split('\t');
            let name = f.next()?.trim();
            let version = f.next().unwrap_or("").trim();
            let arch = f.next().unwrap_or("").trim();
            let status = f.next().unwrap_or("").trim();
            if status != "installed" || name.is_empty() {
                return None;
            }
            Some(ManualPkg {
                name: name.to_string(),
                version: version.to_string(),
                arch: arch.to_string(),
                origin: String::new(),
            })
        })
        .collect()
}

fn attach_origins(pkgs: &mut [ManualPkg]) {
    if pkgs.is_empty() {
        return;
    }
    let mut cmd = Command::new("apt-cache");
    cmd.arg("policy");
    for p in pkgs.iter() {
        cmd.arg(&p.name);
    }
    let Ok(out) = cmd.output() else {
        return;
    };
    let found = origins_from_policy(&String::from_utf8_lossy(&out.stdout));
    for p in pkgs.iter_mut() {
        if let Some(origin) = found.get(&p.name) {
            p.origin = origin.clone();
        }
    }
}

fn arch_at(base: &Path) -> String {
    if base == Path::new("/") {
        return std::env::consts::ARCH.to_string();
    }
    dpkg_arch(&fs::read_to_string(base.join("var/lib/dpkg/status")).unwrap_or_default())
}

fn manual_packages(status: &str, extended: &str) -> Vec<ManualPkg> {
    let mut installed = Vec::new();
    let mut name = String::new();
    let mut version = String::new();
    let mut arch = String::new();
    let mut ok = false;
    for line in status.lines().chain(std::iter::once("")) {
        if line.is_empty() {
            if ok && !name.is_empty() {
                installed.push(ManualPkg {
                    name: std::mem::take(&mut name),
                    version: std::mem::take(&mut version),
                    arch: std::mem::take(&mut arch),
                    origin: String::new(),
                });
            } else {
                name.clear();
                version.clear();
                arch.clear();
            }
            ok = false;
            continue;
        }
        if let Some(v) = line.strip_prefix("Package: ") {
            name = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("Version: ") {
            version = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("Architecture: ") {
            arch = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("Status: ") {
            ok = v.trim() == "install ok installed";
        }
    }
    let mut auto = std::collections::HashSet::new();
    let mut pkg = String::new();
    let mut is_auto = false;
    for line in extended.lines().chain(std::iter::once("")) {
        if line.is_empty() {
            if is_auto && !pkg.is_empty() {
                auto.insert(std::mem::take(&mut pkg));
            } else {
                pkg.clear();
            }
            is_auto = false;
            continue;
        }
        if let Some(v) = line.strip_prefix("Package: ") {
            pkg = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("Auto-Installed: ") {
            is_auto = v.trim() == "1";
        }
    }
    installed
        .into_iter()
        .filter(|p| !auto.contains(&p.name) && pkg_ok(&p.name) && !skip_pkg(&p.name))
        .collect()
}

/// Apt origin of the installed version. `apt-cache policy` marks that version with `***`.
fn origins_from_policy(text: &str) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let mut current = String::new();
    let mut want_url = false;
    for line in text.lines() {
        if !line.starts_with(' ') && !line.starts_with('\t') && line.ends_with(':') && !line.contains(' ') {
            current = line.trim_end_matches(':').to_string();
            want_url = false;
            continue;
        }
        if current.is_empty() {
            continue;
        }
        let t = line.trim();
        if t.starts_with("***") {
            want_url = true;
            continue;
        }
        if !want_url {
            continue;
        }
        let mut parts = t.split_whitespace();
        let pri = parts.next().unwrap_or("");
        if !pri.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let url = parts.next().unwrap_or("");
        if !(url.starts_with("http://") || url.starts_with("https://") || url.starts_with("mirror+") || url.starts_with("file:")) {
            continue;
        }
        let suite = parts.next().unwrap_or("");
        let origin = if suite.is_empty() { url.to_string() } else { format!("{url} {suite}") };
        map.entry(current.clone()).or_insert(origin);
        want_url = false;
    }
    map
}

fn dpkg_arch(status: &str) -> String {
    let mut name = String::new();
    let mut arch = String::new();
    for line in status.lines().chain(std::iter::once("")) {
        if line.is_empty() {
            if name == "dpkg" && !arch.is_empty() {
                return arch;
            }
            name.clear();
            arch.clear();
            continue;
        }
        if let Some(v) = line.strip_prefix("Package: ") {
            name = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("Architecture: ") {
            arch = v.trim().to_string();
        }
    }
    String::new()
}

fn apt_manual() -> Vec<String> {
    let out = Command::new("apt-mark").arg("showmanual").output();
    let Ok(out) = out else { return vec![] };
    String::from_utf8_lossy(&out.stdout).lines().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

fn skip_pkg(name: &str) -> bool {
    name.starts_with("linux-image-")
        || name.starts_with("linux-headers-")
        || name.starts_with("linux-kbuild-")
        || name == "linux-libc-dev"
        || name.ends_with("-arm64")
        || name.ends_with("-amd64")
        || name.ends_with("-armhf")
}

fn pkg_ok(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit() => {}
        _ => return false,
    }
    name.len() < 80
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '+' | '-' | '.'))
}

fn mcp_bins_at(root: &Path) -> Value {
    let mut map = serde_json::Map::new();
    let Ok(rd) = fs::read_dir(&root) else { return Value::Object(map) };
    for ent in rd.flatten() {
        let bin = ent.path().join("bin");
        if !bin.is_dir() {
            continue;
        }
        let mut arches = Vec::new();
        if let Ok(sub) = fs::read_dir(&bin) {
            for s in sub.flatten() {
                if s.path().is_dir() {
                    arches.push(Value::String(s.file_name().to_string_lossy().into_owned()));
                }
            }
        }
        if !arches.is_empty() {
            map.insert(ent.file_name().to_string_lossy().into_owned(), Value::Array(arches));
        }
    }
    Value::Object(map)
}

fn debian_codename_at(base: &Path) -> String {
    let text = fs::read_to_string(base.join("etc/os-release")).unwrap_or_default();
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("VERSION_CODENAME=") {
            return v.trim_matches('"').to_string();
        }
    }
    String::new()
}

fn shipped_set_at(base: &Path) -> std::collections::HashSet<String> {
    fs::read_to_string(base.join("etc/bia-shipped"))
        .unwrap_or_default()
        .lines()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn copy_tree(ctx: Option<&Ctx>, src: &Path, dst: &Path, skip: &dyn Fn(&str) -> bool) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| format!("{}: {e}", dst.display()))?;
    let mut stack = vec![(src.to_path_buf(), dst.to_path_buf())];
    while let Some((cur, dest)) = stack.pop() {
        let rd = fs::read_dir(&cur).map_err(|e| format!("{}: {e}", cur.display()))?;
        for ent in rd {
            let ent = ent.map_err(|e| e.to_string())?;
            let name = ent.file_name();
            let name_s = name.to_string_lossy();
            if skip(&name_s) || !rel_ok(Path::new(&*name_s)) {
                continue;
            }
            let from = ent.path();
            let meta = fs::symlink_metadata(&from).map_err(|e| e.to_string())?;
            if meta.file_type().is_symlink() {
                continue;
            }
            let to = dest.join(&name);
            if meta.is_dir() {
                fs::create_dir_all(&to).map_err(|e| e.to_string())?;
                stack.push((from, to));
            } else if meta.is_file() {
                copy_file(ctx, &from, &to)?;
            }
        }
    }
    Ok(())
}


fn named_with_unfolded(path: &Path, bytes: u64) -> PathBuf {
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("fold.tar.gz");
    let stem = name.strip_suffix(".tar.gz").unwrap_or(name);
    let stem = match stem.rfind(".unfold-") {
        Some(i) if !stem[i + ".unfold-".len()..].is_empty() && stem[i + ".unfold-".len()..].bytes().all(|b| b.is_ascii_digit()) => &stem[..i],
        _ => stem,
    };
    path.with_file_name(format!("{stem}.unfold-{bytes}.tar.gz"))
}

/// Unpacked size stored in `name.unfold-<bytes>.tar.gz`. Anything else is not a sized archive.
fn unfolded_size(path: &Path) -> Option<u64> {
    let name = path.file_name()?.to_str()?;
    let stem = name.strip_suffix(".tar.gz")?;
    let i = stem.rfind(".unfold-")?;
    let digits = &stem[i + ".unfold-".len()..];
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// Refuse when the disks that will hold a full copy do not have `need` bytes free.
/// The temporary tree and the destination are both full copies, so one disk that holds both needs twice `need`.
fn guard_space(action: &str, need: u64, temp: &Path, dest: &Path, force: bool, tool: Option<&str>) -> Result<(), String> {
    if force || need == 0 {
        return Ok(());
    }
    let temp = dir_ready(temp)?;
    let dest = dir_ready(dest)?;
    let same = device_id(&temp)? == device_id(&dest)?;
    if same {
        let share = if action == "unfold" {
            "unpacking and restoring share this disk"
        } else {
            "the temporary copy and the archive share this disk"
        };
        space_short(action, &dest, need, 2, tool, share)
    } else {
        space_short(action, &dest, need, 1, tool, "")?;
        space_short(action, &temp, need, 1, tool, "")
    }
}

fn dir_ready(path: &Path) -> Result<PathBuf, String> {
    if path.is_dir() {
        return Ok(path.to_path_buf());
    }
    if let Some(parent) = path.parent() {
        if parent.is_dir() {
            return Ok(parent.to_path_buf());
        }
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        return Ok(parent.to_path_buf());
    }
    Err(format!("{} is not a directory", path.display()))
}

fn space_short(action: &str, dir: &Path, need: u64, copies: u64, tool: Option<&str>, share: &str) -> Result<(), String> {
    let free = free_bytes(dir)?;
    let req = need.saturating_mul(copies);
    if free >= req {
        return Ok(());
    }
    let subject = if action == "unfold" {
        format!("the archive needs {need} bytes when unpacked")
    } else {
        format!("the folder is {need} bytes")
    };
    let mut msg = format!("not enough free space to {action}. {subject} and {} has {free} bytes free.", dir.display());
    if copies > 1 {
        msg.push_str(&format!(" {share}, so it needs {req} bytes."));
    }
    if let Some(tool) = tool {
        msg.push_str(&format!(" Ask the user, then call {tool} again with force=true only after they confirm."));
    }
    Err(msg)
}

fn device_id(path: &Path) -> Result<u64, String> {
    use std::os::unix::fs::MetadataExt;
    fs::metadata(path).map(|m| m.dev()).map_err(|e| format!("{}: {e}", path.display()))
}

fn free_bytes(path: &Path) -> Result<u64, String> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(|_| format!("{} has a nul byte", path.display()))?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::statvfs(c.as_ptr(), &mut st) };
    if rc != 0 {
        return Err(format!("{}: {}", path.display(), std::io::Error::last_os_error()));
    }
    let fr = if st.f_frsize == 0 { st.f_bsize } else { st.f_frsize };
    Ok((st.f_bavail as u64).saturating_mul(fr as u64))
}

fn under_files(path: &Path) -> Result<PathBuf, String> {
    let root = files_root();
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let root = root.canonicalize().unwrap_or(root);
    let abs = if path.is_absolute() { path.to_path_buf() } else { root.join(path) };
    if abs.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err("path must stay under /home/bia/files".into());
    }
    let parent = abs.parent().unwrap_or(&root);
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let parent = parent.canonicalize().map_err(|e| e.to_string())?;
    if parent != root && !parent.starts_with(&root) {
        return Err("path must stay under /home/bia/files".into());
    }
    Ok(abs)
}
/// gzip tar of `meta/manifest.json` plus the NUL-separated `list` of paths under `base`.
fn tar_create(meta: &Path, base: &Path, list: &Path, dest: &Path) -> Result<(), String> {
    let st = Command::new("tar")
        .arg("-czf")
        .arg(dest)
        .args(["--no-recursion", "--null", "--ignore-failed-read", "--warning=no-file-changed"])
        .arg("-C")
        .arg(meta)
        .arg("manifest.json")
        .arg("-C")
        .arg(base)
        .arg("-T")
        .arg(list)
        .status()
        .map_err(|e| format!("tar: {e}"))?;
    // 1 = a live file changed while it was read (logs, browser state); the archive is complete.
    match st.code() {
        Some(0) | Some(1) => Ok(()),
        _ => Err(format!("tar create exited {st}")),
    }
}

fn tar_extract(archive: &Path, dest: &Path) -> Result<(), String> {
    require_gzip(archive)?;
    let st = Command::new("tar")
        .arg("-C")
        .arg(dest)
        .arg("-xzf")
        .arg(archive)
        .status()
        .map_err(|e| format!("tar: {e}"))?;
    if st.success() { Ok(()) } else { Err(format!("tar extract exited {st}")) }
}

fn require_gzip(archive: &Path) -> Result<(), String> {
    let mut f = File::open(archive).map_err(|e| format!("{}: {e}", archive.display()))?;
    let mut magic = [0u8; 2];
    let n = f.read(&mut magic).map_err(|e| e.to_string())?;
    if n == 2 && magic == [0x1f, 0x8b] {
        Ok(())
    } else {
        Err("archive is not gzip".into())
    }
}

#[cfg(test)]
mod tests {
    use super::{apt_install, broken_packages, dpkg_arch, manual_packages, ManualPkg};
    use bia_mcp::Ctx;
    use std::process::Command;
    use std::fs;
    use std::path::Path;

    #[test]
    fn manual_skips_auto_and_kernels() {
        let status = "\
Package: curl
Status: install ok installed
Version: 8.14.1-2
Architecture: arm64

Package: linux-image-lts
Status: install ok installed
Version: 6.12
Architecture: arm64

Package: libfoo
Status: install ok installed
Version: 1
Architecture: arm64

Package: half
Status: install ok unpacked
Version: 1
Architecture: arm64
";
        let extended = "\
Package: libfoo
Auto-Installed: 1

Package: curl
Auto-Installed: 0
";
        let got = manual_packages(status, extended);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "curl");
        assert_eq!(got[0].version, "8.14.1-2");
        assert_eq!(got[0].arch, "arm64");
    }

    #[test]
    fn dpkg_package_arch() {
        let status = "\
Package: bash
Architecture: all

Package: dpkg
Architecture: arm64
";
        assert_eq!(dpkg_arch(status), "arm64");
    }

    #[test]
    fn policy_origin_is_the_installed_version() {
        let text = "\
curl:
  Installed: 8.14.1-2
  Candidate: 9.0
  Version table:
     9.0 500
        500 http://example.test/debian sid/main arm64 Packages
 *** 8.14.1-2 500
        500 http://deb.debian.org/debian trixie/main arm64 Packages
";
        let got = super::origins_from_policy(text);
        assert_eq!(got.get("curl").map(String::as_str), Some("http://deb.debian.org/debian trixie/main"));
    }

    #[test]
    fn fold_archive_is_gzip() {
        let dir = std::env::temp_dir().join(format!("bia-carry-gz-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let base = dir.join("root");
        fs::create_dir_all(base.join("home/bia/files")).unwrap();
        fs::write(base.join("home/bia/files/hello.txt"), b"hello").unwrap();
        let meta = dir.join("meta");
        fs::create_dir_all(&meta).unwrap();
        fs::write(meta.join("manifest.json"), b"{}").unwrap();
        let mut m = super::Members::default();
        m.walk(&base, Path::new("home/bia"), None, &|_| true).unwrap();
        m.write_list(&meta.join("members")).unwrap();
        let dest = dir.join("fold.tar.gz");
        super::tar_create(&meta, &base, &meta.join("members"), &dest).unwrap();
        let bytes = fs::read(&dest).unwrap();
        assert_eq!(&bytes[..2], &[0x1f, 0x8b]);
        let out = dir.join("out");
        fs::create_dir_all(&out).unwrap();
        super::tar_extract(&dest, &out).unwrap();
        assert_eq!(fs::read(out.join("home/bia/files/hello.txt")).unwrap(), b"hello");
        assert_eq!(fs::read(out.join("manifest.json")).unwrap(), b"{}");
        let plain = dir.join("plain.tar");
        let st = std::process::Command::new("tar")
            .arg("-C")
            .arg(&base)
            .arg("-cf")
            .arg(&plain)
            .arg(".")
            .status()
            .unwrap();
        assert!(st.success());
        assert_eq!(super::tar_extract(&plain, &out).unwrap_err(), "archive is not gzip");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn fold_packs_both_mcp_architectures() {
        let dir = std::env::temp_dir().join(format!("bia-carry-bins-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let src = dir.join("src/carry");
        for arch in ["aarch64", "x86_64"] {
            let bin = src.join("bin").join(arch);
            fs::create_dir_all(&bin).unwrap();
            fs::write(bin.join("bia-mcp-carry"), arch.as_bytes()).unwrap();
        }
        fs::write(
            src.join("mcp.json"),
            r#"{"command":{"aarch64":"bin/aarch64/bia-mcp-carry","x86_64":"bin/x86_64/bia-mcp-carry"}}"#,
        )
        .unwrap();
        let dst = dir.join("dst");
        let missed = super::copy_arch_bins(&dir.join("src"), &dst);
        assert!(missed.is_empty(), "{missed:?}");
        fs::copy(src.join("mcp.json"), dst.join("carry/mcp.json")).unwrap();
        assert_eq!(fs::read(dst.join("carry/bin/aarch64/bia-mcp-carry")).unwrap(), b"aarch64");
        assert_eq!(fs::read(dst.join("carry/bin/x86_64/bia-mcp-carry")).unwrap(), b"x86_64");
        assert!(super::missing_mcp_bins(&dst).is_empty());
        // Existing destination binaries must not be replaced by archive copies.
        fs::write(dst.join("carry/bin/aarch64/bia-mcp-carry"), b"system").unwrap();
        let missed = super::copy_arch_bins(&dir.join("src"), &dst);
        assert!(missed.is_empty(), "{missed:?}");
        assert_eq!(fs::read(dst.join("carry/bin/aarch64/bia-mcp-carry")).unwrap(), b"system");
        assert_eq!(fs::read(dst.join("carry/bin/x86_64/bia-mcp-carry")).unwrap(), b"x86_64");
        let _ = fs::remove_file(dst.join("carry/bin/x86_64/bia-mcp-carry"));
        assert_eq!(
            super::missing_mcp_bins(&dst),
            vec!["carry bin/x86_64/bia-mcp-carry".to_string()]
        );
        let missed = super::copy_arch_bins(&dir.join("src"), &dst);
        assert!(missed.is_empty(), "{missed:?}");
        assert_eq!(fs::read(dst.join("carry/bin/x86_64/bia-mcp-carry")).unwrap(), b"x86_64");
        assert_eq!(fs::read(dst.join("carry/bin/aarch64/bia-mcp-carry")).unwrap(), b"system");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn notice_asks_for_repair_only_when_something_failed() {
        let clean = super::boot_notice("this boot restored from a carry archive", &[], &[]);
        assert_eq!(clean, "this boot restored from a carry archive");
        assert!(!clean.contains("Repair these yourself"));
        let dirty = super::boot_notice(
            "this boot restored from a carry archive",
            &["curl".into()],
            &["carry bin/x86_64/bia-mcp-carry".into()],
        );
        assert!(dirty.contains("packages that did not install: curl"));
        assert!(dirty.contains("Search the internet for each of these and download the .deb"));
        assert!(dirty.contains("binaries that were not copied: carry bin/x86_64/bia-mcp-carry"));
        assert!(dirty.contains("Repair these yourself now"));
        assert!(dirty.contains("Ask the user first only before a destructive step"));
    }

    /// Real dpkg/apt (run as root in a throwaway container):
    /// cargo test -p bia-mcp-carry -- --ignored apt_install_survives
    #[test]
    #[ignore = "installs packages; container only"]
    fn apt_install_survives_failing_and_chatty_scripts() {
        let dir = std::env::temp_dir().join(format!("bia-apt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let debs = dir.join("debs");
        std::fs::create_dir_all(&debs).unwrap();
        let build = |name: &str, postinst: &str| {
            let root = dir.join(name);
            std::fs::create_dir_all(root.join("DEBIAN")).unwrap();
            std::fs::write(
                root.join("DEBIAN/control"),
                format!("Package: {name}\nVersion: 1\nArchitecture: all\nMaintainer: t <t@t>\nDescription: t\n"),
            )
            .unwrap();
            let pi = root.join("DEBIAN/postinst");
            std::fs::write(&pi, format!("#!/bin/sh\nset -e\n{postinst}\n")).unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&pi, std::fs::Permissions::from_mode(0o755)).unwrap();
            let out = debs.join(format!("{name}_1_all.deb"));
            assert!(Command::new("dpkg-deb").args(["--build", "--root-owner-group"]).arg(&root).arg(&out).status().unwrap().success());
        };
        build("biachatty", "echo hello | sed s/h/H/");
        build("biabroken", "echo broken >&2; exit 1");
        build("biagood", "true");
        let pkg = |n: &str| ManualPkg { name: n.into(), version: "1".into(), arch: "all".into(), origin: String::new() };
        let failed = apt_install(&Ctx::console(), &[pkg("biabroken"), pkg("biachatty"), pkg("biagood")], &debs);
        let inst = crate::debs::installed_at(Path::new("/"));
        let has = |n: &str| inst.iter().any(|(p, _, _)| p == n);
        assert_eq!(failed, ["biabroken"]);
        assert!(has("biachatty"), "a postinst that prints must not fail");
        assert!(has("biagood"));
        assert!(broken_packages().is_empty(), "nothing left half-configured");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn apt_status_lines() {
        assert_eq!(super::apt_status("dlstatus:1:50.0:Retrieving file 1 of 2"), Some((15, "Retrieving file 1 of 2".into())));
        assert_eq!(super::apt_status("pmstatus:jq:100:Installed jq"), Some((100, "Installed jq".into())));
        assert_eq!(super::apt_status("pmstatus:libc6:arm64:50:Unpacking libc6:arm64"), Some((65, "Unpacking libc6:arm64".into())));
        assert_eq!(super::apt_status("pmerror:x.deb:20:failed"), None);
        assert_eq!(super::apt_status("Setting up jq (1.6) ..."), None);
    }

    #[test]
    fn stashed_mcp_keeps_venv_symlinks() {
        use std::os::unix::fs::symlink;
        let dir = std::env::temp_dir().join(format!("bia-carry-stash-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let venv = dir.join("stash/google-workspace/.venv");
        fs::create_dir_all(venv.join("bin")).unwrap();
        fs::create_dir_all(venv.join("lib/python3.11/site-packages")).unwrap();
        fs::write(venv.join("lib/python3.11/site-packages/x.py"), b"x").unwrap();
        fs::write(venv.join("pyvenv.cfg"), b"home = /usr/bin").unwrap();
        symlink("/usr/bin/python3", venv.join("bin/python3")).unwrap();
        symlink("python3", venv.join("bin/python")).unwrap();
        symlink("lib", venv.join("lib64")).unwrap();
        // the fold already restored a copy; the stash replaces it
        let live = dir.join("MCPs");
        fs::create_dir_all(live.join("google-workspace/.venv")).unwrap();
        let ctx = bia_mcp::Ctx::console();
        super::restore_stashed_mcps_into(&ctx, &dir.join("stash"), &live).unwrap();
        let out = live.join("google-workspace/.venv");
        assert_eq!(fs::read_link(out.join("bin/python3")).unwrap(), Path::new("/usr/bin/python3"));
        assert_eq!(fs::read_link(out.join("bin/python")).unwrap(), Path::new("python3"));
        assert_eq!(fs::read_link(out.join("lib64")).unwrap(), Path::new("lib"));
        assert_eq!(fs::read(out.join("lib/python3.11/site-packages/x.py")).unwrap(), b"x");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn fold_packs_the_whole_home() {
        let dir = std::env::temp_dir().join(format!("bia-carry-home-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let home = dir.join("home/bia");
        fs::create_dir_all(home.join("files/telegram")).unwrap();
        fs::write(home.join("files/telegram/ferogram.session"), b"session").unwrap();
        fs::create_dir_all(home.join("files/whatsapp")).unwrap();
        fs::write(home.join("files/whatsapp/whatsapp.db"), b"wa-session").unwrap();
        fs::create_dir_all(home.join("model")).unwrap();
        fs::write(home.join("model/weights.gguf"), b"model").unwrap();
        fs::create_dir_all(home.join("files/carry")).unwrap();
        let archive = home.join("files/carry/fold.tar.gz");
        fs::write(&archive, b"old-archive").unwrap();
        let mut m = super::Members::default();
        m.walk(&dir, Path::new("home/bia"), Some(&archive), &|_| true).unwrap();
        let has = |p: &str| m.list.iter().any(|x| x == Path::new(p));
        assert!(has("home/bia/files/telegram/ferogram.session"));
        assert!(has("home/bia/files/whatsapp/whatsapp.db"));
        assert!(has("home/bia/model/weights.gguf"));
        assert!(has("home/bia/files/carry"));
        assert!(!has("home/bia/files/carry/fold.tar.gz"));
        assert_eq!(m.files, 3);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn busy_errors_are_recognized() {
        let busy = std::io::Error::from_raw_os_error(libc::ETXTBSY as i32);
        assert!(super::text_file_busy(&busy));
        let ebusy = std::io::Error::from_raw_os_error(libc::EBUSY as i32);
        assert!(super::text_file_busy(&ebusy));
        let other = std::io::Error::from_raw_os_error(libc::EPERM as i32);
        assert!(!super::text_file_busy(&other));
    }

    #[test]
    fn archive_name_carries_unfolded_size() {
        let named = super::named_with_unfolded(Path::new("/home/bia/files/carry/fold-10.tar.gz"), 4821);
        assert_eq!(named.file_name().unwrap(), "fold-10.unfold-4821.tar.gz");
        assert_eq!(super::unfolded_size(&named), Some(4821));
        let again = super::named_with_unfolded(&named, 9);
        assert_eq!(again.file_name().unwrap(), "fold-10.unfold-9.tar.gz");
        assert_eq!(super::unfolded_size(Path::new("fold-10.tar.gz")), None);
        assert_eq!(super::unfolded_size(Path::new("backup.unfold-12.tar")), None);
        assert_eq!(super::unfolded_size(Path::new("x.unfold-12-y.tar.gz")), None);
    }


    #[test]
    fn fold_space_refuses_without_force() {
        let dir = std::env::temp_dir().join(format!("bia-carry-space-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let err = super::guard_space("fold", u64::MAX, &dir, &dir, false, Some("carry_fold")).unwrap_err();
        assert!(err.contains("not enough free space to fold"), "{err}");
        assert!(err.contains("the folder is"));
        assert!(err.contains("Ask the user"));
        assert!(err.contains("carry_fold"));
        assert!(err.contains("force=true"));
        super::guard_space("fold", u64::MAX, &dir, &dir, true, Some("carry_fold")).unwrap();
        if unsafe { libc::geteuid() } != 0 {
            let err = super::fold_into(&bia_mcp::Ctx::console(), &dir.join("x.tar.gz")).unwrap_err();
            assert!(err.contains("must run as root"), "{err}");
            let err = super::unfold_file(&bia_mcp::Ctx::console(), &dir.join("x.tar.gz"), super::Mode::Restore).unwrap_err();
            assert!(err.contains("must run as root"), "{err}");
        }
        let unfold = super::guard_space("unfold", u64::MAX, &dir, &dir, false, Some("carry_unfold")).unwrap_err();
        assert!(unfold.contains("when unpacked"), "{unfold}");
        assert!(unfold.contains("carry_unfold"));
        assert!(unfold.contains("force=true"));
        let _ = fs::remove_dir_all(&dir);
    }
}
