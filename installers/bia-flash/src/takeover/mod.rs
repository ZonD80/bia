//! `bia-flash takeover`: replace the running Linux (a VPS, a server) with bia, from the
//! GitHub release, with the operator's keys, a new PKI whose root certificate the operator
//! already holds, the network carried over by MAC address, and optional setup
//! instructions for bia's model.
//!
//! Order: sources (configs, flags) → detection → questions → every check (RAM, disk,
//! boot loader, space) → summary and confirmation → download and check the image →
//! PKI and seeds → gate (root certificate download) → one-shot boot entry → reboot.
//! Nothing on the machine changes before the confirmation; the disk is only written by
//! bia's rescue image after the reboot, from RAM.
pub mod announce;
pub mod config;
pub mod gate;
pub mod net;
pub mod stage;

use crate::fmm::FmmInput;
use anyhow::{anyhow, bail, Context, Result};
use config::{Config, Merged};
use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

/// A takeover message here and on every other terminal and console (announce).
macro_rules! say {
    () => { announce::all("") };
    ($($t:tt)*) => { announce::all(&format!($($t)*)) };
}

/// The oldest release whose rescue image knows the takeover mode.
pub const MIN_VERSION: (u32, u32, u32) = (1, 1, 3);
/// RAM the rescue needs besides the image (kernel, initramfs unpacked, tools).
const RESCUE_RAM: u64 = 768 << 20;
const RAM_MARGIN: u64 = 512 << 20;

#[derive(Debug, Default, Clone)]
pub struct Args {
    pub configs: Vec<String>,
    pub config_sha256: Vec<String>,
    pub flags: Value,
    pub os_image: Option<PathBuf>,
    pub yes: bool,
    pub dump_config: bool,
    pub show_secrets: bool,
    pub no_test: bool,
    pub dry_run: bool,
    pub staging: Option<PathBuf>,
}

/// Questions on the terminal (`/dev/tty`, so `curl … | sudo bash` works), or none.
struct Ask {
    tty: Option<BufReader<fs::File>>,
}

impl Ask {
    fn new(interactive: bool) -> Ask {
        let tty = if interactive { fs::File::open("/dev/tty").ok().map(BufReader::new) } else { None };
        Ask { tty }
    }

    fn interactive(&self) -> bool {
        self.tty.is_some()
    }

    fn line(&mut self, q: &str, default: &str) -> Result<String> {
        let Some(t) = self.tty.as_mut() else { return Ok(default.to_string()) };
        if default.is_empty() {
            eprint!("{q}: ");
        } else {
            eprint!("{q} [{default}]: ");
        }
        std::io::stderr().flush().ok();
        let mut s = String::new();
        if t.read_line(&mut s)? == 0 {
            bail!("no answer (end of input)");
        }
        let s = s.trim().to_string();
        Ok(if s.is_empty() { default.to_string() } else { s })
    }

    /// Several lines ending with an empty one; `@file` reads a file; `e` opens $EDITOR.
    fn text(&mut self, q: &str) -> Result<String> {
        if self.tty.is_none() {
            return Ok(String::new());
        }
        eprintln!("{q}");
        eprintln!("Type the text and finish with an empty line, @file to read a file, e for $EDITOR, or just press Enter to skip:");
        let mut lines = vec![];
        loop {
            eprint!("> ");
            std::io::stderr().flush().ok();
            let mut s = String::new();
            if self.tty.as_mut().unwrap().read_line(&mut s)? == 0 {
                break;
            }
            let s = s.trim_end_matches(['\n', '\r']).to_string();
            if lines.is_empty() {
                if let Some(p) = s.strip_prefix('@') {
                    return fs::read_to_string(p.trim()).with_context(|| format!("read {}", p.trim()));
                }
                if s.trim() == "e" {
                    let tmp = std::env::temp_dir().join(format!("bia-setup-{}.md", std::process::id()));
                    fs::write(&tmp, "")?;
                    let ed = std::env::var("EDITOR").unwrap_or_else(|_| "nano".into());
                    Command::new("sh").arg("-c").arg(format!("{ed} \"$0\" </dev/tty >/dev/tty", ed = ed)).arg(&tmp).status()?;
                    let t = fs::read_to_string(&tmp).unwrap_or_default();
                    let _ = fs::remove_file(&tmp);
                    return Ok(t.trim().to_string());
                }
            }
            if s.is_empty() {
                break;
            }
            lines.push(s);
        }
        Ok(lines.join("\n"))
    }
}

