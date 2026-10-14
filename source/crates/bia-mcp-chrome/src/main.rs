//! Chrome MCP. Lifecycle + passkey native host; page automation is Google chrome-devtools-mcp.
mod bridge;
mod crx;
mod devtools;
mod frame;
mod host;
mod install;
mod pack;
mod proc;
mod watch;

use bia_mcp::{Ctx, Server};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;

fn main() {
    if std::env::args().any(|a| a == "--native-host") {
        let _ = host::run();
        return;
    }
    let dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    if std::env::args().any(|a| a == "--install-only") {
        let updates = install::Updates::new();
        match install::install(&dir, &updates) {
            Ok(v) => {
                println!("{v}");
                return;
            }
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
    }
    let updates = install::Updates::new();
    install::warm(&dir, &updates);
    install::serve(updates.clone());
    let bridge = bridge::Bridge::start(&dir);
    let bridge = Arc::new(bridge);
    let log_bridge = bridge.clone();

    let dt = match devtools::DevTools::start(&dir) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("chrome-devtools-mcp: {e}");
            std::process::exit(1);
        }
    };

    let b_status = bridge.clone();
    let b_install = bridge.clone();
    let dir_install = dir.clone();
    let updates_install = updates.clone();
    let dir_start = dir.clone();
    let updates_start = updates.clone();
    let dir_restart = dir.clone();
    let updates_restart = updates.clone();
    let dt_restart = dt.clone();
    let dt_start = dt.clone();

    let mut server = Server::new("chrome", env!("CARGO_PKG_VERSION"))
        .tool(
            "install",
            "Install the passkey Chrome add-on and restart Chrome with remote debugging on the bia profile. Page tools come from chrome-devtools-mcp. If this fails because Chrome is not installed, run apt-get install -y google-chrome-stable and call install again.",
            json!({"type": "object", "properties": {}}),
            {
                let dt = dt.clone();
                move |ctx, _a| {
                    let mut v = install::install(&dir_install, &updates_install)?;
                    proc::require()?;
                    let pid = proc::restart()?;
                    let connected = proc::wait_connected(|| b_install.connected(), 20);
                    let debug = proc::wait_debug(15);
                    if let Err(e) = dt.restart() {
                        ctx.log(format!("chrome-devtools-mcp restart: {e}"));
                    }
                    v["pid"] = json!(pid);
                    v["connected"] = json!(connected);
                    v["debug_ready"] = json!(debug);
                    v["debug_url"] = json!(proc::DEBUG_URL);
                    if !connected {
                        ctx.log("Chrome restarted but the passkey extension has not connected yet");
                    }
                    if !debug {
                        ctx.log(format!("Chrome restarted but {} is not ready", proc::DEBUG_URL));
                    }
                    Ok(v.to_string())
                }
            },
        )
        .tool(
            "status",
            "Is Chrome installed, running, remote-debugging ready, and is the passkey add-on connected.",
            json!({"type": "object", "properties": {}}),
            move |_, _| {
                let mut v = proc::status()?;
                v["connected"] = json!(b_status.connected());
                Ok(v.to_string())
            },
        )
        .tool(
            "start",
            "Start Chrome on the bia profile with remote debugging for chrome-devtools-mcp.",
            json!({"type": "object", "properties": {}}),
            {
                let b = bridge.clone();
                move |_, _a| {
                    install::publish(&dir_start, &updates_start)?;
                    let pid = proc::start()?;
                    let connected = proc::wait_connected(|| b.connected(), 20);
                    let debug = proc::wait_debug(15);
                    let _ = dt_start.restart();
                    Ok(json!({
                        "pid": pid,
                        "connected": connected,
                        "debug_ready": debug,
                        "debug_url": proc::DEBUG_URL
                    })
                    .to_string())
                }
            },
        )
        .tool(
            "restart",
            "Stop Chrome and start it again (same profile + remote debugging). Restarts chrome-devtools-mcp too.",
            json!({"type": "object", "properties": {}}),
            {
                let b = bridge.clone();
                move |_, _a| {
                    install::publish(&dir_restart, &updates_restart)?;
                    let pid = proc::restart()?;
                    let connected = proc::wait_connected(|| b.connected(), 20);
                    let debug = proc::wait_debug(15);
                    dt_restart.restart()?;
                    Ok(json!({
                        "pid": pid,
                        "connected": connected,
                        "debug_ready": debug,
                        "debug_url": proc::DEBUG_URL
                    })
                    .to_string())
                }
            },
        )
        .tool(
            "stop",
            "Stop Google Chrome.",
            json!({"type": "object", "properties": {}}),
            move |_, _| proc::stop(),
        );

    let google_tools = dt.list_tools().unwrap_or_else(|e| {
        eprintln!("chrome-devtools-mcp tools/list: {e}");
        Vec::new()
    });
    for t in google_tools {
        // Skip lifecycle name collisions; omit upload_file (use CDP DOM.setFileInputFiles).
        if matches!(
            t.name.as_str(),
            "install" | "status" | "start" | "restart" | "stop" | "upload_file"
        ) {
            continue;
        }
        let dt = dt.clone();
        let name = t.name.clone();
        let name_for_call = name.clone();
        let desc = if t.description.is_empty() {
            format!("chrome-devtools-mcp: {name}")
        } else {
            t.description
        };
        server = server.tool(&name, &desc, t.schema, move |_ctx, a| dt.call(&name_for_call, a));
    }

    let watcher = watch::Watcher::start();
    server.run_with(move |ctx: Ctx| {
        log_bridge.set_log(ctx.clone());
        watcher.set_ctx(ctx);
    });
}
