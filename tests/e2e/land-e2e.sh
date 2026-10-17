#!/bin/bash
# End-to-end QEMU test of the land ISO and carry fly/land.
#
#   tests/e2e/land-e2e.sh images   bia-land.iso (make iso), a blank land disk, fly.img from
#                              dist/$ARCH/root.img with ./fmm.json, a marker and eth1
#   tests/e2e/land-e2e.sh run      VM 1 boots the ISO, installs onto its blank disk (static
#                              address), reboots and waits in land. VM 2 (fly.img) asks
#                              its agent to fly there; both sides confirm the code; the
#                              land disk unfolds, reboots and starts the flyer's agent.
#   tests/e2e/land-e2e.sh check    inspect the land disk
#   tests/e2e/land-e2e.sh all      images + run + check
#
# The two VMs share a private L2 link (QEMU stream netdev over a unix socket): land eth0
# 10.77.0.2/24, fly eth1 10.77.0.3/24. The flyer also has user-net on eth0 for FMM and
# routes (nftables masquerade, test image only) the land VM to the internet: bia-land
# does not take a flight until deb.debian.org is reachable.
# Files live in dist/le-$ARCH/. Env: ARCH (default arm64), MEM (4G), CPUS (4)
set -euo pipefail
SELF=$(cd "$(dirname "$0")" && pwd)/$(basename "$0")
cd "$(dirname "$SELF")/../.."
ROOT=$(pwd)
ARCH=${ARCH:-arm64}
LE=$ROOT/dist/le-$ARCH
case $ARCH in
  arm64) PLATFORM=linux/arm64 ;;
  x86_64) PLATFORM=linux/amd64 ;;
  *) echo "ARCH=$ARCH" >&2; exit 1 ;;
esac
mkdir -p "$LE"

image_tool() {
  docker run --rm -i --privileged --platform "$PLATFORM" -e ARCH="$ARCH" \
    -v "$LE":/ut:rw -v "$ROOT":/work:ro "bia-build:$ARCH" python3 /work/tests/e2e/land-e2e-img.py "$@"
}

token() {
  [ -s "$LE/token" ] || od -An -N8 -tx1 /dev/urandom | tr -d ' \n' >"$LE/token"
  cat "$LE/token"
}

do_images() {
  pgrep -q qemu-system 2>/dev/null && { echo "QEMU is running; stop it first" >&2; exit 1; }
  make ARCH="$ARCH" iso
  rm -f "$LE/land.img" "$LE/token"
  qemu-img create -f raw "$LE/land.img" "${LAND_GB:-16}G" >/dev/null
  ARCH=$ARCH IMG="$LE/fly.img" IMAGE_MB="${IMAGE_MB:-12288}" tools/mkimage.sh
  image_tool prep-fly "$(token)"
}

do_run() {
  ARCH=$ARCH LE=$LE MEM=${MEM:-4G} CPUS=${CPUS:-4} python3 "$ROOT/tests/e2e/land-e2e-drive.py"
}

case ${1:-} in
  images) do_images ;;
  run) do_run ;;
  check) image_tool check "$(token)" ;;
  all) do_images; do_run; image_tool check "$(token)"; echo "======== land e2e passed" ;;
  *) sed -n '2,17p' "$SELF" >&2; exit 2 ;;
esac
