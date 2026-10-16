#!/bin/bash
# Boot dist/bia.img in QEMU via UEFI. MODE=gui (window + serial here) | serial (headless) | socket (serial on a unix socket)
# NET=bridge (default): vmnet-bridged onto this Mac's Wi-Fi so the guest DHCP's a LAN address.
# NET=user: SLIRP NAT + hostfwd 127.0.0.1:8080 -> guest :80
# FMM keys: ./fmm.json, source/flash/files/state/fmm.json, or FMM=path. Passed as fw_cfg
# opt/bia/fmm.json (guest: /sys/firmware/qemu_fw_cfg/by_name/opt/bia/fmm.json/raw).
# The flash image is not edited. The agent copies the file into files/state/fmm.json
# on boot when that state file is still missing.
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$(pwd)
ARCH=${ARCH:-arm64}; IMG=${IMG:-dist/bia.img}; MEM=${MEM:-8G}; CPUS=${CPUS:-6}; MODE=${MODE:-gui}
USB=${USB:-0}; SOCK=${SOCK:-/tmp/bia-serial.sock}; MON=${MON:-/tmp/bia-monitor.sock}
NET=${NET:-bridge}
WEBPORT=${WEBPORT:-8080}
# Second virtio disk. Boot installs the flash root here and uses it as /.
INTERNAL=${INTERNAL:-1}
INTERNAL_GB=${INTERNAL_GB:-20}
INTERNAL_IMG=${INTERNAL_IMG:-dist/internal.img}
FW=/opt/homebrew/share/qemu
QEMU_BIN=${QEMU_BIN:-$(command -v qemu-system-aarch64)}
mkdir -p dist

wifi_if() {
  if [ -n "${WIFI_IF:-}" ]; then
    echo "$WIFI_IF"
    return
  fi
  networksetup -listallhardwareports 2>/dev/null | awk '/Hardware Port: Wi-Fi/{getline; print $2; exit}'
}

has_gpt() {
  [ "$(dd if="$1" bs=1 skip=512 count=8 2>/dev/null)" = "EFI PART" ]
}

# macOS DiskImages holds byte 100 of an attached raw image. QEMU's raw
# driver locks that byte and exits with "Failed to lock byte 100" until
# the image is detached.
detach_if_attached() {
  local img="$1" devs d
  [ -n "$img" ] && [ -f "$img" ] || return 0
  devs=$(python3 - "$img" <<'PY'
import os, plistlib, subprocess, sys
img = os.path.realpath(sys.argv[1])
info = plistlib.loads(subprocess.check_output(["hdiutil", "info", "-plist"]))
seen = []
for image in info.get("images") or []:
    path = image.get("image-path") or ""
    try:
        same = os.path.realpath(path) == img
    except OSError:
        same = path == img
    if not same:
        continue
    for ent in image.get("system-entities") or []:
        dev = ent.get("dev-entry") or ""
        rest = dev[len("/dev/disk"):] if dev.startswith("/dev/disk") else ""
        if rest.isdigit() and dev not in seen:
            seen.append(dev)
print("\n".join(seen))
PY
)
  [ -n "$devs" ] || return 0
  while IFS= read -r d; do
    [ -n "$d" ] || continue
    echo "== detaching $img ($d)"
    diskutil unmountDisk force "$d" >/dev/null 2>&1 || true
    diskutil eject force "$d" >/dev/null
    local i
    for i in 1 2 3 4 5 6 7 8 9 10; do
      [ -e "$d" ] || break
      sleep 0.1
    done
    if [ -e "$d" ]; then
      echo "still attached: $img ($d); QEMU cannot lock it"
      exit 1
    fi
  done <<<"$devs"
}