fn run_cmd(cmd: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(cmd).args(args).output().with_context(|| format!("run {cmd}"))?;
    if !out.status.success() {
        bail!("{cmd} {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn have(cmd: &str) -> bool {
    Command::new("sh").arg("-c").arg(format!("command -v {cmd} >/dev/null 2>&1")).status().is_ok_and(|s| s.success())
}

fn version_tuple(v: &str) -> (u32, u32, u32) {
    let mut it = v.trim_start_matches('v').split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).map(|s| s.parse().unwrap_or(0));
    (it.next().unwrap_or(0), it.next().unwrap_or(0), it.next().unwrap_or(0))
}

/// Files every bia root holds (builders/rootfs.sh). Taking over a bia would erase its agent,
/// e.g. when a misquoted `ssh root@host curl … | bash` runs the pipe on bia itself.
const BIA_MARKERS: [&str; 2] = ["etc/bia-release", "etc/bia-shipped"];

fn bia_marker(root: &Path) -> Option<PathBuf> {
    BIA_MARKERS.iter().map(|m| root.join(m)).find(|p| p.exists())
}

fn mib(b: u64) -> String {
    format!("{} MiB", b >> 20)
}

/// What a new agent starts with (not in land mode).
struct AgentPlan {
    fmm: FmmInput,
    lang: String,
    setup: String,
    root: bia_pki::RootSeed,
    root_from: Vec<bia_pki::Cidr>,
}

/// Letters and digits a land secret needs at least (26 from 32 symbols: 130 bits).
pub const MIN_LAND_SECRET: usize = 26;

/// A land secret: 28 characters (140 bits) from an alphabet without look-alikes, in groups
/// of 4. Same format as carry's `new_land_secret`.
pub fn land_secret() -> String {
    const A: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut b = [0u8; 28];
    getrandom::getrandom(&mut b).expect("random");
    let s: String = b.iter().map(|x| A[(*x as usize) % 32] as char).collect();
    s.as_bytes().chunks(4).map(|c| String::from_utf8_lossy(c).to_string()).collect::<Vec<_>>().join("-")
}

/// One line of the summary: field, value, where it came from.
struct Row(String, String, String);

/// Facts about this machine.
struct Machine {
    arch: String,
    uefi: bool,
    secure_boot: bool,
    ram: u64,
    disk: Option<String>,
    disk_note: String,
    display: bool,
}

fn detect() -> Result<Machine> {
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "arm64",
        a => bail!("bia runs on x86_64 and arm64, not {a}"),
    }
    .to_string();
    let uefi = Path::new("/sys/firmware/efi").is_dir();
    let secure_boot = fs::read_dir("/sys/firmware/efi/efivars")
        .ok()
        .and_then(|d| d.flatten().find(|e| e.file_name().to_string_lossy().starts_with("SecureBoot-")))
        .and_then(|e| fs::read(e.path()).ok())
        .is_some_and(|b| b.last() == Some(&1));
    let ram = fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|m| m.lines().find(|l| l.starts_with("MemTotal:")).and_then(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok()))
        .unwrap_or(0)
        * 1024;
    // the disk holding / (through partitions, LVM, crypt, RAID)
    let src = run_cmd("findmnt", &["-no", "SOURCE", "/"]).unwrap_or_default();
    let src = src.split('[').next().unwrap_or("").to_string();
    let (mut disk, mut disk_note) = (None, String::new());
    if !src.is_empty() {
        let tree = run_cmd("lsblk", &["-nrs", "-o", "NAME,TYPE", &src]).unwrap_or_default();
        let disks: Vec<&str> = tree.lines().filter_map(|l| l.split_once(' ').filter(|(_, t)| *t == "disk").map(|(n, _)| n)).collect();
        let odd: Vec<&str> = tree.lines().filter_map(|l| l.split_once(' ').map(|(_, t)| t)).filter(|t| t.starts_with("raid") || *t == "crypt").collect();
        let mut uniq = disks.clone();
        uniq.sort();
        uniq.dedup();
        if uniq.len() == 1 && odd.is_empty() {
            disk = Some(format!("/dev/{}", uniq[0]));
        } else {
            disk_note = format!("/ is on {src} over {} ({}); name the disk with --disk", uniq.join(", "), odd.join(", "));
        }
    }
    let display = fs::read_dir("/dev/dri").ok().is_some_and(|d| d.flatten().any(|e| e.file_name().to_string_lossy().starts_with("card")));
    Ok(Machine { arch, uefi, secure_boot, ram, disk, disk_note, display })
}

fn capture_network() -> Result<(Vec<net::Iface>, Vec<String>)> {
    let j = |args: &[&str]| -> Result<Value> { Ok(serde_json::from_str(&run_cmd("ip", args)?).unwrap_or(Value::Array(vec![]))) };
    let link = j(&["-j", "-d", "link", "show"])?;
    let addr = j(&["-j", "addr", "show"])?;
    let r4 = j(&["-j", "route", "show", "default"])?;
    let r6 = j(&["-j", "-6", "route", "show", "default"]).unwrap_or(Value::Array(vec![]));
    let mut dns = fs::read_to_string("/run/systemd/resolve/resolv.conf").map(|t| net::parse_resolv(&t)).unwrap_or_default();
    if dns.is_empty() {
        dns = fs::read_to_string("/etc/resolv.conf").map(|t| net::parse_resolv(&t)).unwrap_or_default();
    }
    net::capture(&link, &addr, &r4, &r6, &dns)
}

/// Where to stage: a directory on a filesystem GRUB reads, with room for the image.
/// Returns (directory, path as GRUB sees it on that filesystem).
/// A staging directory: where it is, how the boot loader sees it, and (for the rescue to
/// read the image itself) its filesystem's UUID and the path inside that filesystem.
struct Staging {
    dir: PathBuf,
    rel: String,
    fs_uuid: String,
}

fn staging_dir(need: u64, explicit: Option<&Path>, bios: bool) -> Result<Staging> {
    let cands: Vec<PathBuf> = match explicit {
        Some(p) => vec![p.to_path_buf()],
        None => vec![PathBuf::from("/boot/bia-takeover"), PathBuf::from("/bia-takeover")],
    };
    let mut why = vec![];
    for dir in cands {
        let parent = dir.parent().unwrap_or(Path::new("/"));
        let info = run_cmd("findmnt", &["-no", "TARGET,FSROOT,FSTYPE,SOURCE", "-T", &parent.to_string_lossy()]).unwrap_or_default();
        let f: Vec<&str> = info.split_whitespace().collect();
        let [target, fsroot, fstype, source] = f[..] else { continue };
        if explicit.is_none() && parent == Path::new("/boot") && target != "/boot" {
            continue; // /boot is not its own filesystem; / comes next
        }
        if !matches!(fstype, "ext2" | "ext3" | "ext4" | "xfs" | "btrfs" | "vfat") {
            why.push(format!("{target} is {fstype}, which the boot loader may not read"));
            continue;
        }
        let dev = source.split('[').next().unwrap_or(source);
        let tree = run_cmd("lsblk", &["-nrs", "-o", "TYPE", dev]).unwrap_or_default();
        if tree.lines().any(|t| t == "crypt") {
            why.push(format!("{target} is encrypted"));
            continue;
        }
        // legacy BIOS: the rescue mounts this filesystem itself (GRUB cannot load the
        // image there), and it reads plain partitions with ext2/3/4 or FAT
        let plain_partition = tree.lines().next() == Some("part");
        if bios && (!plain_partition || !matches!(fstype, "ext2" | "ext3" | "ext4" | "vfat")) {
            why.push(format!("{target} is {fstype} on {}; on a legacy BIOS machine the image needs a plain ext4 or FAT partition (e.g. /boot)", tree.lines().next().unwrap_or("?")));
            continue;
        }
        let free = run_cmd("df", &["-B1", "--output=avail", &parent.to_string_lossy()])
            .ok()
            .and_then(|o| o.lines().nth(1).and_then(|l| l.trim().parse::<u64>().ok()))
            .unwrap_or(0);
        if free < need {
            why.push(format!("{target} has {} free, {} needed", mib(free), mib(need)));
            continue;
        }
        let under = dir.strip_prefix(target).map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
        let fsroot = fsroot.trim_end_matches('/');
        let rel = format!("{fsroot}/{}", under.trim_start_matches('/'));
        let fs_uuid = run_cmd("findmnt", &["-no", "UUID", "-T", &parent.to_string_lossy()]).unwrap_or_default();
        return Ok(Staging { dir, rel, fs_uuid });
    }
    bail!("no place for the takeover files ({}); free some space or pass --staging", why.join("; "))
}

