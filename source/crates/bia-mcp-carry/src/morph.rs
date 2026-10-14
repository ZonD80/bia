//! Booted from the flash: fold the agent that lives on the other disk, then
//! reboot so the next boot unfolds it onto this system.
use bia_mcp::Ctx;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const UPDATE: &str = "/bia-update";
const MNT: &str = "/run/bia-morph";
const LEAD: &str = "this boot restored the agent that was on the disk";

struct RoMount {
    path: PathBuf,
    mounted: bool,
}

impl Drop for RoMount {
    fn drop(&mut self) {
        if self.mounted {
            let _ = Command::new("umount").arg(&self.path).status();
        }
    }
}

pub fn morph(ctx: &Ctx, disk_arg: Option<&str>) -> Result<String, String> {
    let disk_arg = disk_arg
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            "pass disk=/dev/... for the partition that holds the agent (lsblk -f)".to_string()
        })?;
    let root_dev = root_device()?;
    let disk = resolve_morph_disk(disk_arg)?;
    if same_underlying_disk(&root_dev, &disk) {
        return Err(format!(
            "disk={} is on the same disk as this boot's root ({}); morph folds an agent from another disk",
            disk.display(),
            root_dev.display()
        ));
    }
    if Path::new(UPDATE).join("pending").is_file() || Path::new(UPDATE).join("copying").is_file() {
        return Err("an unfold is already waiting for the next boot".into());
    }
    ctx.log(format!("morph: this boot is {} ({})", root_dev.display(), fs_label(&root_dev)));
    ctx.log(format!("morph: folding the agent on {}", disk.display()));
    let mount = mount_ro(&disk)?;
    let agent = mount.path.join("home/bia");
    if !agent.join("bia.toml").is_file() && !agent.join("files/state").is_dir() {
        return Err(format!("{} has no agent", disk.display()));
    }
    fs::create_dir_all(UPDATE).map_err(|e| e.to_string())?;
    let partial = PathBuf::from(format!("{UPDATE}/fold.tar.gz.partial"));
    let tar = PathBuf::from(format!("{UPDATE}/fold.tar.gz"));
    let _ = fs::remove_file(&partial);
    if let Err(e) = crate::archive::fold_root(ctx, &mount.path, &partial) {
        let _ = fs::remove_file(&partial);
        return Err(e);
    }
    let _ = fs::remove_file(&tar);
    fs::rename(&partial, &tar).map_err(|e| format!("fold.tar.gz: {e}"))?;
    if let Err(e) = stash_disk_only(ctx, &agent.join("MCPs")) {
        let _ = fs::remove_file(&tar);
        return Err(e);
    }
    fs::write(Path::new(UPDATE).join("lead"), format!("{LEAD}\n")).map_err(|e| e.to_string())?;
    fs::write(Path::new(UPDATE).join("pending"), "pending\n").map_err(|e| e.to_string())?;
    let _ = Command::new("sync").status();
    drop(mount);
    ctx.log("morph: rebooting to unfold");
    Command::new("systemctl")
        .arg("reboot")
        .spawn()
        .map_err(|e| format!("reboot: {e}. the fold is waiting; the next boot unfolds it"))?;
    Ok(format!("{LEAD}. rebooting; the next boot unfolds it"))
}

fn resolve_morph_disk(raw: &str) -> Result<PathBuf, String> {
    let path = if raw.starts_with("/dev/") {
        PathBuf::from(raw)
    } else {
        PathBuf::from(format!("/dev/{raw}"))
    };
    if !path.exists() {
        return Err(format!("{raw} does not exist"));
    }
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let is_part = Path::new(&format!("/sys/class/block/{name}/partition")).is_file();
    if !is_part {
        let disk_name = name.to_string();
        for label in ["BIADISK", "BIALAND", "BIAROOT", "BIAFLASH"] {
            for dev in devices_with_label(label) {
                if block_disk(&dev) == disk_name {
                    return Ok(dev);
                }
            }
        }
        return Err(format!(
            "{raw} is a whole disk with no BIADISK/BIAROOT partition; pass the partition (e.g. /dev/sda2)"
        ));
    }
    Ok(path)
}

