//! FMM — Frontier Model Mode: chat with tool calling over HTTP against OpenRouter / OpenAI
//! (chat completions API), Anthropic (Messages API) or a ChatGPT plan (Responses API, signed in
//! by bia-flash), streaming the reply. For ChatGPT, bia owns the OAuth session: it renews the
//! one-hour access token before requests and rotates the 30-day refresh token at least daily.
use crate::tools::{Msg, Role, ToolCall, ToolSpec};
use anyhow::{bail, Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FmmConfig {
    pub provider: String,
    #[serde(default)]
    pub base_url: String,
    pub api_key: String,
    #[serde(default)]
    pub model: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Software context budget in tokens. 0 = default (131072).
    #[serde(default)]
    pub ctx: u32,
    /// Provider `chatgpt`: the sign-in from bia-flash. `api_key` mirrors its access token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chatgpt: Option<ChatGptAuth>,
}

/// "Sign in with ChatGPT" token set (same shape bia-flash writes).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ChatGptAuth {
    pub client_id: String,
    /// This machine's stable host ID (bia replaces the flasher's when seeding).
    pub host_id: String,
    pub access_token: String,
    pub refresh_token: String,
    #[serde(default)]
    pub id_token: String,
    /// Unix seconds.
    pub expires_at: u64,
    #[serde(default)]
    pub refreshed_at: u64,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub scopes: Vec<String>,
}

const CHATGPT_TOKEN_URL: &str = "https://auth.openai.com/api/accounts/oauth/token";
const CHATGPT_RESOURCE: &str = "https://api.openai.com/v1";

/// One refresh at a time: the refresh token rotates, so two racing refreshes would burn it.
static CHATGPT_REFRESH: Mutex<()> = Mutex::new(());

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn new_host_id() -> String {
    let mut b = [0u8; 16];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        use std::io::Read;
        let _ = f.read_exact(&mut b);
    }
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "urn:uuid:{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

