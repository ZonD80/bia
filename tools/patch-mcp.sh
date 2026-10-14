#!/bin/bash
# Rebuild one or more MCP binaries and hot-patch them into ./source/flash plus the QEMU
# images (bia.img BIAROOT and internal.img), so you can reboot and test without a
# full rootfs/image rebuild.
#
# Usage:
#   tools/patch-mcp.sh chrome
#   tools/patch-mcp.sh secrets chrome
#   tools/patch-mcp.sh all
#   SKIP_BUILD=1 tools/patch-mcp.sh chrome   # patch existing dist/$ARCH binary
#
# Env:
#   ARCH=arm64|x86_64   (default arm64)
#   IMG=dist/bia.img
#   INTERNAL_IMG=dist/internal.img
#   SKIP_BUILD=1        skip cargo; use existing OUT binaries
#   SKIP_INTERNAL=1     do not patch internal.img
#   SKIP_FLASH_IMG=1    do not patch bia.img
set -euo pipefail
cd "$(dirname "$0")/.."

ARCH=${ARCH:-arm64}
case $ARCH in
  arm64)  RARCH=aarch64; PLATFORM=linux/arm64 ;;
  x86_64) RARCH=x86_64; PLATFORM=linux/amd64 ;;
  *) echo "unknown ARCH=$ARCH (want arm64|x86_64)" >&2; exit 1 ;;
esac

IMG=${IMG:-dist/bia.img}
INTERNAL_IMG=${INTERNAL_IMG:-dist/internal.img}
BUILDER="bia-build:$ARCH"
NTGCALLS_VER=${NTGCALLS_VER:-v3.0.0}

ALL_MCPS=(system telegram whatsapp secrets chrome screen carry)

usage() {
  echo "usage: tools/patch-mcp.sh <mcp…|all>" >&2
  echo "  mcps: ${ALL_MCPS[*]}" >&2
  exit 1
}

[ $# -ge 1 ] || usage

MCPS=()
for arg in "$@"; do
  case $arg in
    all)
      MCPS=("${ALL_MCPS[@]}")
      ;;
    system|telegram|whatsapp|secrets|chrome|screen|carry)
      MCPS+=("$arg")
      ;;
    *)
      echo "unknown mcp: $arg" >&2
      usage
      ;;
  esac
done

# De-dupe while preserving order.
DEDUPED=()
for m in "${MCPS[@]}"; do
  skip=
  for d in "${DEDUPED[@]+"${DEDUPED[@]}"}"; do
    [ "$d" = "$m" ] && skip=1 && break
  done
  [ -n "$skip" ] || DEDUPED+=("$m")
done
MCPS=("${DEDUPED[@]}")

if pgrep -q qemu-system 2>/dev/null; then
  echo "QEMU is running; stop it before patching disk images." >&2
  exit 1
fi

if ! docker image inspect "$BUILDER" >/dev/null 2>&1; then
  echo "missing docker image $BUILDER — run: make builder ARCH=$ARCH" >&2
  exit 1
fi

if [ "${SKIP_BUILD:-}" != 1 ]; then
  echo "== building MCP crates ($ARCH): ${MCPS[*]}"
  docker run --rm --privileged --platform "$PLATFORM" \
    -v "$PWD":/work -v "bia-cache-$ARCH":/cache "$BUILDER" \
    /work/builders/build.sh "$ARCH" rust "${MCPS[@]}"
else
  echo "== SKIP_BUILD=1 — using existing binaries under dist/$ARCH"
fi

NEED_TELEGRAM=
NEED_CHROME=
for m in "${MCPS[@]}"; do
  OUT="dist/$ARCH/bia-mcp-$m"
  [ -x "$OUT" ] || { echo "no binary at $OUT" >&2; exit 1; }
  FLASH_BIN="source/flash/MCPs/$m/bin/$RARCH/bia-mcp-$m"
  echo "== source/flash/MCPs/$m <- $OUT"
  mkdir -p "$(dirname "$FLASH_BIN")"
  cp -f "$OUT" "$FLASH_BIN"
  chmod 755 "$FLASH_BIN"
  ls -lh "$FLASH_BIN"
  [ "$m" = telegram ] && NEED_TELEGRAM=1
  [ "$m" = chrome ] && NEED_CHROME=1