# Empty GPT so bash (fdisk/mkfs) can use the whole second disk.
write_empty_gpt() {
  python3 - "$1" <<'PY'
import os, struct, sys, uuid, zlib
path = sys.argv[1]
size = os.path.getsize(path)
sector = 512
n = size // sector
if n < 68:
    sys.exit("internal image is too small for a GPT")
entries = b"\x00" * (128 * 128)
entries_crc = zlib.crc32(entries) & 0xFFFFFFFF
disk_guid = uuid.uuid4().bytes_le

def header(current, backup, part_lba):
    h = bytearray(92)
    h[0:8] = b"EFI PART"
    struct.pack_into("<I", h, 8, 0x00010000)
    struct.pack_into("<I", h, 12, 92)
    struct.pack_into("<Q", h, 24, current)
    struct.pack_into("<Q", h, 32, backup)
    struct.pack_into("<Q", h, 40, 34)
    struct.pack_into("<Q", h, 48, n - 34)
    h[56:72] = disk_guid
    struct.pack_into("<Q", h, 72, part_lba)
    struct.pack_into("<I", h, 80, 128)
    struct.pack_into("<I", h, 84, 128)
    struct.pack_into("<I", h, 88, entries_crc)
    struct.pack_into("<I", h, 16, zlib.crc32(h) & 0xFFFFFFFF)
    return bytes(h).ljust(sector, b"\x00")

mbr = bytearray(sector)
mbr[450] = 0xEE
struct.pack_into("<I", mbr, 454, 1)
struct.pack_into("<I", mbr, 458, min(n - 1, 0xFFFFFFFF))
mbr[510:512] = b"\x55\xAA"
with open(path, "r+b") as f:
    f.write(mbr)
    f.write(header(1, n - 1, 2))
    f.write(entries)
    f.seek((n - 33) * sector)
    f.write(entries)
    f.seek((n - 1) * sector)
    f.write(header(n - 1, 1, n - 33))
    # The new GPT already covers the first 17KiB, including a whole-disk
    # superblock at byte 1024. A partitioned ext4 starts at 1MiB, and its
    # backup superblocks still hold the volume label, so zero those too.
    groups = (0, 1, 3, 5, 7, 9, 25, 27, 49, 81, 125, 243, 343, 729, 2187, 6561)
    for g in groups:
        off = 1024 * 1024 + g * 32768 * 4096
        if off + 4096 <= size:
            f.seek(off)
            f.write(b"\x00" * 4096)
    for g in groups:
        if g == 0:
            continue
        off = g * 32768 * 4096
        if off + 4096 <= size:
            f.seek(off)
            f.write(b"\x00" * 4096)
PY
}

