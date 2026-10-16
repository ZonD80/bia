use crate::blockio::BlockIo;
use crate::fmm::{fmm_config_json, FmmInput, ESP_FMM_REL};
use crate::hf::{self, HfModel};
use crate::image::{self, ImageReader, HEAD};
use anyhow::{bail, Context, Result};
use fatfs::{Dir, FileSystem, FsOptions};
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
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
    /// bia disk image, raw or xz (`bia-<arch>.img[.xz]`: a release download or `bia-flash image`).
    pub image: PathBuf,
    /// sha256 of the raw image, checked before the ESP is written (release images).
    pub image_sha256: Option<String>,
    /// Stick capacity in bytes. Required for image files; for block devices may be 0 to probe.
    pub size_bytes: u64,
    /// Optional FMM keys written to `EFI/bia/fmm.json`.
    pub fmm: Option<FmmInput>,
    /// Create the target as a sparse image file when it does not exist.
    pub create_image: bool,
    /// GGUF from Hugging Face, streamed onto the end of BIAROOT for bia-grow to move in.
    pub model: Option<HfModel>,
    /// Hugging Face token for gated/private repos (never written to the stick).
    pub hf_token: Option<String>,
    /// The owner's language (catalog code), written to `EFI/bia/lang.json`.
    pub lang: Option<String>,
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

/// Write the bia image onto a stick (or image file) with its partitions over the whole
/// disk, then the optional model and FMM keys. Erases the target.
///
/// The image is read once, front to back (it may be xz): its ESP is kept in memory, the
/// BIOS boot partition and BIAROOT are copied, and the ESP — with `fmm.json` and
/// `models.txt` added — is written last, so an interrupted write never boots.
pub fn write_stick(opts: FlashOptions, mut progress: ProgressFn) -> Result<()> {
    let mut img = ImageReader::open(&opts.image)?;
    let mut hash = opts.image_sha256.as_ref().map(|_| Sha256::new());
    let mut seen = |b: &[u8]| {
        if let Some(h) = hash.as_mut() {
            h.update(b);
        }
    };
    let mut head = vec![0u8; HEAD as usize];
    img.read_exact_at_pos(&mut head)?;
    seen(&head);
    let il = image::parse_layout(&head).with_context(|| format!("read {}", opts.image.display()))?;

    let mut size = opts.size_bytes;
    if size == 0 {
        size = probe_size(&opts.disk).context("could not determine disk size; pass --size")?;
    }
    let span = model_span(opts.model.as_ref());
    let want_root = il.root_len + if span > 0 { span + MODEL_HEADROOM } else { 0 };
    if bia_disk::root_len(size) < want_root {
        let need = size - bia_disk::root_len(size).min(size) + want_root;
        bail!(
            "target too small ({} MiB); need at least {} MiB for this system{}",
            size >> 20,
            need.div_ceil(1 << 20),
            if span > 0 { " and model" } else { "" }
        );
    }

    progress(&format!("opening {}", opts.disk.display()));
    let mut file = BlockIo::new(open_target(&opts.disk, size, opts.create_image)?);

    progress("writing GPT (ESP + BIOS + BIAROOT)");
    let layout = bia_disk::write_gpt(&mut file, size)?;
    if (layout.boot_start, layout.boot_len, layout.bios_start, layout.bios_len, layout.root_start)
        != (il.boot_start, il.boot_len, il.bios_start, il.bios_len, il.root_start)
    {
        bail!("the image's partitions do not match this writer's layout (newer image? update bia-flash)");
    }
    progress(&format!(
        "BIAROOT spans {} MiB; ext4 grows to fill it on first boot",
        layout.root_len >> 20
    ));
    // MBR boot code (GRUB boot.img on x86_64) comes from the image; the partition
    // table part of the protective MBR is the one just written for this disk.
    let mut mbr = [0u8; 512];
    file.seek(SeekFrom::Start(0))?;
    file.read_exact(&mut mbr).context("read MBR")?;
    mbr[..440].copy_from_slice(&head[..440]);
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&mbr).context("write MBR")?;
    // The ESP is written last. Until then an old boot sector here must not boot.
    file.seek(SeekFrom::Start(layout.boot_start))?;
    file.write_all(&vec![0u8; bia_disk::ALIGN as usize])?;

    progress("reading the ESP from the image");
    img.skip_to(il.boot_start, &mut seen)?;
    let mut esp = io::Cursor::new(vec![0u8; usize::try_from(il.boot_len).context("ESP size")?]);
    img.read_exact_at_pos(esp.get_mut())?;
    seen(esp.get_ref());

    img.skip_to(il.bios_start, &mut seen)?;
    let mut bios = vec![0u8; il.bios_len as usize];
    img.read_exact_at_pos(&mut bios)?;
    seen(&bios);
    file.seek(SeekFrom::Start(layout.bios_start))?;
    file.write_all(&bios).context("write BIOS boot partition")?;

    img.skip_to(il.root_start, &mut seen)?;
    progress(&format!("writing BIAROOT ({} MiB)", il.root_len >> 20));
    file.seek(SeekFrom::Start(layout.root_start))?;
    {
        let mut meter = Meter::new("BIAROOT".into(), il.root_len, &mut progress);
        let mut buf = vec![0u8; 8 << 20];
        let mut left = il.root_len;
        while left > 0 {
            let n = buf.len().min(left as usize);
            img.read_exact_at_pos(&mut buf[..n])?;
            seen(&buf[..n]);
            file.write_all(&buf[..n]).context("write BIAROOT")?;
            left -= n as u64;
            meter.add(n as u64);
        }
    }
    img.drain(&mut seen)?;
    file.sync_all().ok();
    if let (Some(h), Some(want)) = (hash, opts.image_sha256.as_ref()) {
        let got: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if !got.eq_ignore_ascii_case(want) {
            bail!("the bia image does not match its release checksum; the stick was not made bootable (download it again)");
        }
        progress("image checksum ok");
    }

    if let Some(model) = &opts.model {
        let list = write_model(&mut file, &layout, il.root_len, model, opts.hf_token.as_deref(), &mut progress)?;
        add_esp_file(&mut esp, ESP_MODELS_REL, list.as_bytes())?;
        progress(&format!("wrote ESP {ESP_MODELS_REL}"));
    }
    if let Some(ref fmm) = opts.fmm {
        add_esp_file(&mut esp, ESP_FMM_REL, &fmm_config_json(fmm)?)?;
        progress(&format!(
            "wrote ESP {ESP_FMM_REL} (provider={}, model={})",
            fmm.provider, fmm.model
        ));
    }

    if let Some(code) = &opts.lang {
        add_esp_file(&mut esp, crate::lang::ESP_LANG_REL, &crate::lang::lang_json(code))?;
        progress(&format!("wrote ESP {} ({code})", crate::lang::ESP_LANG_REL));
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
    progress("done");
    Ok(())
}

