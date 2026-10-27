//! "Set up my server": address and login (password, key file or pasted key, or a new key
//! for the hosting provider), the server's host key on first contact, the checks, one
//! confirmation, the takeover, then root's certificate saved and trusted here and the wait
//! until the new bia answers.
use super::{card, danger_button, primary_button, FlashApp, ACCENT, GOOD};
use bia_deploy::remote::{self, ssh::Ssh, trust, Agent, Auth, Deploy, Done, Facts, HostKey, Login, Plan, Progress};
use eframe::egui;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Copy, PartialEq)]
pub enum How {
    Password,
    Key,
    NewKey,
}

#[derive(Clone, Copy, PartialEq)]
enum Step {
    Login,
    Connecting,
    HostKey,
    Checking,
    Confirm,
    Running,
    Installed,
}

enum Msg {
    Connected(Result<(Arc<Ssh>, Facts), Conn>),
    Plan(Result<Plan, String>),
    Line(String),
    Certificate(PathBuf),
    Rebooting,
    Ran(Result<Done, String>),
    Trusted(trust::Report),
    Tick(Duration),
    Up(Result<(), String>),
}

enum Conn {
    HostKey(HostKey),
    Other(String),
}

pub struct Server {
    pub land: bool,
    pub land_secret: String,
    host: String,
    port: String,
    user: String,
    pub how: How,
    password: String,
    key_text: String,
    key_from: String,
    passphrase: String,
    new_key: Option<(String, PathBuf)>,
    step: Step,
    ssh: Option<Arc<Ssh>>,
    facts: Option<Facts>,
    plan: Option<Plan>,
    understood: bool,
    host_key: Option<HostKey>,
    error: Option<String>,
    progress: (String, Option<f32>, String),
    cert: Option<PathBuf>,
    rebooting: bool,
    done: Option<Done>,
    trusted: Option<trust::Report>,
    waited: Duration,
    up: Option<Result<(), String>>,
    /// The address bia was put on (the done page shows it after the login is gone).
    target: String,
    /// The ChatGPT sign-in went to this server (the window drops its copy).
    pub chatgpt_used: bool,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
}

impl Server {
    pub fn new() -> Server {
        let (tx, rx) = mpsc::channel();
        let new_key = remote::keys::saved().into_iter().next().map(|(p, k)| (k, p));
        Server {
            land: false,
            land_secret: bia_deploy::takeover::land_secret(),
            host: String::new(),
            port: "22".into(),
            user: "root".into(),
            how: How::Password,
            password: String::new(),
            key_text: String::new(),
            key_from: String::new(),
            passphrase: String::new(),
            new_key,
            step: Step::Login,
            ssh: None,
            facts: None,
            plan: None,
            understood: false,
            host_key: None,
            error: None,
            progress: (String::new(), None, String::new()),
            cert: None,
            rebooting: false,
            done: None,
            trusted: None,
            waited: Duration::ZERO,
            up: None,
            chatgpt_used: false,
            target: String::new(),
            tx,
            rx,
        }
    }

