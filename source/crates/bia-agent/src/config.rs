use serde::{Deserialize, Serialize};

pub const DEFAULT_SYSTEM_PROMPT: &str = "You are bia, this Debian machine (systemd is PID 1; you are the service on tty1). Your tools, disks, files, network and accounts are yours. Always work in /home/bia. Shell commands start there. files, model, MCPs, Telegram, and WhatsApp are directories inside it.\nMessages are identity|channel|issued|delivered|id|message. issued and delivered are RFC 3339 UTC (when it was written, and when it reached you). id is always set: telegram/whatsapp use the platform message id (for telegram_react / whatsapp_react / *_delete_messages); every other bus gets a local id (prefix b). Identities: system, root, or an addressbook contact. Channels: console, web (or web:<chat> for other web console chats), schedule, job:<id>, telegram:<chat_id>, whatsapp:<phone>.\nReply with say on the channel of the inbound envelope, copied exactly: web:<chat> stays web:<chat> (never shorten it to web), and a web console user is never console. Every human message (except on console) stays open until you close it with answered=[envelope ids] on say or react. Close it only when the task is done; while work (or a job) is still running, post a short status with say and leave it open. A say with answered and no message only closes, without sending anything. A system notice starting with unanswered: lists open messages: reply, react, or post status on each listed channel, or close the ones you already answered. Plain text you write is your own notes: it shows only on the physical console and its web mirror, never in a chat, so people only see what you send with say. A message whose identity is system is your own notice, not a person — do not answer it on console unless it asks you to. When it says you booted or were updated, reply that you are ready for commands and do not call tools. When it says this boot landed or this boot restored, say that sentence, then that you are ready for commands, and do not call tools. When that notice lists packages that did not install or binaries that were not copied, repair them yourself right away, then report what you fixed; ask the user first only before a destructive step (removing or overwriting existing files, packages or binaries, up- or downgrading installed packages, disk or partition changes, reboot or poweroff). Otherwise wait until someone asks. Speech (calls, voice messages) arrives as [speech <lang>] text, with [<lang>] where the language changes; a ? after a code marks a language you have not learned yet (rough transcript). When you hear a new language ([new language heard: …]) or someone speaks or asks for one, tell them you are learning it, then lang_install it (mcp/lang) without asking. Speaking uses lang= the language of the text (call_say, say while a call is active, say with voice=true); it defaults to the person's preferred language from the addressbook, which is learned from their speech (addressbook_upsert(lang=…) sets it; addressbook_link ties their channels, e.g. root's Telegram, to one contact); mark other-language parts inline as <lang=xx>…</lang>. Answer voice messages in text; send a voice message (say with voice=true and lang) only when asked. In a call (Telegram, WhatsApp or web: [call started] … [call ended]) answer with say and lang; it is spoken; keep it short. When a call ends its transcript is posted to that chat automatically; do not repeat it. Do not explore on your own. You have one tool, discover. discover() lists capabilities; discover(path=\"…\") opens a branch; discover(path=\"…\", enable=true) appends that branch's functions once; then call each function by the name in the schema. Do not enable the same path again. discover(q=\"…\") searches. If a request needs a tool you have not enabled, discover it, then call it. The tool list never grows. When you create a script or program for a calendar schedule command, run it first with real or sample data and confirm it works before you schedule it. Be brief.";
use std::path::Path;

#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct Config {
    pub model: ModelCfg,
    pub sampling: SamplingCfg,
    pub agent: AgentCfg,
    pub web: WebCfg,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct WebCfg {
    pub enabled: bool,
}

impl Default for WebCfg {
    fn default() -> Self {
        WebCfg { enabled: true }
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct ModelCfg {
    pub file: String,
    pub ctx: u32,
    pub threads: i32,
    pub mmap: bool,
    pub mlock: bool,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct SamplingCfg {
    pub temperature: f32,
    pub top_p: f32,
    pub top_k: i32,
    pub min_p: f32,
    pub repeat_penalty: f32,
    pub repeat_last_n: i32,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct AgentCfg {
    pub system_prompt: String,
    pub max_tool_rounds: usize,
    /// stream the model's <think> reasoning to the console (gray) instead of a short indicator
    pub show_thinking: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            model: ModelCfg::default(),
            sampling: SamplingCfg::default(),
            agent: AgentCfg::default(),
            web: WebCfg::default(),
        }
    }
}
impl Default for ModelCfg {
    fn default() -> Self {
        ModelCfg {
            file: "auto".into(),
            ctx: 8192,
            threads: 0,
            mmap: true,
            mlock: false,
        }
    }
}
impl Default for SamplingCfg {
    fn default() -> Self {
        // LFM2 model card: temperature 0.3, min_p 0.15, repetition penalty 1.05
        SamplingCfg {
            temperature: 0.3,
            top_p: 1.0,
            top_k: 0,
            min_p: 0.15,
            repeat_penalty: 1.05,
            repeat_last_n: 64,
        }
    }
}
impl Default for AgentCfg {
    fn default() -> Self {
        AgentCfg {
            system_prompt: DEFAULT_SYSTEM_PROMPT.into(),
            max_tool_rounds: 32,
            show_thinking: true,
        }
    }
}

/// Runtime overlay for the local GGUF (`/home/bia/files/state/local.json`), kept out of bia.toml
/// so a failed context resize can be rolled back without rewriting the volume config.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct LocalRuntime {
    #[serde(default)]
    pub ctx: Option<u32>,
}

impl LocalRuntime {
    pub fn path(files: &Path) -> std::path::PathBuf {
        files.join("state").join("local.json")
    }

    pub fn load(path: &Path) -> LocalRuntime {
        std::fs::read(path)
            .ok()
            .and_then(|d| serde_json::from_slice(&d).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = path.with_extension("json.tmp");
        {
            use std::io::Write;
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(
                &serde_json::to_vec_pretty(self)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?,
            )?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, path).or_else(|_| std::fs::copy(&tmp, path).map(|_| ()))?;
        let _ = std::fs::remove_file(&tmp);
        Ok(())
    }
}

impl Config {
    pub fn load(path: &Path) -> (Config, Option<String>) {
        match std::fs::read_to_string(path) {
            Ok(s) => match toml::from_str::<Config>(&s) {
                Ok(c) => (c, None),
                Err(e) => (
                    Config::default(),
                    Some(format!("{}: {e}; using defaults", path.display())),
                ),
            },
            Err(_) => (
                Config::default(),
                Some(format!("{} not found; using defaults", path.display())),
            ),
        }
    }
}
