#!/bin/bash
# Build a UEFI ISO that installs bia and waits in land. No GGUF.
# Host (macOS) re-enters the bia-build container. Default ARCH is x86_64.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
ARCH=${ARCH:-x86_64}
case $ARCH in
  arm64) PLATFORM=linux/arm64; EFI_NAME=BOOTAA64.EFI; FILEARCH=aarch64 ;;
  x86_64) PLATFORM=linux/amd64; EFI_NAME=BOOTX64.EFI; FILEARCH=x86_64 ;;
  *) echo "unknown ARCH $ARCH"; exit 1 ;;
esac

if [ "$(uname -s)" != Linux ] || [ ! -f /work/builders/build.sh ]; then
  if ! docker image inspect "bia-build:$ARCH" >/dev/null 2>&1; then
    echo "no bia-build:$ARCH — run: make ARCH=$ARCH builder"
    exit 1
  fi
  exec docker run --rm --privileged --platform "$PLATFORM" \
    -v "$ROOT":/work -v "bia-cache-$ARCH":/cache \
    -e ARCH="$ARCH" \
    "bia-build:$ARCH" /work/tools/mkiso.sh
fi

IMG="/work/dist/$ARCH/root.img"
[ -f "$IMG" ] || { echo "no $IMG — make ARCH=$ARCH rootfs first"; exit 1; }
if ! command -v xorriso >/dev/null 2>&1 || ! command -v mkfs.vfat >/dev/null 2>&1; then
  apt-get update -qq
  DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends xorriso dosfstools >/dev/null
fi

CACHE=${CACHE:-/cache}
STAGE="$CACHE/iso-stage-$ARCH"
MNT="$CACHE/iso-mnt-$ARCH"
FATMNT="$CACHE/iso-fat-$ARCH"
OUT="/work/dist/$ARCH/bia-land.iso"

cleanup() {
  umount "$STAGE/dev" 2>/dev/null || true
  umount "$STAGE/sys" 2>/dev/null || true
  umount "$STAGE/proc" 2>/dev/null || true
  umount "$FATMNT" 2>/dev/null || true
  umount "$MNT" 2>/dev/null || true
}
trap cleanup EXIT
cleanup
rm -rf "$STAGE" "$FATMNT"
mkdir -p "$STAGE" "$MNT" "$FATMNT" "$(dirname "$OUT")"

echo "== reading $IMG"
mount -o loop,ro "$IMG" "$MNT"
tar -C "$MNT" \
  --exclude=./home/bia/model \
  --exclude=./swapfile \
  --exclude=./proc --exclude=./sys --exclude=./dev --exclude=./run --exclude=./tmp \
  -cf - . | tar -C "$STAGE" -xf -
umount "$MNT"
mkdir -p "$STAGE/proc" "$STAGE/sys" "$STAGE/dev" "$STAGE/run" "$STAGE/tmp" "$STAGE/boot/EFI/BOOT"
chmod 1777 "$STAGE/tmp"

install -m 0755 /work/source/os/overlay/usr/local/sbin/bia-land "$STAGE/usr/local/sbin/bia-land"
install -m 0644 /work/source/os/overlay/etc/systemd/system/bia-land.service "$STAGE/etc/systemd/system/bia-land.service"
install -m 0644 /work/source/os/overlay/etc/systemd/system/bia.service "$STAGE/etc/systemd/system/bia.service"
mkdir -p "$STAGE/etc/systemd/system/multi-user.target.wants"
ln -sfn ../bia-land.service "$STAGE/etc/systemd/system/multi-user.target.wants/bia-land.service"
printf 'setup\n' >"$STAGE/etc/bia-land-mode"
cat >"$STAGE/etc/fstab" <<'EOF'
# live ISO. The installer writes the disk fstab.
tmpfs /tmp tmpfs defaults 0 0
EOF
install -m 0644 /work/source/os/overlay/etc/initramfs-tools/modules "$STAGE/etc/initramfs-tools/modules"

BIN_SRC="/work/dist/$ARCH/bia-mcp-carry"
BIN_DEST="$STAGE/home/bia/MCPs/carry/bin/$FILEARCH/bia-mcp-carry"
if [ -f "$BIN_SRC" ]; then
  mkdir -p "$(dirname "$BIN_DEST")"
  cp -f "$BIN_SRC" "$BIN_DEST"
  chmod 755 "$BIN_DEST"
  uid=$(awk -F: '$1=="bia"{print $3}' "$STAGE/etc/passwd")
  gid=$(awk -F: '$1=="bia"{print $4}' "$STAGE/etc/passwd")
  if [ -n "${uid:-}" ]; then
    chown "$uid:${gid:-$uid}" "$BIN_DEST"
  fi