    /// Sample state for screenshots of the later steps (`BIA_DEPLOY_PAGE`).
    pub fn demo(&mut self, page: &str) {
        self.host = "203.0.113.10".into();
        self.target = self.host.clone();
        self.facts = Some(Facts { hostname: "vps-4f2a".into(), os: "Ubuntu 24.04.3 LTS".into(), arch: "x86_64".into(), ram_mib: 3900, root_login: true });
        let row = |k: &str, v: &str| (k.to_string(), v.to_string(), String::new());
        self.plan = Some(Plan {
            disk: "/dev/sda".into(),
            land: false,
            rows: vec![
                row("mode", "new agent"),
                row("fmm", "openrouter deepseek/deepseek-v4.1-flash key sk-or-…9f2c"),
                row("lang", "English (en)"),
                row("network", "eth0 52:54:00:12:34:56 dhcp"),
                row("disk", "/dev/sda 40960 MiB"),
                row("system", "bia 1.4.3 (v1.4.3): 1190 MiB download, 3456 MiB on disk"),
            ],
        });
        match page {
            "hostkey" => {
                self.host_key = Some(HostKey::Changed {
                    known: "SHA256:q3Xh1vXo9bK0Q2m7Jd4sJxT1y8fGz6uWcR5pLmN0aE4".into(),
                    now: "SHA256:Zt8bW2xQ0pLr7nYc4Hk1sVd9Ua6Ej3Mf5Gi2Oq8Rw0T".into(),
                });
                self.step = Step::HostKey;
            }
            "confirm" => self.step = Step::Confirm,
            "running" => {
                self.step = Step::Running;
                self.progress = ("downloading bia-x86_64.img.xz".into(), Some(0.42), "500 / 1190 MiB".into());
                self.cert = Some(PathBuf::from("bia-root.p12"));
            }
            _ => {
                self.step = Step::Installed;
                self.done = Some(Done { folder: Some(PathBuf::from("/tmp")), ca_pem: String::new(), ca_sha256: "4f1c9a0e7b2d…".into(), agent_name: "bia-4f1c".into() });
                self.trusted = Some(trust::Report {
                    done: vec!["root's certificate is in your login keychain".into(), "bia's certificate authority is trusted".into()],
                    todo: vec!["Firefox keeps its own certificates: import bia-root.p12 there if you use it".into()],
                });
                self.up = Some(Ok(()));
            }
        }
    }

    /// On the first step (the window's own Back button leaves the server pages).
    pub fn at_start(&self) -> bool {
        self.step == Step::Login
    }

    /// Work is going on in the background (the window keeps repainting).
    pub fn busy(&self) -> bool {
        matches!(self.step, Step::Connecting | Step::Checking | Step::Running) || (self.step == Step::Installed && self.up.is_none())
    }

    fn login(&self) -> Result<Login, String> {
        let host = self.host.trim().trim_start_matches("ssh://").trim_start_matches(|c| c == '[').trim_end_matches(']').to_string();
        // root@1.2.3.4 typed into the address field
        let (user, host) = match host.split_once('@') {
            Some((u, h)) => (u.to_string(), h.to_string()),
            None => (self.user.trim().to_string(), host),
        };
        if host.is_empty() {
            return Err("type the server's address (the IP your provider shows, like 203.0.113.10)".into());
        }
        let port: u16 = self.port.trim().parse().map_err(|_| "the port is a number, usually 22".to_string())?;
        let auth = match self.how {
            How::Password if self.password.is_empty() => return Err("type the server's password".into()),
            How::Password => Auth::Password(self.password.clone()),
            How::Key if self.key_text.trim().is_empty() => return Err("choose the key file, or paste the key".into()),
            How::Key => Auth::Key { pem: self.key_text.clone(), passphrase: Some(self.passphrase.clone()).filter(|p| !p.is_empty()) },
            How::NewKey => match &self.new_key {
                Some((_, p)) => Auth::Key { pem: std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?, passphrase: None },
                None => return Err("make a key first, and give it to the provider".into()),
            },
        };
        Ok(Login { host, port, user: if user.is_empty() { "root".into() } else { user }, auth })
    }

    fn connect(&mut self, accept: Option<String>) {
        let l = match self.login() {
            Ok(l) => l,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        self.error = None;
        self.step = Step::Connecting;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let r = Ssh::connect(&l, accept.as_deref())
                .map_err(|e| match e.downcast_ref::<HostKey>() {
                    Some(h) => Conn::HostKey(h.clone()),
                    None => Conn::Other(format!("{e:#}")),
                })
                .and_then(|s| remote::facts(&s).map(|f| (Arc::new(s), f)).map_err(|e| Conn::Other(format!("{e:#}"))));
            let _ = tx.send(Msg::Connected(r));
        });
    }

    fn back_to_login(&mut self) {
        self.step = Step::Login;
        self.ssh = None;
        self.plan = None;
        self.understood = false;
    }
}

