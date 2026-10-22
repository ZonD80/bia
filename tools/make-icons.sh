#!/bin/bash
# Build every bia icon from brand/potato.svg (macOS host: headless Chrome renders,
# sips scales, iconutil packs the .icns).
#
#   website/public/icon.svg             favicon (cropped mark, no ground shadow)
#   website/public/favicon.ico          16/32/48
#   website/public/apple-touch-icon.png 180, full-bleed panel square (iOS rounds it)
#   installers/macos/AppIcon.icns       bia flash.app (panel tile, macOS grid)
#   installers/bia-flash/assets/icon.ico      Windows .exe resource (16…256)
#   installers/bia-flash/assets/icon-256.png  window icon (egui; Linux/Windows title bar)
#   source/crates/bia-mcp-web/web/icon-192.png          web console notifications + manifest
#   source/crates/bia-mcp-web/web/apple-touch-icon.png  web console on an iOS home screen
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$(pwd)
CHROME=${CHROME:-"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"}
[ -x "$CHROME" ] || { echo "need Google Chrome (set CHROME=…)" >&2; exit 1; }
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
mkdir -p installers/bia-flash/assets

# ---- variants (SVG) ----
python3 - "$ROOT/brand/potato.svg" "$TMP" <<'PY'
import re, sys
src, tmp = sys.argv[1], sys.argv[2]
svg = open(src).read()
inner = svg[svg.index(">", svg.index("<svg")) + 1 : svg.rindex("</svg>")]
no_shadow = re.sub(r'\s*<!-- contact shadow -->\s*<ellipse[^>]*url\(#ground\)[^>]*/>', "", inner)

# Mark only, cropped to the potato and wires (favicon, Windows/Linux icons).
crop = '<svg viewBox="38 20 196 196" xmlns="http://www.w3.org/2000/svg">' + no_shadow + "</svg>\n"
open(f"{tmp}/mark.svg", "w").write(crop)

# macOS-style tile: 824px rounded square on the 1024 grid, light test-chamber panels.
tile = f'''<svg viewBox="0 0 1024 1024" xmlns="http://www.w3.org/2000/svg">
  <defs>
    <linearGradient id="tile" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0%" stop-color="#fbfbfc"/><stop offset="100%" stop-color="#cfd4d9"/>
    </linearGradient>
    <linearGradient id="seam" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0%" stop-color="#9aa2aa" stop-opacity="0.55"/><stop offset="100%" stop-color="#7d868f" stop-opacity="0.55"/>
    </linearGradient>
    <filter id="drop" x="-20%" y="-20%" width="140%" height="140%">
      <feDropShadow dx="0" dy="14" stdDeviation="18" flood-color="#000" flood-opacity="0.32"/>
    </filter>
    <clipPath id="round"><rect x="100" y="100" width="824" height="824" rx="185"/></clipPath>
  </defs>
  <g filter="url(#drop)"><rect x="100" y="100" width="824" height="824" rx="185" fill="url(#tile)"/></g>
  <g clip-path="url(#round)">
    <path d="M100 374H924M100 650H924M374 100V374M650 374V650M512 650V924" stroke="url(#seam)" stroke-width="5" fill="none"/>
    <path d="M100 377H924M100 653H924M377 100V374M653 377V650M515 653V924" stroke="#ffffff" stroke-opacity="0.7" stroke-width="3" fill="none"/>
    <rect x="100" y="100" width="824" height="824" rx="185" fill="none" stroke="#ffffff" stroke-opacity="0.8" stroke-width="6"/>
  </g>
  <rect x="100.5" y="100.5" width="823" height="823" rx="185" fill="none" stroke="#5b636b" stroke-opacity="0.35" stroke-width="2"/>
  <svg x="150" y="140" width="724" height="724" viewBox="20 6 210 210">{inner}</svg>
</svg>
'''
open(f"{tmp}/tile.svg", "w").write(tile)

