//! Ferogram client: auth, messaging, bans, update loop (+ call command pump).

use crate::calls::{self, CallCmd};
use bia_mcp::Ctx;
use ferogram::reactions::InputReactions;
use ferogram::tl;
use ferogram::update::Update;
use ferogram::{
    BinaryFileBackend, Client, ExponentialBackoff, InputMessage, LoginToken, PasswordToken,
    SendCodeOutcome, SessionBackend, SignInError, ShutdownToken,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// Recent inbound message id → chat id. Private-chat DeleteMessages updates have
/// no peer; we need this to route `[deleted]` notices to the right channel.
const MSG_INDEX_CAP: usize = 4000;
/// Suppress delete notices for ids we just removed via telegram_delete_messages.
const SELF_DELETE_SUPPRESS: Duration = Duration::from_secs(120);

/// Cap catch-up so a long offline stretch cannot flood the model.
const CATCHUP_MAX_DIALOGS: i32 = 100;
const CATCHUP_MAX_PER_CHAT: i32 = 50;
const CATCHUP_MAX_TOTAL: usize = 150;

/// Unread catch-up runs once per healthy connection; reset on drop/reconnect.
static CATCHUP_DONE: AtomicBool = AtomicBool::new(false);

/// Telegram app credentials, compiled in from the build environment (repo `.env`,
/// read by builders/build.sh) so they ship in the binary but never in git.
pub const API_ID: i32 = parse_api_id(env!(
    "BIA_TG_API_ID",
    "set BIA_TG_API_ID in .env (see .env.example)"
));
pub const API_HASH: &str = env!(
    "BIA_TG_API_HASH",
    "set BIA_TG_API_HASH in .env (see .env.example)"
);

const fn parse_api_id(s: &str) -> i32 {
    let b = s.as_bytes();
    assert!(!b.is_empty(), "BIA_TG_API_ID is empty");
    let mut i = 0;
    let mut n: i32 = 0;
    while i < b.len() {
        assert!(b[i].is_ascii_digit(), "BIA_TG_API_ID must be a number");
        n = n * 10 + (b[i] - b'0') as i32;
        i += 1;
    }
    n
}

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct Cfg {
    #[serde(default)]
    pub phone: String,
}

struct AuthPending {
    login: Option<LoginToken>,
    password: Option<PasswordToken>,
}

struct Inner {
    client: Option<Client>,
    shutdown: Option<ShutdownToken>,
    ctx: Option<Ctx>,
    auth: AuthPending,
    banned: HashSet<i64>,
    phase: String,
    call_tx: Option<mpsc::UnboundedSender<CallCmd>>,
    /// Bumped when a connection is replaced so a dying update loop cannot clear the new one.
    generation: u64,
    /// After logout, do not auto-reconnect until login/restore.
    suppress_reconnect: bool,
}

struct MsgIndex {
    map: HashMap<i32, i64>,
    order: VecDeque<i32>,
}

impl MsgIndex {
    fn note(&mut self, msg_id: i32, chat_id: i64) {
        if msg_id == 0 || chat_id == 0 {
            return;
        }
        if self.map.insert(msg_id, chat_id).is_none() {
            self.order.push_back(msg_id);
        }
        while self.order.len() > MSG_INDEX_CAP {
            if let Some(old) = self.order.pop_front() {
                self.map.remove(&old);
            }
        }
    }

    fn chat_of(&self, msg_id: i32) -> Option<i64> {
        self.map.get(&msg_id).copied()
    }

    fn forget(&mut self, ids: &[i32]) {
        for id in ids {
            self.map.remove(id);
        }
    }
}

pub struct Tg {
    inner: Mutex<Inner>,
    /// Inbound message id → chat (for delete updates without a peer).
    msg_index: Mutex<MsgIndex>,
    /// Ids we deleted via the tool; drop matching Update::MessageDeleted echoes.
    self_deletes: Mutex<HashMap<i32, Instant>>,
    connect_lock: Mutex<()>,
    reconnecting: AtomicBool,
    rt: tokio::runtime::Runtime,
}

impl Tg {
    pub fn global() -> &'static Tg {
        static G: OnceLock<Tg> = OnceLock::new();
        G.get_or_init(|| {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_name("tg-tokio")
                .build()
                .expect("tokio runtime");
            let tg = Tg {
                inner: Mutex::new(Inner {
                    client: None,
                    shutdown: None,
                    ctx: None,
                    auth: AuthPending {
                        login: None,
                        password: None,
                    },
                    banned: read_bans(),
                    phase: "disconnected".into(),
                    call_tx: None,
                    generation: 0,
                    suppress_reconnect: false,
                }),
                msg_index: Mutex::new(MsgIndex {
                    map: HashMap::new(),
                    order: VecDeque::new(),
                }),
                self_deletes: Mutex::new(HashMap::new()),
                connect_lock: Mutex::new(()),
                reconnecting: AtomicBool::new(false),
                rt,
            };
            tg.ensure_health_supervisor();
            tg
        })
    }

    pub fn block_on<F: std::future::Future>(&self, f: F) -> F::Output {
        self.rt.block_on(f)
    }

    pub fn set_ctx(&self, ctx: Ctx) {
        self.inner.lock().unwrap().ctx = Some(ctx.clone());
        // messages that arrived before the agent link existed
        let held: Vec<_> = held_inbound().lock().map(|mut h| std::mem::take(&mut *h)).unwrap_or_default();
        for (identity, channel, id, text) in held {
            ctx.inbound(&identity, &channel, &id, &text);
        }
    }

    pub fn ctx_clone(&self) -> Option<Ctx> {
        self.inner.lock().unwrap().ctx.clone()
    }

    pub fn phase(&self) -> String {
        self.inner.lock().unwrap().phase.clone()
    }

    pub fn client(&self) -> Result<Client, String> {
        self.inner
            .lock()
            .unwrap()
            .client
            .clone()
            .ok_or_else(|| "telegram client not connected; call restore or login".into())
    }

    fn set_phase(&self, p: &str) {
        self.inner.lock().unwrap().phase = p.into();
    }

    pub fn call_tx(&self) -> Result<mpsc::UnboundedSender<CallCmd>, String> {
        self.inner
            .lock()
            .unwrap()
            .call_tx
            .clone()
            .ok_or_else(|| "telegram update loop not running; call restore or login".into())
    }

    pub fn format_chat(chat_id: i64, title: &str, username: &str) -> String {
        if username.is_empty() {
            format!("telegram:{chat_id}  {title}")
        } else {
            format!("telegram:{chat_id}  {title}  @{username}")
        }
    }

    pub fn files_dir() -> PathBuf {
        PathBuf::from(std::env::var("BIA_FILES").unwrap_or_else(|_| "/home/bia/files".into()))
    }

    pub fn cfg_path() -> PathBuf {
        Self::files_dir().join("state").join("telegram.json")
    }

    pub fn session_path() -> PathBuf {
        Self::files_dir().join("telegram").join("ferogram.session")
    }

    pub fn load_cfg() -> Cfg {
        std::fs::read(Self::cfg_path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save_cfg(c: &Cfg) -> Result<(), String> {
        if let Some(p) = Self::cfg_path().parent() {
            std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        std::fs::write(
            Self::cfg_path(),
            serde_json::to_vec_pretty(c).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        unsafe { libc::sync() };
        Ok(())
    }

    pub fn ensure_connected(&self) -> Result<Client, String> {
        if let Ok(c) = self.client() {
            return Ok(c);
        }
        let _guard = self.connect_lock.lock().unwrap();
        if let Ok(c) = self.client() {
            return Ok(c);
        }
        self.block_on(self.connect_async())
    }

    /// Drop the live client without deleting the session file.
    fn drop_connection(&self, reason: &str) {
        let mut g = self.inner.lock().unwrap();
        g.generation = g.generation.wrapping_add(1);
        if let Some(c) = g.client.take() {
            c.disconnect();
        }
        if let Some(s) = g.shutdown.take() {
            s.cancel();
        }
        g.call_tx = None;
        if matches!(g.phase.as_str(), "ready" | "disconnected") {
            g.phase = "disconnected".into();
        }
        drop(g);
        CATCHUP_DONE.store(false, Ordering::SeqCst);
        self.log(format!("connection dropped: {reason}"));
    }

    fn session_exists() -> bool {
        Self::session_path().is_file()
    }

    fn allow_auto_reconnect(&self) -> bool {
        let g = self.inner.lock().unwrap();
        !g.suppress_reconnect && Self::session_exists()
    }

    fn is_msg_id_time_window(err: &str) -> bool {
        err.contains("MsgIdTimeWindow")
    }

    /// Clear persisted MTProto clock skew so the next connect re-derives it.
    /// Needed after network blips / NTP jumps: a stale `time_offset` makes decrypt
    /// fail with `MsgIdTimeWindow` and reconnect loops forever otherwise.
    fn reset_session_time_offsets() -> Result<bool, String> {
        let backend = BinaryFileBackend::new(Self::session_path());
        let Some(mut session) = backend.load().map_err(|e| e.to_string())? else {
            return Ok(false);
        };
        let mut changed = false;
        for dc in &mut session.dcs {
            if dc.time_offset != 0 {
                dc.time_offset = 0;
                changed = true;
            }
        }
        if changed {
            backend.save(&session).map_err(|e| e.to_string())?;
        }
        Ok(changed)
    }

    /// Background reconnect with exponential backoff after network drops.
    fn kick_reconnect(&self) {
        if !self.allow_auto_reconnect() {
            return;
        }
        if self.reconnecting.swap(true, Ordering::SeqCst) {
            return;
        }
        let _ = std::thread::Builder::new()
            .name("tg-reconnect".into())
            .spawn(|| {
                let mut delay = Duration::from_secs(1);
                let mut cleared_time_offset = false;
                for attempt in 1u32..=40 {
                    let tg = Tg::global();
                    if !tg.allow_auto_reconnect() {
                        break;
                    }
                    // Already healthy?
                    if let Ok(c) = tg.client() {
                        if tg.phase() == "ready" {
                            match tg.block_on(async { c.is_authorized().await }) {
                                Ok(true) => break,
                                Ok(false) => {
                                    tg.set_phase("waiting_phone");
                                    tg.log("reconnect: session not authorized; need login");
                                    break;
                                }
                                Err(_) => {}
                            }
                        }
                    }

                    tg.drop_connection(&format!("reconnect attempt {attempt}"));
                    std::thread::sleep(Duration::from_millis(250));

                    let connect = {
                        let _guard = tg.connect_lock.lock().unwrap();
                        if let Ok(c) = tg.client() {
                            Ok(c)
                        } else {
                            tg.block_on(tg.connect_async())
                        }
                    };
                    match connect {
                        Ok(client) => match tg.block_on(async { client.is_authorized().await }) {
                            Ok(true) => {
                                tg.set_phase("ready");
                                tg.log(format!("reconnected (attempt {attempt})"));
                                let n = tg.block_on(deliver_unread_catchup(&client));
                                if n > 0 {
                                    tg.log(format!("catch-up after reconnect: {n}"));
                                }
                                break;
                            }
                            Ok(false) => {
                                tg.set_phase("waiting_phone");
                                tg.log("reconnect: connected but not logged in");
                                break;
                            }
                            Err(e) => {
                                tg.log(format!("reconnect auth probe failed: {e}"));
                                tg.drop_connection("auth probe failed after connect");
                            }
                        },
                        Err(e) => {
                            tg.log(format!("reconnect connect failed: {e}"));
                            if !cleared_time_offset && Self::is_msg_id_time_window(&e) {
                                match Self::reset_session_time_offsets() {
                                    Ok(true) => {
                                        cleared_time_offset = true;
                                        tg.log(
                                            "reconnect: cleared stale session time_offset; retrying",
                                        );
                                        delay = Duration::from_secs(1);
                                        continue;
                                    }
                                    Ok(false) => {}
                                    Err(reset_err) => tg.log(format!(
                                        "reconnect: failed to clear time_offset: {reset_err}"
                                    )),
                                }
                            }
                        }
                    }
                    std::thread::sleep(delay);
                    delay = (delay.saturating_mul(2)).min(Duration::from_secs(60));
                }
                Tg::global().reconnecting.store(false, Ordering::SeqCst);
            });
    }

    fn ensure_health_supervisor(&self) {
        static STARTED: OnceLock<()> = OnceLock::new();
        STARTED.get_or_init(|| {
            let _ = std::thread::Builder::new()
                .name("tg-health".into())
                .spawn(|| loop {
                    std::thread::sleep(Duration::from_secs(15));
                    Tg::global().health_tick();
                });
        });
    }

    /// Probe a supposedly-live session; reconnect on transport death.
    fn health_tick(&self) {
        if !self.allow_auto_reconnect() {
            return;
        }
        let phase = self.phase();
        match (phase.as_str(), self.client().ok()) {
            ("ready", Some(c)) => match self.block_on(async { c.is_authorized().await }) {
                Ok(true) => {}
                Ok(false) => {
                    // Treat "ready but unauthorized" as a dead link worth one reconnect.
                    self.log("health: ready but not authorized; reconnecting");
                    self.kick_reconnect();
                }
                Err(e) => {
                    self.log(format!("health: transport error ({e}); reconnecting"));
                    self.kick_reconnect();
                }
            },
            ("ready", None) | ("disconnected", _) => self.kick_reconnect(),
            _ => {}
        }
    }

    /// Called by status/restore: heal without requiring the model to reconnect manually.
    pub fn heal_if_needed(&self) {
        self.ensure_health_supervisor();
        if !self.allow_auto_reconnect() {
            return;
        }
        match self.client() {
            Ok(c) => match self.block_on(async { c.is_authorized().await }) {
                Ok(true) => self.set_phase("ready"),
                Ok(false) if self.phase() == "ready" => self.kick_reconnect(),
                Err(_) => self.kick_reconnect(),
                _ => {}
            },
            Err(_) if matches!(self.phase().as_str(), "ready" | "disconnected") => {
                self.kick_reconnect();
            }
            Err(_) => {}
        }
    }

    async fn connect_async(&self) -> Result<Client, String> {
        if let Ok(c) = self.client() {
            return Ok(c);
        }
        match self.connect_async_once().await {
            Ok(c) => Ok(c),
            Err(e) if Self::is_msg_id_time_window(&e) => {
                match Self::reset_session_time_offsets() {
                    Ok(true) => {
                        self.log(
                            "connect: cleared stale session time_offset after MsgIdTimeWindow; retrying",
                        );
                        self.connect_async_once().await
                    }
                    Ok(false) => Err(e),
                    Err(reset_err) => Err(format!("{e}; also failed to clear time_offset: {reset_err}")),
                }
            }
            Err(e) => Err(e),
        }
    }

    async fn connect_async_once(&self) -> Result<Client, String> {
        if let Ok(c) = self.client() {
            return Ok(c);
        }
        let path = Self::session_path();
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }

        let (client, shutdown) = Client::builder()
            .api_id(API_ID)
            .api_hash(API_HASH)
            .session(path.to_string_lossy().as_ref())
            .restart_policy(Arc::new(ExponentialBackoff::default()))
            .connect()
            .await
            .map_err(|e| format!("connect failed: {e}"))?;

        // Network errors must not look like "logged out".
        let authorized = match client.is_authorized().await {
            Ok(v) => v,
            Err(e) => {
                client.disconnect();
                return Err(format!("auth probe failed: {e}"));
            }
        };
        let (call_tx, call_rx) = mpsc::unbounded_channel();
        let generation = {
            let mut g = self.inner.lock().unwrap();
            g.generation = g.generation.wrapping_add(1);
            g.client = Some(client.clone());
            g.shutdown = Some(shutdown);
            g.call_tx = Some(call_tx);
            g.suppress_reconnect = false;
            g.phase = if authorized {
                "ready".into()
            } else {
                "waiting_phone".into()
            };
            g.generation
        };

        // P2PCall/NTgCalls is Send but not Sync — keep the update+call loop on a
        // dedicated current-thread runtime (same pattern as tgcalls::Calls workers).
        let client_bg = client.clone();
        std::thread::Builder::new()
            .name("tg-updates".into())
            .spawn(move || {
                let local = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("tg update runtime");
                local.block_on(run_update_loop(client_bg, call_rx, generation));
            })
            .map_err(|e| format!("spawn update loop: {e}"))?;

        Ok(client)
    }

    /// Deliver one inbound message to the model. Returns `true` if it was pushed.
    async fn handle_inbound_message(
        client: &Client,
        msg: ferogram::update::IncomingMessage,
    ) -> Result<bool, String> {
        if msg.outgoing() {
            return Ok(false);
        }
        let chat_id = marked_chat_id(msg.peer_id());
        if chat_id == 0 {
            return Ok(false);
        }
        let tg = Tg::global();
        if tg.is_banned(chat_id) {
            let _ = client
                .send_message(chat_id, InputMessage::text("I've banned you"))
                .await;
            // Still clear the unread badge so banned chats don't pile up.
            let _ = msg.mark_read_with(client).await;
            return Ok(false);
        }
        let identity = {
            let uid = msg.sender_user_id().unwrap_or(0);
            if uid != 0 {
                user_label(client, uid).await
            } else {
                format!("telegram:{chat_id}")
            }
        };
        let mut text = msg.text().unwrap_or("").to_string();
        let mid = msg.id();
        watermark::note(chat_id, mid);
        let msg_id = mid.to_string();
        tg.note_msg(mid, chat_id);
        // During an active voice call, ignore chat media (live STT covers speech).
        if calls::Calls::global().active_chat() == Some(chat_id) {
            if text.is_empty() {
                return Ok(false);
            }
            tg.inbound(&identity, chat_id, &msg_id, &text);
            texts().put(&format!("{chat_id}:{mid}"), msg.date() as i64, msg.text().unwrap_or(""));
            mark_delivered_read(client, &msg, chat_id).await;
            return Ok(true);
        }
        if let Some(tl::enums::MessageMedia::GeoLive(geo_live)) = msg.media() {
            // The first point of a share reaches the model; the moves come as edits.
            let Some((lat, lon, acc)) = geo_of(&geo_live.geo) else { return Ok(false) };
            let (until, _) = live_until(msg.date(), geo_live.period);
            let lk = bia_mcp::share::live_key(&chat_id.to_string(), &msg.sender_user_id().unwrap_or(0).to_string(), chat_id < 0);
            let ev = live().seen(&lk, &identity, lat, lon, acc, until, false, now_secs());
            if ev != bia_mcp::share::LiveEvent::Started {
                return Ok(false);
            }
            let shared = shared_text(msg.media().unwrap()).unwrap_or_default();
            text = format!(
                "{}{shared}\n(the position updates by itself; telegram_live_location gives where they are now)",
                if text.is_empty() { String::new() } else { format!("{text}\n") }
            );
        } else if let Some(shared) = msg.media().and_then(shared_text) {
            // A location or contact: nothing to download, the text is the content.
            text = if text.is_empty() { shared } else { format!("{text}\n{shared}") };
        } else if let Some(media) = msg.media() {
            if let Ok(rel) = save_inbound_media(client, chat_id, media).await {
                if text.is_empty() {
                    text = format!("file: {rel}");
                } else {
                    text = format!("{text}\nfile: {rel}");
                }
                if is_audio(&rel) {
                    // Voice notes: transcribed off the update loop's thread.
                    let abs = Self::files_dir().join(&rel);
                    let lang = bia_speech::contacts::lang_for(&format!("telegram:{chat_id}"));
                    let heard = tokio::task::spawn_blocking(move || bia_speech::transcribe_note_from(&abs, lang.as_deref()))
                        .await
                        .map_err(|e| e.to_string())
                        .and_then(|r| r);
                    text = with_transcript(heard, &text, &format!("telegram/{rel}"));
                }
            }
        }
        if text.is_empty() {
            // No bare "(media)" — skip empty media failures.
            return Ok(false);
        }
        tg.inbound(&identity, chat_id, &msg_id, &text);
        // The text as sent (no file notes), so an edit can show what it was.
        texts().put(&format!("{chat_id}:{mid}"), msg.date() as i64, msg.text().unwrap_or(""));
        mark_delivered_read(client, &msg, chat_id).await;
        Ok(true)
    }

    pub fn inbound(&self, identity: &str, chat_id: i64, id: &str, text: &str) {
        let channel = format!("telegram:{chat_id}");
        match self.ctx_clone() {
            Some(ctx) => ctx.inbound(identity, &channel, id, text),
            None => hold_inbound(identity, &channel, id, text),
        }
    }

    fn note_msg(&self, msg_id: i32, chat_id: i64) {
        if let Ok(mut ix) = self.msg_index.lock() {
            ix.note(msg_id, chat_id);
        }
    }

    fn remember_self_deletes(&self, ids: &[i32]) {
        let now = Instant::now();
        if let Ok(mut m) = self.self_deletes.lock() {
            m.retain(|_, t| now.duration_since(*t) < SELF_DELETE_SUPPRESS);
            for id in ids {
                m.insert(*id, now);
            }
        }
    }

    fn take_self_delete(&self, id: i32) -> bool {
        let now = Instant::now();
        let Ok(mut m) = self.self_deletes.lock() else {
            return false;
        };
        m.retain(|_, t| now.duration_since(*t) < SELF_DELETE_SUPPRESS);
        m.remove(&id).is_some()
    }

    pub fn log(&self, msg: impl Into<String>) {
        if let Some(ctx) = self.ctx_clone() {
            ctx.log(msg.into());
        } else {
            eprintln!("telegram: {}", msg.into());
        }
    }

    pub fn is_banned(&self, chat_id: i64) -> bool {
        self.inner.lock().unwrap().banned.contains(&chat_id)
    }

    pub fn banned_ids(&self) -> Vec<i64> {
        let mut ids: Vec<i64> = self.inner.lock().unwrap().banned.iter().copied().collect();
        ids.sort_unstable();
        ids
    }

    pub fn set_banned(&self, chat_id: i64, on: bool) -> Result<bool, String> {
        let mut g = self.inner.lock().unwrap();
        let changed = if on {
            g.banned.insert(chat_id)
        } else {
            g.banned.remove(&chat_id)
        };
        let ids: Vec<i64> = g.banned.iter().copied().collect();
        let tx = g.call_tx.clone();
        drop(g);
        write_bans(&ids)?;
        if on {
            if let Some(tx) = tx {
                let _ = tx.send(CallCmd::RejectBanned { chat_id });
            }
        }
        Ok(changed)
    }

    pub fn login(&self, phone: &str) -> Result<String, String> {
        let mut c = Self::load_cfg();
        c.phone = phone.to_string();
        Self::save_cfg(&c)?;
        let client = self.ensure_connected()?;
        self.block_on(async {
            if client.is_authorized().await.unwrap_or(false) {
                self.set_phase("ready");
                return Ok("already logged in".into());
            }
            match client.request_login_code(phone).await {
                Ok(SendCodeOutcome::AlreadyAuthorized(name)) => {
                    self.set_phase("ready");
                    let _ = client.save_session().await;
                    Ok(format!("auth: ready — welcome back, {name}"))
                }
                Ok(SendCodeOutcome::CodeRequired(token)) => {
                    self.inner.lock().unwrap().auth.login = Some(token);
                    self.set_phase("waiting_code");
                    Ok("auth: waiting_code — waiting for login code (call submit_code)".into())
                }
                Err(e) => Err(format!("request_login_code: {e}")),
            }
        })
    }

    pub fn submit_code(&self, code: &str) -> Result<String, String> {
        let client = self.ensure_connected()?;
        let token = self
            .inner
            .lock()
            .unwrap()
            .auth
            .login
            .clone()
            .ok_or_else(|| "no pending login; call login(phone) first".to_string())?;
        self.block_on(async {
            match client.sign_in(&token, code).await {
                Ok(name) => {
                    self.set_phase("ready");
                    self.inner.lock().unwrap().auth.login = None;
                    let _ = client.save_session().await;
                    Ok(format!("auth: ready — welcome, {name}"))
                }
                Err(SignInError::PasswordRequired(pw)) => {
                    self.inner.lock().unwrap().auth.password = Some(*pw);
                    self.set_phase("waiting_password");
                    Ok(
                        "auth: waiting_password — waiting for 2FA password (call submit_password)"
                            .into(),
                    )
                }
                Err(SignInError::InvalidCode) => Err("invalid or expired code".into()),
                Err(SignInError::SignUpRequired) => {
                    Err("phone not registered; sign up in the official Telegram app first".into())
                }
                Err(SignInError::Other(e)) => Err(format!("sign_in: {e}")),
            }
        })
    }

    pub fn submit_password(&self, password: &str) -> Result<String, String> {
        let client = self.ensure_connected()?;
        let token = self
            .inner
            .lock()
            .unwrap()
            .auth
            .password
            .clone()
            .ok_or_else(|| "no pending 2FA; call submit_code first".to_string())?;
        self.block_on(async {
            match client.check_password(token, password.as_bytes()).await {
                Ok(name) => {
                    self.set_phase("ready");
                    self.inner.lock().unwrap().auth.password = None;
                    let _ = client.save_session().await;
                    Ok(format!("auth: ready — welcome, {name}"))
                }
                Err(e) => Err(format!("check_password: {e}")),
            }
        })
    }

    pub fn require_ready(&self) -> Result<Client, String> {
        self.inner.lock().unwrap().suppress_reconnect = false;
        for attempt in 0..4 {
            let client = match self.ensure_connected() {
                Ok(c) => c,
                Err(e) if attempt + 1 < 4 && Self::session_exists() => {
                    self.log(format!("require_ready connect failed ({e}); retrying"));
                    std::thread::sleep(Duration::from_millis(400 * (attempt + 1) as u64));
                    continue;
                }
                Err(e) => return Err(e),
            };
            match self.block_on(async { client.is_authorized().await }) {
                Ok(true) => {
                    self.set_phase("ready");
                    return Ok(client);
                }
                Ok(false) => {
                    return Err(format!("not logged in ({})", self.phase()));
                }
                Err(e) => {
                    self.log(format!(
                        "require_ready auth probe failed ({e}); reconnecting ({})",
                        attempt + 1
                    ));
                    self.drop_connection("require_ready transport failure");
                    if attempt + 1 >= 4 || !Self::session_exists() {
                        self.kick_reconnect();
                        return Err(format!("telegram disconnected ({e}); reconnecting"));
                    }
                    std::thread::sleep(Duration::from_millis(400 * (attempt + 1) as u64));
                }
            }
        }
        Err("telegram reconnect failed".into())
    }

    pub fn logout(&self) -> Result<String, String> {
        {
            let mut g = self.inner.lock().unwrap();
            g.suppress_reconnect = true;
        }
        if let Ok(client) = self.client() {
            let _ = self.block_on(async {
                let _ = client.sign_out().await;
                let _ = client.save_session().await;
                client.disconnect();
            });
        }
        {
            let mut g = self.inner.lock().unwrap();
            g.generation = g.generation.wrapping_add(1);
            if let Some(s) = g.shutdown.take() {
                s.cancel();
            }
            g.client = None;
            g.call_tx = None;
            g.phase = "disconnected".into();
            g.auth = AuthPending {
                login: None,
                password: None,
            };
        }
        CATCHUP_DONE.store(false, Ordering::SeqCst);
        let _ = std::fs::remove_file(Self::session_path());
        unsafe { libc::sync() };
        Ok("logged out; call login(phone) to sign in again".into())
    }

    pub fn restore(&self) -> Result<String, String> {
        self.inner.lock().unwrap().suppress_reconnect = false;
        self.ensure_health_supervisor();

        // If a prior connection is half-dead, force a clean reconnect from session.
        if let Ok(c) = self.client() {
            match self.block_on(async { c.is_authorized().await }) {
                Ok(true) => {}
                Ok(false) => {}
                Err(e) => {
                    self.log(format!("restore: dropping dead client ({e})"));
                    self.drop_connection("restore transport failure");
                }
            }
        }

        let client = match self.ensure_connected() {
            Ok(c) => c,
            Err(e) => {
                if Self::session_exists() {
                    self.kick_reconnect();
                }
                return Err(e);
            }
        };
        match self.block_on(async { client.is_authorized().await }) {
            Ok(true) => {
                self.set_phase("ready");
                let n = self.block_on(deliver_unread_catchup(&client));
                if n > 0 {
                    Ok(format!("telegram connected; catch-up delivered {n} unread"))
                } else {
                    Ok("telegram connected".into())
                }
            }
            Ok(false) => {
                self.set_phase("waiting_phone");
                Ok(format!(
                    "telegram not logged in ({}); call login(phone)",
                    self.phase()
                ))
            }
            Err(e) => {
                self.drop_connection("restore auth probe failed");
                self.kick_reconnect();
                Err(format!("telegram reconnecting after network error: {e}"))
            }
        }
    }

    pub fn resolve_target(&self, target: &str) -> Result<(i64, String, String), String> {
        let client = self.require_ready()?;
        let t = normalize_target(target);
        self.block_on(resolve_target_async(&client, &t))
    }

    /// Async resolve for code already running on the telegram tokio runtime.
    pub async fn resolve_target_async(
        &self,
        target: &str,
    ) -> Result<(i64, String, String), String> {
        let client = self.client()?;
        let ok = client.is_authorized().await.unwrap_or(false);
        if !ok {
            return Err(format!("not logged in ({})", self.phase()));
        }
        resolve_target_async(&client, &normalize_target(target)).await
    }

    pub fn send(&self, target: &str, text: &str, files: &[String]) -> Result<String, String> {
        let client = self.require_ready()?;
        let (chat_id, title, username) = self.resolve_target(target)?;
        self.block_on(async {
            if files.is_empty() {
                let m = client
                    .send_message(chat_id, InputMessage::text(text))
                    .await
                    .map_err(|e| format!("send: {e}"))?;
                return Ok(format!(
                    "sent{} → {}",
                    id_note(&[m.id()]),
                    Self::format_chat(chat_id, &title, &username)
                ));
            }
            let root = Self::files_dir();
            let mut n = 0;
            let mut voices = 0;
            let mut ids = Vec::new();
            for f in files {
                let path = resolve_file_path(&root, f)?;
                let uploaded = client
                    .upload_file(&path)
                    .await
                    .map_err(|e| format!("upload {}: {e}", path.display()))?;
                let caption = if n == 0 { text } else { "" };
                let msg = InputMessage::text(caption);
                // OGG/Opus goes out as a voice message, not as a file.
                let media = if is_voice_file(&path) {
                    let (secs, wave) = voice_meta(&path);
                    voices += 1;
                    voice_media(&uploaded, secs, wave)
                } else {
                    uploaded.into()
                };
                let m = client
                    .send_file(chat_id, media, &msg)
                    .await
                    .map_err(|e| format!("send_file: {e}"))?;
                ids.push(m.id());
                n += 1;
            }
            let what = match (voices, n - voices) {
                (0, f) => format!("{f} file(s)"),
                (v, 0) => format!("{v} voice message(s)"),
                (v, f) => format!("{v} voice message(s) and {f} file(s)"),
            };
            Ok(format!("sent {what}{} → {}", id_note(&ids), Self::format_chat(chat_id, &title, &username)))
        })
    }

    /// Send a map pin; with a name or address it goes out as a venue (place card).
    pub fn send_location(&self, target: &str, lat: f64, lon: f64, name: &str, address: &str) -> Result<String, String> {
        bia_mcp::share::check_coords(lat, lon)?;
        let client = self.require_ready()?;
        let (chat_id, title, username) = self.resolve_target(target)?;
        let geo = tl::enums::InputGeoPoint::InputGeoPoint(tl::types::InputGeoPoint { lat, long: lon, accuracy_radius: None });
        let media = if name.trim().is_empty() && address.trim().is_empty() {
            tl::enums::InputMedia::GeoPoint(tl::types::InputMediaGeoPoint { geo_point: geo })
        } else {
            tl::enums::InputMedia::Venue(tl::types::InputMediaVenue {
                geo_point: geo,
                title: if name.trim().is_empty() { address.trim().into() } else { name.trim().into() },
                address: address.trim().into(),
                provider: String::new(),
                venue_id: String::new(),
                venue_type: String::new(),
            })
        };
        self.block_on(async {
            let m = client
                .send_file(chat_id, media, &InputMessage::text(""))
                .await
                .map_err(|e| format!("send location: {e}"))?;
            Ok(format!(
                "sent location {lat:.6}, {lon:.6}{} → {}",
                id_note(&[m.id()]),
                Self::format_chat(chat_id, &title, &username)
            ))
        })
    }

    /// Send a contact card (name + phone, optional e-mail in its vCard).
    pub fn send_contact(&self, target: &str, name: &str, phone: &str, email: &str) -> Result<String, String> {
        if name.trim().is_empty() || phone.trim().is_empty() {
            return Err("a contact needs a name and a phone number".into());
        }
        let client = self.require_ready()?;
        let (chat_id, title, username) = self.resolve_target(target)?;
        let vcard = if email.trim().is_empty() { String::new() } else { bia_mcp::share::build_vcard(name, phone, email, false) };
        let media = tl::enums::InputMedia::Contact(tl::types::InputMediaContact {
            phone_number: phone.trim().into(),
            first_name: name.trim().into(),
            last_name: String::new(),
            vcard,
        });
        self.block_on(async {
            let m = client
                .send_file(chat_id, media, &InputMessage::text(""))
                .await
                .map_err(|e| format!("send contact: {e}"))?;
            Ok(format!(
                "sent contact {}{} → {}",
                name.trim(),
                id_note(&[m.id()]),
                Self::format_chat(chat_id, &title, &username)
            ))
        })
    }

    /// Speak `text` (in `lang`) and send it as a voice message (OGG/Opus voice note with
    /// duration and waveform, shown as a voice bubble).
    pub fn send_voice(&self, target: &str, text: &str, lang: &str) -> Result<String, String> {
        let client = self.require_ready()?;
        let (chat_id, title, username) = self.resolve_target(target)?;
        let lang = &bia_speech::contacts::lang_or(lang, &format!("telegram:{chat_id}"));
        let dir = Self::files_dir().join("voice-out");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let out = dir.join(format!("voice-{}.ogg", bia_speech::paths::stamp()));
        let (secs, samples) = bia_speech::tts::voice_note(text, lang, &out)?;
        let waveform = bia_speech::audio::telegram_waveform(&samples);
        let res = self.block_on(async {
            let uploaded = client
                .upload_file(&out)
                .await
                .map_err(|e| format!("upload voice: {e}"))?;
            let media = voice_media(&uploaded, secs, waveform);
            let m = client
                .send_file(chat_id, media, &InputMessage::text(""))
                .await
                .map_err(|e| format!("send voice: {e}"))?;
            Ok(format!(
                "sent voice message ({secs:.1}s){} → {}",
                id_note(&[m.id()]),
                Self::format_chat(chat_id, &title, &username)
            ))
        });
        let _ = std::fs::remove_file(&out);
        res
    }

    /// Update this account's display name and/or bio. Unset fields are left unchanged.
    pub fn set_profile(
        &self,
        first_name: Option<&str>,
        last_name: Option<&str>,
        bio: Option<&str>,
    ) -> Result<String, String> {
        if first_name.is_none() && last_name.is_none() && bio.is_none() {
            return Err("pass first_name, last_name, and/or bio".into());
        }
        let client = self.require_ready()?;
        self.block_on(async {
            let mut b = client.set_profile("me");
            if first_name.is_some() || last_name.is_some() {
                b = b.name(first_name.unwrap_or(""), last_name.unwrap_or(""));
            }
            if let Some(bio) = bio {
                b = b.bio(bio);
            }
            b.send().await.map_err(|e| format!("set_profile: {e}"))?;
            let mut parts = Vec::new();
            if let Some(f) = first_name {
                parts.push(format!("first_name={f:?}"));
            }
            if let Some(l) = last_name {
                parts.push(format!("last_name={l:?}"));
            }
            if let Some(bio) = bio {
                parts.push(format!("bio={bio:?}"));
            }
            Ok(format!("profile updated ({})", parts.join(", ")))
        })
    }

    /// Set this account's profile photo from a file under /home/bia/files.
    pub fn set_photo(&self, path: &str) -> Result<String, String> {
        let client = self.require_ready()?;
        let abs = resolve_file_path(&Self::files_dir(), path)?;
        self.block_on(async {
            client
                .set_profile("me")
                .photo_path(abs.clone())
                .send()
                .await
                .map_err(|e| format!("set_photo: {e}"))?;
            Ok(format!("profile photo set from {}", abs.display()))
        })
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<(i64, String, String)>, String> {
        let client = self.require_ready()?;
        self.block_on(async {
            let peers = client
                .search_contacts(query, limit as i32)
                .await
                .map_err(|e| format!("search: {e}"))?;
            let mut rows = Vec::new();
            for p in peers.into_iter().take(limit) {
                let id = encode_bot_api_id(&p);
                let (title, un) = peer_label(&client, &p).await;
                rows.push((id, title, un));
            }
            Ok(rows)
        })
    }

    pub fn chats(&self) -> Result<Vec<(i64, String, bool)>, String> {
        let client = self.require_ready()?;
        self.block_on(async {
            let dialogs = client
                .get_dialogs(50)
                .await
                .map_err(|e| format!("dialogs: {e}"))?;
            let mut rows = Vec::new();
            for d in dialogs {
                let Some(peer) = d.peer().cloned() else {
                    continue;
                };
                let id = encode_bot_api_id(&peer);
                rows.push((id, d.title(), self.is_banned(id)));
            }
            Ok(rows)
        })
    }

    pub fn history(
        &self,
        target: &str,
        limit: usize,
        before: i64,
    ) -> Result<(i64, String, String, Vec<String>), String> {
        let client = self.require_ready()?;
        let (chat_id, title, username) = self.resolve_target(target)?;
        self.block_on(async {
            let page = client
                .get_message_history(chat_id, limit as i32, before as i32, 0)
                .await
                .map_err(|e| format!("history: {e}"))?;
            let mut lines = Vec::new();
            for m in page.messages {
                let id = m.id();
                let body = match (m.text().filter(|t| !t.is_empty()), m.media().and_then(shared_text)) {
                    (Some(t), Some(sh)) => format!("{t} {sh}"),
                    (Some(t), None) => t.to_string(),
                    (None, Some(sh)) => sh,
                    (None, None) => "(media)".into(),
                }
                .replace('\n', " ");
                let who = if m.outgoing() {
                    "me".into()
                } else {
                    m.sender_user_id()
                        .map(|u| u.to_string())
                        .unwrap_or_else(|| "?".into())
                };
                lines.push(format!("{id}  [{who}]  {body}"));
            }
            Ok((chat_id, title, username, lines))
        })
    }

    /// Replace the text (or caption) of a message this account sent.
    pub fn edit(&self, target: &str, message_id: i64, text: &str) -> Result<String, String> {
        if text.trim().is_empty() {
            return Err("new text is empty".into());
        }
        let client = self.require_ready()?;
        let (chat_id, title, username) = self.resolve_target(target)?;
        let msg_id = i32::try_from(message_id)
            .ok()
            .filter(|i| *i > 0)
            .ok_or_else(|| format!("bad message id: {message_id}"))?;
        self.block_on(async {
            client
                .edit_message(chat_id, msg_id, InputMessage::text(text))
                .await
                .map_err(|e| format!("edit: {e}"))?;
            Ok(format!(
                "edited message {msg_id} → {}",
                Self::format_chat(chat_id, &title, &username)
            ))
        })
    }

    /// React to a message. Empty `emoji` removes this account's reaction.
    pub fn react(
        &self,
        target: &str,
        message_id: i64,
        emoji: &str,
    ) -> Result<(i64, String, String), String> {
        if message_id == 0 {
            return Err("message id 0 is not valid".into());
        }
        let client = self.require_ready()?;
        let (chat_id, title, username) = self.resolve_target(target)?;
        let msg_id = i32::try_from(message_id).map_err(|_| format!("bad message id: {message_id}"))?;
        let emoji = emoji.trim();
        self.block_on(async {
            let reaction = if emoji.is_empty() {
                InputReactions::remove()
            } else {
                InputReactions::emoticon(emoji)
            };
            client
                .send_reaction(chat_id, msg_id, reaction)
                .await
                .map_err(|e| format!("react: {e}"))?;
            Ok((chat_id, title, username))
        })
    }

    pub fn delete_messages(
        &self,
        target: &str,
        ids: &[i64],
        revoke: bool,
    ) -> Result<(usize, i64, String, String), String> {
        let client = self.require_ready()?;
        let (chat_id, title, username) = self.resolve_target(target)?;
        let id32: Vec<i32> = ids.iter().map(|i| *i as i32).collect();
        self.block_on(async {
            let peer = client
                .resolve(chat_id)
                .await
                .map_err(|e| format!("resolve: {e}"))?;
            match peer {
                tl::enums::Peer::Channel(ref c) => {
                    let input = client
                        .resolve_to_input_peer(&peer)
                        .await
                        .map_err(|e| format!("input peer: {e}"))?;
                    let (channel_id, access_hash) = match input {
                        tl::enums::InputPeer::Channel(ch) => (ch.channel_id, ch.access_hash),
                        _ => (c.channel_id, 0),
                    };
                    let req = tl::functions::channels::DeleteMessages {
                        channel: tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                            channel_id,
                            access_hash,
                        }),
                        id: id32.clone(),
                    };
                    client
                        .invoke(&req)
                        .await
                        .map_err(|e| format!("delete: {e}"))?;
                }
                _ => {
                    client
                        .delete_messages(&id32, revoke)
                        .await
                        .map_err(|e| format!("delete: {e}"))?;
                }
            }
            // Tool already reported success; ignore the matching Update::MessageDeleted echo.
            self.remember_self_deletes(&id32);
            if let Ok(mut ix) = self.msg_index.lock() {
                ix.forget(&id32);
            }
            Ok((ids.len(), chat_id, title, username))
        })
    }

    pub fn delete_history(
        &self,
        target: &str,
        revoke: bool,
        remove: bool,
    ) -> Result<(i64, String, String), String> {
        let client = self.require_ready()?;
        let (chat_id, title, username) = self.resolve_target(target)?;
        self.block_on(async {
            client
                .delete_chat_history(chat_id, 0, revoke)
                .await
                .map_err(|e| format!("delete_history: {e}"))?;
            if remove {
                let _ = client.delete_dialog(chat_id).await;
            }
            Ok((chat_id, title, username))
        })
    }

    pub fn set_typing(&self, chat_id: i64, on: bool) {
        let Ok(client) = self.client() else {
            return;
        };
        self.rt.spawn(async move {
            let action = if on {
                tl::enums::SendMessageAction::SendMessageTypingAction
            } else {
                tl::enums::SendMessageAction::SendMessageCancelAction
            };
            let _ = client.send_chat_action(chat_id, action).await;
        });
    }
}

async fn mark_delivered_read(
    client: &Client,
    msg: &ferogram::update::IncomingMessage,
    chat_id: i64,
) {
    if let Err(e) = msg.mark_read_with(client).await {
        Tg::global().log(format!("mark_read chat={chat_id}: {e}"));
    }
}

/// What was delivered: the last time bia was connected and caught up (`since`, advanced
/// by a heartbeat while connected) and the newest message id delivered per chat.
/// Catch-up after a restart delivers every incoming message newer than that, whether or
/// not it was read meanwhile (e.g. by the owner on their phone).
mod watermark {
    use super::Tg;
    use serde::{Deserialize, Serialize};
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    #[derive(Default, Serialize, Deserialize)]
    struct Mark {
        #[serde(default)]
        since: i64,
        #[serde(default)]
        seen: HashMap<String, i32>,
    }

    fn path() -> std::path::PathBuf {
        Tg::files_dir().join("telegram").join("catchup.json")
    }

    fn mark() -> &'static Mutex<Mark> {
        static M: OnceLock<Mutex<Mark>> = OnceLock::new();
        M.get_or_init(|| {
            Mutex::new(std::fs::read(path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default())
        })
    }

    fn save(m: &Mark) {
        let p = path();
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let tmp = p.with_extension("json.tmp");
        if std::fs::write(&tmp, serde_json::to_vec(m).unwrap_or_default()).is_ok() {
            let _ = std::fs::rename(&tmp, &p);
        }
    }

    pub fn now() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    /// 0 when nothing was recorded yet (first start).
    pub fn since() -> i64 {
        mark().lock().map(|m| m.since).unwrap_or(0)
    }

    pub fn seen(chat: i64) -> i32 {
        mark().lock().ok().and_then(|m| m.seen.get(&chat.to_string()).copied()).unwrap_or(0)
    }

    /// A message of `chat` reached the model.
    pub fn note(chat: i64, id: i32) {
        if let Ok(mut m) = mark().lock() {
            let e = m.seen.entry(chat.to_string()).or_insert(0);
            if id > *e {
                *e = id;
                save(&m);
            }
        }
    }

    /// Everything up to `t` has been delivered.
    pub fn advance(t: i64) {
        if let Ok(mut m) = mark().lock() {
            if t > m.since {
                m.since = t;
                save(&m);
            }
        }
    }

    /// While connected and caught up, move `since` forward once a minute, so the next
    /// catch-up starts about where this session stopped.
    pub fn heartbeat() {
        static STARTED: OnceLock<()> = OnceLock::new();
        STARTED.get_or_init(|| {
            std::thread::spawn(|| loop {
                std::thread::sleep(std::time::Duration::from_secs(60));
                if super::CATCHUP_DONE.load(std::sync::atomic::Ordering::SeqCst) && Tg::global().client().is_ok() {
                    advance(now());
                }
            });
        });
    }
}

fn message_date(m: &tl::enums::Message) -> i64 {
    match m {
        tl::enums::Message::Message(x) => x.date as i64,
        tl::enums::Message::Service(x) => x.date as i64,
        _ => 0,
    }
}

/// After reconnect/boot: deliver every incoming message that arrived since bia was last
/// connected and caught up (read or unread), oldest first, then mark the chats read.
/// Runs at most once per healthy connection (reset when the connection drops).
/// Skips broadcast channels and banned chats. The first start ever (no watermark yet)
/// delivers unread messages only.
async fn deliver_unread_catchup(client: &Client) -> usize {
    if CATCHUP_DONE.swap(true, Ordering::SeqCst) {
        return 0;
    }
    // Don't mark chats read if we can't push to the model yet.
    if Tg::global().ctx_clone().is_none() {
        CATCHUP_DONE.store(false, Ordering::SeqCst);
        return 0;
    }
    match client.is_authorized().await {
        Ok(true) => {}
        Ok(false) | Err(_) => {
            CATCHUP_DONE.store(false, Ordering::SeqCst);
            return 0;
        }
    }
    let started = watermark::now();
    let since = watermark::since();

    let dialogs = match client.get_dialogs(CATCHUP_MAX_DIALOGS).await {
        Ok(d) => d,
        Err(e) => {
            Tg::global().log(format!("catch-up dialogs: {e}"));
            CATCHUP_DONE.store(false, Ordering::SeqCst);
            return 0;
        }
    };

    let mut delivered = 0usize;
    let mut announced = false;
    let mut more: Vec<String> = Vec::new();
    for d in dialogs {
        let unread = d.unread_count();
        // Nothing new in this chat since we were last online (or, first start: unread).
        let newest = d.message.as_ref().map(message_date).unwrap_or(0);
        if since > 0 && newest <= since {
            continue;
        }
        if since == 0 && unread <= 0 {
            continue;
        }
        let Some(peer) = d.peer().cloned() else {
            continue;
        };
        // Skip broadcast channels (noise); keep DMs, groups, megagroups.
        if let Some(tl::enums::Chat::Channel(c)) = &d.chat {
            if c.broadcast {
                continue;
            }
        }
        let chat_id = encode_bot_api_id(&peer);
        if chat_id == 0 || Tg::global().is_banned(chat_id) {
            if chat_id != 0 && Tg::global().is_banned(chat_id) {
                let _ = client.mark_read(chat_id).await;
            }
            continue;
        }
        if delivered >= CATCHUP_MAX_TOTAL {
            more.push(format!("telegram:{chat_id} ({})", d.title()));
            continue;
        }

        let page = match client.get_message_history(chat_id, CATCHUP_MAX_PER_CHAT, 0, 0).await {
            Ok(p) => p,
            Err(e) => {
                Tg::global().log(format!("catch-up history chat={chat_id}: {e}"));
                continue;
            }
        };
        let seen = watermark::seen(chat_id);
        // Newest-first → incoming since we were offline → deliver oldest-first.
        let mut batch: Vec<_> = page
            .messages
            .into_iter()
            .filter(|m| !m.outgoing() && m.id() > seen)
            .filter(|m| if since > 0 { m.date() as i64 > since } else { true })
            .collect();
        if since == 0 {
            batch.truncate(unread.clamp(0, CATCHUP_MAX_PER_CHAT) as usize);
        }
        if batch.len() >= CATCHUP_MAX_PER_CHAT as usize {
            more.push(format!("telegram:{chat_id} ({}, older ones)", d.title()));
        }
        batch.reverse();

        for msg in batch {
            if delivered >= CATCHUP_MAX_TOTAL {
                more.push(format!("telegram:{chat_id} ({})", d.title()));
                break;
            }
            if !announced && since > 0 {
                announced = true;
                Tg::global().inbound(
                    "system",
                    chat_id,
                    "",
                    &format!(
                        "[catch-up: Telegram messages that arrived while you were offline (since {}) follow, oldest first]",
                        crate::client::utc_hm(since)
                    ),
                );
            }
            match Tg::handle_inbound_message(client, msg).await {
                Ok(true) => delivered += 1,
                Ok(false) => {}
                Err(e) => Tg::global().log(format!("catch-up deliver chat={chat_id}: {e}")),
            }
        }
        // Ensure the dialog unread badge clears even if some messages were empty/skipped.
        if let Err(e) = client.mark_read(chat_id).await {
            Tg::global().log(format!("catch-up mark_read chat={chat_id}: {e}"));
        }
    }
    if !more.is_empty() {
        more.dedup();
        if let Some(first) = more.first().and_then(|m| m.split_whitespace().next()).and_then(|c| c.strip_prefix("telegram:")).and_then(|c| c.parse::<i64>().ok()) {
            Tg::global().inbound(
                "system",
                first,
                "",
                &format!("[catch-up: more messages from while you were offline were not shown, in {}; telegram_history(chat_id) shows them]", more.join(", ")),
            );
        }
    }
    watermark::advance(started);
    watermark::heartbeat();

    if delivered > 0 {
        Tg::global().log(format!("catch-up: delivered {delivered} message(s) from while offline"));
    }
    delivered
}

/// `HH:MM UTC` (with the date when it is not today).
pub fn utc_hm(t: i64) -> String {
    let now = watermark::now();
    let (h, m) = ((t / 3600) % 24, (t / 60) % 60);
    if now / 86_400 == t / 86_400 {
        format!("{h:02}:{m:02} UTC")
    } else {
        let days = t / 86_400;
        // civil date from days since 1970-01-01 (Howard Hinnant's algorithm)
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = yoe + era * 400 + i64::from(month <= 2);
        format!("{year}-{month:02}-{day:02} {h:02}:{m:02} UTC")
    }
}

async fn run_update_loop(
    client: Client,
    mut call_rx: mpsc::UnboundedReceiver<CallCmd>,
    generation: u64,
) {
    let mut stream = client.stream_updates();
    // Outbound call audio (TTS + silence) is paced on per-call threads; see audio_out.
    let mut call_state = calls::CallLoopState::new();
    // Live locations that ran out or went quiet: checked twice a minute.
    let mut live_tick = tokio::time::interval(Duration::from_secs(30));
    loop {
        tokio::select! {
            biased;
            cmd = call_rx.recv() => {
                let Some(cmd) = cmd else { break; };
                call_state.handle_cmd(&client, &mut stream, cmd).await;
            }
            _ = live_tick.tick() => {
                live_sweep();
                texts().flush();
            }
            maybe = stream.next() => {
                let Some(upd) = maybe else { break; };
                match upd {
                    Update::NewMessage(msg) => {
                        if let Err(e) = Tg::handle_inbound_message(&client, msg).await {
                            Tg::global().log(format!("inbound error: {e}"));
                        }
                    }
                    Update::MessageEdited(msg) => {
                        // Live locations move by editing their message; real edits are
                        // reported with the old and new text.
                        handle_edit(&client, msg).await;
                    }
                    Update::MessageDeleted(del) => {
                        handle_message_deleted(del);
                    }
                    Update::Raw(raw) => {
                        if matches!(&raw.inner, tl::enums::Update::MessageReactions(_)) {
                            handle_reaction_update(&client, &raw.inner).await;
                        }
                        call_state.on_raw(&client, *raw).await;
                    }
                    _ => {}
                }
            }
        }
    }
    // Clear only if this loop still owns the global client, then auto-reconnect.
    let should_reconnect = {
        let mut g = Tg::global().inner.lock().unwrap();
        if g.generation != generation {
            false
        } else {
            if let Some(c) = g.client.take() {
                c.disconnect();
            }
            if let Some(s) = g.shutdown.take() {
                s.cancel();
            }
            g.call_tx = None;
            g.phase = "disconnected".into();
            !g.suppress_reconnect && Tg::session_exists()
        }
    };
    if should_reconnect {
        CATCHUP_DONE.store(false, Ordering::SeqCst);
        Tg::global().log("update loop ended; reconnecting");
        Tg::global().kick_reconnect();
    } else if Tg::global().inner.lock().unwrap().generation == generation {
        Tg::global().set_phase("disconnected");
    }
}

async fn save_inbound_media(
    client: &Client,
    chat_id: i64,
    media: &tl::enums::MessageMedia,
) -> Result<String, String> {
    let inbox = Tg::files_dir()
        .join("inbox")
        .join("telegram")
        .join(chat_id.to_string());
    std::fs::create_dir_all(&inbox).map_err(|e| e.to_string())?;
    let stamp = chrono_stamp();
    // Named from Telegram's metadata (voice notes → .ogg, original file names kept);
    // a bare .bin hid voice notes from transcription.
    let name = format!("{stamp}-{}", media_file_name(media));
    let dest = inbox.join(&name);
    client
        .download_media(media, ferogram::media::MediaQuality::Original, &dest, None)
        .await
        .map_err(|e| format!("download: {e}"))?;
    Ok(format!("inbox/telegram/{chat_id}/{name}"))
}

/// Live locations per chat (`files/state/telegram-live.json`).
fn live() -> &'static bia_mcp::share::LiveTracker {
    static T: OnceLock<bia_mcp::share::LiveTracker> = OnceLock::new();
    T.get_or_init(|| bia_mcp::share::LiveTracker::open(Tg::files_dir().join("state").join("telegram-live.json")))
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn geo_of(g: &tl::enums::GeoPoint) -> Option<(f64, f64, Option<u32>)> {
    match g {
        tl::enums::GeoPoint::GeoPoint(p) => Some((p.lat, p.long, p.accuracy_radius.map(|a| a.max(0) as u32))),
        tl::enums::GeoPoint::Empty => None,
    }
}

/// End of a live share (None: "until I stop") and whether it is already over. When the
/// sender stops early, Telegram shortens `period` to the time it ran.
fn live_until(date: i32, period: i32) -> (Option<u64>, bool) {
    if period == i32::MAX {
        return (None, false);
    }
    let until = (date.max(0) as u64).saturating_add(period.max(0) as u64);
    (Some(until), until <= now_secs())
}

/// Last known text of recent messages (`files/state/telegram-texts.json`).
fn texts() -> &'static bia_mcp::share::TextMemory {
    static T: OnceLock<bia_mcp::share::TextMemory> = OnceLock::new();
    T.get_or_init(|| bia_mcp::share::TextMemory::open(Tg::files_dir().join("state").join("telegram-texts.json"), 5000))
}

/// An edited message. Live locations: record the point (see `handle_live_edit`).
/// Otherwise a real edit by someone else reaches the model with when it was sent, when
/// it was edited, the old text and the new one. Hidden edits (link previews), edits that
/// do not change the text, and our own edits are not reported.
async fn handle_edit(client: &Client, msg: ferogram::update::IncomingMessage) {
    if msg.outgoing() {
        return;
    }
    if matches!(msg.media(), Some(tl::enums::MessageMedia::GeoLive(_))) {
        return handle_live_edit(client, msg).await;
    }
    if msg.edit_hide() {
        return;
    }
    let Some(edited) = msg.edit_date() else { return };
    let chat_id = marked_chat_id(msg.peer_id());
    if chat_id == 0 || Tg::global().is_banned(chat_id) {
        return;
    }
    let mid = msg.id();
    let key = format!("{chat_id}:{mid}");
    let new = msg.text().unwrap_or("").to_string();
    let old = texts().get(&key);
    if old.as_ref().is_some_and(|(_, t)| *t == new) {
        return;
    }
    let who = match msg.sender_user_id().unwrap_or(0) {
        0 => format!("telegram:{chat_id}"),
        uid => user_label(client, uid).await,
    };
    let sent = old.as_ref().map(|(s, _)| *s).or(Some(msg.date() as i64));
    let text = bia_mcp::share::edit_text(&mid.to_string(), sent, edited as i64, old.as_ref().map(|(_, t)| t.as_str()), &new);
    texts().put(&key, msg.date() as i64, &new);
    Tg::global().inbound(&who, chat_id, &format!("{mid}-edit-{edited}"), &text);
}

/// A live location moved: record the point; tell the model only when the share was
/// stopped (or one started while bia was away).
async fn handle_live_edit(client: &Client, msg: ferogram::update::IncomingMessage) {
    let Some(tl::enums::MessageMedia::GeoLive(live_media)) = msg.media() else { return };
    let Some((lat, lon, acc)) = geo_of(&live_media.geo) else { return };
    let chat_id = marked_chat_id(msg.peer_id());
    if chat_id == 0 || Tg::global().is_banned(chat_id) {
        return;
    }
    let (until, stopped) = live_until(msg.date(), live_media.period);
    let chat = bia_mcp::share::live_key(&chat_id.to_string(), &msg.sender_user_id().unwrap_or(0).to_string(), chat_id < 0);
    let who = match live().get(&chat) {
        Some(l) => l.who,
        None => match msg.sender_user_id().unwrap_or(0) {
            0 => format!("telegram:{chat_id}"),
            uid => user_label(client, uid).await,
        },
    };
    let now = now_secs();
    let id = format!("{}-live-{now}", msg.id());
    match live().seen(&chat, &who, lat, lon, acc, until, stopped, now) {
        bia_mcp::share::LiveEvent::Stopped => {
            if let Some(l) = live().get(&chat) {
                Tg::global().inbound(&who, chat_id, &id, &bia_mcp::share::live_ended_text(&l, now));
            }
        }
        bia_mcp::share::LiveEvent::Started if !stopped => {
            if let Some(text) = shared_text(msg.media().unwrap()) {
                Tg::global().inbound(&who, chat_id, &id, &text);
            }
        }
        _ => {}
    }
}

/// Shares that ran out or went quiet: one line each to the model.
fn live_sweep() {
    let now = now_secs();
    for (chat, l) in live().sweep(now) {
        if let Ok(chat_id) = bia_mcp::share::live_chat(&chat).parse::<i64>() {
            Tg::global().inbound(&l.who, chat_id, &format!("live-end-{chat}-{now}"), &bia_mcp::share::live_ended_text(&l, now));
        }
    }
}

/// Where someone sharing a live location in `target` is now.
impl Tg {
    pub fn live_location(&self, target: &str) -> Result<String, String> {
        let (chat_id, title, username) = self.resolve_target(target)?;
        match live().status_for_chat(&chat_id.to_string(), now_secs()) {
            Some(s) => Ok(s),
            None => Err(format!("no live location from {} (ask them to share one)", Self::format_chat(chat_id, &title, &username))),
        }
    }
}

/// Text for a shared location, venue, live location or contact (None for files).
fn shared_text(media: &tl::enums::MessageMedia) -> Option<String> {
    use bia_mcp::share;
    use tl::enums::{GeoPoint, MessageMedia};
    let point = |g: &GeoPoint| match g {
        GeoPoint::GeoPoint(p) => Some((p.lat, p.long, p.accuracy_radius.map(|a| a.max(0) as u32))),
        GeoPoint::Empty => None,
    };
    match media {
        MessageMedia::Geo(m) => {
            let (lat, lon, acc) = point(&m.geo)?;
            Some(share::location_text(lat, lon, "", "", acc, None, ""))
        }
        MessageMedia::GeoLive(m) => {
            let (lat, lon, acc) = point(&m.geo)?;
            Some(share::location_text(lat, lon, "", "", acc, Some(m.period.max(0) as u32), ""))
        }
        MessageMedia::Venue(m) => {
            let (lat, lon, acc) = point(&m.geo)?;
            Some(share::location_text(lat, lon, &m.title, &m.address, acc, None, ""))
        }
        MessageMedia::Contact(m) => {
            let mut card = if m.vcard.trim().is_empty() { share::Card::default() } else { share::parse_vcard(&m.vcard) };
            let name = format!("{} {}", m.first_name.trim(), m.last_name.trim()).trim().to_string();
            if !name.is_empty() {
                card.name = name;
            }
            let phone = m.phone_number.trim();
            if !phone.is_empty() {
                let phone = if phone.starts_with('+') { phone.to_string() } else { format!("+{phone}") };
                let digits = |p: &str| p.chars().filter(|c| c.is_ascii_digit()).collect::<String>();
                if !card.phones.iter().any(|p| digits(p) == digits(&phone)) {
                    card.phones.insert(0, phone);
                }
            }
            let extra = if m.user_id != 0 { format!("on Telegram as chat {}", m.user_id) } else { String::new() };
            Some(share::contact_text(&card, &extra))
        }
        _ => None,
    }
}

/// File name for downloaded media: `voice.ogg`, the sender's file name, or `media.<ext>`.
fn media_file_name(media: &tl::enums::MessageMedia) -> String {
    use tl::enums::{Document, DocumentAttribute, MessageMedia};
    match media {
        MessageMedia::Photo(_) => "photo.jpg".into(),
        MessageMedia::Document(md) => {
            let Some(Document::Document(d)) = &md.document else {
                return "media.bin".into();
            };
            let mut voice = md.voice;
            let mut file_name = None;
            for a in &d.attributes {
                match a {
                    DocumentAttribute::Audio(x) => voice |= x.voice,
                    DocumentAttribute::Filename(f) => file_name = Some(f.file_name.clone()),
                    _ => {}
                }
            }
            let ext = ext_for_mime(&d.mime_type);
            if voice {
                return format!("voice.{}", if ext == "bin" { "ogg" } else { ext });
            }
            match file_name.map(|n| safe_file_name(&n)).filter(|n| !n.is_empty()) {
                Some(n) if n.contains('.') => n,
                Some(n) => format!("{n}.{ext}"),
                None => format!("media.{ext}"),
            }
        }
        _ => "media.bin".into(),
    }
}

fn ext_for_mime(mime: &str) -> &'static str {
    match mime.split(';').next().unwrap_or("").trim() {
        "audio/ogg" | "audio/opus" | "application/ogg" => "ogg",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/mp4" | "audio/x-m4a" | "audio/aac" | "audio/m4a" => "m4a",
        "audio/wav" | "audio/x-wav" | "audio/wave" => "wav",
        "audio/flac" | "audio/x-flac" => "flac",
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "video/quicktime" => "mov",
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "application/pdf" => "pdf",
        "application/zip" => "zip",
        "text/plain" => "txt",
        _ => "bin",
    }
}

