//! bia deploy, the window app: first bia's brain (ChatGPT or an API key, checked by waking
//! bia), then where bia goes: a USB stick, or a server over SSH (`gui_server.rs`).
#![cfg(feature = "gui")]
// A window app on Windows: no console window behind it.
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use bia_deploy::chatgpt::{self, ChatGptAuth};
use bia_deploy::hf::{self, HfModel, HfRepo};
use bia_deploy::{
    default_model, filter_models, is_deepseek_or_qwen_key, list_models, list_removable, probe_key,
    parse_progress, provider_for_key, test_model, write_stick, KEY_GUIDES, KEY_STEPS, PROVIDERS, CloudModel, DiskInfo, FlashOptions, FmmInput,
    LocalFiles, LOCAL_FILES_NAME, LOCAL_FMM_CANDIDATES,
};
use eframe::egui;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};

mod gui_server;

/// bia's blue (the hello bubble, primary buttons).
const ACCENT: egui::Color32 = egui::Color32::from_rgb(38, 92, 160);
const GOOD: egui::Color32 = egui::Color32::from_rgb(40, 160, 70);
const DANGER: egui::Color32 = egui::Color32::from_rgb(190, 50, 45);


fn main() -> eframe::Result<()> {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([680.0, 840.0])
            // Content scrolls, so the window can shrink to fit any screen.
            .with_min_inner_size([420.0, 320.0])
            .with_title("bia deploy")
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon-256.png"))
                    .unwrap_or_default(),
            ),
        ..Default::default()
    };
    eframe::run_native(
        "bia deploy",
        opts,
        Box::new(|cc| {
            style(&cc.egui_ctx);
            Ok(Box::new(FlashApp::new()))
        }),
    )
}

enum NetMsg {
    Repos(Result<Vec<HfRepo>, String>),
    Models(String, Result<Vec<HfModel>, String>),
    /// Provider, base_url and models for an API key that only the APIs could identify.
    Probe(String, Result<(String, String, Vec<CloudModel>), String>),
    /// Result of "Continue with ChatGPT".
    Signed(Result<ChatGptAuth, String>),
    /// Models of a provider for a key.
    Cloud(String, Result<Vec<CloudModel>, String>),
    /// The latest bia release's OS manifest.
    Os(Result<bia_deploy::release::Manifest, String>),
    /// bia woke up on the model (its hello), for these settings; a renewed ChatGPT sign-in.
    Verified(String, Result<(bia_deploy::Greeting, Option<ChatGptAuth>), String>),
}

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Brain,
    Where,
    Stick,
    Server,
}

/// What the write is doing, shown under the write button.
enum Status {
    Idle,
    /// Current step; fraction and detail when the step reports a percentage.
    Working { step: String, fraction: Option<f32>, detail: String },
    Done,
    Failed(String),
}

struct FlashApp {
    page: Page,
    /// The model settings bia woke up on (`fmm_sig`); Continue needs the current ones.
    verified: Option<String>,
    verifying: bool,
    verify_error: Option<String>,
    srv: gui_server::Server,
    /// BIA_DEPLOY_SHOT: save the window as this PNG after a few frames, then quit.
    shot: Option<(PathBuf, u32)>,
    disks: Vec<DiskInfo>,
    selected: Option<usize>,
    all_disks: bool,
    /// The OS from the latest GitHub release (None until it loaded, or if it failed).
    os: Option<bia_deploy::release::Manifest>,
    os_note: String,
    /// Developers: write local image files instead of the release.
    local_os: bool,
    /// `local-files.json` from a local build (all image paths in one file).
    local_path: String,
    /// What `local_path` names, or why it could not be used.
    local: Result<LocalFiles, String>,
    arch: String,
    /// The owner's language (catalog code) and the choices.
    lang: String,
    languages: Vec<bia_deploy::lang::Language>,
    fmm_path: String,
    provider: String,
    api_key: String,
    /// Key the provider was last detected for (avoids probing on every frame).
    detected_key: String,
    base_url: String,
    model: String,
    cloud_models: Vec<CloudModel>,
    model_query: String,
    /// ChatGPT sign-in for this stick (provider `chatgpt`; `api_key` holds its access token).
    chatgpt: Option<ChatGptAuth>,
    signing_in: bool,
    /// Why the last ChatGPT sign-in failed, shown under the button.
    signin_error: Option<String>,
    /// What bia says in the bubble while writing: its hello, then the ready line.
    said: String,
    /// The flash sequence is waiting for the model's hello.
    waking: bool,
    ctx: String,
    use_fmm: bool,
    use_model: bool,
    hf_query: String,
    hf_token: String,
    hf_repos: Vec<HfRepo>,
    hf_repo: Option<usize>,
    hf_models: Vec<HfModel>,
    hf_model: Option<usize>,
    hf_busy: bool,
    hf_tx: Sender<NetMsg>,
    hf_rx: Receiver<NetMsg>,
    log: String,
    busy: bool,
    status: Status,
    tx: Sender<String>,
    rx: Receiver<String>,
}