fn same_underlying_disk(a: &Path, b: &Path) -> bool {
    let da = block_disk(a);
    let db = block_disk(b);
    !da.is_empty() && da == db
}

fn mount_ro(dev: &Path) -> Result<RoMount, String> {
    let path = PathBuf::from(MNT);
    let _ = Command::new("umount").arg(&path).status();
    fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    let st = Command::new("mount")
        .args(["-o", "ro,noload"])
        .arg(dev)
        .arg(&path)
        .status()
        .map_err(|e| format!("mount: {e}"))?;
    if !st.success() {
        return Err(format!("could not mount {} read-only", dev.display()));
    }
    Ok(RoMount { path, mounted: true })
}

fn stash_disk_only(ctx: &Ctx, disk_mcps: &Path) -> Result<(), String> {
    let dest = PathBuf::from(format!("{UPDATE}/local-mcps"));
    let _ = fs::remove_dir_all(&dest);
    fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
    let here = Path::new("/home/bia/MCPs");
    let Ok(rd) = fs::read_dir(disk_mcps) else {
        ctx.log("morph: the disk has no MCP directory");
        return Ok(());
    };
    let mut n = 0u32;
    for ent in rd.flatten() {
        let name = ent.file_name();
        let name_s = name.to_string_lossy();
        if name_s.contains('/') || name_s.contains("..") || name_s.is_empty() {
            continue;
        }
        if !ent.path().is_dir() {
            continue;
        }
        if here.join(&name).is_dir() {
            ctx.log(format!("morph: MCP {name_s} is on this flash; it keeps this image's bin/ and mcp.json"));
            continue;
        }
        ctx.log(format!("morph: keeping MCP {name_s} from the disk, including its binaries"));
        let st = Command::new("cp")
            .args(["-a"])
            .arg(ent.path())
            .arg(&dest)
            .status()
            .map_err(|e| format!("cp: {e}"))?;
        if !st.success() {
            return Err(format!("could not keep MCP {name_s}"));
        }
        n += 1;
    }
    if n == 0 {
        ctx.log("morph: no MCP exists only on the disk");
    }
    Ok(())
}

fn root_device() -> Result<PathBuf, String> {
    let text = fs::read_to_string("/proc/mounts").map_err(|e| format!("/proc/mounts: {e}"))?;
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let dev = parts.next().unwrap_or("");
        let mnt = parts.next().unwrap_or("");
        if mnt == "/" && dev.starts_with("/dev/") {
            return Ok(PathBuf::from(dev));
        }
    }
    Err("cannot see the root device".into())
}

fn devices_with_label(label: &str) -> Vec<PathBuf> {
    let spec = format!("LABEL={label}");
    let Ok(out) = Command::new("blkid").args(["-c", "/dev/null", "-o", "device", "-t", &spec]).output() else {
        return vec![];
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|s| s.starts_with("/dev/"))
        .map(PathBuf::from)
        .collect()
}

fn fs_label(dev: &Path) -> String {
    let Ok(out) = Command::new("blkid").args(["-s", "LABEL", "-o", "value"]).arg(dev).output() else {
        return String::new();
    };
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn block_disk(dev: &Path) -> String {
    let canon = fs::canonicalize(dev).unwrap_or_else(|_| dev.to_path_buf());
    let Some(name) = canon.file_name() else { return String::new() };
    let name = name.to_string_lossy();
    let class = PathBuf::from(format!("/sys/class/block/{name}"));
    if class.join("partition").is_file() {
        if let Ok(link) = fs::read_link(&class) {
            if let Some(disk) = link.parent().and_then(|p| p.file_name()) {
                return disk.to_string_lossy().into_owned();
            }
        }
    }
    name.into_owned()
}
