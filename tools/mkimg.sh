#!/bin/bash
# Build the minimal bia disk image dist/<arch>/bia-<arch>.img from a local build: GPT,
# ESP (shim, GRUB, kernel, initramfs, rescue image), BIOS GRUB (x86_64) and BIAROOT
# from root.img. No FMM keys, no model. Releases ship it as bia-<arch>.img.xz; bia-deploy
# writes it (GUI local-files.json, make flash, the e2e tests).
#
#   tools/mkimg.sh [arch]        (default $ARCH, else x86_64)
#   OUT=path.img tools/mkimg.sh  another output path
#   KERNEL=0                     keep dist/<arch>/vmlinuz-lts + initramfs-lts as they are
#                                (default: refresh them from root.img, tools/esp-kernel.sh)
set -euo pipefail
cd "$(dirname "$0")/.."
ARCH=${1:-${ARCH:-x86_64}}
DIST=dist/$ARCH
ROOTIMG=$DIST/root.img
OUT=${OUT:-$DIST/bia-$ARCH.img}
[ -f "$ROOTIMG" ] || { echo "mkimg: no $ROOTIMG (make ARCH=$ARCH build)"; exit 1; }
[ "${KERNEL:-1}" = 0 ] || tools/esp-kernel.sh "$ROOTIMG" "$DIST"
cargo build --release -q -p bia-deploy
target_dir=$(cargo metadata --format-version=1 --no-deps \
  | python3 -c 'import sys,json; print(json.load(sys.stdin)["target_directory"])')
"$target_dir/release/bia-deploy" image --root "$ROOTIMG" --esp source/flash --kern "$DIST" \
  --arch "$ARCH" --out "$OUT.tmp"
mv -f "$OUT.tmp" "$OUT"
echo "== $OUT ($(( $(stat -f%z "$OUT" 2>/dev/null || stat -c%s "$OUT") >> 20 )) MiB)"
