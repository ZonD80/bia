//! Group administration: what a group allows its members, its admins and their rights,
//! restricting one member, the title / description / photo, and join requests.
//! Supergroups get everything; basic groups (older small chats) have default permissions,
//! admins without separate rights, title, description and photo.
use crate::client::{resolve_file_path, Tg};
use crate::groups::{group, input_user, no_rights, Group};
use ferogram::tl;
use ferogram::Client;

/// What a member may do, by name (each maps to one ChatBannedRights flag).
pub const MEMBER_ACTIONS: &[&str] = &[
    "send_text", "send_photos", "send_videos", "send_video_notes", "send_voices", "send_audios", "send_files",
    "send_stickers", "send_gifs", "send_games", "send_inline", "add_links", "send_polls", "send_reactions",
    "invite_users", "pin_messages", "change_info", "manage_topics",
];

/// Admin rights by name (ChatAdminRights flags).
pub const ADMIN_RIGHTS: &[&str] = &[
    "change_info", "delete_messages", "ban_users", "invite_users", "pin_messages", "manage_call", "manage_topics",
    "post_stories", "edit_stories", "delete_stories", "add_admins", "anonymous",
];

/// Rights a new admin gets when none are named: running the group, not making admins.
const DEFAULT_ADMIN: &[&str] = &[
    "change_info", "delete_messages", "ban_users", "invite_users", "pin_messages", "manage_call", "manage_topics",
];

fn banned_flag<'a>(r: &'a mut tl::types::ChatBannedRights, name: &str) -> Option<&'a mut bool> {
    Some(match name {
        "send_text" => &mut r.send_plain,
        "send_photos" => &mut r.send_photos,
        "send_videos" => &mut r.send_videos,
        "send_video_notes" => &mut r.send_roundvideos,
        "send_voices" => &mut r.send_voices,
        "send_audios" => &mut r.send_audios,
        "send_files" => &mut r.send_docs,
        "send_stickers" => &mut r.send_stickers,
        "send_gifs" => &mut r.send_gifs,
        "send_games" => &mut r.send_games,
        "send_inline" => &mut r.send_inline,
        "add_links" => &mut r.embed_links,
        "send_polls" => &mut r.send_polls,
        "send_reactions" => &mut r.send_reactions,
        "invite_users" => &mut r.invite_users,
        "pin_messages" => &mut r.pin_messages,
        "change_info" => &mut r.change_info,
        "manage_topics" => &mut r.manage_topics,
        _ => return None,
    })
}

/// Expand `send_media` / `send_messages` / `all` into the actions they stand for.
fn expand(names: &[String]) -> Result<Vec<String>, String> {
    let media = ["send_photos", "send_videos", "send_video_notes", "send_voices", "send_audios", "send_files", "send_stickers", "send_gifs"];
    let mut out = Vec::new();
    for n in names {
        let n = n.trim().to_ascii_lowercase().replace([' ', '-'], "_");
        match n.as_str() {
            "" => {}
            "send_media" => out.extend(media.iter().map(|s| s.to_string())),
            "send_messages" | "send" => {
                out.push("send_text".into());
                out.extend(media.iter().map(|s| s.to_string()));
                out.extend(["send_games", "send_inline", "add_links", "send_polls"].map(String::from));
            }
            "all" => out.extend(MEMBER_ACTIONS.iter().map(|s| s.to_string())),
            other if MEMBER_ACTIONS.contains(&other) => out.push(other.to_string()),
            other => return Err(format!("unknown member action {other:?}; known: {}, send_media, send_messages, all", MEMBER_ACTIONS.join(", "))),
        }
    }
    Ok(out)
}

/// Keep Telegram's summary flags consistent with the detailed ones.
fn settle(r: &mut tl::types::ChatBannedRights) {
    r.send_media = r.send_photos && r.send_videos && r.send_roundvideos && r.send_voices && r.send_audios && r.send_docs;
    r.send_messages = r.send_plain && r.send_media && r.send_stickers && r.send_gifs;
}

