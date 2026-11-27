#!/bin/bash
# Statuses while bia works, in QEMU with the real image and a real cloud model (./fmm.json):
# the work tree's agent in dist/<arch>/root.img, a fake telegram bus with slow tools in place of
# the real one (status-qemu-fake.py), a group with two people and a Spanish voice call.
#
#   tests/e2e/status-qemu.sh images   build the agent, patch a copy of root.img, write the stick
#                                     (bia-deploy image, decoded onto a disk, keys on its ESP)
#   tests/e2e/status-qemu.sh run      boot it until the fake bus says STATUS-E2E-DONE, /poweroff
#   tests/e2e/status-qemu.sh check    the logs and the agent's state off the stick, checked
#   tests/e2e/status-qemu.sh all      images + run + check
#
# Env: ARCH (arm64), MEM (4G), CPUS (4), TIMEOUT (minutes for the scenario, 25). Files in
# dist/se-<arch>/ (the stick holds the keys: delete it when done).
set -euo pipefail
SELF=$(cd "$(dirname "$0")" && pwd)/$(basename "$0")
cd "$(dirname "$SELF")/../.."
ROOT=$(pwd)
ARCH=${ARCH:-arm64}
SE=$ROOT/dist/se-$ARCH
case $ARCH in arm64) PLATFORM=linux/arm64 ;; x86_64) PLATFORM=linux/amd64 ;; *) echo "ARCH=$ARCH?"; exit 2 ;; esac
mkdir -p "$SE"

image_tool() {
  docker run --rm -i --privileged --platform "$PLATFORM" -v "$SE":/se:rw -v "$ROOT":/work:ro \
    "bia-build:$ARCH" python3 /work/tests/e2e/status-qemu-img.py "$@"
}

do_images() {
  pgrep -q qemu-system 2>/dev/null && { echo "QEMU is running; stop it first" >&2; exit 1; }
  [ -f fmm.json ] || { echo "need ./fmm.json (a real cloud model)"; exit 1; }
  [ -f "dist/$ARCH/root.img" ] || { echo "no dist/$ARCH/root.img (make ARCH=$ARCH build)"; exit 1; }
  echo "== the work tree's agent (release)"
  docker run --rm --platform "$PLATFORM" -v "$ROOT":/work -v "bia-cache-$ARCH":/cache -w /work \
    -e CARGO_HOME=/cache/cargo -e CARGO_TARGET_DIR="/cache/target-$ARCH-se" -e BIA_TG_API_ID=1 -e BIA_TG_API_HASH=test \
    "bia-build:$ARCH" bash -lc 'cargo build --release -q -p bia-agent && mkdir -p dist/se-'"$ARCH"'/bin &&
      strip -o dist/se-'"$ARCH"'/bin/bia-agent "$CARGO_TARGET_DIR/release/bia-agent"'
  cp "dist/$ARCH/root.img" "$SE/root.img"
  image_tool prep
  cargo build --release -q -p bia-deploy
  rm -f "$SE/stick.img" "$SE/bia.img"
  target/release/bia-deploy image --root "$SE/root.img" --esp source/flash --kern "dist/$ARCH" --arch "$ARCH" --out "$SE/bia.img"
  mv "$SE/bia.img" "$SE/stick.img"
  truncate -s $(( ${STICK_GB:-8} << 30 )) "$SE/stick.img"
  image_tool keys
  rm -f "$SE/root.img"
}

do_run() {
  ARCH=$ARCH LE=$SE MEM=${MEM:-4G} CPUS=${CPUS:-4} TIMEOUT=${TIMEOUT:-25} python3 "$ROOT/tests/e2e/status-qemu-drive.py"
}

do_check() {
  rm -rf "$SE/out"
  image_tool extract
  python3 "$ROOT/tests/e2e/status-qemu-check.py" "$SE/out"
}

case ${1:-} in
  images) do_images ;;
  run) do_run ;;
  check) do_check ;;
  all) do_images; do_run; do_check ;;
  *) sed -n '2,15p' "$SELF" >&2; exit 2 ;;
esac
