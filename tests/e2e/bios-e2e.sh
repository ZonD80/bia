#!/bin/bash
# End-to-end QEMU test of legacy BIOS (SeaBIOS, no UEFI): boot the flash image, let the
# agent install onto the internal disk, then boot that disk with the stick removed.
#
#   tests/e2e/bios-e2e.sh images   flash.img written by bia-deploy from dist/x86_64/bia-x86_64.img
#                              (./fmm.json; current dist binaries patched in) + a blank internal disk
#   tests/e2e/bios-e2e.sh run      SeaBIOS boots the stick -> agent asked to install onto the
#                              internal disk (carry_to_disk) -> powers off (BIOS cannot be
#                              told to boot the disk) -> QEMU restarted without the stick ->
#                              SeaBIOS boots the disk -> agent on BIADISK
#   tests/e2e/bios-e2e.sh check    inspect the internal disk (GPT, BIOS GRUB, ESP, fstab)
#   tests/e2e/bios-e2e.sh all      images + run + check
#
# x86_64 under TCG (no acceleration on Apple silicon): slow. Files in dist/be-x86_64/.
# Env: MEM (6G), CPUS (4), TIMEOUT (minutes for boot/copy phases, 60)
set -euo pipefail
SELF=$(cd "$(dirname "$0")" && pwd)/$(basename "$0")
cd "$(dirname "$SELF")/../.."
ROOT=$(pwd)
ARCH=x86_64
BE=$ROOT/dist/be-$ARCH
mkdir -p "$BE"

image_tool() {
  docker run --rm -i --privileged --platform linux/amd64 -e ARCH="$ARCH" \
    -v "$BE":/ut:rw -v "$ROOT":/work:ro "bia-build:$ARCH" python3 /work/tests/e2e/bios-e2e-img.py "$@"
}

do_images() {
  pgrep -q qemu-system 2>/dev/null && { echo "QEMU is running; stop it first" >&2; exit 1; }
  # The stick is written the way users get one: bia-deploy (native BIOS GRUB embed, FMM
  # keys on the ESP after the hello test, ext4 grown by bia-grow on first boot).
  tools/mkimg.sh "$ARCH"
  rm -f "$BE/flash.img"
  target/release/bia-deploy write --disk "$BE/flash.img" --image --size $(( ${STICK_GB:-16} << 30 )) \
    --os-image "dist/$ARCH/bia-$ARCH.img" --fmm fmm.json --yes
  rm -f "$BE/internal.img"
  qemu-img create -f raw "$BE/internal.img" "${INTERNAL_GB:-20}G" >/dev/null
  image_tool prep
}

do_run() {
  ARCH=$ARCH LE=$BE MEM=${MEM:-6G} CPUS=${CPUS:-4} TIMEOUT=${TIMEOUT:-60} python3 "$ROOT/tests/e2e/bios-e2e-drive.py"
}

case ${1:-} in
  images) do_images ;;
  run) do_run ;;
  check) image_tool check ;;
  all) do_images; do_run; image_tool check; echo "======== BIOS e2e passed" ;;
  *) sed -n '2,15p' "$SELF" >&2; exit 2 ;;
esac