fn rights_of(r: &tl::enums::ChatBannedRights) -> tl::types::ChatBannedRights {
    let tl::enums::ChatBannedRights::ChatBannedRights(x) = r;
    x.clone()
}

/// (allowed, not allowed) action names.
fn describe_banned(r: &tl::types::ChatBannedRights) -> (Vec<&'static str>, Vec<&'static str>) {
    let mut c = r.clone();
    let (mut can, mut cannot) = (vec![], vec![]);
    for n in MEMBER_ACTIONS {
        if *banned_flag(&mut c, n).unwrap() {
            cannot.push(*n);
        } else {
            can.push(*n);
        }
    }
    (can, cannot)
}

fn admin_rights(names: &[String]) -> Result<tl::types::ChatAdminRights, String> {
    let mut r = tl::types::ChatAdminRights { other: true, ..no_admin() };
    let names: Vec<String> = if names.is_empty() {
        DEFAULT_ADMIN.iter().map(|s| s.to_string()).collect()
    } else if names.iter().any(|n| n.trim() == "all") {
        ADMIN_RIGHTS.iter().filter(|n| **n != "anonymous").map(|s| s.to_string()).collect()
    } else {
        names.to_vec()
    };
    for n in &names {
        let n = n.trim().to_ascii_lowercase().replace([' ', '-'], "_");
        let f = match n.as_str() {
            "change_info" => &mut r.change_info,
            "delete_messages" => &mut r.delete_messages,
            "ban_users" => &mut r.ban_users,
            "invite_users" => &mut r.invite_users,
            "pin_messages" => &mut r.pin_messages,
            "manage_call" => &mut r.manage_call,
            "manage_topics" => &mut r.manage_topics,
            "post_stories" => &mut r.post_stories,
            "edit_stories" => &mut r.edit_stories,
            "delete_stories" => &mut r.delete_stories,
            "add_admins" => &mut r.add_admins,
            "anonymous" => &mut r.anonymous,
            other => return Err(format!("unknown admin right {other:?}; known: {}, all", ADMIN_RIGHTS.join(", "))),
        };
        *f = true;
    }
    Ok(r)
}

/// No admin rights at all (taking admin away).
fn no_admin() -> tl::types::ChatAdminRights {
    tl::types::ChatAdminRights {
        change_info: false, post_messages: false, edit_messages: false, delete_messages: false, ban_users: false,
        invite_users: false, pin_messages: false, add_admins: false, anonymous: false, manage_call: false,
        other: false, manage_topics: false, post_stories: false, edit_stories: false, delete_stories: false,
        manage_direct_messages: false, manage_ranks: false, manage_linked_peers: false,
    }
}

fn admin_names(r: &tl::enums::ChatAdminRights) -> Vec<&'static str> {
    let tl::enums::ChatAdminRights::ChatAdminRights(r) = r;
    let all = [
        ("change_info", r.change_info), ("delete_messages", r.delete_messages), ("ban_users", r.ban_users),
        ("invite_users", r.invite_users), ("pin_messages", r.pin_messages), ("manage_call", r.manage_call),
        ("manage_topics", r.manage_topics), ("post_stories", r.post_stories), ("edit_stories", r.edit_stories),
        ("delete_stories", r.delete_stories), ("add_admins", r.add_admins), ("anonymous", r.anonymous),
    ];
    all.into_iter().filter(|(_, on)| *on).map(|(n, _)| n).collect()
}

fn name_in(users: &[tl::enums::User], id: i64) -> String {
    for u in users {
        if let tl::enums::User::User(u) = u {
            if u.id == id {
                let name = [u.first_name.clone().unwrap_or_default(), u.last_name.clone().unwrap_or_default()]
                    .join(" ")
                    .trim()
                    .to_string();
                return match u.username.as_deref().filter(|s| !s.is_empty()) {
                    Some(un) if !name.is_empty() => format!("{name} (@{un}, {id})"),
                    Some(un) => format!("@{un} ({id})"),
                    None if !name.is_empty() => format!("{name} ({id})"),
                    None => format!("user {id}"),
                };
            }
        }
    }
    format!("user {id}")
}

