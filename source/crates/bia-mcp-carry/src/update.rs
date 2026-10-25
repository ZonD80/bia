//! Attended OS update (never automatic at boot): fold this agent into
//! /bia-update, mark the rescue image on this ESP and reboot. The rescue image
//! copies the other bia root (flash or internal disk; preferred) or the newest
//! GitHub release (downloaded and unpacked into /bia-update/net) onto this one
//! offline, then this system boots and unfolds the agent.
use bia_mcp::Ctx;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};

pub fn update(ctx: &Ctx, from: Option<&str>, force: bool) -> Result<String, String> {
    if Path::new("/bia-update/pending").is_file() {
        return Err("an update is waiting to be unfolded; reboot to finish it".into());
    }
    ctx.log("update: starting attended OS update (bia-update start); progress: /bia-update/update.log");
    let mut args = vec!["start".to_string()];
    if force {
        args.push("--force".into());
    }
    match from.map(str::trim).filter(|s| !s.is_empty()) {
        Some("internet" | "net" | "github" | "web") => args.push("--from=internet".into()),
        Some(dev) => args.push(format!("--from={dev}")),
        None => {}
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
            "update staged; rebooting into the rescue image".into()
        } else {
            text
        })
    } else {
        Err(if text.trim().is_empty() {
            "update failed".into()
        } else {
            text
        })
    }
}
