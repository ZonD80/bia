#!/usr/bin/env python3
"""Embed GRUB i386-pc into a GPT disk/image for legacy BIOS boot.

Expects:
  - GPT with a BIOS Boot Partition (type 21686148-6449-6E6F-744E-656564454649)
  - boot.img and core.img under the ESP (FAT), typically at boot/grub/i386-pc/

Writes (what grub-bios-setup does for an embedded core.img):
  - boot.img (440 bytes) into the protective MBR boot area, pointing at core.img's LBA,
    with the boot-drive check jump replaced by NOPs (hard disk boot)
  - core.img into the BIOS Boot Partition (contiguous), its first sector's blocklist
    pointing at the sectors right after it (grub-mkimage leaves the default 2 there,
    which only works for core.img at LBA 1)

Usage:
  grub-bios-embed.py <disk-or-image> <esp-mount-or-dir>
  grub-bios-embed.py --esp-offset <byte-off> --bios-offset <byte-off> \\
      <disk-or-image> <path-to-i386-pc-dir>
"""
from __future__ import annotations

import argparse
import os
import struct
import sys
import uuid

BIOS_TYPE = uuid.UUID("21686148-6449-6E6F-744E-656564454649")
EFI_TYPE = uuid.UUID("C12A7328-F81F-11D2-BA4B-00A0C93EC93B")
# GRUB i386-pc boot.img: 64-bit LBA of core.img at this offset.
KERNEL_SECTOR_OFF = 0x5C
# boot.img: 2-byte jmp that distrusts the BIOS drive number; setup NOPs it for disks.
DRIVE_CHECK_OFF = 0x66
# core.img sector 0 (diskboot): blocklist {u64 start, u16 len, u16 segment} at the end.
BLOCKLIST_OFF = 512 - 12


def read_gpt_partitions(dev: str):
    with open(dev, "rb") as f:
        f.seek(512)
        hdr = f.read(92)
        if hdr[0:8] != b"EFI PART":
            raise SystemExit(f"{dev}: no GPT header at LBA1")
        part_lba = struct.unpack_from("<Q", hdr, 72)[0]
        part_count = struct.unpack_from("<I", hdr, 80)[0]
        part_size = struct.unpack_from("<I", hdr, 84)[0]
        f.seek(part_lba * 512)
        entries = f.read(part_count * part_size)
    parts = []
    for i in range(part_count):
        off = i * part_size
        ent = entries[off : off + part_size]
        if len(ent) < 56:
            break
        type_guid = uuid.UUID(bytes_le=ent[0:16])
        if type_guid.int == 0:
            continue
        first = struct.unpack_from("<Q", ent, 32)[0]
        last = struct.unpack_from("<Q", ent, 40)[0]
        name = ent[56:128].decode("utf-16-le", errors="ignore").rstrip("\0")
        parts.append(
            {
                "index": i + 1,
                "type": type_guid,
                "first_lba": first,
                "last_lba": last,
                "name": name,
            }
        )
    return parts


def find_core_and_boot(esp_dir: str):
    candidates = [
        os.path.join(esp_dir, "boot/grub/i386-pc"),
        os.path.join(esp_dir, "grub/i386-pc"),
        esp_dir,
    ]
    for d in candidates:
        boot = os.path.join(d, "boot.img")
        core = os.path.join(d, "core.img")
        if os.path.isfile(boot) and os.path.isfile(core):
            return boot, core
    raise SystemExit(f"no boot.img+core.img under {esp_dir}")


def _sector_size(fd: int) -> int:
    """Logical block size for raw device I/O. macOS /dev/rdisk* rejects unaligned writes."""
    if sys.platform == "darwin":
        import array
        import fcntl

        # DKIOCGETBLOCKSIZE = _IOR('d', 24, u_int32_t)
        buf = array.array("I", [0])
        try:
            fcntl.ioctl(fd, 0x40046418, buf, True)
            if buf[0] >= 512:
                return int(buf[0])
        except OSError:
            pass
    return 512


def _align_up(n: int, a: int) -> int:
    return (n + a - 1) // a * a


