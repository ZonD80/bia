#!/bin/bash
# Build a minimal Debian sys root (debootstrap minbase + systemd + Debian kernel) and ESP files.
# Runs inside the bia-build container (Debian/glibc host + Debian guest). Agent/MCPs are dynamic.
#   builders/rootfs.sh <arm64|x86_64>
set -euo pipefail
ARCH=${1:-arm64}
case $ARCH in
  arm64)
    DEBARCH=arm64; FILEARCH=aarch64; EFI_NAME=BOOTAA64.EFI; KPKG=linux-image-arm64
    BOOT_PKGS="shim-signed grub-efi-arm64-signed grub-efi-arm64-bin grub-common"
    ;;
  x86_64)
    DEBARCH=amd64; FILEARCH=x86_64; EFI_NAME=BOOTX64.EFI; KPKG=linux-image-amd64
    BOOT_PKGS="shim-signed grub-efi-amd64-signed grub-efi-amd64-bin grub-common grub-pc-bin"
    ;;
  *) echo "unknown arch $ARCH"; exit 1 ;;
esac

CACHE=${CACHE:-/cache}
OUT=/work/dist/$ARCH
# Docker Desktop bind-mounts /work from macOS; that FS cannot store setuid. Keep the tree on /cache.
ROOT=$CACHE/rootfs-$ARCH
ESP=/work/source/flash/EFI
AGENT="$OUT/bia-agent"
ROOT_MB=${ROOT_MB:-7100}
DEBIAN_REL=${DEBIAN_REL:-trixie}
MIRROR=${MIRROR:-http://deb.debian.org/debian}

[ -f "$AGENT" ] || { echo "no agent at $AGENT — build rust first"; exit 1; }
command -v debootstrap >/dev/null || { echo "debootstrap missing — rebuild the builder image"; exit 1; }

echo "== debian $DEBIAN_REL minbase + $KPKG ($DEBARCH)"
mkdir -p "$CACHE" "$OUT" "$ESP/BOOT" "$CACHE/debian-apt-$DEBARCH"
# leftover tree from a previous run may still have proc/sys/dev mounted
if [ -d "$ROOT" ]; then
  umount -l "$ROOT/dev/pts" 2>/dev/null || true
  umount -l "$ROOT/dev" 2>/dev/null || true
  umount -l "$ROOT/sys" 2>/dev/null || true
  umount -l "$ROOT/proc" 2>/dev/null || true
fi
rm -rf "$ROOT"
mkdir -p "$ROOT"

KEYRING=""
for k in \
  /usr/share/keyrings/debian-archive-keyring.gpg \
  /usr/share/keyrings/debian-archive-keyring.pgp
do
  [ -f "$k" ] && KEYRING=$k && break
done
DEB_OPTS=(--variant=minbase --arch="$DEBARCH" --cache-dir="$CACHE/debian-apt-$DEBARCH")
if [ -n "$KEYRING" ]; then
  DEB_OPTS+=(--keyring="$KEYRING")
else
  echo "   warning: no debian-archive-keyring; debootstrap --no-check-gpg"
  DEB_OPTS+=(--no-check-gpg)
fi

debootstrap "${DEB_OPTS[@]}" "$DEBIAN_REL" "$ROOT" "$MIRROR"

mkdir -p "$ROOT/dev" "$ROOT/proc" "$ROOT/sys" "$ROOT/run" "$ROOT/tmp" \
  "$ROOT/boot" \
  "$ROOT/usr/local/sbin" "$ROOT/home/bia" "$ROOT/var/log" \
  "$ROOT/var/cache/apt/archives"
chmod 1777 "$ROOT/tmp"

cp /etc/resolv.conf "$ROOT/etc/resolv.conf"
cat > "$ROOT/etc/apt/sources.list" <<EOF
deb $MIRROR $DEBIAN_REL main
deb $MIRROR $DEBIAN_REL-updates main
deb http://security.debian.org/debian-security $DEBIAN_REL-security main
EOF

cat > "$ROOT/usr/sbin/policy-rc.d" <<'EOF'
#!/bin/sh
exit 101
EOF
chmod +x "$ROOT/usr/sbin/policy-rc.d"

mount -t proc proc "$ROOT/proc"
mount -t sysfs sys "$ROOT/sys"
mount --bind /dev "$ROOT/dev"
mount -t devpts devpts "$ROOT/dev/pts" 2>/dev/null || true
cleanup() {
  umount -l "$ROOT/dev/pts" 2>/dev/null || true
  umount -l "$ROOT/dev" 2>/dev/null || true
  umount -l "$ROOT/sys" 2>/dev/null || true
  umount -l "$ROOT/proc" 2>/dev/null || true
}
trap cleanup EXIT

echo "== overlay"
cp -a /work/source/os/overlay/. "$ROOT/"
install -m 0755 "$AGENT" "$ROOT/usr/local/sbin/bia-agent"

export DEBIAN_FRONTEND=noninteractive
export SYSTEMD_OFFLINE=1
in_chroot() { chroot "$ROOT" env DEBIAN_FRONTEND=noninteractive SYSTEMD_OFFLINE=1 "$@"; }

echo "== apt-get $KPKG + systemd extras"
in_chroot apt-get update
in_chroot apt-get install -y --no-install-recommends \
  systemd systemd-sysv udev kmod \
  ifupdown dhcpcd-base iproute2 iputils-ping \
  "$KPKG" initramfs-tools \
  $BOOT_PKGS \
  e2fsprogs dosfstools exfatprogs ntfs-3g \
  util-linux fdisk parted mount procps rsync \
  efibootmgr \
  bash ca-certificates wget curl bzip2 \
  python3-minimal \
  wpasupplicant iw \
  tzdata adduser passwd login \
  kbd kexec-tools \
  alsa-utils \
  xserver-xorg-core xserver-xorg-input-libinput xauth x11-xserver-utils x11-utils \
  xdotool scrot \
  openbox tint2 x11vnc novnc kitty fonts-dejavu-core fonts-noto-color-emoji fontconfig \
  libvulkan1 mesa-vulkan-drivers \
  libgomp1 libssl3 libstdc++6 libopus0 \
  ffmpeg

# Intel Arc / Xe via Level Zero. Missing on some suites; Vulkan still covers those GPUs.
in_chroot apt-get install -y --no-install-recommends libze1 libze-intel-gpu1 \
  || echo "   intel level-zero not in this suite; Arc uses Vulkan"

in_chroot adduser --disabled-password --gecos "" --home /home/bia --shell /bin/bash bia 2>/dev/null || true
in_chroot usermod -aG video,render,input,audio bia 2>/dev/null || true
in_chroot install -d -o bia -g bia -m 755 /home/bia
in_chroot install -d -o bia -g bia -m 700 /home/bia/.config /home/bia/.local
in_chroot install -d -o bia -g bia -m 700 /home/bia/.config/bia-chrome /home/bia/.local/share
in_chroot chsh -s /bin/bash root >/dev/null 2>&1 || true
ln -sf /usr/share/zoneinfo/UTC "$ROOT/etc/localtime"

# overlay again so dpkg cannot clobber units/fstab
cp -a /work/source/os/overlay/. "$ROOT/"
install -m 0755 "$AGENT" "$ROOT/usr/local/sbin/bia-agent"

echo "== systemd enable"
in_chroot systemctl enable bia-grow.service
in_chroot systemctl enable bia-update.service
in_chroot systemctl enable bia.service
in_chroot systemctl enable getty@tty2.service
in_chroot systemctl enable networking.service
in_chroot systemctl mask getty@tty1.service
in_chroot systemctl mask serial-getty@ttyS0.service
# arm64: the SPCR console is ttyAMA0; a getty there would eat the agent's serial input.
in_chroot systemctl mask serial-getty@ttyAMA0.service
in_chroot systemctl mask systemd-resolved.service || true
in_chroot systemctl set-default multi-user.target

# empty machine-id so first boot generates a unique one
: > "$ROOT/etc/machine-id"
rm -f "$ROOT/var/lib/dbus/machine-id"

echo "== llama.cpp prebuilt servers"
/work/builders/llama-prebuilt.sh "$ARCH" "$ROOT"

echo "== speech (sherpa STT/TTS + ntgcalls media helper)"
chmod +x /work/builders/speech-prebuilt.sh
/work/builders/speech-prebuilt.sh "$ARCH" "$ROOT"

echo "== bia release"
# MCPs ship in /home/bia/MCPs (copied from flash/ when the ext4 image is packed).
# Chrome policy is written here so it is part of the root and the shipped list.
for d in /work/source/flash/MCPs/*; do
  [ -f "$d/mcp.json" ] || continue
  name=$(basename "$d")
  if ! ls "$d/bin/$FILEARCH/"* >/dev/null 2>&1; then
    echo "MCP $name has no bin/$FILEARCH binary — make rust && make mcps before rootfs"
    exit 1
  fi
done
rm -f "$OUT/chrome-native-host" "$OUT/chrome-user"
if [ -x "$OUT/bia-mcp-chrome" ] && [ -d /work/source/flash/MCPs/chrome ]; then
  stage=$(mktemp -d)
  cp -a /work/source/flash/MCPs/chrome "$stage/chrome"
  ( cd "$stage/chrome" && BIA_SYSROOT="$ROOT" \
    BIA_CHROME_MCP_BIN="/home/bia/MCPs/chrome/bin/$FILEARCH/bia-mcp-chrome" \
    BIA_CHROME_NATIVE_HOST="/home/bia/MCPs/chrome/native-host" \
    "$OUT/bia-mcp-chrome" --install-only )
  cp -a "$stage/chrome/native-host" "$OUT/chrome-native-host"
  cp -a "$stage/chrome/chrome-user" "$OUT/chrome-user"
  rm -rf "$stage"
fi
{
  (cd /work/source/os/overlay && find . -type f | sed 's|^\./||' | grep -v '^etc/fstab$')
  echo usr/local/sbin/bia-agent
  echo usr/local/share/bia/rescue.cpio.gz
  (cd "$ROOT" && find usr/local/lib/bia/llama -type f 2>/dev/null | sed 's|^\./||')
  (cd "$ROOT" && find usr/local/lib/bia/speech -type f 2>/dev/null | sed 's|^\./||')
  (cd "$ROOT" && find etc/opt/chrome etc/opt/google/chrome etc/chromium -type f 2>/dev/null | sed 's|^\./||')
} | sort -u > "$ROOT/etc/bia-shipped"
date -u +%Y%m%d%H%M%S > "$ROOT/etc/bia-release"

echo "== initramfs"
in_chroot update-initramfs -u || in_chroot update-initramfs -c -k all

VMLINUZ=$(ls "$ROOT/boot"/vmlinuz-* 2>/dev/null | grep -v '\.old$' | head -1)
INITRD=$(ls "$ROOT/boot"/initrd.img-* 2>/dev/null | grep -v '\.old$' | head -1)
[ -n "$VMLINUZ" ] && [ -n "$INITRD" ] || { echo "missing vmlinuz/initrd in /boot"; ls -la "$ROOT/boot"; exit 1; }
cp "$VMLINUZ" "$OUT/vmlinuz-lts"
cp "$INITRD" "$OUT/initramfs-lts"
# Keep per-arch names on the shared flash/EFI tree so the other ISA is not overwritten.
# Do not unwrap/modify the kernel: Debian Secure Boot signatures must stay intact.
cp "$VMLINUZ" "$ESP/vmlinuz-lts-$FILEARCH"
cp "$INITRD" "$ESP/initramfs-lts-$FILEARCH"

echo "== rescue image overlay"
# Shipped in the root (bia-esp-grub / bia-update install it onto an ESP) and
# next to the kernel for mkimage/flash.
/work/builders/rescue.sh "$ROOT" "$ROOT/usr/local/share/bia/rescue.cpio.gz"
cp "$ROOT/usr/local/share/bia/rescue.cpio.gz" "$OUT/rescue.cpio.gz"

# Stock Secure Boot: shim (Microsoft-signed) + Debian-signed GRUB under flash/.
# Also stages i386-pc core.img for legacy BIOS (x86_64).
# flash/EFI/* becomes volume /EFI/*; flash/boot/grub becomes volume /boot/grub.
chmod +x /work/tools/esp-grub.sh
# flash/ is shared by both arches: only the hook goes there; mkimage/flash copy
# the per-arch rescue.cpio.gz onto the ESP.
SYSROOT="$ROOT" RESCUE_CPIO= CMDLINE="root=LABEL=BIAROOT rootfstype=ext4 rootwait rw net.ifnames=0 console=tty0 console=ttyS0,115200" \
  /work/tools/esp-grub.sh "$ARCH" /work/source/flash BIAROOT
# Per-arch copies stay under flash/EFI for the shared tree; FAT root names are
# filled at flash/mkimage time from dist/$ARCH via esp-kernel.sh.

in_chroot apt-get clean
rm -rf "$ROOT/var/lib/apt/lists/"*
rm -f "$ROOT/usr/sbin/policy-rc.d"
# Docker's resolv.conf is not valid in the guest; DHCP rewrites this at boot.
printf 'nameserver 1.1.1.1\nnameserver 8.8.8.8\n' > "$ROOT/etc/resolv.conf"

cleanup
trap - EXIT

echo "== ext4 image ${ROOT_MB}M"
IMG="$OUT/root.img"
rm -f "$IMG"
USED=$(du -sxm "$ROOT" | awk '{print $1}')
FLASH_MB=0
if [ -d /work/source/flash ]; then
  FLASH_MB=$(du -sxm /work/source/flash | awk '{print $1}')
  # Models never go into the image (bia-flash puts one on the stick); a dev's
  # local *.gguf in flash/model must not count.
  gguf_mb=0
  while IFS= read -r -d '' f; do
    b=$(stat -c%s "$f" 2>/dev/null || stat -f%z "$f")
    gguf_mb=$((gguf_mb + (b + 1048575) / 1048576))
  done < <(find /work/source/flash -name '*.gguf' -print0 2>/dev/null)
  FLASH_MB=$((FLASH_MB - gguf_mb))
  [ "$FLASH_MB" -lt 0 ] && FLASH_MB=0
fi
NEED=$((USED + FLASH_MB + 256 + 64))
if [ "$NEED" -gt "$ROOT_MB" ]; then
  ROOT_MB=$(( (NEED + 63) / 64 * 64 ))
  echo "OS ${USED}M + /home/bia ${FLASH_MB}M + swap 256M need ${NEED}M; root image grows to ${ROOT_MB}M"
elif [ "$NEED" -lt "$ROOT_MB" ]; then
  # Only as large as what is packed: bia-grow fills the stick on first boot.
  ROOT_MB=$(( (NEED + 63) / 64 * 64 ))
  echo "OS ${USED}M + /home/bia ${FLASH_MB}M + swap 256M → root ${ROOT_MB}M"
fi
truncate -s "${ROOT_MB}M" "$IMG"
mkfs.ext4 -F -L BIAROOT "$IMG" >/dev/null
MNT=$CACHE/rootfs-mnt-$ARCH
mkdir -p "$MNT"
mount -o loop "$IMG" "$MNT"
# skip bind-mounted proc/sys/dev even if umount failed
tar -C "$ROOT" --one-file-system -cf - . | tar -C "$MNT" -xf -
mkdir -p "$MNT/proc" "$MNT/sys" "$MNT/dev/pts" "$MNT/run/lock" "$MNT/tmp" "$MNT/home/bia"
chmod 1777 "$MNT/tmp"
if [ -d /work/source/flash ]; then
  echo "== /home/bia from flash/ (MCPs, files; no model)"
  # files/state and files/inbox are machine-local (FMM keys, sessions, conversation):
  # ship the empty folders only. Keys reach a stick through bia-flash --fmm, never the image.
  tar_ex=(--exclude=./EFI --exclude=./boot --exclude=./lost+found
    --exclude='./files/state/*' --exclude='./files/inbox/*/*'
    --exclude='.DS_Store' --exclude='._*' --exclude='*.gguf')
  tar -C /work/source/flash "${tar_ex[@]}" -cf - . | tar -C "$MNT/home/bia" -xf -
