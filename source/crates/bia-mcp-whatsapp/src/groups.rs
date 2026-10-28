//! Groups: create one, add and remove members, leave, invite link.
use crate::client::Wa;
use whatsapp_rust::prelude::*;
use whatsapp_rust::wacore_binary::JidExt;
use crate::client::key_of_jid;
use whatsapp_rust::{
    GroupCreateOptions, GroupDescription, GroupParticipantOptions, GroupSubject, JoinGroupResult,
    MemberAddMode, MembershipApprovalMode, PreviousDescription,
};

/// What happened to each person: (added, not added with why).
fn split(
    results: &[whatsapp_rust::wacore::iq::groups::ParticipantChangeResponse],
    names: &[(Jid, String)],
) -> (Vec<String>, Vec<String>) {
    let name = |j: &Jid| {
        names
            .iter()
            .find(|(x, _)| x.user == j.user)
            .map(|(_, n)| n.clone())
            .unwrap_or_else(|| j.user.to_string())
    };
    let mut ok = vec![];
    let mut bad = vec![];
    for r in results {
        match &r.error {
            None => ok.push(name(&r.jid)),
            Some(e) if e == "403" => bad.push(format!(
                "{}: their privacy settings do not allow adding them; send them the invite link",
                name(&r.jid)
            )),
            Some(e) if e == "409" => ok.push(format!("{} (already a member)", name(&r.jid))),
            Some(e) => bad.push(format!("{}: error {e}", name(&r.jid))),
        }
    }
    (ok, bad)
}

fn report(head: String, added: &[String], failed: &[String], link: Option<String>) -> String {
    let mut out = head;
    if !added.is_empty() {
        out.push_str(&format!("\nadded: {}", added.join(", ")));
    }
    if !failed.is_empty() {
        out.push_str(&format!("\nnot added: {}", failed.join("; ")));
    }
    if let Some(l) = link {
        out.push_str(&format!("\ninvite link: {l}"));
    }
    out
}

/// A chat.whatsapp.com link as whatsapp-rust takes it (scheme added when it was left out).
fn invite_link_url(link: &str) -> Result<String, String> {
    let l = link.trim();
    let l = if l.starts_with("chat.whatsapp.com/") { format!("https://{l}") } else { l.to_string() };
    if l.contains("chat.whatsapp.com/") || l.contains("code=") {
        Ok(l)
    } else if !l.is_empty() && l.chars().all(|c| c.is_ascii_alphanumeric()) {
        Ok(link_url(l))
    } else {
        Err(format!("{link:?} is not a WhatsApp group link (https://chat.whatsapp.com/…)"))
    }
}

fn link_url(code_or_url: String) -> String {
    if code_or_url.starts_with("http") {
        code_or_url
    } else {
        format!("https://chat.whatsapp.com/{code_or_url}")
    }
}

impl Wa {
    /// People (phone numbers, names from whatsapp_search) as JIDs.
    fn member_jids(&self, members: &[String]) -> (Vec<(Jid, String)>, Vec<String>) {
        let mut ok = vec![];
        let mut bad = vec![];
        for m in members {
            match self.resolve_target(m) {
                Ok(t) if t.jid.is_group() => {
                    bad.push(format!("{m}: that is a group, not a person"))
                }
                Ok(t) => ok.push((t.jid, m.clone())),
                Err(e) => bad.push(e),
            }
        }
        (ok, bad)
    }

    fn group_jid(&self, chat: &str) -> Result<Jid, String> {
        let t = self.resolve_target(chat)?;
        if !t.jid.is_group() {
            return Err(format!(
                "{chat} is not a group (group ids look like 1203630…@g.us)"
            ));
        }
        Ok(t.jid)
    }