fn duration(secs: i32) -> String {
    match secs {
        0 => "off".into(),
        s if s % 86400 == 0 => format!("{} day(s)", s / 86400),
        s if s % 3600 == 0 => format!("{} hour(s)", s / 3600),
        s if s % 60 == 0 => format!("{} minute(s)", s / 60),
        s => format!("{s}s"),
    }
}

/// The group as Telegram describes it: (title, full info, chat object, users).
async fn full(client: &Client, g: &Group) -> Result<(tl::enums::ChatFull, Option<tl::enums::Chat>, Vec<tl::enums::User>), String> {
    let r = match g {
        Group::Super(ch) => client.invoke(&tl::functions::channels::GetFullChannel { channel: ch.clone() }).await,
        Group::Basic(id) => client.invoke(&tl::functions::messages::GetFullChat { chat_id: *id }).await,
    }
    .map_err(|e| format!("group info: {e}"))?;
    let tl::enums::messages::ChatFull::ChatFull(r) = r;
    let want = match g {
        Group::Super(tl::enums::InputChannel::InputChannel(c)) => c.channel_id,
        Group::Basic(id) => *id,
        _ => 0,
    };
    let chat = r.chats.into_iter().find(|c| match c {
        tl::enums::Chat::Channel(x) => x.id == want,
        tl::enums::Chat::Chat(x) => x.id == want,
        _ => false,
    });
    Ok((r.full_chat, chat, r.users))
}

fn defaults_of(chat: &Option<tl::enums::Chat>) -> tl::types::ChatBannedRights {
    let d = match chat {
        Some(tl::enums::Chat::Channel(c)) => c.default_banned_rights.clone(),
        Some(tl::enums::Chat::Chat(c)) => c.default_banned_rights.clone(),
        _ => None,
    };
    d.map(|r| rights_of(&r)).unwrap_or_else(|| rights_of(&no_rights()))
}

