#!/bin/bash
# Grow the ext4 filesystem at BYTE_OFFSET inside a raw image up to BYTE_LENGTH.
# The partition is larger than the filesystem that was copied into it.
set -euo pipefail
img=${1:-}
off=${2:-}
len=${3:-}
arch=${ARCH:-arm64}
if [ -z "$img" ] || [ -z "$off" ] || [ -z "$len" ]; then
  echo "usage: grow-ext4.sh IMAGE BYTE_OFFSET BYTE_LENGTH" >&2
  exit 2
fi
img=$(cd "$(dirname "$img")" && pwd)/$(basename "$img")
[ -f "$img" ] || { echo "no $img"; exit 1; }
echo "== growing ext4 at offset $off to $((len / 1024 / 1024)) MiB"
docker run --rm --privileged -i -v "$img":/img:rw "bia-build:$arch" sh -s -- "$off" "$len" <<'EOF'
set -eu
off=$1
len=$2
# bia-build is Debian: e2fsprogs is already in the image (no apk).
command -v resize2fs >/dev/null && command -v losetup >/dev/null \
  || { echo "resize2fs/losetup missing in bia-build — rebuild the builder image"; exit 1; }
losetup -j /img -n -O NAME 2>/dev/null | while read -r old; do
  [ -n "$old" ] && losetup -d "$old" || true
done
loop=$(losetup -f --show -o "$off" --sizelimit "$len" /img)
trap 'losetup -d "$loop"' EXIT
got=$(blockdev --getsize64 "$loop")
if [ "$got" -lt "$len" ]; then
  echo "loop $loop is $got bytes, wanted $len" >&2
  exit 1
fi
set +e
e2fsck -fy "$loop"
rc=$?
set -e
[ "$rc" -le 2 ]
resize2fs "$loop"
EOF
