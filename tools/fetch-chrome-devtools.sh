#!/usr/bin/env bash
# Vendor Node + chrome-devtools-mcp under source/flash/MCPs/chrome/ for the guest.
# Idempotent; re-run to bump versions.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CHROME="$ROOT/source/flash/MCPs/chrome"
NODE_VER="${NODE_VER:-24.21.0}"
CDM_VER="${CDM_VER:-1.10.1}"
CACHE="${BIA_CACHE:-$ROOT/cache}/chrome-devtools"

mkdir -p "$CACHE" "$CHROME/runtime" "$CHROME/devtools"

fetch() {
  local url="$1" dest="$2"
  if [ -f "$dest" ]; then
    return 0
  fi
  echo "== download $(basename "$dest")"
  curl -fsSL --retry 3 -o "$dest.partial" "$url"
  mv "$dest.partial" "$dest"
}

# --- Node (official binaries; only bin/node is kept) -----------------------------------------
for pair in "aarch64:arm64" "x86_64:x64"; do
  rarch="${pair%%:*}"
  narch="${pair##*:}"
  tarball="$CACHE/node-v${NODE_VER}-linux-${narch}.tar.xz"
  fetch "https://nodejs.org/dist/v${NODE_VER}/node-v${NODE_VER}-linux-${narch}.tar.xz" "$tarball"
  dest="$CHROME/runtime/$rarch"
  if [ ! -x "$dest/node" ] || [ "$tarball" -nt "$dest/node" ]; then
    echo "== extract node $rarch"
    rm -rf "$dest"
    mkdir -p "$dest"
    tar -xJf "$tarball" -C "$dest" --strip-components=2 "node-v${NODE_VER}-linux-${narch}/bin/node"
    chmod 755 "$dest/node"
  fi
  ls -lh "$dest/node"
done

# --- chrome-devtools-mcp (self-contained build/src; no node_modules) -------------------------
tgz="$CACHE/chrome-devtools-mcp-${CDM_VER}.tgz"
fetch "https://registry.npmjs.org/chrome-devtools-mcp/-/chrome-devtools-mcp-${CDM_VER}.tgz" "$tgz"
stamp="$CHROME/devtools/.version"
if [ ! -f "$stamp" ] || [ "$(cat "$stamp")" != "$CDM_VER" ] || [ ! -f "$CHROME/devtools/build/src/bin/chrome-devtools-mcp.js" ]; then
  echo "== extract chrome-devtools-mcp $CDM_VER"
  rm -rf "$CHROME/devtools"
  mkdir -p "$CHROME/devtools"
  tar -xzf "$tgz" -C "$CHROME/devtools" --strip-components=1
  printf '%s\n' "$CDM_VER" >"$CHROME/devtools/.version"
fi
ls -lh "$CHROME/devtools/package.json"
echo "== chrome-devtools runtime ready (node $NODE_VER, mcp $CDM_VER)"
