//! The prompt loop: commands, chat turns, tool rounds, streaming output, persistence.
use crate::console::{
    Console, Inbound, BCYAN, BGREEN, BOLD, CYAN, GRAY, GREEN, RED, RESET, YELLOW,
};
use crate::llm::{LlamaToken, Llm, LocalSpec, Stop};
use crate::remote::{fmm_ctx, FmmConfig, Remote};
use crate::state;
use crate::tools::{
    envelope, human, normalize_history, Msg, Role, ToolCall, ToolFormat, Tools, CH_CONSOLE,
    ID_SYSTEM,
};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;

/// Language of the first `[speech xx]` tag in a message (`?` = not learned yet).
fn speech_lang(text: &str) -> Option<String> {
    let i = text.find("[speech ")?;
    let rest = &text[i + 8..];
    let code = rest[..rest.find(']')?].trim().trim_end_matches('?');
    (!code.is_empty() && code.len() <= 4 && code.chars().all(|c| c.is_ascii_lowercase()))
        .then(|| code.to_string())
}

/// The physical console (screen, serial). Every other channel — web included — is a bus.
fn local_channel(ch: &str) -> bool {
    ch == CH_CONSOLE
}

/// Run `bia-update <args>` and stream stderr lines to `on_line`. Blocks until exit/reboot.
fn run_bia_update(args: &[&str], mut on_line: impl FnMut(&str)) -> Result<(), String> {
    let mut child = Command::new("/usr/local/sbin/bia-update")
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("bia-update: {e}"))?;
    if let Some(err) = child.stderr.take() {
        for line in BufReader::new(err).lines().flatten() {
            if !line.is_empty() {
                on_line(&line);
            }
        }
    }
    let status = child.wait().map_err(|e| format!("bia-update: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("exit {}", status.code().unwrap_or(-1)))
    }
}

/// Slash command typed on the physical console, without the leading `/`.
fn console_command(ib: &Inbound) -> Option<&str> {
    if !local_channel(&ib.channel) {
        return None;
    }
    ib.text.trim().strip_prefix('/')
}

/// A human message that the model must eventually close with answered=[ids].
/// The physical console is not tracked: the person there watches the model work.
fn needs_answer(ib: &Inbound) -> bool {
    if ib.identity == ID_SYSTEM
        || ib.channel == CH_CONSOLE
        || ib.channel.starts_with(crate::jobs::PREFIX)
        || ib.channel == crate::tools::CH_SCHEDULE
        || console_command(ib).is_some()
    {
        return false;
    }
    let t = ib.text.trim_start();
    // call state events ([call ready], [call ended], …); [incoming call] still needs a response
    let call_event = ["[call ", "[voice "].iter().any(|p| t.starts_with(p));
    !t.is_empty()
        && !t.starts_with("[reaction]")
        && !t.starts_with("[deleted]")
        && !t.starts_with("[live location ended]")
        && !call_event
}

/// `false` stops generation. A slash command is queued and stops. Other input is queued and the model continues. A blank Enter stops when `stop_on_blank` is set.
fn keep_generating(
    con: &Console,
    queued: &std::cell::RefCell<Vec<Inbound>>,
    stop_on_blank: bool,
) -> bool {
    match con.try_read_inbound() {
        None => true,
        Some(ib) if local_channel(&ib.channel) && ib.text.trim().is_empty() => {
            if stop_on_blank {
                false
            } else {
                con.hold_inbound(ib);
                true
            }
        }
        Some(ib) if console_command(&ib).is_some() => {
            con.write(&con.c(GRAY, &format!("\n  (stopping for {})\n", ib.text.trim())));
            queued.borrow_mut().push(ib);
            false
        }
        Some(ib) => {
            con.write(&con.c(
                GRAY,
                &format!(
                    "\n  (queued: {}@{} {})\n",
                    ib.identity,
                    ib.channel,
                    ib.text.trim()
                ),
            ));
            queued.borrow_mut().push(ib);
            true
        }
    }
}

pub struct Repl {
    pub con: Arc<Console>,
    pub llm: Option<Llm>,
    pub local_spec: Option<LocalSpec>,
    pub tools: Tools,
    pub format: ToolFormat,
    /// Chat template opens the assistant turn with `<think>`.
    pub think: bool,
    pub system: String,
    pub history: Vec<Msg>,
    pub max_tool_rounds: usize,
    pub session_log: Option<PathBuf>,
    pub state_path: PathBuf,
    /// messages that arrived while the model was busy
    pub pending: Vec<Inbound>,
    /// job results that arrived during a reply; delivered into the same turn
    pub job_ready: Vec<Inbound>,
    pub show_thinking: bool,
    /// Frontier Model Mode: the remote brain, when configured and enabled
    pub remote: Option<Remote>,
    pub fmm_path: PathBuf,
    pub memory: crate::memory::Store,
    /// last known FMM prompt+completion tokens (header ctx meter; 0 until the first remote reply)
    pub fmm_ctx_used: usize,
    pub fmm_backups: std::collections::HashMap<String, Vec<u8>>,
}

pub enum Exit {
    Reboot,
    Poweroff,
    Quit,
}

impl Repl {
    pub fn run(&mut self) -> Exit {
        self.con.writeln("");
        self.con.writeln(&self.con.c(GRAY, "Type a message. Commands: /help /tools /status /forget /reboot /poweroff. Press Enter while the agent is writing to interrupt it. A /command stops the model and runs immediately."));
        self.set_state("idle");
        loop {
            self.tools.reap_dead_mcps();
            if !self.job_ready.is_empty() {
                let jobs = std::mem::take(&mut self.job_ready);
                self.pending.splice(0..0, jobs);
            }
            // green prompt; the kernel echoes the typed line in the same color
            // deliver messages that arrived while we were busy
            if !self.pending.is_empty() {
                let batch = std::mem::take(&mut self.pending);
                let (cmds, batch): (Vec<_>, Vec<_>) = batch
                    .into_iter()
                    .partition(|ib| console_command(ib).is_some());
                self.pending = batch;
                for ib in cmds {
                    let cmd = console_command(&ib).unwrap_or("");
                    self.con
                        .writeln(&self.con.c(GRAY, &format!("  running /{cmd}")));
                    if let Some(e) = self.command(cmd) {
                        return e;
                    }
                }
                let batch = std::mem::take(&mut self.pending);
                if batch.is_empty() {
                    continue;
                }
                self.con.writeln(&self.con.c(
                    GRAY,
                    &format!("  delivering {} queued message(s)", batch.len()),
                ));
                self.turn(batch);
                if let Some(e) = self.run_pending_commands() {
                    return e;
                }
                self.after_turn();
                if let Some(p) = self.tools.pending_exit.take() {
                    self.save_state(true);
                    return if p.poweroff {
                        Exit::Poweroff
                    } else {
                        Exit::Reboot
                    };
                }
                continue;
            }
            self.con.write(&format!(
                "\n{}{} ",
                self.con.c(BGREEN, "->"),
                if self.con.color { GREEN } else { "" }
            ));
            let ib = loop {
                let now = crate::clock::unix(crate::clock::now());
                let wait = self
                    .tools
                    .unanswered
                    .secs_until_due(now)
                    .map(|s| s.clamp(1, crate::unanswered::TICK_SECS));
                let Some(wait) = wait else {
                    break self.con.read_inbound();
                };
                match self
                    .con
                    .read_inbound_timeout(std::time::Duration::from_secs(wait))
                {
                    Ok(ib) => break ib,
                    Err(()) => {
                        if let Some(n) = self.unanswered_due() {
                            break Some(n);
                        }
                    }
                }
            };
            self.con.write(RESET);
            let Some(ib) = ib else { return Exit::Quit };
            if local_channel(&ib.channel) {
                let line = ib.text.trim().to_string();
                if line.is_empty() {
                    continue;
                }
                if let Some(cmd) = console_command(&ib) {
                    match self.command(cmd) {
                        Some(e) => return e,
                        None => continue,
                    }
                }
                self.turn(vec![Inbound { text: line, ..ib }]);
            } else {
                if ib.text.trim().is_empty() {
                    continue;
                }
                self.turn(vec![ib]);
            }
            if let Some(e) = self.run_pending_commands() {
                return e;
            }
            self.after_turn();
            if let Some(p) = self.tools.pending_exit.take() {
                self.save_state(true);
                return if p.poweroff {
                    Exit::Poweroff
                } else {
                    Exit::Reboot
                };
            }
        }
    }

    fn set_state(&self, st: &str) {
        let msgs = self.history.len();
        let total = self.ctx_budget();
        let used = if self.remote.is_some() {
            self.fmm_ctx_used
                .max(self.history_token_estimate())
                .min(total)
        } else {
            self.llm.as_ref().map(|l| l.cached_tokens()).unwrap_or(0)
        };
        self.con.update_status(|s| {
            s.state = st.to_string();
            s.ctx_used = used;
            s.ctx_total = total;
            s.messages = msgs;
        });
    }

    fn history_token_estimate(&self) -> usize {
        self.history
            .iter()
            .map(|m| m.content.chars().count().div_ceil(4))
            .sum()
    }

    /// Read files/state/fmm.json and switch brains accordingly. Returns a status line.
    pub fn reload_fmm(&mut self, test: bool) -> String {
        let was_remote = self.remote.is_some();
        let cfg = FmmConfig::load(&self.fmm_path);
        let line = match cfg {
            None => {
                self.remote = None;
                "FMM: not configured, local model".to_string()
            }
            Some(c) if !c.enabled => {
                self.remote = None;
                "FMM: disabled, local model".to_string()
            }
            Some(c) => match Remote::from_config(&c).map(|r| r.with_store(&self.fmm_path)) {
                Err(e) => {
                    self.remote = None;
                    format!("FMM: invalid configuration ({e}); local model")
                }
                Ok(r) => {
                    if test {
                        match r.test() {
                            Ok(msg) => {
                                let l = format!("FMM: on, {} ctx {} ({msg})", r.label(), r.ctx);
                                self.remote = Some(r);
                                l
                            }
                            Err(e) => {
                                self.remote = None;
                                format!("FMM: {} unreachable ({e:#}); local model not loaded. /fmm on to retry or /fmm off to load the GGUF", r.label())
                            }
                        }
                    } else {
                        let l = format!("FMM: on, {} ctx {}", r.label(), r.ctx);
                        self.remote = Some(r);
                        l
                    }
                }
            },
        };
        let want_local = self.remote.is_none()
            && !matches!(FmmConfig::load(&self.fmm_path), Some(c) if c.enabled);
        if want_local {
            if let Err(e) = self.ensure_local() {
                return format!("{line} ({e})");
            }
        }
        if was_remote && self.remote.is_none() {
            if let Some(l) = self.llm.as_mut() {
                l.reset();
            }
        }
        let model = match &self.remote {
            Some(r) => format!("FMM {}", r.label()),
            None => self
                .llm
                .as_ref()
                .map(|l| l.info.name.clone())
                .unwrap_or_else(|| "none".into()),
        };
        let detail = if self.remote.is_some() {
            String::new()
        } else if let Some(l) = &self.llm {
            format!("{} {}", human_count(l.info.n_params), l.info.arch)
        } else {
            String::new()
        };
        let ctx_total = self.ctx_budget();
        self.con.update_status(|st| {
            st.model = model;
            st.model_detail = detail;
            st.ctx_total = ctx_total;
        });
        line
    }

