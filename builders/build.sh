#!/bin/bash
# Runs inside the bia-build container (Debian/glibc). /work = repo, /cache = volume.
#   builders/build.sh <arm64|x86_64> [rust|mcps|rootfs|all] [mcp…]
# For rust, optional trailing names limit which crates to build:
#   system secrets chrome screen carry telegram whatsapp meta agent
# (meta = the messenger and instagram MCPs and their Go helper bia-meta)
set -euo pipefail
ARCH=${1:-arm64}; WHAT=${2:-all}
if [ "$#" -ge 2 ]; then shift 2; else shift "$#" || true; fi
FILTERS=("$@")
case $ARCH in
  arm64)  RARCH=aarch64; EFI=BOOTAA64.EFI ;; # shim
  x86_64) RARCH=x86_64; EFI=BOOTX64.EFI ;;   # shim
  *) echo "unknown arch $ARCH"; exit 1 ;;
esac

export CARGO_HOME=/cache/cargo
export CARGO_TARGET_DIR=/cache/target-$ARCH
export CACHE=/cache
OUT=/work/dist/$ARCH
mkdir -p "$OUT" /work/cache
cd /work

ensure_ntgcalls_dylib() {
  local ver=${NTGCALLS_VER:-v3.0.0}
  local dest="/cache/ntgcalls-dylib-$ARCH"
  local dest_ver="/cache/ntgcalls-dylib-$ARCH-$ver"
  local asset
  case $ARCH in
    arm64) asset="ntgcalls.linux-arm64-shared_libs.zip" ;;
    x86_64) asset="ntgcalls.linux-x86_64-shared_libs.zip" ;;
  esac
  # Version the cache so bumping NTGCALLS_VER actually picks up a new .so.
  local zip="/cache/speech/$ver-$asset"
  mkdir -p /cache/speech "$dest_ver" "$dest"
  if [ ! -s "$zip" ]; then
    echo "   fetching $ver/$asset"
    curl -fL --retry 3 -A bia-build -o "$zip.partial" \
      "https://github.com/pytgcalls/ntgcalls/releases/download/${ver}/${asset}"
    mv "$zip.partial" "$zip"
  fi
  if [ ! -f "$dest_ver/libntgcalls.so" ]; then
    rm -rf /tmp/ntg-dylib-unpack
    mkdir -p /tmp/ntg-dylib-unpack
    unzip -qo "$zip" -d /tmp/ntg-dylib-unpack
    local so
    so=$(find /tmp/ntg-dylib-unpack -name 'libntgcalls.so' | head -1)
    [ -n "$so" ] || { echo "libntgcalls.so missing from $asset"; exit 1; }
    cp -a "$so" "$dest_ver/libntgcalls.so"
    local hdr
    hdr=$(find /tmp/ntg-dylib-unpack -name 'ntgcalls.h' | head -1 || true)
    [ -n "$hdr" ] && cp -a "$hdr" "$dest_ver/"
  fi
  cp -a "$dest_ver/libntgcalls.so" "$dest/libntgcalls.so"
  [ -f "$dest_ver/ntgcalls.h" ] && cp -a "$dest_ver/ntgcalls.h" "$dest/"
  echo "   installed $dest/libntgcalls.so ($ver)"
}

# Go toolchain for bia-meta (ensure_go)
. /work/builders/go-env.sh

# bia-meta: static Go binary (no cgo), the protocol side of the messenger/instagram MCPs.
go_meta() {
  ensure_go
  local ver
  ver=$(sed -n 's/^version = "\(.*\)"/\1/p' /work/Cargo.toml | head -1)
  echo "== go bia-meta $ver ($(go version))"
  (cd /work/source/go/bia-meta && CGO_ENABLED=0 go build -trimpath -buildvcs=false -ldflags "-s -w -X main.version=$ver" -o "$OUT/bia-meta" .)
  ls -l "$OUT/bia-meta"
}