impl Tg {
    pub fn group_info(&self, chat: &str) -> Result<String, String> {
        let client = self.require_ready()?;
        self.block_on(async {
            let (g, _) = group(&client, chat).await?;
            let (fullc, c, users) = full(&client, &g).await?;
            let mut out = Vec::new();
            let (title, me_creator, my_rights, noforwards, join_request) = match &c {
                Some(tl::enums::Chat::Channel(x)) => (x.title.clone(), x.creator, x.admin_rights.clone(), x.noforwards, x.join_request),
                Some(tl::enums::Chat::Chat(x)) => (x.title.clone(), x.creator, x.admin_rights.clone(), x.noforwards, false),
                _ => (String::new(), false, None, false, false),
            };
            let kind = if matches!(g, Group::Super(_)) { "supergroup" } else { "basic group" };
            out.push(format!("{kind} {chat} \"{title}\""));
            let (about, members, slow, hidden, ttl, pending) = match &fullc {
                tl::enums::ChatFull::ChannelFull(f) => (
                    f.about.clone(), f.participants_count, f.slowmode_seconds.unwrap_or(0), f.hidden_prehistory,
                    f.ttl_period.unwrap_or(0), f.requests_pending.unwrap_or(0),
                ),
                tl::enums::ChatFull::ChatFull(f) => {
                    let n = match &f.participants {
                        tl::enums::ChatParticipants::ChatParticipants(p) => Some(p.participants.len() as i32),
                        _ => None,
                    };
                    (f.about.clone(), n, 0, false, f.ttl_period.unwrap_or(0), f.requests_pending.unwrap_or(0))
                }
                _ => (String::new(), None, 0, false, 0, 0),
            };
            if let Some(n) = members {
                out.push(format!("members: {n}"));
            }
            if !about.trim().is_empty() {
                out.push(format!("description: {}", about.trim()));
            }
            let (can, cannot) = describe_banned(&defaults_of(&c));
            out.push(format!("members can: {}", if can.is_empty() { "nothing".into() } else { can.join(", ") }));
            if !cannot.is_empty() {
                out.push(format!("members cannot: {}", cannot.join(", ")));
            }
            if let Group::Super(_) = g {
                out.push(format!(
                    "slow mode: {}; join approval: {}; new members see earlier messages: {}",
                    duration(slow),
                    if join_request { "on" } else { "off" },
                    if hidden { "no" } else { "yes" }
                ));
            }
            out.push(format!(
                "protected content (no forwarding or saving): {}; disappearing messages: {}",
                if noforwards { "on" } else { "off" },
                duration(ttl)
            ));
            if pending > 0 {
                out.push(format!("pending join requests: {pending} (telegram_join_requests)"));
            }
            // admins
            let mut admins = Vec::new();
            match &g {
                Group::Super(ch) => {
                    let req = tl::functions::channels::GetParticipants {
                        channel: ch.clone(),
                        filter: tl::enums::ChannelParticipantsFilter::ChannelParticipantsAdmins,
                        offset: 0,
                        limit: 100,
                        hash: 0,
                    };
                    if let Ok(tl::enums::channels::ChannelParticipants::ChannelParticipants(p)) = client.invoke(&req).await {
                        for part in &p.participants {
                            match part {
                                tl::enums::ChannelParticipant::Creator(x) => admins.push(format!(
                                    "{} — owner{}",
                                    name_in(&p.users, x.user_id),
                                    x.rank.as_deref().map(|r| format!(", title \"{r}\"")).unwrap_or_default()
                                )),
                                tl::enums::ChannelParticipant::Admin(x) => admins.push(format!(
                                    "{} — admin: {}{}",
                                    name_in(&p.users, x.user_id),
                                    admin_names(&x.admin_rights).join(", "),
                                    x.rank.as_deref().map(|r| format!("; title \"{r}\"")).unwrap_or_default()
                                )),
                                _ => {}
                            }
                        }
                    }
                }
                Group::Basic(_) => {
                    if let tl::enums::ChatFull::ChatFull(f) = &fullc {
                        if let tl::enums::ChatParticipants::ChatParticipants(p) = &f.participants {
                            for part in &p.participants {
                                match part {
                                    tl::enums::ChatParticipant::Creator(x) => admins.push(format!("{} — owner", name_in(&users, x.user_id))),
                                    tl::enums::ChatParticipant::Admin(x) => admins.push(format!("{} — admin", name_in(&users, x.user_id))),
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
            if !admins.is_empty() {
                out.push(format!("admins:\n  {}", admins.join("\n  ")));
            }
            out.push(if me_creator {
                "you are the owner".to_string()
            } else if let Some(r) = &my_rights {
                format!("your admin rights: {}", admin_names(r).join(", "))
            } else {
                "you are not an admin here (you cannot change these settings)".to_string()
            });
            Ok(out.join("\n"))
        })
    }

    /// Make someone an admin (with rights and a title) or take it back.
    pub fn set_admin(&self, chat: &str, member: &str, admin: bool, rights: &[String], title: &str) -> Result<String, String> {
        let client = self.require_ready()?;
        self.block_on(async {
            let (g, _) = group(&client, chat).await?;
            let (u, _) = input_user(&client, member).await?;
            match g {
                Group::Super(ch) => {
                    let r = if admin { admin_rights(rights)? } else { no_admin() };
                    let shown = admin_names(&tl::enums::ChatAdminRights::ChatAdminRights(r.clone())).join(", ");
                    let req = tl::functions::channels::EditAdmin {
                        channel: ch,
                        user_id: u,
                        admin_rights: tl::enums::ChatAdminRights::ChatAdminRights(r),
                        rank: (admin && !title.trim().is_empty()).then(|| title.trim().to_string()),
                    };
                    client.invoke(&req).await.map_err(|e| format!("set admin: {e}"))?;
                    Ok(if admin {
                        format!("{member} is an admin of {chat} with: {shown}{}", if title.trim().is_empty() { String::new() } else { format!("; title \"{}\"", title.trim()) })
                    } else {
                        format!("{member} is no longer an admin of {chat}")
                    })
                }
                Group::Basic(chat_id) => {
                    let req = tl::functions::messages::EditChatAdmin { chat_id, user_id: u, is_admin: admin };
                    client.invoke(&req).await.map_err(|e| format!("set admin: {e}"))?;
                    let note = if admin && (!rights.is_empty() || !title.is_empty()) {
                        " (a basic group has no separate admin rights or titles: every admin may do everything)"
                    } else {
                        ""
                    };
                    Ok(format!("{member} {} {chat}{note}", if admin { "is an admin of" } else { "is no longer an admin of" }))
                }
            }
        })
    }

    /// What every member may do (allow/deny on top of the current settings), plus slow mode,
    /// join approval, history for new members, protected content and disappearing messages.
    #[allow(clippy::too_many_arguments)]
    pub fn group_permissions(
        &self,
        chat: &str,
        allow: &[String],
        deny: &[String],
        slow_mode: Option<i32>,
        join_approval: Option<bool>,
        history_for_new: Option<bool>,
        protect_content: Option<bool>,
        disappearing: Option<i32>,
    ) -> Result<String, String> {
        let client = self.require_ready()?;
        self.block_on(async {
            let (g, peer) = group(&client, chat).await?;
            let mut done = Vec::new();
            if !allow.is_empty() || !deny.is_empty() {
                let (_, c, _) = full(&client, &g).await?;
                let mut r = defaults_of(&c);
                for n in expand(allow)? {
                    *banned_flag(&mut r, &n).unwrap() = false;
                }
                for n in expand(deny)? {
                    *banned_flag(&mut r, &n).unwrap() = true;
                }
                r.view_messages = false;
                r.until_date = 0;
                settle(&mut r);
                let req = tl::functions::messages::EditChatDefaultBannedRights {
                    peer: peer.clone(),
                    banned_rights: tl::enums::ChatBannedRights::ChatBannedRights(r.clone()),
                };
                match client.invoke(&req).await {
                    Ok(_) => {}
                    Err(e) if e.to_string().contains("NOT_MODIFIED") => {}
                    Err(e) => return Err(format!("permissions: {e}")),
                }
                let (can, _) = describe_banned(&r);
                done.push(format!("members can now: {}", if can.is_empty() { "nothing".into() } else { can.join(", ") }));
            }
            let need_super = |what: &str| -> Result<tl::enums::InputChannel, String> {
                match &g {
                    Group::Super(ch) => Ok(ch.clone()),
                    Group::Basic(_) => Err(format!("{what} works only in supergroups (this is a basic group)")),
                }
            };
            if let Some(s) = slow_mode {
                let allowed = [0, 10, 30, 60, 300, 900, 3600];
                if !allowed.contains(&s) {
                    return Err(format!("slow_mode_seconds must be one of {allowed:?}"));
                }
                let channel = need_super("slow mode")?;
                client.invoke(&tl::functions::channels::ToggleSlowMode { channel, seconds: s }).await.map_err(|e| format!("slow mode: {e}"))?;
                done.push(format!("slow mode {}", duration(s)));
            }
            if let Some(on) = join_approval {
                let channel = need_super("join approval")?;
                let req = tl::functions::channels::ToggleJoinRequest { apply_to_invites: false, channel, enabled: on, guard_bot: None };
                client.invoke(&req).await.map_err(|e| format!("join approval: {e}"))?;
                done.push(format!("join approval {}", if on { "on" } else { "off" }));
            }
            if let Some(see) = history_for_new {
                let channel = need_super("history for new members")?;
                client.invoke(&tl::functions::channels::TogglePreHistoryHidden { channel, enabled: !see }).await.map_err(|e| format!("history: {e}"))?;
                done.push(format!("new members {} earlier messages", if see { "see" } else { "do not see" }));
            }
            if let Some(on) = protect_content {
                let req = tl::functions::messages::ToggleNoForwards { peer: peer.clone(), enabled: on, request_msg_id: None };
                client.invoke(&req).await.map_err(|e| format!("protected content: {e}"))?;
                done.push(format!("protected content {}", if on { "on" } else { "off" }));
            }
            if let Some(secs) = disappearing {
                let req = tl::functions::messages::SetHistoryTtl { peer: peer.clone(), period: secs.max(0) };
                client.invoke(&req).await.map_err(|e| format!("disappearing messages: {e}"))?;
                done.push(format!("disappearing messages {}", duration(secs.max(0))));
            }
            if done.is_empty() {
                return Err("nothing to change: pass allow, deny or one of the settings".into());
            }
            Ok(format!("{chat}: {}", done.join("; ")))
        })
    }

    /// Restrict one member of a supergroup (deny actions, optionally for a while), ban them
    /// from it (deny=["all"]), or lift it all (deny empty).
    pub fn restrict_member(&self, chat: &str, member: &str, deny: &[String], seconds: i64) -> Result<String, String> {
        let client = self.require_ready()?;
        self.block_on(async {
            let (g, _) = group(&client, chat).await?;
            let Group::Super(ch) = g else {
                return Err("restricting one member works only in supergroups; in a basic group use telegram_remove_member".into());
            };
            let (_, p) = input_user(&client, member).await?;
            let until = if seconds > 0 { (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0) + seconds) as i32 } else { 0 };
            let ban = deny.iter().any(|d| matches!(d.trim(), "all" | "ban" | "view_messages"));
            let mut r = rights_of(&no_rights());
            if ban {
                r = rights_of(&crate::groups::kicked());
            } else {
                for n in expand(deny)? {
                    *banned_flag(&mut r, &n).unwrap() = true;
                }
                settle(&mut r);
            }
            r.until_date = until;
            let req = tl::functions::channels::EditBanned { channel: ch, participant: p, banned_rights: tl::enums::ChatBannedRights::ChatBannedRights(r.clone()) };
            client.invoke(&req).await.map_err(|e| format!("restrict: {e}"))?;
            let when = if seconds > 0 { format!(" for {}", duration(seconds.min(i32::MAX as i64) as i32)) } else { String::new() };
            Ok(if ban {
                format!("{member} is banned from {chat}{when} (they cannot rejoin until lifted: restrict_member with deny=[])")
            } else if deny.is_empty() {
                format!("{member} has no restrictions in {chat} now")
            } else {
                let (_, cannot) = describe_banned(&r);
                format!("{member} in {chat} cannot: {}{when}", cannot.join(", "))
            })
        })
    }

    /// Title, description and photo of a group (only what is passed changes).
    pub fn set_group_info(&self, chat: &str, title: Option<&str>, description: Option<&str>, photo: Option<&str>) -> Result<String, String> {
        if title.is_none() && description.is_none() && photo.is_none() {
            return Err("pass title, description and/or photo".into());
        }
        let client = self.require_ready()?;
        let photo = match photo {
            Some(p) => Some(resolve_file_path(&Tg::files_dir(), p)?),
            None => None,
        };
        self.block_on(async {
            let (_, peer) = group(&client, chat).await?;
            let target = chat.trim().trim_start_matches("telegram:");
            let mut b = client.set_profile(target);
            let mut what = Vec::new();
            if let Some(t) = title.map(str::trim).filter(|t| !t.is_empty()) {
                b = b.title(t);
                what.push(format!("title \"{t}\""));
            }
            if let Some(d) = description {
                // an empty description clears it; the builder skips empty strings, so send it directly
                if d.trim().is_empty() {
                    let req = tl::functions::messages::EditChatAbout { peer: peer.clone(), about: String::new() };
                    client.invoke(&req).await.map_err(|e| format!("description: {e}"))?;
                    what.push("description cleared".into());
                } else {
                    b = b.about(d.trim());
                    what.push("description".into());
                }
            }
            if let Some(p) = &photo {
                b = b.photo_path(p.clone());
                what.push(format!("photo from {}", p.display()));
            }
            b.send().await.map_err(|e| format!("group info: {e}"))?;
            Ok(format!("{chat}: set {}", what.join(", ")))
        })
    }

    pub fn join_requests(&self, chat: &str) -> Result<String, String> {
        let client = self.require_ready()?;
        self.block_on(async {
            let (_, peer) = group(&client, chat).await?;
            let req = tl::functions::messages::GetChatInviteImporters {
                requested: true,
                subscription_expired: false,
                peer,
                link: None,
                q: None,
                offset_date: 0,
                offset_user: tl::enums::InputUser::Empty,
                limit: 100,
            };
            let tl::enums::messages::ChatInviteImporters::ChatInviteImporters(r) =
                client.invoke(&req).await.map_err(|e| format!("join requests: {e}"))?;
            if r.importers.is_empty() {
                return Ok(format!("no pending join requests in {chat}"));
            }
            let lines: Vec<String> = r
                .importers
                .iter()
                .map(|tl::enums::ChatInviteImporter::ChatInviteImporter(i)| {
                    let about = i.about.as_deref().filter(|a| !a.is_empty()).map(|a| format!(" — \"{a}\"")).unwrap_or_default();
                    format!("{} asked {}{about}", name_in(&r.users, i.user_id), bia_mcp::share::rfc3339(i.date as i64))
                })
                .collect();
            Ok(format!("{} join request(s) in {chat} (answer with telegram_answer_join_request):\n{}", r.count, lines.join("\n")))
        })
    }

    pub fn answer_join_request(&self, chat: &str, member: &str, approve: bool) -> Result<String, String> {
        let client = self.require_ready()?;
        self.block_on(async {
            let (_, peer) = group(&client, chat).await?;
            let (u, _) = input_user(&client, member).await?;
            let req = tl::functions::messages::HideChatJoinRequest { approved: approve, peer, user_id: u };
            client.invoke(&req).await.map_err(|e| format!("join request: {e}"))?;
            Ok(format!("{} {member}'s request to join {chat}", if approve { "approved" } else { "declined" }))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn member_actions_expand_and_map() {
        let all = expand(&["all".into()]).unwrap();
        assert_eq!(all.len(), MEMBER_ACTIONS.len());
        let media = expand(&["send media".into()]).unwrap();
        assert!(media.contains(&"send_photos".to_string()) && !media.contains(&"send_text".to_string()));
        assert!(expand(&["fly".into()]).is_err());
        let mut r = rights_of(&no_rights());
        for n in MEMBER_ACTIONS {
            assert!(banned_flag(&mut r, n).is_some(), "{n} maps to a flag");
        }
        for n in expand(&["send_messages".into()]).unwrap() {
            *banned_flag(&mut r, &n).unwrap() = true;
        }
        settle(&mut r);
        assert!(r.send_messages && r.send_media && !r.send_reactions);
        let (can, cannot) = describe_banned(&r);
        assert!(can.contains(&"send_reactions") && cannot.contains(&"send_text"));
    }

    #[test]
    fn admin_rights_by_name() {
        let d = admin_rights(&[]).unwrap();
        assert!(d.ban_users && d.delete_messages && !d.add_admins && !d.anonymous);
        let a = admin_rights(&["all".into()]).unwrap();
        assert!(a.add_admins && !a.anonymous);
        let p = admin_rights(&["pin_messages".into()]).unwrap();
        assert_eq!(admin_names(&tl::enums::ChatAdminRights::ChatAdminRights(p)), vec!["pin_messages"]);
        assert!(admin_rights(&["rule_world".into()]).is_err());
        assert_eq!(duration(86400), "1 day(s)");
        assert_eq!(duration(0), "off");
    }
}