fn esp_mount() -> Result<PathBuf> {
    for m in ["/boot/efi", "/efi", "/boot"] {
        if run_cmd("findmnt", &["-no", "FSTYPE", m]).is_ok_and(|t| t == "vfat") {
            return Ok(PathBuf::from(m));
        }
    }
    bail!("the EFI system partition is not mounted (looked at /boot/efi, /efi, /boot)")
}

pub fn run(a: Args) -> Result<()> {
    let r = takeover(a);
    if let Err(e) = &r {
        // the terminal that runs it gets the error from main
        announce::mirror(&format!("stopped: {e:#}"));
    }
    r
}

fn takeover(a: Args) -> Result<()> {
    // ---- sources ----
    let mut m = Merged::new();
    for (i, c) in a.configs.iter().enumerate() {
        let v = config::load(c, a.config_sha256.get(i).map(String::as_str))?;
        m.add(v, &config::source_name(c))?;
    }
    if a.flags.as_object().is_some_and(|o| !o.is_empty()) {
        m.add(a.flags.clone(), "flag")?;
    }
    for w in &m.warnings {
        say!("warning: {w}");
    }
    let cfg: Config = m.config()?;
    if a.dump_config && !cfg!(target_os = "linux") {
        println!("{}", serde_json::to_string_pretty(&config::masked(&m.value, a.show_secrets))?);
        return Ok(());
    }
    if !cfg!(target_os = "linux") {
        bail!("takeover runs on the Linux machine it replaces");
    }
    if run_cmd("id", &["-u"]).unwrap_or_default() != "0" && !a.dump_config {
        bail!("run as root (sudo)");
    }
    if let Some(p) = bia_marker(Path::new("/")) {
        bail!(
            "this machine already runs bia ({}); a takeover would erase it. Nothing was changed.\n  \
             To update bia: carry_update (or /update on the console).\n  \
             To take over another server, run takeover.sh on that server, with the whole pipe inside the quotes:\n  \
             ssh root@<server> 'curl -fsSL …/takeover.sh | bash -s -- …'",
            p.display()
        );
    }
    let mut ask = Ask::new(!a.yes && !a.dump_config);
    let mut rows: Vec<Row> = vec![];
    let src = |k: &str, dflt: &str| m.source_of(k).map(str::to_string).unwrap_or_else(|| dflt.to_string());

    // ---- detection ----
    let mach = detect()?;
    say!(
        "== this machine: {}, {}{}, {} RAM{}",
        mach.arch,
        if mach.uefi { "UEFI" } else { "legacy BIOS" },
        if mach.secure_boot { " with Secure Boot" } else { "" },
        mib(mach.ram),
        if mach.display { "" } else { ", no display device (bia uses a virtual screen; web → display still works)" }
    );
    if !mach.uefi && mach.arch == "arm64" {
        bail!("arm64 without UEFI is not supported");
    }

    // ---- questions ----
    // land mode: the agent flies in later with its own keys, language and certificates
    let land = match &cfg.land {
        None | Some(Value::Bool(false)) => None,
        Some(Value::Bool(true)) => Some(String::new()),
        Some(Value::Object(o)) => Some(o.get("secret").and_then(Value::as_str).unwrap_or("").to_string()),
        Some(v) => bail!("land must be true or {{\"secret\": \"…\"}}, not {v}"),
    };
    let land = match land {
        Some(given) => {
            let given = if given.trim().is_empty() && ask.interactive() {
                ask.line("Land secret you made beforehand (Enter: make a new one here)", "")?
            } else {
                given
            };
            let norm: String = given.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
            if !given.trim().is_empty() && norm.len() < MIN_LAND_SECRET {
                bail!("the land secret needs at least {MIN_LAND_SECRET} letters and digits (make one with `bia-flash land-secret` or carry_new_land_secret)");
            }
            rows.push(Row(
                "land secret".into(),
                if given.trim().is_empty() { "made here, shown before the reboot".into() } else { "given (kept through updates until an agent lands)".into() },
                src("land", "asked"),
            ));
            Some(given)
        }
        None => None,
    };
    rows.push(Row(
        "mode".into(),
        if land.is_some() {
            "land: bia waits for one of your bias to fly its agent here (no keys, no web app until then)".into()
        } else {
            "new agent".into()
        },
        src("land", "default"),
    ));
    let mut ips: Vec<std::net::IpAddr> = vec![];
    let agent = if land.is_none() {
        let fc = cfg.fmm.clone().unwrap_or_default();
        let mut key = fc.api_key.clone().unwrap_or_default();
        if key.trim().is_empty() {
            if !ask.interactive() {
                bail!("an FMM API key is required (fmm.api_key in a config, --key or --fmm); see `bia-flash keys`");
            }
            key = ask.line("API key for bia's model (OpenRouter, OpenAI, Anthropic, xAI, DeepSeek, Qwen)", "")?;
            if key.is_empty() {
                bail!("an API key is required; see `bia-flash keys`");
            }
        }
        let mut fmm = FmmInput {
            provider: fc.provider.clone().unwrap_or_default(),
            api_key: key,
            model: fc.model.clone().unwrap_or_default(),
            base_url: fc.base_url.clone().unwrap_or_default(),
            ctx: fc.ctx.unwrap_or(131_072),
            enabled: Some(true),
            chatgpt: None,
        }
        .resolve()?;
        if fc.model.as_deref().unwrap_or("").is_empty() && ask.interactive() {
            fmm.model = ask.line(&format!("Model ({} ids; `bia-flash models` lists them)", fmm.provider), &fmm.model)?;
        }
        rows.push(Row("fmm".into(), format!("{} {} key {}", fmm.provider, fmm.model, config::mask(&fmm.api_key)), src("fmm", "asked")));

        let lang_in = match &cfg.lang {
            Some(l) => l.clone(),
            None => ask.line("Owner's language (code or name)", &crate::lang::host_default())?,
        };
        let lang = crate::lang::find(&lang_in).with_context(|| format!("unknown language {lang_in:?} (see bia-flash langs)"))?;
        rows.push(Row("lang".into(), format!("{} ({})", lang.name, lang.code), src("lang", if ask.interactive() { "asked" } else { "default: this system's locale" })));

        let setup = match &cfg.setup {
            Some(s) => s.trim().to_string(),
            None => ask
                .text("Setup instructions for bia (optional): bia's model reads them once at its first boot, e.g. \"Connect to the bia at https://198.51.100.4 and set up trust with it.\"")?
                .trim()
                .to_string(),
        };
        rows.push(Row(
            "setup".into(),
            if setup.is_empty() { "(none)".into() } else { first_lines(&setup, 2) },
            src("setup", if ask.interactive() { "asked" } else { "default: empty" }),
        ));

        let pki = cfg.pki.clone().unwrap_or_default();
        let root = match &pki.root {
            Some(Value::Object(o)) if o.get("csr").and_then(Value::as_str).is_some() => {
                bia_pki::RootSeed::Csr(o["csr"].as_str().unwrap().to_string())
            }
            Some(Value::String(s)) if s == "generate" => bia_pki::RootSeed::Generate,
            None => bia_pki::RootSeed::Generate,
            Some(v) => bail!("pki.root must be \"generate\" or {{\"csr\": \"<PEM>\"}}, not {v}"),
        };
        let root_from: Vec<bia_pki::Cidr> = pki.root_from.clone().unwrap_or_default().iter().map(|c| bia_pki::Cidr::parse(c)).collect::<Result<_>>()?;
        rows.push(Row(
            "root certificate".into(),
            match &root {
                bia_pki::RootSeed::Generate => "made here, downloaded from the gate page".to_string(),
                bia_pki::RootSeed::Csr(_) => "your CSR, signed here (your key never leaves you)".to_string(),
            } + &if root_from.is_empty() { String::new() } else { format!("; only from {}", root_from.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(", ")) },
            src("pki", "default"),
        ));
        Some(AgentPlan { fmm, lang: lang.code, setup, root, root_from })
    } else {
        for k in ["fmm", "setup", "pki"] {
            if m.source_of(k).is_some() {
                say!("note: land mode ignores {k}: the agent that flies here brings its own");
            }
        }
        None
    };

    // network
    let (ifaces, net_src) = match &cfg.network {
        Some(Value::Object(o)) => {
            let list: Vec<net::Iface> = serde_json::from_value(o.get("interfaces").cloned().unwrap_or(Value::Array(vec![]))).context("network.interfaces")?;
            for i in &list {
                i.validate()?;
            }
            if list.is_empty() {
                bail!("network.interfaces is empty");
            }
            (list, src("network", "config"))
        }
        Some(Value::String(s)) if s == "auto" => (capture_network()?.0, format!("detected ({})", src("network", "auto"))),
        None => {
            let (l, w) = capture_network()?;
            for w in w {
                say!("note: {w}");
            }
            (l, "detected".into())
        }
        Some(v) => bail!("network must be \"auto\" or {{\"interfaces\": [...]}}, not {v}"),
    };
    for i in &ifaces {
        rows.push(Row("network".into(), i.summary(), net_src.clone()));
    }

    // server certificate names and addresses
    let mut names: Vec<String> = vec![];
    let mut agent_name = None;
    if let Some(_a) = &agent {
        let pki = cfg.pki.clone().unwrap_or_default();
                for i in &ifaces {
            for a in &i.addresses {
                if let Ok(ip) = a.split('/').next().unwrap_or("").parse() {
                    ips.push(ip);
                }
            }
        }
        let mut extra = pki.ips.clone().unwrap_or_default();
        names = pki.names.clone().unwrap_or_default();
        if pki.ips.is_none() && pki.names.is_none() && ask.interactive() {
            let more = ask.line("Public address or DNS name of this machine, if it is behind NAT or has a name (optional, space-separated)", "")?;
            for w in more.split_whitespace() {
                if w.parse::<std::net::IpAddr>().is_ok() {
                    extra.push(w.into());
                } else {
                    names.push(w.into());
                }
            }
        }
        for e in &extra {
            ips.push(bia_pki::parse_ip(e)?);
        }
        ips.sort();
        ips.dedup();
        rows.push(Row(
            "server certificate".into(),
            format!("bia, localhost{}{}", names.iter().map(|n| format!(", {n}")).collect::<String>(), ips.iter().map(|i| format!(", {i}")).collect::<String>()),
            src("pki", "detected"),
        ));
        agent_name = pki.agent_name.clone().filter(|n| !n.trim().is_empty());
    }

    // disk
    let disk = match (&cfg.disk, &mach.disk) {
        (Some(d), _) => d.clone(),
        (None, Some(d)) => d.clone(),
        (None, None) => bail!("{}", if mach.disk_note.is_empty() { "cannot tell which disk holds / ; name it with --disk".to_string() } else { mach.disk_note.clone() }),
    };
    let disk_size: u64 = run_cmd("blockdev", &["--getsize64", &disk])?.parse().context("disk size")?;
    let ptuuid = run_cmd("blkid", &["-p", "-s", "PTUUID", "-o", "value", &disk]).unwrap_or_default().to_ascii_lowercase();
    if ptuuid.is_empty() {
        bail!("{disk} has no partition table id; cannot find it again after the reboot");
    }
    let model = run_cmd("lsblk", &["-dno", "MODEL", &disk]).unwrap_or_default();
    rows.push(Row("disk".into(), format!("{disk} {} {}", mib(disk_size), model.trim()), src("disk", "detected (holds /)")));

    // ---- release and every check, before anything changes ----
    let (image_path, raw_size, raw_sha, xz_size, xz_sha, version) = match &a.os_image {
        Some(p) => {
            let raw = crate::image::raw_size(p)?;
            (Some(p.clone()), raw, String::new(), fs::metadata(p)?.len(), String::new(), "local image".to_string())
        }
        None => {
            let tag = cfg.os.clone().unwrap_or_else(|| "latest".into());
            if tag.starts_with("https://") {
                std::env::set_var("BIA_RELEASE_URL", &tag);
            }
            let man = crate::release::manifest(if tag.starts_with("https://") { "latest" } else { &tag })?;
            if version_tuple(&man.version) < MIN_VERSION {
                bail!("bia {} cannot take over a machine; {}.{}.{} or newer is needed", man.version, MIN_VERSION.0, MIN_VERSION.1, MIN_VERSION.2);
            }
            let e = man.os.get(&mach.arch).with_context(|| format!("release {} has no {} image", man.tag, mach.arch))?;
            let (r, rs, x, xs) = (e.image.raw_size, e.image.raw_sha256.clone(), e.image.size, e.image.sha256.clone());
            (None, r, rs, x, xs, format!("bia {} ({})", man.version, man.tag))
        }
    };
    rows.push(Row("system".into(), format!("{version}: {} download, {} on disk", mib(xz_size), mib(raw_size)), src("os", "default: latest release")));
    if raw_size > disk_size {
        bail!("{disk} ({}) is smaller than the bia image ({})", mib(disk_size), mib(raw_size));
    }
    let boot_extra = 200 << 20; // kernel, initramfs, rescue, seeds (generous)
    // UEFI: GRUB loads the image as an initrd, which the kernel copies once more while
    // unpacking. Legacy BIOS: the rescue copies it from the disk into RAM once.
    let need_ram = if mach.uefi { 2 * (xz_size + boot_extra) } else { xz_size + 2 * boot_extra } + RESCUE_RAM + RAM_MARGIN;
    if mach.ram < need_ram {
        bail!(
            "this machine has {} of RAM; taking it over needs about {} (the image is loaded into memory before the disk is written)",
            mib(mach.ram),
            mib(need_ram)
        );
    }
    let staging = staging_dir(xz_size + boot_extra + (64 << 20), a.staging.as_deref(), !mach.uefi)?;
    let (stage_dir, rel) = (staging.dir.clone(), staging.rel.clone());
    if mach.uefi {
        if !have("efibootmgr") {
            bail!("efibootmgr is missing (apt-get install efibootmgr)");
        }
        esp_mount()?;
    } else {
        let grub = if have("grub-reboot") { "grub-reboot" } else if have("grub2-reboot") { "grub2-reboot" } else { bail!("this BIOS machine has no grub-reboot (GRUB 2 is needed)") };
        let cfgp = grub_dir()?.join("grub.cfg");
        let text = fs::read_to_string(&cfgp).with_context(|| format!("read {}", cfgp.display()))?;
        if !text.contains("custom.cfg") {
            bail!("{} does not read custom.cfg (no 41_custom); cannot add a one-shot entry", cfgp.display());
        }
        let _ = grub;
    }
    rows.push(Row("staging".into(), format!("{} (as {rel} for the boot loader)", stage_dir.display()), "detected".into()));
    rows.push(Row("RAM".into(), format!("{} of {} needed", mib(mach.ram), mib(need_ram)), "checked".into()));

    // ---- summary ----
    if a.dump_config {
        let mut v = m.value.clone();
        config::merge(&mut v, json!({"disk": disk, "os": cfg.os.clone().unwrap_or_else(|| "latest".into()), "network": {"interfaces": ifaces}}));
        if let Some(ag) = &agent {
            config::merge(&mut v, json!({
                "lang": ag.lang, "setup": ag.setup,
                "fmm": {"provider": ag.fmm.provider, "api_key": ag.fmm.api_key, "model": ag.fmm.model},
                "pki": {"names": names, "ips": ips.iter().map(|i| i.to_string()).collect::<Vec<_>>()},
            }));
        }
        println!("{}", serde_json::to_string_pretty(&config::masked(&v, a.show_secrets))?);
        return Ok(());
    }
    say!("\n== bia takeover plan");
    for Row(k, v, s) in &rows {
        say!("  {k:<19} {v}   [{s}]");
    }
    say!();
    if land.is_some() {
        say!("After the reboot everything on {disk} is replaced by bia, which waits for one of your bias to fly");
        say!("its agent here over the internet (TCP 4791 must be reachable) with the land secret you get next.");
    } else {
        say!("After the reboot everything on {disk} is replaced by bia. The only way in is then bia's web app");
        say!("(https://<address>) with the root certificate you get in the next step, or this provider's console.");
    }
    let wipe = match &cfg.wipe {
        Some(w) => w.clone(),
        None => ask.line(&format!("Type {disk} to continue"), "")?,
    };
    if wipe != disk {
        bail!("not confirmed (wipe must name {disk}); nothing was changed");
    }

    // ---- model check, download ----
    if let Some(ag) = agent.as_ref().filter(|_| !a.no_test) {
        say!("== waking {} {} …", ag.fmm.provider, ag.fmm.model);
        let g = crate::test_model(&ag.fmm).context("the model did not answer (fix the key or model, or --no-test)")?;
        say!("\n  bia: {}\n", g.hello);
    }
    fs::create_dir_all(&stage_dir)?;
    let image = match image_path {
        Some(p) => p,
        None => {
            std::env::set_var("BIA_FLASH_CACHE", stage_dir.join("cache"));
            let tag = cfg.os.clone().unwrap_or_else(|| "latest".into());
            let man = crate::release::manifest(if tag.starts_with("https://") { "latest" } else { &tag })?;
            crate::release::fetch(&man, &mach.arch, &mut |msg| say!("== {msg}"))?.path
        }
    };
    let xz_sha = if xz_sha.is_empty() { sha256_file(&image)? } else { xz_sha };
    let raw_sha = if raw_sha.is_empty() {
        say!("== checking the local image");
        let mut r = crate::image::ImageReader::open(&image)?;
        let mut h = <sha2::Sha256 as sha2::Digest>::new();
        r.drain(&mut |b| sha2::Digest::update(&mut h, b))?;
        sha2::Digest::finalize(h).iter().map(|b| format!("{b:02x}")).collect()
    } else {
        raw_sha
    };
    let staged_img = stage_dir.join("bia.img.xz");
    if image != staged_img {
        let _ = fs::remove_file(&staged_img);
        if fs::hard_link(&image, &staged_img).is_err() {
            fs::copy(&image, &staged_img).context("copy the image into the staging directory")?;
        }
    }
    let boot = stage::boot_files(&staged_img, &mach.arch)?;

    // ---- PKI and seeds ----
    let work = stage_dir.join("work");
    let _ = fs::remove_dir_all(&work);
    let files = work.join("files");
    fs::create_dir_all(&files)?;
    let esp = work.join("esp");
    let seed = esp.join("EFI/bia/seed");
    fs::create_dir_all(seed.join("net"))?;
    let mut seeded = None;
    let mut land_code = None;
    if let Some(ag) = agent {
        let sd = bia_pki::seed(&files, &bia_pki::SeedOptions { names: names.clone(), ips: ips.clone(), root: ag.root, root_from: ag.root_from, agent_name })?;
        copy_tree(&sd.dir, &seed.join("pki"))?;
        stage::write_file(&esp.join("EFI/bia/fmm.json"), &crate::fmm_config_json(&ag.fmm)?)?;
        stage::write_file(&esp.join("EFI/bia/lang.json"), &crate::lang::lang_json(&ag.lang))?;
        if !ag.setup.is_empty() {
            stage::write_file(&seed.join("setup.md"), format!("{}\n", ag.setup).as_bytes())?;
        }
        seeded = Some(sd);
    } else if let Some(given) = &land {
        let code = if given.trim().is_empty() { land_secret() } else { given.trim().to_string() };
        let norm: String = code.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_uppercase()).collect();
        if norm.len() < MIN_LAND_SECRET {
            bail!("the land secret needs at least {MIN_LAND_SECRET} letters and digits (make one with `bia-flash land-secret` or carry_new_land_secret)");
        }
        fs::create_dir_all(seed.join("land"))?;
        stage::write_file(&seed.join("land/secret"), format!("{norm}\n").as_bytes())?;
        land_code = Some(code);
    }
    for i in &ifaces {
        // FAT (the ESP) allows no ':' in names: 525400123456.conf
        stage::write_file(&seed.join("net").join(format!("{}.conf", i.mac.to_ascii_lowercase().replace(':', ""))), net::stanza(i).as_bytes())?;
    }
    if let Some(r) = net::resolv(&ifaces) {
        stage::write_file(&seed.join("net/resolv.conf"), r.as_bytes())?;
    }
    let mut cpio = stage::Cpio::new();
    cpio.dir("bia-takeover");
    let img_path = format!("{}/bia.img.xz", rel.trim_end_matches('/'));
    let on_disk = (!mach.uefi).then(|| stage::ImageOnDisk { fs_uuid: &staging.fs_uuid, path: &img_path });
    if !mach.uefi && staging.fs_uuid.is_empty() {
        bail!("cannot tell the UUID of the filesystem holding {}", stage_dir.display());
    }
    cpio.file("bia-takeover/info", stage::info(&ptuuid, disk_size, raw_size, &raw_sha, &xz_sha, &mach.arch, on_disk)?.as_bytes(), 0o600);
    stage::add_tree(&mut cpio, &esp, "bia-takeover/esp")?;
    let mut tk = [0u8; 4];
    getrandom::getrandom(&mut tk).ok();
    let id = format!("takeover-{}.id", tk.iter().map(|b| format!("{b:02x}")).collect::<String>());
    stage::write_file(&stage_dir.join("seed.cpio"), &cpio.finish())?;
    stage::write_file(&stage_dir.join("vmlinuz-lts"), &boot.kernel)?;
    stage::write_file(&stage_dir.join("initramfs-lts"), &boot.initramfs)?;
    stage::write_file(&stage_dir.join("rescue.cpio.gz"), &boot.rescue)?;
    stage::write_file(&stage_dir.join(&id), b"bia takeover\n")?;

    // ---- gate: the operator gets root's certificate before anything is overwritten ----
    if let Some(seeded) = &seeded {
        say!("\n== CA fingerprint (SHA-256) of the new bia:\n   {}", seeded.ca_sha256);
        say!("   other bias know this one as {}", seeded.agent_name);
        if let Some(p12) = seeded.root_p12.clone() {
            let port = cfg.gate_port.unwrap_or(8443);
            let g = gate::start(&files, port, p12, seeded.ca_sha256.clone())
                .context("root's certificate can only leave this machine through the gate page; nothing was changed")?;
            let mut here = global_ips();
            here.extend(ips.iter().copied());
            here.sort();
            here.dedup();
            let urls: Vec<String> = here
                .iter()
                .map(|ip| if ip.is_ipv6() { format!("[{ip}]") } else { ip.to_string() })
                .chain(names.iter().cloned())
                .map(|h| format!("https://{h}:{}/{}/", g.port, g.token))
                .collect();
            say!("\n== ROOT'S CERTIFICATE MUST BE DOWNLOADED NOW (bia-root.p12, password 12345678)");
            say!("   This disk is replaced; after the takeover this certificate is the only way into bia.");
            say!("   Nothing is changed and nothing reboots until it was downloaded from the gate page:");
            for u in &urls {
                eprintln!("       {u}");
            }
            announce::mirror("       (the link is on the terminal that runs the takeover: whoever has it gets root's key)");
            say!("   (the browser warns about the certificate: it is the new bia's own CA, check the fingerprint)");
            if let Some(u) = urls.first() {
                eprintln!("   From a shell (an agent: from your own machine, into your files; never print it):");
                eprintln!("       curl -fk -o bia-root.p12 {u}root.p12");
                eprintln!("       wget --no-check-certificate -O bia-root.p12 {u}root.p12");
                let host = here.first().map_or("<this server>".to_string(), |ip| ip.to_string());
                eprintln!("   If port {} is blocked from outside, through ssh:", g.port);
                eprintln!("       ssh root@{host} 'curl -fsk https://127.0.0.1:{}/{}/root.p12' > bia-root.p12", g.port, g.token);
            }
            wait_for_root(&g.downloaded, cfg.gate_wait.unwrap_or(0))?;
        }
        if let Some(pem) = &seeded.root_pem {
            stage::write_file(Path::new("/root/bia-root.crt"), pem.as_bytes())?;
            say!("== root certificate for your CSR: /root/bia-root.crt");
        }
    }
    if let Some(code) = &land_code {
        let saved = PathBuf::from("/root/bia-land-secret.txt");
        stage::write_file(&saved, format!("{code}\n").as_bytes())?;
        eprintln!("\n== land secret (also in {}; this disk is replaced, so write it down):\n\n   {code}\n", saved.display());
        eprintln!("   After the reboot, tell one of your bias: fly to this server's address with land secret {code}");
        eprintln!("   (carry_fly host=<address> secret=<land secret>). TCP 4791 must be open in the provider's firewall.");
        announce::mirror("== the land secret is on the terminal that runs the takeover (and in /root/bia-land-secret.txt until the reboot)");
        if ask.interactive() {
            loop {
                if ask.line("Type YES once you have the land secret", "")? == "YES" {
                    break;
                }
                eprintln!("Without it no bia can land here; you would have to take the server over again.");
            }
        }
    }
    // the CA key is on the old disk only until the reboot; the seed copy is in seed.cpio
    let _ = fs::remove_dir_all(&work);

    if a.dry_run {
        say!("== dry run: staged in {}; no boot entry was added", stage_dir.display());
        return Ok(());
    }

    // ---- one-shot boot entry ----
    if mach.uefi {
        install_uefi(&mach.arch, &boot, &rel, &id)?;
    } else {
        install_bios(&rel, &id)?;
    }
    let reboot = match cfg.reboot {
        Some(r) => r,
        None if ask.interactive() => ask.line("Reboot into the takeover now? (yes/no)", "yes")?.starts_with('y'),
        None => false,
    };
    say!("\n== ready. At the next boot bia's rescue writes bia onto {disk} from memory (a few minutes), then bia starts.");
    if land_code.is_some() {
        say!("   It then waits in land mode for a flight with the land secret.");
    } else {
        say!("   Then open https://<address>/ with root's certificate.");
    }
    say!("   Rebooting without the takeover: remove {} and the boot entry \"bia takeover\".", stage_dir.display());
    if reboot {
        say!("== rebooting into the takeover now; bia's rescue reports on this console next");
        let _ = Command::new("systemctl").arg("reboot").status();
    } else {
        say!("   Reboot when ready: systemctl reboot");
    }
    Ok(())
}

