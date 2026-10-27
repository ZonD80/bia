#!/bin/bash
# End-to-end QEMU test of taking over a running Linux (tools/takeover.sh → bia-deploy takeover).
#
#   tests/e2e/takeover-e2e.sh images  local "release" in dist/te-<arch>/www: bia-<arch>.img.xz
#                                     (work tree overlay, agent, MCPs), the Linux bia-deploy
#                                     tarball, bia-os.json with both; the Ubuntu cloud image
#   tests/e2e/takeover-e2e.sh run     boot Ubuntu (UEFI, no display device) with cloud-init from
#                                     www/; it runs takeover.sh unattended with a config (./fmm.json
#                                     keys, setup instructions). The driver downloads root.p12 from
#                                     the gate page, lets it reboot, watches the rescue write bia,
#                                     then checks bia over mTLS with that certificate
#   tests/e2e/takeover-e2e.sh all     images + run
#
# LAND=1: the takeover ends in land mode (--land with a land secret); a second VM (fly.img,
# bia with ./fmm.json) is asked to fly its agent there through the host's port forward with
# that secret; the server unfolds the agent and boots it (images: also builds fly.img).
# LAND=agent: no takeover by the test: the flyer's agent gets ssh access to Ubuntu and is
# asked in plain words to prepare it as a land and fly there (takeover.sh over ssh itself).
# DEPLOY=1: this computer's `bia-deploy server` takes Ubuntu over through ssh (password login
# as ubuntu, sudo with that password) instead of cloud-init running takeover.sh.
# NET=onlink: Ubuntu runs on a static /32 with an on-link gateway before the takeover.
# FIRMWARE=bios (ARCH=x86_64): SeaBIOS, the takeover goes through GRUB custom.cfg + grub-reboot.
#
# Env: ARCH (arm64), MEM (6G), CPUS (4), XZ_LEVEL (1), DISK_GB (16), TIMEOUT (minutes, 40).
set -euo pipefail
SELF=$(cd "$(dirname "$0")" && pwd)/$(basename "$0")
cd "$(dirname "$SELF")/../.."
ROOT=$(pwd)
ARCH=${ARCH:-arm64}
TE=$ROOT/dist/te-$ARCH
case $ARCH in
  arm64) PLATFORM=linux/arm64; UBUNTU=noble-server-cloudimg-arm64.img ;;
  x86_64) PLATFORM=linux/amd64; UBUNTU=noble-server-cloudimg-amd64.img ;;
  *) echo "ARCH=$ARCH?"; exit 2 ;;
esac
mkdir -p "$TE/cache" "$TE/www/rel"