/// Keep letters, digits, `.`, `-`, `_`; no leading dots; at most 80 characters.
fn safe_file_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' })
        .collect();
    s.trim_start_matches('.').chars().take(80).collect()
}

/// Notify the model that messages were deleted in a chat (`system|telegram:…|[deleted] …`).
fn handle_message_deleted(del: ferogram::update::MessageDeletion) {
    let tg = Tg::global();
    let ids = del.message_ids;
    if ids.is_empty() {
        return;
    }
    // Channel / supergroup deletes carry the channel id; private deletes do not.
    let chat_hint = del.channel_id.map(|c| -(c + 1_000_000_000_000));
    let mut keep = Vec::new();
    for id in &ids {
        if !tg.take_self_delete(*id) {
            keep.push(*id);
        }
    }
    let mut by_chat: HashMap<i64, Vec<i32>> = HashMap::new();
    {
        let ix = tg.msg_index.lock().unwrap();
        for id in &keep {
            let chat = chat_hint.or_else(|| ix.chat_of(*id));
            let Some(chat) = chat else {
                continue;
            };
            if tg.is_banned(chat) {
                continue;
            }
            by_chat.entry(chat).or_default().push(*id);
        }
    }
    if let Ok(mut ix) = tg.msg_index.lock() {
        ix.forget(&ids);
    }
    for (chat_id, mids) in by_chat {
        let list = mids
            .iter()
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        let first = mids.first().map(|i| i.to_string()).unwrap_or_default();
        let body = if mids.len() == 1 {
            format!("[deleted] {list}")
        } else {
            format!("[deleted] {} messages: {list}", mids.len())
        };
        tg.inbound("system", chat_id, &first, &body);
    }
}

