use crate::blockio::BlockIo;
use crate::fmm::{fmm_config_json, FmmInput, ESP_FMM_REL};
use crate::hf::{self, HfModel};
use anyhow::{bail, Context, Result};
use fatfs::{format_volume, Dir, FileSystem, FormatVolumeOptions, FsOptions};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub type ProgressFn = Box<dyn FnMut(&str)>;

/// Model list on the ESP, read by `bia-grow` on first boot. One line per file:
/// `<byte offset inside BIAROOT> <size> <sha256 or -> <name>`.
pub const ESP_MODELS_REL: &str = "EFI/bia/models.txt";

/// Free space between root.img and the model at the end of BIAROOT. bia-grow moves the
/// model into ext4 through it in steps, so it bounds the step size, not the model size.
pub const MODEL_HEADROOM: u64 = 512 * 1024 * 1024;

pub struct FlashOptions {
    /// Whole-disk path (`/dev/rdiskN`, `/dev/sdb`, `\\.\PhysicalDriveN`) or a raw image file.
    pub disk: PathBuf,
    /// Prebuilt Debian root image (`dist/$ARCH/root.img`).
    pub root_img: PathBuf,
    /// Directory whose `EFI/` and optional `boot/` are copied onto the ESP (typically `flash/`).
    pub esp_src: PathBuf,
    /// Kernel + initramfs directory (often same as root_img's parent).
    pub kern_dir: PathBuf,
    /// Stick capacity in bytes. Required for image files; for block devices may be 0 to probe.
    pub size_bytes: u64,
    /// Target arch for pruning the wrong EFI stubs (`x86_64` | `arm64`).
    pub arch: String,
    /// Optional FMM keys written to `EFI/bia/fmm.json`.
    pub fmm: Option<FmmInput>,
    /// Create the target as a sparse image file when it does not exist.
    pub create_image: bool,
    /// GGUF from Hugging Face, streamed onto the end of BIAROOT for bia-grow to move in.
    pub model: Option<HfModel>,
    /// Hugging Face token for gated/private repos (never written to the stick).
    pub hf_token: Option<String>,
}

/// Bytes the model takes at the end of BIAROOT (each file starts on a 1 MiB boundary).
fn model_span(model: Option<&HfModel>) -> u64 {
    model.map_or(0, |m| {
        m.files
            .iter()
            .map(|f| f.size.div_ceil(bia_disk::ALIGN) * bia_disk::ALIGN)
            .sum()
    })
}