    pub fn create_group(&self, title: &str, members: &[String]) -> Result<String, String> {
        let title = title.trim();
        if title.is_empty() {
            return Err("title is required".into());
        }
        let client = self.require_ready()?;
        let (people, mut failed) = self.member_jids(members);
        let opts = GroupCreateOptions::builder()
            .subject(title)
            .participants(
                people
                    .iter()
                    .map(|(j, _)| GroupParticipantOptions::new(j.clone()))
                    .collect(),
            )
            .build();
        self.block_on(async {
            let created = client
                .groups()
                .create_group(opts)
                .await
                .map_err(|e| format!("create group: {e}"))?;
            let gid = created.metadata.id.clone();
            let inside: Vec<String> = created
                .metadata
                .participants
                .iter()
                .map(|p| p.jid.user.to_string())
                .collect();
            let mut added = vec![];
            for (j, n) in &people {
                let user = j.user.to_string();
                if inside.iter().any(|u| *u == user) {
                    added.push(n.clone());
                } else {
                    failed.push(format!(
                        "{n}: not added (privacy settings?); send them the invite link"
                    ));
                }
            }
            let link = client
                .groups()
                .get_invite_link(gid.clone(), false)
                .await
                .ok()
                .map(link_url);
            Ok(report(
                format!("created group \"{title}\": chat_id {gid} (whatsapp:{gid})"),
                &added,
                &failed,
                link,
            ))
        })
    }

    pub fn add_members(&self, chat: &str, members: &[String]) -> Result<String, String> {
        if members.is_empty() {
            return Err("members is empty".into());
        }
        let client = self.require_ready()?;
        let gid = self.group_jid(chat)?;
        let (people, mut failed) = self.member_jids(members);
        let jids: Vec<Jid> = people.iter().map(|(j, _)| j.clone()).collect();
        self.block_on(async {
            let mut added = vec![];
            if !jids.is_empty() {
                let r = client
                    .groups()
                    .add_participants(gid.clone(), &jids)
                    .await
                    .map_err(|e| format!("add: {e}"))?;
                let (ok, bad) = split(&r, &people);
                added = ok;
                failed.extend(bad);
            }
            let link = if failed.is_empty() {
                None
            } else {
                client
                    .groups()
                    .get_invite_link(gid.clone(), false)
                    .await
                    .ok()
                    .map(link_url)
            };
            Ok(report(format!("group {gid}"), &added, &failed, link))
        })
    }

    pub fn remove_member(&self, chat: &str, member: &str) -> Result<String, String> {
        let client = self.require_ready()?;
        let gid = self.group_jid(chat)?;
        let (people, failed) = self.member_jids(&[member.to_string()]);
        let Some((jid, _)) = people.first().cloned() else {
            return Err(failed.join("; "));
        };
        self.block_on(async {
            let r = client
                .groups()
                .remove_participants(gid.clone(), &[jid])
                .await
                .map_err(|e| format!("remove: {e}"))?;
            match r.first().and_then(|x| x.error.clone()) {
                None => Ok(format!("removed {member} from {gid}")),
                Some(e) => Err(format!(
                    "remove {member}: error {e} (only group admins can remove people)"
                )),
            }
        })
    }

    pub fn leave_group(&self, chat: &str) -> Result<String, String> {
        let client = self.require_ready()?;
        let gid = self.group_jid(chat)?;
        self.block_on(async {
            client
                .groups()
                .leave(gid.clone())
                .await
                .map_err(|e| format!("leave: {e}"))?;
            Ok(format!("left {gid}"))
        })
    }

    /// Join a group by its invite link (chat.whatsapp.com/…).
    pub fn join_group(&self, link: &str) -> Result<String, String> {
        let client = self.require_ready()?;
        let code = invite_link_url(link)?;
        self.block_on(async {
            // the title first: the join answers with the group's id only
            let title = client.groups().get_invite_info(&code).await.ok().map(|m| m.subject);
            let named = |jid: &Jid| match &title {
                Some(t) if !t.is_empty() => format!("\"{t}\" ({jid})"),
                _ => jid.to_string(),
            };
            match client.groups().join_with_invite_code(&code).await {
                Ok(JoinGroupResult::Joined(jid)) => Ok(format!("joined the group {}: chat_id {jid}; bia can write there now", named(&jid))),
                Ok(JoinGroupResult::PendingApproval(jid)) => Ok(format!(
                    "asked to join the group {}: an admin has to approve it; once approved its messages start arriving",
                    named(&jid)
                )),
                Err(e) => {
                    let m = e.to_string();
                    let l = m.to_ascii_lowercase();
                    // server errors print as code=NNN, text='…'
                    if l.contains("code=409") {
                        Ok(format!("already a member of that group{}", title.map(|t| format!(" \"{t}\"")).unwrap_or_default()))
                    } else if l.contains("code=410") || l.contains("code=406") || l.contains("code=404") {
                        Err("this invite link was reset or is wrong; ask for a new one".into())
                    } else if l.contains("code=401") || l.contains("code=403") {
                        Err("this account was removed from that group and cannot join it with a link".into())
                    } else {
                        Err(format!("join: {m}"))
                    }
                }
            }
        })
    }

