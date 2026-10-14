#!/usr/bin/env bash
# Release bia on GitHub (zond80/bia): check what the release for this version already
# has, build only the missing packages, notarize the Mac flasher, upload.
#
#   tools/release.sh [-n NOTES] [--only PKG,…] [--dry-run] [--build-only] [--allow-dirty] [--rebuild-os] [--force]
#
#   --only        packages and/or groups, comma-separated:
#                   flashers = flash-macos,flash-linux,flash-windows   os = os-x86_64,os-arm64
#                 e.g. --force --only flashers      --only os-arm64,flash-macos
#   --dry-run     only show what is missing      --build-only  build into dist/release/<tag>, upload nothing
#   --allow-dirty allow uncommitted/unpushed work (tests; never for a real release)
#   --force       rebuild every selected package even if the release has it (the OS from
#                 scratch, as --rebuild-os) and replace its assets; without --only: all of them
#
# Tag: v<workspace version> (Cargo.toml). Packages and their release assets:
#   os-x86_64      bia-root-x86_64.NNN.zst (1 GiB raw parts) + bia-boot-x86_64.tar.zst
#   os-arm64       bia-root-arm64.NNN.zst  + bia-boot-arm64.tar.zst
#                  (+ bia-os.json, the manifest bia-flash downloads the OS with)
#   flash-macos    bia-flash-macos-universal.dmg + bia-flash-cli-macos-universal.zip
#                  (Developer ID signed + notarized; credentials in .env)
#   flash-linux    bia-flash-linux-x86_64.tar.gz  (built in bia-build:x86_64)
#   flash-windows  bia-flash-windows-x86_64.zip   (cross-built with MinGW in bia-build:x86_64)
# Asset names carry no version, so https://github.com/zond80/bia/releases/latest/download/<name>
# always points at the newest release (website download links, bia-flash).
#
# Before anything ships, every built file is scanned for the FMM API keys, the Apple
# app password, $HF_TOKEN and this machine's home path.
set -euo pipefail
SELF=$(cd "$(dirname "$0")" && pwd)/$(basename "$0")
cd "$(dirname "$SELF")/.."
ROOT=$(pwd)
REPO=zond80/bia
ALL=(os-x86_64 os-arm64 flash-macos flash-linux flash-windows)

NOTES="" ONLY="" DRY=0 DIRTY=0 REBUILD_OS=0 BUILD_ONLY=0 FORCE=0
die() { echo "error: $*" >&2; exit 1; }
usage() { awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' "$SELF"; }
while [[ $# -gt 0 ]]; do
  case $1 in
    -n|--notes) [[ $# -ge 2 ]] || die "--notes needs a value"; NOTES=$2; shift 2 ;;
    --only) [[ $# -ge 2 ]] || die "--only needs a value (see --help)"; ONLY=$2; shift 2 ;;
    --dry-run) DRY=1; shift ;;
    --build-only) BUILD_ONLY=1; shift ;;
    --allow-dirty) DIRTY=1; shift ;;
    --rebuild-os) REBUILD_OS=1; shift ;;
    --force) FORCE=1; REBUILD_OS=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown option $1 (see --help)" ;;
  esac
done

# ---- what to release: --only packages and groups, in ALL's order, no duplicates ----
PKGS=("${ALL[@]}")
if [[ -n "$ONLY" ]]; then
  want=" "
  IFS=, read -r -a items <<<"$ONLY"
  for i in "${items[@]}"; do
    i=${i//[[:space:]]/}
    case $i in
      "") ;;
      flashers|flash) want+="flash-macos flash-linux flash-windows " ;;
      os) want+="os-x86_64 os-arm64 " ;;
      *) [[ " ${ALL[*]} " == *" $i "* ]] || die "unknown package '$i' (packages: ${ALL[*]}; groups: flashers, os)"
         want+="$i " ;;
    esac
  done
  PKGS=()
  for p in "${ALL[@]}"; do [[ $want == *" $p "* ]] && PKGS+=("$p"); done
  [[ ${#PKGS[@]} -gt 0 ]] || die "--only selects nothing"
fi

for c in gh git python3 zstd docker; do command -v "$c" >/dev/null || die "'$c' is required"; done
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
manifest_has_arch() {
  has bia-os.json || return 1
  gh release download "$TAG" -R "$REPO" -p bia-os.json -O - 2>/dev/null \
    | python3 -c "import json,sys; sys.exit(0 if '$1' in json.load(sys.stdin).get('os', {}) else 1)"
}
complete() { # pkg
  case $1 in
    os-*) local a=${1#os-}; has "bia-boot-$a.tar.zst" && has "bia-root-$a.000.zst" && manifest_has_arch "$a" ;;
    flash-macos) has bia-flash-macos-universal.dmg && has bia-flash-cli-macos-universal.zip ;;
    flash-linux) has bia-flash-linux-x86_64.tar.gz ;;
    flash-windows) has bia-flash-windows-x86_64.zip ;;
    *) die "unknown package $1" ;;
  esac
}