/// Deliver an inbound reaction update to the model (skip own-reaction echoes).
async fn handle_reaction_update(client: &Client, upd: &tl::enums::Update) {
    let tl::enums::Update::MessageReactions(u) = upd else {
        return;
    };
    let chat_id = encode_bot_api_id(&u.peer);
    if chat_id == 0 {
        return;
    }
    let tg = Tg::global();
    if tg.is_banned(chat_id) {
        return;
    }

    let reactions = match &u.reactions {
        tl::enums::MessageReactions::MessageReactions(r) => r,
    };

    // Skip updates that only reflect our own reaction (tool echo).
    if let Some(recent) = &reactions.recent_reactions {
        let mut saw_unread = false;
        let mut only_mine = true;
        for rr in recent {
            let tl::enums::MessagePeerReaction::MessagePeerReaction(p) = rr;
            if p.unread {
                saw_unread = true;
                if !p.my {
                    only_mine = false;
                }
            }
        }
        if saw_unread && only_mine {
            return;
        }
    }

    let summary = format_reaction_summary(reactions);
    let body = if summary.is_empty() {
        "[reaction] cleared".to_string()
    } else {
        format!("[reaction] {summary}")
    };

    // Prefer a non-my unread reactor as the identity; else the chat.
    let identity = {
        let mut uid = 0i64;
        if let Some(recent) = &reactions.recent_reactions {
            for rr in recent {
                let tl::enums::MessagePeerReaction::MessagePeerReaction(p) = rr;
                if p.unread && !p.my {
                    if let tl::enums::Peer::User(user) = &p.peer_id {
                        uid = user.user_id;
                        break;
                    }
                }
            }
        }
        if uid != 0 {
            user_label(client, uid).await
        } else {
            format!("telegram:{chat_id}")
        }
    };
    tg.inbound(&identity, chat_id, &u.msg_id.to_string(), &body);
}

