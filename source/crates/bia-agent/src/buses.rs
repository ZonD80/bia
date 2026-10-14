//! Buses (live transports) and channels (conversations the envelope uses).
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const BUS_CONSOLE: &str = "console";
pub const CH_CONSOLE: &str = "console";
pub const BUS_WEB: &str = "web";
pub const CH_WEB: &str = "web";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChannelInfo {
    pub id: String,
    pub bus: String,
    pub title: String,
}

#[derive(Serialize, Deserialize, Default)]
struct File {
    #[serde(default)]
    channels: Vec<ChannelInfo>,
}

pub struct Buses {
    path: PathBuf,
    channels: Vec<ChannelInfo>,
}

fn builtin(id: &str, bus: &str, title: &str) -> ChannelInfo {
    ChannelInfo {
        id: id.into(),
        bus: bus.into(),
        title: title.into(),
    }
}

fn is_builtin(id: &str) -> bool {
    id == CH_CONSOLE || id == CH_WEB
}

impl Buses {
    pub fn load(path: &Path) -> Buses {
        let mut channels = vec![
            builtin(CH_CONSOLE, BUS_CONSOLE, "console (screen, serial)"),
            builtin(CH_WEB, BUS_WEB, "web console (HTTPS)"),
        ];
        if let Ok(b) = std::fs::read(path) {
            if let Ok(f) = serde_json::from_slice::<File>(&b) {
                for c in f.channels {
                    if !is_builtin(&c.id) && !channels.iter().any(|x| x.id == c.id) {
                        channels.push(c);
                    }
                }
            }
        }
        Buses {
            path: path.to_path_buf(),
            channels,
        }
    }

    fn persist(&self) {
        let stored: Vec<ChannelInfo> = self
            .channels
            .iter()
            .filter(|c| !is_builtin(&c.id))
            .cloned()
            .collect();
        if let Some(p) = self.path.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        if let Ok(buf) = serde_json::to_vec_pretty(&File { channels: stored }) {
            let _ = std::fs::write(&self.path, buf);
        }
    }

    pub fn list(&self) -> &[ChannelInfo] {
        &self.channels
    }

    pub fn upsert(&mut self, id: &str, bus: &str, title: &str) {
        if let Some(c) = self.channels.iter_mut().find(|c| c.id == id) {
            if !title.is_empty() {
                c.title = title.to_string();
            }
            c.bus = bus.to_string();
        } else {
            self.channels.push(ChannelInfo {
                id: id.into(),
                bus: bus.into(),
                title: if title.is_empty() {
                    id.into()
                } else {
                    title.into()
                },
            });
        }
        self.persist();
    }

    pub fn ids_csv(&self) -> String {
        self.channels
            .iter()
            .map(|c| c.id.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}
