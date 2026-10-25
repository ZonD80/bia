#!/bin/bash
# Container test of bia-mcp-chrome's file dialog driver (dialog.rs, no QEMU): Xvfb + openbox +
# Chromium (same GTK dialog as Google Chrome) as user bia, a page with single, multiple and
# folder inputs, and the real choose() via the ignored choose_from_env test. Covers a
# completion-popup trap, a non-ASCII name, multi-select (first refused because the GTK
# bookmarks file did not exist when the dialog opened), folder upload with Chrome's
# "Upload N files?" prompt confirmed by the tool (over CDP), cancel and Save. Also the
# DefaultAutomaticDownloadsSetting policy: several downloads from one click, no prompt.
#
#   tests/integration/chrome-dialog-test.sh            (runs itself in bia-dialog:$ARCH)
set -uo pipefail
ARCH=${ARCH:-arm64}
case $ARCH in arm64) PLATFORM=linux/arm64 ;; x86_64) PLATFORM=linux/amd64 ;; *) exit 2 ;; esac
if [ "${IN_CONTAINER:-}" != 1 ]; then
  cd "$(dirname "$0")/../.."
  docker build -q --platform "$PLATFORM" -t "bia-dialog:$ARCH" - >/dev/null <<EOF || exit 1
FROM bia-build:$ARCH
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    xvfb openbox xdotool x11-utils scrot chromium fonts-dejavu-core python3 \
  && rm -rf /var/lib/apt/lists/* && (id bia || useradd -m bia)
EOF
  exec docker run --rm --platform "$PLATFORM" -e IN_CONTAINER=1 -e ARCH="$ARCH" \
    -v bia-dialog-target:/target -e CARGO_TARGET_DIR=/target \
    -v "$PWD":/work:ro -w /work "bia-dialog:$ARCH" bash /work/tests/integration/chrome-dialog-test.sh
fi

FAILED=0
check() { if eval "$2"; then echo "ok   $1"; else echo "FAIL $1"; FAILED=1; fi; }

echo "building..."
cargo test -q -p bia-mcp-chrome --no-run 2>&1 | tail -3
run_choose() { # choose_from_env with BIA_DIALOG_* set by the caller; prints choose's result
  { cargo test -q -p bia-mcp-chrome choose_from_env -- --ignored --nocapture 2>&1 || true; } | grep '^choose:' | tee -a /tmp/choose.log
}

export DISPLAY=:0 LANG=C.UTF-8
Xvfb :0 -screen 0 1280x800x24 >/dev/null 2>&1 &
sleep 1
openbox >/dev/null 2>&1 &
sleep 1

F=/home/bia/files/up
mkdir -p "$F/dir" /home/bia/www
echo a >"$F/a.txt"; echo trap >"$F/a.txt.bak"; echo bb >"$F/b.txt"; echo ccc >"$F/ü ß.txt"
echo d1 >"$F/dir/one.txt"; echo d22 >"$F/dir/two.txt"
cat >/home/bia/www/index.html <<'HTML'
<!doctype html><title>idle</title>
<style>input{position:absolute;left:10px;font-size:24px}</style>
<input id=s type=file style="top:10px">
<input id=m type=file multiple style="top:80px">
<input id=d type=file webkitdirectory style="top:150px">
<button id=many style="position:absolute;top:220px;left:10px;font-size:24px">many</button>
<script>
for (const i of document.querySelectorAll('input')) {
  i.onchange = () => document.title = i.id + ':' + [...i.files].map(f => (f.webkitRelativePath || f.name) + '=' + f.size).sort().join(',');
  i.oncancel = () => document.title = i.id + ':cancel';
}
many.onclick = () => { for (let i = 0; i < 4; i++) { const a = document.createElement('a');
  a.href = URL.createObjectURL(new Blob(['x' + i])); a.download = 'many' + i + '.txt'; a.click(); } };
</script>
HTML
# what install.rs writes besides the extension entries
mkdir -p /etc/chromium/policies/managed
echo '{"DefaultAutomaticDownloadsSetting": 1}' >/etc/chromium/policies/managed/bia-chrome.json
chown -R bia:bia /home/bia
(cd /home/bia/www && python3 -m http.server 8000 >/dev/null 2>&1 &)
runuser -u bia -- env DISPLAY=:0 HOME=/home/bia LANG=C.UTF-8 chromium --no-first-run --no-sandbox --test-type \
  --disable-gpu --remote-debugging-port=9222 --user-data-dir=/home/bia/prof --window-position=0,0 --window-size=1200,780 \
  http://127.0.0.1:8000/ >/tmp/chromium.log 2>&1 &
for _ in $(seq 60); do xprop -root _NET_CLIENT_LIST 2>/dev/null | grep -q 0x && break; sleep 0.5; done
sleep 4

browser() { for w in $(xprop -root _NET_CLIENT_LIST | sed 's/.*# //; s/,//g'); do
  xprop -id "$w" WM_WINDOW_ROLE | grep -q '"browser"' && echo "$w"; done | head -1; }
dialog_open() { for w in $(xprop -root _NET_CLIENT_LIST | sed 's/.*# //; s/,//g'); do
  xprop -id "$w" WM_WINDOW_ROLE | grep -q GtkFileChooserDialog && return 0; done; return 1; }
title() { xprop -id "$(browser)" _NET_WM_NAME | sed 's/^[^"]*"//; s/ - Chromium"$//'; }
# page y of the input + the window's content offset
CONTENT_Y=$(( $(xwininfo -id "$(browser)" | awk '/Absolute upper-left Y/{print $4}') + 88 ))
open_input() { # $1 = page y of the input
  xdotool mousemove 60 $((CONTENT_Y + $1 + 15)) click 1
  for _ in $(seq 20); do dialog_open && return 0; sleep 0.25; done; return 1
}

check "no dialog: clear error" "BIA_DIALOG_PATHS=$F/a.txt run_choose | grep -q 'No file dialog is open'"

check "single dialog opens" "open_input 10"
check "watcher sees one dialog" "cargo test -q -p bia-mcp-chrome seen_from_env -- --ignored --nocapture 2>&1 | grep -q 'seen: 1 dialog(s).*Open File for idle'"
check "single with a completion trap" "BIA_DIALOG_PATHS=$F/a.txt run_choose | grep -q Ok"
sleep 0.5; check "page got a.txt" "[ \"\$(title)\" = 's:a.txt=2' ]"

open_input 10
check "single non-ASCII name" "BIA_DIALOG_PATHS='$F/ü ß.txt' run_choose | grep -q Ok"
sleep 0.5; check "page got ü ß.txt" "[ \"\$(title)\" = 's:ü ß.txt=4' ]"

open_input 80
check "multi before bookmarks exist: refused" "BIA_DIALOG_PATHS=$F/a.txt:$F/b.txt run_choose | grep -q 'cancel=true'"
check "bookmarks file now exists" "[ -f /home/bia/.config/gtk-3.0/bookmarks ]"
check "cancel" "BIA_DIALOG_CANCEL=1 BIA_DIALOG_PATHS=x run_choose | grep -q Ok"
sleep 0.5; check "page saw cancel" "[ \"\$(title)\" = 'm:cancel' ]"

open_input 80
check "multi" "BIA_DIALOG_PATHS=$F/a.txt:$F/b.txt:'$F/ü ß.txt' run_choose | grep -q Ok"
sleep 0.5; check "page got 3 files" "[ \"\$(title)\" = 'm:a.txt=2,b.txt=3,ü ß.txt=4' ]"
check "staged as bia" "[ \"\$(stat -c %U /home/bia/files/state/chrome/pick/*/a.txt | head -1)\" = bia ]"

