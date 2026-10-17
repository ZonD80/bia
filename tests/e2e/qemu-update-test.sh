#!/bin/bash
# QEMU dual-disk harness for attended update / carry_rescue through the rescue image.
#
# Layout:
#   dist/bia.img       — flash stick  (fs LABEL=BIAROOT, serial=flash)
#   dist/internal.img  — internal disk (fs LABEL=BIADISK, serial=internal)
#
# Usage:
#   tests/e2e/qemu-update-test.sh prep     # labels, bump flash release, clear /bia-update
#   tests/e2e/qemu-update-test.sh flash    # boot flash (rescue / /update from stick)
#   tests/e2e/qemu-update-test.sh disk     # boot BIADISK (/update stages → flash)
#   tests/e2e/qemu-update-test.sh status   # show labels + releases (no boot)
#
# Env (passed through to run-qemu.sh):
#   ARCH=x86_64|arm64   default x86_64
#   MODE=gui|serial     default gui
#   NET=user|bridge     default user (no sudo)
#   MEM CPUS WEBPORT IMG INTERNAL_IMG
#   BUMP=1              prep: flash release = disk + 1 (default 1)
#   CLEAR=1             prep: wipe /bia-update on both (default 1)
set -euo pipefail
cd "$(dirname "$0")/../.."
ROOT=$(pwd)

cmd=${1:-}
case $cmd in
  prep|flash|disk|status) ;;
  *)
    cat <<'EOF' >&2
usage: tests/e2e/qemu-update-test.sh prep|flash|disk|status

  prep    label flash=BIAROOT, disk=BIADISK; bump flash release if needed;
          clear /bia-update (clean update/rescue run)
  flash   boot flash with internal disk attached (carry_rescue / /update)
  disk    boot BIADISK with flash attached (/update → need-apply → flash)
  status  print filesystem labels + etc/bia-release on both images

Test plan after prep (flash release = disk + 1):
  A) update the disk from the flash
       tests/e2e/qemu-update-test.sh disk   → /update (or ask the agent: carry_update)
       → fold into /bia-update, reboot into the rescue image, copy flash → disk,
         reboot into the disk, unfold
  B) rescue the disk from the flash
       tests/e2e/qemu-update-test.sh flash  → /rescue (carry_rescue)
Automated version of all of this: tests/e2e/update-e2e.sh (make update-e2e).
EOF
    exit 1
    ;;
esac

ARCH=${ARCH:-x86_64}
MODE=${MODE:-gui}
NET=${NET:-user}
IMG=${IMG:-dist/bia.img}
INTERNAL_IMG=${INTERNAL_IMG:-dist/internal.img}
BUMP=${BUMP:-1}
CLEAR=${CLEAR:-1}
BUILDER=bia-build:$ARCH

case $ARCH in
  arm64)  PLATFORM=linux/arm64 ;;
  x86_64) PLATFORM=linux/amd64 ;;
  *) echo "ARCH=$ARCH want arm64|x86_64" >&2; exit 1 ;;
esac

