#!/bin/bash
# Container test of bia-update's internet path (no QEMU): a small fake release
# (GPT image with an ESP and BIAROOT, xz, bia-os.json) is "downloaded" from a
# file:// directory, checked and unpacked into /bia-update/net, like a machine
# with no flash stick attached would. Also checks the version rules.
#
#   tests/integration/update-net-test.sh            (runs itself in bia-build:$ARCH, privileged)
set -uo pipefail
ARCH=${ARCH:-arm64}
case $ARCH in arm64) PLATFORM=linux/arm64 ;; x86_64) PLATFORM=linux/amd64 ;; *) exit 2 ;; esac
if [ "${IN_CONTAINER:-}" != 1 ]; then
  cd "$(dirname "$0")/../.."
  # tmpfs: the test must not depend on free space in the Docker VM
  exec docker run --rm --privileged --platform "$PLATFORM" -e IN_CONTAINER=1 -e ARCH="$ARCH" \
    --tmpfs /bia-update:size=3g --tmpfs /tmp:size=2g \
    -v "$PWD":/work:ro -w /work "bia-build:$ARCH" bash /work/tests/integration/update-net-test.sh
fi

FAILED=0
check() { if eval "$2"; then echo "ok   $1"; else echo "FAIL $1"; FAILED=1; fi; }

