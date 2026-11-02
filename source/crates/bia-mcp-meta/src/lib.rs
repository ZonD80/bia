//! The messenger, instagram and max MCPs: bia's side of an account. The protocol runs in a
//! helper, one process per account (`bia-meta` in Go on mautrix-meta; `bia_max.py` on PyMax),
//! speaking the same JSON lines; this crate starts it, turns what it hears into messages for
//! the model (in order per chat, with files downloaded and voice notes transcribed), and offers
//! the tools.

pub mod audio_out;
pub mod calls;
pub mod helper;
pub mod inbound;
pub mod qr_win;
pub mod state;
pub mod tools;
pub mod tts;

/// Which app this MCP is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Messenger,
    Instagram,
    Max,
}

impl Platform {
    /// Tool prefix, channel prefix, files/<name> folder.
    pub fn name(self) -> &'static str {
        match self {
            Platform::Messenger => "messenger",
            Platform::Instagram => "instagram",
            Platform::Max => "max",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Platform::Messenger => "Messenger",
            Platform::Instagram => "Instagram",
            Platform::Max => "MAX",
        }
    }

    pub fn channel(self, chat: &str) -> String {
        format!("{}:{chat}", self.name())
    }

    /// Strip `messenger:` / `instagram:` from a chat id the model passes.
    pub fn chat_id(self, raw: &str) -> String {
        let t = raw.trim();
        let pre = format!("{}:", self.name());
        t.strip_prefix(&pre).unwrap_or(t).trim().to_string()
    }

    /// The helper's name, for messages about it.
    pub fn helper(self) -> &'static str {
        match self {
            Platform::Max => "bia_max.py",
            _ => "bia-meta",
        }
    }

    /// What a chat_id looks like, for tool descriptions.
    pub fn chat_hint(self) -> &'static str {
        match self {
            Platform::Messenger => "a chat id from messenger_chats or messenger_search: a person's Facebook id (digits), t<digits> for a group, <digits>@g.us for an encrypted group; a name or profile link also works",
            Platform::Instagram => "a chat id from instagram_chats or instagram_search (digits), or @username / a profile link",
            Platform::Max => "a chat id from max_chats, max_search or max_resolve (a number, negative for most groups)",
        }
    }
}

/// Run the MCP for `p` until stdin closes.
pub fn run(p: Platform) {
    tools::serve(p);
}
