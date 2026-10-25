#!/bin/bash
# Write a bootable bia stick: a small EFI partition and one ext4 root.
# New sticks: dist/$ARCH/bia-$ARCH.img (tools/mkimg.sh) written by bia-flash.
# The root image holds the OS, /home/bia (no model) and the swap file. BIAROOT
# spans the whole stick; bia-grow fills it with the ext4 on first boot.
# ERASE=1 wipes the stick. Without ERASE, a stick that already has BIAROOT is
# updated in place (boot + root).
#
#   ARCH=x86_64 make flash          # build x86_64, then write the USB (prompts for confirmation)
#   YES=1 DISK=disk10 tools/flash.sh
#   SKIP_BUILD=1 tools/flash.sh     # just write, using flash/EFI already present
#   ERASE=1 … tools/flash.sh        # wipe the stick even if it already runs bia
#   MEDIA=sd tools/flash.sh         # an SD card (built-in SD reader, which macOS calls internal)
#   MODEL=unsloth/Qwen3-4B-Instruct-2507-GGUF[:file.gguf] make flash
#                                   # download a GGUF from Hugging Face onto the stick
#                                   # (implies ERASE=1; HF_TOKEN for gated repos)
#   Local FMM keys (gitignored): ./fmm.json or source/flash/files/state/fmm.json
#   Optional path: FMM=path/to/fmm.json  (path only — never put the API key in env)
#   The FMM model is tested (one tool call) before anything is written; NO_TEST=1 skips that.
# New / erased sticks use installers/bia-flash (cross-platform). In-place refresh of an
# existing BIAROOT stick uses the macOS diskutil path below.
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$(pwd)
ARCH=${ARCH:-x86_64}
FLASH=${FLASH:-source/flash}
ROOT_MB=${ROOT_MB:-3072}
ROOTIMG=${ROOTIMG:-dist/$ARCH/root.img}
MODEL=${MODEL:-}
# The owner's language for bia-flash --lang (a code like ru, or a name; default: host locale).
BIA_LANG=${BIA_LANG:-}
# usb (a USB stick or card reader) or sd (an SD card in the Mac's built-in reader)
MEDIA=${MEDIA:-usb}
case $MEDIA in
  usb) WANT_PROTO=USB; WHAT_DISK="USB stick" ;;
  sd)  WANT_PROTO="Secure Digital"; WHAT_DISK="SD card" ;;
  *) echo "unknown MEDIA=$MEDIA (want usb or sd)"; exit 1 ;;
esac
if [ "$(id -u)" -eq 0 ]; then
  echo "run as yourself; the script sudo's only for disk writes"
  exit 1
fi

need_efi() {
  case $ARCH in
    x86_64) echo "$FLASH/EFI/BOOT/BOOTX64.EFI" ;;
    arm64)  echo "$FLASH/EFI/BOOT/BOOTAA64.EFI" ;;
    *) echo "unknown ARCH=$ARCH"; exit 1 ;;
  esac
}

if [ "${SKIP_BUILD:-0}" != 1 ]; then
  echo "== building $ARCH"
  make ARCH="$ARCH" ROOT_MB="$ROOT_MB" builder build
fi
EFI=$(need_efi)
[ -f "$EFI" ] || { echo "missing $EFI (build $ARCH first)"; exit 1; }
[ -f "$FLASH/bia.toml" ] || { echo "missing $FLASH/bia.toml"; exit 1; }
[ -f "$ROOTIMG" ] || { echo "missing $ROOTIMG (make build)"; exit 1; }
KERN_DIR=$(cd "$(dirname "$ROOTIMG")" && pwd)
tools/esp-kernel.sh "$ROOTIMG" "$KERN_DIR"

disk_bytes() {
  diskutil info "$1" | sed -n 's/.*Disk Size:.*(\([0-9]*\) Bytes).*/\1/p' | head -1
}

disk_proto() {
  diskutil info "$1" | awk -F': *' '/Protocol:/ {print $2; exit}'
}

disk_name() {
  diskutil info "$1" | awk -F': *' '/Device \/ Media Name:/ {print $2; exit}'
}

whole_id() {
  # /dev/rdisk10, disk10s1, rdisk10 -> disk10
  echo "$1" | sed -E 's|^/dev/||; s|^r||; s|^(disk[0-9]+).*|\1|'
}

disk_removable() {
  diskutil info "$1" | awk -F': *' '/Removable Media:/ {print $2; exit}'
}

# The disk macOS runs from (never a target, whatever it reports).
boot_disk() {
  whole_id "$(diskutil info / | awk -F': *' '/Part of Whole:/ {print $2; exit}')"
}