/// Write a bootable bia stick (or image). Erases the target GPT.
pub fn write_stick(opts: FlashOptions, mut progress: ProgressFn) -> Result<()> {
    if !opts.root_img.is_file() {
        bail!("missing root image {}", opts.root_img.display());
    }
    let root_bytes = fs::metadata(&opts.root_img)?.len();
    let esp_efi = opts.esp_src.join("EFI");
    if !esp_efi.is_dir() {
        bail!("missing ESP tree {}/EFI", opts.esp_src.display());
    }
    let kern_vmlinuz = opts.kern_dir.join("vmlinuz-lts");
    let kern_initrd = opts.kern_dir.join("initramfs-lts");
    if !kern_vmlinuz.is_file() || !kern_initrd.is_file() {
        bail!(
            "missing kernel files in {} (need vmlinuz-lts + initramfs-lts)",
            opts.kern_dir.display()
        );
    }

    let mut size = opts.size_bytes;
    if size == 0 {
        size = probe_size(&opts.disk).context("could not determine disk size; pass --size")?;
    }
    let span = model_span(opts.model.as_ref());
    let need = root_bytes
        + bia_disk::BOOT_SIZE
        + bia_disk::BIOS_SIZE
        + 8 * bia_disk::ALIGN
        + if span > 0 { span + MODEL_HEADROOM } else { 0 };
    if size < need {
        bail!(
            "target too small ({} MiB); need at least {} MiB for this root.img{}",
            size / (1024 * 1024),
            (need + 1024 * 1024 - 1) / (1024 * 1024),
            if span > 0 { " and model" } else { "" }
        );
    }

    progress(&format!("opening {}", opts.disk.display()));
    let mut file = BlockIo::new(open_target(&opts.disk, size, opts.create_image)?);

    progress("writing GPT (ESP + BIOS + BIAROOT)");
    let layout = bia_disk::write_gpt(&mut file, size)?;
    if root_bytes > layout.root_len {
        bail!(
            "root.img ({} bytes) larger than BIAROOT partition ({} bytes)",
            root_bytes,
            layout.root_len
        );
    }
    progress(&format!(
        "BIAROOT spans {} MiB; ext4 grows to fill it on first boot",
        layout.root_len >> 20
    ));
    // The ESP is written last. Until then an old boot sector here must not boot.
    file.seek(SeekFrom::Start(layout.boot_start))?;
    file.write_all(&vec![0u8; bia_disk::ALIGN as usize])?;

    // The ESP is built in memory and written in one sequential pass: FAT formatting and
    // file copies make thousands of small scattered writes, which crawl on USB sticks.
    progress("formatting ESP (FAT32 BIABOOT)");
    let esp_len = usize::try_from(layout.boot_len).context("ESP size")?;
    let mut esp = io::Cursor::new(vec![0u8; esp_len]);
    {
        let label = *b"BIABOOT    ";
        format_volume(
            &mut esp,
            FormatVolumeOptions::new()
                .fat_type(fatfs::FatType::Fat32)
                .volume_label(label),
        )
        .context("format ESP")?;
    }

    progress("copying boot files onto ESP");
    {
        let fs = FileSystem::new(&mut esp, FsOptions::new()).context("open FAT")?;
        let root = fs.root_dir();
        let mut jobs = Vec::new();
        esp_jobs(&esp_efi, Path::new("EFI"), &mut jobs);
        let boot_src = opts.esp_src.join("boot");
        if boot_src.is_dir() {
            esp_jobs(&boot_src, Path::new("boot"), &mut jobs);
        }
        jobs.push((Some(kern_vmlinuz), PathBuf::from("vmlinuz-lts")));
        jobs.push((Some(kern_initrd), PathBuf::from("initramfs-lts")));
        let rescue = opts.kern_dir.join("rescue.cpio.gz");
        if rescue.is_file() {
            jobs.push((Some(rescue), PathBuf::from("bia-rescue/rescue.cpio.gz")));
        }
        let total = jobs
            .iter()
            .filter_map(|(src, _)| src.as_ref())
            .map(|p| fs::metadata(p).map(|m| m.len()).unwrap_or(0))
            .sum();
        let mut meter = Meter::new("ESP files".into(), total, &mut progress);
        for (src, dest) in &jobs {
            match src {
                Some(src) => write_fat_file(&root, dest, src, &mut meter)?,
                None => {
                    ensure_fat_dir(&root, dest)?;
                }
            }
        }
        prune_wrong_arch(&root, &opts.arch)?;
        if let Some(ref fmm) = opts.fmm {
            let bytes = fmm_config_json(fmm)?;
            let dir = ensure_fat_dir(&root, Path::new("EFI/bia"))?;
            let mut f = dir.create_file("fmm.json").context("create fmm.json")?;
            f.write_all(&bytes).context("write fmm.json")?;
            progress(&format!(
                "wrote ESP {ESP_FMM_REL} (provider={}, model={})",
                fmm.provider, fmm.model
            ));
        }
        // drop fs to flush
    }
    progress(&format!(
        "writing BIAROOT ({} MiB)",
        root_bytes / (1024 * 1024)
    ));
    copy_at_offset(
        &mut file,
        &opts.root_img,
        layout.root_start,
        root_bytes,
        &mut Meter::new("BIAROOT".into(), root_bytes, &mut progress),
    )?;
    file.sync_all().ok();

    if let Some(model) = &opts.model {
        let list = write_model(&mut file, &layout, root_bytes, model, opts.hf_token.as_deref(), &mut progress)?;
        esp.set_position(0);
        let fs = FileSystem::new(&mut esp, FsOptions::new()).context("open FAT")?;
        let dir = ensure_fat_dir(&fs.root_dir(), Path::new("EFI/bia"))?;
        let mut f = dir.create_file("models.txt").context("create models.txt")?;
        f.truncate()?;
        f.write_all(list.as_bytes()).context("write models.txt")?;
        progress(&format!("wrote ESP {ESP_MODELS_REL}"));
    }

    progress("writing ESP");
    {
        let esp = esp.into_inner();
        let mut meter = Meter::new("ESP".into(), esp.len() as u64, &mut progress);
        file.seek(SeekFrom::Start(layout.boot_start))?;
        for chunk in esp.chunks(8 * 1024 * 1024) {
            file.write_all(chunk).context("write ESP")?;
            meter.add(chunk.len() as u64);
        }
    }
    file.sync_all().ok();

    if opts.arch == "x86_64" {
        let i386 = opts.esp_src.join("boot/grub/i386-pc");
        match (std::fs::read(i386.join("boot.img")), std::fs::read(i386.join("core.img"))) {
            (Ok(boot), Ok(core)) => {
                progress("embedding BIOS GRUB (legacy boot)");
                bia_disk::embed_bios_grub(&mut file, &layout, &boot, &core).context("embed BIOS GRUB")?;
                file.sync_all().ok();
            }
            _ => progress("no GRUB i386-pc images; this stick boots on UEFI only"),
        }
    }

    progress("done");
    Ok(())
}