/// Block until root's certificate was downloaded from the gate (all of it was sent).
/// `limit` > 0 gives up after that many seconds, before anything was changed.
fn wait_for_root(downloaded: &std::sync::atomic::AtomicBool, limit: u64) -> Result<()> {
    say!("   Waiting for the download; the takeover then continues by itself…");
    let start = std::time::Instant::now();
    while !downloaded.load(std::sync::atomic::Ordering::SeqCst) {
        if limit > 0 && start.elapsed().as_secs() >= limit {
            bail!("root's certificate was not downloaded within {limit} s; nothing was changed. Run the takeover again");
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    say!("== root's certificate was downloaded from the gate page");
    Ok(())
}

/// Global addresses on this machine right now (DHCP ones too), for the gate links.
fn global_ips() -> Vec<std::net::IpAddr> {
    let Ok(out) = run_cmd("ip", &["-j", "addr", "show"]) else { return vec![] };
    let v: Value = serde_json::from_str(&out).unwrap_or_default();
    v.as_array()
        .into_iter()
        .flatten()
        .flat_map(|l| l["addr_info"].as_array().cloned().unwrap_or_default())
        .filter(|a| a["scope"].as_str() == Some("global"))
        .filter_map(|a| a["local"].as_str().and_then(|s| s.parse().ok()))
        .collect()
}

fn first_lines(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    let mut out = lines.iter().take(n).cloned().collect::<Vec<_>>().join(" / ");
    if lines.len() > n {
        out.push_str(&format!(" … ({} lines)", lines.len()));
    }
    out
}

fn sha256_file(p: &Path) -> Result<String> {
    use sha2::Digest;
    let mut f = fs::File::open(p)?;
    let mut h = sha2::Sha256::new();
    std::io::copy(&mut f, &mut h)?;
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to)?;
    for e in fs::read_dir(from)?.flatten() {
        let t = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_tree(&e.path(), &t)?;
        } else {
            fs::copy(e.path(), &t)?;
        }
    }
    Ok(())
}