detect_usb() {
  local ids=()
  # the built-in SD reader is not "external": look at every physical disk for SD
  local scope="external physical"
  [ "$MEDIA" = sd ] && scope="physical"
  while read -r line; do
    case $line in
      /dev/disk*) ids+=("$(whole_id "$line")") ;;
    esac
  done < <(diskutil list $scope 2>/dev/null | awk '/^\/dev\/disk/ {print $1}')
  local usb=()
  local d proto boot
  boot=$(boot_disk)
  for d in "${ids[@]+"${ids[@]}"}"; do
    [ "$d" = "$boot" ] && continue
    proto=$(disk_proto "$d")
    if [ "$proto" = "$WANT_PROTO" ]; then
      usb+=("$d")
    fi
  done
  if [ ${#usb[@]} -eq 1 ]; then
    echo "${usb[0]}"
    return
  fi
  if [ ${#usb[@]} -eq 0 ]; then
    echo "no $WHAT_DISK found. Insert it and retry, or set DISK=diskN" >&2
    diskutil list $scope >&2 || true
    exit 1
  fi
  echo "several ${WHAT_DISK}s; set DISK=diskN:" >&2
  for d in "${usb[@]}"; do
    echo "  $d  $(disk_name "$d")  $(diskutil info "$d" | awk -F': *' '/Disk Size:/ {print $2; exit}')" >&2
  done
  exit 1
}

if [ -n "${DISK:-}" ]; then
  DISK=$(whole_id "$DISK")
else
  DISK=$(detect_usb)
fi

proto=$(disk_proto "$DISK")
internal=$(diskutil info "$DISK" | awk -F': *' '/Device Location:/ {print $2; exit}')
[ "$DISK" = "$(boot_disk)" ] && { echo "refusing $DISK: macOS runs from it"; exit 1; }
if [ "$proto" != "$WANT_PROTO" ]; then
  echo "refusing $DISK: protocol is '$proto' (want $WANT_PROTO for the $WHAT_DISK; MEDIA=usb or MEDIA=sd)"
  exit 1
fi
if [ "$MEDIA" = sd ]; then
  # the built-in reader counts as internal; the card in it must be removable media
  [ "$(disk_removable "$DISK")" = Removable ] || { echo "refusing $DISK: not removable media"; exit 1; }
else
  [ "$internal" = Internal ] && { echo "refusing $DISK: it is an internal disk"; exit 1; }
fi

BYTES=$(disk_bytes "$DISK")
[ -n "$BYTES" ] && [ "$BYTES" -gt 0 ] || { echo "could not read size of $DISK"; exit 1; }
NAME=$(disk_name "$DISK")
HUMAN=$(diskutil info "$DISK" | awk -F': *' '/Disk Size:/ {print $2; exit}')

mp() { diskutil info "$1" | awk -F': +' '/Mount Point/ {print $2; exit}'; }

# Existing stick: do not repartition. BIAROOT is the one root filesystem.
EXISTING=0
# The model goes onto the raw end of a fresh BIAROOT, so it needs the bia-flash path.
[ -n "$MODEL" ] && ERASE=1
if [ "${ERASE:-0}" != 1 ]; then
  for s in $(diskutil list "$DISK" | awk '/^ *[0-9]+:/ {print $NF}'); do
    case $s in ${DISK}s*) ;; *) continue ;; esac
    vol=$(diskutil info "$s" | awk -F': *' '/Volume Name:/ {print $2; exit}')
    case $vol in
      BIAROOT) EXISTING=1 ;;
    esac
  done
fi

# The stick is the EFI slice plus one ext4 root; there is no second partition
# for the weights or for swap. bia-flash checks room for MODEL itself.
ESP_MIB=256
BIOS_MIB=1
GPT_SLACK_MIB=16
root_mib=$(( ($(stat -f%z "$ROOTIMG") + 1048575) / 1048576 ))
need_mib=$((ESP_MIB + BIOS_MIB + root_mib + GPT_SLACK_MIB))
stick_mib=$((BYTES / 1024 / 1024))
if [ "$EXISTING" != 1 ] && [ "$stick_mib" -lt "$need_mib" ]; then
  echo "$DISK is too small ($HUMAN): root.img is ${root_mib}MiB."
  echo "Need ${need_mib}MiB (ESP ${ESP_MIB} + BIOS ${BIOS_MIB} + that root). This stick is ${stick_mib}MiB."
  exit 1
fi

echo "== target $DISK  $NAME  $HUMAN  $WHAT_DISK"
if [ -n "$MODEL" ]; then
  what="root, model $MODEL from Hugging Face"
else
  what="root, no local model"
fi
if [ "$EXISTING" = 1 ]; then
  echo "   existing BIAROOT: refresh boot and root — ${root_mib}MiB $what"
