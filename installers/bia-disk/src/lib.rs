//! GPT layout for bia images and sticks (`bia-mkimage`, `bia-deploy`).
//!
//! Layout (GPT, 512-byte sectors):
//!   p1  BIABOOT  EFI System Partition, FAT32, 256 MiB  -> shim + GRUB + vmlinuz + initramfs
//!   p2  BIABIOS  BIOS boot, 1 MiB (no FS)              -> GRUB core.img (legacy BIOS)
//!   p3  BIAROOT  Linux filesystem, ext4, the rest      -> Debian + /home/bia + swap file

use anyhow::{Context, Result};
use gpt::{disk::LogicalBlockSize, partition_types, GptConfig};
use std::io::{self, Read, Seek, SeekFrom, Write};

pub const BOOT_LABEL: &str = "BIABOOT";
pub const BIOS_LABEL: &str = "BIABIOS";
pub const ROOT_LABEL: &str = "BIAROOT";
pub const BOOT_SIZE: u64 = 256 * 1024 * 1024;
pub const BIOS_SIZE: u64 = 1024 * 1024;
pub const ALIGN: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct Layout {
    pub boot_start: u64,
    pub boot_len: u64,
    pub bios_start: u64,
    pub bios_len: u64,
    pub root_start: u64,
    pub root_len: u64,
}

/// macOS `/dev/rdisk*` (and some other char devices) report 0 from `seek(End)`.
/// The gpt crate sizes the backup header from that, so we pin the known capacity.
#[derive(Debug)]
struct KnownSize<D> {
    inner: D,
    size: u64,
}

impl<D: Read> Read for KnownSize<D> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}
impl<D: Write> Write for KnownSize<D> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
impl<D: Seek> Seek for KnownSize<D> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let pos = match pos {
            SeekFrom::End(off) => {
                let abs = i128::from(self.size) + i128::from(off);
                if abs < 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "seek before start",
                    ));
                }
                SeekFrom::Start(abs as u64)
            }
            other => other,
        };
        self.inner.seek(pos)
    }
}

/// Length of BIAROOT that `write_gpt` lays out on a `size`-byte disk (0 if it does not fit).
pub fn root_len(size: u64) -> u64 {
    let root = size.saturating_sub(BOOT_SIZE + BIOS_SIZE + 6 * ALIGN);
    root - root % ALIGN
}

/// Write a fresh GPT onto `dev` (file already `size` bytes, or a block device):
/// ESP, BIOS boot, and one ext4 root partition filling the rest.
pub fn write_gpt<D>(dev: D, size: u64) -> Result<Layout>
where
    D: Read + Write + Seek + std::fmt::Debug,
{
    let mut dev = KnownSize { inner: dev, size };
    let mbr =
        gpt::mbr::ProtectiveMBR::with_lb_size(u32::try_from((size / 512) - 1).unwrap_or(u32::MAX));
    mbr.overwrite_lba0(&mut dev)
        .context("write protective MBR")?;

    let mut disk = GptConfig::new()
        .writable(true)
        .logical_block_size(LogicalBlockSize::Lb512)
        .create_from_device(dev, None)
        .context("create GPT")?;

    let root = root_len(size);
    if root == 0 {
        anyhow::bail!("disk too small for ESP + BIOS + system");
    }

    disk.add_partition(
        BOOT_LABEL,
        BOOT_SIZE,
        partition_types::EFI,
        0,
        Some(ALIGN / 512),
    )
    .context("add boot partition")?;
    disk.add_partition(
        BIOS_LABEL,
        BIOS_SIZE,
        partition_types::BIOS,
        0,
        Some(ALIGN / 512),
    )
    .context("add BIOS boot partition")?;
    disk.add_partition(
        ROOT_LABEL,
        root,
        partition_types::LINUX_FS,
        0,
        Some(ALIGN / 512),
    )
    .context("add root partition")?;

    let parts = disk.partitions().clone();
    let p_boot = parts.get(&1).context("p1 boot")?;
    let p_bios = parts.get(&2).context("p2 bios")?;
    let p_root = parts.get(&3).context("p3 root")?;
    let layout = Layout {
        boot_start: p_boot.first_lba * 512,
        boot_len: (p_boot.last_lba - p_boot.first_lba + 1) * 512,
        bios_start: p_bios.first_lba * 512,
        bios_len: (p_bios.last_lba - p_bios.first_lba + 1) * 512,
        root_start: p_root.first_lba * 512,
        root_len: (p_root.last_lba - p_root.first_lba + 1) * 512,
    };
    disk.write().context("write GPT")?;
    Ok(layout)
}

/// GRUB i386-pc boot.img: LBA of core.img (u64).
const KERNEL_SECTOR_OFF: usize = 0x5C;
/// boot.img: 2-byte jmp that distrusts the BIOS drive number; NOPed for disks.
const DRIVE_CHECK_OFF: usize = 0x66;
/// core.img sector 0 (diskboot): blocklist {u64 start, u16 len, u16 segment} at its end.
const BLOCKLIST_OFF: usize = 512 - 12;

