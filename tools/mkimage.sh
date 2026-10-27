#!/bin/bash
# Build dist/bia.img: 256 MiB EFI + 1 MiB BIOS boot + one ext4 (OS, /home/bia, swap file).
# The image is the root filesystem plus the ESP (no model; bia-deploy adds one to sticks).
set -euo pipefail
cd "$(dirname "$0")/.."
FLASH=${FLASH:-source/flash}
IMG=${IMG:-dist/bia.img}
IMAGE_MB=${IMAGE_MB:-16384}
ROOT_MB=${ROOT_MB:-7100}
ARCH=${ARCH:-arm64}
ROOTIMG=${ROOTIMG:-dist/$ARCH/root.img}
mkdir -p "$(dirname "$IMG")"
case $ARCH in
  x86_64) EFI_NAME=BOOTX64.EFI ;;
  arm64)  EFI_NAME=BOOTAA64.EFI ;;
  *) echo "unknown ARCH=$ARCH"; exit 1 ;;
esac
[ -f "$FLASH/EFI/BOOT/$EFI_NAME" ] || { echo "no shim $FLASH/EFI/BOOT/$EFI_NAME — make build first"; exit 1; }
[ -f "$ROOTIMG" ] || { echo "no $ROOTIMG — make build first"; exit 1; }
KERN_DIR=$(cd "$(dirname "$ROOTIMG")" && pwd)
tools/esp-kernel.sh "$ROOTIMG" "$KERN_DIR"

cargo build --release -p bia-mkimage -q
root_bytes=$(stat -f%z "$ROOTIMG")
# 16 GiB qemu image. The ext4 is grown to the partition after the smaller root.img is copied.
size=$(( IMAGE_MB * 1024 * 1024 ))
min=$(( root_bytes + 320 * 1024 * 1024 ))
if [ "$size" -lt "$min" ]; then
  echo "root image needs $((min / 1024 / 1024)) MiB; raising the image to that"
  size=$min
fi
echo "== image $IMG: $(( size / 1024 / 1024 )) MiB (ESP + BIOS + one ext4, root image $(( root_bytes / 1024 / 1024 )) MiB)"
rm -f "$IMG"
layout=$(./target/release/bia-mkimage "$IMG" "$size")
echo "   layout (bytes): $layout"
read -r boot_start boot_len bios_start bios_len root_start root_len <<<"$layout"

dev=$(hdiutil attach -nomount -imagekey diskimage-class=CRawDiskImage "$IMG" | head -1 | awk '{print $1}')
trap 'hdiutil detach "$dev" -quiet || true' EXIT
echo "== attached as $dev"
sleep 1

# p1 ESP, p2 BIOS (raw), p3 ext4
s1="${dev}s1"

echo "== formatting ESP"
newfs_msdos -F 32 -v BIABOOT "$s1" > /dev/null
diskutil mount "$s1" > /dev/null
boot=$(diskutil info "$s1" | awk -F': +' '/Mount Point/ {print $2}')
echo "== copying ESP -> $boot"
mkdir -p "$boot/EFI" "$boot/boot"
# flash/EFI also holds per-arch kernel copies from the build (vmlinuz-lts-<arch>,
# initramfs-lts-<arch>); the ESP boots /vmlinuz-lts, and they would fill it.
rsync -rt --exclude='/vmlinuz-lts*' --exclude='/initramfs-lts*' "$FLASH/EFI/" "$boot/EFI/"
if [ -d "$FLASH/boot" ]; then
  cp -R "$FLASH/boot/." "$boot/boot/"
fi
cp -f "$KERN_DIR/vmlinuz-lts" "$KERN_DIR/initramfs-lts" "$boot/"
# Rescue image overlay (GRUB hook in grub.cfg boots it while bia-rescue/next.cfg exists).
if [ -f "$KERN_DIR/rescue.cpio.gz" ]; then
  mkdir -p "$boot/bia-rescue"
  cp -f "$KERN_DIR/rescue.cpio.gz" "$boot/bia-rescue/rescue.cpio.gz"
else
  echo "warning: no $KERN_DIR/rescue.cpio.gz; this image cannot run carry_update"
fi
# Shared flash/EFI has both ISA stubs; QEMU/EDK2 otherwise tries the wrong one.
case $ARCH in
  arm64)  rm -f "$boot/EFI/BOOT/BOOTX64.EFI" "$boot/EFI/BOOT/grubx64.efi" "$boot/EFI/BOOT/mmx64.efi"
          rm -f "$boot/EFI/debian/BOOTX64.EFI" "$boot/EFI/debian/grubx64.efi" ;;
  x86_64) rm -f "$boot/EFI/BOOT/BOOTAA64.EFI" "$boot/EFI/BOOT/grubaa64.efi" "$boot/EFI/BOOT/mmaa64.efi"
          rm -f "$boot/EFI/debian/BOOTAA64.EFI" "$boot/EFI/debian/grubaa64.efi" ;;
esac
sync
diskutil unmount "$s1" > /dev/null
hdiutil detach "$dev" -quiet
trap - EXIT

if [ "$ARCH" = x86_64 ] && [ -f "$FLASH/boot/grub/i386-pc/core.img" ]; then
  echo "== embedding BIOS GRUB (legacy boot)"
  python3 tools/grub-bios-embed.py --raw-pc "$IMG" "$FLASH/boot/grub/i386-pc"
fi

echo "== writing Debian root at offset $root_start"
root_bytes=$(stat -f%z "$ROOTIMG")
if [ "$root_bytes" -gt "$root_len" ]; then
  echo "root.img ($root_bytes) larger than partition ($root_len)"; exit 1
fi
dd if="$ROOTIMG" of="$IMG" bs=1048576 conv=notrunc seek=$((root_start / 1048576)) status=progress
ARCH="$ARCH" tools/grow-ext4.sh "$IMG" "$root_start" "$root_len"
ls -lh "$IMG"
echo "== $IMG ready"