fn form_enc(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// Return a valid ChatGPT access token from `store` (fmm.json), refreshing it when it expires
/// within five minutes or when `force` is set. The rotated set is saved before it is used.
pub fn chatgpt_token(store: &Path, force: bool) -> Result<String> {
    let _guard = CHATGPT_REFRESH.lock().unwrap_or_else(|e| e.into_inner());
    let mut cfg = FmmConfig::load(store).context("fmm.json missing")?;
    let Some(auth) = cfg.chatgpt.as_mut() else {
        bail!("fmm.json has no ChatGPT sign-in");
    };
    if !force && auth.expires_at > unix_now() + 300 {
        return Ok(auth.access_token.clone());
    }
    let body = [
        ("grant_type", "refresh_token"),
        ("client_id", auth.client_id.as_str()),
        ("refresh_token", auth.refresh_token.as_str()),
        ("resource", CHATGPT_RESOURCE),
    ]
    .iter()
    .map(|(k, v)| format!("{k}={}", form_enc(v)))
    .collect::<Vec<_>>()
    .join("&");
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .into();
    let resp = agent
        .post(CHATGPT_TOKEN_URL)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .send(body)
        .context("ChatGPT token refresh: auth.openai.com unreachable")?;
    let status = resp.status().as_u16();
    let text = resp.into_body().read_to_string().unwrap_or_default();
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if status >= 400 {
        let code = v["error"].as_str().unwrap_or("");
        // Terminal refresh errors mean this sign-in is over; anything else may be temporary,
        // so the tokens are kept either way (never erased over a network or server failure).
        if matches!(
            code,
            "invalid_grant"
                | "invalid_refresh_token"
                | "token_expired"
                | "refresh_token_expired"
                | "refresh_token_invalidated"
                | "refresh_token_reused"
        ) {
            bail!("the ChatGPT sign-in has ended ({code}); sign in again with system_fmm_chatgpt (Login with ChatGPT), or set an API key");
        }
        bail!("ChatGPT token refresh failed (HTTP {status} {code})");
    }
    let access = v["access_token"]
        .as_str()
        .context("ChatGPT token refresh: no access_token")?;
    auth.access_token = access.to_string();
    if let Some(r) = v["refresh_token"].as_str() {
        auth.refresh_token = r.to_string();
    }
    if let Some(i) = v["id_token"].as_str() {
        auth.id_token = i.to_string();
    }
    let t = unix_now();
    auth.expires_at = t + v["expires_in"].as_u64().unwrap_or(3600);
    auth.refreshed_at = t;
    if let Some(sc) = v["scope"].as_str() {
        auth.scopes = sc.split_whitespace().map(str::to_string).collect();
    }
    let token = auth.access_token.clone();
    cfg.api_key = token.clone();
    cfg.save(store).context("save rotated ChatGPT tokens")?;
    Ok(token)
}

/// Keep a ChatGPT sign-in alive while bia runs: refresh tokens last 30 days from their last
/// use, so rotate once a day even when FMM is idle or the local model is in use.
pub fn chatgpt_keepalive(store: PathBuf) {
    std::thread::spawn(move || loop {
        let due = FmmConfig::load(&store)
            .and_then(|c| c.chatgpt)
            .is_some_and(|a| unix_now().saturating_sub(a.refreshed_at) > 20 * 3600);
        if due {
            if let Err(e) = chatgpt_token(&store, true) {
                eprintln!("bia: ChatGPT keep-alive: {e:#}");
            }
        }
        std::thread::sleep(Duration::from_secs(1800));
    });
}

fn default_true() -> bool {
    true
}

fn mask_secret(k: &str) -> String {
    let n = k.chars().count();
    if n > 8 {
        let head: String = k.chars().take(4).collect();
        let tail: String = k.chars().skip(n - 4).collect();
        format!("{head}…{tail}")
    } else {
        "****".into()
    }
}

pub const DEFAULT_FMM_CTX: u32 = 131072;

pub fn fmm_ctx(n: u32) -> u32 {
    if n == 0 {
        DEFAULT_FMM_CTX
    } else {
        n.max(512).min(2_000_000)
    }
}

/// Flasher / host-editable provisioning file on the FAT32 ESP.
pub const ESP_FMM_REL: &str = "EFI/bia/fmm.json";

/// QEMU fw_cfg item (`-fw_cfg name=opt/bia/fmm.json,file=...` in tools/run-qemu.sh).
/// User items must start with `opt/`. The flash image is not modified.
pub const QEMU_FMM_ITEM: &str = "opt/bia/fmm.json";

/// Guest sysfs file for `QEMU_FMM_ITEM` once the `qemu_fw_cfg` module is loaded.
pub const QEMU_FMM_FW_CFG: &str = "/sys/firmware/qemu_fw_cfg/by_name/opt/bia/fmm.json/raw";

impl FmmConfig {
    pub fn load(path: &Path) -> Option<FmmConfig> {
        let data = std::fs::read(path).ok()?;
        serde_json::from_slice(&data).ok()
    }
    pub fn masked_key(&self) -> String {
        mask_secret(&self.api_key)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        // A caller may hold a copy loaded before the last refresh. Writing its older ChatGPT
        // tokens back would reuse a rotated refresh token and end the sign-in, so keep the
        // newer set already on disk.
        let merged = self.chatgpt.as_ref().and_then(|mine| {
            let disk = Self::load(path)?.chatgpt?;
            (disk.client_id == mine.client_id && disk.refreshed_at > mine.refreshed_at).then(|| {
                let mut c = self.clone();
                c.api_key = disk.access_token.clone();
                c.chatgpt = Some(disk);
                c
            })
        });
        let this = merged.as_ref().unwrap_or(self);
        let tmp = path.with_extension("json.tmp");
        {
            use std::io::Write;
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(&serde_json::to_vec_pretty(this)?)?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, path).or_else(|_| std::fs::copy(&tmp, path).map(|_| ()))?;
        let _ = std::fs::remove_file(&tmp);
        unsafe { libc::sync() };
        Ok(())
    }

    /// If `state_path` is missing and the ESP has `EFI/bia/fmm.json`, copy it into state and
    /// delete the ESP file so a later local `/fmm` change is not overwritten on reboot.
    pub fn seed_from_esp(boot: &Path, state_path: &Path) -> Option<String> {
        Self::seed_from(
            &boot.join(ESP_FMM_REL),
            state_path,
            "ESP",
            ESP_FMM_REL,
            true,
        )
    }

    /// If `state_path` is missing and QEMU passed `opt/bia/fmm.json`, copy it into state.
    /// The fw_cfg node is read-only and comes back on the next launch, so it is not removed;
    /// an existing state file (including a rotated ChatGPT token) is left as it is.
    pub fn seed_from_qemu(state_path: &Path) -> Option<String> {
        ensure_qemu_fw_cfg_sysfs();
        Self::seed_from(
            Path::new(QEMU_FMM_FW_CFG),
            state_path,
            "QEMU fw_cfg",
            QEMU_FMM_ITEM,
            false,
        )
    }

    fn seed_from(
        src: &Path,
        state_path: &Path,
        origin: &str,
        name: &str,
        remove_src: bool,
    ) -> Option<String> {
        if state_path.is_file() || !src.is_file() {
            return None;
        }
        let mut cfg = match Self::load(src) {
            Some(c) if !c.api_key.trim().is_empty() && !c.provider.trim().is_empty() => c,
            Some(_) => {
                return Some(format!(
                    "{origin} {name}: incomplete FMM config (need provider + api_key); ignored"
                ));
            }
            None => return Some(format!("{origin} {name}: invalid JSON; ignored")),
        };
        // This machine authorizes with its own host ID from now on, not the flasher's.
        if let Some(a) = cfg.chatgpt.as_mut() {
            a.host_id = new_host_id();
        }
        if let Err(e) = cfg.save(state_path) {
            return Some(format!(
                "{origin} {name}: could not seed {}: {e:#}",
                state_path.display()
            ));
        }
        if remove_src {
            let _ = std::fs::remove_file(src);
            unsafe { libc::sync() };
        }
        Some(format!(
            "FMM seeded from {origin} {name}: {} model {} key {}",
            cfg.provider,
            if cfg.model.is_empty() {
                "(default)"
            } else {
                &cfg.model
            },
            cfg.masked_key()
        ))
    }
}

/// Load Debian's `qemu_fw_cfg` module when this machine is QEMU and sysfs is not up yet.
fn ensure_qemu_fw_cfg_sysfs() {
    if Path::new(QEMU_FMM_FW_CFG).is_file() || !qemu_fw_cfg_device() {
        return;
    }
    let _ = std::process::Command::new("modprobe")
        .arg("qemu_fw_cfg")
        .status();
}

fn qemu_fw_cfg_device() -> bool {
    dt_has(Path::new("/proc/device-tree"), b"qemu,fw-cfg-mmio") || acpi_has_qemu_fw_cfg()
}

fn acpi_has_qemu_fw_cfg() -> bool {
    let Ok(rd) = std::fs::read_dir("/sys/bus/acpi/devices") else {
        return false;
    };
    rd.flatten()
        .any(|e| e.file_name().to_string_lossy().starts_with("QEMU0002"))
}

fn dt_has(root: &Path, needle: &[u8]) -> bool {
    if needle.is_empty() {
        return false;
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for ent in rd.flatten() {
            let p = ent.path();
            if ent.file_name() == "compatible" {
                if let Ok(b) = std::fs::read(&p) {
                    if b.windows(needle.len()).any(|w| w == needle) {
                        return true;
                    }
                }
            } else if p.is_dir() {
                stack.push(p);
            }
        }
    }
    false
}

#[cfg(test)]
mod seed_tests {
    use super::*;
    use std::fs;

    #[test]
    fn seeds_from_esp_when_state_missing() {
        let tmp = std::env::temp_dir().join(format!("bia-fmm-seed-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let boot = tmp.join("boot");
        let state = tmp.join("files/state/fmm.json");
        fs::create_dir_all(boot.join("EFI/bia")).unwrap();
        fs::create_dir_all(state.parent().unwrap()).unwrap();
        let cfg = FmmConfig {
            provider: "openrouter".into(),
            base_url: String::new(),
            api_key: "sk-test-abcdefgh".into(),
            model: "x-ai/grok-4.6".into(),
            enabled: true,
            ctx: 131072,
            chatgpt: None,
        };
        fs::write(
            boot.join(ESP_FMM_REL),
            serde_json::to_vec_pretty(&cfg).unwrap(),
        )
        .unwrap();
        let msg = FmmConfig::seed_from_esp(&boot, &state).expect("seed msg");
        assert!(msg.contains("seeded"), "{msg}");
        assert!(state.is_file());
        assert!(!boot.join(ESP_FMM_REL).is_file());
        let loaded = FmmConfig::load(&state).unwrap();
        assert_eq!(loaded.api_key, "sk-test-abcdefgh");
        assert!(FmmConfig::seed_from_esp(&boot, &state).is_none());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn seeds_from_qemu_file_and_leaves_the_source() {
        let tmp = std::env::temp_dir().join(format!("bia-fmm-qemu-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let src = tmp.join("fmm.json");
        let state = tmp.join("files/state/fmm.json");
        fs::create_dir_all(state.parent().unwrap()).unwrap();
        let cfg = FmmConfig {
            provider: "openrouter".into(),
            base_url: String::new(),
            api_key: "sk-test-abcdefgh".into(),
            model: "x-ai/grok-4.6".into(),
            enabled: true,
            ctx: 131072,
            chatgpt: None,
        };
        fs::write(&src, serde_json::to_vec_pretty(&cfg).unwrap()).unwrap();
        let msg = FmmConfig::seed_from(&src, &state, "QEMU fw_cfg", QEMU_FMM_ITEM, false)
            .expect("seed msg");
        assert!(msg.contains("seeded"), "{msg}");
        assert!(msg.contains(QEMU_FMM_ITEM), "{msg}");
        assert!(src.is_file(), "fw_cfg source stays");
        assert_eq!(FmmConfig::load(&state).unwrap().api_key, "sk-test-abcdefgh");
        fs::write(
            &src,
            b"{\"provider\":\"openrouter\",\"api_key\":\"sk-other-zzzzzzzz\"}",
        )
        .unwrap();
        assert!(FmmConfig::seed_from(&src, &state, "QEMU fw_cfg", QEMU_FMM_ITEM, false).is_none());
        assert_eq!(FmmConfig::load(&state).unwrap().api_key, "sk-test-abcdefgh");
        assert!(QEMU_FMM_FW_CFG.contains(QEMU_FMM_ITEM));
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn dt_compatible_finds_fw_cfg() {
        let tmp = std::env::temp_dir().join(format!("bia-fmm-dt-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let node = tmp.join("fw-cfg@9020000");
        fs::create_dir_all(&node).unwrap();
        fs::write(node.join("compatible"), b"qemu,fw-cfg-mmio\0").unwrap();
        assert!(dt_has(&tmp, b"qemu,fw-cfg-mmio"));
        assert!(!dt_has(&tmp, b"qemu,fw-cfg-acpi"));
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn responses_stream_text_and_namespaced_call() {
        let sse = r#"event: response.created
data: {"type":"response.created","response":{"id":"r1"}}

data: {"type":"response.output_text.delta","delta":"Hel"}
data: {"type":"response.output_text.delta","delta":"lo"}
data: {"type":"response.output_item.done","item":{"type":"function_call","call_id":"c9","namespace":"bia","name":"say","arguments":"{\"message\":\"hi\"}"}}
data: {"type":"response.completed","response":{"usage":{"input_tokens":12,"output_tokens":5}}}
"#;
        let mut seen = String::new();
        let r = Remote::read_responses_stream(sse.as_bytes(), &mut |_, p| {
            seen.push_str(p);
            true
        })
        .unwrap();
        assert_eq!(r.text, "Hello");
        assert_eq!(seen, "Hello");
        assert_eq!(r.calls.len(), 1);
        assert_eq!(r.calls[0].name, "say");
        assert_eq!(r.calls[0].id, "c9");
        assert_eq!(r.calls[0].args["message"], "hi");
        assert_eq!((r.input_tokens, r.output_tokens), (12, 5));

        let failed = "data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"subscription_sharing_usage_limit_exceeded\",\"message\":\"x\"}}}\n";
        let Err(e) = Remote::read_responses_stream(failed.as_bytes(), &mut |_, _| true) else {
            panic!("response.failed must be an error");
        };
        assert!(format!("{e}").contains("usage limit"), "{e}");
        let cut = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"x\"}\n";
        assert!(Remote::read_responses_stream(cut.as_bytes(), &mut |_, _| true).is_err());
    }

    #[test]
    fn responses_stream_reasoning_summaries() {
        let sse = r#"data: {"type":"response.reasoning_summary_part.added","summary_index":0}
data: {"type":"response.reasoning_summary_text.delta","delta":"First"}
data: {"type":"response.reasoning_summary_part.added","summary_index":1}
data: {"type":"response.reasoning_summary_text.delta","delta":"Second"}
data: {"type":"response.output_text.delta","delta":"OK"}
data: {"type":"response.completed","response":{"usage":{}}}
"#;
        let mut thought = String::new();
        let r = Remote::read_responses_stream(sse.as_bytes(), &mut |t, p| {
            if t {
                thought.push_str(p);
            }
            true
        })
        .unwrap();
        assert_eq!(thought, "First\n\nSecond");
        assert_eq!(r.text, "OK");
    }

    #[test]
    fn anthropic_thinking_by_model() {
        assert_eq!(
            anthropic_thinking("claude-opus-4-6").unwrap()["type"],
            "adaptive"
        );
        assert!(anthropic_thinking("claude-opus-4-6")
            .unwrap()
            .get("display")
            .is_none());
        for m in [
            "claude-opus-5-5",
            "claude-sonnet-5-5",
            "claude-fable-5-1",
            "claude-opus-4-8",
        ] {
            assert_eq!(
                anthropic_thinking(m).unwrap()["display"],
                "summarized",
                "{m}"
            );
        }
        assert!(anthropic_thinking("claude-haiku-4-5").is_none());
    }

    #[test]
    fn responses_input_shapes() {
        let mut a = Msg::user("x".into());
        a.role = Role::Assistant;
        a.content = String::new();
        a.tool_calls = vec![ToolCall {
            id: "c1".into(),
            name: "bash".into(),
            args: json!({"cmd": "ls"}),
        }];
        let mut t = Msg::user("out".into());
        t.role = Role::Tool;
        t.call_id = "c1".into();
        let items = Remote::responses_input(&[Msg::user("hi".into()), a, t], true);
        assert_eq!(items[0]["content"][0]["type"], "input_text");
        assert_eq!(items[1]["type"], "function_call");
        assert_eq!(items[1]["namespace"], "bia");
        assert_eq!(items[1]["arguments"], "{\"cmd\":\"ls\"}");
        assert_eq!(items[2]["type"], "function_call_output");
        assert_eq!(items[2]["call_id"], "c1");
    }

    #[test]
    fn save_keeps_newer_chatgpt_tokens() {
        let tmp = std::env::temp_dir().join(format!("bia-fmm-rot-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        let path = tmp.join("fmm.json");
        let auth = |rt: &str, at: u64| ChatGptAuth {
            client_id: "c1".into(),
            access_token: format!("access-{rt}"),
            refresh_token: rt.into(),
            refreshed_at: at,
            ..Default::default()
        };
        let mut cfg = FmmConfig {
            provider: "chatgpt".into(),
            base_url: String::new(),
            api_key: "access-new".into(),
            model: "m".into(),
            enabled: true,
            ctx: 0,
            chatgpt: Some(auth("new", 200)),
        };
        cfg.save(&path).unwrap();
        // A stale copy (loaded before the refresh) changes ctx and saves.
        cfg.chatgpt = Some(auth("old", 100));
        cfg.api_key = "access-old".into();
        cfg.ctx = 4096;
        cfg.save(&path).unwrap();
        let got = FmmConfig::load(&path).unwrap();
        assert_eq!(got.ctx, 4096);
        assert_eq!(got.chatgpt.unwrap().refresh_token, "new");
        assert_eq!(got.api_key, "access-new");
        let _ = fs::remove_dir_all(&tmp);
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Api {
    OpenAi,
    Anthropic,
    /// OpenAI Responses API with a ChatGPT sign-in.
    Responses,
}

pub struct Remote {
    pub provider: String,
    pub api: Api,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub ctx: u32,
    /// fmm.json, for providers whose credentials rotate (ChatGPT).
    pub store: Option<PathBuf>,
}

pub struct RemoteResult {
    pub text: String,
    pub calls: Vec<ToolCall>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub stop: String,
}

pub fn default_model(provider: &str) -> &'static str {
    match provider {
        "anthropic" | "claude" => "claude-opus-4-6",
        "openrouter" => "x-ai/grok-4.6",
        "xai" => "grok-4.6",
        "deepseek" => "deepseek-chat",
        "qwen" => "qwen-max",
        _ => "gpt-5",
    }
}

/// Thinking config that streams readable reasoning. Opus/Sonnet 4.6 summarize by default;
/// later models think by default but omit the text unless `display` asks for it. Older
/// models (budget_tokens only) are left as they are.
fn anthropic_thinking(model: &str) -> Option<Value> {
    let m = model.strip_prefix("anthropic.").unwrap_or(model);
    if m.starts_with("claude-opus-4-6") || m.starts_with("claude-sonnet-4-6") {
        return Some(json!({"type": "adaptive"}));
    }
    let current = [
        "claude-opus-4-7",
        "claude-opus-4-8",
        "claude-opus-5",
        "claude-sonnet-5",
        "claude-haiku-5",
        "claude-fable",
        "claude-mythos",
    ];
    current
        .iter()
        .any(|p| m.starts_with(p))
        .then(|| json!({"type": "adaptive", "display": "summarized"}))
}

/// Function tools on the ChatGPT-plan route go in a namespace; set once the route has refused
/// that shape and plain top-level function tools worked instead.
static RESPONSES_FLAT_TOOLS: AtomicBool = AtomicBool::new(false);
/// Set once the Responses route has refused `reasoning` (a non-reasoning model, or an API org
/// not verified for summaries); replies then go without streamed thoughts.
static RESPONSES_NO_REASONING: AtomicBool = AtomicBool::new(false);
const RESPONSES_NS: &str = "bia";

impl Remote {
    pub fn from_config(c: &FmmConfig) -> Result<Remote> {
        let provider = c.provider.to_lowercase();
        let (api, base) = match provider.as_str() {
            "openai" => (Api::OpenAi, "https://api.openai.com/v1"),
            "openrouter" => (Api::OpenAi, "https://openrouter.ai/api/v1"),
            "xai" => (Api::OpenAi, "https://api.x.ai/v1"),
            "deepseek" => (Api::OpenAi, "https://api.deepseek.com/v1"),
            // International (Singapore) endpoint; mainland China keys carry their base_url.
            "qwen" => (Api::OpenAi, "https://dashscope-intl.aliyuncs.com/compatible-mode/v1"),
            "chatgpt" => (Api::Responses, CHATGPT_RESOURCE),
            "anthropic" | "claude" => (Api::Anthropic, "https://api.anthropic.com"),
            other => bail!("unknown provider {other}; use openrouter, openai, anthropic, xai, deepseek or qwen"),
        };
        if c.api_key.trim().is_empty() {
            bail!("api_key is empty");
        }
        Ok(Remote {
            api,
            base_url: if c.base_url.trim().is_empty() {
                base.to_string()
            } else {
                c.base_url.trim_end_matches('/').to_string()
            },
            api_key: c.api_key.trim().to_string(),
            model: if c.model.trim().is_empty() {
                default_model(&provider).to_string()
            } else {
                c.model.trim().to_string()
            },
            provider,
            ctx: fmm_ctx(c.ctx),
            store: None,
        })
    }

    /// Where rotating credentials are read from and saved to (the fmm.json this came from).
    pub fn with_store(mut self, path: &Path) -> Remote {
        self.store = Some(path.to_path_buf());
        self
    }

    pub fn label(&self) -> String {
        format!("{}/{}", self.provider, self.model)
    }

    fn agent(&self, timeout: Duration) -> ureq::Agent {
        let cfg = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(timeout))
            .tls_config(
                ureq::tls::TlsConfig::builder()
                    .root_certs(ureq::tls::RootCerts::WebPki)
                    .build(),
            )
            .build();
        cfg.into()
    }

    /// A minimal request to verify the key and reachability.
    pub fn test(&self) -> Result<String> {
        let r = self.generate(
            "You are a connectivity test.",
            &[],
            &[Msg::user("Reply with the single word OK.".into())],
            &mut |_, _| true,
            false,
            Duration::from_secs(20),
        )?;
        Ok(format!(
            "{} answered: {}",
            self.label(),
            r.text.trim().chars().take(40).collect::<String>()
        ))
    }

    /// Stream a completion. `on_piece(thought, text)` — thought=true is reasoning; return false to stop.
    pub fn generate(
        &self,
        system: &str,
        tools: &[ToolSpec],
        history: &[Msg],
        on_piece: &mut dyn FnMut(bool, &str) -> bool,
        _thinking: bool,
        timeout: Duration,
    ) -> Result<RemoteResult> {
        match self.api {
            Api::OpenAi => self.generate_openai(system, tools, history, on_piece, timeout),
            Api::Anthropic => self.generate_anthropic(system, tools, history, on_piece, timeout),
            Api::Responses => self.generate_responses(system, tools, history, on_piece, timeout),
        }
    }

    // ---------------- OpenAI-compatible chat completions ----------------

    fn openai_messages(system: &str, history: &[Msg]) -> Vec<Value> {
        let shot = latest_shot(history);
        let mut msgs = vec![json!({"role": "system", "content": system})];
        for (i, m) in history.iter().enumerate() {
            match m.role {
                Role::User => msgs.push(json!({"role": "user", "content": m.content})),
                Role::Assistant => {
                    let mut o = json!({"role": "assistant", "content": if m.content.is_empty() { Value::Null } else { Value::String(m.content.clone()) }});
                    if !m.tool_calls.is_empty() {
                        o["tool_calls"] = Value::Array(
                            m.tool_calls
                                .iter()
                                .map(|c| json!({"id": c.id, "type": "function", "function": {"name": c.name, "arguments": c.args.to_string()}}))
                                .collect(),
                        );
                    }
                    msgs.push(o);
                }
                Role::Tool => msgs.push(openai_tool(m, shot == Some(i))),
            }
        }
        msgs
    }

    fn generate_openai(
        &self,
        system: &str,
        tools: &[ToolSpec],
        history: &[Msg],
        on_piece: &mut dyn FnMut(bool, &str) -> bool,
        timeout: Duration,
    ) -> Result<RemoteResult> {
        let mut body = json!({
            "model": self.model,
            "messages": Self::openai_messages(system, history),
            "stream": true,
            "stream_options": {"include_usage": true},
        });
        if self.provider == "openrouter" {
            body["reasoning"] = json!({"enabled": true});
        }
        if !tools.is_empty() {
            body["tools"] = Value::Array(
                tools
                    .iter()
                    .map(|t| json!({"type": "function", "function": {"name": t.name, "description": t.description, "parameters": t.parameters}}))
                    .collect(),
            );
        }
        let agent = self.agent(timeout);
        let mut req = agent
            .post(&format!("{}/chat/completions", self.base_url))
            .header("Authorization", &format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json");
        if self.provider == "openrouter" {
            req = req
                .header("HTTP-Referer", "https://bia.local")
                .header("X-Title", "bia");
        }
        let resp = req.send_json(&body).context("request")?;
        let status = resp.status().as_u16();
        if status >= 400 {
            let text = resp.into_body().read_to_string().unwrap_or_default();
            bail!(
                "HTTP {status}: {}",
                text.chars().take(400).collect::<String>()
            );
        }
        let reader = BufReader::new(resp.into_body().into_reader());
        let mut text = String::new();
        let mut calls: Vec<(String, String, String)> = vec![]; // (id, name, arguments json)
        let (mut input_tokens, mut output_tokens) = (0u64, 0u64);
        let mut stop = String::from("end");
        for line in reader.lines() {
            let line = line.context("stream read")?;
            let Some(data) = line.strip_prefix("data:") else {
                continue;
            };
            let data = data.trim();
            if data == "[DONE]" {
                break;
            }
            let Ok(v) = serde_json::from_str::<Value>(data) else {
                continue;
            };
            if let Some(err) = v.get("error") {
                bail!(
                    "provider error: {}",
                    err.get("message")
                        .and_then(|m| m.as_str())
                        .unwrap_or(&err.to_string())
                );
            }
            if let Some(u) = v.get("usage") {
                if let Some(n) = json_u64(u, &["prompt_tokens", "input_tokens"]) {
                    input_tokens = n;
                }
                if let Some(n) = json_u64(u, &["completion_tokens", "output_tokens"]) {
                    output_tokens = n;
                }
            }
            let Some(choice) = v.get("choices").and_then(|c| c.get(0)) else {
                continue;
            };
            if let Some(r) = choice.get("finish_reason").and_then(|r| r.as_str()) {
                stop = r.to_string();
            }
            let Some(delta) = choice.get("delta") else {
                continue;
            };
            // Servers may carry the same text under two of these keys (vLLM sends both
            // `reasoning` and `reasoning_content`); only the first one present is shown.
            let thought = ["reasoning_content", "reasoning", "thinking"]
                .iter()
                .filter_map(|key| delta.get(*key))
                .find_map(|c| {
                    c.as_str()
                        .or_else(|| {
                            c.get("text")
                                .or_else(|| c.get("content"))
                                .and_then(|x| x.as_str())
                        })
                        .filter(|t| !t.is_empty())
                });
            if let Some(t) = thought {
                if !on_piece(true, t) {
                    stop = "interrupted".into();
                    break;
                }
            }
            if let Some(c) = delta.get("content").and_then(|c| c.as_str()) {
                if !c.is_empty() {
                    text.push_str(c);
                    if !on_piece(false, c) {
                        stop = "interrupted".into();
                        break;
                    }
                }
            }
            if let Some(tcs) = delta.get("tool_calls").and_then(|t| t.as_array()) {
                for tc in tcs {
                    let idx = tc.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                    while calls.len() <= idx {
                        calls.push((String::new(), String::new(), String::new()));
                    }
                    if let Some(id) = tc.get("id").and_then(|i| i.as_str()) {
                        calls[idx].0 = id.to_string();
                    }
                    if let Some(f) = tc.get("function") {
                        if let Some(n) = f.get("name").and_then(|n| n.as_str()) {
                            calls[idx].1.push_str(n);
                        }
                        if let Some(a) = f.get("arguments").and_then(|a| a.as_str()) {
                            calls[idx].2.push_str(a);
                        }
                    }
                }
            }
        }
        let calls = calls
            .into_iter()
            .enumerate()
            .filter(|(_, c)| !c.1.is_empty())
            .map(|(i, (id, name, args))| ToolCall {
                id: if id.is_empty() {
                    format!("call_{i}")
                } else {
                    id
                },
                name,
                args: serde_json::from_str(&args).unwrap_or_else(|_| json!({})),
            })
            .collect();
        Ok(RemoteResult {
            text,
            calls,
            input_tokens,
            output_tokens,
            stop,
        })
    }

    // ---------------- OpenAI Responses API (ChatGPT plan) ----------------

    /// History as Responses input items. The system prompt goes in `instructions` (system
    /// items are refused on this route); a screenshot follows its tool output as a user image.
    fn responses_input(history: &[Msg], namespaced: bool) -> Vec<Value> {
        let shot = latest_shot(history);
        let mut items = vec![];
        for (i, m) in history.iter().enumerate() {
            match m.role {
                Role::User => items.push(
                    json!({"role": "user", "content": [{"type": "input_text", "text": m.content}]}),
                ),
                Role::Assistant => {
                    if !m.content.is_empty() {
                        items.push(json!({"role": "assistant", "content": [{"type": "output_text", "text": m.content}]}));
                    }
                    for c in &m.tool_calls {
                        let mut call = json!({"type": "function_call", "call_id": c.id, "name": c.name, "arguments": c.args.to_string()});
                        if namespaced {
                            call["namespace"] = json!(RESPONSES_NS);
                        }
                        items.push(call);
                    }
                }
                Role::Tool => {
                    items.push(json!({"type": "function_call_output", "call_id": m.call_id, "output": m.content}));
                    if shot == Some(i) {
                        if let Some((mime, b64)) = image_path(&m.content).and_then(load_image) {
                            items.push(json!({"role": "user", "content": [
                                {"type": "input_text", "text": "Screenshot from the tool result above."},
                                {"type": "input_image", "image_url": format!("data:{mime};base64,{b64}")}
                            ]}));
                        }
                    }
                }
            }
        }
        items
    }

    fn generate_responses(
        &self,
        system: &str,
        tools: &[ToolSpec],
        history: &[Msg],
        on_piece: &mut dyn FnMut(bool, &str) -> bool,
        timeout: Duration,
    ) -> Result<RemoteResult> {
        let mut forced = false;
        loop {
            let namespaced = !tools.is_empty() && !RESPONSES_FLAT_TOOLS.load(Ordering::Relaxed);
            let reasoning = !RESPONSES_NO_REASONING.load(Ordering::Relaxed);
            let token = match &self.store {
                Some(p) => chatgpt_token(p, forced)?,
                None => self.api_key.clone(),
            };
            // No sampling or token-limit fields: the ChatGPT-plan route refuses them.
            let mut body = json!({
                "model": self.model,
                "instructions": system,
                "input": Self::responses_input(history, namespaced),
                "store": false,
                "stream": true,
            });
            // Reasoning summaries are only streamed when asked for; they feed the gray thoughts.
            if reasoning {
                body["reasoning"] = json!({"summary": "auto"});
            }
            if !tools.is_empty() {
                let fns: Vec<Value> = tools
                    .iter()
                    .map(|t| json!({"type": "function", "name": t.name, "description": t.description, "parameters": t.parameters}))
                    .collect();
                body["tools"] = if namespaced {
                    json!([{"type": "namespace", "name": RESPONSES_NS, "description": "bia's tools", "tools": fns}])
                } else {
                    Value::Array(fns)
                };
            }
            let resp = self
                .agent(timeout)
                .post(&format!("{}/responses", self.base_url))
                .header("Authorization", &format!("Bearer {token}"))
                .header("Content-Type", "application/json")
                .send_json(&body)
                .context("request")?;
            let status = resp.status().as_u16();
            if status >= 400 {
                let text = resp.into_body().read_to_string().unwrap_or_default();
                let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                let code = v["error"]["code"].as_str().unwrap_or("").to_string();
                let msg = v["error"]["message"].as_str().unwrap_or("").to_string();
                // A token can be revoked or expire early: renew once and retry.
                if status == 401 && !forced && self.store.is_some() {
                    forced = true;
                    continue;
                }
                if status == 400
                    && namespaced
                    && (msg.contains("namespace") || code.contains("unsupported"))
                {
                    RESPONSES_FLAT_TOOLS.store(true, Ordering::Relaxed);
                    continue;
                }
                if status == 400
                    && reasoning
                    && (msg.contains("reasoning") || msg.contains("summar"))
                {
                    RESPONSES_NO_REASONING.store(true, Ordering::Relaxed);
                    continue;
                }
                bail!("HTTP {status}: {}", chatgpt_plan_error(&code, &msg, &text));
            }
            return Self::read_responses_stream(
                BufReader::new(resp.into_body().into_reader()),
                on_piece,
            );
        }
    }

    fn read_responses_stream(
        reader: impl BufRead,
        on_piece: &mut dyn FnMut(bool, &str) -> bool,
    ) -> Result<RemoteResult> {
        let mut text = String::new();
        let mut calls = vec![];
        let (mut input_tokens, mut output_tokens) = (0u64, 0u64);
        let mut stop = String::from("end");
        let mut completed = false;
        for line in reader.lines() {
            let line = line.context("stream read")?;
            let Some(data) = line.strip_prefix("data:") else {
                continue;
            };
            let Ok(v) = serde_json::from_str::<Value>(data.trim()) else {
                continue;
            };
            match v["type"].as_str().unwrap_or("") {
                "response.output_text.delta" => {
                    let d = v["delta"].as_str().unwrap_or("");
                    if !d.is_empty() {
                        text.push_str(d);
                        if !on_piece(false, d) {
                            stop = "interrupted".into();
                            break;
                        }
                    }
                }
                "response.reasoning_summary_part.added" => {
                    if v["summary_index"].as_u64().unwrap_or(0) > 0 && !on_piece(true, "\n\n") {
                        stop = "interrupted".into();
                        break;
                    }
                }
                "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                    let d = v["delta"].as_str().unwrap_or("");
                    if !d.is_empty() && !on_piece(true, d) {
                        stop = "interrupted".into();
                        break;
                    }
                }
                "response.output_item.done" => {
                    let item = &v["item"];
                    if item["type"] == "function_call" {
                        let args = item["arguments"].as_str().unwrap_or("{}");
                        calls.push(ToolCall {
                            id: item["call_id"].as_str().unwrap_or("").to_string(),
                            name: item["name"].as_str().unwrap_or("").to_string(),
                            args: serde_json::from_str(args).unwrap_or_else(|_| json!({})),
                        });
                    }
                }
                "response.completed" => {
                    let u = &v["response"]["usage"];
                    input_tokens = json_u64(u, &["input_tokens"]).unwrap_or(0);
                    output_tokens = json_u64(u, &["output_tokens"]).unwrap_or(0);
                    completed = true;
                    break;
                }
                "response.incomplete" => {
                    stop = v["response"]["incomplete_details"]["reason"]
                        .as_str()
                        .unwrap_or("incomplete")
                        .to_string();
                    completed = true;
                    break;
                }
                "response.failed" | "error" => {
                    let e = if v["response"]["error"].is_null() {
                        &v
                    } else {
                        &v["response"]["error"]
                    };
                    let code = e["code"].as_str().unwrap_or("");
                    let msg = e["message"].as_str().unwrap_or("request failed");
                    bail!("provider error: {}", chatgpt_plan_error(code, msg, msg));
                }
                _ => {}
            }
        }
        if !completed && stop != "interrupted" {
            bail!("the reply stream ended before response.completed");
        }
        let calls = calls
            .into_iter()
            .enumerate()
            .filter(|(_, c): &(usize, ToolCall)| !c.name.is_empty())
            .map(|(i, mut c)| {
                if c.id.is_empty() {
                    c.id = format!("call_{i}");
                }
                c
            })
            .collect();
        Ok(RemoteResult {
            text,
            calls,
            input_tokens,
            output_tokens,
            stop,
        })
    }

    // ---------------- Anthropic Messages API ----------------

    fn anthropic_messages(history: &[Msg]) -> Vec<Value> {
        // consecutive user/tool entries are merged into one user turn (tool_result blocks first)
        let mut msgs: Vec<Value> = vec![];
        let mut user_blocks: Vec<Value> = vec![];
        let flush = |msgs: &mut Vec<Value>, blocks: &mut Vec<Value>| {
            if !blocks.is_empty() {
                blocks.sort_by_key(|b| if b["type"] == "tool_result" { 0 } else { 1 });
                msgs.push(json!({"role": "user", "content": std::mem::take(blocks)}));
            }
        };
        let shot = latest_shot(history);
        for (i, m) in history.iter().enumerate() {
            match m.role {
                Role::User => user_blocks.push(json!({"type": "text", "text": m.content})),
                Role::Tool => user_blocks.push(anthropic_tool(m, shot == Some(i))),
                Role::Assistant => {
                    flush(&mut msgs, &mut user_blocks);
                    let mut blocks = vec![];
                    if !m.content.trim().is_empty() {
                        blocks.push(json!({"type": "text", "text": m.content}));
                    }
                    for c in &m.tool_calls {
                        blocks.push(json!({"type": "tool_use", "id": c.id, "name": c.name, "input": c.args}));
                    }
                    if blocks.is_empty() {
                        continue;
                    }
                    msgs.push(json!({"role": "assistant", "content": blocks}));
                }
            }
        }
        flush(&mut msgs, &mut user_blocks);
        msgs
    }

    fn generate_anthropic(
        &self,
        system: &str,
        tools: &[ToolSpec],
        history: &[Msg],
        on_piece: &mut dyn FnMut(bool, &str) -> bool,
        timeout: Duration,
    ) -> Result<RemoteResult> {
        let mut msgs = Self::anthropic_messages(history);
        if let Some(last) = msgs.last_mut() {
            if let Some(content) = last.get_mut("content").and_then(|c| c.as_array_mut()) {
                if let Some(block) = content.last_mut() {
                    if block.get("type").and_then(|t| t.as_str()) != Some("text")
                        || !block
                            .get("text")
                            .and_then(|t| t.as_str())
                            .unwrap_or("")
                            .is_empty()
                    {
                        block["cache_control"] = json!({"type": "ephemeral"});
                    }
                }
            }
        }
        let mut body = json!({
            "model": self.model,
            "max_tokens": 16000,
            "stream": true,
            "system": [{"type": "text", "text": system, "cache_control": {"type": "ephemeral"}}],
            "messages": msgs,
        });
        if !tools.is_empty() {
            let mut arr: Vec<Value> = tools.iter().map(|t| json!({"name": t.name, "description": t.description, "input_schema": t.parameters})).collect();
            if let Some(last) = arr.last_mut() {
                last["cache_control"] = json!({"type": "ephemeral"});
            }
            body["tools"] = Value::Array(arr);
        }
        // refusal fallbacks (server-side) for the Opus 5 / Fable family
        let with_fallbacks =
            self.model.starts_with("claude-opus-5") || self.model.starts_with("claude-fable");
        if with_fallbacks {
            body["fallbacks"] = json!("default");
        }
        if let Some(t) = anthropic_thinking(&self.model) {
            body["thinking"] = t;
        }
        let agent = self.agent(timeout);
        let resp = loop {
            let mut req = agent
                .post(&format!("{}/v1/messages", self.base_url))
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", "2023-06-01")
                .header("Content-Type", "application/json");
            if with_fallbacks {
                req = req.header("anthropic-beta", "server-side-fallback-2026-07-01");
            }
            let resp = req.send_json(&body).context("request")?;
            let status = resp.status().as_u16();
            if status >= 400 {
                let text = resp.into_body().read_to_string().unwrap_or_default();
                // A model or proxy that refuses the thinking config: answer without it.
                if status == 400 && text.contains("thinking") && body.get("thinking").is_some() {
                    body.as_object_mut().unwrap().remove("thinking");
                    continue;
                }
                bail!(
                    "HTTP {status}: {}",
                    text.chars().take(400).collect::<String>()
                );
            }
            break resp;
        };
        let reader = BufReader::new(resp.into_body().into_reader());
        let mut text = String::new();
        let mut blocks: Vec<(String, String, String, String)> = vec![]; // (type, id, name, partial json)
        let (mut input_tokens, mut output_tokens) = (0u64, 0u64);
        let mut stop = String::from("end_turn");
        for line in reader.lines() {
            let line = line.context("stream read")?;
            let Some(data) = line.strip_prefix("data:") else {
                continue;
            };
            let Ok(v) = serde_json::from_str::<Value>(data.trim()) else {
                continue;
            };
            match v.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                "error" => bail!(
                    "provider error: {}",
                    v.pointer("/error/message")
                        .and_then(|m| m.as_str())
                        .unwrap_or("unknown")
                ),
                "message_start" => {
                    if let Some(u) = v.pointer("/message/usage") {
                        if let Some(n) = json_u64(u, &["input_tokens", "prompt_tokens"]) {
                            input_tokens = n;
                        }
                    }
                }
                "content_block_start" => {
                    let idx = v.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                    while blocks.len() <= idx {
                        blocks.push(Default::default());
                    }
                    let cb = &v["content_block"];
                    blocks[idx].0 = cb
                        .get("type")
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .to_string();
                    blocks[idx].1 = cb
                        .get("id")
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .to_string();
                    blocks[idx].2 = cb
                        .get("name")
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .to_string();
                }
                "content_block_delta" => {
                    let idx = v.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                    while blocks.len() <= idx {
                        blocks.push(Default::default());
                    }
                    let d = &v["delta"];
                    match d.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                        "text_delta" => {
                            let t = d.get("text").and_then(|t| t.as_str()).unwrap_or("");
                            text.push_str(t);
                            if !on_piece(false, t) {
                                stop = "interrupted".into();
                                break;
                            }
                        }
                        "thinking_delta" => {
                            let t = d.get("thinking").and_then(|t| t.as_str()).unwrap_or("");
                            if !t.is_empty() && !on_piece(true, t) {
                                stop = "interrupted".into();
                                break;
                            }
                        }
                        "input_json_delta" => blocks[idx]
                            .3
                            .push_str(d.get("partial_json").and_then(|t| t.as_str()).unwrap_or("")),
                        _ => {}
                    }
                }
                "message_delta" => {
                    if let Some(s) = v.pointer("/delta/stop_reason").and_then(|s| s.as_str()) {
                        stop = s.to_string();
                    }
                    if let Some(u) = v.get("usage") {
                        if let Some(n) = json_u64(u, &["output_tokens", "completion_tokens"]) {
                            output_tokens = n;
                        }
                        if let Some(n) = json_u64(u, &["input_tokens", "prompt_tokens"]) {
                            input_tokens = n;
                        }
                    }
                }
                "message_stop" => break,
                _ => {}
            }
        }
        if stop == "refusal" {
            bail!("the provider declined this request (refusal)");
        }
        let calls = blocks
            .into_iter()
            .filter(|b| b.0 == "tool_use")
            .map(|(_, id, name, args)| ToolCall {
                id,
                name,
                args: if args.trim().is_empty() {
                    json!({})
                } else {
                    serde_json::from_str(&args).unwrap_or_else(|_| json!({}))
                },
            })
            .collect();
        Ok(RemoteResult {
            text,
            calls,
            input_tokens,
            output_tokens,
            stop,
        })
    }
}

/// Plain words for ChatGPT-plan error codes (OpenAI SIWC "Errors and recovery").
fn chatgpt_plan_error(code: &str, msg: &str, raw: &str) -> String {
    match code {
        "subscription_sharing_user_not_eligible" => "this ChatGPT plan can't be used by bia".into(),
        "subscription_sharing_usage_limit_exceeded" => {
            "the ChatGPT plan's usage limit is reached for now (ChatGPT settings → Usage)".into()
        }
        "subscription_sharing_usage_unavailable" | "subscription_sharing_user_unavailable" => {
            "ChatGPT couldn't check the plan right now; try again shortly".into()
        }
        "subscription_sharing_invalid_user" => "ChatGPT did not accept this sign-in".into(),
        "" => raw.chars().take(400).collect(),
        c => format!("{c}: {msg}"),
    }
}

/// Newest tool result that names a screenshot. Older shots stay text so the request does not carry every frame.
fn latest_shot(history: &[Msg]) -> Option<usize> {
    history.iter().enumerate().rev().find_map(|(i, m)| {
        if m.role == Role::Tool && image_path(&m.content).is_some() {
            Some(i)
        } else {
            None
        }
    })
}

fn image_path(content: &str) -> Option<&str> {
    content.lines().find_map(|line| {
        let path = line.trim().strip_prefix("image:")?.trim();
        if path.starts_with("/home/bia/files/")
            && !path.contains("..")
            && (path.ends_with(".png") || path.ends_with(".jpg") || path.ends_with(".jpeg"))
        {
            Some(path)
        } else {
            None
        }
    })
}

fn load_image(path: &str) -> Option<(&'static str, String)> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.is_empty() || bytes.len() > 2_000_000 {
        return None;
    }
    let mime = if path.ends_with(".png") {
        "image/png"
    } else {
        "image/jpeg"
    };
    Some((
        mime,
        base64::engine::general_purpose::STANDARD.encode(bytes),
    ))
}

fn openai_tool(m: &Msg, with_image: bool) -> Value {
    if with_image {
        if let Some(path) = image_path(&m.content) {
            if let Some((mime, b64)) = load_image(path) {
                return json!({
                    "role": "tool",
                    "tool_call_id": m.call_id,
                    "content": [
                        {"type": "text", "text": m.content},
                        {"type": "image_url", "image_url": {"url": format!("data:{mime};base64,{b64}")}}
                    ]
                });
            }
        }
    }
    json!({"role": "tool", "tool_call_id": m.call_id, "content": m.content})
}

fn anthropic_tool(m: &Msg, with_image: bool) -> Value {
    if with_image {
        if let Some(path) = image_path(&m.content) {
            if let Some((mime, b64)) = load_image(path) {
                return json!({
                    "type": "tool_result",
                    "tool_use_id": m.call_id,
                    "content": [
                        {"type": "text", "text": m.content},
                        {"type": "image", "source": {"type": "base64", "media_type": mime, "data": b64}}
                    ]
                });
            }
        }
    }
    json!({"type": "tool_result", "tool_use_id": m.call_id, "content": m.content})
}

fn json_u64(v: &Value, keys: &[&str]) -> Option<u64> {
    for k in keys {
        let Some(x) = v.get(*k) else { continue };
        if let Some(n) = x.as_u64() {
            return Some(n);
        }
        if let Some(n) = x.as_i64() {
            return Some(n.max(0) as u64);
        }
        if let Some(s) = x.as_str() {
            if let Ok(n) = s.parse::<u64>() {
                return Some(n);
            }
        }
        if let Some(n) = x.as_f64() {
            return Some(n.max(0.0) as u64);
        }
    }
    None
}