fi
[ -x "$BIN_DEST" ] || { echo "no carry binary at $BIN_DEST — make ARCH=$ARCH rust mcps rootfs first"; exit 1; }

echo "== initramfs (iso9660)"
mount --bind /proc "$STAGE/proc"
mount --bind /sys "$STAGE/sys"
mount --bind /dev "$STAGE/dev"
chroot "$STAGE" update-initramfs -u
umount "$STAGE/dev"
umount "$STAGE/sys"
umount "$STAGE/proc"

VMLINUZ=$(ls "$STAGE/boot"/vmlinuz-* 2>/dev/null | grep -v '\.old$' | head -1)
INITRD=$(ls "$STAGE/boot"/initrd.img-* 2>/dev/null | grep -v '\.old$' | head -1)
[ -n "$VMLINUZ" ] && [ -n "$INITRD" ] || { echo "missing kernel or initrd in the root image"; exit 1; }
[ -f "/work/source/flash/EFI/BOOT/$EFI_NAME" ] || { echo "no shim at /work/source/flash/EFI/BOOT/$EFI_NAME — make rootfs first"; exit 1; }

kb=$(( ($(stat -c%s "$VMLINUZ") + $(stat -c%s "$INITRD") + 64 * 1048576 + 1048575) / 1048576 + 64 ))
echo "== EFI boot image ${kb}M (shim + signed GRUB)"
FAT="$CACHE/efiboot-$ARCH.img"
rm -f "$FAT"
dd if=/dev/zero of="$FAT" bs=1M count="$kb" status=none
mkfs.vfat -F 32 -n BIAEFI "$FAT" >/dev/null
mount -o loop "$FAT" "$FATMNT"
chmod +x /work/tools/esp-grub.sh
ISO_CMDLINE='root=LABEL=BIAISO rootfstype=iso9660 rootwait ro systemd.volatile=overlay fsck.mode=skip net.ifnames=0 console=tty0 console=ttyS0,115200 bia.land=1'
# The live ISO never updates itself: no rescue overlay on its EFI image.
SYSROOT="$STAGE" RESCUE_CPIO= CMDLINE="$ISO_CMDLINE" /work/tools/esp-grub.sh "$ARCH" "$FATMNT" BIAISO
# ISO ESP is labelled BIAEFI, not BIABOOT — rewrite search in grub.cfg.
for cfg in "$FATMNT/boot/grub/grub.cfg" "$FATMNT/EFI/BOOT/grub.cfg" "$FATMNT/EFI/debian/grub.cfg"; do
  [ -f "$cfg" ] || continue
  cat >"$cfg" <<EOF
set default=0
set timeout=10
search --no-floppy --set=root --label BIAEFI
if [ -z "\$root" ]; then
  search --no-floppy --set=root --file /vmlinuz-lts
fi
menuentry "bia" {
  linux /vmlinuz-lts $ISO_CMDLINE
  initrd /initramfs-lts
}
EOF
done
cp -f "$VMLINUZ" "$FATMNT/vmlinuz-lts"
cp -f "$INITRD" "$FATMNT/initramfs-lts"
# Keep a copy under the ISO tree too (land installer reads /boot).
mkdir -p "$STAGE/boot/EFI/BOOT"
cp -a "$FATMNT/EFI/." "$STAGE/boot/EFI/"
# Booted as a CD, signed GRUB sees only the ISO9660 tree: its prefix is
# (cd0)/EFI/debian, not the FAT image. Point that config at the ISO's /boot.
for cfg in "$STAGE/EFI/debian/grub.cfg" "$STAGE/boot/grub/grub.cfg"; do
  mkdir -p "$(dirname "$cfg")"
  cat >"$cfg" <<EOF
set default=0
set timeout=10
search --no-floppy --set=root --label BIAISO
menuentry "bia" {
  linux /boot/$(basename "$VMLINUZ") $ISO_CMDLINE
  initrd /boot/$(basename "$INITRD")
}
EOF
done
umount "$FATMNT"
cp -f "$FAT" "$STAGE/boot/efiboot.img"
rm -f "$FAT"

echo "== $OUT"
rm -f "$OUT"
xorriso -as mkisofs \
  -V BIAISO \
  -J -R -iso-level 3 \
  -e boot/efiboot.img \
  -no-emul-boot \
  -isohybrid-gpt-basdat \
  -o "$OUT" \
  "$STAGE"
ls -lh "$OUT"
echo "== boot this ISO, not the flash image. It asks DHCP or a static address, installs onto a disk, and waits in land."