    pub fn group_invite_link(&self, chat: &str) -> Result<String, String> {
        let client = self.require_ready()?;
        let gid = self.group_jid(chat)?;
        self.block_on(async {
            client
                .groups()
                .get_invite_link(gid, false)
                .await
                .map(link_url)
                .map_err(|e| format!("invite link: {e} (only group admins can make one)"))
        })
    }
}

fn secs_text(s: u32) -> String {
    match s {
        0 => "off".into(),
        s if s % 86400 == 0 => format!("{} day(s)", s / 86400),
        s if s % 3600 == 0 => format!("{} hour(s)", s / 3600),
        s => format!("{s}s"),
    }
}

/// The admin settings of a group, as one call each (only what is passed changes).
pub struct GroupSettings {
    pub only_admins_send: Option<bool>,
    pub only_admins_edit_info: Option<bool>,
    pub only_admins_add: Option<bool>,
    pub join_approval: Option<bool>,
    pub disappearing_seconds: Option<u32>,
}

impl Wa {
    fn member_label(&self, j: &Jid, pn: Option<&Jid>) -> String {
        let key = key_of_jid(pn.unwrap_or(j));
        let title = self.title_for(&key);
        if title.is_empty() || title == key || title == format!("+{key}") {
            format!("+{key}")
        } else {
            format!("{title} ({key})")
        }
    }

    pub fn group_info(&self, chat: &str) -> Result<String, String> {
        let client = self.require_ready()?;
        let gid = self.group_jid(chat)?;
        let me = client.pn().map(|j| j.user.to_string()).unwrap_or_default();
        self.block_on(async {
            let m = client.groups().get_metadata(&gid).await.map_err(|e| format!("group info: {e}"))?;
            let mut out = vec![format!("group {gid} \"{}\"", m.subject)];
            out.push(format!("members: {}", m.size.map(|n| n as usize).unwrap_or(m.participants.len())));
            if let Some(d) = m.description.as_deref().filter(|d| !d.trim().is_empty()) {
                out.push(format!("description: {}", d.trim()));
            }
            let add = match m.member_add_mode {
                Some(MemberAddMode::AdminAdd) => "admins only",
                Some(MemberAddMode::AllMemberAdd) => "everyone",
                None => "unknown",
            };
            out.push(format!(
                "who may send: {}; who may edit the group info: {}; who may add members: {add}",
                if m.is_announcement { "admins only" } else { "everyone" },
                if m.is_locked { "admins only" } else { "everyone" },
            ));
            let eph = m.ephemeral.as_ref().and_then(|e| e.expiration).unwrap_or(0);
            out.push(format!(
                "join approval: {}; disappearing messages: {}",
                if m.membership_approval { "on" } else { "off" },
                secs_text(eph)
            ));
            let mut admins = vec![];
            let mut i_am_admin = false;
            for p in &m.participants {
                if p.is_admin() || p.is_super_admin() {
                    let user = p.phone_number.as_ref().unwrap_or(&p.jid).user.to_string();
                    if user == me {
                        i_am_admin = true;
                    }
                    let role = if p.is_super_admin() { "owner" } else { "admin" };
                    admins.push(format!("{} — {role}", self.member_label(&p.jid, p.phone_number.as_ref())));
                }
            }
            if !admins.is_empty() {
                out.push(format!("admins:\n  {}", admins.join("\n  ")));
            }
            if m.membership_approval {
                if let Ok(reqs) = client.groups().get_membership_requests(gid.clone()).await {
                    if !reqs.is_empty() {
                        out.push(format!("pending join requests: {} (whatsapp_join_requests)", reqs.len()));
                    }
                }
            }
            out.push(if i_am_admin { "you are an admin".into() } else { "you are not an admin here (you cannot change these settings)".into() });
            Ok(out.join("\n"))
        })
    }

