//! The bia disk image: what a release ships (`bia-<arch>.img.xz`) and what the writer puts
//! on a stick. It is a whole small disk — GPT, ESP (FAT32 BIABOOT: shim, GRUB, kernel,
//! initramfs, rescue image), BIOS boot partition with GRUB core.img (x86_64; boot.img in
//! the MBR) and BIAROOT holding `root.img` — only as large as the system needs.
//!
//! Any image writer (Etcher, Rufus, Raspberry Pi Imager, dd) can put it on a stick; on
//! first boot `bia-grow` moves the backup GPT to the end and grows BIAROOT and ext4 over
//! the whole stick. `bia-flash write` instead lays the same partitions over the whole
//! stick straight away and adds FMM keys and a model (`write.rs`).

use anyhow::{bail, Context, Result};
use fatfs::{format_volume, Dir, FileSystem, FormatVolumeOptions, FsOptions};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::write::{ensure_fat_dir, Meter, ProgressFn};

/// GPT header + entries live in the first MiB; the ESP starts after it.
pub const HEAD: u64 = bia_disk::ALIGN;
const XZ_MAGIC: [u8; 6] = [0xFD, b'7', b'z', b'X', b'Z', 0];
const EFI_TYPE: [u8; 16] = guid_le("C12A7328-F81F-11D2-BA4B-00A0C93EC93B");
const BIOS_TYPE: [u8; 16] = guid_le("21686148-6449-6E6F-744E-656564454649");

/// Inputs of `build_image`: a local build's root filesystem, ESP tree and kernel files.
pub struct ImageSources {
    /// BIAROOT ext4 image (`dist/<arch>/root.img`).
    pub root_img: PathBuf,
    /// Tree with `EFI/` and optional `boot/` (`source/flash`).
    pub esp_src: PathBuf,
    /// Directory with `vmlinuz-lts`, `initramfs-lts` and optional `rescue.cpio.gz`.
    pub kern_dir: PathBuf,
    /// `x86_64` or `arm64`: picks the EFI stubs and whether BIOS GRUB is embedded.
    pub arch: String,
}

/// Size of the smallest disk that holds a `root_bytes` root image.
pub fn min_disk_size(root_bytes: u64) -> u64 {
    root_bytes.div_ceil(bia_disk::ALIGN) * bia_disk::ALIGN
        + bia_disk::BOOT_SIZE
        + bia_disk::BIOS_SIZE
        + 8 * bia_disk::ALIGN
}