ensure_internal_img() {
  case $INTERNAL_IMG in
    /*) ;;
    *) INTERNAL_IMG="$ROOT/$INTERNAL_IMG" ;;
  esac
  if [ "$INTERNAL" != 1 ]; then
    INTERNAL_IMG=""
    return
  fi
  mkdir -p "$(dirname "$INTERNAL_IMG")"
  if [ ! -f "$INTERNAL_IMG" ]; then
    echo "== creating ${INTERNAL_GB}GiB sparse $INTERNAL_IMG (QEMU disk, serial=internal)"
    if command -v qemu-img >/dev/null 2>&1; then
      qemu-img create -f raw "$INTERNAL_IMG" "${INTERNAL_GB}G" >/dev/null
    else
      dd if=/dev/zero of="$INTERNAL_IMG" bs=1048576 count=0 seek=$((INTERNAL_GB * 1024)) 2>/dev/null
    fi
  fi
  if [ "${WIPE_INTERNAL:-}" = 1 ]; then
    echo "== wiping $INTERNAL_IMG to an empty GPT"
    write_empty_gpt "$INTERNAL_IMG"
  elif ! has_gpt "$INTERNAL_IMG"; then
    echo "== empty GPT on $INTERNAL_IMG (the agent installs BIADISK on request)"
    write_empty_gpt "$INTERNAL_IMG"
  fi
}

ensure_internal_img

if [ "$NET" = bridge ] && [ "$(id -u)" -ne 0 ]; then
  echo "== vmnet-bridged on Wi-Fi needs root (guest gets a DHCP address from the LAN router)"
  exec sudo -E PATH="$PATH" HOME="$HOME" \
    QEMU_BIN="$QEMU_BIN" NET="$NET" WIFI_IF="${WIFI_IF:-}" \
    ARCH="$ARCH" IMG="$IMG" MEM="$MEM" CPUS="$CPUS" MODE="$MODE" \
    USB="$USB" SOCK="$SOCK" MON="$MON" WEBPORT="$WEBPORT" \
    BOOT="${BOOT:-flash}" \
    INTERNAL="$INTERNAL" INTERNAL_GB="$INTERNAL_GB" INTERNAL_IMG="$INTERNAL_IMG" \
    FMM="${FMM:-}" \
    -- "$ROOT/tools/run-qemu.sh"
fi

case $IMG in
  /*) ;;
  *) IMG="$ROOT/$IMG" ;;
esac
case $ARCH in
  arm64)
    [ -n "$QEMU_BIN" ] || QEMU_BIN=$(command -v qemu-system-aarch64)
    cp $FW/edk2-arm-vars.fd dist/efivars-arm64.fd
    QEMU=("$QEMU_BIN" -M virt -accel hvf -cpu host
      -drive if=pflash,format=raw,readonly=on,file=$FW/edk2-aarch64-code.fd
      -drive if=pflash,format=raw,file="$ROOT/dist/efivars-arm64.fd") ;;
  x86_64)
    QEMU_BIN=${QEMU_BIN/qemu-system-aarch64/qemu-system-x86_64}
    [ -x "$QEMU_BIN" ] || QEMU_BIN=$(command -v qemu-system-x86_64)
    cp $FW/edk2-i386-vars.fd dist/efivars-x86_64.fd
    QEMU=("$QEMU_BIN" -M q35 -accel tcg -cpu max
      -drive if=pflash,format=raw,readonly=on,file=$FW/edk2-x86_64-code.fd
      -drive if=pflash,format=raw,file="$ROOT/dist/efivars-x86_64.fd") ;;
esac
QEMU+=(-smp "$CPUS" -m "$MEM" -device virtio-rng-pci)
case $NET in
  user)
    QEMU+=(-netdev "user,id=n0,hostfwd=tcp:127.0.0.1:${WEBPORT}-:80" -device virtio-net-pci,netdev=n0)
    WEBMSG="web console from this Mac: http://127.0.0.1:${WEBPORT}/  (QEMU user-net NAT; do not open the 10.0.2.x address)"
    ;;
  bridge)
    IFACE=$(wifi_if)
    [ -n "$IFACE" ] || { echo "no Wi-Fi interface; set WIFI_IF=en0 (or the Device from networksetup -listallhardwareports)"; exit 1; }
    ifconfig "$IFACE" >/dev/null 2>&1 || { echo "Wi-Fi interface $IFACE is not present"; exit 1; }
    QEMU+=(-netdev "vmnet-bridged,id=n0,ifname=$IFACE" -device virtio-net-pci,netdev=n0)
    WEBMSG="web console: guest DHCP on Wi-Fi $IFACE (LAN IP printed at boot; not 127.0.0.1:${WEBPORT})"
    echo "== bridged onto $IFACE ($(ifconfig "$IFACE" | awk '/inet /{print $2; exit}'))"
    ;;
  *)
    echo "NET=$NET is not user|bridge"
    exit 1
    ;;
esac
BOOT=${BOOT:-flash}
case $INTERNAL_IMG in
  /*) ;;
  *) [ -z "$INTERNAL_IMG" ] || INTERNAL_IMG="$ROOT/$INTERNAL_IMG" ;;
esac
case $IMG in
  /*) ;;
  *) IMG="$ROOT/$IMG" ;;
esac
if [ "$BOOT" = disk ]; then
  QEMU+=(-drive if=none,id=d0,format=raw,file="$INTERNAL_IMG" -device virtio-blk-pci,drive=d0,serial=internal)
  if [ -f "$IMG" ]; then
    echo "== boot from internal disk $INTERNAL_IMG (serial=internal); flash $IMG stays attached (serial=flash)"
    QEMU+=(-drive if=none,id=d1,format=raw,file="$IMG" -device virtio-blk-pci,drive=d1,serial=flash)
  else
    echo "== boot from internal disk $INTERNAL_IMG (serial=internal)"
  fi
else
  echo "== boot from flash $IMG (serial=flash); internal disk stays attached (serial=internal)"
  QEMU+=(-drive if=none,id=d0,format=raw,file="$IMG" -device virtio-blk-pci,drive=d0,serial=flash)
  if [ -n "${INTERNAL_IMG:-}" ] && [ -f "$INTERNAL_IMG" ]; then
    QEMU+=(-drive if=none,id=d1,format=raw,file="$INTERNAL_IMG" -device virtio-blk-pci,drive=d1,serial=internal)
  fi
fi
# Same lookup as tools/flash.sh. The item name matches QEMU_FMM_ITEM in bia-agent.
fmm_file=""
if [ -n "${FMM:-}" ]; then
  fmm_file=$FMM
elif [ -f "$ROOT/fmm.json" ]; then
  fmm_file=$ROOT/fmm.json
elif [ -f "$ROOT/source/flash/files/state/fmm.json" ]; then
  fmm_file=$ROOT/source/flash/files/state/fmm.json
fi
if [ -n "$fmm_file" ]; then
  [ -f "$fmm_file" ] || { echo "FMM file not found: $fmm_file"; exit 1; }
  case $fmm_file in
    /*) ;;
    *) fmm_file="$ROOT/$fmm_file" ;;
  esac
  echo "== FMM keys from $fmm_file (QEMU fw_cfg; flash image unchanged)"
  QEMU+=(-fw_cfg "name=opt/bia/fmm.json,file=$fmm_file")
fi
case $MODE in
  gui) QEMU+=(-device virtio-gpu-pci -device qemu-xhci -device usb-kbd -device usb-tablet -display cocoa -serial mon:stdio) ;;
  serial) QEMU+=(-nographic) ;;
  socket) QEMU+=(-display none -monitor none -serial "unix:$SOCK,server,nowait") ;;
  # headless with a virtual screen + keyboard; drive it via the monitor socket (sendkey, screendump)
  headless) QEMU+=(-device ramfb -device qemu-xhci -device usb-kbd -display none
              -monitor "unix:$MON,server,nowait" -serial "unix:$SOCK,server,nowait") ;;
esac
detach_if_attached "$IMG"
if [ -n "${INTERNAL_IMG:-}" ] && [ -f "$INTERNAL_IMG" ]; then
  detach_if_attached "$INTERNAL_IMG"
fi
echo "${QEMU[@]}"
echo "$WEBMSG"
exec "${QEMU[@]}"