else
  echo "   this ERASES the whole stick and writes bia ($ARCH): EFI + BIOS + one ext4 root"
  echo "   root.img is ${root_mib}MiB — $what"
fi
if [ "${YES:-0}" != 1 ]; then
  printf "type %s to continue: " "$DISK" >/dev/tty
  read -r ans </dev/tty
  [ "$ans" = "$DISK" ] || { echo "aborted"; exit 1; }
fi

FLASHDIR="$ROOT/$FLASH"

# ---- bia-flash (preferred for full erase / new sticks) ----
if [ "$EXISTING" != 1 ]; then
  echo "== building bia-flash"
  cargo build --release -p bia-flash -q
  target_dir=$(cargo metadata --format-version=1 --no-deps 2>/dev/null \
    | python3 -c 'import sys,json; print(json.load(sys.stdin)["target_directory"])')
  FLASH_BIN="$target_dir/release/bia-flash"
  [ -x "$FLASH_BIN" ] || { echo "missing $FLASH_BIN"; exit 1; }
  fmm_args=()
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
    fmm_args=(--fmm "$fmm_file")
    [ "${NO_TEST:-0}" = 1 ] && fmm_args+=(--no-test)
    echo "== FMM keys from $fmm_file -> EFI/bia/fmm.json"
  else
    [ -n "$MODEL" ] || echo "note: no fmm.json (tried ./fmm.json, source/flash/files/state/fmm.json) and no MODEL — stick has no brain until one is configured"
    fmm_args=(--no-fmm)
  fi
  model_args=()
  [ -n "$MODEL" ] && model_args=(--model "$MODEL")
  [ -n "$BIA_LANG" ] && model_args+=(--lang "$BIA_LANG")
  echo "== unmounting $DISK"
  diskutil unmountDisk force "$DISK" >/dev/null || true
  echo "== bia-flash write /dev/r$DISK"
  echo "== bia disk image from $ROOTIMG"
  KERNEL=0 OUT="$KERN_DIR/bia-$ARCH.img" tools/mkimg.sh "$ARCH"
  sudo --preserve-env=HF_TOKEN "$FLASH_BIN" write \
    --disk "/dev/r$DISK" \
    --os-image "$KERN_DIR/bia-$ARCH.img" \
    --size "$BYTES" \
    --yes \
    "${fmm_args[@]+"${fmm_args[@]}"}" \
    "${model_args[@]+"${model_args[@]}"}"
  echo "== done. eject and boot the $WHAT_DISK (UEFI)."
  exit 0
fi


# Existing stick: refresh boot and root in place with diskutil; no repartitioning.
sudo env DISK="$DISK" ARCH="$ARCH" ROOTIMG="$(cd "$(dirname "$ROOTIMG")" && pwd)/$(basename "$ROOTIMG")" KERN_DIR="$KERN_DIR" FLASHDIR="$FLASHDIR" ROOT="$(pwd)" \
  bash -s <<'EOF'
set -euo pipefail
mp() { diskutil info "$1" | awk -F': +' '/Mount Point/ {print $2; exit}'; }
part_bytes() {
  diskutil info "$1" | sed -n 's/.*Disk Size:.*(\([0-9]*\) Bytes).*/\1/p' | head -1
}
# diskutil's "M" is decimal MB (1e6). root.img is GNU truncate 4096M = 4096 MiB.
write_root() {
  local dev=$1 img_bytes part
  img_bytes=$(stat -f%z "$ROOTIMG")
  part=$(part_bytes "$dev")
  if [ -z "$part" ] || [ "$img_bytes" -gt "$part" ]; then
    echo "BIAROOT $dev is ${part:-unknown} bytes, root.img is $img_bytes bytes."
    echo "The stick needs a larger root slice. Re-flash with ERASE=1."
    exit 1
  fi
  echo "== writing Debian root -> $dev ($img_bytes bytes into $part-byte slice)"
  diskutil unmount "$dev" >/dev/null 2>&1 || true
  dd if="$ROOTIMG" of="/dev/r$dev" bs=1m
}

echo "== unmounting $DISK"
diskutil unmountDisk force "$DISK" >/dev/null || true

