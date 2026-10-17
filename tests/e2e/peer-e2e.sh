#!/bin/bash
# End-to-end QEMU test of agent-to-agent talk over the web API, with a certificate
# request that a person approves later.
#
#   tests/e2e/peer-e2e.sh images   a.img + b.img from dist/$ARCH/root.img (FMM keys from ./fmm.json,
#                              current agent + system MCP from dist/$ARCH)
#   tests/e2e/peer-e2e.sh run      bia-b is told bia-a's address and asked to get access and say
#                              hello. It files POST /v1/pki/requests and polls the request id.
#                              After a deliberate delay, root on bia-a tells bia to approve; bia-b
#                              collects the certificate, sends a chat message over mTLS and
#                              reports bia-a's answer and the approval message (a code word root gave bia-a).
#   tests/e2e/peer-e2e.sh check    inspect bia-a's request store, issued cert and chat
#   tests/e2e/peer-e2e.sh all      images + run + check
#
# Each VM has user-net on eth0 (FMM) and a private link on eth1 (QEMU stream netdev):
# bia-a 10.77.0.2, bia-b 10.77.0.3. Files live in dist/pe-$ARCH/.
# Env: ARCH (default arm64), MEM (4G), CPUS (4), DELAY (seconds before approving, 90)
set -euo pipefail
SELF=$(cd "$(dirname "$0")" && pwd)/$(basename "$0")
cd "$(dirname "$SELF")/../.."
ROOT=$(pwd)
ARCH=${ARCH:-arm64}
PE=$ROOT/dist/pe-$ARCH
case $ARCH in
  arm64) PLATFORM=linux/arm64 ;;
  x86_64) PLATFORM=linux/amd64 ;;
  *) echo "ARCH=$ARCH" >&2; exit 1 ;;
esac
mkdir -p "$PE"

image_tool() {
  docker run --rm -i --privileged --platform "$PLATFORM" -e ARCH="$ARCH" \
    -v "$PE":/ut:rw -v "$ROOT":/work:ro "bia-build:$ARCH" python3 /work/tests/e2e/peer-e2e-img.py "$@"
}

do_images() {
  pgrep -qf "qemu-system.*$PE/" 2>/dev/null && { echo "this test's QEMU is running; stop it first" >&2; exit 1; }
  for n in a b; do
    ARCH=$ARCH IMG="$PE/$n.img" IMAGE_MB="${IMAGE_MB:-12288}" tools/mkimage.sh
    image_tool prep "$n"
  done
}

do_run() {
  ARCH=$ARCH LE=$PE MEM=${MEM:-4G} CPUS=${CPUS:-4} DELAY=${DELAY:-90} python3 "$ROOT/tests/e2e/peer-e2e-drive.py"
}

case ${1:-} in
  images) do_images ;;
  run) do_run ;;
  check) image_tool check ;;
  all) do_images; do_run; image_tool check; echo "======== peer e2e passed" ;;
  *) sed -n '2,18p' "$SELF" >&2; exit 2 ;;
esac
