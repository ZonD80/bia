//! bia: systemd service that mounts the volumes, loads the model, runs the console agent.
// the generated OpenAPI document is one large json! literal
#![recursion_limit = "512"]
mod addressbook;
mod bash;
mod buses;
mod clock;
mod config;
mod console;
mod gguf;
mod hw;
mod jobs;
mod llama_srv;
mod llm;
mod mcp;
mod memory;
mod outbox;
mod release;
mod remote;
mod repl;
mod schedules;
mod state;
mod sys;
mod tools;
mod tune;
mod tz;
mod unanswered;
mod weblink;

#[path = "../../consock.rs"]
mod consock;
#[path = "../../listen.rs"]
mod listen;

use anyhow::{Context, Result};
use console::{Boot, Console};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tools::human;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    // one rustls crypto provider for the whole process (HTTPS to cloud models)
    bia_pki::install_crypto_provider();
    let system = sys::is_system();
    let con = Console::open(system);
    let result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(con.clone(), system)));
    let exit = match result {
        Ok(Ok(e)) => e,
        Ok(Err(e)) => {
            con.writeln(&format!("\n!! fatal: {e:#}"));
            recovery(&con, system)
        }
        Err(p) => {
            let msg = p
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_default();
            con.writeln(&format!("\n!! panic: {msg}"));
            recovery(&con, system)
        }
    };
    if matches!(exit, repl::Exit::Reboot | repl::Exit::Poweroff) && system {
        con.writeln(match exit {
            repl::Exit::Poweroff => "powering off",
            _ => "rebooting",
        });
        state::mark_exit(
            std::path::Path::new("/home/bia/files/state/last_exit"),
            matches!(exit, repl::Exit::Poweroff),
        );
        std::thread::sleep(Duration::from_millis(300));
        sys::reboot(matches!(exit, repl::Exit::Poweroff));
    }
}

/// After a fatal error the service waits so /reboot still works;
/// /exit lets systemd respawn; /forget clears history then respawns clean.
fn recovery(con: &Arc<Console>, system: bool) -> repl::Exit {
    if !system {
        return repl::Exit::Quit;
    }
    con.writeln("agent stopped. /reboot, /poweroff, /forget, or /exit");
    loop {
        con.write("\nbia(recovery)> ");
        match con.read_line().as_deref().map(str::trim) {
            Some("/reboot") => return repl::Exit::Reboot,
            Some("/poweroff") => return repl::Exit::Poweroff,
            Some("/forget") => {
                let dir = std::path::Path::new("/home/bia/files/state");
                state::forget(&dir.join("conversation.json"));
                let _ = std::fs::remove_file(dir.join("conversation.json.bak"));
                memory::Store::open(dir).forget();
                con.writeln("forgotten; restarting");
                return repl::Exit::Quit;
            }
            Some("/exit") | None => return repl::Exit::Quit,
            _ => {}
        }
    }
}

