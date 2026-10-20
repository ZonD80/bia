//! Carry: fold a backup, unfold it, or fly it onto a machine that is landing.
mod archive;
mod debs;
mod fly;
mod migrate;
mod morph;
mod rescue;
mod update;

use bia_mcp::{arg_bool, arg_str, arg_str_opt, arg_u32_opt, Server};
use serde_json::json;
use std::io::{BufRead, BufReader, Write};
use std::time::Duration;

fn console_line(s: &str) {
    eprintln!("{s}");
    let _ = std::io::stderr().flush();
}

fn watch_code(path: &str) {
    let Ok(f) = std::fs::File::open(path) else { return };
    let mut lines = BufReader::new(f);
    loop {
        let mut line = String::new();
        match lines.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                let t = line.trim();
                if t.is_empty() {
                    continue;
                }
                match fly::confirm(t) {
                    Ok(s) => console_line(&s),
                    Err(e) => console_line(&e),
                }
            }
        }
    }
}

/// The first VT plus every active kernel console (ttyS0 on x86, ttyAMA0 on arm64).
fn land_terminals() -> Vec<String> {
    let mut v = vec!["/dev/tty1".to_string()];
    let active = std::fs::read_to_string("/sys/class/tty/console/active").unwrap_or_default();
    for c in active.split_whitespace() {
        let dev = format!("/dev/{c}");
        if c != "tty0" && !v.contains(&dev) && std::path::Path::new(&dev).exists() {
            v.push(dev);
        }
    }
    if v.len() == 1 {
        v.push("/dev/ttyS0".into());
    }
    v
}

fn port_arg(args: &serde_json::Value) -> u16 {
    arg_u32_opt(args, "port").and_then(|n| u16::try_from(n).ok()).filter(|n| *n != 0).unwrap_or(fly::TCP_PORT)
}

fn timeout_arg(args: &serde_json::Value, default: u64) -> Duration {
    let secs = arg_u32_opt(args, "timeout_secs").map(|n| n as u64).filter(|n| *n > 0).unwrap_or(default);
    Duration::from_secs(secs.min(7200))
}

