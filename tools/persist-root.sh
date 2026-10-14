#!/bin/bash
# Replace the BIAROOT slice inside an existing bia.img.
# Leaves internal.img alone. The new root image already contains /home/bia from flash/.
set -euo pipefail
cd "$(dirname "$0")/.."
ARCH=${ARCH:-arm64}
IMG=${IMG:-dist/bia.img}
ROOTIMG=${ROOTIMG:-dist/$ARCH/root.img}

pgrep qemu-system >/dev/null && { echo "QEMU is running; stop it before make run persist"; exit 1; }
[ -f "$IMG" ] || { echo "no $IMG — make image once first"; exit 1; }
[ -f "$ROOTIMG" ] || { echo "no $ROOTIMG"; exit 1; }
[ "$(id -u)" -eq 0 ] && { echo "make run persist must run as you (not root) so the image can be attached"; exit 1; }

info_field() {
  diskutil info "$1" | awk -F': *' -v key="$2" 'index($0, key":") { sub(/^[^:]*: */, ""); gsub(/[[:space:]]+$/, ""); print; exit }'
}

echo "== updating BIAROOT in $IMG"
dev=$(hdiutil attach -nomount -imagekey diskimage-class=CRawDiskImage "$IMG" | head -1 | awk '{print $1}')
trap 'hdiutil detach "$dev" -quiet 2>/dev/null || true' EXIT
disk=${dev#/dev/}

root_dev=""
for s in $(diskutil list "$disk" | awk '/^ *[0-9]+:/ {print $NF}'); do
  case $s in ${disk}s*) ;; *) continue ;; esac
  media=$(info_field "$s" "Media Name")
  vol=$(info_field "$s" "Volume Name")
  ptype=$(info_field "$s" "Partition Type")
  # diskutil does not surface the GPT name BIAROOT; the slice is "Linux Filesystem".
  if [ "$media" = BIAROOT ] || [ "$vol" = BIAROOT ] || [ "$ptype" = "Linux Filesystem" ]; then
    root_dev=$s
    break
  fi
done
[ -n "$root_dev" ] || { echo "no Linux root slice in $IMG"; diskutil list "$disk"; exit 1; }

fs=$(info_field "$root_dev" "File System Personality")
case "$fs" in
  *ExFAT*|*FAT*|*MS-DOS*) echo "refusing $root_dev: filesystem is $fs"; exit 1 ;;
esac

img_bytes=$(stat -f%z "$ROOTIMG")
part=$(diskutil info "$root_dev" | sed -n 's/.*Disk Size:.*(\([0-9]*\) Bytes).*/\1/p' | head -1)
if [ -z "$part" ] || [ "$img_bytes" -gt "$part" ]; then
  echo "BIAROOT $root_dev is ${part:-unknown} bytes, root.img is $img_bytes. Refusing to write."
  exit 1
fi
echo "== writing $ROOTIMG -> /dev/r$root_dev ($img_bytes bytes into $part-byte slice)"
part_off=$(diskutil info "$root_dev" | awk '/Partition Offset:/ {
  if (match($0, /[0-9]+ Bytes/)) {
    s = substr($0, RSTART, RLENGTH)
    sub(/ Bytes/, "", s)
    print s
    exit
  }
}')
diskutil unmount "$root_dev" >/dev/null 2>&1 || true
dd if="$ROOTIMG" of="/dev/r$root_dev" bs=1m
sync

# GRUB loads the kernel from the EFI slice. The new initramfs is what installs
# or updates the internal disk, so the EFI files have to match this root.
boot_dev=""
for s in $(diskutil list "$disk" | awk '/^ *[0-9]+:/ {print $NF}'); do
  case $s in ${disk}s*) ;; *) continue ;; esac
  media=$(info_field "$s" "Media Name")
  vol=$(info_field "$s" "Volume Name")
  ptype=$(info_field "$s" "Partition Type")
  if [ "$media" = BIABOOT ] || [ "$vol" = BIABOOT ] || [ "$ptype" = EFI ]; then
    boot_dev=$s
    break
  fi
done
kern_dir=$(cd "$(dirname "$ROOTIMG")" && pwd)
if [ -n "$boot_dev" ] && [ -f "$kern_dir/vmlinuz-lts" ] && [ -f "$kern_dir/initramfs-lts" ]; then
  diskutil mount "$boot_dev" >/dev/null
  boot=$(info_field "$boot_dev" "Mount Point")
  if [ -n "$boot" ] && [ "$boot" != "Not applicable (no file system)" ]; then
    echo "== kernel + initramfs -> $boot"
    cp -f "$kern_dir/vmlinuz-lts" "$kern_dir/initramfs-lts" "$boot/"
    sync
    diskutil unmount "$boot_dev" >/dev/null 2>&1 || true
  else
    echo "BIABOOT did not mount; the next boot still has the previous kernel"
  fi
else
  echo "no EFI slice or no $kern_dir/vmlinuz-lts; kernel on the stick was left as-is"
fi

hdiutil detach "$dev" -quiet
trap - EXIT
if [ -n "${part:-}" ] && [ "$part" -gt "$img_bytes" ]; then
  if [ -z "${part_off:-}" ]; then
    echo "could not read the BIAROOT offset; the filesystem was not grown to the partition"
    exit 1
  fi
  ARCH="$ARCH" tools/grow-ext4.sh "$IMG" "$part_off" "$part"
fi
echo "== flash root updated. internal.img was left in place."
echo "   the next disk boot copies shipped files onto the running system. MCPs are updated from this image's /home/bia/MCPs."
