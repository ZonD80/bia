#!/bin/bash
# Boot QEMU from the flash image or the internal disk. Does not build.
#   tools/run-only.sh flash|disk none|flash|disk|everything
# Cleanup of flash rewrites bia.img from dist/<arch>/root.img (fresh /home/bia: files,
# model, conversation). Nothing is rebuilt.
# Cleanup of disk replaces the internal disk image with an empty GPT.
set -euo pipefail
cd "$(dirname "$0")/.."
from=${1:-}
clean=${2:-}
case $from in
  flash|disk) ;;
  *) echo "usage: run-only.sh flash|disk none|flash|disk|everything"; exit 1 ;;
esac
case $clean in
  none|flash|disk|everything) ;;
  *) echo "usage: run-only.sh flash|disk none|flash|disk|everything"; exit 1 ;;
esac

pgrep qemu-system >/dev/null && { echo "QEMU is running; stop it first"; exit 1; }
[ "$(id -u)" -eq 0 ] && { echo "run this as you, not root"; exit 1; }
# Booting from the internal disk does not need the flash image.
# Flash boot still does.
if [ "$from" = flash ] || [ "$clean" = flash ] || [ "$clean" = everything ]; then
  [ -f dist/bia.img ] || { echo "no dist/bia.img"; exit 1; }
fi

reset_flash() {
  echo "== rewriting dist/bia.img from dist/${ARCH:-arm64}/root.img"
  make ARCH="${ARCH:-arm64}" image
}

wipe_disk=0
case $clean in
  flash|everything) reset_flash ;;
esac
case $clean in
  disk|everything) wipe_disk=1 ;;
esac

echo "== boot from $from"
exec env ARCH="${ARCH:-arm64}" BOOT="$from" WIPE_INTERNAL="$wipe_disk" MODE="${MODE:-gui}" NET="${NET:-bridge}" INTERNAL="${INTERNAL:-1}" tools/run-qemu.sh
