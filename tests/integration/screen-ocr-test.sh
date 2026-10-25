#!/bin/bash
# Container e2e of screen_ocr: the real bia-mcp-screen binary over MCP stdio, Xvfb + openbox +
# Chromium showing English and Russian buttons and light-on-dark text. Chrome's profile asks
# for ru,en-US, so the eslav recognizer is downloaded (as bia's) next to English. Clicks at
# the x,y screen_ocr reports (with screen_click, as the model would) must press the right
# button; a region gives the same screen coordinates and nothing outside it; screen_shot has
# no words. OCR runs on the image's onnxruntime, taken from the bia-cache-$ARCH build volume.
#
#   tests/integration/screen-ocr-test.sh            (runs itself in bia-dialog:$ARCH)
set -uo pipefail
ARCH=${ARCH:-arm64}
case $ARCH in arm64) PLATFORM=linux/arm64 ;; x86_64) PLATFORM=linux/amd64 ;; *) exit 2 ;; esac
if [ "${IN_CONTAINER:-}" != 1 ]; then
  cd "$(dirname "$0")/../.."
  # the image chrome-dialog-test.sh builds
  docker build -q --platform "$PLATFORM" -t "bia-dialog:$ARCH" - >/dev/null <<EOF || exit 1
FROM bia-build:$ARCH
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    xvfb openbox xdotool x11-utils scrot chromium fonts-dejavu-core python3 \
  && rm -rf /var/lib/apt/lists/* && (id bia || useradd -m bia)
EOF
  exec docker run --rm --platform "$PLATFORM" -e IN_CONTAINER=1 -e ARCH="$ARCH" \
    -v bia-dialog-target:/target -e CARGO_TARGET_DIR=/target -v "bia-cache-$ARCH":/biacache:ro \
    -v "$PWD":/work:ro -w /work "bia-dialog:$ARCH" bash /work/tests/integration/screen-ocr-test.sh
fi

ORT=$(ls /biacache/speech/sherpa-unpack/*/lib/libonnxruntime.so 2>/dev/null | head -1)
[ -n "$ORT" ] || { echo "no onnxruntime in bia-cache-$ARCH (build the image once)"; exit 1; }
export BIA_ONNXRUNTIME=$ORT

FAILED=0
check() { if eval "$2"; then echo "ok   $1"; else echo "FAIL $1"; FAILED=1; fi; }

echo "building..."
cargo build -q -p bia-mcp-screen 2>&1 | tail -3
BIN=/target/debug/bia-mcp-screen

export DISPLAY=:0 LANG=C.UTF-8
Xvfb :0 -screen 0 1280x800x24 >/dev/null 2>&1 &
sleep 1
openbox >/dev/null 2>&1 &
sleep 1

mkdir -p /home/bia/www /home/bia/.config/bia-chrome/Default
cat >/home/bia/www/index.html <<'HTML'
<!doctype html><meta charset=utf-8><title>idle</title>
<style>body{font:20px sans-serif;margin:30px} button{font-size:20px;margin:10px 20px 10px 0;padding:8px 18px}
#night{background:#1f1f1f;color:#eee;padding:12px;width:300px;margin-top:20px}</style>
<h1>bia screen test</h1>
<button id=en onclick="document.title='pressed-en'">Press me</button>
<button id=ru onclick="document.title='pressed-ru'">Нажми меня</button>
<div id=night>Night mode text</div>
HTML
# the page languages Chrome asks sites for: Russian, then English
echo '{"intl":{"accept_languages":"ru,en-US"}}' >/home/bia/.config/bia-chrome/Default/Preferences
chown -R bia:bia /home/bia
(cd /home/bia/www && python3 -m http.server 8000 >/dev/null 2>&1 &)
runuser -u bia -- env DISPLAY=:0 HOME=/home/bia LANG=C.UTF-8 chromium --no-first-run --no-sandbox --test-type \
  --disable-gpu --user-data-dir=/home/bia/.config/bia-chrome --window-position=0,0 --window-size=1200,780 \
  http://127.0.0.1:8000/ >/tmp/chromium.log 2>&1 &
for _ in $(seq 60); do xprop -root _NET_CLIENT_LIST 2>/dev/null | grep -q 0x && break; sleep 0.5; done
sleep 4

browser() { for w in $(xprop -root _NET_CLIENT_LIST | sed 's/.*# //; s/,//g'); do
  xprop -id "$w" WM_WINDOW_ROLE | grep -q '"browser"' && echo "$w"; done | head -1; }