impl FlashApp {
    fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let (hf_tx, hf_rx) = mpsc::channel();
        let default_fmm = FmmInput::find_local(None)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "fmm.json".into());
        let mut app = Self {
            page: Page::Brain,
            verified: None,
            verifying: false,
            verify_error: None,
            srv: gui_server::Server::new(),
            shot: std::env::var_os("BIA_DEPLOY_SHOT").map(|p| (PathBuf::from(p), 0)),
            disks: Vec::new(),
            selected: None,
            all_disks: false,
            os: None,
            os_note: "looking up the latest bia release…".into(),
            local_os: false,
            local_path: LocalFiles::find()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            local: Err(String::new()),
            arch: "x86_64".into(),
            lang: bia_deploy::lang::host_default(),
            languages: bia_deploy::lang::languages(),
            fmm_path: default_fmm,
            provider: "openrouter".into(),
            api_key: String::new(),
            detected_key: String::new(),
            base_url: String::new(),
            cloud_models: Vec::new(),
            model_query: String::new(),
            said: String::new(),
            waking: false,
            chatgpt: None,
            signing_in: false,
            signin_error: None,
            model: "x-ai/grok-4.6".into(),
            ctx: "131072".into(),
            use_fmm: true,
            use_model: false,
            hf_query: String::new(),
            hf_token: std::env::var("HF_TOKEN").unwrap_or_default(),
            hf_repos: Vec::new(),
            hf_repo: None,
            hf_models: Vec::new(),
            hf_model: None,
            hf_busy: false,
            hf_tx,
            hf_rx,
            log: String::new(),
            busy: false,
            status: Status::Idle,
            tx,
            rx,
        };
        app.refresh_disks();
        app.load_fmm_file();
        app.load_local_files();
        app.load_release();
        // BIA_DEPLOY_PAGE: open on another page (with BIA_DEPLOY_SHOT, for screenshots)
        match std::env::var("BIA_DEPLOY_PAGE").unwrap_or_default().as_str() {
            "where" => app.page = Page::Where,
            "stick" => app.page = Page::Stick,
            "server" => app.page = Page::Server,
            "server-key" => {
                app.page = Page::Server;
                app.srv.how = gui_server::How::NewKey;
            }
            "land" => {
                app.page = Page::Server;
                app.srv.land = true;
            }
            p @ ("hostkey" | "confirm" | "running" | "installed") => {
                app.page = Page::Server;
                app.srv.demo(p);
            }
            _ => {}
        }
        app
    }

    fn load_release(&mut self) {
        let tx = self.hf_tx.clone();
        std::thread::spawn(move || {
            let r = bia_deploy::release::manifest("latest").map_err(|e| format!("{e:#}"));
            let _ = tx.send(NetMsg::Os(r));
        });
    }

    /// Read `local_path`; in local mode its arch becomes the target arch.
    fn load_local_files(&mut self) {
        let path = self.local_path.trim();
        self.local = if path.is_empty() {
            Err(String::new())
        } else {
            LocalFiles::load(std::path::Path::new(path)).map_err(|e| format!("{e:#}"))
        };
        if let (true, Ok(l)) = (self.local_os, &self.local) {
            self.arch = l.arch.clone();
        }
    }

    /// "Choose…": native file dialog for local-files.json.
    fn pick_local_files(&mut self) {
        let mut dialog = rfd::FileDialog::new()
            .set_title("Choose local-files.json from a bia build")
            .add_filter("bia build", &["json"]);
        let current = PathBuf::from(self.local_path.trim());
        if let Some(dir) = current.parent().filter(|d| d.is_dir()) {
            dialog = dialog.set_directory(dir);
        } else if let Ok(cwd) = std::env::current_dir() {
            dialog = dialog.set_directory(cwd);
        }
        if let Some(p) = dialog.set_file_name(LOCAL_FILES_NAME).pick_file() {
            self.local_path = p.display().to_string();
            self.load_local_files();
        }
    }

    /// "Load…": native file dialog for fmm.json.
    fn pick_fmm_file(&mut self) {
        let mut dialog = rfd::FileDialog::new()
            .set_title("Choose fmm.json")
            .add_filter("fmm.json", &["json"]);
        let current = PathBuf::from(self.fmm_path.trim());
        if let Some(dir) = current.parent().filter(|d| d.is_dir()) {
            dialog = dialog.set_directory(dir);
        } else if let Ok(cwd) = std::env::current_dir() {
            dialog = dialog.set_directory(cwd);
        }
        if let Some(p) = dialog.set_file_name("fmm.json").pick_file() {
            self.fmm_path = p.display().to_string();
            self.load_fmm_file();
        }
    }

    fn load_fmm_file(&mut self) {
        let path = PathBuf::from(self.fmm_path.trim());
        if path.as_os_str().is_empty() {
            return;
        }
        match FmmInput::load(&path) {
            Ok(c) => {
                self.provider = c.provider;
                self.api_key = c.api_key;
                self.detected_key = self.api_key.clone();
                self.base_url = c.base_url;
                self.model = c.model;
                self.ctx = c.ctx.to_string();
                self.use_fmm = true;
                self.append_log(&format!("loaded FMM from {}", path.display()));
                self.fetch_models();
            }
            Err(e) => {
                self.append_log(&format!(
                    "no FMM file at {} ({e:#}); tried {}",
                    path.display(),
                    LOCAL_FMM_CANDIDATES.join(", ")
                ));
            }
        }
    }

    fn refresh_disks(&mut self) {
        match list_removable(self.all_disks) {
            Ok(d) => {
                self.disks = d;
                self.selected = if self.disks.is_empty() { None } else { Some(0) };
            }
            Err(e) => {
                self.log.push_str(&format!("list disks: {e:#}\n"));
                self.disks.clear();
                self.selected = None;
            }
        }
    }

    fn token(&self) -> Option<String> {
        Some(self.hf_token.trim().to_string()).filter(|t| !t.is_empty())
    }

    fn hf_search(&mut self) {
        let q = self.hf_query.trim().to_string();
        if q.is_empty() || self.hf_busy {
            return;
        }
        self.hf_busy = true;
        self.hf_repos.clear();
        self.hf_repo = None;
        self.hf_models.clear();
        self.hf_model = None;
        let (tx, token) = (self.hf_tx.clone(), self.token());
        std::thread::spawn(move || {
            let r = hf::search(&q, 30, token.as_deref()).map_err(|e| format!("{e:#}"));
            let _ = tx.send(NetMsg::Repos(r));
        });
    }

    fn hf_list(&mut self, idx: usize) {
        let repo = self.hf_repos[idx].id.clone();
        self.hf_busy = true;
        self.hf_models.clear();
        self.hf_model = None;
        let (tx, token) = (self.hf_tx.clone(), self.token());
        std::thread::spawn(move || {
            let r = hf::models(&repo, token.as_deref()).map_err(|e| format!("{e:#}"));
            let _ = tx.send(NetMsg::Models(repo, r));
        });
    }

    /// Switch provider, carrying the model along if it was the old provider's default.
    fn set_provider(&mut self, provider: &str, base_url: String) {
        let m = self.model.trim();
        if m.is_empty() || m == default_model(self.provider.trim()) {
            self.model = default_model(provider).into();
        }
        self.provider = provider.into();
        self.base_url = base_url;
    }

    fn fmm_input(&self) -> FmmInput {
        FmmInput {
            provider: self.provider.clone(),
            api_key: self.api_key.clone(),
            model: self.model.clone(),
            base_url: self.base_url.clone(),
            ctx: self.ctx.trim().parse().unwrap_or(131_072),
            enabled: Some(true),
            chatgpt: self.chatgpt.clone(),
        }
    }

    /// "Continue with ChatGPT": browser sign-in in the background.
    fn chatgpt_sign_in(&mut self) {
        if self.signing_in {
            return;
        }
        self.signing_in = true;
        self.signin_error = None;
        let tx = self.hf_tx.clone();
        let log = self.tx.clone();
        std::thread::spawn(move || {
            // A panic here would leave the window waiting forever: report it as a failure.
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                chatgpt::sign_in(None, &mut |s| {
                    eprintln!("chatgpt: {s}");
                    let _ = log.send(s.to_string());
                })
                .map_err(|e| format!("{e:#}"))
            }))
            .unwrap_or_else(|p| {
                let msg = p
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_default();
                Err(format!("ChatGPT sign-in crashed: {msg}"))
            });
            eprintln!("chatgpt: sign-in finished: {:?}", r.as_ref().map(|a| &a.email));
            let _ = tx.send(NetMsg::Signed(r));
        });
    }

    fn chatgpt_sign_out(&mut self) {
        self.chatgpt = None;
        self.api_key.clear();
        self.detected_key.clear();
        self.provider = "openrouter".into();
        self.model.clear();
        self.cloud_models.clear();
    }

    /// Load the provider's model list for the current key in the background.
    fn fetch_models(&mut self) {
        let key = self.api_key.trim().to_string();
        let provider = self.provider.trim().to_lowercase();
        if key.is_empty() || provider.is_empty() {
            return;
        }
        self.cloud_models.clear();
        self.hf_busy = true;
        let (tx, base) = (self.hf_tx.clone(), self.base_url.clone());
        std::thread::spawn(move || {
            let r = if provider == "chatgpt" {
                chatgpt::list_models(&key)
            } else {
                list_models(&provider, &key, &base)
            }
            .map_err(|e| format!("{e:#}"));
            let _ = tx.send(NetMsg::Cloud(key, r));
        });
    }

    /// Called when the key field changes: name the provider from the key's format, or
    /// ask DeepSeek / Qwen in the background when the format is shared.
    fn detect_provider(&mut self) {
        let key = self.api_key.trim().to_string();
        if key == self.detected_key {
            return;
        }
        self.detected_key = key.clone();
        if let Some(p) = provider_for_key(&key) {
            if p != self.provider {
                self.set_provider(p, String::new());
            }
            self.append_log(&format!("{p} key; loading its models …"));
            self.fetch_models();
        } else if is_deepseek_or_qwen_key(&key) {
            self.append_log("DeepSeek or Qwen key; asking which one accepts it …");
            self.hf_busy = true;
            let tx = self.hf_tx.clone();
            std::thread::spawn(move || {
                let r = probe_key(&key)
                    .map(|(p, b, m)| (p.to_string(), b, m))
                    .map_err(|e| format!("{e:#}"));
                let _ = tx.send(NetMsg::Probe(key, r));
            });
        }
    }

    fn append_log(&mut self, line: &str) {
        self.log.push_str(line);
        if !line.ends_with('\n') {
            self.log.push('\n');
        }
    }

    /// "The stick is ready" over the whole window: a dimmed backdrop that swallows clicks and a
    /// centered dialog, so the message cannot end up scrolled away or under the window edge.
    fn done_dialog(&mut self, ctx: &egui::Context) {
        let screen = ctx.screen_rect();
        egui::Area::new(egui::Id::new("done-backdrop"))
            .order(egui::Order::Middle)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                ui.painter()
                    .rect_filled(screen, 0.0, egui::Color32::from_black_alpha(160));
                ui.allocate_rect(screen, egui::Sense::click());
            });
        let mut close = false;
        egui::Window::new("stick ready")
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .max_width((screen.width() - 32.0).max(200.0))
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new("The stick is ready")
                            .size(24.0)
                            .strong()
                            .color(egui::Color32::from_rgb(40, 160, 70)),
                    );
                    ui.add_space(4.0);
                    ui.label("You can remove it and boot a computer from it.");
                    if !self.said.is_empty() {
                        ui.add_space(6.0);
                        ui.label(egui::RichText::new(format!("bia:  {}", self.said)).italics());
                    }
                    ui.add_space(10.0);
                    if ui.add_sized([160.0, 32.0], egui::Button::new("OK")).clicked() {
                        close = true;
                    }
                    ui.add_space(4.0);
                });
            });
        if close || ctx.input(|i| i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Escape)) {
            self.status = Status::Idle;
        }
    }

    /// A problem that stops the write: shown under the write button and in the log.
    fn fail(&mut self, msg: &str) {
        self.append_log(&format!("error: {msg}"));
        self.status = Status::Failed(msg.to_string());
    }

    fn start_write(&mut self) {
        if self.busy {
            return;
        }
        let Some(idx) = self.selected else {
            self.fail("select a disk");
            return;
        };
        let disk = self.disks[idx].path.clone();
        let size_bytes = self.disks[idx].size_bytes;
        // The OS: the GitHub release (downloaded in the worker), or local files.
        let release = if self.local_os {
            None
        } else {
            match &self.os {
                Some(m) if m.os.contains_key(&self.arch) => Some(m.clone()),
                Some(m) => {
                    self.fail(&format!("release {} has no {} system", m.tag, self.arch));
                    return;
                }
                None => {
                    self.fail(&format!("the bia system is not available yet: {}", self.os_note));
                    return;
                }
            }
        };
        let image = if release.is_some() {
            PathBuf::new()
        } else {
            // Re-read: the build may have run again since the file was chosen.
            self.load_local_files();
            match &self.local {
                Ok(l) => l.image.clone(),
                Err(e) if e.is_empty() => {
                    self.fail(&format!("choose the {LOCAL_FILES_NAME} of a local build"));
                    return;
                }
                Err(e) => {
                    let e = e.clone();
                    self.fail(&e);
                    return;
                }
            }
        };
        let fmm = self.use_fmm.then(|| self.fmm_input());
        if let Some(ref f) = fmm {
            if let Err(e) = f.validate() {
                self.fail(&format!("FMM: {e:#}"));
                return;
            }
        }
        let model = if self.use_model {
            match self.hf_model.and_then(|i| self.hf_models.get(i)) {
                Some(m) => Some(m.clone()),
                None => {
                    self.fail("pick a model file, or untick the local model");
                    return;
                }
            }
        } else {
            None
        };
        let arch = self.arch.clone();
        let opts = FlashOptions {
            disk: disk.clone(),
            image,
            image_sha256: None,
            size_bytes,
            fmm,
            create_image: false,
            model,
            hf_token: self.token(),
            lang: Some(self.lang.clone()),
        };
        self.busy = true;
        self.status = Status::Working {
            step: "starting".into(),
            fraction: None,
            detail: String::new(),
        };
        self.append_log(&format!("writing {} …", disk.display()));
        self.append_log("API keys stay on this computer and the stick (plaintext on ESP).");
        // First step of the sequence: wake bia on the chosen model. Its hello proves the
        // key, the model and tool calling work; nothing is written if it does not answer.
        self.said.clear();
        self.waking = opts.fmm.is_some();
        let tx = self.tx.clone();
        let mut opts = opts;
        std::thread::spawn(move || {
            let mut greeting = None;
            // A ChatGPT access token lasts an hour: renew it first and flash the renewed set.
            if let Some(f) = opts.fmm.as_mut() {
                if let Err(e) = f.refresh_chatgpt() {
                    let _ = tx.send(format!("error: {e:#}"));
                    let _ = tx.send("__DONE__".into());
                    return;
                }
            }
            let used_chatgpt = opts.fmm.as_ref().is_some_and(|f| f.chatgpt.is_some());
            if let Some(f) = opts.fmm.clone() {
                let _ = tx.send(format!("== waking bia on {} {} …", f.provider, f.model));
                match test_model(&f) {
                    Ok(g) => {
                        let _ = tx.send(format!("== bia woke up on {}", g.summary));
                        let _ = tx.send(format!("__SAY__{}", g.hello));
                        greeting = Some(g);
                    }
                    Err(e) => {
                        let _ = tx.send(format!(
                            "error: the model did not answer, nothing was written: {e:#}"
                        ));
                        let _ = tx.send("__DONE__".into());
                        return;
                    }
                }
            }
            if let Some(m) = release {
                let _ = tx.send(format!("== bia {} for {arch}", m.tag));
                let got = bia_deploy::release::fetch(&m, &arch, &mut |msg| {
                    let _ = tx.send(format!("== {msg}"));
                });
                match got {
                    Ok(f) => {
                        opts.image = f.path;
                        opts.image_sha256 = Some(f.raw_sha256);
                    }
                    Err(e) => {
                        let _ = tx.send(format!("error: could not download the bia system, nothing was written: {e:#}"));
                        let _ = tx.send("__DONE__".into());
                        return;
                    }
                }
            }
            let r = write_stick(
                opts,
                Box::new({
                    let tx = tx.clone();
                    move |msg| {
                        let _ = tx.send(format!("== {msg}"));
                    }
                }),
            );
            match r {
                Ok(()) => {
                    let _ = tx.send("finished OK".into());
                    if used_chatgpt {
                        let _ = tx.send("__CHATGPT_USED__".into());
                    }
                    if let Some(g) = greeting {
                        let _ = tx.send(format!("__SAY__{}", g.ready));
                    }
                }
                Err(e) => {
                    let _ = tx.send(format!("error: {e:#}"));
                }
            }
            let _ = tx.send("__DONE__".into());
        });
    }
    /// What `verified` compares: the settings that decide whether bia wakes up.
    fn fmm_sig(&self) -> String {
        format!("{}|{}|{}|{}|{}", self.provider.trim(), self.api_key.trim(), self.model.trim(), self.base_url.trim(), self.chatgpt.as_ref().map_or("", |a| a.email.as_str()))
    }

    fn brain_ok(&self) -> bool {
        self.verified.as_deref() == Some(self.fmm_sig().as_str())
    }

    /// "Wake bia up": the model must answer through bia's own tool before anything else.
    fn verify(&mut self) {
        let mut f = self.fmm_input();
        if let Err(e) = f.validate() {
            self.verify_error = Some(format!("{e:#}"));
            return;
        }
        self.verifying = true;
        self.waking = true;
        self.verify_error = None;
        self.said.clear();
        let (tx, sig) = (self.hf_tx.clone(), self.fmm_sig());
        std::thread::spawn(move || {
            // a ChatGPT access token lasts an hour; the renewed set replaces the window's
            let r = f
                .refresh_chatgpt()
                .and_then(|_| test_model(&f))
                .map(|g| (g, f.chatgpt.clone()))
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(NetMsg::Verified(sig, r));
        });
    }

    fn page_ui(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("bia").size(30.0).strong().color(ACCENT));
            ui.label(egui::RichText::new("deploy").size(30.0).weak());
        });
        let third = match self.page {
            Page::Stick => "USB stick",
            Page::Server => "Server",
            _ => "Install",
        };
        let at = match self.page {
            Page::Brain => 0,
            Page::Where => 1,
            _ => 2,
        };
        ui.horizontal(|ui| {
            for (i, name) in ["Brain", "Where", third].iter().enumerate() {
                let (fill, text) = if i == at {
                    (ACCENT, egui::Color32::WHITE)
                } else if i < at {
                    (ui.visuals().widgets.inactive.bg_fill, ui.visuals().text_color())
                } else {
                    (egui::Color32::TRANSPARENT, ui.visuals().weak_text_color())
                };
                egui::Frame::none().fill(fill).rounding(12.0).inner_margin(egui::Margin::symmetric(12.0, 4.0)).show(ui, |ui| {
                    ui.label(egui::RichText::new(format!("{}  {name}", i + 1)).color(text));
                });
                if i < 2 {
                    ui.label(egui::RichText::new("›").weak());
                }
            }
        });
        ui.add_space(10.0);
        match self.page {
            Page::Brain => self.brain_page(ui),
            Page::Where => self.where_page(ui),
            Page::Stick => {
                if ui.button("‹  Back").clicked() {
                    self.page = Page::Where;
                }
                card(ui, |ui| self.stick_page(ui));
            }
            Page::Server => {
                if self.srv.at_start() && ui.button("‹  Back").clicked() {
                    self.page = if self.srv.land && !self.brain_ok() { Page::Brain } else { Page::Where };
                }
                self.server_page(ui);
            }
        }
        ui.add_space(14.0);
        egui::CollapsingHeader::new(egui::RichText::new("Details").weak()).id_salt("details").show(ui, |ui| self.log_section(ui));
    }

    fn brain_page(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            ui.label(egui::RichText::new("First, bia's brain").size(21.0).strong());
            ui.label(egui::RichText::new("bia thinks with a cloud AI model. Use your ChatGPT plan, or an API key from a model provider.").weak());
            ui.add_space(6.0);
            self.fmm_section(ui);
        });
        ui.add_space(8.0);
        card(ui, |ui| {
            ui.label(egui::RichText::new("bia's language").size(19.0).strong());
            self.lang_section(ui);
        });
        ui.add_space(10.0);
        if self.use_fmm {
            ui.horizontal(|ui| {
                if self.brain_ok() {
                    if primary_button(ui, "Continue  ›", true).clicked() {
                        self.page = Page::Where;
                    }
                    ui.label(egui::RichText::new("✔ bia woke up").color(GOOD));
                } else if primary_button(ui, "Wake bia up", !self.verifying).clicked() {
                    self.verify();
                }
            });
            self.bubble(ui);
            if let Some(e) = &self.verify_error {
                ui.colored_label(ui.visuals().error_fg_color, format!("bia did not wake up: {e}"));
            }
        } else if primary_button(ui, "Continue without a cloud AI  ›", true).clicked() {
            self.page = Page::Stick;
        }
        ui.add_space(12.0);
        if ui
            .link("No key needed: I am preparing a server that one of my bias will fly its agent to (land)")
            .clicked()
        {
            self.srv.land = true;
            self.page = Page::Server;
        }
    }

    fn where_page(&mut self, ui: &mut egui::Ui) {
        if ui.button("‹  Back").clicked() {
            self.page = Page::Brain;
        }
        ui.label(egui::RichText::new("Where should bia live?").size(21.0).strong());
        ui.add_space(6.0);
        let full = ui.available_width();
        let wide = full > 560.0;
        // both cards get the same width, measured before the first one takes its share
        let w = if wide { (full - 12.0) / 2.0 } else { full };
        let mut go = None;
        let mut choice = |ui: &mut egui::Ui, icon: &str, title: &str, text: &str, page: Page| {
            let r = egui::Frame::none()
                .fill(ui.visuals().faint_bg_color)
                .stroke(egui::Stroke::new(1.0_f32, ui.visuals().widgets.noninteractive.bg_stroke.color))
                .rounding(16.0)
                .inner_margin(egui::Margin::same(18.0))
                .show(ui, |ui| {
                    let inner = (w - 38.0).max(100.0);
                    ui.set_width(inner);
                    ui.set_min_height(170.0);
                    ui.vertical(|ui| {
                        ui.set_max_width(inner);
                        ui.label(egui::RichText::new(icon).size(40.0));
                        ui.label(egui::RichText::new(title).size(19.0).strong());
                        ui.add(egui::Label::new(egui::RichText::new(text).weak()).wrap());
                    });
                })
                .response
                .interact(egui::Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if r.clicked() {
                go = Some(page);
            }
        };
        let stick = ("💾", "On a USB stick", "Write a stick, then start any PC or Mac from it. bia lives on the stick, or moves onto the computer's disk.");
        let server = ("☁", "On my server", "A server or VPS you rent from a provider. bia logs in to it and replaces its system; you reach bia from anywhere.");
        if wide {
            ui.horizontal_top(|ui| {
                choice(ui, stick.0, stick.1, stick.2, Page::Stick);
                choice(ui, server.0, server.1, server.2, Page::Server);
            });
        } else {
            choice(ui, stick.0, stick.1, stick.2, Page::Stick);
            ui.add_space(8.0);
            choice(ui, server.0, server.1, server.2, Page::Server);
        }
        if let Some(p) = go {
            if p == Page::Server {
                self.srv.land = false;
            }
            self.page = p;
        }
    }

    /// The USB stick: disk, computer type, system, optional local model, write.
    fn stick_page(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui
                .checkbox(&mut self.all_disks, "show all disks")
                .changed()
            {
                self.refresh_disks();
            }
            if ui.button("Refresh").clicked() {
                self.refresh_disks();
            }
        });

        egui::ComboBox::from_label("Target disk")
            .selected_text(match self.selected {
                Some(i) => format!(
                    "{} — {} ({} MiB)",
                    self.disks[i].path.display(),
                    self.disks[i].name,
                    self.disks[i].size_bytes / (1024 * 1024)
                ),
                None => "(none)".into(),
            })
            .show_ui(ui, |ui| {
                for (i, d) in self.disks.iter().enumerate() {
                    ui.selectable_value(
                        &mut self.selected,
                        Some(i),
                        format!(
                            "{} — {} ({} MiB)",
                            d.path.display(),
                            d.name,
                            d.size_bytes / (1024 * 1024)
                        ),
                    );
                }
            });

        ui.horizontal(|ui| {
            ui.label("The computer it will run on");
            let label = |a: &str| match a {
                "x86_64" => "PC (Intel or AMD, 64-bit)".to_string(),
                "arm64" => "ARM (arm64)".to_string(),
                other => other.to_string(),
            };
            let arches: Vec<String> = match (&self.os, self.local_os, &self.local) {
                (_, true, Ok(l)) => vec![l.arch.clone()],
                (Some(m), false, _) => m.os.keys().cloned().collect(),
                _ => vec!["x86_64".into(), "arm64".into()],
            };
            egui::ComboBox::from_id_salt("arch")
                .selected_text(label(&self.arch))
                .show_ui(ui, |ui| {
                    for a in &arches {
                        ui.selectable_value(&mut self.arch, a.clone(), label(a));
                    }
                });
        });
        if !self.local_os {
            let text = match &self.os {
                Some(m) => match m.os.get(&self.arch) {
                    Some(e) => {
                        let left = bia_deploy::release::missing_bytes(m, &self.arch);
                        format!(
                            "System: {} — {:.1} GiB; {}",
                            self.os_note,
                            e.image.raw_size as f64 / (1u64 << 30) as f64,
                            if left == 0 {
                                "already downloaded".to_string()
                            } else {
                                format!("{} MiB to download from GitHub", left >> 20)
                            }
                        )
                    }
                    None => format!("System: {} has no {} build", self.os_note, self.arch),
                },
                None => format!("System: {}", self.os_note),
            };
            ui.label(text);
        }
        egui::CollapsingHeader::new("Advanced: local image files (developers)")
            .default_open(false)
            .show(ui, |ui| {
                if ui
                    .checkbox(&mut self.local_os, "Write local files instead of the GitHub release")
                    .changed()
                {
                    self.load_local_files();
                }
                ui.add_enabled_ui(self.local_os, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(LOCAL_FILES_NAME);
                        if ui
                            .text_edit_singleline(&mut self.local_path)
                            .on_hover_text("Written by make build / make rootfs as dist/<arch>/local-files.json")
                            .lost_focus()
                        {
                            self.load_local_files();
                        }
                        if ui.button("Choose…").clicked() {
                            self.pick_local_files();
                        }
                    });
                    match &self.local {
                        Ok(l) => {
                            let size = bia_deploy::image::raw_size(&l.image).unwrap_or(0);
                            let mut text = format!(
                                "{} build: {} ({} MiB)",
                                l.arch,
                                l.image.display(),
                                size >> 20,
                            );
                            if !l.built.is_empty() {
                                text += &format!("\nbuilt {}", l.built);
                            }
                            ui.label(egui::RichText::new(text).small());
                        }
                        Err(e) if e.is_empty() => {
                            ui.label(
                                egui::RichText::new("Run make build (or make rootfs), then choose dist/<arch>/local-files.json.")
                                    .small()
                                    .weak(),
                            );
                        }
                        Err(e) => {
                            ui.colored_label(ui.visuals().error_fg_color, e);
                        }
                    }
                });
            });

        ui.separator();
        ui.checkbox(&mut self.use_model, "Local model from Hugging Face (GGUF)");
        if self.use_model {
            ui.horizontal(|ui| {
                ui.label("search");
                let r = ui.text_edit_singleline(&mut self.hf_query);
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ui.add_enabled(!self.hf_busy, egui::Button::new("Search")).clicked() || enter {
                    self.hf_search();
                }
                if self.hf_busy {
                    ui.spinner();
                }
            });
            ui.horizontal(|ui| {
                ui.label("HF token");
                ui.add(egui::TextEdit::singleline(&mut self.hf_token).password(true))
                    .on_hover_text("Only for gated or private repos. Not written to the stick.");
            });
            let mut pick_repo = None;
            egui::ComboBox::from_label("Repo")
                .width(420.0)
                .selected_text(
                    self.hf_repo
                        .map(|i| self.hf_repos[i].id.clone())
                        .unwrap_or_else(|| "(search first)".into()),
                )
                .show_ui(ui, |ui| {
                    for (i, r) in self.hf_repos.iter().enumerate() {
                        let text = format!("{}  ({} downloads)", r.id, r.downloads);
                        if ui.selectable_label(self.hf_repo == Some(i), text).clicked() {
                            pick_repo = Some(i);
                        }
                    }
                });
            if let Some(i) = pick_repo {
                if self.hf_repo != Some(i) {
                    self.hf_repo = Some(i);
                    self.hf_list(i);
                }
            }
            egui::ComboBox::from_label("File")
                .width(420.0)
                .selected_text(
                    self.hf_model
                        .map(|i| {
                            let m = &self.hf_models[i];
                            format!("{}  ({} MiB)", m.label(), m.size() >> 20)
                        })
                        .unwrap_or_else(|| "(pick a repo)".into()),
                )
                .show_ui(ui, |ui| {
                    for (i, m) in self.hf_models.iter().enumerate() {
                        let text = format!("{}  ({} MiB)", m.label(), m.size() >> 20);
                        ui.selectable_value(&mut self.hf_model, Some(i), text);
                    }
                });
        }
        if let Some(d) = self.selected.and_then(|i| self.disks.get(i)) {
            let root = match (&self.os, self.local_os) {
                (Some(m), false) => m.os.get(&self.arch).map_or(0, |e| e.image.raw_size),
                _ => self
                    .local
                    .as_ref()
                    .ok()
                    .and_then(|l| bia_deploy::image::raw_size(&l.image).ok())
                    .unwrap_or(0),
            };
            let model = if self.use_model {
                self.hf_model.and_then(|i| self.hf_models.get(i)).map_or(0, |m| m.size())
            } else {
                0
            };
            let gib = |b: u64| b as f64 / (1u64 << 30) as f64;
            let mut text = format!(
                "The system takes the whole {:.1} GiB stick (root grows on first boot).",
                gib(d.size_bytes)
            );
            if model > 0 {
                let need = root + model + bia_deploy::MODEL_HEADROOM + (300 << 20);
                if need > d.size_bytes {
                    text = format!(
                        "Too small: needs {:.1} GiB for system + model, stick has {:.1} GiB.",
                        gib(need),
                        gib(d.size_bytes)
                    );
                } else {
                    text += &format!(" Model {:.1} GiB, {:.1} GiB left free.", gib(model), gib(d.size_bytes - need));
                }
            }
            ui.label(text);
        }

        ui.separator();
        ui.add_enabled_ui(!self.busy, |ui| {
            if ui
                .add_sized([ui.available_width(), 36.0], egui::Button::new("Erase disk & write bia"))
                .clicked()
            {
                self.start_write();
            }
        });
        match &self.status {
            Status::Idle => {}
            // The "waking bia" line above already says what is happening.
            Status::Working { .. } if self.waking => {}
            Status::Working { step, fraction, detail } => {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(egui::RichText::new(step).strong());
                });
                if let Some(f) = fraction {
                    let text = format!("{:.0}%  {detail}", f * 100.0);
                    ui.add(egui::ProgressBar::new(*f).text(text));
                }
            }
            // Shown as a dialog over the whole window (below).
            Status::Done => {}
            Status::Failed(e) => {
                ui.add_space(4.0);
                let red = ui.visuals().error_fg_color;
                egui::Frame::none()
                    .stroke(egui::Stroke::new(1.0_f32, red))
                    .rounding(6.0)
                    .inner_margin(egui::Margin::same(10.0))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.colored_label(red, egui::RichText::new("Could not write the stick").strong());
                        ui.label(e);
                    });
            }
        }

    }

    /// The cloud model: ChatGPT sign-in or an API key, and the model.
    fn fmm_section(&mut self, ui: &mut egui::Ui) {
        ui.checkbox(&mut self.use_fmm, "Use a cloud AI (needs an API key)");
        if self.use_fmm {
            // Sign in with ChatGPT first (no key needed), or paste an API key below.
            ui.label(egui::RichText::new("Use your ChatGPT plan").strong());
            match &self.chatgpt {
                Some(a) => {
                    let email = a.email.clone();
                    ui.horizontal(|ui| {
                        ui.label(format!("Signed in to ChatGPT as {email}."));
                        if ui.button("Sign out").clicked() {
                            self.chatgpt_sign_out();
                        }
                    });
                }
                None => {
                    ui.horizontal(|ui| {
                        let button = egui::Button::new(
                            egui::RichText::new("Continue with ChatGPT").color(egui::Color32::WHITE),
                        )
                        .fill(egui::Color32::from_rgb(16, 16, 16))
                        .rounding(18.0)
                        .min_size(egui::vec2(220.0, 34.0));
                        if ui.add_enabled(!self.signing_in, button).clicked() {
                            self.chatgpt_sign_in();
                        }
                        if self.signing_in {
                            ui.spinner();
                            ui.label("finish signing in in your browser …");
                        }
                    });
                    if let Some(e) = &self.signin_error {
                        ui.colored_label(ui.visuals().error_fg_color, e);
                    }
                }
            }
            ui.label(
                egui::RichText::new(
                    "bia itself is free. With ChatGPT, bia's requests count toward your ChatGPT plan's limits; nothing is charged by bia.",
                )
                .small()
                .weak(),
            );
        }
        if self.use_fmm && self.chatgpt.is_none() {
            ui.add_space(4.0);
            ui.label(egui::RichText::new("Or use an API key").strong());
            key_help(ui, self.api_key.trim().is_empty());
            egui::Grid::new("key").num_columns(2).spacing([12.0, 10.0]).show(ui, |ui| {
                ui.label("API key");
                if ui
                    .add(egui::TextEdit::singleline(&mut self.api_key).password(true).hint_text("paste the key here").desired_width(360.0))
                    .changed()
                {
                    self.detect_provider();
                }
                ui.end_row();
                if !self.api_key.trim().is_empty() && !self.provider.trim().is_empty() {
                    ui.label("Provider");
                    ui.label(egui::RichText::new(&self.provider).strong());
                    ui.end_row();
                }
            });
        }
        if self.use_fmm {
            egui::Grid::new("model").num_columns(2).spacing([12.0, 10.0]).show(ui, |ui| {
                ui.label("Model");
                ui.add(egui::TextEdit::singleline(&mut self.model).desired_width(360.0));
                ui.end_row();
                if !self.cloud_models.is_empty() {
                    ui.label("Search");
                    ui.add(egui::TextEdit::singleline(&mut self.model_query).hint_text("e.g. claude, gpt, grok").desired_width(360.0));
                    ui.end_row();
                }
            });
            if !self.cloud_models.is_empty() {
                let hits = filter_models(&self.cloud_models, &self.model_query);
                let mut pick = None;
                egui::ScrollArea::vertical()
                    .id_salt("cloud-models")
                    .max_height(140.0)
                    .show(ui, |ui| {
                        for m in hits.iter().take(300) {
                            let mut text = m.id.clone();
                            if m.name != m.id {
                                text += &format!("  — {}", m.name);
                            }
                            if m.ctx > 0 {
                                text += &format!("  ({}k ctx)", m.ctx / 1000);
                            }
                            if ui.selectable_label(self.model == m.id, text).clicked() {
                                pick = Some((m.id.clone(), m.ctx));
                            }
                        }
                        if hits.len() > 300 {
                            ui.label(format!("… {} more; refine the search", hits.len() - 300));
                        }
                    });
                if let Some((id, ctx)) = pick {
                    self.model = id;
                    if ctx > 0 {
                        self.ctx = ctx.min(2_000_000).to_string();
                    }
                }
            }
            egui::CollapsingHeader::new(egui::RichText::new("Advanced").weak()).id_salt("fmm-advanced").show(ui, |ui| {
                egui::Grid::new("fmm-adv").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                    ui.label("fmm.json");
                    ui.horizontal(|ui| {
                        let r = ui.add(egui::TextEdit::singleline(&mut self.fmm_path).desired_width(260.0));
                        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            self.load_fmm_file();
                        }
                        if ui.button("Load…").clicked() {
                            self.pick_fmm_file();
                        }
                    });
                    ui.end_row();
                    if self.chatgpt.is_none() {
                        ui.label("Provider");
                        let r = ui
                            .add(egui::TextEdit::singleline(&mut self.provider).desired_width(160.0))
                            .on_hover_text("Filled in from the key: openrouter, openai, anthropic, xai, deepseek or qwen");
                        if r.changed() && PROVIDERS.contains(&self.provider.trim()) {
                            self.fetch_models();
                        }
                        ui.end_row();
                    }
                    ui.label("Context");
                    ui.add(egui::TextEdit::singleline(&mut self.ctx).desired_width(100.0)).on_hover_text("How many tokens the model reads at once");
                    ui.end_row();
                });
            });
        }
    }

    /// The owner's language.
    fn lang_section(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Language");
            let name = |l: &bia_deploy::lang::Language| {
                if l.voice { l.name.clone() } else { format!("{} (text only)", l.name) }
            };
            let current = self.languages.iter().find(|l| l.code == self.lang).map(name).unwrap_or_else(|| self.lang.clone());
            egui::ComboBox::from_id_salt("lang")
                .selected_text(current)
                .height(400.0)
                .show_ui(ui, |ui| {
                    for l in &self.languages {
                        ui.selectable_value(&mut self.lang, l.code.clone(), name(l));
                    }
                });
        });
        ui.label(
            egui::RichText::new(
                "bia talks in it from the first boot, with its keyboard; its voice downloads on the first boot. Text only: no voice exists, so bia speaks English aloud.",
            )
            .small()
            .weak(),
        );
    }

    /// bia talks: its hello once the model answered, its ready line after writing.
    fn bubble(&mut self, ui: &mut egui::Ui) {
        if self.waking {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(format!("waking bia on {} …", self.model.trim()));
            });
        } else if !self.said.is_empty() {
            ui.add_space(6.0);
            egui::Frame::none()
                .fill(egui::Color32::from_rgb(38, 92, 160))
                .rounding(14.0)
                .inner_margin(egui::Margin::symmetric(14.0, 10.0))
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(format!("bia:  {}", self.said))
                            .size(17.0)
                            .color(egui::Color32::WHITE),
                    );
                });
        }

    }

    /// What happened, for when something goes wrong.
    fn log_section(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .id_salt("log")
            .max_height(180.0)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                ui.monospace(&self.log);
            });
    }

}