# ---- a fake release ---------------------------------------------------------------
R=/tmp/release
rm -rf "$R" /bia-update/*
mkdir -p "$R"
IMG=/tmp/bia-test.img
truncate -s 320M "$IMG"
# GPT: ESP (64 MiB) + BIAROOT (no sfdisk/parted in the builder)
python3 - "$IMG" <<'PY'
import struct, sys, uuid, zlib
path = sys.argv[1]
size = __import__("os").path.getsize(path)
sectors = size // 512
parts = [(2048, 2048 + 64 * 2048 - 1, "C12A7328-F81F-11D2-BA4B-00A0C93EC93B", "ESP"),
         (2048 + 64 * 2048, sectors - 34, "0FC63DAF-8483-4772-8E79-3D69D8477DE4", "BIAROOT")]
ents = b""
for first, last, t, name in parts:
    ents += uuid.UUID(t).bytes_le + uuid.uuid4().bytes_le + struct.pack("<QQQ", first, last, 0) + name.encode("utf-16-le").ljust(72, b"\0")
ents = ents.ljust(128 * 128, b"\0")
def header(cur, alt, ent_lba):
    h = struct.pack("<8sIIIIQQQQ16sQIII", b"EFI PART", 0x10000, 92, 0, 0, cur, alt, 34, sectors - 34,
                    uuid.UUID(int=0x1234).bytes_le, ent_lba, 128, 128, zlib.crc32(ents))
    return h[:16] + struct.pack("<I", zlib.crc32(h)) + h[20:]
mbr = bytearray(512)
mbr[446:462] = struct.pack("<BBBBBBBBII", 0, 0, 2, 0, 0xEE, 0xFF, 0xFF, 0xFF, 1, min(sectors - 1, 0xFFFFFFFF))
mbr[510:512] = b"\x55\xaa"
with open(path, "r+b") as f:
    f.write(mbr); f.write(header(1, sectors - 1, 2).ljust(512, b"\0")); f.write(ents)
    f.seek((sectors - 33) * 512); f.write(ents); f.write(header(sectors - 1, 1, sectors - 33).ljust(512, b"\0"))
PY
# partitions on their own loop devices (containers have no udev for loopNpM nodes)
L1=$(losetup --show -f -o $((2048 * 512)) --sizelimit $((64 << 20)) "$IMG")
L2=$(losetup --show -f -o $(((2048 + 64 * 2048) * 512)) --sizelimit $(( $(stat -c %s "$IMG") - (2048 + 64 * 2048 + 34) * 512 )) "$IMG")
mkfs.vfat -n BIAESP "$L1" >/dev/null
mkfs.ext4 -q -L BIAROOT "$L2"
mkdir -p /tmp/m-esp /tmp/m-root
mount "$L1" /tmp/m-esp && mount "$L2" /tmp/m-root
echo kernel > /tmp/m-esp/vmlinuz-lts
echo initrd > /tmp/m-esp/initramfs-lts
mkdir -p /tmp/m-esp/EFI/BOOT && echo loader > /tmp/m-esp/EFI/BOOT/BOOTAA64.EFI
mkdir -p /tmp/m-root/etc /tmp/m-root/var/lib/dpkg /tmp/m-root/home/bia/MCPs/web /tmp/m-root/usr/bin
echo 20991231000000 > /tmp/m-root/etc/bia-release
echo 9.9.9 > /tmp/m-root/etc/bia-version
cp /var/lib/dpkg/status /tmp/m-root/var/lib/dpkg/status   # same architecture as this container
echo hello > /tmp/m-root/usr/bin/new-tool
ln /tmp/m-root/usr/bin/new-tool /tmp/m-root/usr/bin/new-tool-hardlink
umount /tmp/m-esp /tmp/m-root
losetup -d "$L1" "$L2"
xz -T0 -c "$IMG" > "$R/bia-$ARCH.img.xz"
python3 - "$R" "$ARCH" "$IMG" <<'PY'
import hashlib, json, os, sys
r, arch, img = sys.argv[1:]
sha = lambda p: hashlib.sha256(open(p, "rb").read()).hexdigest()
xz = f"{r}/bia-{arch}.img.xz"
json.dump({"version": "9.9.9", "tag": "v9.9.9", "os": {arch: {"image": {
    "file": os.path.basename(xz), "size": os.path.getsize(xz), "sha256": sha(xz),
    "raw_size": os.path.getsize(img), "raw_sha256": sha(img)}}}}, open(f"{r}/bia-os.json", "w"))
PY

# ---- bia-update's functions --------------------------------------------------------
export BIA_UPDATE_LIB=1 BIA_RELEASE_URL="file://$R"
# shellcheck disable=SC1091
source /work/source/os/overlay/usr/local/sbin/bia-update
set +e

check "9.9.9 is newer than 1.1.1" 'version_newer 9.9.9 1.1.1'
check "1.10.0 is newer than 1.9.9" 'version_newer 1.10.0 1.9.9'
check "1.1.1 is not newer than 1.1.1" '! version_newer 1.1.1 1.1.1'
check "a root without a version file is 0" '[ "$(ver_of /nonexistent)" = 0 ] && version_newer 1.0.0 0'

out=$( (net_stage 1.1.1 0) 2>/tmp/stage.err ); rc=$?
check "internet stage succeeds ($rc)" '[ $rc -eq 0 ]'
check "it reports the release ($(echo "$out" | tail -n1))" '[ "$(echo "$out" | tail -n1)" = "9.9.9 v9.9.9" ]'
check "root tree unpacked with its version" '[ "$(cat /bia-update/net/root/etc/bia-version)" = 9.9.9 ]'
check "hard links kept" '[ "$(stat -c %h /bia-update/net/root/usr/bin/new-tool)" = 2 ]'
check "boot files unpacked" '[ -f /bia-update/net/esp/vmlinuz-lts ] && [ -f /bia-update/net/esp/EFI/BOOT/BOOTAA64.EFI ]'
check "image and download removed" '[ ! -e /bia-update/net/os.img ] && ! ls /bia-update/net/*.xz >/dev/null 2>&1'
check "no loop device left" '! losetup -a 2>/dev/null | grep -q "/bia-update/net/os.img"'
grep -q "download bia-$ARCH.img.xz" /tmp/stage.err; check "progress shown to the agent" '[ $? -eq 0 ]'

out=$( (net_stage 9.9.9 0) 2>/dev/null ); rc=$?
check "same version is refused ($rc)" '[ $rc -ne 0 ] && echo "$out" | grep -q "already runs 9.9.9"'
out=$( (net_stage 9.9.9 1) 2>/dev/null ); rc=$?
check "force installs the same version" '[ $rc -eq 0 ]'

# a corrupt download is refused and leaves nothing unpacked
rm -rf /bia-update/*
cp "$R/bia-$ARCH.img.xz" /tmp/good.xz
printf 'x' | dd of="$R/bia-$ARCH.img.xz" bs=1 seek=100 conv=notrunc status=none
out=$( (net_stage 1.1.1 0) 2>/dev/null ); rc=$?
check "a bad sha256 is refused" '[ $rc -ne 0 ] && echo "$out" | grep -q "does not match its sha256"'
check "nothing unpacked after a failure" '[ ! -d /bia-update/net/root ]'
cp /tmp/good.xz "$R/bia-$ARCH.img.xz"

# no internet: a clear refusal
out=$( (BIA_RELEASE_URL=file:///nonexistent; RELEASE_URL=$BIA_RELEASE_URL; net_stage 1.1.1 0) 2>/dev/null ); rc=$?
check "an unreachable release is refused" '[ $rc -ne 0 ] && echo "$out" | grep -q "cannot download bia-os.json"'

# the rescue marker carries the directory, not a UUID
check "DIR= sources pass through the marker" 'grep -q "case \$dev in DIR=\*)" /work/source/os/overlay/usr/local/sbin/bia-update'

echo "RESULT $([ $FAILED -eq 0 ] && echo PASSED || echo FAILED)"
exit $FAILED