/// Build the minimal bia disk image at `out` (replaced). Returns its size.
pub fn build_image(src: &ImageSources, out: &Path, progress: &mut ProgressFn) -> Result<u64> {
    if !src.root_img.is_file() {
        bail!("missing root image {}", src.root_img.display());
    }
    let esp_efi = src.esp_src.join("EFI");
    if !esp_efi.is_dir() {
        bail!("missing ESP tree {}/EFI", src.esp_src.display());
    }
    let vmlinuz = src.kern_dir.join("vmlinuz-lts");
    let initrd = src.kern_dir.join("initramfs-lts");
    if !vmlinuz.is_file() || !initrd.is_file() {
        bail!(
            "missing kernel files in {} (need vmlinuz-lts + initramfs-lts)",
            src.kern_dir.display()
        );
    }
    let x86 = matches!(src.arch.as_str(), "x86_64" | "amd64");
    if !x86 && !matches!(src.arch.as_str(), "arm64" | "aarch64") {
        bail!("unknown arch {:?} (x86_64 or arm64)", src.arch);
    }
    let root_bytes = fs::metadata(&src.root_img)?.len();
    let size = min_disk_size(root_bytes);

    let _ = fs::remove_file(out);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(out)
        .with_context(|| format!("create {}", out.display()))?;
    file.set_len(size)?;
    progress(&format!("GPT for a {} MiB image", size >> 20));
    let mut file = file;
    let layout = bia_disk::write_gpt(&mut file, size)?;
    let mut file = io::BufWriter::with_capacity(8 << 20, file);
    if root_bytes > layout.root_len {
        bail!("root.img ({root_bytes} bytes) larger than BIAROOT ({} bytes)", layout.root_len);
    }

    progress("building the ESP (FAT32 BIABOOT)");
    let mut esp = io::Cursor::new(vec![0u8; usize::try_from(layout.boot_len)?]);
    format_volume(
        &mut esp,
        FormatVolumeOptions::new()
            .fat_type(fatfs::FatType::Fat32)
            .volume_label(*b"BIABOOT    "),
    )
    .context("format ESP")?;
    {
        let fs = FileSystem::new(&mut esp, FsOptions::new()).context("open FAT")?;
        let root = fs.root_dir();
        let mut jobs = Vec::new();
        esp_jobs(&esp_efi, Path::new("EFI"), x86, &mut jobs);
        let boot_src = src.esp_src.join("boot");
        if boot_src.is_dir() {
            esp_jobs(&boot_src, Path::new("boot"), x86, &mut jobs);
        }
        jobs.push((Some(vmlinuz), PathBuf::from("vmlinuz-lts")));
        jobs.push((Some(initrd), PathBuf::from("initramfs-lts")));
        let rescue = src.kern_dir.join("rescue.cpio.gz");
        if rescue.is_file() {
            jobs.push((Some(rescue), PathBuf::from("bia-rescue/rescue.cpio.gz")));
        } else {
            progress("warning: no rescue.cpio.gz; sticks from this image cannot run carry_update");
        }
        let total = jobs
            .iter()
            .filter_map(|(s, _)| s.as_ref())
            .map(|p| fs::metadata(p).map_or(0, |m| m.len()))
            .sum();
        let mut meter = Meter::new("ESP files".into(), total, progress);
        for (from, dest) in &jobs {
            match from {
                Some(from) => write_fat_file(&root, dest, from, &mut meter)?,
                None => {
                    ensure_fat_dir(&root, dest)?;
                }
            }
        }
        prune_wrong_arch(&root, x86)?;
    }
    file.seek(SeekFrom::Start(layout.boot_start))?;
    file.write_all(esp.get_ref())?;

    progress(&format!("BIAROOT ({} MiB)", root_bytes >> 20));
    file.seek(SeekFrom::Start(layout.root_start))?;
    let mut meter = Meter::new("BIAROOT".into(), root_bytes, progress);
    let mut r = File::open(&src.root_img)?;
    let mut buf = vec![0u8; 8 << 20];
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        meter.add(n as u64);
    }
    drop(meter);
    let mut file = file.into_inner().map_err(|e| e.into_error())?;

    if x86 {
        let i386 = src.esp_src.join("boot/grub/i386-pc");
        match (fs::read(i386.join("boot.img")), fs::read(i386.join("core.img"))) {
            (Ok(boot), Ok(core)) => {
                progress("embedding BIOS GRUB (legacy boot)");
                bia_disk::embed_bios_grub(&mut file, &layout, &boot, &core).context("embed BIOS GRUB")?;
            }
            _ => progress("no GRUB i386-pc images; this image boots on UEFI only"),
        }
    }
    file.sync_all()?;
    progress(&format!("image {} ({} MiB)", out.display(), size >> 20));
    Ok(size)
}

/// The partitions of a bia image, read from its first MiB (protective MBR + GPT).
pub fn parse_layout(head: &[u8]) -> Result<bia_disk::Layout> {
    if head.len() < 34 * 512 || &head[512..520] != b"EFI PART" {
        bail!("not a bia disk image (no GPT)");
    }
    let h = &head[512..1024];
    let u32_at = |o: usize| u32::from_le_bytes(h[o..o + 4].try_into().unwrap());
    let u64_at = |b: &[u8], o: usize| u64::from_le_bytes(b[o..o + 8].try_into().unwrap());
    let entries_lba = u64_at(h, 72);
    let count = u32_at(80) as usize;
    let esz = u32_at(84) as usize;
    if esz < 128 || entries_lba < 2 {
        bail!("bad GPT header");
    }
    let mut parts = Vec::new();
    for i in 0..count.min(128) {
        let o = (entries_lba as usize) * 512 + i * esz;
        let Some(e) = head.get(o..o + esz) else { break };
        if e[..16].iter().all(|&b| b == 0) {
            continue;
        }
        let (first, last) = (u64_at(e, 32), u64_at(e, 40));
        if last < first {
            bail!("bad GPT entry {}", i + 1);
        }
        let ty: [u8; 16] = e[..16].try_into().unwrap();
        parts.push((ty, first * 512, (last - first + 1) * 512));
    }
    let [boot, bios, root] = parts[..] else {
        bail!("not a bia disk image ({} partitions, want ESP + BIOS boot + BIAROOT)", parts.len());
    };
    if boot.0 != EFI_TYPE || bios.0 != BIOS_TYPE {
        bail!("not a bia disk image (partition types)");
    }
    if boot.1 != HEAD || boot.1 + boot.2 > bios.1 || bios.1 + bios.2 > root.1 {
        bail!("not a bia disk image (partition order)");
    }
    Ok(bia_disk::Layout {
        boot_start: boot.1,
        boot_len: boot.2,
        bios_start: bios.1,
        bios_len: bios.2,
        root_start: root.1,
        root_len: root.2,
    })
}