TODO=()
for p in "${PKGS[@]}"; do
  if [[ $FORCE -eq 1 ]]; then echo "   build $p (--force)"; TODO+=("$p")
  elif complete "$p"; then echo "   have  $p"; else echo "   build $p"; TODO+=("$p"); fi
done
echo "== release $TAG at ${SHA:0:12} ($([[ $EXISTS -eq 1 ]] && echo exists || echo new)); to build: ${TODO[*]:-nothing}"
[[ ${#TODO[@]} -gt 0 ]] || { echo "== nothing missing"; exit 0; }
[[ $DRY -eq 0 ]] || exit 0

if [[ $BUILD_ONLY -eq 0 && $EXISTS -eq 0 && -z "$NOTES" ]]; then
  [[ -t 0 ]] || die "release notes are required for a new release (-n)"
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
out.add(os.path.expanduser("~") + "/")
open(sys.argv[1], "w").write("\n".join(sorted(out)) + "\n")
os.chmod(sys.argv[1], 0o600)
PY
trap 'rm -f "$SECRETS"' EXIT
scan() { # files…  (raw binaries, before they are packed)
  local f
  for f in "$@"; do
    if LC_ALL=C grep -a -q -F -f "$SECRETS" "$f"; then die "a secret or home path is inside $f; not releasing it"; fi
  done
}

UPLOAD=()
OS_BUILT=()

build_os() { # arch
  local a=$1
  if [[ $REBUILD_OS -eq 1 || ! -f "dist/$a/root.img" ]]; then
    echo "== building the $a OS (make ARCH=$a build)"
    make ARCH="$a" build
  else
    echo "== using dist/$a/root.img from $(date -r "dist/$a/root.img" '+%F %R') (--rebuild-os to rebuild)"
  fi
  rm -f "$OUT"/bia-root-"$a".*.zst "$OUT/bia-boot-$a.tar.zst" "$OUT/os-$a.json"
  python3 tools/os-bundle.py "$a" "$OUT" --secrets "$SECRETS"
  UPLOAD+=("$OUT"/bia-root-"$a".*.zst "$OUT/bia-boot-$a.tar.zst")
  OS_BUILT+=("$a")
}

build_macos() {
  echo "== macOS flasher (universal, signed, notarized)"
  installers/macos/release.sh --universal
  local d=installers/dist/macos-universal
  scan "$d/bia-flash" "$d/bia flash.app/Contents/MacOS/bia-flash-gui"
  cp -f "$d/bia-flash-$VERSION-macos-universal.dmg" "$OUT/bia-flash-macos-universal.dmg"
  cp -f "$d/bia-flash-cli-$VERSION-macos-universal.zip" "$OUT/bia-flash-cli-macos-universal.zip"
  UPLOAD+=("$OUT/bia-flash-macos-universal.dmg" "$OUT/bia-flash-cli-macos-universal.zip")
}

# Linux and Windows flashers: built in the x86_64 builder (paths remapped, no debuginfo).
docker_flash() { # target features…
  docker run --rm --platform linux/amd64 -v "$ROOT":/work -v bia-cache-x86_64:/cache -w /work \
    -e CARGO_TARGET_DIR=/cache/flash-target -e CARGO_PROFILE_RELEASE_DEBUG=0 -e CARGO_PROFILE_RELEASE_STRIP=debuginfo \
    -e RUSTFLAGS="--remap-path-prefix=/work=/bia --remap-path-prefix=/cache/cargo=/cargo --remap-path-prefix=/root=/home" \
    bia-build:x86_64 bash -lc "export CARGO_HOME=\${CARGO_HOME:-/cache/cargo}; $1"
}

build_linux() {
  echo "== Linux flasher (x86_64)"
  docker_flash "cargo build --release -p bia-flash --features gui && mkdir -p /work/dist/release/.linux && cp /cache/flash-target/release/bia-flash /cache/flash-target/release/bia-flash-gui /work/dist/release/.linux/"
  local s="$ROOT/dist/release/.linux" st
  scan "$s/bia-flash" "$s/bia-flash-gui"
  st=$(mktemp -d)
  mkdir -p "$st/bia-flash"
  cp "$s/bia-flash" "$s/bia-flash-gui" installers/bia-flash/assets/icon-256.png "$st/bia-flash/"
  cat >"$st/bia-flash/README.txt" <<EOF
bia flash $VERSION — write a bootable bia USB stick (Linux x86_64)

  sudo ./bia-flash-gui        window app (needs root to write the stick)
  sudo ./bia-flash list
  sudo ./bia-flash write --disk /dev/sdX --yes   # downloads the bia system from GitHub

https://github.com/$REPO
EOF
  tar -C "$st" -czf "$OUT/bia-flash-linux-x86_64.tar.gz" bia-flash
  rm -rf "$st" "$s"
  UPLOAD+=("$OUT/bia-flash-linux-x86_64.tar.gz")
}

build_windows() {
  echo "== Windows flasher (x86_64, MinGW cross build)"
  docker_flash "command -v x86_64-w64-mingw32-gcc >/dev/null || { apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends gcc-mingw-w64-x86-64 binutils-mingw-w64-x86-64 >/dev/null; }; rustup target list --installed | grep -qx x86_64-pc-windows-gnu || rustup target add x86_64-pc-windows-gnu; cargo build --release -p bia-flash --features gui --target x86_64-pc-windows-gnu && mkdir -p /work/dist/release/.windows && cp /cache/flash-target/x86_64-pc-windows-gnu/release/bia-flash.exe /cache/flash-target/x86_64-pc-windows-gnu/release/bia-flash-gui.exe /work/dist/release/.windows/"
  local s="$ROOT/dist/release/.windows" st
  scan "$s/bia-flash.exe" "$s/bia-flash-gui.exe"
  st=$(mktemp -d)
  mkdir -p "$st/bia-flash"
  cp "$s/bia-flash.exe" "$s/bia-flash-gui.exe" "$st/bia-flash/"
  printf 'bia flash %s - write a bootable bia USB stick (Windows x86_64)\r\n\r\nRun bia-flash-gui.exe as administrator.\r\nThe bia system is downloaded from GitHub.\r\n\r\nhttps://github.com/%s\r\n' "$VERSION" "$REPO" >"$st/bia-flash/README.txt"
  rm -f "$OUT/bia-flash-windows-x86_64.zip"
  (cd "$st" && zip -qr "$OUT/bia-flash-windows-x86_64.zip" bia-flash)
  rm -rf "$st" "$s"
  UPLOAD+=("$OUT/bia-flash-windows-x86_64.zip")
}

for p in "${TODO[@]}"; do
  case $p in
    os-*) build_os "${p#os-}" ;;
    flash-macos) build_macos ;;
    flash-linux) build_linux ;;
    flash-windows) build_windows ;;
  esac
