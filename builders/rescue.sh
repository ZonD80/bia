#!/bin/bash
# Build the bia rescue overlay (rescue.cpio.gz) from a Debian sysroot.
#
#   builders/rescue.sh <sysroot> <out.cpio.gz>
#
# GRUB loads it after the ESP's own initramfs-lts with rdinit=/bia-rescue/init,
# so drivers, udev and the dynamic loader come from that initramfs and always
# match the kernel. This archive only adds /bia-rescue: the rescue script and
# the tools it needs (rsync, mount, e2fsck, efibootmgr, …) with their libraries.
# Runs inside the bia-build container; the sysroot must be the same arch.
set -euo pipefail
ROOT=${1:?sysroot}
OUT=${2:?out.cpio.gz}
STAGE_REL=tmp/bia-rescue-stage
STAGE=$ROOT/$STAGE_REL
SRC=$ROOT/usr/local/share/bia/rescue

[ -f "$SRC/rescue" ] || { echo "no $SRC/rescue (copy source/os/overlay first)"; exit 1; }
[ -x "$ROOT/usr/bin/rsync" ] || { echo "no rsync in $ROOT (apt-get install rsync)"; exit 1; }

BINS=(
  bash rsync mount umount mountpoint blkid lsblk findmnt e2fsck chattr efibootmgr setsid
  sync sleep cat cp mv rm mkdir ln ls chmod stat df du date tr head tail wc sort cut
  awk sed grep cmp dirname basename readlink tee uname dmesg insmod
  xz dd sha256sum blockdev partx
)

rm -rf "$STAGE"
mkdir -p "$STAGE/bia-rescue/bin" "$STAGE/bia-rescue/lib" "$STAGE/bia-rescue/mod"

in_root() { chroot "$ROOT" /usr/bin/env -i PATH=/usr/sbin:/usr/bin:/sbin:/bin "$@"; }

loader=
for l in /usr/lib/ld-linux-aarch64.so.1 /lib/ld-linux-aarch64.so.1 /usr/lib64/ld-linux-x86-64.so.2 /lib64/ld-linux-x86-64.so.2; do
  if [ -e "$ROOT$l" ]; then loader=$l; break; fi
done
[ -n "$loader" ] || { echo "no dynamic loader in $ROOT"; exit 1; }

declare -A LIBS=()
for b in "${BINS[@]}"; do
  path=$(in_root sh -c "command -v $b" || true)
  [ -n "$path" ] || { echo "rescue: $b missing in sysroot"; exit 1; }
  real=$(in_root readlink -f "$path")
  cp -L "$ROOT$real" "$STAGE/bia-rescue/bin/$b"
  # The loader prints "soname => /path (addr)" for every needed library.
  while read -r lib; do
    [ -n "$lib" ] && LIBS[$lib]=1
  done < <(in_root "$loader" --list "$real" 2>/dev/null | awk '$2 == "=>" && $3 ~ /^\// { print $3 }')
done
for lib in "${!LIBS[@]}"; do
  cp -L "$ROOT$lib" "$STAGE/bia-rescue/lib/$(basename "$lib")"
done

# Filesystem modules the rescue needs, with their dependencies, in load order.
# Do not rely on the main initramfs for these (MODULES=most may leave them out).
kver=$(ls "$ROOT/lib/modules" | sort -V | tail -1)
MDIR=$ROOT/lib/modules/$kver
WANT=(ext4 vfat nls_cp437 nls_ascii nls_utf8 nls_iso8859-1 efivarfs)
declare -A SEEN=()
: > "$STAGE/bia-rescue/mod/order"
add_mod() {
  local name=$1 rel dep
  rel=$(awk -v n="$name" '{ f = $1; sub(/:$/, "", f); b = f; sub(/.*\//, "", b); sub(/\.ko(\.(xz|zst|gz))?$/, "", b); gsub(/-/, "_", b); if (b == n) { print f; exit } }' "$MDIR/modules.dep")
  [ -n "$rel" ] || return 0   # built in (or absent)
  [ -n "${SEEN[$rel]:-}" ] && return 0
  SEEN[$rel]=1
  for dep in $(awk -v f="$rel:" '$1 == f { $1 = ""; print }' "$MDIR/modules.dep"); do
    dep=$(basename "$dep"); dep=${dep%%.ko*}; add_mod "${dep//-/_}"
  done
  local out=$STAGE/bia-rescue/mod/$(basename "${rel%%.ko*}").ko
  case $rel in
    *.ko) cp "$MDIR/$rel" "$out" ;;
    *.ko.xz) xz -dc "$MDIR/$rel" > "$out" ;;
    *.ko.zst) zstd -qdc "$MDIR/$rel" > "$out" ;;
    *.ko.gz) gzip -dc "$MDIR/$rel" > "$out" ;;
  esac
  basename "$out" >> "$STAGE/bia-rescue/mod/order"
}
for m in "${WANT[@]}"; do add_mod "${m//-/_}"; done

install -m 0755 "$SRC/init" "$STAGE/bia-rescue/init"
install -m 0755 "$SRC/rescue" "$STAGE/bia-rescue/rescue"
install -m 0644 "$SRC/takeover" "$STAGE/bia-rescue/takeover"
install -m 0755 "$ROOT/usr/local/sbin/bia-rescue-hook" "$STAGE/bia-rescue/bia-rescue-hook"
printf '%s kernel %s\n' "$(date -u +%Y%m%d%H%M%S)" "$kver" > "$STAGE/bia-rescue/version"

mkdir -p "$(dirname "$OUT")"
in_root sh -c "cd /$STAGE_REL && find . -mindepth 1 | sort | cpio -o -H newc -R 0:0 --quiet" | gzip -9n > "$OUT"
echo "== rescue overlay $OUT ($(du -h "$OUT" | cut -f1), ${#BINS[@]} tools, ${#LIBS[@]} libs, modules: $(tr '\n' ' ' < "$STAGE/bia-rescue/mod/order"), kernel $kver)"
rm -rf "$STAGE"