impl eframe::App for FlashApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(msg) = self.hf_rx.try_recv() {
            self.hf_busy = false;
            match msg {
                NetMsg::Repos(Ok(r)) => {
                    if r.is_empty() {
                        self.append_log("no GGUF repos found");
                    }
                    self.hf_repos = r;
                }
                NetMsg::Models(repo, Ok(m)) => {
                    if m.is_empty() {
                        self.append_log(&format!("{repo} has no GGUF files"));
                    }
                    self.hf_model = m
                        .iter()
                        .position(|m| m.files[0].path.to_uppercase().contains("Q4_K_M"));
                    self.hf_models = m;
                }
                NetMsg::Probe(key, Ok((p, base, models))) if key == self.api_key.trim() => {
                    self.set_provider(&p, base);
                    self.append_log(&format!("key accepted by {p}: {} models", models.len()));
                    self.cloud_models = models;
                }
                NetMsg::Cloud(key, Ok(models)) if key == self.api_key.trim() => {
                    self.append_log(&format!("{}: {} models", self.provider, models.len()));
                    if self.provider == "chatgpt" && self.model.trim().is_empty() {
                        if let Some(d) = chatgpt::default_model(&models) {
                            self.model = d.id.clone();
                            if d.ctx > 0 {
                                self.ctx = d.ctx.min(2_000_000).to_string();
                            }
                        }
                    }
                    self.cloud_models = models;
                }
                NetMsg::Cloud(key, Err(e)) if key == self.api_key.trim() => {
                    self.append_log(&e);
                }
                NetMsg::Cloud(..) => {}
                NetMsg::Os(r) => match r {
                    Ok(m) => {
                        self.os_note = format!("bia {} ({})", m.version, m.tag);
                        if !m.os.contains_key(&self.arch) {
                            if let Some(a) = m.os.keys().next() {
                                self.arch = a.clone();
                            }
                        }
                        self.os = Some(m);
                    }
                    Err(e) => {
                        self.os_note = format!("could not reach GitHub: {e}");
                        self.append_log(&self.os_note.clone());
                    }
                },
                NetMsg::Signed(r) => {
                    eprintln!("chatgpt: window got the sign-in result (ok={})", r.is_ok());
                    self.signing_in = false;
                    match r {
                        Ok(auth) => {
                            self.append_log(&format!("signed in to ChatGPT as {}", auth.email));
                            self.provider = "chatgpt".into();
                            self.api_key = auth.access_token.clone();
                            self.detected_key = self.api_key.clone();
                            self.base_url.clear();
                            self.model.clear();
                            self.chatgpt = Some(auth);
                            self.fetch_models();
                        }
                        Err(e) => {
                            self.append_log(&e);
                            self.signin_error = Some(e);
                        }
                    }
                }
                NetMsg::Verified(sig, r) => {
                    self.verifying = false;
                    self.waking = false;
                    match r {
                        Ok((g, auth)) => {
                            if auth.is_some() {
                                self.chatgpt = auth;
                            }
                            self.append_log(&format!("== bia woke up on {}", g.summary));
                            self.said = g.hello;
                            self.verify_error = None;
                            self.verified = Some(sig);
                        }
                        Err(e) => {
                            self.append_log(&format!("error: {e}"));
                            self.said.clear();
                            self.verify_error = Some(e);
                        }
                    }
                }
                NetMsg::Probe(key, Err(e)) if key == self.api_key.trim() => {
                    self.append_log(&e);
                }
                NetMsg::Probe(..) => {}
                NetMsg::Repos(Err(e)) | NetMsg::Models(_, Err(e)) => {
                    self.append_log(&format!("Hugging Face: {e}"));
                }
            }
        }
        while let Ok(msg) = self.rx.try_recv() {
            if msg == "__DONE__" {
                self.busy = false;
                self.waking = false;
                if matches!(self.status, Status::Working { .. }) {
                    self.status = Status::Idle;
                }
            } else if let Some(e) = msg.strip_prefix("error: ") {
                self.status = Status::Failed(e.to_string());
                self.append_log(&msg);
            } else if msg == "finished OK" {
                self.status = Status::Done;
                self.append_log(&msg);
            } else if let Some(line) = msg.strip_prefix("== ") {
                match parse_progress(line) {
                    Some((step, f, detail)) => {
                        // The bar shows the percentages; the log keeps only finished steps.
                        if f >= 1.0 {
                            self.append_log(&msg);
                        }
                        self.status = Status::Working {
                            step: step.to_string(),
                            fraction: Some(f),
                            detail: detail.to_string(),
                        };
                    }
                    None => {
                        self.append_log(&msg);
                        self.status = Status::Working {
                            step: line.to_string(),
                            fraction: None,
                            detail: String::new(),
                        };
                    }
                }
            } else if msg == "__CHATGPT_USED__" {
                // The stick rotates these tokens from now on; another stick needs its own sign-in.
                self.chatgpt_sign_out();
                self.append_log("The ChatGPT sign-in now belongs to this stick. Press Continue with ChatGPT again for another stick.");
            } else if let Some(line) = msg.strip_prefix("__SAY__") {
                self.waking = false;
                self.said = line.to_string();
                self.append_log(&format!("bia: {line}"));
            } else {
                self.append_log(&msg);
            }
        }

        self.server_poll();
        if let Some((path, frames)) = self.shot.as_mut() {
            *frames += 1;
            if *frames == 8 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
            }
            let path = path.clone();
            let shot = ctx.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
            });
            if let Some(img) = shot {
                let [w, h] = img.size;
                let px: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
                if let Err(e) = image::save_buffer(&path, &px, w as u32, h as u32, image::ExtendedColorType::Rgba8) {
                    eprintln!("screenshot: {e}");
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ctx.request_repaint();
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("page")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    self.page_ui(ui);
                });
        });

        if matches!(self.status, Status::Done) {
            self.done_dialog(ctx);
        }

        if self.busy || self.hf_busy {
            ctx.request_repaint();
        } else if self.verifying || self.srv.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        } else if self.signing_in {
            // The sign-in result arrives from the browser while this window has no input
            // events; poll so it shows (and the API-key fields hide) without a mouse move.
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
    }
}

