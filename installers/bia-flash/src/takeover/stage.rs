//! What the old system's boot loader starts once: bia's own kernel, initramfs and rescue
//! overlay, with the image and the seeds as extra initrds. Everything is staged in one
//! directory on a filesystem the boot loader can read; the image is loaded with GRUB's
//! `newc:` initrd syntax, so it is not copied a second time.
use crate::image::{parse_layout, ImageReader, HEAD};
use anyhow::{bail, Context, Result};
use fatfs::{FileSystem, FsOptions};
use std::io::{self, Read, Write};
use std::path::Path;

/// Boot files from the image's ESP.
pub struct BootFiles {
    pub kernel: Vec<u8>,
    pub initramfs: Vec<u8>,
    pub rescue: Vec<u8>,
    /// shim, GRUB and MokManager under the names shim expects next to it.
    pub efi: Vec<(String, Vec<u8>)>,
}

/// Read the boot files out of a bia image (`.img` or `.img.xz`).
pub fn boot_files(image: &Path, arch: &str) -> Result<BootFiles> {
    let mut r = ImageReader::open(image)?;
    let mut head = vec![0u8; HEAD as usize];
    r.read_exact_at_pos(&mut head)?;
    let l = parse_layout(&head)?;
    r.skip_to(l.boot_start, &mut |_| {})?;
    let mut esp = vec![0u8; usize::try_from(l.boot_len)?];
    r.read_exact_at_pos(&mut esp)?;
    let fs = FileSystem::new(io::Cursor::new(esp), FsOptions::new()).context("image ESP")?;
    let read = |p: &str| -> Result<Vec<u8>> {
        let mut f = fs.root_dir().open_file(p).with_context(|| format!("the image has no {p}"))?;
        let mut v = vec![];
        f.read_to_end(&mut v)?;
        Ok(v)
    };
    let (shim, grub, mm) = match arch {
        "arm64" => ("EFI/BOOT/BOOTAA64.EFI", "EFI/BOOT/grubaa64.efi", "EFI/BOOT/mmaa64.efi"),
        _ => ("EFI/BOOT/BOOTX64.EFI", "EFI/BOOT/grubx64.efi", "EFI/BOOT/mmx64.efi"),
    };
    let rescue = read("bia-rescue/rescue.cpio.gz").context("this bia image has no rescue image, so it cannot take over")?;
    let mut efi = vec![
        (if arch == "arm64" { "shimaa64.efi" } else { "shimx64.efi" }.to_string(), read(shim)?),
        (grub.rsplit('/').next().unwrap().to_string(), read(grub)?),
    ];
    if let Ok(m) = read(mm) {
        efi.push((mm.rsplit('/').next().unwrap().to_string(), m));
    }
    Ok(BootFiles { kernel: read("vmlinuz-lts")?, initramfs: read("initramfs-lts")?, rescue, efi })
}

/// A newc (SVR4, no CRC) cpio archive, as the kernel unpacks initramfs.
pub struct Cpio {
    buf: Vec<u8>,
    ino: u32,
}

impl Cpio {
    pub fn new() -> Self {
        Cpio { buf: vec![], ino: 1 }
    }

    fn entry(&mut self, name: &str, mode: u32, data: &[u8]) {
        let namez = format!("{name}\0");
        let hdr = format!(
            "070701{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}",
            self.ino, mode, 0, 0, 1, 0, data.len(), 0, 0, 0, 0, namez.len(), 0
        );
        self.ino += 1;
        self.buf.extend_from_slice(hdr.as_bytes());
        self.buf.extend_from_slice(namez.as_bytes());
        while self.buf.len() % 4 != 0 {
            self.buf.push(0);
        }
        self.buf.extend_from_slice(data);
        while self.buf.len() % 4 != 0 {
            self.buf.push(0);
        }
    }

    pub fn dir(&mut self, name: &str) {
        self.entry(name, 0o040755, &[]);
    }

    pub fn file(&mut self, name: &str, data: &[u8], mode: u32) {
        self.entry(name, 0o100000 | mode, data);
    }

    pub fn finish(mut self) -> Vec<u8> {
        self.entry("TRAILER!!!", 0, &[]);
        while self.buf.len() % 512 != 0 {
            self.buf.push(0);
        }
        self.buf
    }
}

