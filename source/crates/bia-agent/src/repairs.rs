//! Tell bia when a state file was found broken and restored (or kept aside). Any process
//! (the agent or an MCP) that reads state through bia_mcp::statefile appends a line to
//! files/state/repairs.log; this posts the new lines as a system|state notice.
use crate::console::{Console, Inbound};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

pub const CH_STATE: &str = "state";
const SEEN: &str = "repairs.seen";

pub fn start(state: PathBuf, con: Arc<Console>) {
    let _ = std::thread::Builder::new().name("repairs".into()).spawn(move || loop {
        if let Some(text) = check(&state) {
            con.push_inbound(
                Inbound {
                    identity: crate::tools::ID_SYSTEM.into(),
                    channel: CH_STATE.into(),
                    issued: crate::clock::now(),
                    id: String::new(),
                    text,
                }
                .ensure_id(),
            );
        }
        std::thread::sleep(Duration::from_secs(30));
    });
}

/// New repairs since the last look → the notice.
fn check(state: &Path) -> Option<String> {
    let seen = std::fs::read_to_string(state.join(SEEN)).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
    let (lines, at) = bia_mcp::statefile::repairs_since(state, seen);
    if at != seen {
        let _ = std::fs::write(state.join(SEEN), at.to_string());
    }
    if lines.is_empty() {
        return None;
    }
    Some(format!(
        "[state] a saved state file was found broken (another program may have written over it):\n{}\n\
         Check that what it holds looks right (for schedules: schedule_list) before changing or recreating \
         anything, and tell root what happened. Files of your own programs belong outside files/state.",
        lines.iter().map(|l| format!("- {l}")).collect::<Vec<_>>().join("\n")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_each_repair_once() {
        let d = std::env::temp_dir().join(format!("bia-repairs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        assert!(check(&d).is_none());
        let p = d.join("x.json");
        std::fs::write(&p, b"{").unwrap();
        let _ = bia_mcp::statefile::read::<serde_json::Value>(&p);
        let n = check(&d).unwrap();
        assert!(n.contains("x.json could not be read"), "{n}");
        assert!(check(&d).is_none());
        let _ = std::fs::remove_dir_all(&d);
    }
}
