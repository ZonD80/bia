//! Where the web MCP listens, as it publishes it in files/state/web.json. The agent only shows
//! this address (status header, /web, first-boot tour); web chats are the web MCP's bus.
use crate::console::Console;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Default, PartialEq)]
pub struct Link {
    pub port: u16,
    pub urls: Vec<String>,
    pub error: String,
}

fn path(files: &Path) -> PathBuf {
    files.join("state").join("web.json")
}

pub fn read(files: &Path) -> Option<Link> {
    let v: Value = serde_json::from_slice(&std::fs::read(path(files)).ok()?).ok()?;
    Some(Link {
        port: v.get("port").and_then(|p| p.as_u64()).unwrap_or(0) as u16,
        urls: v
            .get("urls")
            .and_then(|u| u.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        error: v
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("")
            .to_string(),
    })
}

/// `web: https://… CA https://…/v1/pki/ca`, or why it is not listening.
pub fn info_line(files: &Path) -> String {
    match read(files) {
        None => "web: not started".into(),
        Some(l) if l.port == 0 => format!(
            "web: not listening{}",
            if l.error.is_empty() {
                String::new()
            } else {
                format!(" ({})", l.error)
            }
        ),
        Some(l) if l.urls.is_empty() => "web: (no network yet)  CA at /v1/pki/ca".into(),
        Some(l) => format!(
            "web: {}  CA {}/v1/pki/ca",
            l.urls.join(" "),
            l.urls[0].trim_end_matches('/')
        ),
    }
}

/// The old web.json of a previous boot must not show a stale address.
pub fn clear(files: &Path) {
    let _ = std::fs::remove_file(path(files));
}

/// Keep the status header's web address in step with web.json.
pub fn watch(con: Arc<Console>, files: PathBuf) {
    let _ = std::thread::Builder::new()
        .name("weblink".into())
        .spawn(move || {
            let mut last = None;
            loop {
                let l = read(&files);
                if l != last {
                    let (port, login) = match &l {
                        Some(l) => (l.port, l.urls.join(" ")),
                        None => (0, String::new()),
                    };
                    con.update_status(|s| {
                        s.web_port = port;
                        s.web_login = login;
                    });
                    last = l;
                }
                std::thread::sleep(Duration::from_secs(2));
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_line_from_web_json() {
        let dir = std::env::temp_dir().join(format!("bia-weblink-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("state")).unwrap();
        assert_eq!(info_line(&dir), "web: not started");
        std::fs::write(
            dir.join("state/web.json"),
            r#"{"port":443,"urls":["https://10.0.0.5/"],"ca":"/v1/pki/ca"}"#,
        )
        .unwrap();
        assert_eq!(
            info_line(&dir),
            "web: https://10.0.0.5/  CA https://10.0.0.5/v1/pki/ca"
        );
        std::fs::write(
            dir.join("state/web.json"),
            r#"{"port":0,"error":"port 443 busy","urls":[]}"#,
        )
        .unwrap();
        assert_eq!(info_line(&dir), "web: not listening (port 443 busy)");
        clear(&dir);
        assert!(read(&dir).is_none());
    }
}