fn format_reaction_emoji(r: &tl::enums::Reaction) -> String {
    match r {
        tl::enums::Reaction::Emoji(e) => e.emoticon.clone(),
        tl::enums::Reaction::CustomEmoji(c) => format!("custom:{}", c.document_id),
        tl::enums::Reaction::Paid => "⭐".into(),
        tl::enums::Reaction::Empty => String::new(),
    }
}

fn format_reaction_summary(r: &tl::types::MessageReactions) -> String {
    let mut parts = Vec::new();
    for res in &r.results {
        let tl::enums::ReactionCount::ReactionCount(c) = res;
        let emoji = format_reaction_emoji(&c.reaction);
        if emoji.is_empty() || c.count <= 0 {
            continue;
        }
        if c.count == 1 {
            parts.push(emoji);
        } else {
            parts.push(format!("{emoji}×{}", c.count));
        }
    }
    parts.join(" ")
}

async fn user_label(client: &Client, user_id: i64) -> String {
    match client.get_users_by_id(&[user_id]).await {
        Ok(users) => users
            .into_iter()
            .flatten()
            .next()
            .map(|u| {
                let name = u.full_name();
                if let Some(un) = u.username() {
                    if !un.is_empty() {
                        return format!("{name} (@{un})");
                    }
                }
                if name.is_empty() {
                    format!("user:{user_id}")
                } else {
                    name
                }
            })
            .unwrap_or_else(|| format!("user:{user_id}")),
        Err(_) => format!("user:{user_id}"),
    }
}

