//! Contacts: people linked to channels, with a trust level and a preferred language.
//! Persisted under /home/bia/files/state. A channel belongs to one contact (hard link);
//! web and console chats belong to the contact named like the client identity.
//! The channel → language map is mirrored to /home/bia/files/speech/contacts.json for
//! the speech MCPs (calls and voice notes start in the person's language).
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Contact {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub identifier: String,
    #[serde(default = "normal_trust")]
    pub trust: String,
    #[serde(default)]
    pub channels: Vec<String>,
    #[serde(default)]
    pub notes: String,
    /// Preferred language (en, ru, …): set explicitly, or learned from their speech.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub lang: String,
    /// `lang` was set explicitly (never overridden by what is heard).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub lang_set: bool,
    /// A different language heard once; it replaces `lang` when heard again.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub lang_pending: String,
}

/// Channels that belong to the client identity rather than to a stored link.
pub fn local_channel(channel: &str) -> bool {
    channel == "console" || channel == "web" || channel.starts_with("web:")
}


fn normal_trust() -> String {
    "normal".into()
}

#[derive(Serialize, Deserialize, Default)]
struct File {
    #[serde(default)]
    contacts: Vec<Contact>,
}

pub struct AddressBook {
    path: PathBuf,
    contacts: Vec<Contact>,
}