    fn ctx_budget(&self) -> usize {
        if let Some(r) = &self.remote {
            r.ctx as usize
        } else {
            self.llm.as_ref().map(|l| l.n_ctx).unwrap_or(0)
        }
    }

    fn persist_local_ctx(&self, n: u32) {
        let p = self.state_path.with_file_name("local.json");
        let mut rt = crate::config::LocalRuntime::load(&p);
        rt.ctx = Some(n);
        let _ = rt.save(&p);
        unsafe { libc::sync() };
    }

    fn push_history(&mut self, mut m: crate::tools::Msg) {
        if m.id.is_empty() {
            m.id = self.memory.next_msg_id();
        }
        let role = match m.role {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        };
        let _ = self
            .memory
            .journal(&m.id, role, &m.name, &m.content, &m.call_id);
        self.history.push(m);
    }

    fn live_tokens(&self) -> usize {
        if self.remote.is_some() {
            // FMM: prefer last API-reported usage. Local ChatML char/4 is not the
            // OpenAI/Anthropic token count and was missing the watermark entirely.
            return self.fmm_ctx_used.max(self.history_token_estimate());
        }
        let prompt = self.render_prompt(true);
        if let Some(l) = &self.llm {
            l.tokenize(&prompt, l.info.has_bos)
                .map(|t| t.len())
                .unwrap_or(prompt.len() / 4)
        } else {
            prompt.len() / 4
        }
    }

    fn spine_len(&self) -> usize {
        let mut i = self.history.len();
        while i > 0
            && !crate::memory::is_stub(&self.history[i - 1].name, &self.history[i - 1].content)
        {
            i -= 1;
        }
        i
    }

    fn ensure_room(&mut self) {
        for _ in 0..4 {
            let lim = self.ctx_budget();
            if lim == 0 {
                return;
            }
            // Must match need_watermark / compact_once — the old `+ 16` gate returned
            // before compact whenever we were in the watermark band (lim-2048 .. lim-16).
            if !self.tools.pending_reflect.get() && !self.need_watermark() {
                return;
            }
            if !self.compact_once() {
                return;
            }
        }
    }

    fn compact_once(&mut self) -> bool {
        let spine = self.spine_len();
        let tail = self.history.len() - spine;
        let forced = self.tools.pending_reflect.get();
        if !forced && !self.need_watermark() {
            return false;
        }
        const SPINE_STUB_LIMIT: usize = 8;
        if spine > SPINE_STUB_LIMIT && tail == 0 {
            return self.compact_range(0, spine, true);
        }
        if tail > 0 {
            return self.compact_range(spine, self.history.len(), false);
        }
        if spine > 0 {
            return self.compact_range(0, spine, true);
        }
        self.tools.pending_reflect.set(false);
        false
    }

    fn compact_range(&mut self, start: usize, end: usize, rollup: bool) -> bool {
        if start >= end || end > self.history.len() {
            return false;
        }
        let span: Vec<Msg> = self.history[start..end].to_vec();
        let t_start = span
            .iter()
            .find_map(|m| crate::memory::issued_of(&m.content))
            .unwrap_or_else(crate::clock::rfc3339_now);
        let t_end = span
            .iter()
            .rev()
            .find_map(|m| crate::memory::issued_of(&m.content))
            .unwrap_or_else(|| t_start.clone());
        let msg_ids: Vec<String> = span
            .iter()
            .map(|m| m.id.clone())
            .filter(|s| !s.is_empty())
            .collect();
        let child_ids: Vec<String> = if rollup {
            span.iter()
                .filter_map(|m| crate::memory::stub_id(&m.content))
                .collect()
        } else {
            vec![]
        };
        let depth = if rollup {
            1 + span
                .iter()
                .map(|m| crate::memory::stub_depth(&m.content))
                .max()
                .unwrap_or(0)
        } else {
            0
        };
        self.set_state("reflecting");
        self.con.writeln(&self.con.c(
            GRAY,
            &format!(
                "  [reflecting {} .. {} ({} {}, depth {})]",
                t_start,
                t_end,
                span.len(),
                if rollup { "stubs" } else { "messages" },
                depth
            ),
        ));
        let known = self.memory.category_names();
        let instr = crate::memory::instruction(&t_start, &t_end, &known, rollup);
        self.history.push(Msg::user(envelope(
            ID_SYSTEM,
            CH_CONSOLE,
            crate::clock::now(),
            &crate::console::mint_msg_id(),
            &instr,
        )));
        let Some(text) = self.generate_reflect() else {
            self.history.pop();
            self.set_state("idle");
            return false;
        };
        self.history.pop();
        let mut parsed = crate::memory::parse_blocks(&text, &t_start, &t_end);
        if parsed.len() < 2 {
            let idents = span_identities(&span);
            let tools = span_tools(&span);
            let cats: Vec<String> = span
                .iter()
                .filter_map(|m| crate::memory::stub_field(&m.content, "category"))
                .collect();
            for extra in
                crate::memory::mechanical(&t_start, &t_end, span.len(), &idents, &tools, &cats)
            {
                let cat = crate::memory::normalize_category(&extra.category);
                if !parsed
                    .iter()
                    .any(|p| crate::memory::normalize_category(&p.category) == cat)
                {
                    parsed.push(extra);
                }
            }
        }
        if parsed.is_empty() {
            parsed = crate::memory::mechanical(
                &t_start,
                &t_end,
                span.len(),
                &span_identities(&span),
                &span_tools(&span),
                &[],
            );
        }
        let recs = self.memory.build_records(
            parsed,
            depth,
            if rollup { vec![] } else { msg_ids },
            child_ids,
            &t_start,
            &t_end,
        );
        if let Err(e) = self.memory.save_records(&recs) {
            self.con.writeln(
                &self
                    .con
                    .c(RED, &format!("  [could not save reflections: {e:#}]")),
            );
            return false;
        }
        let tail = self.history.split_off(end);
        let _gone = self.history.split_off(start);
        for rec in &recs {
            let mut m = Msg::user(envelope(
                ID_SYSTEM,
                CH_CONSOLE,
                crate::clock::now(),
                &crate::console::mint_msg_id(),
                &crate::memory::stub_body(rec),
            ));
            m.name = crate::memory::stub_name().into();
            self.push_history(m);
            let preview: String = rec.text.chars().take(120).collect();
            self.con.writeln(&self.con.c(
                GRAY,
                &format!(
                    "  [{}] {} .. {}  {}",
                    rec.category,
                    rec.t_start,
                    rec.t_end,
                    preview.replace('\n', " ")
                ),
            ));
        }
        self.history.extend(tail);
        if let Some(l) = self.llm.as_mut() {
            l.reset();
        }
        // Drop stale API usage so the next watermark check sees the compacted window.
        self.fmm_ctx_used = self.history_token_estimate();
        self.tools.pending_reflect.set(false);
        self.save_state(false);
        true
    }

    /// `None` when a slash command or a blank Enter stopped the reflection.
    fn generate_reflect(&mut self) -> Option<String> {
        let queued: std::cell::RefCell<Vec<Inbound>> = std::cell::RefCell::new(vec![]);
        let out = if self.remote.is_some() {
            let history = self
                .tools
                .fold_invoke_for_remote(normalize_history(self.format, &self.history));
            let system = self.system.clone();
            let remote = self.remote.as_ref().unwrap();
            let con = self.con.clone();
            let mut shown = Printer::new(con.clone(), "\u{0}never", self.show_thinking, false);
            match remote.generate(
                &system,
                &self.tools.prompt_specs(),
                &history,
                &mut |thought, piece| {
                    if thought {
                        shown.push_thought(piece);
                    } else {
                        shown.push(piece);
                    }
                    keep_generating(&con, &queued, false)
                },
                false,
                std::time::Duration::from_secs(180),
            ) {
                Ok(r) => {
                    shown.flush();
                    if r.stop == "interrupted" {
                        None
                    } else {
                        Some(r.text)
                    }
                }
                Err(e) => {
                    shown.flush();
                    self.con
                        .writeln(&self.con.c(RED, &format!("  [reflect: {e:#}]")));
                    Some(String::new())
                }
            }
        } else if self.llm.is_some() {
            let prompt = self.fit_history();
            let con = self.con.clone();
            let mut shown = Printer::new(con.clone(), "\u{0}never", self.show_thinking, false);
            let con2 = self.con.clone();
            match self.llm.as_mut().unwrap().generate(
                &prompt,
                1536,
                |done, total| {
                    if total > 256 {
                        con2.write(&format!(
                            "\r{}",
                            con2.c(GRAY, &format!("[reading {done}/{total} tokens]"))
                        ));
                    }
                    keep_generating(&con, &queued, false)
                },
                |piece| {
                    shown.push(piece);
                    keep_generating(&con, &queued, false)
                },
            ) {
                Ok((text, stats)) => {
                    shown.flush();
                    con.write("\r                                        \r");
                    if stats.stop == Stop::Interrupted {
                        None
                    } else {
                        Some(text)
                    }
                }
                Err(e) => {
                    shown.flush();
                    self.con
                        .writeln(&self.con.c(RED, &format!("  [reflect: {e}]")));
                    Some(String::new())
                }
            }
        } else {
            Some(String::new())
        };
        self.stash_busy(queued.into_inner());
        out
    }