done

# ---- manifest: the existing one plus the architectures built now ----
if [[ ${#OS_BUILT[@]} -gt 0 ]]; then
  old="$OUT/bia-os.old.json"
  rm -f "$old"
  [[ $EXISTS -eq 1 ]] && has bia-os.json && gh release download "$TAG" -R "$REPO" -p bia-os.json -O "$old"
  python3 - "$OUT" "$VERSION" "$TAG" "$old" "${OS_BUILT[@]}" <<'PY'
import json, os, sys
out, version, tag, old, arches = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4], sys.argv[5:]
m = json.load(open(old)) if os.path.isfile(old) else {}
m.update({"version": version, "tag": tag})
m.setdefault("os", {})
for a in arches:
    m["os"][a] = json.load(open(f"{out}/os-{a}.json"))
json.dump(m, open(f"{out}/bia-os.json", "w"), indent=2)
print("== bia-os.json:", ", ".join(sorted(m["os"])))
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
# --clobber: an OS rebuilt for a missing manifest entry replaces its old parts.
gh release upload "$TAG" -R "$REPO" --clobber "${UPLOAD[@]}"
# A rebuilt OS may have fewer root parts than the one it replaced: drop the leftovers.
for a in ${OS_BUILT[@]+"${OS_BUILT[@]}"}; do
  while read -r name; do
    [[ -f "$OUT/$name" ]] && continue
    echo "   removing stale $name"
    gh release delete-asset "$TAG" "$name" -R "$REPO" -y
  done < <(grep -E "^bia-root-$a\.[0-9]+\.zst$" <<<"$HAVE" || true)
done
echo "== done: $(gh release view "$TAG" -R "$REPO" --json url -q .url)"
