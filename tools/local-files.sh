#!/bin/bash
# Write dist/<arch>/local-files.json: absolute paths of a local build's image files, for the
# bia-flash GUI ("Advanced: local image files" → Choose…). Runs on the host after the build,
# so the paths are this machine's, not the build container's /work.
set -euo pipefail
cd "$(dirname "$0")/.."
ARCH=${1:-${ARCH:-x86_64}}
FLASH=${FLASH:-source/flash}
DIST=$(pwd)/dist/$ARCH
ESP=$(cd "$FLASH" && pwd)
for f in "$DIST/root.img" "$DIST/vmlinuz-lts" "$DIST/initramfs-lts"; do
  [ -f "$f" ] || { echo "local-files: missing $f (make build ARCH=$ARCH)"; exit 1; }
done
[ -d "$ESP/EFI" ] || { echo "local-files: missing $ESP/EFI"; exit 1; }
json() { printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g'; }
OUT=$DIST/local-files.json
cat > "$OUT.tmp" <<JSON
{
  "arch": "$ARCH",
  "root": "$(json "$DIST/root.img")",
  "esp": "$(json "$ESP")",
  "kern": "$(json "$DIST")",
  "built": "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
}
JSON
mv "$OUT.tmp" "$OUT"
echo "== $OUT"