fn grub_dir() -> Result<PathBuf> {
    for d in ["/boot/grub", "/boot/grub2"] {
        if Path::new(d).join("grub.cfg").is_file() {
            return Ok(PathBuf::from(d));
        }
    }
    bail!("no GRUB 2 configuration in /boot/grub or /boot/grub2")
}

/// UEFI: shim + bia's GRUB in EFI/bia-takeover on the old ESP, a boot entry for it and
/// BootNext (shim is signed by Microsoft, so this works under Secure Boot).
fn install_uefi(arch: &str, boot: &stage::BootFiles, rel: &str, id: &str) -> Result<()> {
    let esp = esp_mount()?;
    let dir = esp.join("EFI/bia-takeover");
    fs::create_dir_all(&dir)?;
    for (name, data) in &boot.efi {
        stage::write_file(&dir.join(name), data)?;
    }
    stage::write_file(&dir.join("grub.cfg"), stage::grub_cfg_uefi(rel, id).as_bytes())?;
    stage::write_file(&dir.join(id), b"bia takeover\n")?;
    // bia's (Debian's) signed GRUB reads EFI/debian/grub.cfg first when it exists
    for other in ["EFI/debian/grub.cfg", "boot/grub/grub.cfg"] {
        let p = esp.join(other);
        if p.is_file() {
            let orig = fs::read_to_string(&p)?;
            let backup = p.with_extension("cfg.bia-orig");
            if !backup.exists() {
                fs::write(&backup, &orig)?;
            }
            if !orig.contains("# bia takeover") {
                fs::write(&p, format!("{}{orig}", stage::grub_hook(id)))?;
            }
            say!("note: {} now starts the takeover first (original in {})", p.display(), backup.display());
        }
    }
    let src = run_cmd("findmnt", &["-no", "SOURCE", &esp.to_string_lossy()])?;
    let name = Path::new(&src).file_name().map(|n| n.to_string_lossy().to_string()).ok_or_else(|| anyhow!("ESP device"))?;
    let part = fs::read_to_string(format!("/sys/class/block/{name}/partition"))?.trim().to_string();
    let parent = run_cmd("lsblk", &["-no", "PKNAME", &src])?;
    let parent = parent.lines().next().unwrap_or("").trim().to_string();
    let order = run_cmd("efibootmgr", &[])?.lines().find_map(|l| l.strip_prefix("BootOrder: ").map(str::to_string)).unwrap_or_default();
    // drop entries left by an earlier try
    for l in run_cmd("efibootmgr", &[])?.lines() {
        if l.contains("bia takeover") && l.starts_with("Boot") {
            let n = &l[4..8];
            let _ = run_cmd("efibootmgr", &["-q", "-b", n, "-B"]);
        }
    }
    let loader = format!("\\EFI\\bia-takeover\\{}", if arch == "arm64" { "shimaa64.efi" } else { "shimx64.efi" });
    run_cmd("efibootmgr", &["-q", "-c", "-d", &format!("/dev/{parent}"), "-p", &part, "-L", "bia takeover", "-l", &loader])?;
    let new = run_cmd("efibootmgr", &[])?
        .lines()
        .filter(|l| l.starts_with("Boot") && l.contains("bia takeover"))
        .map(|l| l[4..8].to_string())
        .last()
        .ok_or_else(|| anyhow!("the new boot entry did not appear"))?;
    if !order.is_empty() {
        let _ = run_cmd("efibootmgr", &["-q", "-o", &order]);
    }
    run_cmd("efibootmgr", &["-q", "-n", &new])?;
    say!("== next boot: Boot{new} (bia takeover), once");
    Ok(())
}