/// Stream `model` from the Hub onto the end of BIAROOT and return the `models.txt` body.
fn write_model(
    file: &mut BlockIo,
    layout: &bia_disk::Layout,
    root_bytes: u64,
    model: &HfModel,
    token: Option<&str>,
    progress: &mut ProgressFn,
) -> Result<String> {
    let span = model_span(Some(model));
    let start = (layout.root_len - span) / bia_disk::ALIGN * bia_disk::ALIGN;
    let root_end = root_bytes.div_ceil(bia_disk::ALIGN) * bia_disk::ALIGN;
    if start < root_end + MODEL_HEADROOM {
        bail!(
            "stick too small for {} ({} MiB) next to root.img",
            model.label(),
            model.size() >> 20
        );
    }
    let mut list = format!("# {} from huggingface.co/{}\n", model.label(), model.repo);
    let mut off = start;
    for f in &model.files {
        progress(&format!(
            "downloading {}/{} ({} MiB) onto the stick",
            model.repo,
            f.path,
            f.size >> 20
        ));
        file.seek(SeekFrom::Start(layout.root_start + off))?;
        // Network reads arrive in odd sizes; batch them so the raw device sees big aligned writes.
        let mut pending: Vec<u8> = Vec::with_capacity(8 * 1024 * 1024);
        {
            let mut meter = Meter::new(f.name().to_string(), f.size, progress);
            hf::download(&model.repo, f, token, &mut |chunk| {
                pending.extend_from_slice(chunk);
                if pending.len() >= 8 * 1024 * 1024 {
                    file.write_all(&pending).context("write model")?;
                    pending.clear();
                }
                meter.add(chunk.len() as u64);
                Ok(())
            })?;
        }
        file.write_all(&pending).context("write model")?;
        file.sync_all().ok();
        let sum = if f.sha256.is_empty() { "-" } else { f.sha256.as_str() };
        list += &format!("{off} {} {sum} {}\n", f.size, f.name());
        off += f.size.div_ceil(bia_disk::ALIGN) * bia_disk::ALIGN;
    }
    Ok(list)
}

fn open_target(path: &Path, size: u64, create_image: bool) -> Result<File> {
    let is_dev = path.starts_with("/dev")
        || path
            .to_string_lossy()
            .starts_with(r"\\.\PhysicalDrive");
    if create_image || (!is_dev && !path.exists()) {
        let f = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
            .with_context(|| format!("create {}", path.display()))?;
        f.set_len(size)?;
        return Ok(f);
    }
    #[cfg(target_os = "macos")]
    if is_dev {
        unmount_disk(path)?;
    }
    let opened = OpenOptions::new().read(true).write(true).open(path);
    // Not root (the GUI): macOS asks for an admin password and hands back the open device.
    #[cfg(target_os = "macos")]
    let opened = match opened {
        Err(e) if is_dev && e.kind() == std::io::ErrorKind::PermissionDenied => authopen(path),
        r => r.map_err(Into::into),
    };
    opened.with_context(|| {
        format!(
            "open {} (need root/admin for raw disks)",
            path.display()
        )
    })
}