    pub fn set_admin(&self, chat: &str, member: &str, admin: bool) -> Result<String, String> {
        let client = self.require_ready()?;
        let gid = self.group_jid(chat)?;
        let (people, failed) = self.member_jids(&[member.to_string()]);
        let Some((jid, _)) = people.first().cloned() else {
            return Err(failed.join("; "));
        };
        self.block_on(async {
            let g = client.groups();
            let r = if admin { g.promote_participants(gid.clone(), &[jid]).await } else { g.demote_participants(gid.clone(), &[jid]).await }
                .map_err(|e| format!("set admin: {e} (only group admins can do this)"))?;
            match r.first().and_then(|x| x.error.clone()) {
                None => Ok(format!("{member} {} {gid}", if admin { "is now an admin of" } else { "is no longer an admin of" })),
                Some(e) if e == "404" => Err(format!("{member} is not a member of {gid}")),
                Some(e) => Err(format!("set admin {member}: error {e} (only group admins can do this)")),
            }
        })
    }

    pub fn group_permissions(&self, chat: &str, s: &GroupSettings) -> Result<String, String> {
        let client = self.require_ready()?;
        let gid = self.group_jid(chat)?;
        self.block_on(async {
            let g = client.groups();
            let fail = |what: &str, e: whatsapp_rust::GroupError| format!("{what}: {e} (only group admins can change it)");
            let mut done = vec![];
            if let Some(on) = s.only_admins_send {
                g.set_announce(gid.clone(), on).await.map_err(|e| fail("who may send", e))?;
                done.push(format!("{} may send", if on { "only admins" } else { "everyone" }));
            }
            if let Some(on) = s.only_admins_edit_info {
                g.set_locked(gid.clone(), on).await.map_err(|e| fail("who may edit info", e))?;
                done.push(format!("{} may edit the group info", if on { "only admins" } else { "everyone" }));
            }
            if let Some(on) = s.only_admins_add {
                let mode = if on { MemberAddMode::AdminAdd } else { MemberAddMode::AllMemberAdd };
                g.set_member_add_mode(gid.clone(), mode).await.map_err(|e| fail("who may add members", e))?;
                done.push(format!("{} may add members", if on { "only admins" } else { "everyone" }));
            }
            if let Some(on) = s.join_approval {
                let mode = if on { MembershipApprovalMode::On } else { MembershipApprovalMode::Off };
                g.set_membership_approval(gid.clone(), mode).await.map_err(|e| fail("join approval", e))?;
                done.push(format!("join approval {}", if on { "on" } else { "off" }));
            }
            if let Some(secs) = s.disappearing_seconds {
                let allowed = [0, 86400, 604800, 7776000];
                if !allowed.contains(&secs) {
                    return Err(format!("disappearing_seconds must be 0 (off), 86400 (a day), 604800 (a week) or 7776000 (90 days); nothing after this was changed (done: {})", if done.is_empty() { "nothing".into() } else { done.join("; ") }));
                }
                g.set_ephemeral(gid.clone(), secs).await.map_err(|e| fail("disappearing messages", e))?;
                done.push(format!("disappearing messages {}", secs_text(secs)));
            }
            if done.is_empty() {
                return Err("nothing to change: pass one of the settings".into());
            }
            Ok(format!("{gid}: {}", done.join("; ")))
        })
    }