/// A bia image read front to back, raw (`.img`) or xz-compressed (`.img.xz`, multi-threaded
/// decoding of `xz -T0` output).
pub struct ImageReader {
    inner: Box<dyn Read + Send>,
    pos: u64,
}

impl ImageReader {
    pub fn open(path: &Path) -> Result<Self> {
        let mut f = File::open(path).with_context(|| format!("open {}", path.display()))?;
        let mut magic = [0u8; 6];
        let n = f.read(&mut magic)?;
        f.seek(SeekFrom::Start(0))?;
        let inner: Box<dyn Read + Send> = if n == 6 && magic == XZ_MAGIC {
            let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(8) as u32;
            let stream = liblzma::stream::MtStreamBuilder::new()
                .threads(threads)
                .memlimit_threading(1 << 30)
                .memlimit_stop(u64::MAX)
                .decoder()
                .context("xz decoder")?;
            Box::new(liblzma::read::XzDecoder::new_stream(BufReader::with_capacity(1 << 20, f), stream))
        } else {
            Box::new(BufReader::with_capacity(8 << 20, f))
        };
        Ok(Self { inner, pos: 0 })
    }

    /// Bytes consumed so far.
    pub fn pos(&self) -> u64 {
        self.pos
    }

    /// Fill `buf` completely (an image ending early is an error).
    pub fn read_exact_at_pos(&mut self, buf: &mut [u8]) -> Result<()> {
        self.inner
            .read_exact(buf)
            .with_context(|| format!("image ends early or is damaged (at byte {})", self.pos))?;
        self.pos += buf.len() as u64;
        Ok(())
    }

    /// Skip forward to `to`, passing the skipped bytes to `seen`.
    pub fn skip_to(&mut self, to: u64, seen: &mut dyn FnMut(&[u8])) -> Result<()> {
        if to < self.pos {
            bail!("image read out of order ({to} < {})", self.pos);
        }
        let mut buf = vec![0u8; 1 << 20];
        while self.pos < to {
            let n = ((to - self.pos) as usize).min(buf.len());
            self.read_exact_at_pos(&mut buf[..n])?;
            seen(&buf[..n]);
        }
        Ok(())
    }

    /// Read whatever is left; returns how many bytes that was.
    pub fn drain(&mut self, seen: &mut dyn FnMut(&[u8])) -> Result<u64> {
        let mut buf = vec![0u8; 1 << 20];
        let mut n_all = 0;
        loop {
            let n = self.inner.read(&mut buf).with_context(|| format!("image is damaged (at byte {})", self.pos))?;
            if n == 0 {
                return Ok(n_all);
            }
            seen(&buf[..n]);
            self.pos += n as u64;
            n_all += n as u64;
        }
    }
}

/// Raw size of a local image: the file size of an `.img`, the xz index's figure for `.img.xz`.
pub fn raw_size(path: &Path) -> Result<u64> {
    let mut f = File::open(path)?;
    let len = f.metadata()?.len();
    let mut magic = [0u8; 6];
    if f.read(&mut magic)? < 6 || magic != XZ_MAGIC {
        return Ok(len);
    }
    // Stream footer (12 bytes): CRC32, backward size, flags, "YZ". The index before it lists
    // every block's uncompressed size (multibyte integers).
    let mut foot = [0u8; 12];
    f.seek(SeekFrom::End(-12))?;
    f.read_exact(&mut foot)?;
    if &foot[10..12] != b"YZ" {
        bail!("{}: no xz stream footer (padding or several streams?)", path.display());
    }
    let index_len = (u64::from(u32::from_le_bytes(foot[4..8].try_into().unwrap())) + 1) * 4;
    let mut index = vec![0u8; index_len as usize];
    f.seek(SeekFrom::End(-12 - index_len as i64))?;
    f.read_exact(&mut index)?;
    let mut p = 1; // index indicator 0x00
    let vli = |p: &mut usize| -> Result<u64> {
        let mut v = 0u64;
        for i in 0..9 {
            let b = *index.get(*p).context("xz index")?;
            *p += 1;
            v |= u64::from(b & 0x7F) << (7 * i);
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        bail!("xz index")
    };
    let records = vli(&mut p)?;
    let mut total = 0;
    for _ in 0..records {
        let _unpadded = vli(&mut p)?;
        total += vli(&mut p)?;
    }
    Ok(total)
}

const fn guid_le(s: &str) -> [u8; 16] {
    let b = s.as_bytes();
    const fn hex(c: u8) -> u8 {
        match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            _ => c - b'A' + 10,
        }
    }
    const fn byte(b: &[u8], i: usize) -> u8 {
        hex(b[i]) << 4 | hex(b[i + 1])
    }
    // text positions of the 16 bytes, in on-disk (mixed-endian) order
    let pos = [6, 4, 2, 0, 11, 9, 16, 14, 19, 21, 24, 26, 28, 30, 32, 34];
    let mut out = [0u8; 16];
    let mut i = 0;
    while i < 16 {
        out[i] = byte(b, pos[i]);
        i += 1;
    }
    out
}