/// Unmount every volume of the stick (BIABOOT is auto-mounted) before raw writes.
#[cfg(target_os = "macos")]
fn unmount_disk(path: &Path) -> Result<()> {
    let name = path.to_string_lossy().replacen("/dev/rdisk", "/dev/disk", 1);
    let out = std::process::Command::new("diskutil")
        .args(["unmountDisk", &name])
        .output()
        .context("diskutil unmountDisk")?;
    if !out.status.success() {
        bail!(
            "could not unmount {name} (close any files open on the stick): {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

/// Open a raw disk read-write via `/usr/libexec/authopen`, which shows the system admin prompt
/// and passes the descriptor back over a socket (`-stdoutpipe`, SCM_RIGHTS).
#[cfg(target_os = "macos")]
fn authopen(path: &Path) -> Result<File> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::net::UnixStream;

    let (ours, theirs) = UnixStream::pair().context("socketpair")?;
    let mut child = std::process::Command::new("/usr/libexec/authopen")
        .args(["-stdoutpipe", "-o", &libc::O_RDWR.to_string()])
        .arg(path)
        .stdout(std::process::Stdio::from(OwnedFd::from(theirs)))
        .spawn()
        .context("run /usr/libexec/authopen")?;

    let mut fd = None;
    let mut data = [0u8; 256];
    // u64s keep the control buffer aligned for cmsghdr.
    let mut control = [0u64; 8];
    loop {
        let mut iov = libc::iovec {
            iov_base: data.as_mut_ptr().cast(),
            iov_len: data.len(),
        };
        // SAFETY: msghdr is plain data; every pointer set below outlives the recvmsg call.
        let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = control.as_mut_ptr().cast();
        msg.msg_controllen = std::mem::size_of_val(&control) as _;
        // SAFETY: valid socket and msghdr.
        let n = unsafe { libc::recvmsg(ours.as_raw_fd(), &mut msg, 0) };
        if n < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            let _ = child.kill();
            let _ = child.wait();
            return Err(e).context("read from authopen");
        }
        // SAFETY: walking the control messages recvmsg just filled in.
        unsafe {
            let mut c = libc::CMSG_FIRSTHDR(&msg);
            while !c.is_null() {
                if (*c).cmsg_level == libc::SOL_SOCKET && (*c).cmsg_type == libc::SCM_RIGHTS {
                    let raw = std::ptr::read_unaligned(libc::CMSG_DATA(c) as *const libc::c_int);
                    fd = Some(OwnedFd::from_raw_fd(raw));
                }
                c = libc::CMSG_NXTHDR(&msg, c);
            }
        }
        if fd.is_some() || n == 0 {
            break;
        }
    }
    let status = child.wait().context("wait for authopen")?;
    match fd {
        Some(fd) => Ok(File::from(fd)),
        None if !status.success() => bail!("administrator access was not granted"),
        None => bail!("authopen returned no device"),
    }
}

fn probe_size(path: &Path) -> Result<u64> {
    #[cfg(target_os = "linux")]
    {
        if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
            let sys = PathBuf::from(format!("/sys/block/{name}/size"));
            if let Ok(s) = fs::read_to_string(&sys) {
                if let Ok(sectors) = s.trim().parse::<u64>() {
                    return Ok(sectors * 512);
                }
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
            let id = name.trim_start_matches('r');
            let out = std::process::Command::new("diskutil")
                .args(["info", "-plist", id])
                .output()?;
            let json = std::process::Command::new("plutil")
                .args(["-convert", "json", "-o", "-", "--", "-"])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .spawn()
                .and_then(|mut child| {
                    use std::io::Write;
                    if let Some(mut stdin) = child.stdin.take() {
                        stdin.write_all(&out.stdout)?;
                    }
                    child.wait_with_output()
                })?;
            let v: serde_json::Value = serde_json::from_slice(&json.stdout)?;
            if let Some(n) = v.get("TotalSize").and_then(|s| s.as_u64()) {
                return Ok(n);
            }
        }
    }
    let mut f = File::open(path)?;
    let len = f.seek(SeekFrom::End(0))?;
    if len == 0 {
        bail!("size probe returned 0");
    }
    Ok(len)
}

fn copy_at_offset(
    dst: &mut BlockIo,
    src: &Path,
    offset: u64,
    len: u64,
    meter: &mut Meter,
) -> Result<()> {
    let path = src.display().to_string();
    let mut src = File::open(src).with_context(|| format!("open {path}"))?;
    dst.seek(SeekFrom::Start(offset))?;
    let mut buf = vec![0u8; 8 * 1024 * 1024];
    let mut left = len;
    while left > 0 {
        let chunk = buf.len().min(left as usize);
        let n = src.read(&mut buf[..chunk])?;
        if n == 0 {
            bail!("unexpected EOF reading {path}");
        }
        dst.write_all(&buf[..n])?;
        left -= n as u64;
        meter.add(n as u64);
    }
    Ok(())
}

fn ensure_fat_dir<'a, IO: fatfs::ReadWriteSeek + 'a>(
    root: &Dir<'a, IO>,
    rel: &Path,
) -> Result<Dir<'a, IO>> {
    let mut cur = root.clone();
    for comp in rel.components() {
        let name = comp.as_os_str().to_string_lossy();
        if name == "." || name.is_empty() {
            continue;
        }
        cur = match cur.open_dir(name.as_ref()) {
            Ok(d) => d,
            Err(_) => cur.create_dir(name.as_ref()).with_context(|| {
                format!("create FAT dir {}", rel.display())
            })?,
        };
    }
    Ok(cur)
}

