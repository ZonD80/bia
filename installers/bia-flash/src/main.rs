//! bia-flash — write a bootable bia USB stick (or image), optionally with FMM keys and a
//! GGUF model from Hugging Face.
use anyhow::{bail, Context, Result};
use bia_flash::{
    chatgpt, filter_models, hf, list_models, list_removable, probe_key, provider_for_key, test_model, write_stick,
    FlashOptions, FmmInput, KEY_GUIDES, KEY_STEPS, LOCAL_FMM_CANDIDATES,
};
use clap::{Parser, Subcommand};
use std::io::{self, Write};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "bia-flash", about = "Write a bootable bia USB stick")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// List removable (or all) disks.
    List {
        /// Include non-removable whole disks.
        #[arg(long)]
        all: bool,
    },
    /// How and where to get an API key for each provider.
    Keys,
    /// Languages bia can be flashed with (`write --lang`).
    Langs,
    /// Continue with ChatGPT: sign in in the browser so your ChatGPT plan pays for bia.
    /// Writes an FMM file for `write --fmm`; one sign-in is for one stick.
    Chatgpt {
        /// Model id (see the list printed after sign-in). Without it the default (the id
        /// containing `6.1-sol`) is used; if the account has none, the list is shown and
        /// nothing is saved.
        #[arg(long)]
        model: Option<String>,
        /// Your ChatGPT email, to pick the account in the browser.
        #[arg(long)]
        email: Option<String>,
        /// Where to save the FMM file (owner-only permissions).
        #[arg(long, default_value = "fmm.json")]
        out: PathBuf,
    },
    /// List the cloud models an FMM key can use. The provider comes from the key
    /// (DeepSeek/Qwen keys are tried against both APIs). Put the chosen id in fmm.json `model`.
    Models {
        /// Words that must all appear in the id or name, e.g. `claude opus`.
        query: Vec<String>,
        /// FMM JSON with at least `api_key` (default: fmm.json, then source/flash/files/state/fmm.json).
        #[arg(long)]
        fmm: Option<PathBuf>,
    },
    /// Let the FMM model say hello (one tool-calling request, the way the agent makes it).
    Test {
        /// FMM JSON (default: fmm.json, then source/flash/files/state/fmm.json).
        #[arg(long)]
        fmm: Option<PathBuf>,
    },
    /// Search Hugging Face for GGUF models.
    Search {
        /// Words to search for, e.g. `qwen3 30b`.
        query: Vec<String>,
        #[arg(long, default_value = "20")]
        limit: usize,
        /// Hugging Face token (default: $HF_TOKEN).
        #[arg(long)]
        hf_token: Option<String>,
    },
    /// List the GGUF files of a Hugging Face repo (split models count as one).
    Files {
        /// Repo id, e.g. `unsloth/Qwen3-30B-A3B-Instruct-2507-GGUF`.
        repo: String,
        #[arg(long)]
        hf_token: Option<String>,
    },
    /// Download the bia OS for an architecture from GitHub releases (cached), or list them.
    Os {
        /// `latest` or a release tag such as `v1.0.0`.
        #[arg(long, default_value = "latest")]
        tag: String,
        /// Architecture to download (`x86_64` or `arm64`); omit to only show the release.
        #[arg(long)]
        arch: Option<String>,
    },
    /// Build the minimal bia disk image (what releases ship as `bia-<arch>.img.xz`) from a
    /// local build: GPT, ESP, BIOS GRUB (x86_64) and BIAROOT, no keys and no model.
    Image {
        /// Root filesystem image, e.g. `dist/x86_64/root.img`.
        #[arg(long)]
        root: PathBuf,
        /// Tree with `EFI/` (and optional `boot/`).
        #[arg(long, default_value = "source/flash")]
        esp: PathBuf,
        /// Directory with `vmlinuz-lts` + `initramfs-lts` (defaults to root's parent).
        #[arg(long)]
        kern: Option<PathBuf>,
        /// Architecture: `x86_64` or `arm64`.
        #[arg(long)]
        arch: String,
        /// Output image (replaced).
        #[arg(long)]
        out: PathBuf,
    },
    /// Erase a disk (or create an image file) and write bia, with FMM keys and a model.
    Write {
        /// Target whole disk (`/dev/rdiskN`, `/dev/sdb`, `\\.\PhysicalDriveN`) or image path.
        #[arg(long)]
        disk: PathBuf,
        /// Local bia disk image (`bia-<arch>.img` or `.img.xz`, from `bia-flash image` or a
        /// release). Without it the image is downloaded from the GitHub release named by `--os`.
        #[arg(long)]
        os_image: Option<PathBuf>,
        /// GitHub release to take the OS from when `--os-image` is not given: `latest` or a tag.
        #[arg(long, default_value = "latest")]
        os: String,
        /// Architecture of the release image: `x86_64` or `arm64`.
        #[arg(long, default_value = "x86_64")]
        arch: String,
        /// Disk/image size in bytes (required for new image files; probed for block devices).
        #[arg(long, default_value = "0")]
        size: u64,
        /// Create `disk` as a sparse image file.
        #[arg(long = "image")]
        image_file: bool,
        /// Skip confirmation prompt.
        #[arg(long)]
        yes: bool,
        /// Local FMM config JSON (provider, api_key, model, …). Prefer this over env vars.
        /// If omitted, tries `fmm.json` then `source/flash/files/state/fmm.json`.
        #[arg(long)]
        fmm: Option<PathBuf>,
        /// Do not auto-load a local fmm.json when `--fmm` is omitted.
        #[arg(long)]
        no_fmm: bool,
        /// Write even if the FMM model test fails (e.g. flashing offline).
        #[arg(long)]
        no_test: bool,
        /// Local model from Hugging Face: `repo` or `repo:file.gguf` (see `search` / `files`).
        /// Without a file the repo's only GGUF, or its Q4_K_M, is taken.
        #[arg(long)]
        model: Option<String>,
        /// Hugging Face token for gated or private repos (default: $HF_TOKEN).
        #[arg(long)]
        hf_token: Option<String>,
        /// The owner's language (code or English name, see `langs`): bia speaks it from the
        /// first boot, with its voice and keyboard. Default: the host's locale, else English.
        #[arg(long)]
        lang: Option<String>,
    },
    /// Print a new land secret (140 bits) for `takeover --land --land-secret`: make it
    /// before the takeover, so whoever sets the server up knows it for the flight.
    LandSecret,
    /// Replace THIS running Linux (a VPS or server) with bia: keys, a new PKI whose root
    /// certificate you download first, the network carried over by MAC address, and
    /// optional setup instructions for bia. Asks for whatever the configs and flags leave
    /// open; nothing changes before you confirm. Run as root on the machine itself.
    Takeover {
        /// Deployment config (JSON file or https:// URL); repeatable, later ones win.
        /// Partial configs are fine, e.g. only {"fmm": {"key": "…"}}.
        #[arg(long)]
        config: Vec<String>,
        /// sha256 of the config at the same position (pins a URL).
        #[arg(long)]
        config_sha256: Vec<String>,
        /// Disk to replace (default: the one holding /).
        #[arg(long)]
        disk: Option<String>,
        /// Confirm without asking: must name the same disk.
        #[arg(long)]
        wipe: Option<String>,
        /// `latest`, a release tag, or an https:// directory with bia-os.json.
        #[arg(long)]
        os: Option<String>,
        /// Local bia image (`.img.xz`) instead of a release (tests).
        #[arg(long)]
        os_image: Option<PathBuf>,
        /// FMM JSON file (provider, api_key, model, …).
        #[arg(long)]
        fmm: Option<PathBuf>,
        /// FMM API key (provider from its format).
        #[arg(long)]
        key: Option<String>,
        /// Model id for the key's provider.
        #[arg(long)]
        model: Option<String>,
        /// Setup instructions for bia's model (read once at its first boot).
        #[arg(long)]
        setup: Option<String>,
        #[arg(long)]
        setup_file: Option<PathBuf>,
        /// The owner's language (code or name).
        #[arg(long)]
        lang: Option<String>,
        /// DNS name for bia's HTTPS certificate (repeatable).
        #[arg(long)]
        name: Vec<String>,
        /// Extra address for it, e.g. the public one behind NAT (repeatable).
        #[arg(long)]
        ip: Vec<String>,
        /// Your own CSR (PEM) for root's certificate instead of a .p12 made here.
        #[arg(long)]
        root_csr: Option<PathBuf>,
        /// The name other bias know this one by.
        #[arg(long)]
        agent_name: Option<String>,
        /// Port of the one-time page that hands out root's certificate.
        #[arg(long)]
        gate_port: Option<u16>,
        /// Where to stage the takeover files (default /boot/bia-takeover or /bia-takeover).
        #[arg(long)]
        staging: Option<PathBuf>,
        /// Ask nothing: missing required values are errors, optional ones stay empty.
        #[arg(long)]
        yes: bool,
        /// Reboot into the takeover at the end without asking.
        #[arg(long)]
        reboot: bool,
        /// Detect and resolve everything, print the config (secrets masked), change nothing.
        #[arg(long)]
        dump_config: bool,
        #[arg(long)]
        show_secrets: bool,
        /// Skip the model check.
        #[arg(long)]
        no_test: bool,
        /// Stage everything but add no boot entry.
        #[arg(long)]
        dry_run: bool,
        /// Land mode: bia waits for one of your bias to fly its agent here over the
        /// internet (no keys or certificates are made here). Prints a land secret.
        #[arg(long)]
        land: bool,
        /// Land mode with this secret instead of a new one.
        #[arg(long)]
        land_secret: Option<String>,
    },
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        let msg = format!("{e:#}");
        if msg.contains("key") || msg.contains("no FMM file") {
            eprintln!("Need an API key? Run: bia-flash keys");
        }
        std::process::exit(1);
    }
}