/// "How do I get a key?": the three steps and a link per provider. Open while no key is
/// entered; collapsible after.
fn key_help(ui: &mut egui::Ui, open: bool) {
    egui::CollapsingHeader::new("How do I get an API key?")
        .id_salt("key-help")
        .default_open(open)
        .show(ui, |ui| {
            for (i, step) in KEY_STEPS.iter().enumerate() {
                ui.label(format!("{}. {step}", i + 1));
            }
            ui.add_space(4.0);
            for g in KEY_GUIDES {
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new(g.name).strong());
                    ui.hyperlink_to("Get a key", g.key_url);
                    if let Some(pay) = g.pay_url {
                        ui.hyperlink_to("Add credit", pay);
                    }
                });
                ui.label(egui::RichText::new(g.about).small().weak());
            }
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(
                    "The key is saved on the stick so bia can use it. Keep the stick safe, like a house key.",
                )
                .small(),
            );
        });
}

/// The window's look: roomier, rounder, bigger text.
fn style(ctx: &egui::Context) {
    use egui::{FontId, TextStyle};
    // both themes: the window follows the system's light or dark mode
    ctx.all_styles_mut(|s| {
    s.spacing.item_spacing = egui::vec2(10.0, 8.0);
    s.spacing.button_padding = egui::vec2(12.0, 6.0);
    s.spacing.interact_size.y = 28.0;
    s.text_styles.insert(TextStyle::Heading, FontId::proportional(26.0));
    s.text_styles.insert(TextStyle::Body, FontId::proportional(15.0));
    s.text_styles.insert(TextStyle::Button, FontId::proportional(15.0));
    s.text_styles.insert(TextStyle::Small, FontId::proportional(12.5));
    s.text_styles.insert(TextStyle::Monospace, FontId::monospace(13.0));
    for w in [&mut s.visuals.widgets.noninteractive, &mut s.visuals.widgets.inactive, &mut s.visuals.widgets.hovered, &mut s.visuals.widgets.active, &mut s.visuals.widgets.open] {
        w.rounding = egui::Rounding::same(9.0);
    }
    let dark = s.visuals.dark_mode;
    // fields stand out on the cards; cards stand out on the page
    s.visuals.extreme_bg_color = if dark { egui::Color32::from_gray(18) } else { egui::Color32::from_gray(236) };
    s.visuals.faint_bg_color = if dark { egui::Color32::from_gray(38) } else { egui::Color32::WHITE };
    s.visuals.panel_fill = if dark { egui::Color32::from_gray(27) } else { egui::Color32::from_gray(242) };
    s.visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0_f32, if dark { egui::Color32::from_gray(70) } else { egui::Color32::from_gray(200) });
    s.visuals.selection.bg_fill = ACCENT;
    s.visuals.selection.stroke = egui::Stroke::new(1.0_f32, egui::Color32::WHITE);
    s.visuals.hyperlink_color = egui::Color32::from_rgb(70, 130, 210);
    s.visuals.window_rounding = egui::Rounding::same(14.0);
    });
}

/// A rounded panel around one step.
fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::none()
        .fill(ui.visuals().faint_bg_color)
        .stroke(egui::Stroke::new(1.0_f32, ui.visuals().widgets.noninteractive.bg_stroke.color))
        .rounding(14.0)
        .inner_margin(egui::Margin::same(16.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

fn filled_button(ui: &mut egui::Ui, text: &str, fill: egui::Color32, enabled: bool) -> egui::Response {
    let b = egui::Button::new(egui::RichText::new(text).size(16.0).strong().color(egui::Color32::WHITE))
        .fill(fill)
        .rounding(18.0)
        .min_size(egui::vec2(180.0, 38.0));
    // in a row the text sits in the middle of the button (top-down layouts put it left)
    ui.horizontal(|ui| ui.add_enabled(enabled, b)).inner
}

fn primary_button(ui: &mut egui::Ui, text: &str, enabled: bool) -> egui::Response {
    filled_button(ui, text, ACCENT, enabled)
}

fn danger_button(ui: &mut egui::Ui, text: &str, enabled: bool) -> egui::Response {
    filled_button(ui, text, DANGER, enabled)
}