fn write_fat_file<IO: fatfs::ReadWriteSeek>(
    root: &Dir<'_, IO>,
    rel: &Path,
    src: &Path,
    meter: &mut Meter,
) -> Result<()> {
    if let Some(parent) = rel.parent().filter(|p| !p.as_os_str().is_empty()) {
        let dir = ensure_fat_dir(root, parent)?;
        let name = rel
            .file_name()
            .and_then(|s| s.to_str())
            .context("file name")?;
        return write_fat_file_in(&dir, name, src, meter);
    }
    let name = rel
        .file_name()
        .and_then(|s| s.to_str())
        .context("file name")?;
    write_fat_file_in(root, name, src, meter)
}

fn write_fat_file_in<IO: fatfs::ReadWriteSeek>(
    dir: &Dir<'_, IO>,
    name: &str,
    src: &Path,
    meter: &mut Meter,
) -> Result<()> {
    let data = fs::read(src).with_context(|| format!("read {}", src.display()))?;
    let mut f = dir
        .create_file(name)
        .with_context(|| format!("create FAT file {name}"))?;
    for chunk in data.chunks(4 * 1024 * 1024) {
        f.write_all(chunk)
            .with_context(|| format!("write FAT file {name}"))?;
        meter.add(chunk.len() as u64);
    }
    Ok(())
}

/// Files (`Some(src)`) and directories (`None`) of `src_dir`, mapped under `dest_rel` on the ESP.
fn esp_jobs(src_dir: &Path, dest_rel: &Path, jobs: &mut Vec<(Option<PathBuf>, PathBuf)>) {
    for entry in walkdir::WalkDir::new(src_dir).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        let rel = path.strip_prefix(src_dir).unwrap_or(path);
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        // Skip per-arch kernel copies that would fill the ESP (same as flash.sh).
        if name.starts_with("vmlinuz-lts-") || name.starts_with("initramfs-lts-") {
            continue;
        }
        let dest = dest_rel.join(rel);
        if entry.file_type().is_dir() {
            jobs.push((None, dest));
        } else if entry.file_type().is_file() {
            jobs.push((Some(path.to_path_buf()), dest));
        }
    }
}

/// Reports `what: N% (done/total MiB, speed, ETA)` every 5% or every 3 seconds.
struct Meter<'p> {
    what: String,
    total: u64,
    done: u64,
    start: Instant,
    last: Instant,
    progress: &'p mut ProgressFn,
}

