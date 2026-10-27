#!/usr/bin/env bash
# Release bia on GitHub (zond80/bia): check what the release for this version already
# has, build only the missing packages, notarize the Mac flasher, upload.
#
#   tools/release.sh [-n NOTES] [--only PKG,…]  (NOTES default: this version's Changes in RELEASE_NOTES.md) [--dry-run] [--build-only] [--publish] [--within PKG,…] [--allow-dirty] [--rebuild-os] [--force]
#
#   --only        packages and/or groups, comma-separated:
#                   flashers = flash-macos,flash-linux,flash-linux-arm64,flash-windows
#                   os = os-x86_64,os-arm64
#                   land = land-x86_64,land-arm64
#                 e.g. --force --only flashers      --only os-arm64,flash-macos
#   --dry-run     only show what is missing      --build-only  build into dist/release/<tag>, upload nothing
#   --publish     build nothing: upload what --build-only runs left in dist/release/<tag> (their
#                 .built-* records), merging bia-os.json once (GitHub Actions: builds on several
#                 runners, one publish job; .github/workflows/release.yml)
#   --within      like --only, but narrows instead of choosing: a runner's own packages, so a
#                 workflow can pass the person's --only and its runner's set together
#   --allow-dirty allow uncommitted/unpushed work (tests; never for a real release)
#   --force       rebuild every selected package even if the release has it (the OS from
#                 scratch, as --rebuild-os) and replace its assets; without --only: all of them
#
# Tag: v<workspace version> (Cargo.toml). Packages and their release assets:
#   os-x86_64      bia-x86_64.img.xz  minimal bia disk image (tools/os-bundle.py: GPT, ESP,
#                  BIOS GRUB, BIAROOT; no keys, no model). Etcher/Rufus/dd write it as it is
#                  (bia-grow fills the stick on first boot); bia-deploy adds keys and a model
#   os-arm64       bia-arm64.img.xz
#                  (+ bia-os.json, the manifest bia-deploy downloads the OS with)
#   flash-macos    bia-deploy-macos-universal.dmg + bia-deploy-cli-macos-universal.zip
#                  (Developer ID signed + notarized; credentials in .env)
#   flash-linux    bia-deploy-linux-x86_64.tar.gz  (built in bia-build:x86_64)
#   flash-linux-arm64  bia-deploy-linux-arm64.tar.gz  (built in bia-build:arm64)
#   takeover       takeover.sh  replaces a running Linux (a VPS) with bia; downloads the
#                  Linux flasher for its architecture, checked against bia-os.json "tools"
#   flash-windows  bia-deploy-windows-x86_64.zip   (cross-built with MinGW in bia-build:x86_64)
#   land-x86_64    bia-land-x86_64.iso.xz  installer ISO that installs bia and waits to land
#   land-arm64     bia-land-arm64.iso.xz   (xz: the raw ISO is over GitHub's 2 GiB file limit;
#                  Etcher and Raspberry Pi Imager write .iso.xz directly)
# Asset names carry no version, so https://github.com/zond80/bia/releases/latest/download/<name>
# always points at the newest release (website download links, bia-deploy).
#
# Before anything ships, every built file is scanned for the FMM API keys, the Apple
# app password, $HF_TOKEN and this machine's home path.
set -euo pipefail
SELF=$(cd "$(dirname "$0")" && pwd)/$(basename "$0")
cd "$(dirname "$SELF")/.."
ROOT=$(pwd)
REPO=zond80/bia
ALL=(os-x86_64 os-arm64 land-x86_64 land-arm64 flash-macos flash-linux flash-linux-arm64 flash-windows takeover)

NOTES="" ONLY="" WITHIN="" DRY=0 DIRTY=0 REBUILD_OS=0 BUILD_ONLY=0 PUBLISH=0 FORCE=0
die() { echo "error: $*" >&2; exit 1; }
usage() { awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' "$SELF"; }
while [[ $# -gt 0 ]]; do
  case $1 in
    -n|--notes) [[ $# -ge 2 ]] || die "--notes needs a value"; NOTES=$2; shift 2 ;;
    --only) [[ $# -ge 2 ]] || die "--only needs a value (see --help)"; ONLY=$2; shift 2 ;;
    --dry-run) DRY=1; shift ;;
    --build-only) BUILD_ONLY=1; shift ;;
    --publish) PUBLISH=1; shift ;;
    --within) [[ $# -ge 2 ]] || die "--within needs a value"; WITHIN=$2; shift 2 ;;
    --allow-dirty) DIRTY=1; shift ;;
    --rebuild-os) REBUILD_OS=1; shift ;;
    --force) FORCE=1; REBUILD_OS=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown option $1 (see --help)" ;;
  esac
