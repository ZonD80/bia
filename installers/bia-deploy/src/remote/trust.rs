//! Hand the new bia's key to this computer's browser: nothing goes into a certificate store.
//! Root's key is sealed as a .biaconfig under a fresh password, kept next to bia-root.p12, and
//! the default browser opens bia's page with that file in the link (`#biaconfig=`, never sent
//! to any server). The person types the password there; the browser keeps a key of its own.
use anyhow::Result;
use base64::Engine;
use bia_pki::biaconfig;
use std::path::Path;

/// The password `bia_pki` puts on root's .p12.
pub const P12_PASSWORD: &str = "12345678";

/// What was done and what is left for the person (one line each).
#[derive(Debug, Default, Clone)]
pub struct Report {
    pub done: Vec<String>,
    pub todo: Vec<String>,
}

/// Seal `<folder>`'s key for `host` as `<folder>/bia-root.biaconfig` and, when `open`, open
/// bia's page with it in the default browser (as the desktop user under sudo).
pub fn install(folder: &Path, host: &str, open: bool) -> Result<Report> {
    let password = biaconfig::generate_password()?;
    let sealed = biaconfig::seal_folder(folder, host, "bia", &password)?;
    let file = folder.join("bia-root.biaconfig");
    std::fs::write(&file, &sealed)?;
    let h = host.trim();
    let base = if h.contains(':') && !h.starts_with('[') && h.matches(':').count() > 1 { format!("https://[{h}]") } else { format!("https://{h}") };
    let link = format!("{base}/#biaconfig={}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&sealed));
    let mut r = Report::default();
    let by_hand = format!("open {base}/ in a browser, drop {} on the page and type the password {password}", file.display());
    if !open {
        r.todo.push(by_hand);
        return Ok(r);
    }
    match open_as_person(&link) {
        Ok(()) => r.done.push(format!("bia opened in your browser with root's key: type the password {password} on its page. The browser then keeps a key of its own; {} also opens with that password in any other browser", file.display())),
        Err(e) => r.todo.push(format!("{by_hand} (no browser opened: {e})")),
    }
    Ok(r)
}

/// The default browser of the person at this computer: under sudo, the user who ran it.
fn open_as_person(url: &str) -> Result<()> {
    if cfg!(target_os = "linux") {
        if let Some(u) = std::env::var("SUDO_USER").ok().filter(|u| !u.is_empty() && u != "root") {
            let ok = std::process::Command::new("sudo").args(["-u", &u, "xdg-open", url]).status().is_ok_and(|s| s.success());
            return if ok { Ok(()) } else { anyhow::bail!("could not open a browser") };
        }
    }
    crate::chatgpt::open_browser(url)
}