do_images() {
  [ -f fmm.json ] || { echo "need ./fmm.json (bia runs on its FMM keys)"; exit 1; }
  [ -f "dist/$ARCH/root.img" ] || { echo "no dist/$ARCH/root.img (make ARCH=$ARCH build)"; exit 1; }
  echo "== root.img with the work tree's overlay scripts, agent and MCPs"
  cp "dist/$ARCH/root.img" "$TE/root.img"
  docker run --rm -i --privileged --platform "$PLATFORM" -e ARCH="$ARCH" -e MODE=dd \
    -v "$TE":/ie:rw -v "$ROOT":/work:ro "bia-build:$ARCH" python3 /work/tests/e2e/img-e2e-img.py patch
  echo "== rescue image from the work tree (rescue/takeover changes)"
  docker run --rm -i --privileged --platform "$PLATFORM" -v "$TE":/ie:rw -v "$ROOT":/work:ro "bia-build:$ARCH" bash -c '
    set -e; mkdir -p /mnt/r; mount -o loop /ie/root.img /mnt/r
    trap "umount /mnt/r" EXIT
    bash /work/builders/rescue.sh /mnt/r /mnt/r/usr/local/share/bia/rescue.cpio.gz'
  tools/esp-kernel.sh "$TE/root.img" "dist/$ARCH"
  cargo build --release -q -p bia-deploy
  local img=$TE/www/rel/bia-$ARCH.img
  rm -f "$img" "$img.xz"
  target/release/bia-deploy image --root "$TE/root.img" --esp source/flash --kern "dist/$ARCH" --arch "$ARCH" --out "$img"
  echo "== xz -${XZ_LEVEL:-1} -T0"
  xz -"${XZ_LEVEL:-1}" -T0 -k -f "$img"
  echo "== Linux bia-deploy ($ARCH)"
  docker run --rm --platform "$PLATFORM" -v "$ROOT":/work -v "bia-cache-$ARCH":/cache -w /work \
    -e CARGO_TARGET_DIR=/cache/flash-target "bia-build:$ARCH" \
    bash -lc 'export CARGO_HOME=${CARGO_HOME:-/cache/cargo}; cargo build --release -q -p bia-deploy && mkdir -p /work/dist/te-'"$ARCH"'/tool/bia-deploy && cp /cache/flash-target/release/bia-deploy /work/dist/te-'"$ARCH"'/tool/bia-deploy/'
  tar -C "$TE/tool" -czf "$TE/www/rel/bia-deploy-linux-$ARCH.tar.gz" bia-deploy
  python3 - "$TE/www/rel" "$ARCH" <<'PY'
import hashlib, json, os, sys
d, arch = sys.argv[1:]
def sha(p):
    h = hashlib.sha256()
    with open(p, "rb") as f:
        while c := f.read(16 << 20):
            h.update(c)
    return h.hexdigest()
img, xz, tool = f"{d}/bia-{arch}.img", f"{d}/bia-{arch}.img.xz", f"{d}/bia-deploy-linux-{arch}.tar.gz"
json.dump({"version": "1.1.4", "tag": "e2e", "os": {arch: {"image": {
    "file": os.path.basename(xz), "size": os.path.getsize(xz), "sha256": sha(xz),
    "raw_size": os.path.getsize(img), "raw_sha256": sha(img)}}},
    "tools": {os.path.basename(tool): {"file": os.path.basename(tool), "size": os.path.getsize(tool), "sha256": sha(tool)}}},
    open(f"{d}/bia-os.json", "w"), indent=2)
PY
  rm -f "$img"
  cp tools/takeover.sh "$TE/www/takeover.sh"
  if [ ! -f "$TE/cache/$UBUNTU" ]; then
    echo "== Ubuntu cloud image"
    curl -fL -o "$TE/cache/$UBUNTU.part" "https://cloud-images.ubuntu.com/noble/current/$UBUNTU"
    mv "$TE/cache/$UBUNTU.part" "$TE/cache/$UBUNTU"
  fi
  if [ -n "${LAND:-}" ]; then
    echo "== fly.img (bia with ./fmm.json, flies to the taken-over server)"
    ARCH=$ARCH IMG="$TE/fly.img" IMAGE_MB="${IMAGE_MB:-12288}" tools/mkimage.sh
    head -c 8 /dev/urandom | od -An -tx1 | tr -d ' \n' > "$TE/token"
    rm -f "$TE/fly_key" "$TE/fly_key.pub"
    ssh-keygen -q -t ed25519 -N "" -C bia-fly-e2e -f "$TE/fly_key"
    docker run --rm -i --privileged --platform "$PLATFORM" -e ARCH="$ARCH" \
      -v "$TE":/ut:rw -v "$ROOT":/work:ro "bia-build:$ARCH" python3 /work/tests/e2e/takeover-e2e-img.py prep-fly "$(cat "$TE/token")"
  fi
  ls -l "$TE/www/rel"
}

do_run() {
  pgrep -q qemu-system 2>/dev/null && { echo "QEMU is running; stop it first" >&2; exit 1; }
  [ -z "${DEPLOY:-}" ] || cargo build --release -q -p bia-deploy
  rm -f "$TE/disk.qcow2"
  qemu-img create -q -f qcow2 -F qcow2 -b "$TE/cache/$UBUNTU" "$TE/disk.qcow2" "${DISK_GB:-16}G"
  ARCH=$ARCH TE=$TE LE=$TE UBUNTU=$UBUNTU LAND=${LAND:-} DEPLOY=${DEPLOY:-} NET=${NET:-dhcp} FIRMWARE=${FIRMWARE:-uefi} python3 tests/e2e/takeover-e2e-drive.py
}

case ${1:-all} in
  images) do_images ;;
  run) do_run ;;
  all) do_images; do_run ;;
  *) echo "usage: $0 images|run|all"; exit 2 ;;
esac
