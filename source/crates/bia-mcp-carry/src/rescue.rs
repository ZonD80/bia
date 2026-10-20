//! Booted from the flash: fold BIADISK's agent, then the rescue image copies
//! this OS onto the disk offline and boots it to unfold that agent. Also the
//! offline disk check (carry_fsck), which goes through the same rescue image.
use bia_mcp::Ctx;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};

pub fn rescue(ctx: &Ctx, disk: Option<&str>) -> Result<String, String> {
    let disk = disk.map(str::trim).filter(|s| !s.is_empty());
    if Path::new("/bia-update/pending").is_file() {
        return Err("an update is waiting to be unfolded; reboot to finish it".into());
    }
    let mut args: Vec<String> = vec!["rescue".into()];
    if let Some(d) = disk {
        args.push(d.to_string());
        ctx.log(format!("rescue: pushing this OS onto {d}"));
    } else {
        ctx.log("rescue: pushing this OS onto BIADISK");
    }
    stage(ctx, &args, "rescue staged; rebooting into the rescue image", "rescue failed")
}

/// Check and repair this root offline: the rescue image runs e2fsck -f -y on it while
/// it is not mounted, then boots it again; the result arrives at that boot.
pub fn fsck(ctx: &Ctx) -> Result<String, String> {
    if Path::new("/bia-update/pending").is_file() {
        return Err("an update is waiting to be unfolded; reboot to finish it first".into());
    }
    ctx.log("fsck: checking this root offline in the rescue image");
    stage(ctx, &["fsck".to_string()], "disk check staged; rebooting into the rescue image", "the disk check could not be staged")
}

/// Run bia-update with `args` (it reboots on success); its log lines go to the console.
fn stage(ctx: &Ctx, args: &[String], staged: &str, failed: &str) -> Result<String, String> {
    let mut child = Command::new("/usr/local/sbin/bia-update")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("bia-update: {e}"))?;
    let ctx_log = ctx.clone();
    let err = child.stderr.take();
    let logged = std::thread::spawn(move || {
        if let Some(err) = err {
            for line in BufReader::new(err).lines() {
                let Ok(line) = line else { continue };
                if !line.is_empty() {
                    ctx_log.log(line);
                }
            }
        }
    });
    let mut text = String::new();
    if let Some(mut out) = child.stdout.take() {
        let _ = std::io::Read::read_to_string(&mut out, &mut text);
    }
    let status = child.wait().map_err(|e| format!("bia-update: {e}"))?;
    let _ = logged.join();
    if status.success() {
        Ok(if text.trim().is_empty() {
            staged.into()
        } else {
            text
        })
    } else {
        Err(if text.trim().is_empty() {
            failed.into()
        } else {
            text
        })
    }
}
