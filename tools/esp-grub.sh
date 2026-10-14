#!/bin/bash
# Host/build wrapper: same implementation as the guest tool.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
exec "$ROOT/source/os/overlay/usr/local/sbin/bia-esp-grub" "$@"
