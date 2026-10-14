use bia_mcp::Ctx;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

fn run(ctx: &Ctx, args: &[&str]) -> Result<String, String> {
    let mut child = Command::new("/usr/local/sbin/bia-migrate")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("bia-migrate: {e}"))?;
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
    let status = child.wait().map_err(|e| format!("bia-migrate: {e}"))?;
    let _ = logged.join();
    let label = args.first().copied().unwrap_or("?");
    if status.success() {
        Ok(if text.trim().is_empty() {
            format!("bia-migrate {label} finished")
        } else {
            text
        })
    } else {
        Err(if text.trim().is_empty() {
            format!("bia-migrate {label} failed")
        } else {
            text
        })
    }
}

pub struct ToDiskArgs {
    pub disk: Option<String>,
    pub partition: Option<String>,
    pub shrink_from: Option<String>,
    pub size_mib: Option<u32>,
    pub wipe: bool,
}

pub fn to_disk(ctx: &Ctx, a: ToDiskArgs) -> Result<String, String> {
    let mut args: Vec<String> = vec!["to-disk".into()];
    if let Some(d) = a.disk {
        if !d.is_empty() {
            args.push(format!("--disk={d}"));
        }
    }
    if a.wipe {
        args.push("--wipe".into());
    }
    if let Some(p) = a.partition {
        if !p.is_empty() {
            args.push(format!("--partition={p}"));
        }
    }
    if let Some(p) = a.shrink_from {
        if !p.is_empty() {
            args.push(format!("--shrink-from={p}"));
        }
    }
    if let Some(n) = a.size_mib {
        if n > 0 {
            args.push(format!("--size-mib={n}"));
        }
    }
    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    run(ctx, &refs)
}

pub fn to_flash(ctx: &Ctx, disk: Option<&str>) -> Result<String, String> {
    let mut args: Vec<String> = vec!["to-flash".into()];
    if let Some(d) = disk {
        if !d.is_empty() {
            args.push(format!("--disk={d}"));
        }
    }
    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    run(ctx, &refs)
}