identify() {
  boot_dev=""; root_dev=""
  for s in $(diskutil list "$DISK" | awk '/^ *[0-9]+:/ {print $NF}'); do
    case $s in ${DISK}s*) ;; *) continue ;; esac
    vol=$(diskutil info "$s" | awk -F': *' '/Volume Name:/ {print $2; exit}')
    fs=$(diskutil info "$s" | awk -F': *' '/File System Personality:/ {print $2; exit}')
    ptype=$(diskutil info "$s" | awk -F': *' '/Partition Type:/ {print $2; exit}')
    case "$ptype" in
      *EFI*) [ -z "$boot_dev" ] && boot_dev=$s; continue ;;
      *Swap*|*Linux*Swap*) continue ;;
      *Linux*|*Ext4*|*Unix*) [ -z "$root_dev" ] && root_dev=$s; continue ;;
    esac
    case "$fs" in
      *MS-DOS*|*FAT*) [ -z "$boot_dev" ] && boot_dev=$s; continue ;;
      *Ext4*|*Linux*) [ -z "$root_dev" ] && root_dev=$s; continue ;;
    esac
    [ "$vol" = BIABOOT ] && boot_dev=$s
    [ "$vol" = BIAROOT ] && root_dev=$s
  done
}

echo "== keeping the partition table and rewriting the root image"
identify
if [ -z "${root_dev:-}" ]; then
  echo "this stick has no BIAROOT partition. Re-flash with ERASE=1 to get Debian sys-on-USB."
  exit 1
fi
img_bytes=$(stat -f%z "$ROOTIMG")
part=$(part_bytes "$root_dev")
if [ -z "$part" ] || [ "$img_bytes" -gt "$part" ]; then
  echo "BIAROOT $root_dev is ${part:-unknown} bytes, root.img is $img_bytes. Slice is too small. Re-flash with ERASE=1."
  exit 1
fi
write_root "$root_dev"
diskutil list "$DISK"
[ -n "$boot_dev" ] || { echo "no EFI/BIABOOT slice"; diskutil list "$DISK"; exit 1; }

diskutil mount "$boot_dev" >/dev/null || true
boot=$(mp "$boot_dev")
[ -n "$boot" ] && [ "$boot" != "Not applicable (no file system)" ] || { echo "BIABOOT did not mount ($boot_dev)"; exit 1; }

echo "== copying boot -> $boot"
rm -rf "$boot/EFI" "$boot/boot"
mkdir -p "$boot/EFI" "$boot/boot"
# flash/EFI also holds per-arch kernel copies from the build (vmlinuz-lts-<arch>,
# initramfs-lts-<arch>); the ESP boots /vmlinuz-lts, and they would fill it.
rsync -rt --exclude='/vmlinuz-lts*' --exclude='/initramfs-lts*' "$FLASHDIR/EFI/" "$boot/EFI/"
if [ -d "$FLASHDIR/boot" ]; then
  cp -R "$FLASHDIR/boot/." "$boot/boot/"
fi
cp -f "${KERN_DIR}/vmlinuz-lts" "${KERN_DIR}/initramfs-lts" "$boot/"
# Rescue image overlay (GRUB hook in grub.cfg boots it while bia-rescue/next.cfg exists).
if [ -f "$KERN_DIR/rescue.cpio.gz" ]; then
  mkdir -p "$boot/bia-rescue"
  cp -f "$KERN_DIR/rescue.cpio.gz" "$boot/bia-rescue/rescue.cpio.gz"
else
  echo "warning: no $KERN_DIR/rescue.cpio.gz; this image cannot run carry_update"
fi
case $ARCH in
  arm64)
    rm -f "$boot/EFI/BOOT/BOOTX64.EFI" "$boot/EFI/BOOT/grubx64.efi" "$boot/EFI/BOOT/mmx64.efi"
    rm -f "$boot/EFI/debian/BOOTX64.EFI" "$boot/EFI/debian/grubx64.efi"
    ;;
  x86_64)
    rm -f "$boot/EFI/BOOT/BOOTAA64.EFI" "$boot/EFI/BOOT/grubaa64.efi" "$boot/EFI/BOOT/mmaa64.efi"
    rm -f "$boot/EFI/debian/BOOTAA64.EFI" "$boot/EFI/debian/grubaa64.efi"
    ;;
esac
sync
diskutil unmountDisk force "$DISK" >/dev/null

# Legacy BIOS: embed GRUB into the protective MBR + BIOS boot partition.
if [ "$ARCH" = x86_64 ] && [ -f "$FLASHDIR/boot/grub/i386-pc/core.img" ]; then
  echo "== embedding BIOS GRUB on /dev/r$DISK"
  python3 "$ROOT/tools/grub-bios-embed.py" --raw-pc "/dev/r$DISK" "$FLASHDIR/boot/grub/i386-pc" \
    || echo "BIOS embed failed (UEFI Secure Boot path still works)"
fi

diskutil eject "$DISK" >/dev/null
echo "== $DISK ejected. Boot UEFI (Secure Boot OK with stock shim) or legacy BIOS."
echo "   the root grows over the whole stick on first boot (bia-grow)."
EOF