done

# ---- what to release: --only packages and groups, in ALL's order, no duplicates ----
expand() { # comma list → " pkg pkg … " (groups expanded)
  local want=" " i items
  IFS=, read -r -a items <<<"$1"
  for i in "${items[@]}"; do
    i=${i//[[:space:]]/}
    case $i in
      "") ;;
      flashers|flash) want+="flash-macos flash-linux flash-linux-arm64 flash-windows " ;;
      os) want+="os-x86_64 os-arm64 " ;;
      land) want+="land-x86_64 land-arm64 " ;;
      *) [[ " ${ALL[*]} " == *" $i "* ]] || die "unknown package '$i' (packages: ${ALL[*]}; groups: flashers, os, land)"
         want+="$i " ;;
    esac
  done
  echo "$want"
}
PKGS=("${ALL[@]}")
for sel in "$ONLY" "$WITHIN"; do
  [[ -n "$sel" ]] || continue
  want=$(expand "$sel")
  keep=()
  for p in "${PKGS[@]}"; do [[ $want == *" $p "* ]] && keep+=("$p"); done
  PKGS=(${keep[@]+"${keep[@]}"})
done
if [[ ${#PKGS[@]} -eq 0 ]]; then
  [[ -n "$WITHIN" ]] || die "--only selects nothing"
  echo "== nothing of ${ONLY:-all} is built on this runner ($WITHIN)"
  exit 0
fi

# docker only where a package needs it (the macOS runner has none)
for c in gh git python3 xz; do command -v "$c" >/dev/null || die "'$c' is required"; done
gh auth status >/dev/null 2>&1 || die "gh is not logged in (gh auth login)"

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
[[ -n "$VERSION" ]] || die "no version in Cargo.toml"
TAG="v$VERSION"
SHA=$(git rev-parse HEAD)
OUT="$ROOT/dist/release/$TAG"
mkdir -p "$OUT"

# ---- the release must be the pushed commit it is tagged on ----
if [[ $DIRTY -eq 0 ]]; then
  [[ -z "$(git status --porcelain)" ]] || die "uncommitted changes; commit them (or --allow-dirty for a test run)"
  git fetch -q origin
  git branch -r --contains "$SHA" | grep -q "origin/" || die "HEAD $SHA is not pushed to origin"
fi

# ---- what the release already has ----
EXISTS=0
HAVE=""
if gh release view "$TAG" -R "$REPO" >/dev/null 2>&1; then
  EXISTS=1
  HAVE=$(gh release view "$TAG" -R "$REPO" --json assets -q '.assets[].name')
fi
has() { grep -qx "$1" <<<"$HAVE"; }
manifest_has_tool() {
  has bia-os.json || return 1
  gh release download "$TAG" -R "$REPO" -p bia-os.json -O - 2>/dev/null \
    | python3 -c "import json,sys; sys.exit(0 if '$1' in json.load(sys.stdin).get('tools', {}) else 1)"
}
manifest_has_arch() {
  has bia-os.json || return 1
  gh release download "$TAG" -R "$REPO" -p bia-os.json -O - 2>/dev/null \
    | python3 -c "import json,sys; sys.exit(0 if 'image' in json.load(sys.stdin).get('os', {}).get('$1', {}) else 1)"
}
complete() { # pkg
  case $1 in
    os-*) local a=${1#os-}; has "bia-$a.img.xz" && manifest_has_arch "$a" ;;
    flash-macos) has bia-deploy-macos-universal.dmg && has bia-deploy-cli-macos-universal.zip ;;
    flash-linux) has bia-deploy-linux-x86_64.tar.gz && manifest_has_tool bia-deploy-linux-x86_64.tar.gz ;;
    flash-linux-arm64) has bia-deploy-linux-arm64.tar.gz && manifest_has_tool bia-deploy-linux-arm64.tar.gz ;;
    takeover) has takeover.sh ;;
    flash-windows) has bia-deploy-windows-x86_64.zip ;;
    land-*) has "bia-land-${1#land-}.iso.xz" ;;
    *) die "unknown package $1" ;;
  esac
}