fi
if [ -f "$OUT/chrome-native-host" ] && [ -d "$MNT/home/bia/MCPs/chrome" ]; then
  cp -a "$OUT/chrome-native-host" "$MNT/home/bia/MCPs/chrome/native-host"
  chmod 755 "$MNT/home/bia/MCPs/chrome/native-host"
  [ -f "$OUT/chrome-user" ] && cp -a "$OUT/chrome-user" "$MNT/home/bia/MCPs/chrome/chrome-user"
fi
bia_uid=$(awk -F: '$1=="bia"{print $3}' "$ROOT/etc/passwd")
bia_gid=$(awk -F: '$1=="bia"{print $4}' "$ROOT/etc/passwd")
if [ -n "$bia_uid" ]; then
  chown -R "$bia_uid:${bia_gid:-$bia_uid}" "$MNT/home/bia"
fi
chmod 755 "$MNT/home/bia"
echo "== swap file 256 MiB"
# Fully allocated (swapon refuses files with holes) with a real mkswap header.
dd if=/dev/zero of="$MNT/swapfile" bs=1M count=256 status=none
chmod 600 "$MNT/swapfile"
mkswap "$MNT/swapfile" >/dev/null
umount "$MNT"
rmdir "$MNT" || true
e2fsck -fy "$IMG" >/dev/null || true
cleanup
rm -rf "$ROOT" || true
ls -lh "$IMG" "/work/source/flash/EFI/BOOT/$EFI_NAME" "$OUT/vmlinuz-lts" "$ESP/vmlinuz-lts-$FILEARCH"
echo "== rootfs done ($ARCH, debian $DEBIAN_REL, ${USED}M used)"
