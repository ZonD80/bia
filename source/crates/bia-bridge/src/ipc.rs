//! Commands and MCP sessions talk to the profile's daemon, starting it when it is not
//! running (the default; `--no-daemon` avoids it where a command can work without).
use crate::profile::Profile;
use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

fn info(p: &Profile) -> Result<(u16, String)> {
    let v: Value = serde_json::from_slice(&std::fs::read(p.path("daemon.json")).context("no daemon")?)?;
    Ok((v["port"].as_u64().unwrap_or(0) as u16, v["token"].as_str().unwrap_or("").to_string()))
}

fn raw(p: &Profile, mut req: Value, timeout: Duration) -> Result<Value> {
    let (port, token) = info(p)?;
    let mut s = TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_secs(2))?;
    s.set_read_timeout(Some(timeout))?;
    req["token"] = json!(token);
    writeln!(s, "{req}")?;
    let mut line = String::new();
    BufReader::new(s).read_line(&mut line)?;
    if line.trim().is_empty() {
        bail!("the daemon closed the connection");
    }
    Ok(serde_json::from_str(&line)?)
}

pub fn ping(p: &Profile) -> Result<()> {
    let v = raw(p, json!({"op": "ping"}), Duration::from_secs(3))?;
    if v["ok"] == true {
        Ok(())
    } else {
        Err(anyhow!("no daemon"))
    }
}

/// Start the profile's daemon in the background unless it runs.
pub fn ensure(p: &Profile) -> Result<()> {
    if ping(p).is_ok() {
        return Ok(());
    }
    let exe = std::env::current_exe()?;
    let mut c = std::process::Command::new(exe);
    c.args(["daemon", "--bia", &p.name])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // its own process group: it outlives the shell or agent session that started it
        c.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    }
    c.spawn().context("start the daemon")?;
    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        if ping(p).is_ok() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    bail!("the daemon did not start (see {})", p.path("daemon.log").display())
}

/// One request to the daemon (started if needed). `wait` requests block up to their timeout.
pub fn call(p: &Profile, req: Value) -> Result<Value> {
    ensure(p)?;
    let t = req["timeout"].as_u64().unwrap_or(0) + 60;
    let v = raw(p, req, Duration::from_secs(t))?;
    if v["ok"] != true {
        bail!("{}", v["error"].as_str().unwrap_or("failed"));
    }
    Ok(v)
}

pub fn stop(p: &Profile) -> Result<bool> {
    if ping(p).is_err() {
        return Ok(false);
    }
    let _ = raw(p, json!({"op": "stop"}), Duration::from_secs(5));
    Ok(true)
}