/// Create or replace `rel` (e.g. `EFI/bia/fmm.json`) in the in-memory ESP.
fn add_esp_file(esp: &mut io::Cursor<Vec<u8>>, rel: &str, data: &[u8]) -> Result<()> {
    esp.set_position(0);
    let fs = FileSystem::new(&mut *esp, FsOptions::new()).context("open ESP")?;
    let rel = Path::new(rel);
    let dir = match rel.parent() {
        Some(p) if !p.as_os_str().is_empty() => ensure_fat_dir(&fs.root_dir(), p)?,
        _ => fs.root_dir(),
    };
    let name = rel.file_name().and_then(|s| s.to_str()).context("file name")?;
    let mut f = dir.create_file(name).with_context(|| format!("create {}", rel.display()))?;
    f.truncate()?;
    f.write_all(data).with_context(|| format!("write {}", rel.display()))?;
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
            if let Ok(s) = std::fs::read_to_string(&sys) {
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

pub(crate) fn ensure_fat_dir<'a, IO: fatfs::ReadWriteSeek + 'a>(
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

/// Reports `what: N% (done/total MiB, speed, ETA)` every 5% or every 3 seconds.
pub(crate) struct Meter<'p> {
    what: String,
    total: u64,
    done: u64,
    start: Instant,
    last: Instant,
    progress: &'p mut ProgressFn,
}

impl<'p> Meter<'p> {
    pub(crate) fn new(what: String, total: u64, progress: &'p mut ProgressFn) -> Self {
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

    pub(crate) fn add(&mut self, n: u64) {
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

#[cfg(test)]
pub(crate) mod image_tests {
    use super::*;
    use crate::image::{build_image, parse_layout, raw_size, ImageSources};
    use std::fs;

    /// A fake local build: root.img with a byte pattern, an ESP tree with both
    /// architectures' stubs, BIOS GRUB images and kernel files.
    pub(crate) fn fake_build(dir: &Path) -> ImageSources {
        let flash = dir.join("flash");
        for d in ["EFI/BOOT", "EFI/debian", "boot/grub/i386-pc"] {
            fs::create_dir_all(flash.join(d)).unwrap();
        }
        for f in ["EFI/BOOT/BOOTX64.EFI", "EFI/BOOT/BOOTAA64.EFI", "EFI/debian/grub.cfg", "EFI/vmlinuz-lts-x86_64"] {
            fs::write(flash.join(f), f.as_bytes()).unwrap();
        }
        let mut boot = vec![0u8; 512];
        boot[0] = 0xEB;
        boot[0x66] = 0xEB;
        boot[0x67] = 0x05;
        boot[0x180..0x184].copy_from_slice(b"GRUB");
        let mut core = vec![0x11u8; 3 * 512];
        core[500..508].copy_from_slice(&2u64.to_le_bytes());
        core[508..510].copy_from_slice(&2u16.to_le_bytes());
        fs::write(flash.join("boot/grub/i386-pc/boot.img"), &boot).unwrap();
        fs::write(flash.join("boot/grub/i386-pc/core.img"), &core).unwrap();
        let dist = dir.join("dist");
        fs::create_dir_all(&dist).unwrap();
        fs::write(dist.join("vmlinuz-lts"), b"kernel").unwrap();
        fs::write(dist.join("initramfs-lts"), b"initrd").unwrap();
        fs::write(dist.join("rescue.cpio.gz"), b"rescue").unwrap();
        let root: Vec<u8> = (0..(5u32 << 20) + 4096).map(|i| (i % 251) as u8).collect();
        fs::write(dist.join("root.img"), &root).unwrap();
        ImageSources { root_img: dist.join("root.img"), esp_src: flash, kern_dir: dist, arch: "x86_64".into() }
    }

    pub(crate) fn xz(src: &Path, dst: &Path) {
        let stream = liblzma::stream::MtStreamBuilder::new()
            .threads(2)
            .block_size(1 << 20) // several blocks, as `xz -T0` writes
            .preset(1)
            .encoder()
            .unwrap();
        let mut enc = liblzma::write::XzEncoder::new_stream(File::create(dst).unwrap(), stream);
        io::copy(&mut File::open(src).unwrap(), &mut enc).unwrap();
        enc.finish().unwrap();
    }

    fn sha(path: &Path) -> String {
        Sha256::digest(fs::read(path).unwrap()).iter().map(|b| format!("{b:02x}")).collect()
    }

    fn esp_file(disk: &[u8], l: &bia_disk::Layout, rel: &str) -> Option<Vec<u8>> {
        let esp = disk[l.boot_start as usize..(l.boot_start + l.boot_len) as usize].to_vec();
        let fs = FileSystem::new(io::Cursor::new(esp), FsOptions::new()).unwrap();
        let mut f = fs.root_dir().open_file(rel).ok()?;
        let mut out = Vec::new();
        f.read_to_end(&mut out).unwrap();
        Some(out)
    }

    #[test]
    fn image_builds_and_writes_over_a_bigger_stick() {
        let dir = std::env::temp_dir().join(format!("bia-image-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let src = fake_build(&dir);
        let img = dir.join("bia-x86_64.img");
        let size = build_image(&src, &img, &mut (Box::new(|_: &str| {}) as ProgressFn)).unwrap();
        assert_eq!(fs::metadata(&img).unwrap().len(), size);
        assert_eq!(size, crate::image::min_disk_size(fs::metadata(&src.root_img).unwrap().len()));

        // The image itself: a whole small disk that boots as it is (dd / Etcher).
        let raw = fs::read(&img).unwrap();
        let il = parse_layout(&raw[..HEAD as usize]).unwrap();
        assert_eq!(il.boot_start, HEAD);
        assert!(il.root_start + il.root_len <= size);
        assert_eq!(&raw[0x180..0x184], b"GRUB", "boot.img in the MBR");
        assert_eq!(&raw[0x66..0x68], &[0x90, 0x90], "drive check patched out");
        assert_eq!(raw[il.bios_start as usize + 512], 0x11, "core.img in the BIOS boot partition");
        let root = fs::read(&src.root_img).unwrap();
        assert_eq!(&raw[il.root_start as usize..il.root_start as usize + root.len()], &root[..]);
        assert_eq!(esp_file(&raw, &il, "EFI/BOOT/BOOTX64.EFI").unwrap(), b"EFI/BOOT/BOOTX64.EFI");
        assert!(esp_file(&raw, &il, "EFI/BOOT/BOOTAA64.EFI").is_none(), "arm64 stub pruned");
        assert!(esp_file(&raw, &il, "EFI/vmlinuz-lts-x86_64").is_none(), "per-arch kernel copy skipped");
        assert_eq!(esp_file(&raw, &il, "vmlinuz-lts").unwrap(), b"kernel");
        assert_eq!(esp_file(&raw, &il, "bia-rescue/rescue.cpio.gz").unwrap(), b"rescue");
        assert!(esp_file(&raw, &il, "boot/grub/i386-pc/core.img").is_some());
        assert!(esp_file(&raw, &il, "EFI/bia/fmm.json").is_none(), "no keys in the image");

        // xz, as released; the raw size comes from the xz index.
        let img_xz = dir.join("bia-x86_64.img.xz");
        xz(&img, &img_xz);
        assert_eq!(raw_size(&img_xz).unwrap(), size);
        assert_eq!(raw_size(&img).unwrap(), size);

        // Written by bia-flash onto a 600 MiB "stick" with keys.
        let stick = dir.join("stick.img");
        let stick_size = 600u64 << 20;
        let fmm: FmmInput = serde_json::from_value(serde_json::json!({
            "provider": "openrouter", "api_key": "sk-or-v1-test", "model": "x-ai/grok-4.6"
        }))
        .unwrap();
        let opts = |image: &Path, sha: Option<String>| FlashOptions {
            disk: stick.clone(),
            image: image.to_path_buf(),
            image_sha256: sha,
            size_bytes: stick_size,
            fmm: Some(fmm.clone()),
            create_image: true,
            model: None,
            hf_token: None,
            lang: Some("ru".into()),
        };
        write_stick(opts(&img_xz, Some(sha(&img))), Box::new(|_| {})).unwrap();
        let d = fs::read(&stick).unwrap();
        assert_eq!(d.len() as u64, stick_size);
        let sl = parse_layout(&d[..HEAD as usize]).unwrap();
        assert_eq!((sl.boot_start, sl.bios_start, sl.root_start), (il.boot_start, il.bios_start, il.root_start));
        assert_eq!(sl.root_len, bia_disk::root_len(stick_size), "BIAROOT spans the whole stick");
        assert!(sl.root_len > il.root_len);
        // backup GPT header at the very end of the stick
        assert_eq!(&d[d.len() - 512..d.len() - 504], b"EFI PART");
        assert_eq!(&d[..440], &raw[..440], "MBR boot code from the image");
        assert_eq!(&d[510..512], &[0x55, 0xAA]);
        assert_eq!(&d[sl.bios_start as usize..(sl.bios_start + sl.bios_len) as usize],
                   &raw[il.bios_start as usize..(il.bios_start + il.bios_len) as usize]);
        assert_eq!(&d[sl.root_start as usize..(sl.root_start + il.root_len) as usize],
                   &raw[il.root_start as usize..(il.root_start + il.root_len) as usize]);
        let keys: serde_json::Value = serde_json::from_slice(&esp_file(&d, &sl, "EFI/bia/fmm.json").unwrap()).unwrap();
        assert_eq!(keys["api_key"], "sk-or-v1-test");
        assert_eq!(keys["enabled"], true);
        let lang: serde_json::Value = serde_json::from_slice(&esp_file(&d, &sl, "EFI/bia/lang.json").unwrap()).unwrap();
        assert_eq!(lang["lang"], "ru");
        assert_eq!(esp_file(&d, &sl, "EFI/BOOT/BOOTX64.EFI").unwrap(), b"EFI/BOOT/BOOTX64.EFI");

        // A raw .img works the same; a wrong checksum leaves the stick unbootable (no ESP).
        let err = write_stick(opts(&img, Some("00".repeat(32))), Box::new(|_| {})).unwrap_err();
        assert!(format!("{err:#}").contains("checksum"), "{err:#}");
        let d = fs::read(&stick).unwrap();
        assert!(d[sl.boot_start as usize..(sl.boot_start + bia_disk::ALIGN) as usize].iter().all(|&b| b == 0));

        // Too small a stick is refused before anything is written.
        let small = FlashOptions { size_bytes: size - (4 << 20), disk: dir.join("small.img"), ..opts(&img, None) };
        assert!(format!("{:#}", write_stick(small, Box::new(|_| {})).unwrap_err()).contains("too small"));
        assert!(!dir.join("small.img").exists());

        // A truncated download is caught by the decoder.
        let cut = dir.join("cut.img.xz");
        let z = fs::read(&img_xz).unwrap();
        fs::write(&cut, &z[..z.len() / 2]).unwrap();
        assert!(write_stick(opts(&cut, None), Box::new(|_| {})).is_err());
        fs::remove_dir_all(&dir).ok();
    }
}