impl FlashApp {
    fn deploy(&self) -> Deploy {
        let agent = if self.srv.land {
            Agent::Land { secret: self.srv.land_secret.clone() }
        } else {
            Agent::New { fmm: self.fmm_input(), lang: self.lang.clone() }
        };
        let host = self.srv.login().map(|l| l.host).unwrap_or_default();
        Deploy { host, agent, os: None }
    }

    fn check_server(&mut self) {
        let Some(s) = self.srv.ssh.clone() else { return };
        self.srv.step = Step::Checking;
        self.srv.error = None;
        let (tx, d) = (self.srv.tx.clone(), self.deploy());
        std::thread::spawn(move || {
            let r = remote::plan(&s, &d, &mut |l| {
                let _ = tx.send(Msg::Line(l));
            });
            let _ = tx.send(Msg::Plan(r.map_err(|e| format!("{e:#}"))));
        });
    }

    fn run_takeover(&mut self) {
        let (Some(s), Some(p)) = (self.srv.ssh.clone(), self.srv.plan.clone()) else { return };
        self.srv.step = Step::Running;
        self.srv.error = None;
        self.srv.progress = ("starting".into(), None, String::new());
        let (tx, d) = (self.srv.tx.clone(), self.deploy());
        self.srv.target = d.host.clone();
        self.srv.chatgpt_used = matches!(&d.agent, Agent::New { fmm, .. } if fmm.chatgpt.is_some());
        let land = matches!(d.agent, Agent::Land { .. });
        std::thread::spawn(move || {
            let sink = std::sync::Mutex::new(tx.clone());
            let r = remote::run(&s, &d, &p.disk, &|m| {
                let t = sink.lock().unwrap();
                let _ = t.send(match m {
                    Progress::Line(l) => Msg::Line(l),
                    Progress::Certificate(p) => Msg::Certificate(p),
                    Progress::Rebooting => Msg::Rebooting,
                });
            });
            drop(s);
            let done = match r {
                Ok(d) => d,
                Err(e) => {
                    let _ = tx.send(Msg::Ran(Err(format!("{e:#}"))));
                    return;
                }
            };
            let _ = tx.send(Msg::Ran(Ok(done.clone())));
            if let Some(dir) = &done.folder {
                if let Ok(r) = trust::install(dir, &format!("bia {}", d.host)) {
                    let _ = tx.send(Msg::Trusted(r));
                }
            }
            let ca = (!land).then_some(done.ca_pem.as_str());
            let r = remote::wait_up(&d.host, ca, Duration::from_secs(25 * 60), &mut |t| {
                let _ = tx.send(Msg::Tick(t));
            });
            let _ = tx.send(Msg::Up(r.map_err(|e| format!("{e:#}"))));
        });
    }

