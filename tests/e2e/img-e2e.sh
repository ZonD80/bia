#!/bin/bash
# End-to-end QEMU test of the bia disk image (bia-<arch>.img.xz, what releases ship).
#
#   tests/e2e/img-e2e.sh images   bia-<arch>.img.xz from dist/<arch>/root.img (overlay scripts,
#                             agent, carry/system MCPs from the work tree), then the stick:
#                               MODE=flash  a local "GitHub release" (bia-os.json + .img.xz
#                                           over HTTP) written by bia-deploy onto the whole
#                                           stick with ./fmm.json and a Hugging Face model
#                               MODE=dd     the .img.xz decoded onto a bigger disk as Etcher,
#                                           Rufus or dd would, plus ./fmm.json copied onto
#                                           its ESP by hand (EFI/bia/fmm.json)
#   tests/e2e/img-e2e.sh run      boot the stick (FIRMWARE=uefi|bios): bia-grow grows BIAROOT over
#                             the disk (dd) or moves the model in (flash); agent comes up on
#                             the FMM keys; /poweroff
#   tests/e2e/img-e2e.sh check    inspect the stick: GPT over the whole disk, ext4 filling BIAROOT,
#                             keys seeded, model in /home/bia/model (flash), BIOS GRUB (x86_64)
#   tests/e2e/img-e2e.sh all      images + run + check
#
# Env: ARCH (arm64), MODE (flash), FIRMWARE (uefi; bios needs ARCH=x86_64), STICK_GB (8),
#      MODEL (bartowski/SmolLM2-135M-Instruct-GGUF:SmolLM2-135M-Instruct-Q4_K_M.gguf),
#      XZ_LEVEL (1; releases use 6), MEM (4G), CPUS (4), TIMEOUT (minutes per boot phase, 30)
# Files in dist/ie-<arch>/.
set -euo pipefail
SELF=$(cd "$(dirname "$0")" && pwd)/$(basename "$0")
cd "$(dirname "$SELF")/../.."
ROOT=$(pwd)
ARCH=${ARCH:-arm64}
MODE=${MODE:-flash}
FIRMWARE=${FIRMWARE:-uefi}
MODEL=${MODEL:-bartowski/SmolLM2-135M-Instruct-GGUF:SmolLM2-135M-Instruct-Q4_K_M.gguf}
IE=$ROOT/dist/ie-$ARCH
case $ARCH in arm64) PLATFORM=linux/arm64 ;; x86_64) PLATFORM=linux/amd64 ;; *) echo "ARCH=$ARCH?"; exit 2 ;; esac
case $MODE in flash|dd) ;; *) echo "MODE=$MODE? (flash or dd)"; exit 2 ;; esac
[ "$FIRMWARE" = uefi ] || [ "$FIRMWARE/$ARCH" = bios/x86_64 ] || { echo "FIRMWARE=$FIRMWARE needs ARCH=x86_64"; exit 2; }
mkdir -p "$IE"

image_tool() {
  docker run --rm -i --privileged --platform "$PLATFORM" -e ARCH="$ARCH" -e MODE="$MODE" \
    -v "$IE":/ie:rw -v "$ROOT":/work:ro "bia-build:$ARCH" python3 /work/tests/e2e/img-e2e-img.py "$@"
}

do_images() {
  pgrep -q qemu-system 2>/dev/null && { echo "QEMU is running; stop it first" >&2; exit 1; }
  [ -f fmm.json ] || { echo "need ./fmm.json (the agent runs on its FMM keys)"; exit 1; }
  [ -f "dist/$ARCH/root.img" ] || { echo "no dist/$ARCH/root.img (make ARCH=$ARCH build)"; exit 1; }
  echo "== root.img with the work tree's overlay scripts, agent and MCPs"
  cp "dist/$ARCH/root.img" "$IE/root.img"
  image_tool patch
  tools/esp-kernel.sh "$IE/root.img" "dist/$ARCH"
  cargo build --release -q -p bia-deploy
  local bf=target/release/bia-deploy img=$IE/rel/bia-$ARCH.img
  rm -rf "$IE/rel" "$IE/cache" "$IE/stick.img"
  mkdir -p "$IE/rel"
  "$bf" image --root "$IE/root.img" --esp source/flash --kern "dist/$ARCH" --arch "$ARCH" --out "$img"
  echo "== xz -${XZ_LEVEL:-1} -T0 (releases use -6)"
  xz -"${XZ_LEVEL:-1}" -T0 -k -f "$img"
  python3 - "$IE/rel" "$ARCH" <<'PY'
import hashlib, json, os, sys
d, arch = sys.argv[1:]
def sha(p):
    h = hashlib.sha256()
    with open(p, "rb") as f:
        while c := f.read(16 << 20):
            h.update(c)
    return h.hexdigest()
img, xz = f"{d}/bia-{arch}.img", f"{d}/bia-{arch}.img.xz"
json.dump({"version": "0.0.0-e2e", "tag": "e2e", "os": {arch: {"image": {
    "file": os.path.basename(xz), "size": os.path.getsize(xz), "sha256": sha(xz),
    "raw_size": os.path.getsize(img), "raw_sha256": sha(img)}}}}, open(f"{d}/bia-os.json", "w"))
PY
  ls -l "$IE/rel"
  local stick=$(( ${STICK_GB:-8} << 30 ))
  if [ "$MODE" = flash ]; then
    local port=$(( 20000 + RANDOM % 20000 ))
    python3 -m http.server "$port" --bind 127.0.0.1 --directory "$IE/rel" >"$IE/http.log" 2>&1 &
    local srv=$!
    trap "kill $srv 2>/dev/null || true" EXIT
    sleep 1
    echo "== bia-deploy write from the local release (keys + $MODEL)"
    BIA_RELEASE_URL=http://127.0.0.1:$port BIA_DEPLOY_CACHE=$IE/cache \
      "$bf" write --disk "$IE/stick.img" --image --size "$stick" --arch "$ARCH" \
      --fmm fmm.json --model "$MODEL" --yes
  else
    echo "== decoding the image onto a ${STICK_GB:-8} GiB disk, as an image writer does"
    xz -dc "$IE/rel/bia-$ARCH.img.xz" >"$IE/stick.img"
    truncate -s "$stick" "$IE/stick.img"
    image_tool keys
  fi
}

do_run() {
  ARCH=$ARCH LE=$IE MODE=$MODE FIRMWARE=$FIRMWARE MODEL=$MODEL MEM=${MEM:-4G} CPUS=${CPUS:-4} \
    TIMEOUT=${TIMEOUT:-30} python3 "$ROOT/tests/e2e/img-e2e-drive.py"
}

case ${1:-} in
  images) do_images ;;
  run) do_run ;;
  check) image_tool check "$MODEL" ;;
  all) do_images; do_run; image_tool check "$MODEL"; echo "======== image e2e passed ($ARCH, $MODE, $FIRMWARE)" ;;
  *) sed -n '2,25p' "$SELF" >&2; exit 2 ;;
esac