async fn resolve_target_async(
    client: &Client,
    target: &str,
) -> Result<(i64, String, String), String> {
    let peer = client
        .resolve(target)
        .await
        .map_err(|e| format!("resolve: {e}"))?;
    let chat_id = encode_bot_api_id(&peer);
    let (title, username) = peer_label(client, &peer).await;
    Ok((chat_id, title, username))
}

fn normalize_target(target: &str) -> String {
    let t = target.trim();
    t.strip_prefix("telegram:")
        .or_else(|| t.strip_prefix("telegram-"))
        .unwrap_or(t)
        .trim()
        .to_string()
}

fn encode_bot_api_id(peer: &tl::enums::Peer) -> i64 {
    match peer {
        tl::enums::Peer::User(u) => u.user_id,
        tl::enums::Peer::Chat(c) => -c.chat_id,
        tl::enums::Peer::Channel(c) => -(c.channel_id + 1_000_000_000_000),
    }
}

fn marked_chat_id(peer: Option<&tl::enums::Peer>) -> i64 {
    peer.map(encode_bot_api_id).unwrap_or(0)
}

async fn peer_label(client: &Client, peer: &tl::enums::Peer) -> (String, String) {
    match peer {
        tl::enums::Peer::User(u) => match client.get_users_by_id(&[u.user_id]).await {
            Ok(users) => match users.into_iter().flatten().next() {
                Some(u) => {
                    let name = u.full_name();
                    let un = u.username().unwrap_or("").to_string();
                    (
                        if name.is_empty() {
                            format!("user:{}", u.id())
                        } else {
                            name
                        },
                        un,
                    )
                }
                None => (format!("user:{}", u.user_id), String::new()),
            },
            Err(_) => (format!("user:{}", u.user_id), String::new()),
        },
        tl::enums::Peer::Chat(c) => (format!("chat:{}", c.chat_id), String::new()),
        tl::enums::Peer::Channel(c) => (format!("channel:{}", c.channel_id), String::new()),
    }
}