done

if [ -n "$NEED_TELEGRAM" ]; then
  NTG_SO_FLASH="source/flash/MCPs/telegram/lib/libntgcalls.so"
  # The build cache volume holds the shared lib fetched by builders/build.sh.
  NTG_SO_SRC=$(docker run --rm -v "bia-cache-$ARCH":/cache "$BUILDER" \
    sh -c "
      if [ -f /cache/ntgcalls-dylib-$ARCH-$NTGCALLS_VER/libntgcalls.so ]; then
        echo /cache/ntgcalls-dylib-$ARCH-$NTGCALLS_VER/libntgcalls.so
      elif [ -f /cache/ntgcalls-dylib-$ARCH/libntgcalls.so ]; then
        echo /cache/ntgcalls-dylib-$ARCH/libntgcalls.so
      fi
    " 2>/dev/null || true)
  NTG_SO_SRC=$(printf '%s' "$NTG_SO_SRC" | tr -d '\r' | head -n1)
  if [ -n "$NTG_SO_SRC" ]; then
    echo "== flash libntgcalls.so <- $NTG_SO_SRC ($NTGCALLS_VER)"
    mkdir -p "$(dirname "$NTG_SO_FLASH")"
    docker run --rm -v "$PWD":/work -v "bia-cache-$ARCH":/cache "$BUILDER" \
      cp -f "$NTG_SO_SRC" "/work/$NTG_SO_FLASH"
    ls -lh "$NTG_SO_FLASH"
  else
    echo "WARNING: libntgcalls.so not found in cache — guest may keep an old .so" >&2
  fi
fi

if [ -n "$NEED_CHROME" ]; then
  # Force the guest to re-pack the CRX after extension/ source changes.
  rm -f source/flash/MCPs/chrome/extension.packstamp
  echo "== vendor node + chrome-devtools-mcp"
  tools/fetch-chrome-devtools.sh
fi

export PATCH_MCPS="${MCPS[*]}"
export PATCH_RARCH="$RARCH"

patch_image() {
  local img="$1" label="$2"
  [ -f "$img" ] || { echo "skip $label: no $img"; return 0; }
  echo "== patching $label ($img) [${PATCH_MCPS}]"
  local abs
  abs=$(cd "$(dirname "$img")" && pwd)/$(basename "$img")
  docker run --rm --privileged -i --platform "$PLATFORM" \
    -v "$PWD":/work \
    -v "$abs":/img:rw \
    -e PATCH_MCPS="$PATCH_MCPS" \
    -e RARCH="$PATCH_RARCH" \
    "$BUILDER" python3 - <<'PY'
import os, shutil, struct, subprocess, sys

def partitions(path):
    with open(path, "rb") as f:
        f.seek(512)
        h = f.read(92)
        if h[:8] != b"EFI PART":
            raise SystemExit(f"no GPT at LBA1 in {path}")
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
            name = e[56:56 + 72].decode("utf-16le", "replace").split("\x00")[0]
            out.append((name, first * 512, (last - first + 1) * 512))
        return out

def copy_file(src, dest, mode=None):
    os.makedirs(os.path.dirname(dest), exist_ok=True)
    subprocess.check_call(["cp", "-f", src, dest])
    if mode is not None:
        os.chmod(dest, mode)
    elif os.access(src, os.X_OK):
        os.chmod(dest, 0o755)
    st = os.stat(dest)
    print(f"   wrote {dest} ({st.st_size} bytes)")

rarch = os.environ["RARCH"]
mcps = os.environ["PATCH_MCPS"].split()
written = 0
os.makedirs("/mnt/bia-patch", exist_ok=True)
for name, off, size in partitions("/img"):
    if name in ("BIABOOT",) or name.upper().endswith("BOOT"):
        continue
    loop = subprocess.check_output(
        ["losetup", "-f", "--show", "-o", str(off), "--sizelimit", str(size), "/img"],
        text=True,
    ).strip()
    try:
        r = subprocess.run(
            ["mount", "-o", "rw", loop, "/mnt/bia-patch"],
            capture_output=True,
            text=True,
        )
        if r.returncode != 0:
            continue
        try:
            root = "/mnt/bia-patch/home/bia/MCPs"
            if not os.path.isdir(root):
                continue
            for mcp in mcps:
                mcp_root = os.path.join(root, mcp)
                if not os.path.isdir(mcp_root):
                    print(f"   skip {mcp}: no {mcp_root}", file=sys.stderr)
                    continue
                bin_name = f"bia-mcp-{mcp}"
                src = f"/work/source/flash/MCPs/{mcp}/bin/{rarch}/{bin_name}"
                if not os.path.isfile(src):
                    raise SystemExit(f"missing {src}")
                dest_dir = os.path.join(mcp_root, "bin", rarch)
                os.makedirs(dest_dir, exist_ok=True)
                dest = os.path.join(dest_dir, bin_name)
                copy_file(src, dest, mode=0o755)
                written += 1
                print(f"   part={name!r} mcp={mcp}")

                mcp_json = f"/work/source/flash/MCPs/{mcp}/mcp.json"
                if os.path.isfile(mcp_json):
                    copy_file(mcp_json, os.path.join(mcp_root, "mcp.json"))

                if mcp == "telegram":
                    ntg_src = "/work/source/flash/MCPs/telegram/lib/libntgcalls.so"
                    if os.path.isfile(ntg_src):
                        for lib_dest in (
                            "/mnt/bia-patch/usr/local/lib/bia/speech/lib/libntgcalls.so",
                            os.path.join(mcp_root, "lib", "libntgcalls.so"),
                        ):
                            os.makedirs(os.path.dirname(lib_dest), exist_ok=True)
                            subprocess.check_call(["cp", "-f", ntg_src, lib_dest])
                            print(f"   wrote {lib_dest}")

                if mcp == "chrome":
                    src_ext = "/work/source/flash/MCPs/chrome/extension"
                    dst_ext = os.path.join(mcp_root, "extension")
                    if os.path.isdir(src_ext):
                        if os.path.isdir(dst_ext):
                            shutil.rmtree(dst_ext)
                        shutil.copytree(src_ext, dst_ext)
                        print(f"   wrote {dst_ext}/")
                    for tree in ("runtime", "devtools"):
                        src_t = f"/work/source/flash/MCPs/chrome/{tree}"
                        dst_t = os.path.join(mcp_root, tree)
                        if os.path.isdir(src_t):
                            if os.path.isdir(dst_t):
                                shutil.rmtree(dst_t)
                            shutil.copytree(src_t, dst_t, symlinks=True)
                            print(f"   wrote {dst_t}/")
                        else:
                            print(f"   WARNING: missing {src_t}", file=sys.stderr)
                    for extra in ("extension.pem", "extension.crx"):
                        src_e = f"/work/source/flash/MCPs/chrome/{extra}"
                        if os.path.isfile(src_e):
                            copy_file(src_e, os.path.join(mcp_root, extra))
                    stamp = os.path.join(mcp_root, "extension.packstamp")
                    if os.path.isfile(stamp):
                        os.remove(stamp)
                        print(f"   removed {stamp}")
        finally:
            subprocess.run(["umount", "/mnt/bia-patch"], check=False)
            subprocess.run(["umount", "-l", "/mnt/bia-patch"], check=False)
    finally:
        subprocess.run(["losetup", "-d", loop], check=False)

if written == 0:
    print("   WARNING: no /home/bia/MCPs/<name> on this image", file=sys.stderr)
    sys.exit(2)
print(f"   ok ({written} binary copy)")
PY
}

if [ "${SKIP_FLASH_IMG:-}" != 1 ]; then
  patch_image "$IMG" bia.img
fi
if [ "${SKIP_INTERNAL:-}" != 1 ]; then
  patch_image "$INTERNAL_IMG" internal.img
fi

echo "== done. Reboot the guest (or make run) to load: ${MCPS[*]}"
for m in "${MCPS[@]}"; do
  b="source/flash/MCPs/$m/bin/$RARCH/bia-mcp-$m"
  echo "   $b ($(stat -f%z "$b" 2>/dev/null || stat -c%s "$b") bytes)"
done
