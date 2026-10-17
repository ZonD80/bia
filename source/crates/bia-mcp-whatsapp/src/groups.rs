//! Groups: create one, add and remove members, leave, invite link.
use crate::client::Wa;
use whatsapp_rust::prelude::*;
use whatsapp_rust::wacore_binary::JidExt;
use whatsapp_rust::{GroupCreateOptions, GroupParticipantOptions};

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

#[cfg(test)]
mod tests {
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
