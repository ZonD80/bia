#!/bin/bash
# Build the website's social preview card (og:image, 1200×630) from brand/potato.svg
# (macOS host: headless Chrome renders it). Shown when a link to the site is shared in
# Threads, Telegram, WhatsApp, X, …
#
#   website/public/og.png
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$(pwd)
CHROME=${CHROME:-"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"}
[ -x "$CHROME" ] || { echo "need Google Chrome (set CHROME=…)" >&2; exit 1; }
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

python3 - "$ROOT/brand/potato.svg" "$TMP/card.html" <<'PY'
import sys
svg = open(sys.argv[1]).read()
svg = svg.replace('<svg ', '<svg class="potato" ', 1)
open(sys.argv[2], "w").write("""<!doctype html><html><head><meta charset="utf-8">
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Fraunces:opsz,wght@9..144,600;9..144,700&family=Manrope:wght@500;700&display=block">
<style>
html, body { margin: 0; width: 1200px; height: 630px; overflow: hidden; }
body {
  background: radial-gradient(circle at 78% 52%, #f5e8d2 0, #fbf8f2 46%), #fbf8f2;
  color: #1d2126; font-family: Manrope, system-ui, sans-serif;
  display: grid; grid-template-columns: 1fr 520px; align-items: center;
}
.text { padding: 0 0 0 88px; }
.eyebrow { color: #3f6f2a; font-weight: 700; font-size: 24px; letter-spacing: 0.12em; text-transform: uppercase; }
h1 { font-family: Fraunces, Georgia, serif; font-weight: 700; font-size: 168px; line-height: 1; margin: 14px 0 18px; letter-spacing: -0.02em; }
.tag { font-family: Fraunces, Georgia, serif; font-weight: 600; font-size: 50px; line-height: 1.1; margin: 0 0 22px; }
.sub { font-size: 26px; line-height: 1.4; color: #3e4651; margin: 0; max-width: 560px; }
.potato { width: 500px; height: 500px; }
.bar { position: absolute; left: 0; right: 0; bottom: 0; height: 14px; background: #3f6f2a; }
</style></head><body>
<div class="text">
  <div class="eyebrow">Boot into agent</div>
  <h1>bia</h1>
  <p class="tag">Turn anything into someone</p>
  <p class="sub">A helpful AI personality with a computer of its own — in your messengers, in your language.</p>
</div>
<div>""" + svg + """</div>
<div class="bar"></div>
</body></html>
""")
PY

"$CHROME" --headless=new --disable-gpu --hide-scrollbars --force-device-scale-factor=1 \
  --window-size=1200,630 --virtual-time-budget=8000 \
  --screenshot="$ROOT/website/public/og.png" "file://$TMP/card.html" 2>/dev/null
sips -g pixelWidth -g pixelHeight website/public/og.png | tail -2