fn is_voice_file(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "ogg" | "oga" | "opus"))
}

/// Duration and waveform of an audio file (zero and none if it cannot be decoded).
fn voice_meta(p: &Path) -> (f32, Vec<u8>) {
    match bia_speech::audio::decode_file(p) {
        Ok(x) if !x.is_empty() => (x.len() as f32 / 16_000.0, bia_speech::audio::telegram_waveform(&x)),
        _ => (0.0, vec![]),
    }
}

/// An uploaded OGG/Opus file as a Telegram voice message (voice bubble, not a file).
fn voice_media(uploaded: &ferogram::media::UploadedFile, secs: f32, waveform: Vec<u8>) -> tl::enums::InputMedia {
    let mut media = uploaded.as_auto_media();
    if let tl::enums::InputMedia::UploadedDocument(d) = &mut media {
        d.mime_type = "audio/ogg".into();
        d.force_file = false;
        // Same attributes as Telethon's voice_note: the voice-flagged audio attribute
        // plus the file name.
        d.attributes = vec![
            tl::enums::DocumentAttribute::Audio(tl::types::DocumentAttributeAudio {
                voice: true,
                duration: secs.ceil() as i32,
                title: None,
                performer: None,
                waveform: (!waveform.is_empty()).then_some(waveform),
            }),
            tl::enums::DocumentAttribute::Filename(tl::types::DocumentAttributeFilename {
                file_name: "voice.ogg".into(),
            }),
        ];
    }
    media
}