    /// Apply a context-window change to the active brain. Unloads then reloads; restores the previous settings on failure.
    fn apply_context(&mut self, tokens: u32) -> String {
        if self.remote.is_some() {
            let prev = FmmConfig::load(&self.fmm_path);
            let Some(mut c) = prev.clone() else {
                return "FMM is on but fmm.json is missing; context not changed".into();
            };
            let old = fmm_ctx(c.ctx);
            let want = fmm_ctx(tokens);
            c.ctx = want;
            if let Err(e) = c.save(&self.fmm_path) {
                return format!("could not save FMM context {want}: {e:#}; kept {old}");
            }
            match Remote::from_config(&c).map(|r| r.with_store(&self.fmm_path)) {
                Ok(r) => {
                    self.remote = Some(r);
                    self.con.update_status(|s| s.ctx_total = want as usize);
                    format!("FMM context {want} tokens (was {old})")
                }
                Err(e) => {
                    if let Some(old_c) = prev {
                        let _ = old_c.save(&self.fmm_path);
                        if let Ok(r) =
                            Remote::from_config(&old_c).map(|r| r.with_store(&self.fmm_path))
                        {
                            self.remote = Some(r);
                        }
                    }
                    format!("FMM context {want} failed ({e:#}); restored {old}")
                }
            }
        } else {
            let old = {
                let Some(spec) = self.local_spec.as_mut() else {
                    return "no local model path".into();
                };
                let old = spec.cfg.ctx;
                spec.cfg.ctx = tokens;
                old
            };
            self.con.writeln(&self.con.c(
                GRAY,
                &format!("  unloading context {old}, loading {tokens}…"),
            ));
            if self.llm.is_none() {
                match self.ensure_local() {
                    Ok(()) => {
                        let live = self.llm.as_ref().map(|l| l.n_ctx as u32).unwrap_or(tokens);
                        self.persist_local_ctx(live);
                        if let Some(s) = self.local_spec.as_mut() {
                            s.cfg.ctx = live;
                        }
                        self.con.update_status(|s| s.ctx_total = live as usize);
                        format!("local context {live} tokens (was {old})")
                    }
                    Err(e) => {
                        if let Some(s) = self.local_spec.as_mut() {
                            s.cfg.ctx = old;
                        }
                        format!("local context {tokens} failed ({e}); restored {old}")
                    }
                }
            } else {
                match self.llm.as_mut().unwrap().reload_ctx(tokens) {
                    Ok(live) => {
                        if let Some(s) = self.local_spec.as_mut() {
                            s.cfg.ctx = live;
                        }
                        self.persist_local_ctx(live);
                        self.con.update_status(|s| s.ctx_total = live as usize);
                        let fit = {
                            let prompt = self.render_prompt(true);
                            match self
                                .llm
                                .as_ref()
                                .and_then(|l| l.tokenize(&prompt, l.info.has_bos).ok())
                            {
                                Some(t) if t.len() + 16 > live as usize => {
                                    format!("; live prompt is {} tokens and will not fit until it is compacted", t.len())
                                }
                                _ => String::new(),
                            }
                        };
                        if live != tokens {
                            format!("local context {live} tokens (asked {tokens}, clamped to trained window; was {old}){fit}")
                        } else {
                            format!("local context {live} tokens (was {old}){fit}")
                        }
                    }
                    Err(e) => {
                        if let Some(s) = self.local_spec.as_mut() {
                            s.cfg.ctx = old;
                        }
                        self.persist_local_ctx(old);
                        self.con.update_status(|s| {
                            s.ctx_total = self.llm.as_ref().map(|l| l.n_ctx).unwrap_or(old as usize)
                        });
                        format!("{e}")
                    }
                }
            }
        }
    }

    fn ensure_local(&mut self) -> Result<(), String> {
        if self.llm.is_some() {
            return Ok(());
        }
        let Some(spec) = self.local_spec.clone() else {
            return Err("no local model path".into());
        };
        self.con.writeln(&self.con.c(
            GRAY,
            &format!(
                "  loading local model {}…",
                spec.path.file_name().unwrap_or_default().to_string_lossy()
            ),
        ));
        let llm = Llm::load(&spec.path, &spec.cfg, &spec.sampling, &spec.plan, |_| true)
            .map_err(|e| format!("{e:#}"))?;
        self.format = ToolFormat::detect(&llm.info.chat_template);
        self.think = ToolFormat::opens_think(&llm.info.chat_template);
        self.llm = Some(llm);
        Ok(())
    }

    /// This machine's CA (files/state/pki): client certificates for the web MCP.
    fn pki(&self) -> Result<bia_pki::Pki, String> {
        bia_pki::Pki::load_or_create(&self.tools.files_dir).map_err(|e| format!("{e:#}"))
    }

    fn print_web_info(&self) {
        let line = crate::weblink::info_line(&self.tools.files_dir);
        self.con.writeln(&self.con.c(GRAY, &format!("  {line}")));
    }

    /// Things the model scheduled with tools: forgetting the conversation.
    fn after_turn(&mut self) {
        self.tools.reap_dead_mcps();
        if self.tools.pending_forget.take() {
            self.forget_all();
            self.con.writeln(
                &self
                    .con
                    .c(GRAY, "conversation forgotten (requested by the agent)"),
            );
            return;
        }
        if self.tools.pending_reflect.get() || self.need_watermark() {
            self.ensure_room();
        }
    }

    /// True when the live window is within 2048 tokens of the context budget.
    fn need_watermark(&self) -> bool {
        let lim = self.ctx_budget();
        lim > 0 && self.live_tokens().saturating_add(2048) >= lim
    }

    /// Start from scratch: history, model context, loaded MCPs, queue and the saved files.
    pub fn forget_all(&mut self) {
        self.history.clear();
        self.pending.clear();
        self.job_ready.clear();
        self.tools.unanswered.clear();
        self.tools.loaded.clear();
        if let Some(l) = self.llm.as_mut() {
            l.reset();
        }
        state::forget(&self.state_path);
        let _ = std::fs::remove_file(self.state_path.with_extension("json.bak"));
        self.memory.forget();
        self.set_state("idle");
    }

    fn save_state(&self, announce: bool) {
        let pending: Vec<state::SavedPending> = self
            .pending
            .iter()
            .chain(self.job_ready.iter())
            .map(state::SavedPending::from_inbound)
            .collect();
        let name = self
            .remote
            .as_ref()
            .map(|r| format!("FMM {}", r.label()))
            .or_else(|| self.llm.as_ref().map(|l| l.info.name.clone()))
            .unwrap_or_else(|| "none".into());
        match state::save(
            &self.state_path,
            &name,
            &self.history,
            self.tools.loaded_names(),
            pending,
        ) {
            Ok(()) => {
                if announce {
                    self.con.writeln(&self.con.c(
                        GRAY,
                        &format!(
                            "conversation saved ({} messages) -> {}",
                            self.history.len(),
                            self.state_path.display()
                        ),
                    ));
                }
            }
            Err(e) => self.con.writeln(
                &self
                    .con
                    .c(RED, &format!("could not save conversation: {e:#}")),
            ),
        }
    }

