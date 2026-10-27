//! Make this computer know the new bia: root's certificate (bia-root.p12) into the
//! person's certificate store, so browsers offer it to bia's web app, and bia's CA as a
//! trusted root, so they open https://<server>/ without a warning. The system asks for
//! the person's approval where it wants one (macOS password, Windows warning).
use anyhow::Result;
use std::path::Path;
use std::process::Command;

/// The password `bia_pki` puts on root's .p12.
pub const P12_PASSWORD: &str = "12345678";

/// What was done and what is left for the person (one line each).
#[derive(Debug, Default, Clone)]
pub struct Report {
    pub done: Vec<String>,
    pub todo: Vec<String>,
}

fn run(cmd: &str, args: &[&str]) -> std::result::Result<(), String> {
    let mut c = Command::new(cmd);
    c.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // no console window flashing up behind the window app
        c.creation_flags(0x0800_0000);
    }
    match c.output() {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => Err(format!("{}{}", String::from_utf8_lossy(&o.stderr).trim(), String::from_utf8_lossy(&o.stdout).trim())),
        Err(e) => Err(e.to_string()),
    }
}

/// Import `<folder>/bia-root.p12` and trust `<folder>/ca.crt`. `name` labels them.
pub fn install(folder: &Path, name: &str) -> Result<Report> {
    let p12 = folder.join("bia-root.p12");
    let ca = folder.join("ca.crt");
    let (p12s, cas) = (p12.to_string_lossy().to_string(), ca.to_string_lossy().to_string());
    let mut r = Report::default();
    let mut step = |ok: std::result::Result<(), String>, done: &str, todo: &str| match ok {
        Ok(()) => r.done.push(done.to_string()),
        Err(e) => r.todo.push(format!("{todo} ({})", first_line(&e))),
    };
    if cfg!(target_os = "macos") {
        let kc = std::env::var("HOME").map(|h| format!("{h}/Library/Keychains/login.keychain-db")).unwrap_or_default();
        step(
            run("security", &["import", &p12s, "-k", &kc, "-f", "pkcs12", "-P", P12_PASSWORD]),
            "root's certificate is in your login keychain (the browser asks once to use it: Always Allow)",
            "import bia-root.p12 into Keychain Access yourself (password 12345678)",
        );
        step(
            run("security", &["add-trusted-cert", "-r", "trustRoot", "-k", &kc, &cas]),
            "bia's certificate authority is trusted",
            "trust ca.crt in Keychain Access yourself (open it, Trust: Always Trust)",
        );
    } else if cfg!(target_os = "windows") {
        // NoRoot: the CA inside the .p12 does not slip into the trusted roots unasked
        step(
            run("certutil", &["-f", "-user", "-p", P12_PASSWORD, "-importpfx", "My", &p12s, "NoRoot"]),
            "root's certificate is in your personal certificate store",
            "open bia-root.p12 and import it yourself (password 12345678)",
        );
        step(
            run("certutil", &["-user", "-addstore", "Root", &cas]),
            "bia's certificate authority is trusted",
            "open ca.crt and install it under Trusted Root Certification Authorities",
        );
    } else {
        linux(&p12s, &cas, name, &mut step);
    }
    if !cfg!(target_os = "linux") {
        r.todo.push("Firefox keeps its own certificates: import bia-root.p12 there under Settings → Certificates if you use it".into());
    }
    Ok(r)
}

/// Chrome and Chromium read the person's NSS database; Firefox has its own per profile.
/// As root (the stick writer runs under sudo) the files belong to the sudo user.
fn linux(p12: &str, ca: &str, name: &str, step: &mut dyn FnMut(std::result::Result<(), String>, &str, &str)) {
    let user = std::env::var("SUDO_USER").ok().filter(|u| !u.is_empty() && u != "root");
    let home = match &user {
        Some(u) => format!("/home/{u}"),
        None => std::env::var("HOME").unwrap_or_default(),
    };
    let db = format!("sql:{home}/.pki/nssdb");
    let as_user = |cmd: &str, args: &[&str]| -> std::result::Result<(), String> {
        match &user {
            Some(u) => {
                let mut a = vec!["-u", u.as_str(), cmd];
                a.extend_from_slice(args);
                run("sudo", &a)
            }
            None => run(cmd, args),
        }
    };
    let have = |c: &str| Command::new("sh").arg("-c").arg(format!("command -v {c} >/dev/null")).status().is_ok_and(|s| s.success());
    if have("pk12util") && have("certutil") {
        // the files are in root's app folder, which the sudo user cannot read: hand over copies
        let tmp = std::env::temp_dir().join(format!("bia-deploy-trust-{}", std::process::id()));
        let (p12, ca) = match &user {
            Some(u) => {
                let _ = std::fs::create_dir_all(&tmp);
                let (a, b) = (tmp.join("bia-root.p12"), tmp.join("ca.crt"));
                let _ = std::fs::copy(p12, &a).and_then(|_| std::fs::copy(ca, &b));
                let _ = run("chown", &["-R", u, &tmp.to_string_lossy()]);
                (a.to_string_lossy().to_string(), b.to_string_lossy().to_string())
            }
            None => (p12.to_string(), ca.to_string()),
        };
        let (p12, ca) = (p12.as_str(), ca.as_str());
        let _ = as_user("mkdir", &["-p", &format!("{home}/.pki/nssdb")]);
        if !Path::new(&format!("{home}/.pki/nssdb/cert9.db")).exists() {
            let _ = as_user("certutil", &["-d", &db, "-N", "--empty-password"]);
        }
        step(
            as_user("pk12util", &["-d", &db, "-i", p12, "-W", P12_PASSWORD]),
            "root's certificate is in Chrome's certificate store",
            "import bia-root.p12 in Chrome under Settings → Privacy and security → Security → Manage certificates (password 12345678)",
        );
        step(
            as_user("certutil", &["-d", &db, "-A", "-t", "C,,", "-n", name, "-i", ca]),
            "Chrome trusts bia's certificate authority",
            "import ca.crt in Chrome's certificate settings as an authority",
        );
        let _ = std::fs::remove_dir_all(&tmp);
    } else {
        step(
            Err("pk12util and certutil are missing: apt install libnss3-tools".into()),
            "",
            "import bia-root.p12 into your browser (password 12345678) and trust ca.crt",
        );
    }
}

fn first_line(s: &str) -> String {
    s.lines().find(|l| !l.trim().is_empty()).unwrap_or("").chars().take(160).collect()
}