rust() {
  local triple rf_var
  triple=$(rustc -vV | sed -n 's/^host: //p')
  rf_var="CARGO_TARGET_$(echo "$triple" | tr 'a-z-' 'A-Z_')_RUSTFLAGS"
  local base_flags=""
  [ "$ARCH" = x86_64 ] && base_flags="-C target-cpu=x86-64-v3"
  if command -v mold >/dev/null 2>&1; then
    base_flags="$base_flags -C link-arg=-fuse-ld=mold"
    echo "   linker=mold"
  fi

  echo "== rust ($triple, Debian/glibc dynamic)"
  export CARGO_TERM_COLOR=always
  export CARGO_TERM_PROGRESS_WHEN=always
  export CARGO_TERM_PROGRESS_WIDTH=${COLUMNS:-80}

  local -a main_pkgs=()
  local want_tg=0
  local want_wa=0
  local want_meta=0
  if [ ${#FILTERS[@]} -eq 0 ]; then
    main_pkgs=(bia-agent bia-mcp-system bia-mcp-web bia-mcp-secrets bia-mcp-chrome bia-mcp-screen bia-mcp-carry bia-mcp-lang bia-bridge)
    want_tg=1
    want_wa=1
    want_meta=1
  else
    local f
    for f in "${FILTERS[@]}"; do
      case $f in
        agent) main_pkgs+=(bia-agent) ;;
        system|web|secrets|chrome|screen|carry|lang) main_pkgs+=(bia-mcp-"$f") ;;
        bridge) main_pkgs+=(bia-bridge) ;;
        telegram) want_tg=1 ;;
        whatsapp) want_wa=1 ;;
        meta|messenger|instagram) want_meta=1 ;;
        *) echo "unknown rust filter: $f (want agent|system|web|secrets|chrome|screen|carry|lang|bridge|telegram|whatsapp|meta)" >&2; exit 1 ;;
      esac
    done
  fi

  if [ ${#main_pkgs[@]} -gt 0 ]; then
    export "$rf_var=$base_flags"
    local -a cargo_main=(cargo build --release --target "$triple")
    local p
    for p in "${main_pkgs[@]}"; do cargo_main+=(-p "$p"); done
    echo "   building: ${main_pkgs[*]}"
    if command -v stdbuf >/dev/null; then
      stdbuf -oL -eL "${cargo_main[@]}"
    else
      "${cargo_main[@]}"
    fi
    for p in "${main_pkgs[@]}"; do
      strip -o "$OUT/$p" "$CARGO_TARGET_DIR/$triple/release/$p"
    done
    echo "   main bins installed under $OUT"
    # bia-bridge for Windows too (the bia serves every platform's bridge; x86_64 builder only)
    if [[ " ${main_pkgs[*]} " == *" bia-bridge "* ]] && [ "$ARCH" = x86_64 ]; then
      command -v x86_64-w64-mingw32-gcc >/dev/null || { apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends gcc-mingw-w64-x86-64 binutils-mingw-w64-x86-64 >/dev/null; }
      rustup target list --installed | grep -qx x86_64-pc-windows-gnu || rustup target add x86_64-pc-windows-gnu
      cargo build --release -p bia-bridge --target x86_64-pc-windows-gnu
      x86_64-w64-mingw32-strip -o "$OUT/bia-bridge.exe" "$CARGO_TARGET_DIR/x86_64-pc-windows-gnu/release/bia-bridge.exe"
      echo "   bia-bridge.exe (windows x86_64) under $OUT"
    fi
    if command -v ldd >/dev/null && [ -x "$OUT/bia-agent" ]; then
      echo "   ldd bia-agent:"; ldd "$OUT/bia-agent" | sed 's/^/     /' || true
    fi
  fi

  if [ "$want_tg" = 1 ]; then
    # Telegram app credentials are compiled into the binary but kept out of git:
    # take them from the environment, else from the repo's .env (only these two).
    local v val
    for v in BIA_TG_API_ID BIA_TG_API_HASH; do
      if [ -z "${!v:-}" ] && [ -f /work/.env ]; then
        val=$(sed -n "s/^$v=//p" /work/.env | tail -1)
        val=${val%\"}; val=${val#\"}
        [ -n "$val" ] && export "$v=$val"
      fi
      [ -n "${!v:-}" ] || { echo "$v is not set: add it to .env (see .env.example)" >&2; exit 1; }
    done
    # Telegram: dynamic libntgcalls.so + guest rpath (also in crate build.rs).
    ensure_ntgcalls_dylib
    export NTGCALLS_LIB_DIR=/cache/ntgcalls-dylib-$ARCH
    export NTGCALLS_DYLIB=1
    export "$rf_var=$base_flags -C link-arg=-Wl,-rpath,/usr/local/lib/bia/speech/lib"
    echo "   NTGCALLS_LIB_DIR=$NTGCALLS_LIB_DIR NTGCALLS_DYLIB=1"
    local -a cargo_tg=(cargo build --profile release-telegram --target "$triple" -p bia-mcp-telegram)
    if command -v stdbuf >/dev/null; then
      stdbuf -oL -eL "${cargo_tg[@]}"
    else
      "${cargo_tg[@]}"
    fi
    strip -o "$OUT/bia-mcp-telegram" "$CARGO_TARGET_DIR/$triple/release-telegram/bia-mcp-telegram"
    if command -v ldd >/dev/null; then
      echo "   ldd bia-mcp-telegram:"
      ldd "$OUT/bia-mcp-telegram" | sed 's/^/     /' || true
    fi
  fi

  if [ "$want_wa" = 1 ]; then
    export "$rf_var=$base_flags"
    echo "   building bia-mcp-whatsapp (release-whatsapp)"
    local -a cargo_wa=(cargo build --profile release-whatsapp --target "$triple" -p bia-mcp-whatsapp)
    if command -v stdbuf >/dev/null; then
      stdbuf -oL -eL "${cargo_wa[@]}"
    else
      "${cargo_wa[@]}"
    fi
    strip -o "$OUT/bia-mcp-whatsapp" "$CARGO_TARGET_DIR/$triple/release-whatsapp/bia-mcp-whatsapp"
    if command -v ldd >/dev/null; then
      echo "   ldd bia-mcp-whatsapp:"
      ldd "$OUT/bia-mcp-whatsapp" | sed 's/^/     /' || true
    fi
  fi
  if [ "$want_meta" = 1 ]; then
    export "$rf_var=$base_flags"
    echo "   building bia-mcp-meta (messenger + instagram)"
    cargo build --release --target "$triple" -p bia-mcp-meta
    strip -o "$OUT/bia-mcp-messenger" "$CARGO_TARGET_DIR/$triple/release/bia-mcp-messenger"
    strip -o "$OUT/bia-mcp-instagram" "$CARGO_TARGET_DIR/$triple/release/bia-mcp-instagram"
    go_meta
  fi
  ls -l "$OUT"/bia-* 2>/dev/null || true
}

mcps() {
  echo "== assembling MCP folders for $RARCH"
  local M=/work/source/flash/MCPs
  mkdir -p "$M/system/bin/$RARCH" "$M/web/bin/$RARCH" "$M/telegram/bin/$RARCH" "$M/whatsapp/bin/$RARCH" "$M/secrets/bin/$RARCH" "$M/chrome/bin/$RARCH" "$M/screen/bin/$RARCH" "$M/carry/bin/$RARCH" "$M/lang/bin/$RARCH" "$M/messenger/bin/$RARCH" "$M/instagram/bin/$RARCH"
  cp "$OUT/bia-mcp-system" "$M/system/bin/$RARCH/"
  cp "$OUT/bia-mcp-web" "$M/web/bin/$RARCH/"
  # the bia serves bia-bridge to agents (web/bridge/); macOS comes from tools/bridge-macos.sh
  mkdir -p "$M/web/bridge"
  local barch=$RARCH
  [ "$barch" = aarch64 ] && barch=arm64
  [ -x "$OUT/bia-bridge" ] && cp "$OUT/bia-bridge" "$M/web/bridge/bia-bridge-linux-$barch"
  [ -f "$OUT/bia-bridge.exe" ] && cp "$OUT/bia-bridge.exe" "$M/web/bridge/bia-bridge-windows-x86_64.exe"
  cp "$OUT/bia-mcp-telegram" "$M/telegram/bin/$RARCH/"
  cp "$OUT/bia-mcp-whatsapp" "$M/whatsapp/bin/$RARCH/"
  cp "$OUT/bia-mcp-secrets" "$M/secrets/bin/$RARCH/"
  cp "$OUT/bia-mcp-chrome" "$M/chrome/bin/$RARCH/"
  cp "$OUT/bia-mcp-screen" "$M/screen/bin/$RARCH/"
  cp "$OUT/bia-mcp-carry" "$M/carry/bin/$RARCH/"
  cp "$OUT/bia-mcp-lang" "$M/lang/bin/$RARCH/"
  # instagram uses messenger's bia-meta (one Go binary for both)
  cp "$OUT/bia-mcp-messenger" "$OUT/bia-meta" "$M/messenger/bin/$RARCH/"
  cp "$OUT/bia-mcp-instagram" "$M/instagram/bin/$RARCH/"
}

rootfs() {
  echo "== debian rootfs + ESP (minbase + systemd + linux-image)"
  ROOT_MB=${ROOT_MB:-7100} /work/builders/rootfs.sh "$ARCH"
}

case $WHAT in
  rust) rust ;;
  mcps) mcps ;;
  rootfs) rootfs ;;
  all) FILTERS=(); rust; mcps; rootfs ;;
esac
echo "== done: $ARCH $WHAT"