open_input 150
check "folder, Chrome's upload prompt confirmed" "BIA_DIALOG_PATHS=$F/dir run_choose | grep -q 'confirmed Chrome.*Upload 2 files'"
sleep 0.5
check "page got the folder" "[ \"\$(title)\" = 'd:dir/one.txt=3,dir/two.txt=4' ]"

open_input 10
check "unreadable file refused" "chmod 600 $F/b.txt; chown root $F/b.txt; BIA_DIALOG_PATHS=$F/b.txt run_choose | grep -q 'cannot read'"
check "relative path refused" "BIA_DIALOG_PATHS=up/a.txt run_choose | grep -q 'not an absolute path'"
BIA_DIALOG_CANCEL=1 BIA_DIALOG_PATHS=x run_choose >/dev/null

xdotool mousemove 60 $((CONTENT_Y + 235)) click 1; sleep 3
check "4 downloads from one click, no prompt" "[ \"\$(ls /home/bia/Downloads/many*.txt 2>/dev/null | wc -l)\" = 4 ]"

xdotool mousemove 600 500 click 1 key ctrl+s
for _ in $(seq 20); do dialog_open && break; sleep 0.25; done
check "save" "BIA_DIALOG_PATHS=$F/saved.html run_choose | grep -q Ok"
sleep 2; check "saved file exists" "[ -s $F/saved.html ]"

[ $FAILED = 0 ] && echo "PASS" || { echo "--- choose results"; cat /tmp/choose.log; echo "FAILED"; }
exit $FAILED
