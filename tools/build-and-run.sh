#!/bin/bash
# Build the system, then boot QEMU from the flash image or the internal disk.
#   tools/build-and-run.sh flash|disk none|flash|disk|everything
# Cleanup of flash rebuilds bia.img (files, model and conversation start fresh).
# Cleanup of disk replaces the internal disk image with an empty GPT.
# No cleanup rewrites BIAROOT in the existing image (make persist-root) and keeps the internal disk.
set -euo pipefail
cd "$(dirname "$0")/.."
from=${1:-}
clean=${2:-}
case $from in
  flash|disk) ;;
  *) echo "usage: build-and-run.sh flash|disk none|flash|disk|everything"; exit 1 ;;
esac
case $clean in
  none|flash|disk|everything) ;;
  *) echo "usage: build-and-run.sh flash|disk none|flash|disk|everything"; exit 1 ;;
esac

pgrep qemu-system >/dev/null && { echo "QEMU is running; stop it first"; exit 1; }
[ "$(id -u)" -eq 0 ] && { echo "run this as you, not root"; exit 1; }

ARCH=${ARCH:-arm64}
wipe_disk=0
case $clean in
  disk|everything) wipe_disk=1 ;;
esac

case $clean in
  flash|everything)
    echo "== rebuilding the flash image"
    make build
    make image
    ;;
  none|disk)
    if [ ! -f dist/bia.img ]; then
      echo "== no flash image yet; building one"
      make build
      make image
    else
      echo "== updating the flash image root (internal disk kept)"
      make persist-root
    fi
    ;;
esac

echo "== boot from $from"
exec env ARCH="$ARCH" BOOT="$from" WIPE_INTERNAL="$wipe_disk" MODE="${MODE:-gui}" NET="${NET:-bridge}" INTERNAL="${INTERNAL:-1}" tools/run-qemu.sh