impl Default for Cpio {
    fn default() -> Self {
        Self::new()
    }
}

/// Put every file under `root` into the archive below `prefix` (directories first).
pub fn add_tree(c: &mut Cpio, root: &Path, prefix: &str) -> Result<()> {
    c.dir(prefix);
    let mut entries: Vec<_> = std::fs::read_dir(root)?.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = format!("{prefix}/{}", e.file_name().to_string_lossy());
        let ft = e.file_type()?;
        if ft.is_dir() {
            add_tree(c, &e.path(), &name)?;
        } else if ft.is_file() {
            c.file(&name, &std::fs::read(e.path())?, 0o600);
        }
    }
    Ok(())
}

/// Where the rescue reads the image when the boot loader cannot load it (legacy BIOS:
/// GRUB keeps initrds in low memory): the filesystem's UUID and the file's path in it.
pub struct ImageOnDisk<'a> {
    pub fs_uuid: &'a str,
    pub path: &'a str,
}

/// `/bia-takeover/info`, sourced by the rescue (only hex, digits and a plain path in it).
pub fn info(disk_ptuuid: &str, disk_size: u64, raw_size: u64, raw_sha256: &str, xz_sha256: &str, arch: &str, img: Option<ImageOnDisk>) -> Result<String> {
    let hex = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-');
    if !hex(disk_ptuuid) || !hex(raw_sha256) || !hex(xz_sha256) || !matches!(arch, "x86_64" | "arm64") {
        bail!("takeover info: unexpected value");
    }
    let mut s = format!(
        "DISK_PTUUID={}\nDISK_SIZE={disk_size}\nRAW_SIZE={raw_size}\nRAW_SHA256={}\nXZ_SHA256={}\nARCH={arch}\n",
        disk_ptuuid.to_ascii_lowercase(),
        raw_sha256.to_ascii_lowercase(),
        xz_sha256.to_ascii_lowercase()
    );
    if let Some(i) = img {
        let plain = |c: char| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '.' | '-' | '@');
        if !hex(i.fs_uuid) || !i.path.starts_with('/') || !i.path.chars().all(plain) || i.path.contains("..") {
            bail!("takeover info: the staging path {:?} has characters the rescue does not take; pass --staging", i.path);
        }
        s.push_str(&format!("IMG_FS_UUID={}\nIMG_PATH={}\n", i.fs_uuid.to_ascii_lowercase(), i.path));
    }
    Ok(s)
}

/// The one-shot GRUB menu entry. `rel` is the staging directory as GRUB sees it on its
/// filesystem (`/bia-takeover`, `/@/bia-takeover` on a btrfs subvolume), `id` the name of
/// the marker file that finds that filesystem.
/// `image_initrd`: the image goes in as an initrd (`newc:`); false for legacy BIOS, where
/// the rescue reads it from the disk instead.
pub fn grub_entry(rel: &str, id: &str, image_initrd: bool) -> String {
    // Without its initrds the kernel would only panic; then start the old system again
    // (the one-shot entry is used up) instead of hanging. panic=30 covers a later failure.
    let image = if image_initrd { format!(" newc:bia-takeover/bia.img.xz:($bia_tk){rel}/bia.img.xz") } else { String::new() };
    format!(
        r#"if serial --unit=0 --speed=115200; then terminal_output --append serial; fi
search --no-floppy --set=bia_tk --file {rel}/{id}
menuentry "bia takeover" --id bia-takeover {{
  echo "bia takeover: loading bia and its image into memory..."
  linux ($bia_tk){rel}/vmlinuz-lts rdinit=/bia-rescue/init bia.takeover=1 panic=30 console=tty0 console=ttyS0,115200
  if initrd ($bia_tk){rel}/initramfs-lts ($bia_tk){rel}/rescue.cpio.gz ($bia_tk){rel}/seed.cpio{image}; then
    boot
  fi
  echo "bia takeover: the boot loader could not load bia's image into memory; starting the old system again"
  sleep 15
  reboot
}}
"#
    )
}

/// grub.cfg next to shim on the old ESP: boot the takeover entry right away.
pub fn grub_cfg_uefi(rel: &str, id: &str) -> String {
    format!("set timeout=1\nset default=bia-takeover\n{}", grub_entry(rel, id, true))
}