abs() {
  case $1 in
    /*) printf '%s\n' "$1" ;;
    *) printf '%s\n' "$ROOT/$1" ;;
  esac
}
FLASH=$(abs "$IMG")
DISK=$(abs "$INTERNAL_IMG")

need_images() {
  [ -f "$FLASH" ] || { echo "missing flash image: $FLASH (make image ARCH=$ARCH)" >&2; exit 1; }
  [ -f "$DISK" ] || { echo "missing internal image: $DISK" >&2; exit 1; }
}

need_builder() {
  if ! docker image inspect "$BUILDER" >/dev/null 2>&1; then
    echo "missing $BUILDER — run: make builder ARCH=$ARCH" >&2
    exit 1
  fi
}

# $1 = status | prep
run_image_tool() {
  local mode=$1
  docker run --rm --privileged --platform "$PLATFORM" \
    -e MODE="$mode" \
    -e BUMP="$BUMP" \
    -e CLEAR="$CLEAR" \
    -v "$FLASH":/flash:rw \
    -v "$DISK":/disk:rw \
    "$BUILDER" python3 - <<'PY'
import os, struct, subprocess, sys

MODE = os.environ["MODE"]
BUMP = os.environ.get("BUMP", "1") == "1"
CLEAR = os.environ.get("CLEAR", "1") == "1"

def partitions(path):
    with open(path, "rb") as f:
        f.seek(512)
        h = f.read(92)
        if h[0:8] != b"EFI PART":
            return []
        part_lba = struct.unpack_from("<Q", h, 72)[0]
        num = struct.unpack_from("<I", h, 80)[0]
        esz = struct.unpack_from("<I", h, 84)[0]
        f.seek(part_lba * 512)
        out = []
        for _ in range(num):
            e = f.read(esz)
            if e[0:16] == b"\x00" * 16:
                continue
            first, last = struct.unpack_from("<QQ", e, 32)
            name = e[56 : 56 + 72].decode("utf-16le", "replace").split("\x00")[0]
            out.append((name, first * 512, (last - first + 1) * 512))
        return out

def blkid(dev, key):
    r = subprocess.run(
        ["blkid", "-c", "/dev/null", "-o", "value", "-s", key, dev],
        capture_output=True, text=True,
    )
    return (r.stdout or "").strip()

def read_release(mnt):
    p = os.path.join(mnt, "etc/bia-release")
    if not os.path.isfile(p):
        return ""
    return "".join(c for c in open(p).read() if c.isdigit())

def find_root(img):
    """Largest ext4 slice that is not an ESP-named partition. Leaves winner looped."""
    cands = []
    for name, off, size in partitions(img):
        if "BOOT" in (name or "").upper():
            continue
        loop = subprocess.check_output(
            ["losetup", "-f", "--show", "-o", str(off), "--sizelimit", str(size), img],
            text=True,
        ).strip()
        try:
            if blkid(loop, "TYPE") not in ("ext4", "ext3", "ext2"):
                subprocess.run(["losetup", "-d", loop], check=False)
                continue
            cands.append((size, name or "(unnamed)", off, size, loop))
        except Exception:
            subprocess.run(["losetup", "-d", loop], check=False)
    if not cands:
        return None
    cands.sort()  # largest last
    winner = cands[-1]
    for c in cands[:-1]:
        subprocess.run(["losetup", "-d", c[4]], check=False)
    return {"gpt": winner[1], "off": winner[2], "size": winner[3], "loop": winner[4]}

def with_root(img, rw, fn):
    info = find_root(img)
    if not info:
        print(f"  (no ext4 root in {img})")
        return None
    mnt = "/mnt/bia-ut"
    os.makedirs(mnt, exist_ok=True)
    opts = "rw" if rw else "ro"
    try:
        st = subprocess.run(["mount", "-o", opts, info["loop"], mnt], capture_output=True, text=True)
        if st.returncode != 0:
            print(f"  mount failed: {st.stderr.strip()}", file=sys.stderr)
            return None
        try:
            return fn(info, mnt)
        finally:
            subprocess.run(["umount", mnt], check=False)
            subprocess.run(["umount", "-l", mnt], check=False)
    finally:
        subprocess.run(["losetup", "-d", info["loop"]], check=False)

def describe(tag, img):
    print(f"== {tag}  {img}")
    def show(info, mnt):
        label = blkid(info["loop"], "LABEL")
        # Remount path: loop still set while mounted; blkid on loop works.
        rel = read_release(mnt)
        agent = os.path.isfile(f"{mnt}/home/bia/bia.toml") or os.path.isdir(f"{mnt}/home/bia/files")
        upd = os.path.isdir(f"{mnt}/bia-update")
        print(
            f"  gpt={info['gpt']} fs_label={label or '(none)'} "
            f"release={rel or '(none)'} agent={'yes' if agent else 'no'} "
            f"bia-update={'yes' if upd else 'no'}"
        )
        return rel
    return with_root(img, False, show)

def prep_one(tag, img, want_label, set_release=None):
    print(f"== prep {tag} → LABEL={want_label}")
    def edit(info, mnt):
        subprocess.check_call(["e2label", info["loop"], want_label])
        rel = read_release(mnt)
        if set_release is not None:
            os.makedirs(f"{mnt}/etc", exist_ok=True)
            open(f"{mnt}/etc/bia-release", "w").write(str(set_release) + "\n")
            rel = str(set_release)
            print(f"  release → {rel}")
        if CLEAR:
            if os.path.isdir(f"{mnt}/bia-update"):
                subprocess.run(["rm", "-rf", f"{mnt}/bia-update"], check=False)
                print("  cleared /bia-update")
        agent = os.path.isfile(f"{mnt}/home/bia/bia.toml") or os.path.isdir(f"{mnt}/home/bia/files")
        print(
            f"  gpt={info['gpt']} fs_label={want_label} "
            f"release={rel or '(none)'} agent={'yes' if agent else 'no'}"
        )
        if tag == "disk" and not agent:
            print(
                "  WARNING: no agent on disk; run carry_to_disk from flash first "
                "if you need a real fold.",
                file=sys.stderr,
            )
        return int(rel) if rel.isdigit() else 0
    return with_root(img, True, edit)

if MODE == "status":
    describe("flash", "/flash")
    describe("disk", "/disk")
    sys.exit(0)

disk_rel = prep_one("disk", "/disk", "BIADISK")
if disk_rel is None:
    sys.exit(1)
flash_rel = None
if BUMP:
    flash_rel = disk_rel + 1
prep_one("flash", "/flash", "BIAROOT", set_release=flash_rel)
print("== prep done")
PY
}

do_status() {
  need_images
  need_builder
  run_image_tool status
}

do_prep() {
  need_images
  need_builder
  if pgrep -q qemu-system 2>/dev/null; then
    echo "QEMU is running; stop it before prep" >&2
    exit 1
  fi
  run_image_tool prep
  echo
  echo "Boot flash to rescue:  tests/e2e/qemu-update-test.sh flash"
  echo "Boot disk to update:   tests/e2e/qemu-update-test.sh disk"
}

do_boot() {
  local boot=$1
  need_images
  echo "== update-test boot=$boot ARCH=$ARCH NET=$NET MODE=$MODE"
  echo "   flash=$FLASH"
  echo "   disk =$DISK"
  echo
  if [ "$boot" = flash ]; then
    echo "In the guest: carry_rescue()  or  /update"
  else
    echo "In the guest: /update"
    echo "Then reboot cycle: flash (apply) → disk (unfold)"
  fi
  echo
  exec env \
    ARCH="$ARCH" \
    MODE="$MODE" \
    NET="$NET" \
    IMG="$FLASH" \
    INTERNAL=1 \
    INTERNAL_IMG="$DISK" \
    BOOT="$boot" \
    MEM="${MEM:-8G}" \
    CPUS="${CPUS:-6}" \
    WEBPORT="${WEBPORT:-8080}" \
    FRESH=0 \
    "$ROOT/tools/run-qemu.sh"
}

case $cmd in
  status) do_status ;;
  prep) do_prep ;;
  flash) do_boot flash ;;
  disk) do_boot disk ;;
esac