/// Files (`Some(src)`) and directories (`None`) of `src_dir`, mapped under `dest_rel` on the ESP.
fn esp_jobs(src_dir: &Path, dest_rel: &Path, x86: bool, jobs: &mut Vec<(Option<PathBuf>, PathBuf)>) {
    for entry in walkdir::WalkDir::new(src_dir).sort_by_file_name().into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        let rel = path.strip_prefix(src_dir).unwrap_or(path);
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        // Per-arch kernel copies from the build would fill the ESP; it boots /vmlinuz-lts.
        if name.starts_with("vmlinuz-lts-") || name.starts_with("initramfs-lts-") {
            continue;
        }
        // BIOS GRUB is x86 only.
        if !x86 && rel.components().any(|c| c.as_os_str() == "i386-pc") {
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

fn write_fat_file<IO: fatfs::ReadWriteSeek>(root: &Dir<'_, IO>, rel: &Path, src: &Path, meter: &mut Meter) -> Result<()> {
    let name = rel.file_name().and_then(|s| s.to_str()).context("file name")?;
    let dir = match rel.parent().filter(|p| !p.as_os_str().is_empty()) {
        Some(parent) => ensure_fat_dir(root, parent)?,
        None => root.clone(),
    };
    let data = fs::read(src).with_context(|| format!("read {}", src.display()))?;
    let mut f = dir.create_file(name).with_context(|| format!("create FAT file {name}"))?;
    for chunk in data.chunks(4 << 20) {
        f.write_all(chunk).with_context(|| format!("write FAT file {name}"))?;
        meter.add(chunk.len() as u64);
    }
    Ok(())
}

/// Drop the other architecture's shim, GRUB and MokManager from the ESP (the shared
/// `source/flash/EFI` has both; firmware would otherwise try the wrong one).
fn prune_wrong_arch<IO: fatfs::ReadWriteSeek>(root: &Dir<'_, IO>, x86: bool) -> Result<()> {
    let remove: &[&str] = if x86 {
        &["EFI/BOOT/BOOTAA64.EFI", "EFI/BOOT/grubaa64.efi", "EFI/BOOT/mmaa64.efi", "EFI/debian/BOOTAA64.EFI", "EFI/debian/grubaa64.efi"]
    } else {
        &["EFI/BOOT/BOOTX64.EFI", "EFI/BOOT/grubx64.efi", "EFI/BOOT/mmx64.efi", "EFI/debian/BOOTX64.EFI", "EFI/debian/grubx64.efi"]
    };
    for rel in remove {
        let path = Path::new(rel);
        let (Some(parent), Some(name)) = (path.parent(), path.file_name().and_then(|s| s.to_str())) else {
            continue;
        };
        let mut dir = root.clone();
        let mut found = true;
        for c in parent.components() {
            match dir.open_dir(&c.as_os_str().to_string_lossy()) {
                Ok(d) => dir = d,
                Err(_) => {
                    found = false;
                    break;
                }
            }
        }
        if found {
            let _ = dir.remove(name);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guid_bytes_match_the_gpt_crate() {
        assert_eq!(&EFI_TYPE[..4], &[0x28, 0x73, 0x2A, 0xC1]);
        assert_eq!(&BIOS_TYPE[..], b"Hah!IdontNeedEFI");
    }
}