fn main() {
    if std::env::args().any(|a| a == "--land") {
        let ctx = bia_mcp::Ctx::console();
        for path in land_terminals() {
            std::thread::spawn(move || watch_code(&path));
        }
        match fly::land(&ctx, fly::TCP_PORT, Duration::from_secs(30 * 24 * 3600)) {
            Ok(s) => {
                console_line(&s);
            }
            Err(e) => {
                console_line(&format!("land: {e}"));
                std::process::exit(1);
            }
        }
        return;
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--fold-to") {
        let ctx = bia_mcp::Ctx::console();
        let dest = args.get(1).map(String::as_str).unwrap_or("");
        if dest.is_empty() {
            console_line("fold-to needs a path");
            std::process::exit(1);
        }
        match archive::fold_into(&ctx, std::path::Path::new(dest)) {
            Ok(n) => console_line(&format!("{dest}\n{n} bytes")),
            Err(e) => {
                console_line(&format!("fold: {e}"));
                std::process::exit(1);
            }
        }
        return;
    }
    if args.first().map(String::as_str) == Some("--finish-update") {
        let ctx = bia_mcp::Ctx::console();
        let tar = args.get(1).map(String::as_str).unwrap_or("");
        let stash = args.get(2).map(String::as_str).unwrap_or("");
        if tar.is_empty() || stash.is_empty() {
            console_line("finish-update needs the archive path and the local MCP directory");
            std::process::exit(1);
        }
        match archive::finish_update(&ctx, std::path::Path::new(tar), std::path::Path::new(stash)) {
            Ok(s) => console_line(&s),
            Err(e) => {
                console_line(&format!("update: {e}"));
                std::process::exit(1);
            }
        }
        return;
    }
    if args.first().map(String::as_str) == Some("--fold-from") {
        let ctx = bia_mcp::Ctx::console();
        let root = args.get(1).map(String::as_str).unwrap_or("");
        let dest = args.get(2).map(String::as_str).unwrap_or("");
        if root.is_empty() || dest.is_empty() {
            console_line("fold-from needs the root mount and the archive path");
            std::process::exit(1);
        }
        match archive::fold_root(&ctx, std::path::Path::new(root), std::path::Path::new(dest)) {
            Ok(n) => console_line(&format!("{dest}\n{n} bytes")),
            Err(e) => {
                console_line(&format!("fold: {e}"));
                std::process::exit(1);
            }
        }
        return;
    }
    Server::new("carry", env!("CARGO_PKG_VERSION"))
        .tool(
            "fold",
            "Save a gzip backup of /home/bia under /home/bia/files. It includes settings, chats, passwords, Chrome, Telegram, the model, and MCP binaries for aarch64 and x86_64. It also lists each manually installed package with its version, architecture, and apt origin. The file is named name.unfold-<bytes>.tar.gz, where <bytes> is the unpacked size. Refuses when the disk has less free space than /home/bia. If it refuses for space, ask the user and call again with force=true only after they confirm. Returns the file path. The file contains passwords.",
            json!({"type": "object", "properties": {
                "path": {"type": "string", "description": "where to write the backup, under /home/bia/files. Omit for carry/fold-<time>.unfold-<bytes>.tar.gz"},
                "force": {"type": "boolean", "description": "true folds even when free space is below the folder size. Ask the user first. Default false."}
            }}),
            |ctx, a| archive::fold(ctx, arg_str_opt(&a, "path"), arg_bool(&a, "force", false)),
        )
        .tool(
            "unfold",
            "Read a gzip backup and put it on this computer, then reboot. path is the backup file under /home/bia/files. A name like name.unfold-<bytes>.tar.gz carries the unpacked size. An older backup without that size still unfolds: ask the user to confirm, then call again with force=true. Also ask and pass force=true when free space is below <bytes>. Do not pass force before they confirm. MCP binaries already on this machine are kept; the archive only fills missing ones. After reboot, say that this boot restored from a carry archive. If packages did not install or MCP binaries were not copied, repair them yourself after reboot; ask the user first only before a destructive step.",
            json!({"type": "object", "required": ["path"], "properties": {
                "path": {"type": "string", "description": "backup file under /home/bia/files. Older names without .unfold-<bytes> need force after the user confirms."},
                "force": {"type": "boolean", "description": "true unfolds an older backup that has no unpacked size in the name, or unfolds when free space is below that size. Ask the user first. Default false."}
            }}),
            |ctx, a| archive::unfold(ctx, arg_str(&a, "path")?, archive::Mode::Restore, arg_bool(&a, "force", false)),
        )
        .tool(
            "discover",
            "List computers on this network that are waiting to receive a backup. Returns ip and name, one per line. Empty means nobody is waiting.",
            json!({"type": "object", "properties": {
                "timeout_secs": {"type": "integer", "description": "seconds to listen. Default 3."}
            }}),
            |ctx, a| {
                let hosts = fly::discover(ctx, timeout_arg(&a, 3));
                if hosts.is_empty() {
                    Ok("no machine is landing on this network".into())
                } else {
                    Ok(hosts.iter().map(|h| h.label()).collect::<Vec<_>>().join("\n"))
                }
            },
        )
        .tool(
            "land",
            "Wait here for another computer to send its backup. Both screens show the same 6-digit code. Call carry_confirm with that code only after the user says the two screens match. Then this computer reboots with the other computer's settings. Network address and hostname stay.",
            json!({"type": "object", "properties": {
                "port": {"type": "integer", "description": "optional. Default 4791."},
                "timeout_secs": {"type": "integer", "description": "give up after this many seconds if nobody connects. Default 600."}
            }}),
            |ctx, a| fly::land(ctx, port_arg(&a), timeout_arg(&a, 600)),
        )
        .tool(
            "fly",
            "Send this computer's backup to a computer that is waiting. host is the ip from carry_discover. If you omit host and exactly one computer is waiting, send to that one. If several are waiting, do not send. Both screens show a 6-digit code. Call carry_confirm only after the user says they match. A server taken over in land mode (by root, or by you: carry_new_land_secret) waits somewhere on the internet with a land secret instead: pass its address as host and the land secret as secret, with timeout_secs up to 1800 while it is still installing; then no code is shown, both sides prove the secret and the backup travels encrypted. This computer keeps running. The other one reboots.",
            json!({"type": "object", "properties": {
                "host": {"type": "string", "description": "ip or name of the waiting computer. Omit to look for one on this network."},
                "port": {"type": "integer", "description": "optional. Default 4791."},
                "secret": {"type": "string", "description": "the land secret root got when taking over that server (only for such servers)"},
                "timeout_secs": {"type": "integer", "description": "seconds to wait for the connection. Default 30."}
            }}),
            |ctx, a| fly::fly_with(ctx, arg_str_opt(&a, "host"), arg_u32_opt(&a, "port").and_then(|n| u16::try_from(n).ok()), timeout_arg(&a, 30), arg_str_opt(&a, "secret")),
        )
        .tool(
            "new_land_secret",
            "Start making a server you reach over ssh (Ubuntu, Debian; not a bia) a land for this agent: makes a land secret (140 bits), saves it for host, and returns the exact steps (check over ssh, takeover.sh --land, carry_fly). Ask root first: the takeover erases that server. The secret is deleted (here too) once an agent has landed.",
            json!({"type": "object", "properties": {
                "host": {"type": "string", "description": "the server's address or name: save the secret for it"}
            }}),
            |_, a| fly::new_secret_for(arg_str_opt(&a, "host")),
        )
        .tool(
            "confirm",
            "Send the 6-digit code shown on this screen. Call this only after the user says it matches the other screen. A wrong code cancels the transfer.",
            json!({"type": "object", "required": ["code"], "properties": {
                "code": {"type": "string", "description": "the 6 digits on this screen"}
            }}),
            |_, a| fly::confirm(arg_str(&a, "code")?),
        )
        .tool(
            "morph",
            "Fold an agent from another disk onto this boot, then reboot. Required: disk=/dev/... for the partition that holds that agent (use bash lsblk -f). Refuses if disk is on the same physical disk as /.",
            json!({"type": "object", "required": ["disk"], "properties": {
                "disk": {"type": "string", "description": "block device with the agent to fold, e.g. /dev/sda2"}
            }}),
            |ctx, a| morph::morph(ctx, Some(arg_str(&a, "disk")?)),
        )
        .tool(
            "update",
            "Update this computer's OS. Source: the other bia system attached to it (the flash stick when running from the internal disk, or the internal disk when running from the flash) — preferred; when none is attached, the newest bia release from GitHub over the internet (downloaded, checked and unpacked first; needs about twice the image size free). Folds this agent into /bia-update, reboots into the rescue image, which copies the new system onto this one while it does not run, then this computer boots and unfolds the agent (settings, chats, files, packages). The source must have a higher version unless force=true. Not automatic; call only when the user asks to update. The computer reboots several times; tell the user before calling.",
            json!({"type": "object", "properties": {
                "from": {"type": "string", "description": "optional: a partition to update from, e.g. /dev/sdb3 (lsblk -f), or internet for the newest GitHub release even when a flash stick is attached. Omit to use the attached bia system, else the internet."},
                "force": {"type": "boolean", "description": "true installs the source even when its release is not newer. Ask the user first. Default false."}
            }}),
            |ctx, a| update::update(ctx, arg_str_opt(&a, "from"), arg_bool(&a, "force", false)),
        )
        .tool(
            "rescue",
            "From a flash boot: repair or overwrite the internal disk (BIADISK) with this flash system. Folds the disk's agent into the disk's /bia-update, reboots into the rescue image, which copies this flash onto the disk offline and refreshes its bootloader, then the disk boots and unfolds its agent. Use when the disk will not boot. For a normal version bump prefer carry_update on the system being updated. Optional disk=/dev/... for the BIADISK partition or whole disk; omit to find LABEL=BIADISK.",
            json!({"type": "object", "properties": {
                "disk": {"type": "string", "description": "BIADISK partition or whole disk, e.g. /dev/sda2 or /dev/sda. Omit to find LABEL=BIADISK."}
            }}),
            |ctx, a| rescue::rescue(ctx, arg_str_opt(&a, "disk")),
        )
        .tool(
            "to_disk",
            "Copy this whole computer onto a disk so that disk can boot. Required: disk=/dev/sda (whole disk), or partition=/dev/... (disk is inferred). Does not change the disk you are using now. Then reboots into that disk (BootNext 'bia disk'). On a legacy BIOS boot it powers off instead: tell the user to remove the flash stick and turn the computer on. Empty disk: full EFI+root wipe install. Windows already there: also pass partition=/dev/... , or shrink_from+size_mib, or size_mib alone; disk alone on Windows lists volumes. wipe=true erases the whole disk including Windows. Choose devices with bash lsblk.",
            json!({"type": "object", "properties": {
                "disk": {"type": "string", "description": "whole-disk device, e.g. /dev/sda or /dev/nvme0n1"},
                "partition": {"type": "string", "description": "existing block device to format as BIADISK (Windows-coexist). Not the Windows system volume."},
                "shrink_from": {"type": "string", "description": "partition to shrink; creates a new BIADISK of size_mib after it"},
                "size_mib": {"type": "integer", "description": "MiB for the new BIADISK (with shrink_from, or alone from free space)"},
                "wipe": {"type": "boolean", "description": "if true, erase the whole disk even when Windows is present"}
            }}),
            |ctx, a| {
                migrate::to_disk(
                    ctx,
                    migrate::ToDiskArgs {
                        disk: arg_str_opt(&a, "disk").map(|s| s.to_string()),
                        partition: arg_str_opt(&a, "partition").map(|s| s.to_string()),
                        shrink_from: arg_str_opt(&a, "shrink_from").map(|s| s.to_string()),
                        size_mib: arg_u32_opt(&a, "size_mib"),
                        wipe: arg_bool(&a, "wipe", false),
                    },
                )
            },
        )
        .tool(
            "to_flash",
            "Copy this whole computer onto a flash/USB drive so that drive can boot. Required: disk=/dev/... for the stick (lsblk). Does not change the disk you are using now. Then shut down.",
            json!({"type": "object", "required": ["disk"], "properties": {
                "disk": {"type": "string", "description": "whole-disk device for the flash stick, e.g. /dev/sdb"}
            }}),
            |ctx, a| migrate::to_flash(ctx, Some(arg_str(&a, "disk")?)),
        )
        .run();
}