def embed(dev: str, boot_img: str, core_img: str, bios_first_lba: int, bios_last_lba: int):
    with open(boot_img, "rb") as f:
        boot = bytearray(f.read())
    with open(core_img, "rb") as f:
        core = bytearray(f.read())
    if len(boot) < 512:
        raise SystemExit(f"{boot_img}: too short")
    bios_bytes = (bios_last_lba - bios_first_lba + 1) * 512
    if len(core) > bios_bytes:
        raise SystemExit(
            f"core.img is {len(core)} bytes; BIOS partition is only {bios_bytes}"
        )
    # Patch core LBA into boot sector; keep disk's existing 440..512 (protective MBR).
    struct.pack_into("<Q", boot, KERNEL_SECTOR_OFF, bios_first_lba)
    if boot[DRIVE_CHECK_OFF] == 0xEB:
        boot[DRIVE_CHECK_OFF : DRIVE_CHECK_OFF + 2] = b"\x90\x90"
    # diskboot loads the rest of core.img from the sectors after its own.
    _, nsect, _ = struct.unpack_from("<QHH", core, BLOCKLIST_OFF)
    rest = (len(core) + 511) // 512 - 1
    if nsect != rest:
        raise SystemExit(f"{core_img}: blocklist says {nsect} sectors, core.img has {rest} after the first")
    struct.pack_into("<Q", core, BLOCKLIST_OFF, bios_first_lba + 1)
    with open(dev, "rb+") as f:
        sector = _sector_size(f.fileno())
        # Raw character devices (macOS rdisk) require offset+length multiples of sector.
        mbr_n = _align_up(512, sector)
        mbr = bytearray(f.read(mbr_n))
        if len(mbr) < 512:
            raise SystemExit(f"{dev}: short read on MBR")
        new_mbr = bytearray(mbr)
        new_mbr[:440] = boot[:440]
        new_mbr[510:512] = b"\x55\xaa"
        f.seek(0)
        f.write(new_mbr)

        off = bios_first_lba * 512
        if off % sector != 0:
            raise SystemExit(
                f"{dev}: BIOS LBA {bios_first_lba} is not aligned to {sector}-byte sectors"
            )
        # Pad core to a whole number of device sectors (core.img is rarely sector-sized).
        core_write = bytes(core) + b"\0" * (_align_up(len(core), sector) - len(core))
        if len(core_write) > bios_bytes:
            raise SystemExit(
                f"core.img padded to {len(core_write)} exceeds BIOS partition {bios_bytes}"
            )
        f.seek(off)
        f.write(core_write)
        # Zero the rest of the BIOS partition so stale data cannot confuse GRUB.
        pad = bios_bytes - len(core_write)
        while pad > 0:
            chunk = min(pad, 1024 * 1024)
            chunk -= chunk % sector
            if chunk == 0:
                break
            f.write(b"\0" * chunk)
            pad -= chunk
        f.flush()
        os.fsync(f.fileno())
    print(
        f"== BIOS GRUB embedded on {dev}: core LBA {bios_first_lba} ({len(core)} bytes)"
    )


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("device", help="block device or image file")
    ap.add_argument("esp", help="mounted ESP directory (or i386-pc dir with --raw-pc)")
    ap.add_argument(
        "--raw-pc",
        action="store_true",
        help="esp argument is the i386-pc directory itself",
    )
    ap.add_argument("--bios-lba", type=int, help="override BIOS partition first LBA")
    args = ap.parse_args()

    if args.raw_pc:
        boot_img = os.path.join(args.esp, "boot.img")
        core_img = os.path.join(args.esp, "core.img")
        if not (os.path.isfile(boot_img) and os.path.isfile(core_img)):
            raise SystemExit(f"missing boot.img/core.img in {args.esp}")
    else:
        boot_img, core_img = find_core_and_boot(args.esp)

    parts = read_gpt_partitions(args.device)
    bios = next((p for p in parts if p["type"] == BIOS_TYPE), None)
    if bios is None and args.bios_lba is None:
        raise SystemExit(f"{args.device}: no BIOS Boot Partition in GPT")
    if args.bios_lba is not None:
        # Use one MiB if we cannot see the end.
        first = args.bios_lba
        last = first + (1024 * 1024 // 512) - 1
        if bios:
            last = bios["last_lba"]
            first = bios["first_lba"]
    else:
        first = bios["first_lba"]
        last = bios["last_lba"]

    embed(args.device, boot_img, core_img, first, last)


if __name__ == "__main__":
    main()
