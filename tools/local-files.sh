#!/bin/bash
# Build dist/<arch>/bia-<arch>.img (tools/mkimg.sh) and write dist/<arch>/local-files.json
# naming it, for the bia-flash GUI ("Advanced: local image files" → Choose…). Runs on the
# host after the build, so the path is this machine's, not the build container's /work.
set -euo pipefail
cd "$(dirname "$0")/.."
ARCH=${1:-${ARCH:-x86_64}}
DIST=$(pwd)/dist/$ARCH
OUT=$DIST/bia-$ARCH.img tools/mkimg.sh "$ARCH"
json() { printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g'; }
OUT=$DIST/local-files.json
cat > "$OUT.tmp" <<JSON
{
  "arch": "$ARCH",
  "image": "$(json "$DIST/bia-$ARCH.img")",
  "built": "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
}
JSON
mv "$OUT.tmp" "$OUT"
echo "== $OUT"
