//! Conversation persistence: /files/state/conversation.json
use crate::console::Inbound;
use crate::tools::{Msg, Role};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

#[derive(Serialize, Deserialize)]
struct SavedMsg {
    role: String,
    content: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    tool_calls: Vec<SavedCall>,
    #[serde(default)]
    call_id: String,
    #[serde(default)]
    id: String,
}

#[derive(Serialize, Deserialize)]
struct SavedCall {
    id: String,
    name: String,
    args: serde_json::Value,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SavedPending {
    pub issued: u64,
    #[serde(default = "default_root")]
    pub identity: String,
    #[serde(default = "default_console")]
    pub channel: String,
    #[serde(default)]
    pub id: String,
    pub text: String,
}

fn default_root() -> String {
    "root".into()
}
fn default_console() -> String {
    "console".into()
}

impl SavedPending {
    pub fn to_inbound(&self) -> Inbound {
        Inbound {
            identity: self.identity.clone(),
            channel: self.channel.clone(),
            issued: UNIX_EPOCH + Duration::from_secs(self.issued),
            id: self.id.clone(),
            text: self.text.clone(),
        }
        .ensure_id()
    }
    pub fn from_inbound(ib: &Inbound) -> SavedPending {
        SavedPending {
            issued: crate::clock::unix(ib.issued),
            identity: ib.identity.clone(),
            channel: ib.channel.clone(),
            id: ib.id.clone(),
            text: ib.text.clone(),
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct Saved {
    pub version: u32,
    pub saved_at: u64,
    pub model: String,
    #[serde(default)]
    pub loaded_mcps: Vec<String>,
    /// messages not yet delivered to the model
    #[serde(default)]
    pub pending: Vec<SavedPending>,
    messages: Vec<SavedMsg>,
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn save(
    path: &Path,
    model: &str,
    history: &[Msg],
    loaded_mcps: Vec<String>,
    pending: Vec<SavedPending>,
) -> Result<()> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    let s = Saved {
        version: 1,
        saved_at: now(),
        model: model.to_string(),
        loaded_mcps,
        pending,
        messages: history
            .iter()
            .map(|m| SavedMsg {
                role: match m.role {
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    Role::Tool => "tool",
                }
                .into(),
                content: m.content.clone(),
                name: m.name.clone(),
                tool_calls: m
                    .tool_calls
                    .iter()
                    .map(|c| SavedCall {
                        id: c.id.clone(),
                        name: c.name.clone(),
                        args: c.args.clone(),
                    })
                    .collect(),
                call_id: m.call_id.clone(),
                id: m.id.clone(),
            })
            .collect(),
    };
    let tmp = path.with_extension("json.tmp");
    let bak = path.with_extension("json.bak");
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&serde_json::to_vec_pretty(&s)?)?;
        f.sync_all()?; // data + metadata on disk before it replaces the previous file
    }
    if path.exists() {
        let _ = std::fs::remove_file(&bak);
        let _ = std::fs::rename(path, &bak);
    }
    std::fs::rename(&tmp, path).or_else(|_| std::fs::copy(&tmp, path).map(|_| ()))?;
    let _ = std::fs::remove_file(&tmp);
    unsafe { libc::sync() }; // survive a hard power-off
    Ok(())
}

/// Load the conversation; falls back to the previous copy (.bak) if the file is damaged.
/// Returns the saved state, the messages and a note when the backup was used.
pub fn load(path: &Path) -> Result<(Saved, Vec<Msg>, Option<String>)> {
    match load_file(path) {
        Ok((s, h)) => Ok((s, h, None)),
        Err(e) => {
            let bak = path.with_extension("json.bak");
            let (s, h) = load_file(&bak).with_context(|| format!("{e:#}; backup also unusable"))?;
            Ok((
                s,
                h,
                Some(format!(
                    "conversation.json was damaged ({e:#}); restored the previous copy"
                )),
            ))
        }
    }
}

fn load_file(path: &Path) -> Result<(Saved, Vec<Msg>)> {
    let data = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    if data.is_empty() {
        anyhow::bail!("{} is empty", path.display());
    }
    let s: Saved =
        serde_json::from_slice(&data).with_context(|| format!("parse {}", path.display()))?;
    let history = s
        .messages
        .iter()
        .map(|m| Msg {
            role: match m.role.as_str() {
                "assistant" => Role::Assistant,
                "tool" => Role::Tool,
                _ => Role::User,
            },
            content: m.content.clone(),
            name: m.name.clone(),
            tool_calls: m
                .tool_calls
                .iter()
                .map(|c| crate::tools::ToolCall {
                    id: c.id.clone(),
                    name: c.name.clone(),
                    args: c.args.clone(),
                })
                .collect(),
            call_id: m.call_id.clone(),
            id: m.id.clone(),
        })
        .collect();
    Ok((s, history))
}

pub fn forget(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// Written just before a commanded reboot/poweroff; consumed on the next boot.
pub enum LastExit {
    Poweroff,
    Reboot,
}

pub fn mark_exit(path: &Path, poweroff: bool) {
    if let Some(p) = path.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    let kind = if poweroff { "poweroff" } else { "reboot" };
    if std::fs::write(path, kind).is_ok() {
        unsafe { libc::sync() };
    }
}

/// Read and delete the clean-exit marker. `None` means the previous run did not shut down cleanly.
pub fn take_exit(path: &Path) -> Option<LastExit> {
    let s = std::fs::read_to_string(path).ok()?;
    let _ = std::fs::remove_file(path);
    match s.trim() {
        "poweroff" => Some(LastExit::Poweroff),
        "reboot" => Some(LastExit::Reboot),
        _ => None,
    }
}