    fn command(&mut self, cmd: &str) -> Option<Exit> {
        let mut it = cmd.split_whitespace();
        match it.next().unwrap_or("") {
            "help" | "?" => self.con.writeln(&self.con.c(GRAY,
                "/tools     list tools (built-in and from MCPs)\n/status    memory, context usage, model\n/ctx [n]   show or set the active brain's context window (unload/reload; previous restored on failure)\n/reflect   compact the live window into dated categorized memories now\n/save      save the conversation now (it is also autosaved after every turn)\n/fmm [on|off]  Frontier Model Mode status / switch (configured via the system MCP)\n/update [force] [/dev/…|internet]  update this OS via the rescue image: from the attached bia system (flash or disk), else the newest GitHub release over the internet\n/rescue [/dev/…]  from the flash: rewrite the internal disk with this system (keeps the disk's agent)\n/web       show the web app's HTTPS address and CA path (the web MCP)\n/pki issue <id> [user|agent] [root|normal]  issue client cert + one-time pickup URL\n/pki requests | approve <id> | deny <id>  certificate requests from other agents\n/channels  buses and channels\n/contacts  addressbook\n/schedules calendar (tasks and reminders)\n/jobs      in-flight MCP jobs (not calendar)\n/unanswered  human messages the agent has not closed yet\n/prompt    show the frozen system prompt and builtin tool list\n/history [n]  show the last n raw messages (envelopes with timestamps)\n/forget    start from scratch: forget the conversation, context, queue and saved state\n/reboot    reboot the machine (conversation and queue are saved and resumed)\n/poweroff  power off (same)\n/exit      quit (systemd restarts the agent; use /reboot to reboot the machine)",
            )),
            "tools" => {
                let visible: Vec<String> = self.tools.visible_specs().iter().map(|t| t.name.clone()).collect();
                for s in &self.tools.specs {
                    if s.name == "use_mcp" || s.name == "invoke" {
                        continue;
                    }
                    let vis = visible.contains(&s.name);
                    self.con.writeln(&format!("  {} {}{}", self.con.c(if vis { YELLOW } else { GRAY }, &format!("{:<28}", s.name)), first_line(&s.description), if vis { "" } else { "  (not loaded)" }));
                }
                let loaded = self.tools.loaded_names();
                for srv in &mut self.tools.servers {
                    let alive = srv.alive();
                    let is_loaded = loaded.contains(&srv.name);
                    self.con.writeln(&format!("  MCP {} v{} ({}) {}{}", srv.name, srv.version, srv.dir.display(), if alive { "running" } else { "EXITED" }, if is_loaded { ", functions loaded" } else { ", functions not loaded (discover path mcp/<name> enable=true)" }));
                }
            }
            "status" => {
                let m = crate::sys::meminfo();
                if let Some(i) = self.llm.as_ref().map(|l| &l.info) {
                    self.con.writeln(&format!(
                        "machine   {}\ntime      {}  tz {}  ntp {}\nmodel     {} ({}, {} params, {}){}\ncontext   {} / {} tokens in KV cache (trained max {})\nRAM       {} total, {} used (anonymous), {} page cache (mmap'd weights and files; reclaimable), {} available\nswap      {} total, {} free\nhistory   {} messages{}\nmemory    {} reflections, {} journal\nunanswered {} open ({} muted)\nconsoles  {}",
                        crate::hw::summary(),
                        crate::clock::rfc3339_now(),
                        crate::clock::snapshot().timezone,
                        if crate::clock::snapshot().ntp { "on" } else { "off" },
                        i.name, i.arch, human_count(i.n_params), human(i.size_bytes),
                        i.experts.as_ref().map(|e| format!(", {e}")).unwrap_or_default(),
                        self.llm.as_ref().map(|l| l.cached_tokens()).unwrap_or(0), self.llm.as_ref().map(|l| l.n_ctx).unwrap_or(0), i.n_ctx_train,
                        human(m.total), human(m.total.saturating_sub(m.available)), human(m.cached), human(m.available), human(m.swap_total), human(m.swap_free),
                        self.history.len(), if self.state_path.exists() { " (saved)" } else { "" },
                        self.memory.stats().1, self.memory.stats().0,
                        self.tools.unanswered.len(), self.tools.unanswered.muted_len(),
                        self.con.devices.join(", ")
                    ));
                } else {
                    let brain = self.remote.as_ref().map(|r| format!("FMM {}", r.label())).unwrap_or_else(|| "none (FMM configured, local GGUF not loaded)".into());
                    self.con.writeln(&format!(
                        "machine   {}\ntime      {}  tz {}  ntp {}\nmodel     {}\ncontext   {} tokens (FMM budget)\nRAM       {} total, {} available\nhistory   {} messages{}\nmemory    {} reflections, {} journal\nunanswered {} open ({} muted)\nconsoles  {}",
                        crate::hw::summary(),
                        crate::clock::rfc3339_now(),
                        crate::clock::snapshot().timezone,
                        if crate::clock::snapshot().ntp { "on" } else { "off" }, brain, self.ctx_budget(), human(m.total), human(m.available),
                        self.history.len(), if self.state_path.exists() { " (saved)" } else { "" },
                        self.memory.stats().1, self.memory.stats().0,
                        self.tools.unanswered.len(), self.tools.unanswered.muted_len(),
                        self.con.devices.join(", ")
                    ));
                }
            }
            "save" => self.save_state(true),
            "unanswered" => {
                let now = crate::clock::unix(crate::clock::now());
                self.con
                    .writeln(&self.tools.unanswered.format_list(now));
            }
            "reflect" => {
                self.tools.pending_reflect.set(true);
                if self.compact_once() {
                    self.con.writeln(&self.con.c(GRAY, "  reflection stored"));
                } else {
                    self.con.writeln(&self.con.c(GRAY, "  nothing to reflect"));
                }
            }
            "ctx" | "context" => {
                match it.next() {
                    None => {
                        let train = self.llm.as_ref().map(|l| format!(" (trained max {})", l.info.n_ctx_train)).unwrap_or_default();
                        self.con.writeln(&format!("context {} tokens{}", self.ctx_budget(), train));
                    }
                    Some(v) => match v.parse::<u32>() {
                        Ok(n) if (512..=2_000_000).contains(&n) => {
                            let line = self.apply_context(n);
                            self.con.writeln(&self.con.c(GRAY, &format!("  {line}")));
                        }
                        _ => self.con.writeln("usage: /ctx <tokens>   512..2000000"),
                    },
                }
            }
            "fmm" => {
                let sub = it.next().unwrap_or("");
                let line = match sub {
                    "on" => {
                        if let Some(mut c) = FmmConfig::load(&self.fmm_path) {
                            c.enabled = true;
                            let _ = std::fs::write(&self.fmm_path, serde_json::to_vec_pretty(&c).unwrap_or_default());
                        }
                        self.reload_fmm(true)
                    }
                    "off" => {
                        if let Some(mut c) = FmmConfig::load(&self.fmm_path) {
                            c.enabled = false;
                            let _ = std::fs::write(&self.fmm_path, serde_json::to_vec_pretty(&c).unwrap_or_default());
                        }
                        self.reload_fmm(false)
                    }
                    _ => match (&self.remote, FmmConfig::load(&self.fmm_path)) {
                        (Some(r), _) => format!("FMM: on, {} (/fmm off to use the local model)", r.label()),
                        (None, Some(c)) => format!("FMM: off; configured {} model {} key {} (/fmm on to enable)", c.provider, if c.model.is_empty() { "(default)".into() } else { c.model.clone() }, c.masked_key()),
                        (None, None) => "FMM: not configured. Paste an API key to the agent (OpenRouter, OpenAI, Anthropic, xAI, DeepSeek or Qwen); the provider is detected from it.".to_string(),
                    },
                };
                self.con.writeln(&line);
                if matches!(sub, "on" | "off") {
                    self.system_message(&format!("{line} Same conversation continues; replies stream."));
                }
            }
            "web" => self.print_web_info(),
            "pki" => {
                match it.next() {
                    Some("issue") => {
                        let identity = it.next().unwrap_or("root");
                        let role = it.next().unwrap_or("user");
                        let trust = it.next().unwrap_or(if identity == "root" {
                            "root"
                        } else {
                            "normal"
                        });
                        match self.pki().and_then(|p| {
                            p.issue_pickup(identity, role, trust).map_err(|e| format!("{e:#}"))
                        }) {
                            Ok((token, meta)) => {
                                let port = crate::weblink::read(&self.tools.files_dir)
                                    .map(|l| l.port)
                                    .filter(|p| *p > 0)
                                    .unwrap_or(443);
                                self.con.writeln(&self.con.c(
                                    GRAY,
                                    &format!("  issued {} for {identity} (role {role}, trust {trust}); one-time PKCS#12 (password 12345678):", meta.serial),
                                ));
                                let urls = crate::listen::pickup_urls(port, &token);
                                if urls.is_empty() {
                                    self.con.writeln(&self.con.c(GRAY, &format!("  /v1/pki/pickup/{token} (no network address yet)")));
                                }
                                for u in urls {
                                    self.con.writeln(&self.con.c(GRAY, &format!("  {u}")));
                                }
                            }
                            Err(e) => self.con.writeln(&self.con.c(RED, &format!("  pki: {e}"))),
                        }
                    }
                    Some("requests") => match self.pki() {
                        Ok(p) => {
                            let list = p.list_requests();
                            if list.is_empty() {
                                self.con.writeln(&self.con.c(GRAY, "  (no certificate requests)"));
                            }
                            for r in list {
                                let note = if r.note.is_empty() { String::new() } else { format!("  \"{}\"", r.note) };
                                self.con.writeln(&format!(
                                    "  {}  {:<8}  {} ({}) from {}{note}",
                                    r.short(), r.status.as_str(), r.identity, r.role, r.source
                                ));
                            }
                        }
                        Err(e) => self.con.writeln(&self.con.c(RED, &format!("  pki: {e}"))),
                    },
                    Some(verb @ ("approve" | "deny")) => {
                        let id = it.next().unwrap_or("");
                        let reason = it.collect::<Vec<_>>().join(" ");
                        match self.pki().and_then(|p| {
                            p.decide_request(id, verb == "approve", "root@console", &reason)
                                .map_err(|e| format!("{e:#}"))
                        }) {
                            Ok(r) => self.con.writeln(&self.con.c(
                                GRAY,
                                &format!("  certificate request {} for {}: {}", r.short(), r.identity, r.status.as_str()),
                            )),
                            Err(e) => self.con.writeln(&self.con.c(RED, &format!("  pki: {e}"))),
                        }
                    }
                    _ => self.con.writeln(
                        "usage: /pki issue <identity> [user|agent] [root|normal] | /pki requests | /pki approve <id> | /pki deny <id> [reason]",
                    ),
                }
            }
            "channels" | "buses" => {
                for c in self.tools.buses.list() {
                    self.con.writeln(&format!("  {}  ({})  {}", c.id, c.bus, c.title));
                }
            }
            "contacts" | "addressbook" => {
                let list = self.tools.addressbook.list();
                if list.is_empty() {
                    self.con.writeln(&self.con.c(GRAY, "  (addressbook empty)"));
                }
                for c in list {
                    self.con.writeln(&format!("  {}  {}  <{}>  {}  [{}]", c.id, c.name, c.identifier, c.trust, c.channels.join(",")));
                }
            }
            "schedules" | "schedule" | "calendar" => {
                let text = self.tools.schedules.lock().unwrap_or_else(|e| e.into_inner()).format_list();
                for line in text.lines() {
                    self.con.writeln(&format!("  {line}"));
                }
            }
            "jobs" | "job" => {
                let text = self.tools.jobs.lock().unwrap_or_else(|e| e.into_inner()).format_list();
                for line in text.lines() {
                    self.con.writeln(&format!("  {line}"));
                }
            }
            "history" => {
                let n: usize = it.next().and_then(|v| v.parse().ok()).unwrap_or(6);
                let start = self.history.len().saturating_sub(n);
                for m in &self.history[start..] {
                    let role = match m.role { Role::User => "user", Role::Assistant => "assistant", Role::Tool => "tool" };
                    let text: String = m.content.chars().take(300).collect();
                    self.con.writeln(&format!("{} {}{}", self.con.c(YELLOW, &format!("{role:<9}")), text.replace('\n', " "), if m.content.chars().count() > 300 { "..." } else { "" }));
                }
            }
            "prompt" => {
                let frozen = self.tools.prompt_specs();
                let sp = self.format.system_prompt(&self.system, &frozen);
                let shown: String = sp.chars().take(4000).collect();
                self.con.writeln(&self.con.c(GRAY, &format!("frozen system prompt ({} chars, {} builtin tools; loaded MCPs appended in history: {}):", sp.chars().count(), frozen.len(), self.tools.loaded_names().join(",") )));
                self.con.writeln(&shown);
                if sp.chars().count() > 4000 { self.con.writeln(&self.con.c(GRAY, "...")); }
            }
            "forget" | "clear" | "new" | "reset" => {
                self.forget_all();
                self.con.writeln("conversation forgotten: starting from scratch");
            }
            name @ ("update" | "rescue") => {
                // /update [force] [/dev/…|internet]   /rescue [/dev/…]
                let mut args: Vec<String> = vec![if name == "rescue" { "rescue" } else { "start" }.into()];
                for a in it.by_ref() {
                    match a {
                        "force" | "--force" if name == "update" => args.push("--force".into()),
                        "internet" | "net" | "github" if name == "update" => {
                            args.push("--from=internet".into())
                        }
                        dev if name == "update" => args.push(format!("--from={dev}")),
                        dev => args.push(dev.into()),
                    }
                }
                self.con.writeln(
                    "starting attended OS update (fold the agent, then reboot into the rescue image)…",
                );
                self.save_state(true);
                let refs: Vec<&str> = args.iter().map(String::as_str).collect();
                match run_bia_update(&refs, |line| {
                    self.con.writeln(&self.con.c(GRAY, line));
                }) {
                    Ok(()) => self.con.writeln("update staged (if the machine did not reboot, check the lines above)"),
                    Err(e) => self.con.writeln(&self.con.c(RED, &format!("{name}: {e}"))),
                }
            }
            "reboot" => {
                self.save_state(true);
                return Some(Exit::Reboot);
            }
            "poweroff" | "halt" | "shutdown" => {
                self.save_state(true);
                return Some(Exit::Poweroff);
            }
            "exit" | "quit" => return Some(Exit::Quit),
            other => self.con.writeln(&format!("unknown command /{other}; try /help")),
        }
        None
    }