# Full-bleed square for iOS (it rounds the corners itself and blackens transparency).
full = f'''<svg viewBox="0 0 1024 1024" xmlns="http://www.w3.org/2000/svg">
  <defs><linearGradient id="tile" x1="0" y1="0" x2="0" y2="1">
    <stop offset="0%" stop-color="#fbfbfc"/><stop offset="100%" stop-color="#cfd4d9"/></linearGradient></defs>
  <rect width="1024" height="1024" fill="url(#tile)"/>
  <path d="M0 341H1024M0 683H1024M341 0V341M683 341V683M512 683V1024" stroke="#8d959d" stroke-opacity="0.5" stroke-width="6" fill="none"/>
  <path d="M0 344H1024M0 686H1024M344 0V341M686 344V683M515 686V1024" stroke="#ffffff" stroke-opacity="0.7" stroke-width="3" fill="none"/>
  <svg x="70" y="50" width="884" height="884" viewBox="20 6 210 210">{inner}</svg>
</svg>
'''
open(f"{tmp}/full.svg", "w").write(full)
PY

render() { # svg out.png size
  printf '<html><body style="margin:0;background:transparent"><img src="file://%s" width="%s" height="%s"></body></html>' "$1" "$3" "$3" >"$TMP/r.html"
  "$CHROME" --headless=new --disable-gpu --hide-scrollbars --allow-file-access-from-files \
    --default-background-color=00000000 --window-size="$3,$3" --screenshot="$2" "file://$TMP/r.html" >/dev/null 2>&1
  [ -s "$2" ] || { echo "render failed: $1" >&2; exit 1; }
}
scale() { sips -z "$3" "$3" "$1" --out "$2" >/dev/null; }

render "$TMP/mark.svg" "$TMP/mark-1024.png" 1024
render "$TMP/tile.svg" "$TMP/tile-1024.png" 1024
render "$TMP/full.svg" "$TMP/full-1024.png" 1024

# ---- website ----
cp "$TMP/mark.svg" website/public/icon.svg
for n in 16 32 48 64 128 256; do scale "$TMP/mark-1024.png" "$TMP/mark-$n.png" "$n"; done
scale "$TMP/full-1024.png" website/public/apple-touch-icon.png 180

# ---- ICO (PNG entries; Vista+ and every browser read them) ----
ico() { # out sizes…
  local out=$1; shift
  python3 - "$out" "$@" <<PY
import struct, sys
out, sizes = sys.argv[1], [int(s) for s in sys.argv[2:]]
blobs = [open(f"$TMP/mark-{n}.png", "rb").read() for n in sizes]
head = struct.pack("<HHH", 0, 1, len(sizes))
off = 6 + 16 * len(sizes)
dirs = b""
for n, b in zip(sizes, blobs):
    dirs += struct.pack("<BBBBHHII", n % 256, n % 256, 0, 0, 1, 32, len(b), off)
    off += len(b)
open(out, "wb").write(head + dirs + b"".join(blobs))
PY
}
ico website/public/favicon.ico 16 32 48
ico installers/bia-flash/assets/icon.ico 16 32 48 64 128 256
cp "$TMP/mark-256.png" installers/bia-flash/assets/icon-256.png

# ---- web console (bia-mcp-web) ----
scale "$TMP/mark-1024.png" source/crates/bia-mcp-web/web/icon-192.png 192
cp website/public/apple-touch-icon.png source/crates/bia-mcp-web/web/apple-touch-icon.png

# ---- macOS .icns ----
set_dir="$TMP/AppIcon.iconset"
mkdir -p "$set_dir"
for n in 16 32 128 256 512; do
  scale "$TMP/tile-1024.png" "$set_dir/icon_${n}x${n}.png" "$n"
  scale "$TMP/tile-1024.png" "$set_dir/icon_${n}x${n}@2x.png" $((n * 2))
done
iconutil -c icns "$set_dir" -o installers/macos/AppIcon.icns

ls -l website/public/icon.svg website/public/favicon.ico website/public/apple-touch-icon.png \
  installers/macos/AppIcon.icns installers/bia-flash/assets/icon.ico installers/bia-flash/assets/icon-256.png \
  source/crates/bia-mcp-web/web/icon-192.png source/crates/bia-mcp-web/web/apple-touch-icon.png
cp "$TMP/tile-1024.png" "${PREVIEW_DIR:-$TMP}/tile-1024.png" 2>/dev/null || true
cp "$TMP/mark-1024.png" "${PREVIEW_DIR:-$TMP}/mark-1024.png" 2>/dev/null || true