TODO=()
[[ $PUBLISH -eq 1 ]] && PKGS=()
for p in ${PKGS[@]+"${PKGS[@]}"}; do
  if [[ $FORCE -eq 1 ]]; then echo "   build $p (--force)"; TODO+=("$p")
  elif complete "$p"; then echo "   have  $p"; else echo "   build $p"; TODO+=("$p"); fi
done
echo "== release $TAG at ${SHA:0:12} ($([[ $EXISTS -eq 1 ]] && echo exists || echo new)); to build: ${TODO[*]:-nothing}"
[[ ${#TODO[@]} -gt 0 || $PUBLISH -eq 1 ]] || { echo "== nothing missing"; exit 0; }
[[ $DRY -eq 0 ]] || exit 0

# default notes: this version's "Changes" in RELEASE_NOTES.md (the same text bia tells its owner)
if [[ -z "$NOTES" && -f RELEASE_NOTES.md ]]; then
  NOTES=$(awk -v v="## $VERSION" '
    $0 == v { inv = 1; next }
    inv && /^## / { exit }
    inv && /^### / { inc = ($0 == "### Changes"); next }
    inv && inc && NF { print }' RELEASE_NOTES.md)
  [[ -n "$NOTES" ]] && echo "== release notes from RELEASE_NOTES.md ($VERSION)"
fi
if [[ $BUILD_ONLY -eq 0 && $EXISTS -eq 0 && -z "$NOTES" ]]; then
  [[ -t 0 ]] || die "release notes are required for a new release (-n, or a $VERSION section in RELEASE_NOTES.md)"
  echo "Release notes for $TAG (end with Ctrl-D):"
  NOTES=$(cat)
  [[ -n "${NOTES//[[:space:]]/}" ]] || die "release notes cannot be empty"
fi

# ---- secrets that must never ship ----
SECRETS="$OUT/.secrets"
python3 - "$SECRETS" <<'PY'
import json, os, sys
out = set()
for p in ("fmm.json", "source/flash/files/state/fmm.json"):
    try:
        d = json.load(open(p))
    except Exception:
        continue
    for k in ("api_key",):
        if isinstance(d.get(k), str) and len(d[k]) >= 12:
            out.add(d[k])
    c = d.get("chatgpt") or {}
    for k in ("access_token", "refresh_token"):
        if isinstance(c.get(k), str) and len(c[k]) >= 12:
            out.add(c[k])
for f in (".env", "installers/macos/.env"):
    try:
        for line in open(f):
            k, _, v = line.strip().partition("=")
            if k in ("APPLE_APP_SPECIFIC_PASSWORD",) and len(v.strip().strip('"')) >= 8:
                out.add(v.strip().strip('"'))
    except OSError:
        pass
if len(os.environ.get("HF_TOKEN", "")) >= 8:
    out.add(os.environ["HF_TOKEN"])
# this machine's home path (a developer's /Users/<name>/); not on a CI runner, whose
# /home/runner/ is in prebuilt libraries made on GitHub Actions and says nothing about anyone
if not os.environ.get("GITHUB_ACTIONS"):
    out.add(os.path.expanduser("~") + "/")
# no empty line: grep -f takes one as a pattern that matches every file
open(sys.argv[1], "w").write("".join(f"{s}\n" for s in sorted(out) if s))
os.chmod(sys.argv[1], 0o600)
PY
trap 'rm -f "$SECRETS"' EXIT
scan() { # files…  (raw binaries, before they are packed)
  local f
  [[ -s "$SECRETS" ]] || return 0
  for f in "$@"; do
    if LC_ALL=C grep -a -q -F -f "$SECRETS" "$f"; then die "a secret or home path is inside $f; not releasing it"; fi
  done
}

UPLOAD=()
OS_BUILT=()
TOOLS_BUILT=()

# the build container of an architecture (make builder), made when this machine has none
builder() { # arch
  command -v docker >/dev/null || die "'docker' is required for $1 builds"
  docker image inspect "bia-build:$1" >/dev/null 2>&1 || { echo "== build container bia-build:$1"; make ARCH="$1" builder; }
}

SYSTEM_READY=" "
system() { # arch — build dist/<arch> (root.img and binaries) at most once per run
  local a=$1
  [[ $SYSTEM_READY == *" $a "* ]] && return 0
  builder "$a"
  if [[ $REBUILD_OS -eq 1 || ! -f "dist/$a/root.img" ]]; then
    echo "== building the $a OS (make ARCH=$a build)"
    make ARCH="$a" build
  else
    echo "== using dist/$a/root.img from $(date -r "dist/$a/root.img" '+%F %R') (--rebuild-os to rebuild)"
  fi
  SYSTEM_READY+="$a "
}

build_os() { # arch
  local a=$1
  system "$a"
  rm -f "$OUT/bia-$a.img.xz" "$OUT/os-$a.json"
  python3 tools/os-bundle.py "$a" "$OUT" --secrets "$SECRETS"
  UPLOAD+=("$OUT/bia-$a.img.xz")
  OS_BUILT+=("$a")
}

build_land() { # arch
  local a=$1 iso="dist/$1/bia-land.iso" out="$OUT/bia-land-$1.iso.xz"
  system "$a"
  echo "== land installer ISO ($a)"
  rm -f "$iso" "$out"
  ARCH="$a" tools/mkiso.sh
  scan "$iso"
  echo "== compressing $(( $(stat -f%z "$iso" 2>/dev/null || stat -c%s "$iso") >> 20 )) MiB ISO with xz"
  xz -T0 -6 -c "$iso" >"$out"
  local size
  size=$(stat -f%z "$out" 2>/dev/null || stat -c%s "$out")
  (( size < 2 * 1024 * 1024 * 1024 )) || die "$out is $((size >> 20)) MiB, over GitHub's 2 GiB file limit"
  UPLOAD+=("$out")
}

build_macos() {
  echo "== macOS flasher (universal, signed, notarized)"
  installers/macos/release.sh --universal
  local d=installers/dist/macos-universal
  scan "$d/bia-deploy" "$d/bia deploy.app/Contents/MacOS/bia-deploy-gui"
  cp -f "$d/bia-deploy-$VERSION-macos-universal.dmg" "$OUT/bia-deploy-macos-universal.dmg"
  cp -f "$d/bia-deploy-cli-$VERSION-macos-universal.zip" "$OUT/bia-deploy-cli-macos-universal.zip"
  UPLOAD+=("$OUT/bia-deploy-macos-universal.dmg" "$OUT/bia-deploy-cli-macos-universal.zip")
}

# Linux and Windows flashers: built in the builder of their architecture (paths remapped,
# no debuginfo). BUILDER_ARCH picks it (default x86_64).
docker_flash() { # command
  local ba=${BUILDER_ARCH:-x86_64} plat=linux/amd64
  [[ $ba == arm64 ]] && plat=linux/arm64
  builder "$ba"
  docker run --rm --platform "$plat" -v "$ROOT":/work -v "bia-cache-$ba":/cache -w /work \
    -e CARGO_TARGET_DIR=/cache/flash-target -e CARGO_PROFILE_RELEASE_DEBUG=0 -e CARGO_PROFILE_RELEASE_STRIP=debuginfo \
    -e RUSTFLAGS="--remap-path-prefix=/work=/bia --remap-path-prefix=/cache/cargo=/cargo --remap-path-prefix=/root=/home" \
    "bia-build:$ba" bash -lc "export CARGO_HOME=\${CARGO_HOME:-/cache/cargo}; $1; rc=\$?; chown -R $(id -u):$(id -g) /work/dist/release 2>/dev/null; exit \$rc"
}

# A file takeover.sh downloads: size + sha256 into bia-os.json "tools".
tool_entry() { # file
  local f=$1 name
  name=$(basename "$f")
  python3 - "$f" "$OUT/tool-$name.json" <<'PY'
import hashlib, json, os, sys
f, out = sys.argv[1], sys.argv[2]
h = hashlib.sha256()
with open(f, "rb") as fh:
    for b in iter(lambda: fh.read(1 << 20), b""):
        h.update(b)
json.dump({"file": os.path.basename(f), "size": os.path.getsize(f), "sha256": h.hexdigest()}, open(out, "w"))
PY
  TOOLS_BUILT+=("$name")
}

build_linux() { # arch (x86_64 | arm64)
  local a=${1:-x86_64} s="$ROOT/dist/release/.linux-${1:-x86_64}" st tgz
  echo "== Linux flasher ($a)"
  BUILDER_ARCH=$a docker_flash "cargo build --release -p bia-deploy --features gui && mkdir -p /work/dist/release/.linux-$a && cp /cache/flash-target/release/bia-deploy /cache/flash-target/release/bia-deploy-gui /work/dist/release/.linux-$a/"
  scan "$s/bia-deploy" "$s/bia-deploy-gui"
  st=$(mktemp -d)
  mkdir -p "$st/bia-deploy"
  cp "$s/bia-deploy" "$s/bia-deploy-gui" installers/bia-deploy/assets/icon-256.png "$st/bia-deploy/"
  cat >"$st/bia-deploy/README.txt" <<EOF
bia deploy $VERSION — write a bootable bia USB stick (Linux $a)

  sudo ./bia-deploy-gui        window app (needs root to write the stick)
  sudo ./bia-deploy list
  sudo ./bia-deploy write --disk /dev/sdX --yes   # downloads the bia system from GitHub
  sudo ./bia-deploy takeover   # replace this running Linux (e.g. a VPS) with bia

https://github.com/$REPO
EOF
  tgz="$OUT/bia-deploy-linux-$a.tar.gz"
  tar -C "$st" -czf "$tgz" bia-deploy
  rm -rf "$st" "$s"
  tool_entry "$tgz"
  UPLOAD+=("$tgz")
}

build_takeover() {
  echo "== takeover.sh"
  bash -n tools/takeover.sh
  scan tools/takeover.sh
  cp -f tools/takeover.sh "$OUT/takeover.sh"
  UPLOAD+=("$OUT/takeover.sh")
}

build_windows() {
  echo "== Windows flasher (x86_64, MinGW cross build)"
  docker_flash "command -v x86_64-w64-mingw32-gcc >/dev/null || { apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends gcc-mingw-w64-x86-64 binutils-mingw-w64-x86-64 >/dev/null; }; rustup target list --installed | grep -qx x86_64-pc-windows-gnu || rustup target add x86_64-pc-windows-gnu; cargo build --release -p bia-deploy --features gui --target x86_64-pc-windows-gnu && mkdir -p /work/dist/release/.windows && cp /cache/flash-target/x86_64-pc-windows-gnu/release/bia-deploy.exe /cache/flash-target/x86_64-pc-windows-gnu/release/bia-deploy-gui.exe /work/dist/release/.windows/"
  local s="$ROOT/dist/release/.windows" st
  scan "$s/bia-deploy.exe" "$s/bia-deploy-gui.exe"
  st=$(mktemp -d)
  mkdir -p "$st/bia-deploy"
  cp "$s/bia-deploy.exe" "$s/bia-deploy-gui.exe" "$st/bia-deploy/"
  printf 'bia deploy %s - write a bootable bia USB stick (Windows x86_64)\r\n\r\nRun bia-deploy-gui.exe as administrator.\r\nThe bia system is downloaded from GitHub.\r\n\r\nhttps://github.com/%s\r\n' "$VERSION" "$REPO" >"$st/bia-deploy/README.txt"
  rm -f "$OUT/bia-deploy-windows-x86_64.zip"
  (cd "$st" && zip -qr "$OUT/bia-deploy-windows-x86_64.zip" bia-deploy)
  rm -rf "$st" "$s"
  UPLOAD+=("$OUT/bia-deploy-windows-x86_64.zip")
}

for p in ${TODO[@]+"${TODO[@]}"}; do
  case $p in
    os-*) build_os "${p#os-}" ;;
    land-*) build_land "${p#land-}" ;;
    flash-macos) build_macos ;;
    flash-linux) build_linux x86_64 ;;
    flash-linux-arm64) build_linux arm64 ;;
    takeover) build_takeover ;;
    flash-windows) build_windows ;;
  esac