    fn log(&self, role: &str, text: &str) {
        if let Some(p) = &self.session_log {
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(p)
            {
                let _ = writeln!(f, "### {role}\n{text}\n");
            }
        }
    }

    fn render_prompt(&self, add_generation_prompt: bool) -> String {
        self.format.render(
            &self.system,
            &self.tools.prompt_specs(),
            &self.history,
            add_generation_prompt,
            self.think,
        )
    }

    fn fit_history(&mut self) -> Vec<LlamaToken> {
        // append-only: never rewrite the prefix. Reflection (later) will compact when the context fills.
        let prompt = self.render_prompt(true);
        let llm = match self.llm.as_ref() {
            Some(l) => l,
            None => return vec![],
        };
        let toks = llm.tokenize(&prompt, llm.info.has_bos).unwrap_or_default();
        if let Some(p) = &self.session_log {
            if let Some(dir) = p.parent() {
                let _ = std::fs::write(dir.join("last-prompt.txt"), &prompt);
            }
        }
        toks
    }

    /// Execute tool calls, print them, append the results to the history.
    fn run_calls(&mut self, calls: &[ToolCall]) {
        self.tools.fired_job.set(false);
        for c in calls {
            let (raw_name, raw_args) = crate::tools::Tools::expand_call(&c.name, &c.args);
            let (name, args, note) = match self.tools.guide_call(&raw_name, &raw_args) {
                Ok(v) => v,
                Err(e) => {
                    let result = format!("error: {e}");
                    self.con.writeln(
                        &self
                            .con
                            .c(YELLOW, &format!("  -> {raw_name}({})", raw_args)),
                    );
                    self.con
                        .writeln(&self.con.c(RED, &format!("     ! {result}")));
                    self.log(&format!("tool {raw_name}"), &result);
                    self.push_history(Msg::tool(c.name.clone(), c.id.clone(), result));
                    continue;
                }
            };
            let note_body = |body: String| {
                if note.is_empty() {
                    body
                } else {
                    format!("{note}\n{body}")
                }
            };
            let is_say = name == "say";
            let args_s = args.to_string();
            self.set_state(if is_say { "talking" } else { "tool" });
            if !is_say {
                let shown = if c.name != name {
                    format!("{} → {name}({args_s})", c.name)
                } else {
                    format!("{name}({args_s})")
                };
                self.con.writeln(&self.con.c(
                    YELLOW,
                    &format!(
                        "  -> {}",
                        if shown.len() > 200 {
                            format!("{}...", trunc_bytes(&shown, 200))
                        } else {
                            shown
                        }
                    ),
                ));
                if !note.is_empty() {
                    self.con.writeln(&self.con.c(GRAY, &format!("     {note}")));
                }
            }
            if name == "bash" {
                match self.tools.fire_bash(&args, &c.id) {
                    Ok(jid) => {
                        let stub = note_body(format!("accepted job:{jid}; result will arrive as system|job:{jid}|issued|delivered||…"));
                        self.con.writeln(&self.con.c(GRAY, &format!("     {stub}")));
                        self.log(&format!("tool {name}"), &stub);
                        self.push_history(Msg::tool(c.name.clone(), c.id.clone(), stub));
                        self.tools.fired_job.set(true);
                    }
                    Err(e) => {
                        let result = note_body(format!("error: {e}"));
                        self.con
                            .writeln(&self.con.c(RED, &format!("     ! {result}")));
                        self.log(&format!("tool {name}"), &result);
                        self.push_history(Msg::tool(c.name.clone(), c.id.clone(), result));
                    }
                }
                continue;
            }
            if self.tools.is_mcp(&name) {
                let fmm = matches!(
                    name.as_str(),
                    "system_fmm_configure"
                        | "system_fmm_chatgpt"
                        | "system_fmm_enable"
                        | "system_fmm_disable"
                        | "system_fmm_forget"
                );
                let backup = if fmm {
                    std::fs::read(&self.fmm_path).ok()
                } else {
                    None
                };
                match self.tools.fire_mcp(&name, &args, &c.id) {
                    Ok(jid) => {
                        if let Some(b) = backup {
                            self.fmm_backups.insert(jid.clone(), b);
                        }
                        let stub = note_body(format!("accepted job:{jid}; result will arrive as system|job:{jid}|issued|delivered||…"));
                        self.con.writeln(&self.con.c(GRAY, &format!("     {stub}")));
                        self.log(&format!("tool {name}"), &stub);
                        self.push_history(Msg::tool(c.name.clone(), c.id.clone(), stub));
                        self.tools.fired_job.set(true);
                    }
                    Err(e) => {
                        let result = note_body(format!("error: {e}"));
                        self.con
                            .writeln(&self.con.c(RED, &format!("     ! {result}")));
                        self.log(&format!("tool {name}"), &result);
                        self.push_history(Msg::tool(c.name.clone(), c.id.clone(), result));
                    }
                }
                continue;
            }
            let con = self.con.clone();
            let (result, is_err) = self.tools.call(&name, &args, &mut |line| {
                con.writeln(&con.c(GRAY, &format!("     {line}")))
            });
            if !is_say || is_err {
                let shown_result = if name == "discover" && !is_err {
                    if result.chars().count() > 2000 {
                        let end = result
                            .char_indices()
                            .nth(2000)
                            .map(|(i, _)| i)
                            .unwrap_or(result.len());
                        format!("{}…", &result[..end])
                    } else {
                        result.clone()
                    }
                } else if name == "use_mcp" && !is_err {
                    result
                        .split(". Frozen tool list")
                        .next()
                        .unwrap_or(&result)
                        .to_string()
                } else if result.len() > 600 {
                    format!("{}...", trunc_bytes(&result, 600))
                } else {
                    result.clone()
                };
                for l in shown_result.lines() {
                    self.con.writeln(&self.con.c(
                        if is_err { RED } else { GRAY },
                        &format!("     {}{}", if is_err { "! " } else { "" }, l),
                    ));
                }
            }
            self.log(&format!("tool {name}"), &result);
            let result = note_body(result);
            let content = if result.len() > 16 * 1024 {
                format!("{}\n...[truncated]", trunc_bytes(&result, 16 * 1024))
            } else {
                result
            };
            self.push_history(Msg::tool(c.name.clone(), c.id.clone(), content));
            if name == "set_context" {
                if let Some(n) = self.tools.pending_ctx.take() {
                    let line = self.apply_context(n);
                    self.con.writeln(&self.con.c(GRAY, &format!("  {line}")));
                    self.system_message(&line);
                }
            }
        }
    }

    fn on_job_envelope(&mut self, ib: &Inbound) {
        let Some(id) = ib.channel.strip_prefix(crate::jobs::PREFIX) else {
            return;
        };
        // Mid-job notices (e.g. bash stale?) share the channel; only finished jobs run side effects.
        if ib.text.starts_with("stale?") {
            return;
        }
        let job = {
            let st = self.tools.jobs.lock().unwrap_or_else(|e| e.into_inner());
            st.get(id).cloned()
        };
        let Some(job) = job else { return };
        if job.state == "running" || job.state == "killing" {
            return;
        }
        let ok = job.state == "ok";
        if matches!(
            job.tool.as_str(),
            "system_fmm_configure"
                | "system_fmm_chatgpt"
                | "system_fmm_enable"
                | "system_fmm_disable"
                | "system_fmm_forget"
        ) {
            let want_on = matches!(
                job.tool.as_str(),
                "system_fmm_configure" | "system_fmm_chatgpt" | "system_fmm_enable"
            );
            let backup = self.fmm_backups.remove(id);
            if ok {
                let line = self.reload_fmm(true);
                let line = if want_on && self.remote.is_none() {
                    match backup {
                        Some(b) => {
                            let _ = std::fs::write(&self.fmm_path, &b);
                            unsafe { libc::sync() };
                            let restored = self.reload_fmm(false);
                            format!("{line} — restored previous FMM settings ({restored})")
                        }
                        None => {
                            let _ = std::fs::remove_file(&self.fmm_path);
                            unsafe { libc::sync() };
                            let _ = self.ensure_local();
                            format!("{line} — no previous FMM; local model")
                        }
                    }
                } else {
                    line
                };
                self.con.writeln(&self.con.c(GRAY, &format!("  {line}")));
                self.system_message(&format!(
                    "{line} Same conversation continues; replies stream."
                ));
            }
        }
    }

    /// One remote (FMM) generation: streams the text, appends the assistant message with its calls.
    fn remote_step(&mut self) -> anyhow::Result<()> {
        let remote = self.remote.as_ref().unwrap();
        self.set_state("remote");
        self.con.drain();
        let history = self
            .tools
            .fold_invoke_for_remote(normalize_history(self.format, &self.history));
        let tools = self.tools.prompt_specs();
        let con = self.con.clone();
        let mut shown = Printer::new(con.clone(), "\u{0}never", self.show_thinking, false);
        let queued: std::cell::RefCell<Vec<Inbound>> = std::cell::RefCell::new(vec![]);
        let t0 = std::time::Instant::now();
        let out_chars = std::cell::Cell::new(0usize);
        let prompt_est = self.fmm_ctx_used.max(self.history_token_estimate());
        let ctx_total = self.ctx_budget();
        let r = remote.generate(
            &self.system,
            &tools,
            &history,
            &mut |thought, piece| {
                if thought {
                    shown.push_thought(piece);
                    return keep_generating(&con, &queued, true);
                }
                out_chars.set(out_chars.get() + piece.chars().count());
                let secs = t0.elapsed().as_secs_f64().max(0.05);
                let out_est = out_chars.get().div_ceil(4);
                let tps = out_est as f64 / secs;
                con.update_status(|s| {
                    s.tps = tps as f32;
                    s.ctx_used = (prompt_est + out_est).min(ctx_total);
                    s.ctx_total = ctx_total;
                });
                shown.push(piece);
                keep_generating(&con, &queued, true)
            },
            false,
            std::time::Duration::from_secs(600),
        );
        shown.flush();
        self.stash_busy(queued.into_inner());
        let r = r?;
        self.con.writeln("");
        let secs = t0.elapsed().as_secs_f64();
        let out_est = out_chars.get().div_ceil(4) as u64;
        let input_tokens = if r.input_tokens > 0 {
            r.input_tokens
        } else {
            prompt_est as u64
        };
        let output_tokens = if r.output_tokens > 0 {
            r.output_tokens
        } else {
            out_est
        };
        let tps = if secs > 0.0 {
            output_tokens as f64 / secs
        } else {
            0.0
        };
        let used = (input_tokens + output_tokens) as usize;
        self.fmm_ctx_used = used;
        let msgs = self.history.len() + 1; // include the assistant reply about to be stored
        self.con.update_status(|s| {
            s.tps = tps as f32;
            s.ctx_used = used.min(ctx_total);
            s.ctx_total = ctx_total;
            s.messages = msgs;
        });
        let label = self.remote.as_ref().map(|r| r.label()).unwrap_or_default();
        self.con.writeln(&self.con.c(
            GRAY,
            &format!(
                "  [{label}: {} in / {} out tokens, ctx {}/{}, msgs {}, {:.1} tok/s, {:.1}s{}]",
                input_tokens,
                output_tokens,
                used.min(ctx_total),
                ctx_total,
                msgs,
                tps,
                secs,
                if r.stop == "interrupted" {
                    " | interrupted"
                } else {
                    ""
                }
            ),
        ));
        self.log("assistant", &r.text);
        for c in &r.calls {
            self.log("tool-call", &format!("{}({})", c.name, c.args));
        }
        let calls = if r.stop == "interrupted" {
            vec![]
        } else {
            r.calls
        };
        self.push_history(Msg::assistant(r.text, calls));
        Ok(())
    }

