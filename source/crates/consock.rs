//! Where the agent serves its console mirror (a Unix socket) for root's web console.
//! Shared by the agent (listens) and the web MCP (connects).
//!
//! Protocol: the client first sends one JSON line `{"rows":R,"cols":C}`, then more JSON lines
//! (`{"t":"resize","rows","cols"}`, `{"t":"line","v"}`); the agent answers with raw console bytes.
use std::path::{Path, PathBuf};

/// `/run/bia/console.sock` when running as root, else `<files>/state/console.sock` (dev, tests).
pub fn path(files: &Path) -> PathBuf {
    if unsafe { libc::geteuid() } == 0 && Path::new("/run").is_dir() {
        PathBuf::from("/run/bia/console.sock")
    } else {
        files.join("state").join("console.sock")
    }
}
