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
mod hitl;
mod hw;
mod jobs;
mod llama_srv;
mod llm;
mod mcp;
mod memory;
mod openapi;
mod remote;
mod repl;
mod schedules;
mod state;
mod sys;
mod tools;
mod tune;
mod tz;
mod unanswered;
mod web;
mod webcall;
mod webchat;

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
    let web = if cfg.web.enabled {
        // System: HTTPS :443 + HTTP→HTTPS on :80. Dev: HTTPS :8443 only.
        let port = if system_mode { 443 } else { 8443 };
        let http_port = if system_mode { 80 } else { 0 };
        match web::Web::start(con.clone(), port, http_port, paths.files.clone()) {
            Ok((w, warn)) => {
                if let Some(wmsg) = warn {
                    boot.warn(&wmsg);
                }
                boot.note(&format!("web console listening on port {}", w.port));
                Some(w)
            }
            Err(e) => {
                boot.warn(&e);
                None
            }
        }
    } else {
        None
    };
    let fmm_path = paths.files.join("state").join("fmm.json");
    if let Some(msg) = remote::FmmConfig::seed_from_esp(&paths.boot, &fmm_path) {
        boot.note(&msg);
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
    let mut local_spec = model_path.as_ref().zip(plan.as_ref()).map(|(path, plan)| {
        llm::LocalSpec {
            path: path.clone(),
            cfg: model_cfg.clone(),
            sampling: cfg.sampling.clone(),
            plan: plan.clone(),
        }
    });
    let llm = if skip_local {
        boot.step(
            80,
            &match model_path.as_ref().and_then(|p| p.file_name()) {
                Some(n) => format!("FMM is configured; not loading local model {}", n.to_string_lossy()),
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
            llm::Llm::load(&model_path, mctx, &cfg.sampling, &plan, move |f| {
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
    if let Some(w) = &web {
        w.refresh();
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

    // Re-start MCPs that were loaded last session (runs boot_tool e.g. telegram restore).
    for name in &resume_mcps {
        match tools.ensure_loaded(name) {
            Ok(()) => boot.note(&format!("MCP {name}: resumed")),
            Err(e) => boot.warn(&format!("MCP {name}: resume failed: {e:#}")),
        }
    }
    // If a Telegram session file exists, ensure the MCP is up even when not in loaded_mcps.
    let tg_session = paths.files.join("telegram").join("ferogram.session");
    if tg_session.exists() && !tools.mcp_started("telegram") {
        match tools.ensure_loaded("telegram") {
            Ok(()) => boot.note("MCP telegram: session restored"),
            Err(e) => boot.warn(&format!("MCP telegram: restore failed: {e:#}")),
        }
    }
    // WhatsApp linked-device session (SQLite under files/whatsapp/).
    let wa_session = paths.files.join("whatsapp").join("whatsapp.db");
    if wa_session.exists() && !tools.mcp_started("whatsapp") {
        match tools.ensure_loaded("whatsapp") {
            Ok(()) => boot.note("MCP whatsapp: session restored"),
            Err(e) => boot.warn(&format!("MCP whatsapp: restore failed: {e:#}")),
        }
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
    let system = cfg.agent.system_prompt.clone();
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
        web: web.clone(),
        memory: crate::memory::Store::open(&paths.files.join("state")),
        fmm_ctx_used: 0,
        fmm_backups: Default::default(),
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
        boot.note(&format!("FMM: waiting for network before enabling ({host})"));
        let ready = sys::wait_network(Duration::from_secs(90), Some(&host), |msg| {
            boot.note(msg);
        });
        let mut line = repl.reload_fmm(true);
        // Brief retries: route/DNS can be up a moment before the API path works.
        if ready && line.contains("unreachable") {
            for attempt in 2..=6 {
                boot.note(&format!(
                    "FMM: still unreachable; retry {attempt}/6 in 5s"
                ));
                std::thread::sleep(Duration::from_secs(5));
                line = repl.reload_fmm(true);
                if !line.contains("unreachable") {
                    break;
                }
            }
        }
        boot.note(&line);
    }
    if let Some(w) = &web {
        boot.note(&w.info_line());
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
    let hw = crate::hw::summary();
    boot.note(&hw);
    boot.note(&how);
    // One system envelope, then a model turn. Identity is system, not root.
    repl.pending.insert(
        0,
        console::Inbound {
            identity: tools::ID_SYSTEM.into(),
            channel: tools::CH_CONSOLE.into(),
            issued: clock::now(),
            id: String::new(),
            text: format!(
                "{how}. {}",
                if how.contains("Repair these yourself")
                    || how.contains("Propose fixes to the user and wait for approval")
                {
                    "Tell the user this sentence, then repair it yourself without waiting: for each package that did not install, search the internet, download the .deb for the version and architecture named here and install it; copy binaries that were not copied into place. Ask the user first only before a destructive step (removing or overwriting existing files, packages or binaries, up- or downgrading installed packages, disk or partition changes, reboot or poweroff). When done, tell the user what you fixed and what still fails."
                } else if how.contains("this boot landed") || how.contains("this boot restored") {
                    "Say that sentence to the user, then say you are ready for commands. Do not call tools."
                } else {
                    "Reply that you are ready for commands. Do not call tools."
                }
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
    if !replay.is_empty() {
        boot.note(&format!("{} message(s) received before the restart, not yet answered", replay.len()));
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

fn first_chars(s: &str, n: usize) -> String {
    let t: String = s.chars().take(n).collect();
    if s.chars().count() > n {
        format!("{t}...")
    } else {
        t
    }
}