    /// Re-feed a restored conversation into the KV cache so the first answer is fast.
    pub fn warm(&mut self, mut on_progress: impl FnMut(usize, usize)) {
        if self.history.is_empty() {
            return;
        }
        if self.llm.is_none() {
            return;
        }
        let prompt = self.render_prompt(false);
        let llm = self.llm.as_mut().unwrap();
        let Ok(toks) = llm.tokenize(&prompt, llm.info.has_bos) else {
            return;
        };
        if toks.len() + 16 > llm.n_ctx {
            return; // reflection (later) will compact; do not rewrite history
        }
        let _ = llm.generate(
            &toks,
            0,
            |d, t| {
                on_progress(d, t);
                true
            },
            |_| true,
        );
        self.set_state("idle");
    }

    /// Run slash commands that arrived while the model was busy, before another turn or a reflection.
    fn run_pending_commands(&mut self) -> Option<Exit> {
        let batch = std::mem::take(&mut self.pending);
        let mut cmds = Vec::new();
        let mut rest = Vec::new();
        for ib in batch {
            if console_command(&ib).is_some() {
                cmds.push(ib);
            } else {
                rest.push(ib);
            }
        }
        if cmds.is_empty() {
            self.pending = rest;
            return None;
        }
        // Park the rest first so /forget can clear them.
        self.pending = rest;
        for ib in cmds {
            let cmd = console_command(&ib).unwrap_or("");
            self.con
                .writeln(&self.con.c(GRAY, &format!("  running /{cmd}")));
            if let Some(e) = self.command(cmd) {
                return Some(e);
            }
        }
        None
    }

    /// Park messages that arrived during a reply. Job results stay on `job_ready`
    /// so the same turn can hand them to the model; everything else waits until the
    /// next tool or job result is delivered (see `deliver_ready`).
    fn stash_busy(&mut self, queued: Vec<Inbound>) {
        let (jobs, rest): (Vec<_>, Vec<_>) = queued
            .into_iter()
            .partition(|ib| ib.channel.starts_with(crate::jobs::PREFIX));
        self.job_ready.extend(jobs);
        if !rest.is_empty() {
            self.pending.extend(rest);
            self.save_state(false);
        }
    }

    /// Pull any waiting inbound into `job_ready` / `pending`.
    fn collect_inbound(&mut self) {
        while let Some(ib) = self.con.try_read_inbound() {
            if local_channel(&ib.channel) && ib.text.trim().is_empty() {
                continue;
            }
            if ib.channel.starts_with(crate::jobs::PREFIX) {
                self.job_ready.push(ib);
            } else {
                self.con.writeln(&self.con.c(
                    GRAY,
                    &format!(
                        "  (queued: {}@{} {})",
                        ib.identity,
                        ib.channel,
                        ib.text.trim()
                    ),
                ));
                self.pending.push(ib);
            }
        }
    }

    /// Move queued non-command messages into the conversation. Slash commands stay pending.
    fn deliver_queued(&mut self) -> usize {
        let batch = std::mem::take(&mut self.pending);
        let (cmds, msgs): (Vec<_>, Vec<_>) = batch
            .into_iter()
            .partition(|ib| console_command(ib).is_some());
        self.pending = cmds;
        if msgs.is_empty() {
            return 0;
        }
        self.con.writeln(&self.con.c(
            GRAY,
            &format!("  delivering {} queued message(s)", msgs.len()),
        ));
        for ib in &msgs {
            self.record_inbound(ib);
        }
        self.save_state(false);
        msgs.len()
    }

    /// Move finished jobs (and optionally queued messages) into history. Returns how many.
    /// Queued messages ride with tool/job results so the model sees them in the same turn.
    fn deliver_ready(&mut self, with_queued: bool) -> usize {
        self.collect_inbound();
        // Not counted: a finished background send alone is no reason for another round.
        self.record_outbox_notes();
        let jobs = std::mem::take(&mut self.job_ready);
        let n_jobs = jobs.len();
        for ib in &jobs {
            self.record_inbound(ib);
        }
        let n_queued = if with_queued {
            self.deliver_queued()
        } else {
            if !self.pending.is_empty() {
                self.save_state(false);
            }
            0
        };
        let n_nudge = match self.unanswered_due() {
            Some(ib) => {
                self.record_inbound(&ib);
                1
            }
            None => 0,
        };
        n_jobs + n_queued + n_nudge
    }

    /// Run the unanswered check: send due fallbacks, and return the `unanswered:` notice
    /// for the model when any channel is due.
    fn unanswered_due(&mut self) -> Option<Inbound> {
        let now = crate::clock::unix(crate::clock::now());
        let due = self.tools.unanswered.check(now);
        for (identity, channel) in &due.fallback {
            let line = match self.tools.send_fallback(identity, channel) {
                Ok(r) => format!("unanswered: no reply after nudges; sent fallback ({r}); muted until a new message or job event"),
                Err(e) => format!("unanswered: fallback to {identity}@{channel} failed: {e:#}"),
            };
            self.con.writeln(&self.con.c(GRAY, &format!("  {line}")));
            self.log("system", &line);
        }
        if due.nudge.is_empty() {
            return None;
        }
        let jobs = self
            .tools
            .jobs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .running_brief();
        Some(Inbound {
            identity: ID_SYSTEM.into(),
            channel: CH_CONSOLE.into(),
            issued: crate::clock::now(),
            id: crate::console::mint_msg_id(),
            text: crate::unanswered::notice(&due, &jobs, now),
        })
    }

    fn record_inbound(&mut self, ib: &Inbound) {
        self.con.journal_done(&ib.id);
        if ib.channel.starts_with(crate::jobs::PREFIX) {
            self.on_job_envelope(ib);
        }
        self.log(
            "user",
            &format!("{}@{} {}", ib.identity, ib.channel, ib.text),
        );
        let bus = ib.channel.split(':').next().unwrap_or(ib.channel.as_str());
        self.tools.buses.upsert(&ib.channel, bus, "");
        if ib.identity != ID_SYSTEM
            && !ib.channel.starts_with(crate::jobs::PREFIX)
            && ib.channel != crate::tools::CH_SCHEDULE
        {
            self.tools.note_inbound(&ib.identity, &ib.channel);
        }
        // web/console name the person by identity; only linked channels are remembered
        if !crate::addressbook::identity_channel(&ib.channel)
            && ib.identity != ID_SYSTEM
            && ib.channel != crate::tools::CH_SCHEDULE
        {
            let _ =
                self.tools
                    .addressbook
                    .remember_channel(&ib.channel, &ib.identity, &ib.identity);
        }
        // [speech ru] … (calls, voice messages): learn the person's language
        if ib.identity != ID_SYSTEM {
            if let Some(code) = speech_lang(&ib.text) {
                if let Ok(Some(l)) =
                    self.tools
                        .addressbook
                        .observe_lang(&ib.identity, &ib.channel, &code)
                {
                    self.con.writeln(&self.con.c(
                        GRAY,
                        &format!("  addressbook: {}@{} speaks {l}", ib.identity, ib.channel),
                    ));
                }
            }
        }
        if ib.identity == ID_SYSTEM && ib.text.starts_with("unanswered:") {
            for line in ib.text.lines() {
                self.con.writeln(&self.con.c(GRAY, &format!("  {line}")));
            }
        } else if ib.identity == ID_SYSTEM {
            let preview: String = ib
                .text
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(120)
                .collect();
            self.con.writeln(&self.con.c(GRAY, &format!("  {preview}")));
        }
        let id = if ib.id.is_empty() {
            crate::console::mint_msg_id()
        } else {
            ib.id.clone()
        };
        if crate::tools::is_web(&ib.channel) {
            self.tools.web_read(&ib.identity, &ib.channel, &id);
        }
        let now = crate::clock::unix(crate::clock::now());
        if ib.channel.starts_with(crate::jobs::PREFIX) {
            self.tools.unanswered.wake(now);
        } else if needs_answer(ib) {
            self.tools
                .unanswered
                .add(&id, &ib.channel, &ib.identity, now);
        }
        let env = envelope(&ib.identity, &ib.channel, ib.issued, &id, &ib.text);
        self.push_history(Msg::user(env));
    }

    /// Background sends that finished since the last look ([sent o17 → …]), as one quiet
    /// `system|outbox` line. Failures come as normal inbound and wake the model instead.
    fn record_outbox_notes(&mut self) {
        let notes = self.tools.outbox.take_notes();
        if notes.is_empty() {
            return;
        }
        let text = notes.join("\n");
        let env = envelope(
            ID_SYSTEM,
            crate::outbox::CH_OUTBOX,
            crate::clock::now(),
            &crate::console::mint_msg_id(),
            &text,
        );
        self.log("system", &text);
        self.push_history(Msg::user(env));
    }

    /// Queue a `system|console|…` message for the model (answered at the next turn).
    pub fn system_message(&mut self, text: &str) {
        let env = envelope(
            ID_SYSTEM,
            CH_CONSOLE,
            crate::clock::now(),
            &crate::console::mint_msg_id(),
            text,
        );
        self.log("system", text);
        self.push_history(Msg::user(env));
    }