done

# ---- --build-only leaves a record per run; --publish collects them ----
if [[ $PUBLISH -eq 1 ]]; then
  shopt -s nullglob
  recs=("$OUT"/.built-*)
  shopt -u nullglob
  [[ ${#recs[@]} -gt 0 ]] || die "nothing to publish: no .built-* records in $OUT (run --build-only first)"
  for r in "${recs[@]}"; do
    while read -r kind val; do
      case $kind in
        file) [[ -f "$OUT/$val" ]] || die "$r names $val, which is not in $OUT"; UPLOAD+=("$OUT/$val") ;;
        os) OS_BUILT+=("$val") ;;
        tool) TOOLS_BUILT+=("$val") ;;
      esac
    done <"$r"
  done
  echo "== publishing ${#UPLOAD[@]} files from ${#recs[@]} build(s)"
elif [[ $BUILD_ONLY -eq 1 ]]; then
  rec="$OUT/.built-$(date +%s)-$$"
  {
    for f in ${UPLOAD[@]+"${UPLOAD[@]}"}; do echo "file $(basename "$f")"; done
    for a in ${OS_BUILT[@]+"${OS_BUILT[@]}"}; do echo "os $a"; done
    for t in ${TOOLS_BUILT[@]+"${TOOLS_BUILT[@]}"}; do echo "tool $t"; done
  } >"$rec"