fn token(arg: Option<String>) -> Option<String> {
    arg.or_else(|| std::env::var("HF_TOKEN").ok())
        .filter(|t| !t.trim().is_empty())
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Models { query, fmm } => {
            let path = FmmInput::find_local(fmm.as_deref()).with_context(|| {
                format!("no FMM file (pass --fmm, or create one of {})", LOCAL_FMM_CANDIDATES.join(", "))
            })?;
            let data = std::fs::read(&path).with_context(|| format!("read {}", path.display()))?;
            let c: FmmInput = serde_json::from_slice(&data)
                .with_context(|| format!("parse {}", path.display()))?;
            if c.provider.trim() == "chatgpt" {
                let mut c = c;
                c.refresh_chatgpt()?;
                for m in filter_models(&chatgpt::list_models(&c.api_key)?, &query.join(" ")) {
                    println!("{}\t{}", m.id, if m.name != m.id { m.name.as_str() } else { "" });
                }
                return Ok(());
            }
            c.validate()?;
            let mut provider = c.provider.trim().to_lowercase();
            let mut base = c.base_url.trim().to_string();
            let models = if provider.is_empty() {
                match provider_for_key(&c.api_key) {
                    Some(p) => {
                        provider = p.into();
                        list_models(p, &c.api_key, &base)?
                    }
                    None => {
                        let (p, b, m) = probe_key(&c.api_key)?;
                        provider = p.into();
                        if base.is_empty() {
                            base = b;
                        }
                        m
                    }
                }
            } else {
                list_models(&provider, &c.api_key, &base)?
            };
            eprintln!("{provider} key{}; {} models", if base.is_empty() { String::new() } else { format!(" at {base}") }, models.len());
            for m in filter_models(&models, &query.join(" ")) {
                let ctx = if m.ctx > 0 { format!("{}k ctx", m.ctx / 1000) } else { String::new() };
                println!("{}\t{}\t{ctx}", m.id, if m.name != m.id { m.name.as_str() } else { "" });
            }
            Ok(())
        }
        Cmd::Chatgpt { model, email, out } => {
            // Never replace someone's API-key file with a ChatGPT sign-in.
            if let Ok(d) = std::fs::read(&out) {
                let provider = serde_json::from_slice::<serde_json::Value>(&d)
                    .ok()
                    .and_then(|v| v["provider"].as_str().map(str::to_string))
                    .unwrap_or_default();
                if provider != "chatgpt" {
                    bail!("{} already holds other FMM settings; pass --out another.json", out.display());
                }
            }
            let auth = chatgpt::sign_in(email.as_deref(), &mut |s| eprintln!("== {s}"))?;
            eprintln!("== signed in as {}", auth.email);
            let models = chatgpt::list_models(&auth.access_token)?;
            let model = model.or_else(|| {
                let d = chatgpt::default_model(&models)?;
                eprintln!("== model {} (default; --model picks another)", d.id);
                Some(d.id.clone())
            });
            let Some(model) = model else {
                for m in &models {
                    println!("{}\t{}", m.id, if m.name != m.id { m.name.as_str() } else { "" });
                }
                eprintln!("pick one and run again with --model <id> (nothing saved)");
                return Ok(());
            };
            if !models.iter().any(|m| m.id == model) {
                bail!("{model} is not available to this ChatGPT account (see the list without --model)");
            }
            let cfg = FmmInput::from_chatgpt(auth, &model, 0).normalize();
            let mut f = chatgpt::open_private(&out).with_context(|| format!("create {}", out.display()))?;
            f.write_all(&bia_flash::fmm_config_json(&cfg)?)?;
            eprintln!("== saved {} (provider=chatgpt, model={model}); now: bia-flash write … --fmm {}", out.display(), out.display());
            Ok(())
        }
        Cmd::Langs => {
            for l in bia_flash::lang::languages() {
                println!("{:<4} {}{}", l.code, l.name, if l.voice { "" } else { "  (no voice: text only)" });
            }
            Ok(())
        }
        Cmd::Keys => {
            println!("Getting an API key:");
            for (i, step) in KEY_STEPS.iter().enumerate() {
                println!("  {}. {step}", i + 1);
            }
            for g in KEY_GUIDES {
                println!("\n{}\n  {}\n  key:    {}", g.name, g.about, g.key_url);
                if let Some(pay) = g.pay_url {
                    println!("  credit: {pay}");
                }
            }
            println!(
                "\nHave ChatGPT Plus or Pro? No key needed: run `bia-flash chatgpt` to use your plan."
            );
            println!(
                "Put the key in fmm.json (copy fmm.json.example); provider and model can be left out."
            );
            Ok(())
        }
        Cmd::Test { fmm } => {
            let path = FmmInput::find_local(fmm.as_deref()).with_context(|| {
                format!("no FMM file (pass --fmm, or create one of {})", LOCAL_FMM_CANDIDATES.join(", "))
            })?;
            let mut cfg = FmmInput::load(&path)?;
            if cfg.chatgpt.is_some() {
                // Renewing rotates the refresh token: save the new set back right away.
                cfg.refresh_chatgpt()?;
                let mut f = chatgpt::open_private(&path)?;
                f.write_all(&bia_flash::fmm_config_json(&cfg)?)?;
            }
            let g = test_model(&cfg)?;
            println!("bia: {}\nbia: {}\n({})", g.hello, g.ready, g.summary);
            Ok(())
        }
        Cmd::Search {
            query,
            limit,
            hf_token,
        } => {
            let repos = hf::search(&query.join(" "), limit, token(hf_token).as_deref())?;
            if repos.is_empty() {
                println!("(no GGUF repos found)");
            }
            for r in repos {
                println!("{}\t{} downloads\t{} likes", r.id, r.downloads, r.likes);
            }
            Ok(())
        }
        Cmd::Files { repo, hf_token } => {
            for m in hf::models(&repo, token(hf_token).as_deref())? {
                println!("{}\t{} MiB\t{}", m.files[0].path, m.size() >> 20, m.label());
            }
            Ok(())
        }
        Cmd::List { all } => {
            let disks = list_removable(all)?;
            if disks.is_empty() {
                println!("(no disks found)");
                return Ok(());
            }
            for d in disks {
                let kind = if d.removable { "removable" } else { "fixed" };
                println!(
                    "{}\t{}\t{} MiB\t{kind}",
                    d.path.display(),
                    d.name,
                    d.size_bytes / (1024 * 1024)
                );
            }
            Ok(())
        }
        Cmd::Os { tag, arch } => {
            let m = bia_flash::release::manifest(&tag)?;
            println!("bia {} (release {})", m.version, m.tag);
            for (a, e) in &m.os {
                println!(
                    "  {a}: {} — {} MiB image, {} MiB download; {} MiB still to download",
                    e.image.file,
                    e.image.raw_size >> 20,
                    e.image.size >> 20,
                    bia_flash::release::missing_bytes(&m, a) >> 20
                );
            }
            if let Some(a) = arch {
                let f = bia_flash::release::fetch(&m, &a, &mut |msg| eprintln!("== {msg}"))?;
                println!("{}", f.path.display());
            }
            Ok(())
        }
        Cmd::LandSecret => {
            println!("{}", bia_flash::takeover::land_secret());
            Ok(())
        }
        Cmd::Takeover {
            config,
            config_sha256,
            disk,
            wipe,
            os,
            os_image,
            fmm,
            key,
            model,
            setup,
            setup_file,
            lang,
            name,
            ip,
            root_csr,
            agent_name,
            gate_port,
            staging,
            yes,
            reboot,
            dump_config,
            show_secrets,
            no_test,
            dry_run,
            land,
            land_secret,
        } => {
            // flags are the last config layer
            let mut f = serde_json::Map::new();
            let mut put = |k: &str, v: serde_json::Value| {
                f.insert(k.to_string(), v);
            };
            if let Some(d) = disk {
                put("disk", d.into());
            }
            if let Some(w) = wipe {
                put("wipe", w.into());
            }
            if let Some(o) = os {
                put("os", o.into());
            }
            let mut fm = serde_json::Map::new();
            if let Some(p) = fmm {
                let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).with_context(|| format!("read {}", p.display()))?)
                    .with_context(|| format!("parse {}", p.display()))?;
                if let Some(o) = v.as_object() {
                    fm.extend(o.clone());
                }
            }
            if let Some(k) = key {
                fm.insert("api_key".into(), k.into());
            }
            if let Some(m) = model {
                fm.insert("model".into(), m.into());
            }
            if !fm.is_empty() {
                put("fmm", fm.into());
            }
            if let Some(t) = setup {
                put("setup", t.into());
            }
            if let Some(p) = setup_file {
                put("setup", std::fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?.into());
            }
            if let Some(l) = lang {
                put("lang", l.into());
            }
            let mut pk = serde_json::Map::new();
            if !name.is_empty() {
                pk.insert("names".into(), name.into());
            }
            if !ip.is_empty() {
                pk.insert("ips".into(), ip.into());
            }
            if let Some(p) = root_csr {
                let csr = std::fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?;
                pk.insert("root".into(), serde_json::json!({"csr": csr}));
            }
            if let Some(n) = agent_name {
                pk.insert("agent_name".into(), n.into());
            }
            if !pk.is_empty() {
                put("pki", pk.into());
            }
            if let Some(p) = gate_port {
                put("gate_port", p.into());
            }
            if reboot {
                put("reboot", true.into());
            }
            if let Some(sec) = land_secret {
                put("land", serde_json::json!({"secret": sec}));
            } else if land {
                put("land", true.into());
            }
            bia_flash::takeover::run(bia_flash::takeover::Args {
                configs: config,
                config_sha256,
                flags: serde_json::Value::Object(f),
                os_image,
                yes,
                dump_config,
                show_secrets,
                no_test,
                dry_run,
                staging,
            })
        }
        Cmd::Image { root, esp, kern, arch, out } => {
            let kern_dir = kern.unwrap_or_else(|| root.parent().map_or_else(|| PathBuf::from("."), |p| p.to_path_buf()));
            let src = bia_flash::ImageSources { root_img: root, esp_src: esp, kern_dir, arch };
            bia_flash::build_image(&src, &out, &mut (Box::new(|m: &str| eprintln!("== {m}")) as bia_flash::ProgressFn))?;
            Ok(())
        }
        Cmd::Write {
            disk,
            os_image,
            os,
            arch,
            size,
            image_file,
            yes,
            fmm,
            no_fmm,
            no_test,
            model,
            hf_token,
            lang,
        } => {
            let lang = match lang {
                Some(l) => bia_flash::lang::find(&l)
                    .with_context(|| format!("unknown language {l:?} (see bia-flash langs)"))?,
                None => bia_flash::lang::find(&bia_flash::lang::host_default()).context("no English in the catalog")?,
            };
            eprintln!(
                "note: language {} ({}){}",
                lang.name,
                lang.code,
                if lang.voice { "" } else { " — no voice exists for it: bia reads and writes it, and speaks English" }
            );
            let hf_token = token(hf_token);
            let model = match model {
                Some(spec) => {
                    let (repo, file) = match spec.split_once(':') {
                        Some((r, f)) => (r, Some(f)),
                        None => (spec.as_str(), None),
                    };
                    let m = hf::pick(repo, file, hf_token.as_deref())?;
                    eprintln!(
                        "note: model {}/{} ({} MiB) goes onto the stick",
                        m.repo,
                        m.label(),
                        m.size() >> 20
                    );
                    Some(m)
                }
                None => None,
            };
            let mut fmm_file = None;
            let fmm = if no_fmm {
                None
            } else {
                match FmmInput::find_local(fmm.as_deref()) {
                    Some(path) => {
                        let mut cfg = FmmInput::load(&path)
                            .with_context(|| format!("load FMM from {}", path.display()))?;
                        cfg.refresh_chatgpt()?;
                        fmm_file = Some(path.clone());
                        eprintln!(
                            "note: bundling FMM from {} (provider={}, model={})",
                            path.display(),
                            cfg.provider,
                            cfg.model
                        );
                        eprintln!(
                            "note: API key will be stored plaintext on the stick at EFI/bia/fmm.json"
                        );
                        Some(cfg)
                    }
                    None if fmm.is_some() => {
                        bail!(
                            "FMM file not found: {}",
                            fmm.as_ref().unwrap().display()
                        );
                    }
                    None => {
                        eprintln!(
                            "note: no local fmm.json (looked for {}); stick will have no cloud keys",
                            LOCAL_FMM_CANDIDATES.join(", ")
                        );
                        None
                    }
                }
            };
            let chatgpt_cfg = fmm.clone().filter(|c| c.chatgpt.is_some());
            let greeting = match fmm.as_ref().filter(|_| !no_test) {
                Some(cfg) => {
                    eprintln!("== waking {} {} …", cfg.provider, cfg.model);
                    let g = test_model(cfg).context("FMM model test failed (fix fmm.json, or --no-test)")?;
                    eprintln!("\n  bia: {}\n", g.hello);
                    Some(g)
                }
                None => None,
            };
            if !yes {
                eprint!(
                    "This ERASES {} and writes bia. Type the path to continue: ",
                    disk.display()
                );
                io::stderr().flush().ok();
                let mut line = String::new();
                io::stdin().read_line(&mut line)?;
                if line.trim() != disk.to_string_lossy() {
                    bail!("aborted");
                }
            }
            let (image, image_sha256) = match os_image {
                Some(p) => (p, None),
                None => {
                    let m = bia_flash::release::manifest(&os)?;
                    eprintln!("== bia {} ({}) for {arch} from GitHub", m.version, m.tag);
                    let f = bia_flash::release::fetch(&m, &arch, &mut |msg| eprintln!("== {msg}"))?;
                    (f.path, Some(f.raw_sha256))
                }
            };
            let opts = FlashOptions {
                disk: disk.clone(),
                image,
                image_sha256,
                size_bytes: size,
                fmm,
                create_image: image_file,
                model,
                hf_token,
                lang: Some(lang.code),
            };
            write_stick(
                opts,
                Box::new(|msg| {
                    eprintln!("== {msg}");
                }),
            )
            .with_context(|| format!("flash {}", disk.display()))?;
            if let Some(g) = greeting {
                eprintln!("\n  bia: {}\n", g.ready);
            }
            // The stick now owns the ChatGPT session and rotates its tokens; a second stick
            // from the same copy would break both. Drop the tokens from the local file.
            if let (Some(cfg), Some(path)) = (chatgpt_cfg, fmm_file) {
                let mut cfg = cfg;
                cfg.chatgpt = None;
                cfg.api_key.clear();
                if let Ok(mut f) = chatgpt::open_private(&path) {
                    let _ = f.write_all(&serde_json::to_vec_pretty(&cfg).unwrap_or_default());
                }
                eprintln!("note: the ChatGPT sign-in now lives on this stick; run `bia-flash chatgpt` again for another one");
            }
            Ok(())
        }
    }
}