    pub fn set_group_info(&self, chat: &str, title: Option<&str>, description: Option<&str>, photo: Option<&str>) -> Result<String, String> {
        if title.is_none() && description.is_none() && photo.is_none() {
            return Err("pass title, description and/or photo".into());
        }
        let client = self.require_ready()?;
        let gid = self.group_jid(chat)?;
        let jpeg = match photo {
            Some(p) => {
                let abs = crate::client::resolve_file_path(&crate::client::files_dir(), p)?;
                Some(crate::client::profile_jpeg(&abs)?)
            }
            None => None,
        };
        self.block_on(async {
            let g = client.groups();
            let mut done = vec![];
            if let Some(t) = title.map(str::trim).filter(|t| !t.is_empty()) {
                let subject = GroupSubject::new(t).map_err(|e| format!("title: {e}"))?;
                g.set_subject(gid.clone(), subject).await.map_err(|e| format!("title: {e} (only admins, or everyone when the info is not locked)"))?;
                self.note_chat(&key_of_jid(&gid), &gid, Some(t), 0);
                done.push(format!("title \"{t}\""));
            }
            if let Some(d) = description {
                let desc = if d.trim().is_empty() { None } else { Some(GroupDescription::new(d.trim()).map_err(|e| format!("description: {e}"))?) };
                g.set_description(gid.clone(), desc, PreviousDescription::Resolve).await.map_err(|e| format!("description: {e}"))?;
                done.push(if d.trim().is_empty() { "description cleared".to_string() } else { "description".to_string() });
            }
            if let Some(b) = jpeg {
                g.set_profile_picture(gid.clone(), b).await.map_err(|e| format!("photo: {e}"))?;
                done.push("photo".into());
            }
            Ok(format!("{gid}: set {}", done.join(", ")))
        })
    }

    pub fn join_requests(&self, chat: &str) -> Result<String, String> {
        let client = self.require_ready()?;
        let gid = self.group_jid(chat)?;
        self.block_on(async {
            let reqs = client.groups().get_membership_requests(gid.clone()).await.map_err(|e| format!("join requests: {e} (only admins see them)"))?;
            if reqs.is_empty() {
                return Ok(format!("no pending join requests in {gid}"));
            }
            let lines: Vec<String> = reqs
                .iter()
                .map(|r| {
                    let when = r.request_time.map(|t| format!(" asked {}", bia_mcp::share::rfc3339(t as i64))).unwrap_or_default();
                    format!("{}{when}", self.member_label(&r.jid, None))
                })
                .collect();
            Ok(format!("{} join request(s) in {gid} (answer with whatsapp_answer_join_request):\n{}", reqs.len(), lines.join("\n")))
        })
    }

    pub fn answer_join_request(&self, chat: &str, member: &str, approve: bool) -> Result<String, String> {
        let client = self.require_ready()?;
        let gid = self.group_jid(chat)?;
        self.block_on(async {
            // the request may come from a LID: match it against the pending list first
            let reqs = client.groups().get_membership_requests(gid.clone()).await.map_err(|e| format!("join requests: {e}"))?;
            let digits: String = member.chars().filter(|c| c.is_ascii_digit()).collect();
            let jid = reqs
                .iter()
                .map(|r| r.jid.clone())
                .find(|j| !digits.is_empty() && j.user.as_str() == digits)
                .or_else(|| self.member_jids(&[member.to_string()]).0.first().map(|(j, _)| j.clone()))
                .ok_or_else(|| format!("{member} has no pending request in {gid} (whatsapp_join_requests lists them)"))?;
            let g = client.groups();
            let r = if approve { g.approve_membership_requests(gid.clone(), &[jid]).await } else { g.reject_membership_requests(gid.clone(), &[jid]).await }
                .map_err(|e| format!("join request: {e}"))?;
            match r.first().and_then(|x| x.error.clone()) {
                None => Ok(format!("{} {member}'s request to join {gid}", if approve { "approved" } else { "declined" })),
                Some(e) => Err(format!("join request {member}: error {e}")),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn disappearing_texts() {
        assert_eq!(super::secs_text(0), "off");
        assert_eq!(super::secs_text(604800), "7 day(s)");
    }

    #[test]
    fn invite_codes_become_links() {
        assert_eq!(
            super::link_url("AbCd123".into()),
            "https://chat.whatsapp.com/AbCd123"
        );
        assert_eq!(
            super::link_url("https://chat.whatsapp.com/x".into()),
            "https://chat.whatsapp.com/x"
        );
    }
}