fn run(con: Arc<Console>, system_mode: bool) -> Result<repl::Exit> {
    let boot = Arc::new(Boot::new(con.clone()));
    let arch = std::env::consts::ARCH;
    let kernel = sys::kernel_version();
    con.update_status(|s| {
        s.version = VERSION.into();
        s.arch = arch.into();
        s.kernel = kernel.clone();
        s.model = "loading".into();
        s.state = "boot".into();
    });
    con.writeln("");
    con.writeln(&con.c(
        console::BOLD,
        &format!(
            "bia {VERSION} ({arch})  kernel {kernel}  {} cpus  llama.cpp via prebuilt llama-server",
            sys::ncpu()
        ),
    ));
    con.writeln("");

    // ---- volumes ----
    let paths = if system_mode {
        let boot_dev = sys::boot_dev();
        sys::ensure_volume_dirs()?;
        match sys::ensure_volume_writable() {
            Ok(()) => boot.step(
                10,
                &format!(
                    "home: /home/bia boot={} (writable, state persists)",
                    boot_dev
                        .map(|d| format!("/dev/{d}"))
                        .unwrap_or_else(|| "none".into())
                ),
            ),
            Err(e) => {
                boot.step(
                    10,
                    &format!(
                        "home: /home/bia boot={}",
                        boot_dev
                            .map(|d| format!("/dev/{d}"))
                            .unwrap_or_else(|| "none".into())
                    ),
                );
                boot.warn(&format!("/home/bia is not writable ({e:#}); conversation and settings will not survive reboot"));
            }
        }
        let swaps = sys::activate_swap();
        let mut tuned = sys::tune_vm();
        if let Some(g) = tune::performance_governor() {
            tuned.push(g);
        }
        let m = sys::meminfo();
        boot.step(
            15,
            &format!(
                "memory: {} RAM, {} available; swap {}; vm: {}",
                human(m.total),
                human(m.available),
                if swaps.is_empty() {
                    "none".to_string()
                } else {
                    swaps.join(",")
                },
                tuned.join(" ")
            ),
        );
        sys::Paths::system()
    } else {
        let vol = std::env::args()
            .nth(2)
            .filter(|_| std::env::args().nth(1).as_deref() == Some("--volume"))
            .map(PathBuf::from)
            .unwrap_or_else(|| "flash".into());
        boot.step(10, &format!("dev mode, volume = {}", vol.display()));
        sys::Paths::dev(&vol)
    };

    // ---- config + optional local model ----
    let (cfg, warn) = config::Config::load(&paths.volume.join("bia.toml"));
    if let Some(w) = warn {
        boot.warn(&w);
    }
    sys::ensure_inbox(&paths.files);
    clock::restore(&paths.files.join("state").join("clock.json"));
    // messages a previous run received but never handed to the model (reboot, update, crash)
    let replay = con.open_journal(paths.files.join("state").join("inbox.jsonl"));
    {
        let c = clock::snapshot();
        boot.note(&format!(
            "clock: {}  tz {}  ntp {}",
            clock::hms_now(),
            c.timezone,
            if c.ntp { "on" } else { "off" }
        ));
    }
    // The web server is the web MCP (a bus like telegram); the agent only shows its address.
    weblink::clear(&paths.files);
    weblink::watch(con.clone(), paths.files.clone());
    con.set_status_file(paths.files.join("state").join("status.json"));
    // root's web console mirrors this console through the web MCP
    if let Err(e) = con.serve_mirror(consock::path(&paths.files)) {
        boot.warn(&format!("console mirror: {e}"));
    }
    let fmm_path = paths.files.join("state").join("fmm.json");
    // QEMU tests pass the repo fmm.json via fw_cfg. A real stick still seeds from the ESP.
    // Either one runs only while files/state/fmm.json is missing.
    if let Some(msg) = remote::FmmConfig::seed_from_qemu(&fmm_path) {
        boot.note(&msg);
    }
    if let Some(msg) = remote::FmmConfig::seed_from_esp(&paths.boot, &fmm_path) {
        boot.note(&msg);
    }
    if let Some(msg) = seed_language(&paths.boot) {
        boot.note(&msg);
    }
    if system_mode {
        // fcitx5 started before us; a new image or a change while it was down catches up.
        bia_speech::input::sync(None, &bia_speech::registry::current());
        if let Some(msg) = bia_speech::learn::resume() {
            boot.note(&msg);
        }
    }
    remote::chatgpt_keepalive(fmm_path.clone());
    let skip_local = remote::FmmConfig::load(&fmm_path)
        .map(|c| c.enabled && !c.api_key.trim().is_empty())
        .unwrap_or(false);
    let model_path = match sys::find_model(&paths.model, &cfg.model.file) {
        Ok(p) => Some(p),
        Err(e) if skip_local => {
            boot.note(&format!("no local GGUF ({e:#})"));
            None
        }
        Err(e) => return Err(e),
    };
    let size = model_path
        .as_ref()
        .and_then(|p| std::fs::metadata(p).ok())
        .map(|m| m.len())
        .unwrap_or(0);
    let local_rt = config::LocalRuntime::load(&config::LocalRuntime::path(&paths.files));
    let mut model_cfg = cfg.model.clone();
    if let Some(ctx) = local_rt.ctx {
        model_cfg.ctx = ctx;
    }
    let plan = model_path.as_ref().map(|p| tune::plan(&model_cfg, p));
    if let Some(plan) = &plan {
        model_cfg.ctx = plan.ctx;
        if system_mode && plan.mlock {
            // weights are pinned; do not swap KV/heap in preference to keeping file cache
            let _ = std::fs::write("/proc/sys/vm/swappiness", "10");
        }
        boot.note(&plan.summary);
    }
    let mut local_spec =
        model_path
            .as_ref()
            .zip(plan.as_ref())
            .map(|(path, plan)| llm::LocalSpec {
                path: path.clone(),
                cfg: model_cfg.clone(),
                sampling: cfg.sampling.clone(),
                plan: plan.clone(),
            });
    let llm = if skip_local {
        boot.step(
            80,
            &match model_path.as_ref().and_then(|p| p.file_name()) {
                Some(n) => format!(
                    "FMM is configured; not loading local model {}",
                    n.to_string_lossy()
                ),
                None => "FMM is configured; no local model on disk".into(),
            },
        );
        con.update_status(|s| {
            s.model = "FMM".into();
            s.model_detail = "local GGUF skipped".into();
            s.ctx_total = remote::FmmConfig::load(&fmm_path)
                .map(|c| remote::fmm_ctx(c.ctx) as usize)
                .unwrap_or(0);
        });
        None
    } else {
        let model_path = model_path.as_ref().context("local model path")?;
        let plan = plan.as_ref().context("local model plan")?;
        boot.step(
            20,
            &format!(
                "model file: {} ({})",
                model_path.file_name().unwrap_or_default().to_string_lossy(),
                human(size)
            ),
        );
        let t0 = std::time::Instant::now();
        let load = |mctx: &config::ModelCfg| {
            let b = boot.clone();
            let mut last = -1i32;
            llm::Llm::load(model_path, mctx, &cfg.sampling, plan, move |f| {
                let pct = (f * 100.0) as i32;
                if pct != last {
                    last = pct;
                    b.bar(20 + (f * 60.0) as u32, "loading model", f);
                }
                true
            })
        };
        let llm = match load(&model_cfg) {
            Ok(l) => l,
            Err(e) if local_rt.ctx.is_some() && local_rt.ctx != Some(cfg.model.ctx) => {
                boot.warn(&format!(
                    "context {} failed ({e:#}); reloading previous ctx {}",
                    model_cfg.ctx, cfg.model.ctx
                ));
                let l = load(&cfg.model).context("model load after context rollback")?;
                let live = l.n_ctx as u32;
                let mut rt = local_rt;
                rt.ctx = Some(live);
                let _ = rt.save(&config::LocalRuntime::path(&paths.files));
                if let Some(s) = local_spec.as_mut() {
                    s.cfg.ctx = live;
                }
                l
            }
            Err(e) => return Err(e).context("model load"),
        };
        if let Some(s) = local_spec.as_mut() {
            s.cfg.ctx = llm.n_ctx as u32;
        }
        boot.end_inline();
        let i = &llm.info;
        for n in &llm.notes {
            boot.note(n);
        }
        boot.step(80, &format!("model: {} [{}] {} params, {} layers{}, trained ctx {}; loaded in {:.1}s on {} ({} gen / {} prefill threads)",
            i.name, i.arch, repl::human_count(i.n_params), i.n_layer,
            i.experts.as_ref().map(|e| format!(", {e}")).unwrap_or_default(), i.n_ctx_train, t0.elapsed().as_secs_f64(),
            llm.device, llm.threads, llm.threads_batch));
        boot.step(
            85,
            &format!(
                "context: {} tokens, {}",
                llm.n_ctx,
                match (llm.device.as_str(), plan.mmap, llm.mlock) {
                    (dev, _, true) if dev != "cpu" =>
                        "weights on GPU, pinned when they spill to RAM",
                    (_, _, true) => "weights locked in RAM",
                    (_, true, false) => "weights mmap'd (page cache = RAM+SSD)",
                    (_, false, false) => "weights copied into RAM",
                }
            ),
        );
        {
            let experts = i
                .experts
                .as_ref()
                .map(|e| {
                    let nums: Vec<&str> = e
                        .split(|c: char| !c.is_ascii_digit())
                        .filter(|x| !x.is_empty())
                        .collect();
                    if nums.len() >= 2 {
                        format!(" {}x{}", nums[0], nums[1])
                    } else {
                        String::new()
                    }
                })
                .unwrap_or_default();
            let (name, detail, n_ctx) = (
                i.name.clone(),
                format!("{} {}{}", repl::human_count(i.n_params), i.arch, experts),
                llm.n_ctx,
            );
            con.update_status(|s| {
                s.model = name;
                s.model_detail = detail;
                s.ctx_total = n_ctx;
            });
        }
        Some(llm)
    };
    let (format, think) = llm
        .as_ref()
        .map(|l| {
            (
                tools::ToolFormat::detect(&l.info.chat_template),
                tools::ToolFormat::opens_think(&l.info.chat_template),
            )
        })
        .unwrap_or((tools::ToolFormat::Hermes, false));
    if let Some(l) = &llm {
        boot.note(&format!(
            "tool-call format: {:?}{}",
            tools::ToolFormat::detect(&l.info.chat_template),
            if tools::ToolFormat::opens_think(&l.info.chat_template) {
                ", thinking"
            } else {
                ""
            }
        ));
    }

    // ---- MCPs ----
    let mut tools = tools::Tools::new(&paths.files, con.clone());
    {
        let n = tools
            .schedules
            .lock()
            .map(|s| s.enabled_count())
            .unwrap_or(0);
        if n > 0 {
            boot.note(&format!("{n} schedule(s) enabled"));
        }
    }
    let env = vec![
        (
            "BIA_VOLUME".to_string(),
            paths.volume.to_string_lossy().to_string(),
        ),
        (
            "BIA_BOOT".to_string(),
            paths.boot.to_string_lossy().to_string(),
        ),
        (
            "BIA_FILES".to_string(),
            paths.files.to_string_lossy().to_string(),
        ),
        // web MCP: HTTPS :443 + HTTP→HTTPS on :80; dev runs use :8443 only
        (
            "BIA_WEB_PORT".to_string(),
            if system_mode { "443" } else { "8443" }.to_string(),
        ),
        (
            "BIA_WEB_HTTP_PORT".to_string(),
            if system_mode { "80" } else { "0" }.to_string(),
        ),
        (
            "BIA_MCPS".to_string(),
            paths.mcps.to_string_lossy().to_string(),
        ),
    ];
    let log_dir = paths.files.join("logs");
    tools.set_mcp_spawn(arch, env.clone(), log_dir.clone());
    let notes = tools.adopt_local_mcps();
    let listed = tools.listed_mcp_names();
    if listed.is_empty() {
        boot.step(90, &format!("MCPs: none in {}", paths.mcps.display()));
    } else {
        boot.step(
            90,
            &format!("MCPs on disk, started on request: {}", listed.join(", ")),
        );
    }
    for n in notes {
        boot.warn(&n);
    }

    // ---- resume a saved conversation ----
    let state_path = paths.files.join("state").join("conversation.json");
    let last_exit = state::take_exit(&paths.files.join("state").join("last_exit"));
    let mut history = vec![];
    let mut saved_pending: Vec<crate::console::Inbound> = vec![];
    let mut saved_at: Option<u64> = None;
    let mut resume_mcps: Vec<String> = vec![];
    if state_path.exists() {
        match state::load(&state_path) {
            Ok((saved, h, note)) => {
                if let Some(n) = note {
                    boot.warn(&n);
                }
                resume_mcps = saved.loaded_mcps.clone();
                saved_pending = saved.pending.iter().map(|p| p.to_inbound()).collect();
                saved_at = Some(saved.saved_at);
                if !saved_pending.is_empty() {
                    boot.note(&format!(
                        "{} queued message(s) waiting for delivery",
                        saved_pending.len()
                    ));
                }
                boot.step(
                    97,
                    &format!(
                        "resuming conversation: {} messages, saved {} ago{}",
                        h.len(),
                        clock::ago(saved.saved_at),
                        match llm.as_ref() {
                            Some(l) if saved.model != l.info.name =>
                                format!(" (with a different model: {})", saved.model),
                            None if !saved.model.is_empty() =>
                                format!(" (saved with {})", saved.model),
                            _ => String::new(),
                        }
                    ),
                );
                if let Some(last) = h.iter().rev().find(|m| m.role == tools::Role::User) {
                    boot.note(&format!(
                        "last message: {}",
                        first_chars(
                            last.content.rsplit('|').next().unwrap_or(&last.content),
                            100
                        )
                    ));
                }
                history = h;
            }
            Err(e) => boot.warn(&format!("saved conversation not loaded: {e:#}")),
        }
    }

    // MCPs start in the background as jobs (boot does not wait for a messenger restoring its
    // session over the network); each result arrives as system|job:<id>.
    let start = |tools: &mut tools::Tools, name: &str, why: &str| match tools.start_job(name) {
        Ok(m) => boot.note(&format!("MCP {name} ({why}): {m}; its functions are callable when that job finishes")),
        Err(e) => boot.warn(&format!("MCP {name}: start failed: {e:#}")),
    };
    // the ones loaded last session (their boot_tool runs, e.g. telegram restore)
    for name in &resume_mcps {
        start(&mut tools, name, "loaded last session");
    }
    // messenger sessions on disk: up even when not loaded last session
    if paths.files.join("telegram").join("ferogram.session").exists() {
        start(&mut tools, "telegram", "session on disk");
    }
    if paths.files.join("whatsapp").join("whatsapp.db").exists() {
        start(&mut tools, "whatsapp", "session on disk");
    }
    // The web bus is always up when enabled (people reach bia there before any login).
    if cfg.web.enabled {
        start(&mut tools, "web", "web app");
    }

    let n_tools = tools.specs.len();
    con.update_status(|s| s.tools = n_tools);

    boot.step(
        100,
        &format!("ready: {} tools, {} consoles", n_tools, con.devices.len()),
    );
    let session_log = Some(
        paths
            .files
            .join("sessions")
            .join(format!("session-{}.md", clock::stamp_now())),
    );
    let system = with_home_language(cfg.agent.system_prompt.clone());
    let mut repl = repl::Repl {
        con: con.clone(),
        llm,
        local_spec,
        tools,
        format,
        think,
        system,
        history,
        max_tool_rounds: cfg.agent.max_tool_rounds,
        session_log,
        state_path,
        pending: saved_pending,
        job_ready: Vec::new(),
        show_thinking: cfg.agent.show_thinking,
        remote: None,
        fmm_path,
        memory: crate::memory::Store::open(&paths.files.join("state")),
        fmm_ctx_used: 0,
        fmm_backups: Default::default(),
        fmm_probation: None,
    };
    for m in &mut repl.history {
        if m.id.is_empty() {
            m.id = repl.memory.next_msg_id();
        }
    }
    let rows: Vec<(String, String, String, String)> = repl
        .history
        .iter()
        .map(|m| {
            (
                m.id.clone(),
                match m.role {
                    tools::Role::User => "user".into(),
                    tools::Role::Assistant => "assistant".into(),
                    tools::Role::Tool => "tool".into(),
                },
                m.name.clone(),
                m.content.clone(),
            )
        })
        .collect();
    repl.memory.backfill(&rows);
    // Frontier Model Mode: wait for DHCP/DNS, then enable (avoids "temporary failure
    // in name resolution" that forced a manual /fmm on).
    if repl.fmm_path.exists() {
        let (provider, base) = remote::FmmConfig::load(&repl.fmm_path)
            .map(|c| (c.provider, c.base_url))
            .unwrap_or_else(|| ("openrouter".into(), String::new()));
        let host = sys::fmm_dns_host(&provider, &base);
        boot.note(&format!(
            "FMM: waiting for network before enabling ({host})"
        ));
        let ready = sys::wait_network(Duration::from_secs(90), Some(&host), |msg| {
            boot.note(msg);
        });
        let mut line = repl.reload_fmm(true);
        // Brief retries: route/DNS can be up a moment before the API path works.
        if ready && line.contains("unreachable") {
            for attempt in 2..=6 {
                boot.note(&format!("FMM: still unreachable; retry {attempt}/6 in 5s"));
                std::thread::sleep(Duration::from_secs(5));
                line = repl.reload_fmm(true);
                if !line.contains("unreachable") {
                    break;
                }
            }
        }
        boot.note(&line);
    }
    if cfg.web.enabled {
        boot.note(&weblink::info_line(&paths.files));
    }
    let resumed = !repl.history.is_empty();
    let nmsg = repl.history.len();
    let updated = std::fs::read_to_string("/run/bia-updated")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let _ = std::fs::remove_file("/run/bia-updated");
    let migrated = std::fs::read_to_string(paths.files.join("state").join("boot_notice"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let _ = std::fs::remove_file(paths.files.join("state").join("boot_notice"));
    // The very first start of this machine: nothing to resume, no earlier exit, not an update
    // or a landing. The marker keeps the welcome to once (an upgraded machine gets it set on
    // its first boot with this version, without the welcome).
    let welcomed = paths.files.join("state").join("welcomed");
    let was_welcomed = welcomed.exists();
    let first_launch =
        !was_welcomed && migrated.is_none() && updated.is_none() && last_exit.is_none() && !resumed;
    if !welcomed.exists() {
        let _ = std::fs::write(&welcomed, format!("bia {VERSION}\n"));
    }
    // Release notes for every version since the one that ran before. Machines from before
    // versions were recorded (≤ 1.1.0) count as updated when an older bia clearly ran here.
    let state_dir = paths.files.join("state");
    let prev_version = release::last_version(&state_dir);
    let older_bia_ran = was_welcomed || updated.is_some() || last_exit.is_some() || resumed;
    let release_notes = if prev_version.is_some() || (older_bia_ran && !first_launch) {
        release::notice(prev_version.as_deref(), VERSION)
    } else {
        None
    };
    release::record(&state_dir, VERSION);
    // Who was talking to bia just before the restart (from the resumed conversation): the
    // boot reply goes to the screen only, so they hear back on their own chats.
    let was_update = updated.is_some() || release_notes.is_some();
    let talkers = if first_launch {
        vec![]
    } else {
        let book = &repl.tools.addressbook;
        last_talkers(&repl.history, saved_at, was_update, |id, ch| {
            id == tools::ID_ROOT || book.find_for(id, ch).is_some_and(|c| c.trust == "root")
        })
    };
    let report = (!talkers.is_empty()).then(|| report_back(&talkers, was_update));
    let how = if let Some(msg) = migrated {
        if resumed {
            format!(
                "{msg} (bia {VERSION}, {n_tools} tools); conversation resumed with {nmsg} messages"
            )
        } else {
            format!("{msg} (bia {VERSION}, {n_tools} tools); new conversation")
        }
    } else if let Some(msg) = updated {
        msg
    } else {
        match (&last_exit, resumed) {
        (Some(state::LastExit::Poweroff), true) =>
            format!("system booted after a normal poweroff (bia {VERSION}, {n_tools} tools); conversation resumed with {nmsg} messages"),
        (Some(state::LastExit::Reboot), true) =>
            format!("system booted after a normal reboot (bia {VERSION}, {n_tools} tools); conversation resumed with {nmsg} messages"),
        (None, true) => {
            let when = saved_at.map(|t| format!("; last save {}", clock::ago(t))).unwrap_or_default();
            format!("system booted after an unexpected halt{when} (bia {VERSION}, {n_tools} tools); conversation resumed with {nmsg} messages")
        }
        (Some(state::LastExit::Poweroff), false) =>
            format!("system booted after a normal poweroff (bia {VERSION}, {n_tools} tools); new conversation"),
        (Some(state::LastExit::Reboot), false) =>
            format!("system booted after a normal reboot (bia {VERSION}, {n_tools} tools); new conversation"),
        (None, false) =>
            format!("system booted (bia {VERSION}, {n_tools} tools); new conversation"),
    }
    };
    let release_steps = release_notes
        .as_deref()
        .is_some_and(|n| n.contains("Update steps for you"));
    let how = match &release_notes {
        Some(n) => {
            boot.note(&format!(
                "release notes: {} → {VERSION}",
                prev_version.as_deref().unwrap_or("older")
            ));
            format!("{how}.\n{n}")
        }
        None => how,
    };
    // Setup instructions root wrote into the deployment config (bia-flash takeover):
    // handed to the model once, on the first boot that finds them.
    let setup_path = paths.files.join("state").join("setup.md");
    let setup = std::fs::read_to_string(&setup_path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    if setup.is_some() {
        let _ = std::fs::rename(&setup_path, paths.files.join("state").join("setup.done.md"));
        boot.note("setup instructions from the deployment config");
    }
    let setup = setup.map(|t| {
        format!(
            "\nSetup instructions from root, written into the deployment config when this machine was installed (root's own words; carry them out now with your tools, then tell root on console and web what you did and what failed):\n<<<\n{t}\n>>>"
        )
    });
    let hw = crate::hw::summary();
    boot.note(&hw);
    boot.note(how.lines().next().unwrap_or(""));
    // One system envelope, then a model turn. Identity is system, not root.
    repl.pending.insert(
        0,
        console::Inbound {
            identity: tools::ID_SYSTEM.into(),
            channel: tools::CH_CONSOLE.into(),
            issued: clock::now(),
            id: String::new(),
            text: format!(
                "{how}{} {}{BOOT_BATCH}{}",
                if release_notes.is_some() { "\n" } else { "." },
                if how.contains("Repair these yourself")
                    || how.contains("Propose fixes to the user and wait for approval")
                {
                    let mut s = String::from("Tell the user this sentence, then repair it yourself without waiting: for each package that did not install, search the internet, download the .deb for the version and architecture named here and install it; copy binaries that were not copied into place. Ask the user first only before a destructive step (removing or overwriting existing files, packages or binaries, up- or downgrading installed packages, disk or partition changes, reboot or poweroff). When done, tell the user what you fixed and what still fails.");
                    if release_notes.is_some() {
                        s.push_str(" After that, tell them what changed in this update and do the update steps listed above.");
                    }
                    s
                } else if how.contains("this boot landed") || how.contains("this boot restored") {
                    if release_notes.is_some() {
                        format!("Say that sentence to the user first. {}", release::instruction(release_steps))
                    } else {
                        "Say that sentence to the user, then say you are ready for commands. Do not call tools for this notice.".into()
                    }
                } else if release_notes.is_some() {
                    release::instruction(release_steps).to_string()
                } else if first_launch {
                    welcome(cfg.web.enabled.then(|| weblink::info_line(&paths.files)), repl.remote.is_some())
                } else {
                    READY_PLAIN.into()
                },
                setup.as_deref().unwrap_or("")
            ),
        }
        .ensure_id(),
    );
    {
        let lost = repl
            .tools
            .jobs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .fail_running_lost();
        if !lost.is_empty() {
            boot.note(&format!("{} job(s) lost on reboot", lost.len()));
            repl.pending.extend(lost);
        }
    }
    {
        let lost = std::mem::take(&mut repl.tools.outbox_lost);
        if !lost.is_empty() {
            boot.note(&format!(
                "{} telegram/whatsapp send(s) cut short on reboot",
                lost.len()
            ));
            repl.pending.extend(lost);
        }
    }
    if !replay.is_empty() {
        boot.note(&format!(
            "{} message(s) received before the restart, not yet answered",
            replay.len()
        ));
    }
    let mut fresh = Vec::new();
    while let Some(ib) = con.try_read_inbound() {
        fresh.push(ib);
    }
    // older ones first; a bus that delivers the same message again is not doubled
    for ib in replay {
        if !fresh.iter().any(|f| f.id == ib.id) && !repl.pending.iter().any(|p| p.id == ib.id) {
            repl.pending.push(ib);
        }
    }
    repl.pending.extend(fresh);
    // people wrote while bia was off: the plain boot notice asks for their answers, not for
    // a "ready for commands" that the model takes as the whole turn
    if repl.pending.len() > 1 && repl.pending[0].text.contains(READY_PLAIN) {
        let waiting = repl.pending.len() - 1;
        repl.pending[0].text = repl.pending[0]
            .text
            .replace(READY_PLAIN, &catch_up(waiting));
    }
    if let Some(r) = &report {
        let t = repl.pending[0].text.replace(NO_TOOLS, "Do not call other tools for this notice.");
        repl.pending[0].text = format!("{t}\n{r}");
    }
    if !repl.pending.is_empty() {
        boot.note(&format!(
            "{} inbound message(s) waiting (telegram/whatsapp catch-up / schedules / queued)",
            repl.pending.len()
        ));
    }
    if resumed {
        let b = boot.clone();
        let mut last = 0;
        repl.warm(|done, total| {
            let pct = (done * 100 / total.max(1)) as i32;
            if pct != last {
                last = pct;
                b.bar(100, "restoring context", done as f32 / total.max(1) as f32);
            }
        });
        boot.end_inline();
    }
    Ok(repl.run())
}

/// Where the flasher leaves the owner's language: `{"lang": "ru"}`.
const ESP_LANG_REL: &str = "EFI/bia/lang.json";

/// Adopt the language chosen in the flasher once, then delete the ESP file so a later
/// lang_default is not undone on reboot.
fn seed_language(boot: &std::path::Path) -> Option<String> {
    let esp = boot.join(ESP_LANG_REL);
    let text = std::fs::read_to_string(&esp).ok()?;
    let lang = serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|v| v["lang"].as_str().map(str::to_string));
    let msg = match lang {
        Some(l) => match bia_speech::learn::adopt_home(&l) {
            Ok(m) => format!("ESP {ESP_LANG_REL}: {m}"),
            Err(e) => format!("ESP {ESP_LANG_REL}: {e}"),
        },
        None => format!("ESP {ESP_LANG_REL}: no \"lang\"; ignored"),
    };
    let _ = std::fs::remove_file(&esp);
    unsafe { libc::sync() };
    Some(msg)
}

/// What the model says the very first time this machine starts: a short tour that ends with
/// questions, in the owner's language (the system prompt names it).
fn welcome(web: Option<String>, cloud: bool) -> String {
    let code = bia_speech::registry::current().default_lang.clone();
    let lang = if code.is_empty() || code == "en" {
        "English".to_string()
    } else {
        bia_speech::langs::name(&code)
    };
    let web = match web {
        Some(w) if w.contains("https://") => format!("the web app ({w}): from a phone or computer on the same network, the first visit lets them take the root certificate, then they get chats with you, files, the screen and voice there"),
        _ => "the web app: once you are on a network its address shows at the top of the screen; the first visit lets them take the root certificate, then they get chats with you, files, the screen and voice there".into(),
    };
    let brain = if cloud {
        String::new()
    } else {
        "\n- you think with a small local model now; for a stronger one they can give you an API key (OpenRouter, OpenAI, Anthropic, …) or sign in with their ChatGPT plan.".into()
    };
    format!(
        "This is the very first start of this machine, and the user is meeting you now. Write in {lang}. Greet them warmly and say who you are: bia, an assistant that lives on this computer. Then tell them, briefly, one line each:\n\
- ask them what your name should be; remember it and use it from then on.\n\
- {web}.\n\
- you can log in to their Telegram and WhatsApp, so they can write and call you there and you can message for them; ask whether they want that.\n\
- you can create your own accounts on websites and services (e-mail and others) in your own browser, keep the passwords and one-time codes in your password store, and ask them for help when a captcha or a phone check comes up.\n\
- you can schedule jobs, run commands, learn languages and talk by voice.{brain}\n\
End by asking what your name should be and what they would like to start with. Do not call tools for this greeting."
    )
}

/// The plain boot instruction (nothing else waiting).
const READY_PLAIN: &str =
    "Reply that you are ready for commands. Do not call tools for this notice.";

/// The boot instruction when messages are waiting from while bia was off.
fn catch_up(n: usize) -> String {
    format!("{n} message(s) that came while you were off are delivered with this notice. Do not say you are ready for commands; answer and handle each of them now as usual, with tools when they need them.")
}

const NO_TOOLS: &str = "Do not call tools for this notice.";

/// People who wrote shortly before the restart, newest first, one per channel:
/// (identity, channel, when, their last words). After an update root's last chat counts
/// at any age, since root wants to hear about updates.
fn last_talkers(
    history: &[tools::Msg],
    saved_at: Option<u64>,
    was_update: bool,
    is_root: impl Fn(&str, &str) -> bool,
) -> Vec<(String, String, u64, String)> {
    let end = saved_at.unwrap_or_else(|| clock::unix(clock::now()));
    let mut out: Vec<(String, String, u64, String)> = vec![];
    let mut root_seen = false;
    for m in history.iter().rev().filter(|m| m.role == tools::Role::User) {
        let mut f = m.content.splitn(3, '|');
        let (Some(id), Some(ch)) = (f.next(), f.next()) else { continue };
        if id == tools::ID_SYSTEM
            || ch == tools::CH_CONSOLE
            || ch == tools::CH_SCHEDULE
            || ch.starts_with(jobs::PREFIX)
        {
            continue;
        }
        let at = memory::issued_unix(&m.content).unwrap_or(0);
        let root = is_root(id, ch);
        let recent = end.saturating_sub(at) <= TALK_WINDOW_SECS;
        if !(recent || (was_update && root && !root_seen)) {
            if root_seen || !was_update {
                break;
            }
            continue;
        }
        root_seen |= root;
        if out.iter().any(|o| o.1 == ch) {
            continue;
        }
        let said: String = memory::envelope_body(&m.content).split_whitespace().collect::<Vec<_>>().join(" ").chars().take(160).collect();
        out.push((id.to_string(), ch.to_string(), at, said));
        if out.len() >= 5 {
            break;
        }
    }
    out
}

/// How long before the restart a message still counts as part of the talk that led to it.
const TALK_WINDOW_SECS: u64 = 2 * 3600;

/// Added to the boot notice when people wrote from chats just before the restart.
fn report_back(talkers: &[(String, String, u64, String)], was_update: bool) -> String {
    let mut s = String::from("Before this restart these people wrote to you (newest first):");
    for (id, ch, at, said) in talkers {
        s.push_str(&format!("\n- {id} on {ch}, {} ago: \"{said}\"", clock::ago(*at)));
    }
    let what = if was_update {
        "that you are back after the update, the new version and what changed"
    } else {
        "that you are back after the restart"
    };
    s.push_str(&format!("\nThey are not at the screen. Look at the conversation before this notice: everyone who asked for this restart or update, or whom you told you would report or check something after it, gets a short report now on their own channel with say(identity, channel), in their language: {what}, and the outcome of what you said you would check. Root wants it after every update. Those says are part of this notice; do not skip them."));
    s
}

/// Appended to every boot notice: catch-up messages (people who wrote while bia was off,
/// jobs cut short) are delivered in the same turn and must not inherit the notice's
/// "no tools" rule.
const BOOT_BATCH: &str = "\nThat instruction is for this notice only. Messages delivered together with it (people who wrote while you were off, jobs or sends cut short by the restart) are normal: answer and handle each of them as usual, with tools when they need them.";

/// A home language other than English goes into the system prompt, so bia speaks it
/// from the first boot reply on.
fn with_home_language(mut prompt: String) -> String {
    let code = bia_speech::registry::current().default_lang.clone();
    if !code.is_empty() && code != "en" {
        let name = bia_speech::langs::name(&code);
        prompt.push_str(&format!(
            "\nYour owner's language is {name} ({code}): write to root, on console and web, and in boot replies in {name}; answer anyone else in the language they write in."
        ));
    }
    prompt
}

fn first_chars(s: &str, n: usize) -> String {
    let t: String = s.chars().take(n).collect();
    if s.chars().count() > n {
        format!("{t}...")
    } else {
        t
    }
}

#[cfg(test)]
mod boot_notice_tests {
    use crate::tools::Msg;

    fn env(id: &str, ch: &str, at: u64, text: &str) -> Msg {
        Msg::user(format!("{id}|{ch}|{}|{}|m|{text}", crate::clock::rfc3339_unix(at), crate::clock::rfc3339_unix(at)))
    }

    #[test]
    fn talkers_before_the_restart() {
        let t0 = 1_800_000_000;
        let h = vec![
            env("ZonD", "telegram:1", t0 - 86400, "hello"),
            env("Ann", "whatsapp:49", t0 - 7200 - 60, "old"),
            env("ZonD", "telegram:1", t0 - 1200, "update to 1.1.9"),
            env("system", "console", t0 - 600, "notice"),
            env("root", "console", t0 - 500, "typed"),
            env("Bob", "web:c1", t0 - 300, "and me?"),
        ];
        let root = |id: &str, _: &str| id == "ZonD";
        let t = super::last_talkers(&h, Some(t0), false, root);
        let chans: Vec<&str> = t.iter().map(|x| x.1.as_str()).collect();
        assert_eq!(chans, ["web:c1", "telegram:1"]);
        assert_eq!(t[1].3, "update to 1.1.9");
        // root's chat counts at any age after an update
        let h = vec![env("ZonD", "telegram:1", t0 - 86400, "hello"), env("Ann", "whatsapp:49", t0 - 9000, "old")];
        assert!(super::last_talkers(&h, Some(t0), false, root).is_empty());
        let t = super::last_talkers(&h, Some(t0), true, root);
        assert_eq!(t.iter().map(|x| x.1.as_str()).collect::<Vec<_>>(), ["telegram:1"]);
        let r = super::report_back(&t, true);
        assert!(r.contains("ZonD on telegram:1") && r.contains("new version"));
    }

    #[test]
    fn catch_up_replaces_the_ready_order() {
        let notice = format!("System booted. {}{}", super::READY_PLAIN, super::BOOT_BATCH);
        let t = notice.replace(super::READY_PLAIN, &super::catch_up(2));
        assert!(
            !t.contains("ready for commands.")
                && t.contains("2 message(s) that came while you were off")
        );
        assert!(t.contains("Do not say you are ready for commands") && t.contains("with tools"));
    }
}

#[cfg(test)]
mod welcome_tests {
    #[test]
    fn welcome_names_the_topics() {
        let w = super::welcome(
            Some("web: https://10.0.0.5/  CA https://10.0.0.5/v1/pki/ca".into()),
            false,
        );
        for want in [
            "very first start",
            "Write in English",
            "https://10.0.0.5/",
            "Telegram and WhatsApp",
            "own accounts",
            "ChatGPT plan",
            "what your name should be",
            "Do not call tools for this greeting",
        ] {
            assert!(w.contains(want), "missing {want:?} in {w}");
        }
        let w = super::welcome(Some("web: (no network yet)  CA at /v1/pki/ca".into()), true);
        assert!(w.contains("once you are on a network") && !w.contains("ChatGPT plan"));
    }
}