impl<'p> Meter<'p> {
    fn new(what: String, total: u64, progress: &'p mut ProgressFn) -> Self {
        let now = Instant::now();
        Self {
            what,
            total: total.max(1),
            done: 0,
            start: now,
            last: now,
            progress,
        }
    }

    fn add(&mut self, n: u64) {
        let step = |d: u64| d * 20 / self.total;
        let before = step(self.done);
        self.done = (self.done + n).min(self.total);
        if step(self.done) == before && self.last.elapsed() < Duration::from_secs(3) {
            return;
        }
        self.last = Instant::now();
        let secs = self.start.elapsed().as_secs_f64().max(0.001);
        let rate = self.done as f64 / secs;
        let mut msg = format!(
            "{}: {}% ({}/{} MiB, {:.1} MiB/s",
            self.what,
            self.done * 100 / self.total,
            self.done >> 20,
            self.total >> 20,
            rate / (1024.0 * 1024.0)
        );
        if self.done < self.total && rate > 0.0 {
            let eta = ((self.total - self.done) as f64 / rate) as u64;
            msg += &format!(", ~{}:{:02} left", eta / 60, eta % 60);
        }
        msg.push(')');
        (self.progress)(&msg);
    }
}

/// Split a progress line of the form `<step>: <pct>% (<detail>)` (from `Meter` and the release
/// download) into step, fraction 0..=1 and detail. Other lines give `None`.
pub fn parse_progress(line: &str) -> Option<(&str, f32, &str)> {
    let i = line.find("% (")?;
    let head = &line[..i];
    let j = head.rfind(": ")?;
    let pct: u32 = head[j + 2..].trim().parse().ok()?;
    let detail = line[i + 3..].strip_suffix(')').unwrap_or(&line[i + 3..]);
    Some((&head[..j], pct.min(100) as f32 / 100.0, detail))
}

#[cfg(test)]
mod progress_tests {
    use super::parse_progress;

    #[test]
    fn parses_meter_and_download_lines() {
        assert_eq!(
            parse_progress("writing BIAROOT: 45% (1382/3072 MiB, 31.2 MiB/s, ~0:54 left)"),
            Some(("writing BIAROOT", 0.45, "1382/3072 MiB, 31.2 MiB/s, ~0:54 left"))
        );
        assert_eq!(
            parse_progress("downloading root.img.zst: 7% (12 / 900 MiB)"),
            Some(("downloading root.img.zst", 0.07, "12 / 900 MiB"))
        );
        assert_eq!(parse_progress("formatting ESP (FAT32 BIABOOT)"), None);
    }
}

fn prune_wrong_arch<IO: fatfs::ReadWriteSeek>(root: &Dir<'_, IO>, arch: &str) -> Result<()> {
    let remove: &[&str] = match arch {
        "arm64" | "aarch64" => &[
            "EFI/BOOT/BOOTX64.EFI",
            "EFI/BOOT/grubx64.efi",
            "EFI/BOOT/mmx64.efi",
            "EFI/debian/BOOTX64.EFI",
            "EFI/debian/grubx64.efi",
        ],
        "x86_64" | "amd64" => &[
            "EFI/BOOT/BOOTAA64.EFI",
            "EFI/BOOT/grubaa64.efi",
            "EFI/BOOT/mmaa64.efi",
            "EFI/debian/BOOTAA64.EFI",
            "EFI/debian/grubaa64.efi",
        ],
        _ => return Ok(()),
    };
    for rel in remove {
        let path = Path::new(rel);
        let Some(parent) = path.parent() else {
            continue;
        };
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if let Ok(dir) = open_existing_dir(root, parent) {
            let _ = dir.remove(name);
        }
    }
    Ok(())
}

fn open_existing_dir<'a, IO: fatfs::ReadWriteSeek + 'a>(
    root: &Dir<'a, IO>,
    rel: &Path,
) -> Result<Dir<'a, IO>> {
    let mut cur = root.clone();
    for comp in rel.components() {
        let name = comp.as_os_str().to_string_lossy();
        if name == "." || name.is_empty() {
            continue;
        }
        cur = cur
            .open_dir(name.as_ref())
            .with_context(|| format!("open {}", rel.display()))?;
    }
    Ok(cur)
}
