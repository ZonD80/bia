//! Booted from the flash: fold BIADISK's agent, then the rescue image copies
//! this OS onto the disk offline and boots it to unfold that agent.
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
    let mut child = Command::new("/usr/local/sbin/bia-update")
        .args(&args)
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
            "rescue staged; rebooting into the rescue image".into()
        } else {
            text
        })
    } else {
        Err(if text.trim().is_empty() {
            "rescue failed".into()
        } else {
            text
        })
    }
}