    fn turn(&mut self, messages: Vec<Inbound>) {
        let tg: Vec<String> = messages
            .iter()
            .filter(|ib| ib.channel.starts_with("telegram:"))
            .map(|ib| ib.channel.clone())
            .collect();
        let wa: Vec<String> = messages
            .iter()
            .filter(|ib| ib.channel.starts_with("whatsapp:"))
            .map(|ib| ib.channel.clone())
            .collect();
        if !tg.is_empty() {
            self.tools.set_telegram_typing(&tg, true);
        }
        if !wa.is_empty() {
            self.tools.set_whatsapp_typing(&wa, true);
        }
        let mut web: Vec<(String, String)> = messages
            .iter()
            .filter(|ib| crate::tools::is_web(&ib.channel) && ib.identity != ID_SYSTEM)
            .map(|ib| (ib.identity.clone(), ib.channel.clone()))
            .collect();
        web.dedup();
        self.tools.set_web_typing(&web, true);
        self.turn_inner(messages);
        self.tools.set_web_typing(&web, false);
        if !tg.is_empty() {
            self.tools.set_telegram_typing(&tg, false);
        }
        if !wa.is_empty() {
            self.tools.set_whatsapp_typing(&wa, false);
        }
    }

    fn turn_inner(&mut self, messages: Vec<Inbound>) {
        self.record_outbox_notes();
        for ib in &messages {
            self.record_inbound(ib);
        }
        // Queued traffic rides with whatever opened this turn (a job result, a console line, …).
        self.collect_inbound();
        let _ = self.deliver_queued();
        let (start_marker, _) = self.format.markers();
        let mut round = 0;
        loop {
            if self.pending.iter().any(|ib| console_command(ib).is_some()) {
                break;
            }
            self.ensure_room();
            if self.pending.iter().any(|ib| console_command(ib).is_some()) {
                break;
            }
            if self.remote.is_some() {
                match self.remote_step() {
                    Ok(()) => {}
                    Err(e) => {
                        self.con
                            .writeln(&self.con.c(RED, &format!("\n[FMM error: {e:#}]")));
                        self.save_state(false);
                        self.set_state("idle");
                        return;
                    }
                };
                let last = self.history.last().cloned();
                let calls: Vec<ToolCall> = last
                    .filter(|m| m.role == Role::Assistant)
                    .map(|m| m.tool_calls)
                    .unwrap_or_default();
                if calls.is_empty() {
                    if self.deliver_ready(true) > 0 {
                        continue;
                    }
                    break;
                }
                if !calls.iter().all(|c| c.name == "say") {
                    round += 1;
                    if round > self.max_tool_rounds {
                        self.con
                            .writeln(&self.con.c(RED, "  [tool round limit reached]"));
                        break;
                    }
                }
                self.run_calls(&calls);
                if self.tools.fired_job.get() {
                    // Wait outside for the job; keep queued messages so they arrive with it.
                    if self.deliver_ready(false) > 0 {
                        let _ = self.deliver_queued();
                        continue;
                    }
                    if self.tools.pending_exit.get().is_some() {
                        break;
                    }
                    break;
                }
                if self.deliver_ready(true) > 0 {
                    continue;
                }
                if self.tools.pending_exit.get().is_some() {
                    break;
                }
                continue;
            }
            if self.llm.is_none() {
                self.con.writeln(&self.con.c(RED, "[no local model loaded; FMM is configured. /fmm on to retry, /fmm off to load the GGUF]"));
                self.set_state("idle");
                return;
            }
            let prompt = self.fit_history();
            self.con.drain();
            self.set_state("thinking");
            let con = self.con.clone();
            let mut shown = Printer::new(con.clone(), start_marker, self.show_thinking, self.think);
            let con2 = self.con.clone();
            let prefill_shown = std::cell::Cell::new(false);
            let queued: std::cell::RefCell<Vec<Inbound>> = std::cell::RefCell::new(vec![]);
            let res = self.llm.as_mut().unwrap().generate(
                &prompt,
                4096,
                |done, total| {
                    if total > 256 {
                        con2.write(&format!(
                            "\r{}",
                            con2.c(GRAY, &format!("[reading {done}/{total} tokens]"))
                        ));
                        prefill_shown.set(true);
                    }
                    keep_generating(&con, &queued, true)
                },
                |piece| {
                    if prefill_shown.get() {
                        con.write("\r                                        \r");
                        prefill_shown.set(false);
                    }
                    shown.push(piece);
                    keep_generating(&con, &queued, true)
                },
            );
            shown.flush();
            self.stash_busy(queued.into_inner());
            let (text, stats) = match res {
                Ok(r) => r,
                Err(e) => {
                    self.con
                        .writeln(&self.con.c(RED, &format!("\n[error: {e}]")));
                    self.history.pop();
                    self.set_state("idle");
                    return;
                }
            };
            self.con.writeln("");
            let tps = if stats.gen_secs > 0.0 {
                stats.generated as f64 / stats.gen_secs
            } else {
                0.0
            };
            let pps = if stats.prefill_secs > 0.0 {
                (stats.prompt_tokens - stats.reused_tokens) as f64 / stats.prefill_secs
            } else {
                0.0
            };
            let ctx_used = self
                .llm
                .as_ref()
                .map(|l| l.cached_tokens())
                .unwrap_or(stats.prompt_tokens + stats.generated);
            let ctx_total = self
                .llm
                .as_ref()
                .map(|l| l.n_ctx)
                .unwrap_or(self.ctx_budget());
            let msgs = self.history.len() + 1;
            self.con.writeln(&self.con.c(
                GRAY,
                &format!(
                    "  [{} tokens, {:.1} tok/s | prompt {} (+{} new @ {:.0} tok/s) | ctx {}/{} | msgs {}{}]",
                    stats.generated,
                    tps,
                    stats.prompt_tokens,
                    stats.prompt_tokens - stats.reused_tokens,
                    pps,
                    ctx_used.min(ctx_total),
                    ctx_total,
                    msgs,
                    match stats.stop {
                        Stop::Interrupted => " | interrupted",
                        Stop::ContextFull => " | context full",
                        Stop::MaxTokens => " | max tokens",
                        Stop::Eos => "",
                    }
                ),
            ));
            self.con.update_status(|s| {
                s.tps = tps as f32;
                s.ctx_used = ctx_used.min(ctx_total);
                s.ctx_total = ctx_total;
                s.messages = msgs;
            });
            // keep the raw text (thinking included) so the next prompt extends the cached sequence exactly
            let mut calls = self.format.parse_calls(&text);
            if calls.is_empty() {
                let known: Vec<String> = self.tools.specs.iter().map(|t| t.name.clone()).collect();
                calls = self.format.parse_bare_calls(&text, &known);
                if !calls.is_empty() {
                    self.con
                        .writeln(&self.con.c(GRAY, "  (bare tool call recognized)"));
                }
            }
            for (k, c) in calls.iter_mut().enumerate() {
                c.id = format!("local_{}_{k}", self.history.len());
            }
            self.push_history(Msg::assistant(text.clone(), calls.clone()));
            self.log("assistant", &text);
            if stats.stop == Stop::Interrupted {
                break;
            }
            if calls.is_empty() {
                if self.deliver_ready(true) > 0 {
                    continue;
                }
                break;
            }
            // conversation (say) does not count against the tool budget
            if !calls.iter().all(|c| c.name == "say") {
                round += 1;
                if round > self.max_tool_rounds {
                    self.con
                        .writeln(&self.con.c(RED, "  [tool round limit reached]"));
                    break;
                }
            }
            self.run_calls(&calls);
            if self.tools.fired_job.get() {
                if self.deliver_ready(false) > 0 {
                    let _ = self.deliver_queued();
                    continue;
                }
                if self.tools.pending_exit.get().is_some() {
                    break;
                }
                break;
            }
            if self.deliver_ready(true) > 0 {
                continue;
            }
            if self.tools.pending_exit.get().is_some() {
                break;
            }
        }
        if !self.job_ready.is_empty() {
            let jobs = std::mem::take(&mut self.job_ready);
            self.pending.splice(0..0, jobs);
        }
        self.save_state(false);
        self.set_state("idle");
    }
}

/// One console line of a model reply. Headings, emphasis, code and links become ANSI.
/// Fenced blocks are dim and not interpreted. `in_fence` flips on a ``` line.
fn render_md_line(line: &str, in_fence: &mut bool, color: bool) -> String {
    let trimmed = line.trim_start();
    if trimmed.starts_with("```") {
        *in_fence = !*in_fence;
        return String::new();
    }
    if *in_fence {
        return if color {
            format!("{GRAY}{line}{CYAN}")
        } else {
            line.to_string()
        };
    }
    let (prefix, rest) = if let Some(text) = trimmed.strip_prefix("# ") {
        ("", text)
    } else if let Some(text) = trimmed.strip_prefix("## ") {
        ("", text)
    } else if let Some(text) = trimmed.strip_prefix("### ") {
        ("", text)
    } else {
        ("", line)
    };
    let heading =
        trimmed.starts_with("# ") || trimmed.starts_with("## ") || trimmed.starts_with("### ");
    let (lead, body) = if !heading {
        if let Some(text) = line.strip_prefix("- ") {
            ("- ", text)
        } else if let Some(text) = line.strip_prefix("* ") {
            ("- ", text)
        } else {
            ("", line)
        }
    } else {
        let indent = line.len() - trimmed.len();
        (&line[..indent], rest)
    };
    let _ = prefix;
    let inner = render_md_inline(body, color);
    if !color {
        return format!("{lead}{inner}");
    }
    if heading {
        format!("{lead}{BOLD}{inner}\x1b[22;36m")
    } else {
        format!("{lead}{inner}")
    }
}