fi

# ---- manifest: the existing one plus the architectures built now ----
if [[ ${#OS_BUILT[@]} -gt 0 || ${#TOOLS_BUILT[@]} -gt 0 ]]; then
  old="$OUT/bia-os.old.json"
  rm -f "$old"
  [[ $EXISTS -eq 1 ]] && has bia-os.json && gh release download "$TAG" -R "$REPO" -p bia-os.json -O "$old"
  python3 - "$OUT" "$VERSION" "$TAG" "$old" ${OS_BUILT[@]+"${OS_BUILT[@]}"} -- ${TOOLS_BUILT[@]+"${TOOLS_BUILT[@]}"} <<'PY'
import json, os, sys
out, version, tag, old = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
rest = sys.argv[5:]
sep = rest.index("--")
arches, tools = rest[:sep], rest[sep + 1:]
m = json.load(open(old)) if os.path.isfile(old) else {}
m.update({"version": version, "tag": tag})
m.setdefault("os", {})
# Entries from before the disk image (root parts + boot bundle) cannot be written any more.
m["os"] = {a: e for a, e in m["os"].items() if "image" in e}
for a in arches:
    m["os"][a] = json.load(open(f"{out}/os-{a}.json"))
m.setdefault("tools", {})
for t in tools:
    m["tools"][t] = json.load(open(f"{out}/tool-{t}.json"))
json.dump(m, open(f"{out}/bia-os.json", "w"), indent=2)
print("== bia-os.json:", ", ".join(sorted(m["os"])), "| tools:", ", ".join(sorted(m["tools"])) or "-")
PY
  UPLOAD+=("$OUT/bia-os.json")
fi

# ---- publish ----
if [[ $BUILD_ONLY -eq 1 ]]; then
  echo "== built (not uploaded):"
  ls -l "${UPLOAD[@]}"
  exit 0
fi
if [[ $EXISTS -eq 0 ]]; then
  echo "== creating release $TAG on ${SHA:0:12}"
  gh release create "$TAG" -R "$REPO" --target "$SHA" --title "bia $VERSION" --notes "$NOTES"
fi
echo "== uploading ${#UPLOAD[@]} files"
# --clobber: an OS rebuilt for a missing manifest entry replaces its old image.
gh release upload "$TAG" -R "$REPO" --clobber "${UPLOAD[@]}"
# Assets of the old OS packaging (zstd root parts + boot bundle) for a rebuilt arch.
for a in ${OS_BUILT[@]+"${OS_BUILT[@]}"}; do
  while read -r name; do
    echo "   removing stale $name"
    gh release delete-asset "$TAG" "$name" -R "$REPO" -y
  done < <(grep -E "^bia-(root-$a\.[0-9]+\.zst|boot-$a\.tar\.zst)$" <<<"$HAVE" || true)
done
echo "== done: $(gh release view "$TAG" -R "$REPO" --json url -q .url)"