    /// Messages from the background work; called every frame.
    pub fn server_poll(&mut self) {
        while let Ok(m) = self.srv.rx.try_recv() {
            match m {
                Msg::Connected(Ok((s, f))) => {
                    self.append_log(&format!("connected: {} — {} ({}, {} MiB RAM)", f.hostname, f.os, f.arch, f.ram_mib));
                    self.srv.ssh = Some(s);
                    self.srv.facts = Some(f);
                    self.srv.host_key = None;
                    self.check_server();
                }
                Msg::Connected(Err(Conn::HostKey(h))) => {
                    self.srv.host_key = Some(h);
                    self.srv.step = Step::HostKey;
                }
                Msg::Connected(Err(Conn::Other(e))) => {
                    self.append_log(&format!("error: {e}"));
                    self.srv.error = Some(e);
                    self.srv.step = Step::Login;
                }
                Msg::Plan(Ok(p)) => {
                    self.srv.plan = Some(p);
                    self.srv.understood = false;
                    self.srv.step = Step::Confirm;
                }
                Msg::Plan(Err(e)) => {
                    self.append_log(&format!("error: {e}"));
                    self.srv.error = Some(e);
                    self.srv.step = Step::Login;
                }
                Msg::Line(l) => {
                    if let Some(line) = l.strip_prefix("== ") {
                        self.srv.progress = match bia_deploy::parse_progress(line) {
                            Some((s, f, d)) => (s.to_string(), Some(f), d.to_string()),
                            None => (line.to_string(), None, String::new()),
                        };
                    }
                    if !l.contains("% (") {
                        self.append_log(&l);
                    }
                }
                Msg::Certificate(p) => {
                    self.append_log(&format!("root's certificate saved: {}", p.display()));
                    self.srv.cert = Some(p);
                }
                Msg::Rebooting => self.srv.rebooting = true,
                Msg::Ran(Ok(d)) => {
                    if self.srv.chatgpt_used {
                        self.chatgpt_sign_out();
                        self.append_log("The ChatGPT sign-in now belongs to this server. Press Continue with ChatGPT again for another bia.");
                    }
                    self.srv.done = Some(d);
                    self.srv.step = Step::Installed;
                }
                Msg::Ran(Err(e)) => {
                    self.append_log(&format!("error: {e}"));
                    self.srv.error = Some(e);
                    self.srv.step = Step::Confirm;
                }
                Msg::Trusted(r) => {
                    for l in &r.done {
                        self.append_log(&format!("ok: {l}"));
                    }
                    for l in &r.todo {
                        self.append_log(&format!("to do: {l}"));
                    }
                    self.srv.trusted = Some(r);
                }
                Msg::Tick(t) => self.srv.waited = t,
                Msg::Up(r) => self.srv.up = Some(r),
            }
        }
    }