fn render_md_inline(s: &str, color: bool) -> String {
    let b: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut out = String::new();
    while i < b.len() {
        if b[i] == '`' {
            if let Some(end) = b[i + 1..].iter().position(|&c| c == '`') {
                let code: String = b[i + 1..i + 1 + end].iter().collect();
                if color {
                    out.push_str(YELLOW);
                }
                out.push_str(&code);
                if color {
                    out.push_str(CYAN);
                }
                i += end + 2;
                continue;
            }
        }
        if b[i] == '*' && i + 1 < b.len() && b[i + 1] == '*' {
            if let Some(end) = find_pair(&b[i + 2..], '*') {
                let inner: String = b[i + 2..i + 2 + end].iter().collect();
                if color {
                    out.push_str(BOLD);
                }
                out.push_str(&render_md_inline(&inner, false));
                if color {
                    out.push_str("\x1b[22;36m");
                }
                i += end + 4;
                continue;
            }
        }
        if b[i] == '[' {
            if let Some((text, url, n)) = take_link(&b[i..]) {
                if color {
                    out.push_str("\x1b[4m");
                }
                out.push_str(&text);
                if color {
                    out.push_str("\x1b[24m");
                    out.push_str(GRAY);
                }
                out.push(' ');
                out.push_str(&url);
                if color {
                    out.push_str(CYAN);
                }
                i += n;
                continue;
            }
        }
        if (b[i] == '*' || b[i] == '_') && (i + 1 >= b.len() || b[i + 1] != b[i]) {
            let mark = b[i];
            if let Some(end) = b[i + 1..].iter().position(|&c| c == mark) {
                let inner: String = b[i + 1..i + 1 + end].iter().collect();
                if !inner.is_empty() && !inner.contains(mark) {
                    if color {
                        out.push_str("\x1b[3m");
                    }
                    out.push_str(&inner);
                    if color {
                        out.push_str("\x1b[23;36m");
                    }
                    i += end + 2;
                    continue;
                }
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

fn find_pair(s: &[char], mark: char) -> Option<usize> {
    let mut i = 0;
    while i + 1 < s.len() {
        if s[i] == mark && s[i + 1] == mark {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn take_link(s: &[char]) -> Option<(String, String, usize)> {
    if s.first() != Some(&'[') {
        return None;
    }
    let close = s.iter().position(|&c| c == ']')?;
    if close < 2 || s.get(close + 1) != Some(&'(') {
        return None;
    }
    let end = s[close + 2..].iter().position(|&c| c == ')')?;
    let text: String = s[1..close].iter().collect();
    let url: String = s[close + 2..close + 2 + end].iter().collect();
    if text.is_empty() || url.is_empty() {
        return None;
    }
    Some((text, url, close + 2 + end + 1))
}

/// Streams text to the console but hides tool-call markup (and any partial marker at the end)
/// and reasoning blocks.
struct Printer {
    con: Arc<Console>,
    marker: &'static str,
    buf: String,
    hidden: bool,
    printed_any: bool,
    thinking: bool,
    /// inside reasoning that a remote API streams apart from the reply text
    remote_thought: bool,
    think_chars: usize,
    show_thinking: bool,
    /// incomplete output line, rendered as markdown when it ends
    md_buf: String,
    in_fence: bool,
}

const THINK_END: &str = "</think>";

impl Printer {
    fn new(
        con: Arc<Console>,
        marker: &'static str,
        show_thinking: bool,
        start_thinking: bool,
    ) -> Printer {
        let p = Printer {
            con,
            marker,
            buf: String::new(),
            hidden: false,
            printed_any: false,
            thinking: start_thinking,
            remote_thought: false,
            think_chars: 0,
            show_thinking,
            md_buf: String::new(),
            in_fence: false,
        };
        if start_thinking {
            p.con.write(&p.con.c(
                GRAY,
                if show_thinking {
                    "\n~ "
                } else {
                    "\n  thinking"
                },
            ));
        }
        p
    }
    /// Reasoning text, streamed in gray on the console (never to web chats).
    fn emit_thought(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        self.think_chars += s.chars().count();
        if self.show_thinking {
            self.con.write(&self.con.c(GRAY, s));
        } else if self.think_chars % 200 < s.chars().count() {
            self.con.write(&self.con.c(GRAY, "."));
        }
    }
    fn push_thought(&mut self, piece: &str) {
        if !self.remote_thought {
            if piece.trim().is_empty() {
                return;
            }
            self.remote_thought = true;
            self.think_chars = 0;
            self.con.write(&self.con.c(
                GRAY,
                if self.show_thinking {
                    "\n~ "
                } else {
                    "\n  thinking"
                },
            ));
        }
        self.emit_thought(piece);
    }
    /// Close a remote reasoning block once the reply (or the end) arrives.
    fn end_remote_thought(&mut self) {
        if self.remote_thought {
            self.remote_thought = false;
            let n = self.think_chars;
            self.con
                .write(&self.con.c(GRAY, &format!("\n  (thought for {n} chars)\n")));
        }
    }
    fn push(&mut self, piece: &str) {
        if self.hidden {
            return;
        }
        if !piece.is_empty() {
            self.end_remote_thought();
        }
        self.buf.push_str(piece);
        loop {
            if self.thinking {
                if let Some(j) = self.buf.find(THINK_END) {
                    let thought = self.buf[..j].to_string();
                    self.emit_thought(&thought);
                    self.buf.drain(..j + THINK_END.len());
                    self.thinking = false;
                    let n = self.think_chars;
                    self.con
                        .write(&self.con.c(GRAY, &format!("\n  (thought for {n} chars)\n")));
                    continue;
                }
                // stream everything except a possible partial "</think>" at the end
                let keep = (1..THINK_END.len())
                    .rev()
                    .find(|&k| self.buf.ends_with(&THINK_END[..k]))
                    .unwrap_or(0);
                let cut = self.buf.len() - keep;
                if cut > 0 {
                    let t = self.buf[..cut].to_string();
                    self.emit_thought(&t);
                    self.buf.drain(..cut);
                }
                return;
            }
            let trimmed = self.buf.trim_start();
            if let Some(i) = trimmed.find("<think>") {
                let head = trimmed[..i].to_string();
                let off = self.buf.len() - trimmed.len() + i + 7;
                self.emit(&head);
                self.buf.drain(..off);
                self.thinking = true;
                self.think_chars = 0;
                self.con.write(&self.con.c(
                    GRAY,
                    if self.show_thinking {
                        "\n~ "
                    } else {
                        "\n  thinking"
                    },
                ));
                continue;
            }
            break;
        }
        if let Some(i) = self.buf.find(self.marker) {
            let head = self.buf[..i].to_string();
            self.emit(&head);
            self.hidden = true;
            self.buf.clear();
            return;
        }
        let keep = (1..self.marker.len())
            .rev()
            .find(|&k| self.buf.ends_with(&self.marker[..k]))
            .unwrap_or(0);
        let cut = self.buf.len() - keep;
        if cut > 0 {
            let head = self.buf[..cut].to_string();
            self.emit(&head);
            self.buf.drain(..cut);
        }
    }
    fn emit(&mut self, s: &str) {
        let s = s.replace("<|im_end|>", "");
        let s = if self.printed_any {
            s
        } else {
            s.trim_start().to_string()
        };
        if s.is_empty() {
            return;
        }
        if !self.printed_any {
            // cyan arrow, cyan text
            self.con.write(&format!(
                "\n{} {}",
                self.con.c(BCYAN, "<-"),
                if self.con.color { CYAN } else { "" }
            ));
            self.printed_any = true;
        }
        self.write_md(&s);
    }
    fn write_md(&mut self, s: &str) {
        self.md_buf.push_str(s);
        while let Some(i) = self.md_buf.find('\n') {
            let line: String = self.md_buf.drain(..=i).collect();
            let (text, nl) = line.split_at(line.len() - 1);
            let rendered = render_md_line(text, &mut self.in_fence, self.con.color);
            self.con.write(&rendered);
            self.con.write(nl);
        }
    }
    fn flush_md(&mut self) {
        if self.md_buf.is_empty() {
            return;
        }
        let line = std::mem::take(&mut self.md_buf);
        let rendered = render_md_line(&line, &mut self.in_fence, self.con.color);
        self.con.write(&rendered);
    }
    fn flush(&mut self) {
        self.end_remote_thought();
        if !self.hidden && !self.buf.is_empty() {
            let b = std::mem::take(&mut self.buf);
            self.emit(&b);
        }
        self.flush_md();
        if self.printed_any {
            self.con.write(RESET);
        }
    }
}

fn first_line(s: &str) -> String {
    let l = s.lines().next().unwrap_or("");
    if l.len() > 90 {
        format!("{}...", trunc_bytes(l, 90))
    } else {
        l.to_string()
    }
}

/// Truncate to at most `max` bytes without splitting a UTF-8 char (e.g. '—').
fn trunc_bytes(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        &s[..s.floor_char_boundary(max)]
    }
}

fn span_identities(span: &[Msg]) -> Vec<String> {
    let mut v = vec![];
    for m in span {
        if let Some(id) = m.content.split('|').next() {
            if !id.is_empty() && !v.iter().any(|x| x == id) {
                v.push(id.to_string());
            }
        }
    }
    v
}

fn span_tools(span: &[Msg]) -> Vec<String> {
    let mut v = vec![];
    for m in span {
        if m.role == Role::Tool && !m.name.is_empty() && !v.iter().any(|x| x == &m.name) {
            v.push(m.name.clone());
        }
        for c in &m.tool_calls {
            if !v.iter().any(|x| x == &c.name) {
                v.push(c.name.clone());
            }
        }
    }
    v
}

pub fn human_count(n: u64) -> String {
    if n >= 1_000_000_000 {
        format!("{:.1}B", n as f64 / 1e9)
    } else {
        format!("{:.0}M", n as f64 / 1e6)
    }
}

#[cfg(test)]
mod channel_tests {
    use super::*;

    fn ib(identity: &str, channel: &str, text: &str) -> Inbound {
        Inbound {
            identity: identity.into(),
            channel: channel.into(),
            issued: std::time::SystemTime::now(),
            id: "b1".into(),
            text: text.into(),
        }
    }

    #[test]
    fn only_the_physical_console_is_local() {
        assert!(local_channel("console"));
        for ch in ["web", "web:trip", "telegram:1", "whatsapp:420", "schedule"] {
            assert!(!local_channel(ch), "{ch}");
        }
    }

    #[test]
    fn slash_commands_only_on_the_console() {
        assert_eq!(
            console_command(&ib("root", "console", "/status")),
            Some("status")
        );
        // root on web is a person in a chat: "/status" is a message for the model
        assert_eq!(console_command(&ib("root", "web", "/status")), None);
        assert_eq!(console_command(&ib("root", "web:x", "/reboot")), None);
        assert_eq!(console_command(&ib("alex", "telegram:5", "/forget")), None);
    }

    #[test]
    fn web_messages_are_tracked_like_any_bus() {
        assert!(needs_answer(&ib("root", "web", "/status")));
        assert!(needs_answer(&ib("alex", "web:trip", "hi")));
        assert!(!needs_answer(&ib("root", "console", "hi")));
        assert!(!needs_answer(&ib("system", "web", "[call ended]")));
    }

    #[test]
    fn a_blank_web_message_does_not_stop_generation() {
        let con = Console::for_test();
        let queued = std::cell::RefCell::new(vec![]);
        con.push_inbound(ib("root", "web", ""));
        assert!(
            keep_generating(&con, &queued, true),
            "web is not the console's Enter key"
        );
        con.push_inbound(ib("root", "console", ""));
        assert!(
            !keep_generating(&con, &queued, true),
            "blank Enter on the console stops"
        );
        con.push_inbound(ib("root", "web", "/reboot"));
        assert!(
            keep_generating(&con, &queued, true),
            "a web slash line is just queued"
        );
        assert_eq!(queued.borrow().len(), 2);
    }
}