/// Make the disk boot on legacy BIOS, as `grub-bios-setup` does for an embedded core:
/// boot.img into the MBR (pointing at core.img, drive check off, protective MBR
/// partition table kept) and core.img into the BIOS boot partition with its blocklist
/// pointing at the sectors after its first. `dev` must allow whole-sector writes.
pub fn embed_bios_grub<D: Read + Write + Seek>(dev: &mut D, layout: &Layout, boot_img: &[u8], core_img: &[u8]) -> Result<()> {
    if boot_img.len() < 512 {
        anyhow::bail!("boot.img is {} bytes", boot_img.len());
    }
    let core_sectors = core_img.len().div_ceil(512);
    if core_sectors < 2 || (core_sectors as u64) * 512 > layout.bios_len {
        anyhow::bail!("core.img ({} bytes) does not fit the {} byte BIOS boot partition", core_img.len(), layout.bios_len);
    }
    let core_lba = layout.bios_start / 512;
    let mut boot = boot_img[..512].to_vec();
    boot[KERNEL_SECTOR_OFF..KERNEL_SECTOR_OFF + 8].copy_from_slice(&core_lba.to_le_bytes());
    if boot[DRIVE_CHECK_OFF] == 0xEB {
        boot[DRIVE_CHECK_OFF] = 0x90;
        boot[DRIVE_CHECK_OFF + 1] = 0x90;
    }
    let mut core = core_img.to_vec();
    core.resize(core_sectors * 512, 0);
    let listed = u16::from_le_bytes([core[BLOCKLIST_OFF + 8], core[BLOCKLIST_OFF + 9]]) as usize;
    if listed != core_sectors - 1 {
        anyhow::bail!("core.img blocklist lists {listed} sectors; the file has {} after the first", core_sectors - 1);
    }
    core[BLOCKLIST_OFF..BLOCKLIST_OFF + 8].copy_from_slice(&(core_lba + 1).to_le_bytes());

    let mut mbr = vec![0u8; 512];
    dev.seek(SeekFrom::Start(0))?;
    dev.read_exact(&mut mbr).context("read MBR")?;
    mbr[..440].copy_from_slice(&boot[..440]);
    mbr[510] = 0x55;
    mbr[511] = 0xAA;
    dev.seek(SeekFrom::Start(0))?;
    dev.write_all(&mbr).context("write MBR boot code")?;
    let mut part = core;
    part.resize(layout.bios_len as usize, 0);
    dev.seek(SeekFrom::Start(layout.bios_start))?;
    dev.write_all(&part).context("write core.img")?;
    dev.flush()?;
    Ok(())
}

#[cfg(test)]
mod bios_tests {
    use super::*;

    #[test]
    fn embed_patches_like_grub_bios_setup() {
        let mut disk = io::Cursor::new(vec![0u8; 300 * 1024 * 1024]);
        let layout = write_gpt(&mut disk, 300 * 1024 * 1024).unwrap();
        let pmbr_table = disk.get_ref()[446..510].to_vec();
        let mut boot = vec![0u8; 512];
        boot[DRIVE_CHECK_OFF] = 0xEB;
        boot[DRIVE_CHECK_OFF + 1] = 0x05;
        boot[0] = 0xEB; // jmp, like a real boot.img
        let mut core = vec![0x11u8; 3 * 512 + 100];
        core[BLOCKLIST_OFF..BLOCKLIST_OFF + 8].copy_from_slice(&2u64.to_le_bytes()); // grub-mkimage default
        core[BLOCKLIST_OFF + 8..BLOCKLIST_OFF + 10].copy_from_slice(&3u16.to_le_bytes());
        embed_bios_grub(&mut disk, &layout, &boot, &core).unwrap();
        let d = disk.get_ref();
        let lba = layout.bios_start / 512;
        assert_eq!(u64::from_le_bytes(d[KERNEL_SECTOR_OFF..KERNEL_SECTOR_OFF + 8].try_into().unwrap()), lba);
        assert_eq!(&d[DRIVE_CHECK_OFF..DRIVE_CHECK_OFF + 2], &[0x90, 0x90]);
        assert_eq!(&d[446..510], &pmbr_table[..], "protective MBR partition entry kept");
        assert_eq!(&d[510..512], &[0x55, 0xAA]);
        let s = layout.bios_start as usize;
        assert_eq!(u64::from_le_bytes(d[s + BLOCKLIST_OFF..s + BLOCKLIST_OFF + 8].try_into().unwrap()), lba + 1);
        assert_eq!(d[s + 512], 0x11, "rest of core.img follows its first sector");
        // the GPT is untouched
        assert_eq!(&d[512..520], b"EFI PART");
        // a blocklist that does not match the file is refused
        let mut bad = core.clone();
        bad[BLOCKLIST_OFF + 8] = 9;
        assert!(embed_bios_grub(&mut disk, &layout, &boot, &bad).is_err());
    }
}