fn is_audio(rel: &str) -> bool {
    let l = rel.to_ascii_lowercase();
    [".ogg", ".oga", ".opus", ".wav", ".mp3", ".m4a", ".aac", ".flac", ".webm"].iter().any(|e| l.ends_with(e))
}

/// A received voice/audio message for the model: `[voice message] [speech ru] …`, then
/// the message's own lines (file: …), then a notice if a language was just learned.
fn with_transcript(heard: Result<bia_speech::Heard, String>, text: &str, _file: &str) -> String {
    match heard {
        Ok(h) => {
            let spoken = h.text();
            let mut out = if spoken.is_empty() {
                format!("[voice message, no words found]\n{text}")
            } else {
                format!("[voice message] {spoken}\n{text}")
            };
            if let Some(n) = h.notice {
                out.push('\n');
                out.push_str(&n);
            }
            out
        }
        Err(e) => format!("[voice message, transcription failed: {e}]\n{text}"),
    }
}

fn resolve_file_path(root: &Path, f: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(f);
    let abs = if p.is_absolute() {
        p
    } else {
        root.join(f.trim_start_matches('/'))
    };
    if !abs.exists() {
        return Err(format!("file not found: {}", abs.display()));
    }
    Ok(abs)
}

fn bans_path() -> PathBuf {
    Tg::files_dir().join("state").join("telegram-bans.json")
}

