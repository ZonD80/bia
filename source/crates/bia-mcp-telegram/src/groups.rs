//! Groups: create one (a supergroup), add and remove members, leave, invite link.
//! Basic groups (older small chats) are handled with the messages.* calls.
use crate::client::Tg;
use ferogram::tl;
use ferogram::Client;

/// The chats an Updates carries (the new group is one of them).
fn chats_of(u: &tl::enums::Updates) -> Vec<tl::enums::Chat> {
    match u {
        tl::enums::Updates::Updates(x) => x.chats.clone(),
        tl::enums::Updates::Combined(x) => x.chats.clone(),
        _ => vec![],
    }
}

/// Bot-API style id (-100… for supergroups) the other tools take as chat_id.
fn channel_chat_id(channel_id: i64) -> i64 {
    -(channel_id + 1_000_000_000_000)
}

async fn input_user(client: &Client, target: &str) -> Result<(tl::enums::InputUser, tl::enums::InputPeer), String> {
    let t = target.trim().trim_start_matches("telegram:");
    let peer = client.resolve(t).await.map_err(|e| format!("{target}: {e}"))?;
    let input = client
        .resolve_to_input_peer(&peer)
        .await
        .map_err(|e| format!("{target}: {e}"))?;
    match &input {
        tl::enums::InputPeer::User(u) => Ok((
            tl::enums::InputUser::InputUser(tl::types::InputUser {
                user_id: u.user_id,
                access_hash: u.access_hash,
            }),
            input.clone(),
        )),
        _ => Err(format!("{target} is not a person")),
    }
}

enum Group {
    Super(tl::enums::InputChannel),
    Basic(i64),
}

async fn group(client: &Client, target: &str) -> Result<(Group, tl::enums::InputPeer), String> {
    let t = target.trim().trim_start_matches("telegram:");
    let peer = client.resolve(t).await.map_err(|e| format!("resolve {target}: {e}"))?;
    let input = client
        .resolve_to_input_peer(&peer)
        .await
        .map_err(|e| format!("input peer: {e}"))?;
    match &input {
        tl::enums::InputPeer::Channel(c) => Ok((
            Group::Super(tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                channel_id: c.channel_id,
                access_hash: c.access_hash,
            })),
            input.clone(),
        )),
        tl::enums::InputPeer::Chat(c) => Ok((Group::Basic(c.chat_id), input.clone())),
        _ => Err(format!("{target} is not a group")),
    }
}

fn kicked() -> tl::enums::ChatBannedRights {
    tl::enums::ChatBannedRights::ChatBannedRights(tl::types::ChatBannedRights {
        view_messages: true,
        send_messages: true,
        send_media: true,
        send_stickers: true,
        send_gifs: true,
        send_games: true,
        send_inline: true,
        embed_links: true,
        send_polls: true,
        change_info: true,
        invite_users: true,
        pin_messages: true,
        manage_topics: true,
        send_photos: true,
        send_videos: true,
        send_roundvideos: true,
        send_audios: true,
        send_voices: true,
        send_docs: true,
        send_plain: true,
        edit_rank: true,
        send_reactions: true,
        manage_linked_peers: true,
        until_date: 0,
    })
}

fn no_rights() -> tl::enums::ChatBannedRights {
    tl::enums::ChatBannedRights::ChatBannedRights(tl::types::ChatBannedRights {
        view_messages: false,
        send_messages: false,
        send_media: false,
        send_stickers: false,
        send_gifs: false,
        send_games: false,
        send_inline: false,
        embed_links: false,
        send_polls: false,
        change_info: false,
        invite_users: false,
        pin_messages: false,
        manage_topics: false,
        send_photos: false,
        send_videos: false,
        send_roundvideos: false,
        send_audios: false,
        send_voices: false,
        send_docs: false,
        send_plain: false,
        edit_rank: false,
        send_reactions: false,
        manage_linked_peers: false,
        until_date: 0,
    })
}

/// Invite people into a group; returns (added, not added with why).
async fn invite(client: &Client, g: &Group, members: &[String]) -> (Vec<String>, Vec<String>) {
    let mut users = vec![];
    let mut names = vec![];
    let mut failed = vec![];
    for m in members {
        match input_user(client, m).await {
            Ok((u, _)) => {
                users.push(u);
                names.push(m.clone());
            }
            Err(e) => failed.push(e),
        }
    }
    if users.is_empty() {
        return (vec![], failed);
    }
    let missing: Vec<i64> = match g {
        Group::Super(ch) => {
            let req = tl::functions::channels::InviteToChannel { channel: ch.clone(), users: users.clone() };
            match client.invoke(&req).await {
                Ok(tl::enums::messages::InvitedUsers::InvitedUsers(r)) => r
                    .missing_invitees
                    .iter()
                    .map(|tl::enums::MissingInvitee::MissingInvitee(m)| m.user_id)
                    .collect(),
                Err(e) => {
                    failed.push(format!("invite: {e}"));
                    return (vec![], failed);
                }
            }
        }
        Group::Basic(chat_id) => {
            let mut missing = vec![];
            for (u, n) in users.iter().zip(&names) {
                let req = tl::functions::messages::AddChatUser { chat_id: *chat_id, user_id: u.clone(), fwd_limit: 50 };
                if let Err(e) = client.invoke(&req).await {
                    failed.push(format!("{n}: {e}"));
                    if let tl::enums::InputUser::InputUser(x) = u {
                        missing.push(x.user_id);
                    }
                }
            }
            missing
        }
    };
    let mut added = vec![];
    for (u, n) in users.iter().zip(names) {
        let id = match u {
            tl::enums::InputUser::InputUser(x) => x.user_id,
            _ => 0,
        };
        if missing.contains(&id) {
            if !failed.iter().any(|f| f.starts_with(&format!("{n}:"))) {
                failed.push(format!("{n}: their privacy settings do not allow adding them; send them the invite link"));
            }
        } else {
            added.push(n);
        }
    }
    (added, failed)
}