title() { xprop -id "$(browser)" _NET_WM_NAME | sed 's/^[^"]*"//; s/ - Chromium"$//'; }

# One MCP session: each argument is "tool json-args"; prints "== tool" then the result text.
mcp() {
  python3 - "$BIN" "$@" <<'PY'
import json, subprocess, sys
p = subprocess.Popen([sys.argv[1]], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
def send(m): p.stdin.write(json.dumps(m) + "\n"); p.stdin.flush()
def reply(i):
    for line in p.stdout:
        m = json.loads(line)
        if m.get("id") == i: return m
send({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}}})
reply(0)
send({"jsonrpc": "2.0", "method": "notifications/initialized"})
for n, call in enumerate(sys.argv[2:], 1):
    name, _, args = call.partition(" ")
    if name == "list":
        send({"jsonrpc": "2.0", "id": n, "method": "tools/list"})
        print("== list\n" + " ".join(t["name"] for t in reply(n)["result"]["tools"]))
        continue
    send({"jsonrpc": "2.0", "id": n, "method": "tools/call", "params": {"name": name, "arguments": json.loads(args or "{}")}})
    m = reply(n)
    r = m.get("result") or {}
    text = "".join(c.get("text", "") for c in r.get("content", [])) or json.dumps(m.get("error"))
    print(f"== {name}{' ERROR' if r.get('isError') or 'error' in m else ''}\n{text}")
p.stdin.close(); p.wait(timeout=10)
PY
}
# x,y of a word in an ocr result: $1 = file, $2 = word
at() { grep -o "$2 ([0-9]*,[0-9]*)" "$1" | head -1 | tr -dc '0-9,'; }

mcp list >/tmp/list.txt
check "tools: shot and ocr" "grep -qw ocr /tmp/list.txt && grep -qw shot /tmp/list.txt"

mcp 'ocr' >/tmp/ocr.txt
check "ocr: English words" "grep -q 'Press (' /tmp/ocr.txt && grep -q 'screen (' /tmp/ocr.txt"
check "ocr: Russian words (eslav)" "grep -q 'Нажми (' /tmp/ocr.txt"
check "ocr: light text on dark" "grep -q 'Night (' /tmp/ocr.txt"
check "models downloaded as bia's" "[ \"\$(stat -c %U /home/bia/files/ocr/models/eslav_PP-OCRv5_rec_mobile.onnx)\" = bia ]"

PRESS=$(at /tmp/ocr.txt Press); RU=$(at /tmp/ocr.txt Нажми)
mcp "click {\"x\": ${PRESS%,*}, \"y\": ${PRESS#*,}}" >/dev/null; sleep 0.5
check "click at Press's x,y presses Press me" "[ \"\$(title)\" = pressed-en ]"
mcp "click {\"x\": ${RU%,*}, \"y\": ${RU#*,}}" >/dev/null; sleep 0.5
check "click at Нажми's x,y presses Нажми меня" "[ \"\$(title)\" = pressed-ru ]"

# a region around the buttons only: same screen points, no heading
mcp "ocr {\"x\": 0, \"y\": $(( ${PRESS#*,} - 40 )), \"w\": 700, \"h\": 80}" >/tmp/region.txt
RP=$(at /tmp/region.txt Press)
check "region: finds Press" "[ -n '$RP' ]"
check "region: screen coordinates" "[ \$(( ${RP%,*} - ${PRESS%,*} )) -ge -3 ] && [ \$(( ${RP%,*} - ${PRESS%,*} )) -le 3 ] && [ \$(( ${RP#*,} - ${PRESS#*,} )) -ge -3 ] && [ \$(( ${RP#*,} - ${PRESS#*,} )) -le 3 ]"
check "region: nothing outside" "! grep -q 'screen (' /tmp/region.txt"
check "region: partial bounds refused" "mcp 'ocr {\"x\": 5}' | grep -q '== ocr ERROR'"

mcp 'shot' >/tmp/shot.txt
check "shot: picture only" "grep -q '^image: ' /tmp/shot.txt && ! grep -q 'Press (' /tmp/shot.txt"

[ $FAILED = 0 ] && echo "PASS" || { echo "--- ocr"; cat /tmp/ocr.txt; echo "--- region"; cat /tmp/region.txt; echo "FAILED"; }
exit $FAILED