impl AddressBook {
    pub fn load(path: &Path) -> AddressBook {
        let contacts = std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice::<File>(&b).ok())
            .map(|f| f.contacts)
            .unwrap_or_default();
        AddressBook {
            path: path.to_path_buf(),
            contacts,
        }
    }

    fn save(&self) -> Result<()> {
        if let Some(p) = self.path.parent() {
            std::fs::create_dir_all(p)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        {
            use std::io::Write;
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(&serde_json::to_vec_pretty(&File {
                contacts: self.contacts.clone(),
            })?)?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, &self.path)
            .or_else(|_| std::fs::copy(&tmp, &self.path).map(|_| ()))?;
        let _ = std::fs::remove_file(&tmp);
        self.export_langs();
        unsafe { libc::sync() };
        Ok(())
    }

    /// files/speech/contacts.json: {"channels": {channel: lang}, "identities": {id: lang}}.
    fn export_langs(&self) {
        let Some(files) = self.path.parent().and_then(|p| p.parent()) else { return };
        let mut channels = serde_json::Map::new();
        let mut identities = serde_json::Map::new();
        for c in self.contacts.iter().filter(|c| !c.lang.is_empty()) {
            identities.insert(c.id.clone(), c.lang.clone().into());
            for ch in &c.channels {
                channels.insert(ch.clone(), c.lang.clone().into());
            }
        }
        let dir = files.join("speech");
        let _ = std::fs::create_dir_all(&dir);
        let out = dir.join("contacts.json");
        let tmp = out.with_extension("json.tmp");
        let body = serde_json::json!({"channels": channels, "identities": identities});
        if std::fs::write(&tmp, serde_json::to_vec_pretty(&body).unwrap_or_default()).is_ok() {
            let _ = std::fs::rename(&tmp, &out);
        }
    }

    /// The contact behind an envelope: web/console by identity, others by channel.
    pub fn find_for(&self, identity: &str, channel: &str) -> Option<&Contact> {
        if local_channel(channel) {
            return self.contacts.iter().find(|c| c.id == identity);
        }
        self.contacts.iter().find(|c| c.channels.iter().any(|ch| ch == channel))
    }

    /// Speech in `code` was heard from this person. The first language heard is
    /// adopted; a different one replaces it when heard twice in a row. An explicitly
    /// set language is kept. Returns the new language when it changed.
    pub fn observe_lang(&mut self, identity: &str, channel: &str, code: &str) -> Result<Option<String>> {
        let code = code.trim().trim_end_matches('?');
        if code.is_empty() {
            return Ok(None);
        }
        let idx = if local_channel(channel) {
            match self.contacts.iter().position(|c| c.id == identity) {
                Some(i) => i,
                None => {
                    self.contacts.push(Contact {
                        id: identity.to_string(),
                        name: identity.to_string(),
                        identifier: String::new(),
                        trust: if identity == crate::tools::ID_ROOT { "root".into() } else { "normal".into() },
                        channels: vec![],
                        notes: String::new(),
                        lang: String::new(),
                        lang_set: false,
                        lang_pending: String::new(),
                    });
                    self.contacts.len() - 1
                }
            }
        } else {
            match self.contacts.iter().position(|c| c.channels.iter().any(|ch| ch == channel)) {
                Some(i) => i,
                None => return Ok(None),
            }
        };
        let c = &mut self.contacts[idx];
        let changed = if c.lang_set || c.lang == code {
            c.lang_pending.clear();
            None
        } else if c.lang.is_empty() || c.lang_pending == code {
            c.lang = code.to_string();
            c.lang_pending.clear();
            Some(code.to_string())
        } else {
            c.lang_pending = code.to_string();
            None
        };
        self.save()?;
        Ok(changed)
    }

    /// Hard-link `channel` to contact `id`: it leaves any other contact (an auto-created
    /// one left without channels is merged away: its language carries over).
    pub fn link(&mut self, id: &str, channel: &str) -> Result<Contact> {
        if local_channel(channel) {
            bail!("{channel} belongs to the client identity already (contact id = identity)");
        }
        if !self.contacts.iter().any(|c| c.id == id) {
            bail!("no contact {id}; create it with addressbook_upsert first");
        }
        let mut carried = String::new();
        let mut emptied = Vec::new();
        for c in self.contacts.iter_mut().filter(|c| c.id != id) {
            if c.channels.iter().any(|ch| ch == channel) {
                c.channels.retain(|ch| ch != channel);
                if carried.is_empty() {
                    carried = c.lang.clone();
                }
                // auto-created for this channel (no notes, nothing set by hand)
                if c.channels.is_empty() && c.notes.is_empty() && !c.lang_set && c.trust == "normal" {
                    emptied.push(c.id.clone());
                }
            }
        }
        self.contacts.retain(|c| !emptied.contains(&c.id));
        let c = self.contacts.iter_mut().find(|c| c.id == id).expect("checked");
        if !c.channels.iter().any(|ch| ch == channel) {
            c.channels.push(channel.to_string());
        }
        if c.lang.is_empty() && !carried.is_empty() {
            c.lang = carried;
        }
        let out = c.clone();
        self.save()?;
        Ok(out)
    }

    /// Remove `channel` from whichever contact has it.
    pub fn unlink(&mut self, channel: &str) -> Result<bool> {
        let mut hit = false;
        for c in &mut self.contacts {
            let n = c.channels.len();
            c.channels.retain(|ch| ch != channel);
            hit |= c.channels.len() != n;
        }
        if hit {
            self.save()?;
        }
        Ok(hit)
    }

    pub fn list(&self) -> &[Contact] {
        &self.contacts
    }

    pub fn remember_channel(
        &mut self,
        channel: &str,
        name: &str,
        identifier: &str,
    ) -> Result<Contact> {
        if let Some(c) = self.contacts.iter_mut().find(|c| {
            c.channels.iter().any(|ch| ch == channel)
                || (!identifier.is_empty() && c.identifier == identifier)
        }) {
            if !c.channels.iter().any(|ch| ch == channel) {
                c.channels.push(channel.to_string());
            }
            if c.name.is_empty() && !name.is_empty() {
                c.name = name.to_string();
            }
            let out = c.clone();
            self.save()?;
            return Ok(out);
        }
        let id = if identifier.is_empty() {
            channel.replace(':', "-")
        } else {
            identifier.trim_start_matches('@').to_string()
        };
        let c = Contact {
            id: id.clone(),
            name: if name.is_empty() {
                id.clone()
            } else {
                name.to_string()
            },
            identifier: identifier.to_string(),
            trust: "normal".into(),
            channels: vec![channel.to_string()],
            notes: String::new(),
            lang: String::new(),
            lang_set: false,
            lang_pending: String::new(),
        };
        self.contacts.push(c.clone());
        self.save()?;
        Ok(c)
    }

    pub fn upsert(
        &mut self,
        id: &str,
        name: Option<&str>,
        identifier: Option<&str>,
        trust: Option<&str>,
        channels: Option<Vec<String>>,
        notes: Option<&str>,
        lang: Option<&str>,
    ) -> Result<Contact> {
        let lang = lang.map(|l| l.trim().to_ascii_lowercase());
        if let Some(c) = self.contacts.iter_mut().find(|c| c.id == id) {
            if let Some(l) = &lang {
                c.lang = l.clone();
                c.lang_set = !l.is_empty();
                c.lang_pending.clear();
            }
            if let Some(n) = name {
                c.name = n.to_string();
            }
            if let Some(i) = identifier {
                c.identifier = i.to_string();
            }
            if let Some(t) = trust {
                if t != "normal" && t != "root" {
                    bail!("trust must be normal or root");
                }
                c.trust = t.to_string();
            }
            if let Some(ch) = channels {
                c.channels = ch;
            }
            if let Some(n) = notes {
                c.notes = n.to_string();
            }
            let out = c.clone();
            self.save()?;
            return Ok(out);
        }
        let trust = trust.unwrap_or("normal");
        if trust != "normal" && trust != "root" {
            bail!("trust must be normal or root");
        }
        let c = Contact {
            id: id.to_string(),
            name: name.unwrap_or(id).to_string(),
            identifier: identifier.unwrap_or("").to_string(),
            trust: trust.to_string(),
            channels: channels.unwrap_or_default(),
            notes: notes.unwrap_or("").to_string(),
            lang_set: lang.as_deref().is_some_and(|l| !l.is_empty()),
            lang: lang.unwrap_or_default(),
            lang_pending: String::new(),
        };
        self.contacts.push(c.clone());
        self.save()?;
        Ok(c)
    }

    pub fn remove(&mut self, id: &str) -> Result<bool> {
        let n = self.contacts.len();
        self.contacts.retain(|c| c.id != id);
        if self.contacts.len() == n {
            return Ok(false);
        }
        self.save()?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book() -> (AddressBook, PathBuf) {
        let dir = std::env::temp_dir().join(format!("bia-ab-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("state/addressbook.json");
        (AddressBook::load(&path), dir)
    }

    #[test]
    fn language_is_learned_and_explicit_wins() {
        let (mut b, dir) = book();
        b.remember_channel("telegram:5", "user:5", "user:5").unwrap();
        assert_eq!(b.observe_lang("user:5", "telegram:5", "ru").unwrap(), Some("ru".into()), "first heard is adopted");
        assert_eq!(b.observe_lang("user:5", "telegram:5", "en?").unwrap(), None, "one other utterance is not enough");
        assert_eq!(b.observe_lang("user:5", "telegram:5", "ru").unwrap(), None, "back to ru resets");
        assert_eq!(b.observe_lang("user:5", "telegram:5", "en").unwrap(), None);
        assert_eq!(b.observe_lang("user:5", "telegram:5", "en").unwrap(), Some("en".into()), "twice in a row switches");
        let id = b.find_for("x", "telegram:5").unwrap().id.clone();
        b.upsert(&id, None, None, None, None, None, Some("de")).unwrap();
        assert_eq!(b.observe_lang("user:5", "telegram:5", "fr").unwrap(), None);
        assert_eq!(b.observe_lang("user:5", "telegram:5", "fr").unwrap(), None, "explicit language is kept");
        // web chats belong to the identity's contact
        assert_eq!(b.observe_lang("root", "web:c1", "ru").unwrap(), Some("ru".into()));
        assert_eq!(b.find_for("root", "web").unwrap().lang, "ru");
        let m: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("speech/contacts.json")).unwrap()).unwrap();
        assert_eq!(m["channels"]["telegram:5"], "de");
        assert_eq!(m["identities"]["root"], "ru");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn link_moves_channel_and_merges_stub() {
        let (mut b, dir) = book();
        b.remember_channel("telegram:31362240", "user:31362240", "user:31362240").unwrap();
        b.observe_lang("user:31362240", "telegram:31362240", "ru").unwrap();
        b.upsert("root", Some("ZonD"), None, Some("root"), None, None, None).unwrap();
        let c = b.link("root", "telegram:31362240").unwrap();
        assert_eq!(c.channels, ["telegram:31362240"]);
        assert_eq!(c.lang, "ru", "language carried from the stub");
        assert_eq!(b.list().len(), 1, "the auto-created stub is merged away");
        assert!(b.link("nobody", "telegram:1").is_err());
        assert!(b.link("root", "web:x").is_err(), "web chats are linked by identity");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