async fn invite_link(client: &Client, peer: &tl::enums::InputPeer) -> Option<String> {
    let req = tl::functions::messages::ExportChatInvite {
        legacy_revoke_permanent: false,
        request_needed: false,
        peer: peer.clone(),
        expire_date: None,
        usage_limit: None,
        title: None,
        subscription_pricing: None,
    };
    match client.invoke(&req).await {
        Ok(tl::enums::ExportedChatInvite::ChatInviteExported(x)) => Some(x.link),
        _ => None,
    }
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

impl Tg {
    /// A new supergroup with this account as its owner, then the members.
    pub fn create_group(&self, title: &str, members: &[String], about: &str) -> Result<String, String> {
        let title = title.trim();
        if title.is_empty() {
            return Err("title is required".into());
        }
        let client = self.require_ready()?;
        self.block_on(async {
            let req = tl::functions::channels::CreateChannel {
                broadcast: false,
                megagroup: true,
                for_import: false,
                forum: false,
                title: title.to_string(),
                about: about.to_string(),
                geo_point: None,
                address: None,
                ttl_period: None,
            };
            let upd = client.invoke(&req).await.map_err(|e| format!("create group: {e}"))?;
            let ch = chats_of(&upd)
                .into_iter()
                .find_map(|c| match c {
                    tl::enums::Chat::Channel(c) => Some(c),
                    _ => None,
                })
                .ok_or("Telegram did not return the new group")?;
            let input = tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                channel_id: ch.id,
                access_hash: ch.access_hash.unwrap_or(0),
            });
            let peer = tl::enums::InputPeer::Channel(tl::types::InputPeerChannel {
                channel_id: ch.id,
                access_hash: ch.access_hash.unwrap_or(0),
            });
            let g = Group::Super(input);
            let (added, failed) = invite(&client, &g, members).await;
            let link = invite_link(&client, &peer).await;
            let id = channel_chat_id(ch.id);
            Ok(report(
                format!("created group \"{title}\": chat_id {id} (telegram:{id})"),
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
        self.block_on(async {
            let (g, peer) = group(&client, chat).await?;
            let (added, failed) = invite(&client, &g, members).await;
            let link = if failed.is_empty() { None } else { invite_link(&client, &peer).await };
            Ok(report(format!("group {chat}"), &added, &failed, link))
        })
    }

    pub fn remove_member(&self, chat: &str, member: &str) -> Result<String, String> {
        let client = self.require_ready()?;
        self.block_on(async {
            let (g, _) = group(&client, chat).await?;
            let (u, p) = input_user(&client, member).await?;
            match g {
                Group::Super(ch) => {
                    // kick: ban, then lift the ban so they could be added again later
                    let ban = tl::functions::channels::EditBanned { channel: ch.clone(), participant: p.clone(), banned_rights: kicked() };
                    client.invoke(&ban).await.map_err(|e| format!("remove: {e}"))?;
                    let lift = tl::functions::channels::EditBanned { channel: ch, participant: p, banned_rights: no_rights() };
                    let _ = client.invoke(&lift).await;
                }
                Group::Basic(chat_id) => {
                    let req = tl::functions::messages::DeleteChatUser { revoke_history: false, chat_id, user_id: u };
                    client.invoke(&req).await.map_err(|e| format!("remove: {e}"))?;
                }
            }
            Ok(format!("removed {member} from {chat}"))
        })
    }

    pub fn leave_group(&self, chat: &str) -> Result<String, String> {
        let client = self.require_ready()?;
        self.block_on(async {
            let (g, _) = group(&client, chat).await?;
            match g {
                Group::Super(ch) => {
                    let req = tl::functions::channels::LeaveChannel { channel: ch };
                    client.invoke(&req).await.map_err(|e| format!("leave: {e}"))?;
                }
                Group::Basic(chat_id) => {
                    let req = tl::functions::messages::DeleteChatUser {
                        revoke_history: false,
                        chat_id,
                        user_id: tl::enums::InputUser::UserSelf,
                    };
                    client.invoke(&req).await.map_err(|e| format!("leave: {e}"))?;
                }
            }
            Ok(format!("left {chat}"))
        })
    }

    pub fn group_invite_link(&self, chat: &str) -> Result<String, String> {
        let client = self.require_ready()?;
        self.block_on(async {
            let (_, peer) = group(&client, chat).await?;
            invite_link(&client, &peer)
                .await
                .ok_or_else(|| "no invite link (only admins can make one)".to_string())
        })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn supergroup_chat_ids_use_the_minus_100_prefix() {
        assert_eq!(super::channel_chat_id(1_234_567_890), -1_001_234_567_890);
    }
}
