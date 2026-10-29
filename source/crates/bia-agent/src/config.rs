use serde::{Deserialize, Serialize};

pub const DEFAULT_SYSTEM_PROMPT: &str = "You are bia, this Debian machine (systemd is PID 1; you are the service on tty1). Its tools, disks, files, network and accounts are yours. Work in /home/bia: files, model, MCPs, Telegram and WhatsApp are inside it. Messages are identity|channel|issued|delivered|id|message (times are RFC 3339 UTC). identity is system (your own notice, not a person), root, or an addressbook contact. Channels: console (the physical screen), web or web:<chat>, telegram:<chat_id>, whatsapp:<phone>, messenger:<chat_id> (Facebook), instagram:<chat_id>, schedule, job:<id>. People see only what you send with say; plain text you write is your own notes on the console. Reply with say on the inbound channel, copied exactly (web:<chat> is not web). Close each human message with answered=[ids] on say or react once its task is done; while still working, send a short status and leave it open. An unanswered: notice lists the open ones. A boot or update notice: reply that you are ready for commands, with no tools; a landed or restored notice: say that sentence first. Messages delivered with it are normal orders. If it lists packages or binaries that failed, repair them right away and report; ask first only before a destructive step (removing or overwriting, up- or downgrades, disk changes, reboot). Otherwise do not explore on your own. Speech arrives as [speech <lang>] text. On [new language heard: …], or when someone asks for a language, tell them you are learning it, then lang_install it (mcp/lang). Answer voice messages in text; send voice only when asked. In a call keep it short; its transcript is posted for you. You have one tool, discover. discover() lists capabilities; discover(path=\"…\") opens a branch; discover(path=\"…\", enable=true) appends that branch's functions once; then call each function by the name in the schema. Do not enable the same path again. discover(q=\"…\") searches. If a request needs a tool you have not enabled, discover it, then call it. An API key or a named cloud model or provider means fmm_configure(api_key, provider, model) (mcp/system) with the values from that message (provider and model optional; do not ask again); a ChatGPT plan without a key means fmm_chatgpt(). Do not say you cannot. Run a script with real or sample data before you schedule it. Each feature you build or MCP you install lives in its own MCPs/<name>/ (my- prefix, never a shipped name) with all its scripts, works on aarch64 and x86_64, keeps data in files/state/<name>/ and keys in secrets, and lists its needs in README.md, so it can be shared. Start from MCPs/.template. Be brief.";
use std::path::Path;

#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
#[derive(Default)]
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

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
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
            max_tool_rounds: 100,
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
    /// set_model_config: sampling of the local model instead of bia.toml's [sampling]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sampling: Option<SamplingCfg>,
}

impl LocalRuntime {
    pub fn path(files: &Path) -> std::path::PathBuf {
        files.join("state").join("local.json")
    }

    pub fn load(path: &Path) -> LocalRuntime {
        load_json(path)
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        save_json(self, path)
    }
}

/// Runtime overlay for the agent (`/home/bia/files/state/agent.json`): what the model set with
/// set_model_config that is not about one brain.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct AgentRuntime {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tool_rounds: Option<usize>,
}

impl AgentRuntime {
    pub fn path(files: &Path) -> std::path::PathBuf {
        files.join("state").join("agent.json")
    }

    pub fn load(path: &Path) -> AgentRuntime {
        load_json(path)
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        save_json(self, path)
    }
}

fn load_json<T: serde::de::DeserializeOwned + Default>(path: &Path) -> T {
    std::fs::read(path)
        .ok()
        .and_then(|d| serde_json::from_slice(&d).ok())
        .unwrap_or_default()
}

fn save_json<T: Serialize>(v: &T, path: &Path) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension("json.tmp");
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&serde_json::to_vec_pretty(v).map_err(std::io::Error::other)?)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path).or_else(|_| std::fs::copy(&tmp, path).map(|_| ()))?;
    let _ = std::fs::remove_file(&tmp);
    Ok(())
}

impl Config {
    pub fn load(path: &Path) -> (Config, Option<String>) {
        match std::fs::read_to_string(path) {
            Ok(s) => match toml::from_str::<Config>(&s) {
                Ok(mut c) => {
                    let stock = c.agent.system_prompt != DEFAULT_SYSTEM_PROMPT
                        && SHIPPED_PROMPTS.contains(&sha256_hex(&c.agent.system_prompt).as_str());
                    if stock {
                        c.agent.system_prompt = DEFAULT_SYSTEM_PROMPT.into();
                    }
                    // 32 was the shipped bia.toml value before 1.2.2: it means "the default"
                    if c.agent.max_tool_rounds == 32 {
                        c.agent.max_tool_rounds = AgentCfg::default().max_tool_rounds;
                    }
                    (c, None)
                }
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

/// sha256 of every system_prompt an earlier bia.toml shipped with (1.0.0a pre-release, 1.0.0a,
/// 1.1.0, 1.1.1). Updates keep the machine's bia.toml, so such a stock copy would hide the
/// built-in prompt forever; it is replaced by DEFAULT_SYSTEM_PROMPT. A prompt the owner wrote
/// (any other hash) is kept.
const SHIPPED_PROMPTS: &[&str] = &[
    "9eb3e41465ab06d314e219c41158dc932bd83d585db154107dd2fc626b6a1bd3",
    "0901672f6c93071e5e4be98b31d51d20865a1360bb3c1b731bf182ae630256d2",
    "e96efad69512125a8bf8876864c873c40764d74e4f517276f226f976984fbe84",
    "de2bb6ed6b9a9755ab51d5f1a67363bb741c5e1d1ea69bae379f3692d1190eaa",
];

fn sha256_hex(s: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(s.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod prompt_tests {
    use super::*;

    fn load_toml(body: &str) -> Config {
        // one folder per call: tests run in parallel, and two bodies can be the same length
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("bia-cfg-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("bia.toml");
        std::fs::write(&p, body).unwrap();
        let c = Config::load(&p).0;
        let _ = std::fs::remove_dir_all(&dir);
        c
    }

    #[test]
    fn shipped_bia_toml_uses_the_built_in_prompt() {
        let shipped = include_str!("../../../flash/bia.toml");
        assert_eq!(load_toml(shipped).agent.system_prompt, DEFAULT_SYSTEM_PROMPT);
    }

    #[test]
    fn tool_rounds_default_to_100_and_the_old_stock_32_counts_as_default() {
        assert_eq!(load_toml(include_str!("../../../flash/bia.toml")).agent.max_tool_rounds, 100);
        assert_eq!(load_toml("[agent]\nmax_tool_rounds = 32\n").agent.max_tool_rounds, 100);
        assert_eq!(load_toml("[agent]\nmax_tool_rounds = 40\n").agent.max_tool_rounds, 40);
        assert_eq!(load_toml("[agent]\nshow_thinking = false\n").agent.max_tool_rounds, 100);
    }

    #[test]
    fn a_stock_old_prompt_is_replaced_and_an_own_one_kept() {
        let old = load_toml(include_str!("testdata/bia-1.1.1.toml"));
        assert_eq!(old.agent.system_prompt, DEFAULT_SYSTEM_PROMPT);
        let own = load_toml("[agent]\nsystem_prompt = \"You are my bia.\"\n");
        assert_eq!(own.agent.system_prompt, "You are my bia.");
    }
}