fn read_bans() -> HashSet<i64> {
    let Ok(b) = std::fs::read(bans_path()) else {
        return HashSet::new();
    };
    let v: serde_json::Value = serde_json::from_slice(&b).unwrap_or(serde_json::json!({}));
    v.get("chats")
        .and_then(|c| c.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_i64())
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default()
}

fn write_bans(ids: &[i64]) -> Result<(), String> {
    if let Some(p) = bans_path().parent() {
        std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
    }
    let v = serde_json::json!({ "chats": ids });
    std::fs::write(bans_path(), serde_json::to_vec_pretty(&v).unwrap()).map_err(|e| e.to_string())?;
    unsafe { libc::sync() };
    Ok(())
}

fn chrono_stamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86400;
    let tod = secs % 86400;
    let h = tod / 3600;
    let m = (tod % 3600) / 60;
    let s = tod % 60;
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{y:04}{mo:02}{d:02}-{h:02}{m:02}{s:02}")
}

// Keep Arc import used if we add shared state later.
#[allow(dead_code)]
fn _keep_arc() {
    let _ = Arc::new(());
}

/// Inbound messages that arrived before the agent link (`set_ctx`) existed; delivered
/// when it is set instead of being lost (at most 500 kept).
type Held = (String, String, String, String);

fn held_inbound() -> &'static Mutex<Vec<Held>> {
    static H: OnceLock<Mutex<Vec<Held>>> = OnceLock::new();
    H.get_or_init(|| Mutex::new(Vec::new()))
}

fn hold_inbound(identity: &str, channel: &str, id: &str, text: &str) {
    if let Ok(mut h) = held_inbound().lock() {
        if h.len() < 500 {
            h.push((identity.to_string(), channel.to_string(), id.to_string(), text.to_string()));
        }
    }
}

/// `, message id N` (or `ids N, M`) for a send result; empty when Telegram returned no id.
fn id_note(ids: &[i32]) -> String {
    let ids: Vec<String> = ids.iter().filter(|i| **i > 0).map(|i| i.to_string()).collect();
    if ids.is_empty() {
        String::new()
    } else {
        format!(", message id {}", ids.join(", "))
    }
}

#[cfg(test)]
mod shared_tests {
    use super::*;

    fn geo(lat: f64, long: f64) -> tl::enums::GeoPoint {
        tl::enums::GeoPoint::GeoPoint(tl::types::GeoPoint { long, lat, access_hash: 0, accuracy_radius: Some(15) })
    }

    #[test]
    fn locations_and_contacts_become_text() {
        use tl::enums::MessageMedia as M;
        let pin = M::Geo(tl::types::MessageMediaGeo { geo: geo(52.520008, 13.404954) });
        assert_eq!(
            shared_text(&pin).unwrap(),
            "[location] 52.520008, 13.404954 (±15 m) https://maps.google.com/?q=52.520008,13.404954"
        );
        let venue = M::Venue(tl::types::MessageMediaVenue {
            geo: geo(48.8584, 2.2945),
            title: "Eiffel Tower".into(),
            address: "Champ de Mars".into(),
            provider: String::new(),
            venue_id: String::new(),
            venue_type: String::new(),
        });
        assert!(shared_text(&venue).unwrap().starts_with("[location] Eiffel Tower, Champ de Mars — 48.858400, 2.294500"));
        let live = M::GeoLive(tl::types::MessageMediaGeoLive { geo: geo(1.0, 2.0), heading: None, period: 3600, proximity_notification_radius: None });
        assert!(shared_text(&live).unwrap().contains("[live location] 1.000000, 2.000000 (±15 m) (sharing live for 1 h)"));
        let contact = M::Contact(tl::types::MessageMediaContact {
            phone_number: "491701234567".into(),
            first_name: "Jane".into(),
            last_name: "Doe".into(),
            vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Jane Doe\r\nTEL:+49 170 1234567\r\nEMAIL:jane@example.com\r\nEND:VCARD".into(),
            user_id: 42,
        });
        assert_eq!(
            shared_text(&contact).unwrap(),
            "[contact] Jane Doe, +49 170 1234567, jane@example.com; on Telegram as chat 42"
        );
        let bare = M::Contact(tl::types::MessageMediaContact {
            phone_number: "15551234567".into(),
            first_name: "Bob".into(),
            last_name: String::new(),
            vcard: String::new(),
            user_id: 0,
        });
        assert_eq!(shared_text(&bare).unwrap(), "[contact] Bob, +15551234567");
        assert!(shared_text(&M::Geo(tl::types::MessageMediaGeo { geo: tl::enums::GeoPoint::Empty })).is_none());
    }

    #[test]
    fn live_share_timing() {
        let now = now_secs() as i32;
        assert_eq!(live_until(now - 60, i32::MAX), (None, false), "until I stop");
        let (until, over) = live_until(now - 60, 3600);
        assert_eq!((until, over), (Some((now - 60 + 3600) as u64), false));
        // stopped early: Telegram shortens period to the time it ran
        assert!(live_until(now - 600, 300).1);
    }
}