    pub fn server_page(&mut self, ui: &mut egui::Ui) {
        match self.srv.step {
            Step::Login | Step::Connecting => self.login_step(ui),
            Step::HostKey => self.host_key_step(ui),
            Step::Checking => {
                card(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(egui::RichText::new("Looking at the server…").strong());
                    });
                    ui.label("bia checks the disk, memory and network there. Nothing changes yet.");
                });
            }
            Step::Confirm => self.confirm_step(ui),
            Step::Running => self.running_step(ui),
            Step::Installed => self.installed_step(ui),
        }
        if let Some(e) = self.srv.error.clone() {
            ui.add_space(6.0);
            let red = ui.visuals().error_fg_color;
            egui::Frame::none().stroke(egui::Stroke::new(1.0_f32, red)).rounding(10.0).inner_margin(egui::Margin::same(12.0)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.colored_label(red, egui::RichText::new("That did not work").strong());
                ui.label(e);
            });
        }
    }

    fn login_step(&mut self, ui: &mut egui::Ui) {
        let busy = self.srv.step == Step::Connecting;
        card(ui, |ui| {
            ui.label(egui::RichText::new(if self.srv.land { "Which server will receive an agent?" } else { "Which server?" }).size(19.0).strong());
            ui.label(
                egui::RichText::new("A Linux server you rent (a VPS). Its provider's page shows its address and how to log in. Everything on it will be replaced by bia.")
                    .weak(),
            );
            ui.add_space(6.0);
            egui::Grid::new("addr").num_columns(2).spacing([12.0, 10.0]).show(ui, |ui| {
                ui.label("Address");
                ui.add(egui::TextEdit::singleline(&mut self.srv.host).hint_text("203.0.113.10").desired_width(260.0));
                ui.end_row();
                ui.label("User");
                ui.add(egui::TextEdit::singleline(&mut self.srv.user).desired_width(140.0))
                    .on_hover_text("Usually root. Some providers use ubuntu or debian instead; bia then uses sudo.");
                ui.end_row();
                ui.label("Port");
                ui.add(egui::TextEdit::singleline(&mut self.srv.port).desired_width(60.0)).on_hover_text("The SSH port, almost always 22");
                ui.end_row();
            });
        });
        ui.add_space(8.0);
        card(ui, |ui| {
            ui.label(egui::RichText::new("How do you log in?").size(19.0).strong());
            ui.horizontal_wrapped(|ui| {
                ui.selectable_value(&mut self.srv.how, How::Password, "🔑  With a password");
                ui.selectable_value(&mut self.srv.how, How::Key, "📄  With a key file");
                ui.selectable_value(&mut self.srv.how, How::NewKey, "✨  I need a key");
            });
            ui.add_space(4.0);
            match self.srv.how {
                How::Password => {
                    ui.add(egui::TextEdit::singleline(&mut self.srv.password).password(true).hint_text("the server's password").desired_width(300.0));
                    ui.label(egui::RichText::new("The password is used only now, to log in. It is not saved.").small().weak());
                }
                How::Key => {
                    ui.horizontal(|ui| {
                        if ui.button("Choose the key file…").clicked() {
                            if let Some(p) = rfd::FileDialog::new().set_title("Choose your private SSH key").pick_file() {
                                match std::fs::read_to_string(&p) {
                                    Ok(t) => {
                                        self.srv.key_text = t;
                                        self.srv.key_from = p.display().to_string();
                                    }
                                    Err(e) => self.srv.error = Some(format!("{}: {e}", p.display())),
                                }
                            }
                        }
                        if !self.srv.key_from.is_empty() {
                            ui.label(egui::RichText::new(&self.srv.key_from).small());
                        }
                    });
                    ui.label(egui::RichText::new("…or paste it (the whole text, from BEGIN to END):").small());
                    let r = ui.add(egui::TextEdit::multiline(&mut self.srv.key_text).font(egui::TextStyle::Monospace).desired_rows(3).desired_width(f32::INFINITY));
                    if r.changed() {
                        self.srv.key_from.clear();
                    }
                    ui.horizontal(|ui| {
                        ui.label("Passphrase");
                        ui.add(egui::TextEdit::singleline(&mut self.srv.passphrase).password(true).hint_text("only if the key has one").desired_width(200.0));
                    });
                }
                How::NewKey => self.new_key_ui(ui),
            }
        });
        if self.srv.land {
            ui.add_space(8.0);
            self.land_secret_card(ui);
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if primary_button(ui, "Connect", !busy).clicked() {
                self.srv.host_key = None;
                self.srv.connect(None);
            }
            if busy {
                ui.spinner();
                ui.label("connecting…");
            }
        });
    }

    fn new_key_ui(&mut self, ui: &mut egui::Ui) {
        ui.label("When you order a server, the provider asks for an \"SSH key\". bia makes one here; you give the provider its public half, and bia keeps the secret half on this computer.");
        match self.srv.new_key.clone() {
            None => {
                if primary_button(ui, "Make a key", true).clicked() {
                    match remote::keys::generate("bia-deploy") {
                        Ok(k) => {
                            self.append_log(&format!("new SSH key: {}", k.path.display()));
                            self.srv.new_key = Some((k.public, k.path));
                        }
                        Err(e) => self.srv.error = Some(format!("{e:#}")),
                    }
                }
            }
            Some((public, path)) => {
                ui.label(egui::RichText::new("1. Copy this and paste it where the provider asks for an SSH key:").strong());
                egui::Frame::none().fill(ui.visuals().extreme_bg_color).rounding(8.0).inner_margin(egui::Margin::same(8.0)).show(ui, |ui| {
                    ui.add(egui::Label::new(egui::RichText::new(&public).monospace().small()).wrap());
                });
                ui.horizontal(|ui| {
                    if ui.button("📋  Copy").clicked() {
                        ui.ctx().copy_text(public.clone());
                    }
                    if ui.small_button("Make another").clicked() {
                        self.srv.new_key = None;
                    }
                });
                ui.label(egui::RichText::new("2. When the server is ready, type its address above and press Connect (user root, unless the provider says otherwise).").strong());
                ui.label(egui::RichText::new(format!("The secret half is saved in {}", path.display())).small().weak());
            }
        }
    }

    fn land_secret_card(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            ui.label(egui::RichText::new("Land secret").size(19.0).strong());
            ui.label("This server becomes an empty bia that waits for one of your bias to fly its agent there. That bia needs this secret:");
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(&self.srv.land_secret).monospace().size(17.0).strong());
                if ui.button("📋  Copy").clicked() {
                    ui.ctx().copy_text(self.srv.land_secret.clone());
                }
            });
            ui.collapsing(egui::RichText::new("Use the secret my bia made instead").small(), |ui| {
                ui.add(egui::TextEdit::singleline(&mut self.srv.land_secret).desired_width(340.0).font(egui::TextStyle::Monospace));
            });
        });
    }

    fn host_key_step(&mut self, ui: &mut egui::Ui) {
        let h = self.srv.host_key.clone();
        card(ui, |ui| match &h {
            Some(HostKey::Changed { known, now }) => {
                let red = ui.visuals().error_fg_color;
                ui.label(egui::RichText::new("This server's fingerprint changed").size(19.0).strong().color(red));
                ui.label(format!("bia knew it as {known}, now it says {now}."));
                ui.label("A server that was reinstalled gets a new fingerprint. If you did not reinstall it, stop here: someone may be in between.");
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if danger_button(ui, "It was reinstalled: trust the new one", true).clicked() {
                        if let Ok(l) = self.srv.login() {
                            let _ = remote::ssh::forget(&l.id());
                        }
                        self.srv.connect(Some(now.clone()));
                    }
                    if ui.button("Back").clicked() {
                        self.srv.back_to_login();
                    }
                });
            }
            None => {}
        });
    }

    fn confirm_step(&mut self, ui: &mut egui::Ui) {
        let (Some(f), Some(p)) = (self.srv.facts.clone(), self.srv.plan.clone()) else { return };
        card(ui, |ui| {
            ui.label(egui::RichText::new(format!("{}  ·  {}", f.hostname, f.os)).size(19.0).strong());
            ui.label(format!(
                "{} server, {:.1} GiB of memory{}",
                if f.arch == "x86_64" { "Intel/AMD" } else { "ARM" },
                f.ram_mib as f64 / 1024.0,
                if f.root_login { "" } else { "; bia works as root through sudo" }
            ));
            ui.add_space(6.0);
            egui::Grid::new("plan").num_columns(2).spacing([14.0, 4.0]).striped(true).show(ui, |ui| {
                for (k, v, _) in &p.rows {
                    // the takeover's own names, in words; where it stages files is its business
                    let k = match k.as_str() {
                        "staging" => continue,
                        "mode" => "bia",
                        "fmm" => "brain",
                        "lang" => "language",
                        "root certificate" => "bia's key",
                        "server certificate" => "names in its certificate",
                        "land secret" => "land secret",
                        k => k,
                    };
                    ui.label(egui::RichText::new(k).weak());
                    ui.add(egui::Label::new(v).wrap());
                    ui.end_row();
                }
            });
        });
        ui.add_space(8.0);
        let red = ui.visuals().error_fg_color;
        card(ui, |ui| {
            ui.label(egui::RichText::new(format!("⚠  Everything on {} is erased", f.hostname)).size(19.0).strong().color(red));
            ui.label(format!(
                "Its disk ({}) is replaced by bia: files, websites, databases, all of it. Afterwards {}",
                p.disk,
                if p.land {
                    "bia waits for one of your bias to fly its agent there with the land secret."
                } else {
                    "you reach bia in the browser, with the certificate this app saves and installs on this computer."
                }
            ));
            ui.checkbox(&mut self.srv.understood, "I understand, nothing on this server needs to be kept");
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if danger_button(ui, &format!("Replace {} with bia", f.hostname), self.srv.understood).clicked() {
                    self.run_takeover();
                }
                if ui.button("Back").clicked() {
                    self.srv.back_to_login();
                }
            });
        });
        if p.land {
            ui.add_space(8.0);
            self.land_secret_card(ui);
        }
    }

    fn running_step(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            ui.label(egui::RichText::new("Putting bia on the server").size(19.0).strong());
            let (step, frac, detail) = self.srv.progress.clone();
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(egui::RichText::new(&step).strong());
            });
            if let Some(f) = frac {
                ui.add(egui::ProgressBar::new(f).text(format!("{:.0}%  {detail}", f * 100.0)));
            }
            ui.add_space(4.0);
            check(ui, self.srv.cert.is_some(), "bia's key (root's certificate) is saved on this computer");
            check(ui, self.srv.rebooting, "the server restarts into bia's installer");
            ui.label(egui::RichText::new("Keep this window open. Nothing on the server changes before bia's key is safe here.").small().weak());
        });
    }

    fn installed_step(&mut self, ui: &mut egui::Ui) {
        let done = self.srv.done.clone().unwrap_or_default();
        let host = self.srv.target.clone();
        let url = format!("https://{}/", if host.contains(':') { format!("[{host}]") } else { host.clone() });
        card(ui, |ui| match &self.srv.up {
            None => {
                ui.label(egui::RichText::new("bia is installing itself").size(19.0).strong());
                ui.horizontal(|ui| {
                    ui.spinner();
                    let t = self.srv.waited.as_secs();
                    ui.label(format!("The server writes bia onto its disk and starts it. This takes a few minutes… {}:{:02}", t / 60, t % 60));
                });
            }
            Some(Ok(())) => {
                ui.label(egui::RichText::new("🎉  bia is up!").size(22.0).strong().color(GOOD));
                if self.srv.land {
                    ui.label(format!("It waits at {host} for your bia. Tell your bia: fly to {host} with land secret {}", self.srv.land_secret));
                } else {
                    ui.label(format!("Open it in the browser: {url}"));
                    if primary_button(ui, "Open bia", true).clicked() {
                        let _ = bia_deploy::chatgpt::open_browser(&url);
                    }
                }
            }
            Some(Err(e)) => {
                ui.label(egui::RichText::new("bia did not answer yet").size(19.0).strong());
                ui.label(e);
                if !self.srv.land && ui.button("Try to open it anyway").clicked() {
                    let _ = bia_deploy::chatgpt::open_browser(&url);
                }
            }
        });
        if let Some(dir) = done.folder.clone() {
            ui.add_space(8.0);
            card(ui, |ui| {
                ui.label(egui::RichText::new("bia's key").size(19.0).strong());
                ui.label("bia-root.p12 is root's certificate: the only way into this bia. Keep a copy somewhere safe (password 12345678).");
                ui.label(egui::RichText::new(format!("CA fingerprint {}", done.ca_sha256)).monospace().small().weak());
                match &self.srv.trusted {
                    None => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("installing it on this computer…");
                        });
                    }
                    Some(r) => {
                        for l in &r.done {
                            check(ui, true, l);
                        }
                        for l in &r.todo {
                            ui.label(format!("•  {l}"));
                        }
                    }
                }
                ui.horizontal(|ui| {
                    if ui.button("Save a copy…").clicked() {
                        if let Some(to) = rfd::FileDialog::new().set_file_name("bia-root.p12").save_file() {
                            if let Err(e) = std::fs::copy(dir.join("bia-root.p12"), &to) {
                                self.srv.error = Some(format!("{}: {e}", to.display()));
                            }
                        }
                    }
                    if ui.button("Show the folder").clicked() {
                        let _ = bia_deploy::chatgpt::open_browser(&dir.to_string_lossy());
                    }
                });
            });
        }
        if self.srv.land {
            ui.add_space(8.0);
            self.land_secret_card(ui);
        }
        ui.add_space(8.0);
        if ui.button("Set up another server").clicked() {
            let land = self.srv.land;
            self.srv = Server::new();
            self.srv.land = land;
        }
    }
}

fn check(ui: &mut egui::Ui, done: bool, text: &str) {
    ui.horizontal(|ui| {
        if done {
            ui.label(egui::RichText::new("✔").color(GOOD).strong());
            ui.label(text);
        } else {
            ui.label(egui::RichText::new("○").color(ACCENT));
            ui.label(egui::RichText::new(text).weak());
        }
    });
}