/// Prepended to a Debian GRUB's own `EFI/debian/grub.cfg` on the old ESP: Debian's signed
/// GRUB reads that file before the one next to it, so without this it would start the old
/// system instead.
pub fn grub_hook(id_on_esp: &str) -> String {
    format!(
        "# bia takeover (bia-flash): remove this block to boot the old system again\nif search --no-floppy --set=bia_hook --file /EFI/bia-takeover/{id_on_esp}; then\n  configfile ($bia_hook)/EFI/bia-takeover/grub.cfg\nfi\n"
    )
}

/// Write `data` to `path` atomically (owner-only).
pub fn write_file(path: &Path, data: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp-bia");
    {
        let mut f = std::fs::File::create(&tmp).with_context(|| format!("create {}", tmp.display()))?;
        f.write_all(data)?;
        f.sync_all()?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, path).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse our own newc output back: names, sizes, data, alignment.
    #[test]
    fn cpio_round_trip() {
        let mut c = Cpio::new();
        c.dir("bia-takeover");
        c.file("bia-takeover/info", b"DISK_SIZE=1\n", 0o600);
        c.file("bia-takeover/esp/x", b"abc", 0o600);
        let b = c.finish();
        assert_eq!(b.len() % 512, 0);
        let mut i = 0;
        let mut seen = vec![];
        loop {
            assert_eq!(&b[i..i + 6], b"070701");
            let field = |n: usize| usize::from_str_radix(std::str::from_utf8(&b[i + 6 + n * 8..i + 14 + n * 8]).unwrap(), 16).unwrap();
            let (size, nlen) = (field(6), field(11));
            let name = std::str::from_utf8(&b[i + 110..i + 110 + nlen - 1]).unwrap().to_string();
            let mut j = i + 110 + nlen;
            j = (j + 3) & !3;
            let data = b[j..j + size].to_vec();
            j = (j + size + 3) & !3;
            if name == "TRAILER!!!" {
                break;
            }
            seen.push((name, data));
            i = j;
        }
        assert_eq!(seen[1], ("bia-takeover/info".to_string(), b"DISK_SIZE=1\n".to_vec()));
        assert_eq!(seen[2].1, b"abc");
    }

    #[test]
    fn grub_and_info() {
        let e = grub_cfg_uefi("/@/bia-takeover", "takeover-ab12.id");
        assert!(e.contains("--file /@/bia-takeover/takeover-ab12.id"));
        assert!(e.contains("newc:bia-takeover/bia.img.xz:($bia_tk)/@/bia-takeover/bia.img.xz"));
        assert!(e.contains("rdinit=/bia-rescue/init bia.takeover=1 panic=30"));
        assert!(e.contains("then\n    boot\n  fi") && e.contains("reboot\n}"), "never boots without its initrds");
        assert!(e.starts_with("set timeout=1\nset default=bia-takeover\n"));
        let i = info("1a2b3c4d-0000-1111-2222-333344445555", 10, 5, "ab", "cd", "x86_64", None).unwrap();
        assert!(i.contains("DISK_PTUUID=1a2b3c4d-0000-1111-2222-333344445555\n") && !i.contains("IMG_"));
        assert!(info("$(rm -rf /)", 1, 1, "ab", "cd", "x86_64", None).is_err());
        let b = info("ab", 1, 1, "ab", "cd", "x86_64", Some(ImageOnDisk { fs_uuid: "0e4b-ff", path: "/bia-takeover/bia.img.xz" })).unwrap();
        assert!(b.contains("IMG_FS_UUID=0e4b-ff\nIMG_PATH=/bia-takeover/bia.img.xz\n"));
        assert!(info("ab", 1, 1, "ab", "cd", "x86_64", Some(ImageOnDisk { fs_uuid: "0e", path: "/a b;x" })).is_err());
        let bios = grub_entry("/bia-takeover", "t.id", false);
        assert!(!bios.contains("newc:") && bios.contains("seed.cpio; then"), "{bios}");
        assert!(grub_hook("t.id").contains("configfile ($bia_hook)/EFI/bia-takeover/grub.cfg"));
    }
}