/// Legacy BIOS: a menu entry in GRUB's custom.cfg, chosen once with grub-reboot.
fn install_bios(rel: &str, id: &str) -> Result<()> {
    let dir = grub_dir()?;
    let custom = dir.join("custom.cfg");
    let mut text = fs::read_to_string(&custom).unwrap_or_default();
    if !text.is_empty() && !custom.with_extension("cfg.bia-orig").exists() {
        fs::write(custom.with_extension("cfg.bia-orig"), &text)?;
    }
    if let Some(i) = text.find("# bia takeover\n") {
        text.truncate(i);
    }
    text.push_str("# bia takeover\n");
    text.push_str(&stage::grub_entry(rel, id, false));
    fs::write(&custom, text)?;
    let cmd = if have("grub-reboot") { "grub-reboot" } else { "grub2-reboot" };
    run_cmd(cmd, &["bia-takeover"])?;
    say!("== next boot: GRUB entry \"bia takeover\", once");
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn versions() {
        assert!(super::version_tuple("1.1.2") < super::MIN_VERSION);
        assert!(super::version_tuple("v1.1.3") >= super::MIN_VERSION);
        assert!(super::version_tuple("1.2.0") >= super::MIN_VERSION);
        assert_eq!(super::first_lines("a\nb\nc", 2), "a / b … (3 lines)");
        let l = super::land_secret();
        assert_eq!(l.len(), 34, "{l}");
        assert!(l.split('-').all(|g| g.len() == 4 && g.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())));
        assert!(!l.contains(['0', '1', 'I', 'O']), "no look-alikes");
    }

    #[test]
    fn refuses_bia_itself() {
        let d = std::env::temp_dir().join(format!("bia-takeover-marker-{}", std::process::id()));
        std::fs::create_dir_all(d.join("etc")).unwrap();
        assert_eq!(super::bia_marker(&d), None, "plain Linux");
        std::fs::write(d.join("etc/bia-shipped"), "").unwrap();
        assert_eq!(super::bia_marker(&d), Some(d.join("etc/bia-shipped")));
        std::fs::write(d.join("etc/bia-release"), "20261020000000\n").unwrap();
        assert_eq!(super::bia_marker(&d), Some(d.join("etc/bia-release")));
        std::fs::remove_dir_all(&d).unwrap();
    }
}
