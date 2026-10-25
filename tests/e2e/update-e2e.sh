#!/bin/bash
# End-to-end QEMU test of the offline update through the rescue image.
#
#   tests/e2e/update-e2e.sh images            flash.img + disk.img from dist/$ARCH/root.img
#   tests/e2e/update-e2e.sh prep <scenario>   releases + markers for one scenario
#   tests/e2e/update-e2e.sh run <scenario>    boot, trigger /update, follow it to the end
#   tests/e2e/update-e2e.sh check <scenario>  inspect the updated root
#   tests/e2e/update-e2e.sh patch             push overlay scripts + dist binaries into
#                                         both images, rebuild their rescue overlay
#                                         and initramfs
#   tests/e2e/update-e2e.sh all [scenario…]   prep+run+check (default: disk flash rescue)
#
# Scenarios (dst = the system that gets the new OS and keeps its agent):
#   disk    running from the disk, update the disk from a newer flash   (/update)
#   flash   running from the flash, update the flash from a newer disk  (/update)
#   rescue  running from the flash, push the flash onto the disk         (carry_rescue)
#   fsck    running from the disk: its root starts with a broken inode checksum
#           (/usr/bin/df) and the "with errors" state; the boot check (initramfs)
#           repairs it, then /fsck checks it offline in the rescue image  (carry_fsck)
#   twin    like disk, but both ESPs are BIABOOT / FAT serial 1234-5678 (as flashed) and
#           the disk mounts /boot by label: bia-esp-id gives the disk's ESP its own
#   internet  running from the disk, update the disk from a release served over HTTP
#             (real bia-<arch>.img stamped 9.9.9; the flash stays attached but is not
#             newer; /update internet). Needs dist/$ARCH/bia-$ARCH.img (make img).
#
# Images live in dist/ut-$ARCH/ (separate from bia.img / internal.img).
# The test images get ./fmm.json (if present) so the agent uses FMM, not the local model.
# Env: ARCH (default arm64), MEM (4G), CPUS (4), TIMEOUT (minutes per phase, 20)
set -euo pipefail
SELF=$(cd "$(dirname "$0")" && pwd)/$(basename "$0")
cd "$(dirname "$SELF")/../.."
ROOT=$(pwd)
ARCH=${ARCH:-arm64}
UT=$ROOT/dist/ut-$ARCH
FLASH=$UT/flash.img
DISK=$UT/disk.img
IMAGE_MB=${IMAGE_MB:-20480}  # sparse; the internet scenario needs ~2x the release image free
case $ARCH in
  arm64) PLATFORM=linux/arm64 ;;
  x86_64) PLATFORM=linux/amd64 ;;
  *) echo "ARCH=$ARCH" >&2; exit 1 ;;
esac
BUILDER=bia-build:$ARCH
mkdir -p "$UT"

# Run the python image tool in the builder (loop devices, ext4/vfat tools).
image_tool() {
  docker run --rm -i --privileged --platform "$PLATFORM" -e ARCH="$ARCH" \
    -v "$UT":/ut:rw -v "$ROOT":/work:ro -v "$ROOT/tests/e2e/update-e2e-img.py":/img.py:ro \
    "$BUILDER" python3 /img.py "$@"
}

do_images() {
  pgrep -q qemu-system 2>/dev/null && { echo "QEMU is running; stop it first" >&2; exit 1; }
  for img in "$FLASH" "$DISK"; do
    ARCH=$ARCH IMG=$img IMAGE_MB=$IMAGE_MB tools/mkimage.sh
  done
  image_tool make-disk
}

scenario_ok() {
  case ${1:-} in disk|flash|rescue|internet|fsck|twin) ;; *) echo "scenario: disk | flash | rescue | internet | fsck | twin" >&2; exit 2 ;; esac
}

do_prep() { scenario_ok "$1"; image_tool prep "$1"; }
do_check() { scenario_ok "$1"; image_tool check "$1"; }

do_run() {
  scenario_ok "$1"
  local boot=disk
  case $1 in disk|internet|fsck|twin) ;; *) boot=flash ;; esac
  local log=$UT/run-$1.log http=
  if [ "$1" = internet ]; then
    # the "GitHub release": the guest reaches the host's loopback as 10.0.2.2
    python3 -m http.server 8099 --bind 127.0.0.1 --directory "$UT/release" >"$UT/http-release.log" 2>&1 &
    http=$!
    trap 'kill $http 2>/dev/null || true' EXIT
  fi
  python3 "$ROOT/tests/e2e/update-e2e-drive.py" "$1" "$log" -- env \
    ARCH="$ARCH" MODE=headless NET=user BOOT="$boot" IMG="$FLASH" INTERNAL=1 INTERNAL_IMG="$DISK" \
    MEM="${MEM:-4G}" CPUS="${CPUS:-4}" SOCK="$UT/serial.sock" MON="$UT/monitor.sock" \
    WEBPORT="${WEBPORT:-8089}" FRESH=0 "$ROOT/tools/run-qemu.sh"
  local rc=$?
  [ -n "$http" ] && kill "$http" 2>/dev/null
  trap - EXIT
  return $rc
}

cmd=${1:-}
shift || true
case $cmd in
  images) do_images ;;
  prep) do_prep "${1:-}" ;;
  run) do_run "${1:-}" ;;
  check) do_check "${1:-}" ;;
  patch) image_tool patch ;;
  all)
    [ $# -gt 0 ] || set -- disk flash rescue
    for s in "$@"; do
      echo "======== scenario $s"
      do_prep "$s"
      do_run "$s"
      do_check "$s"
    done
    echo "======== all passed: $*"
    ;;
  *) sed -n '2,20p' "$SELF" >&2; exit 2 ;;
esac
