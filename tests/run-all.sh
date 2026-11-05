#!/bin/bash
# Run the bia test suite and print a summary. Each test logs to dist/test-logs/<name>.log;
# a failure does not stop the others.
#
#   tests/run-all.sh [stage…]
#     unit         cargo test for the whole workspace (in bia-build:$ARCH)
#     integration  tests/integration: web MCP over HTTPS + mTLS, web markdown (node, on the host),
#                  bia-update internet staging
#     e2e          QEMU on $ARCH (default arm64): update (internet disk flash rescue fsck twin), peer,
#                  land, img (UEFI)
#     takeover     QEMU on $ARCH: an Ubuntu cloud VM taken over by takeover.sh (DHCP, a static /32
#                  with an on-link gateway, land mode with a second bia flying in, and a bia that
#                  takes the server over itself over ssh and flies there)
#     bios         QEMU on x86_64: img (BIOS), bios install, bios update, takeover (UEFI, BIOS)
#     (no stage = unit integration e2e takeover)
#
# Needs a current build for the QEMU stages: make build img iso ARCH=… (and for bios:
# ARCH=x86_64). ./fmm.json gives the test agents a cloud model (peer and land need one).
# Env: ARCH (arm64), ONLY="name …" to run a subset of the e2e tests.
set -uo pipefail
cd "$(dirname "$0")/.."
ROOT=$(pwd)
ARCH=${ARCH:-arm64}
LOGS=$ROOT/dist/test-logs
mkdir -p "$LOGS"
case $ARCH in arm64) PLATFORM=linux/arm64 ;; x86_64) PLATFORM=linux/amd64 ;; *) echo "ARCH=$ARCH" >&2; exit 2 ;; esac
[ $# -gt 0 ] || set -- unit integration e2e takeover

RESULTS=()
run() {
  local name=$1 t0 rc
  shift
  if [ -n "${ONLY:-}" ] && [[ " $ONLY " != *" $name "* ]]; then
    return 0
  fi
  t0=$(date +%s)
  printf '== %-26s ' "$name"
  "$@" > "$LOGS/$name.log" 2>&1
  rc=$?
  local secs=$(( $(date +%s) - t0 ))
  if [ $rc -eq 0 ]; then
    printf 'PASS  (%dm%02ds)\n' $((secs / 60)) $((secs % 60))
    RESULTS+=("PASS $name")
  else
    printf 'FAIL  (%dm%02ds)  see %s\n' $((secs / 60)) $((secs % 60)) "${LOGS#$ROOT/}/$name.log"
    RESULTS+=("FAIL $name")
  fi
}

builder() {
  docker run --rm --platform "$PLATFORM" -v "$ROOT":/work -v "bia-cache-$ARCH":/cache -w /work \
    -e CARGO_HOME=/cache/cargo -e CARGO_TARGET_DIR="/cache/target-$ARCH-test" \
    -e BIA_TG_API_ID=1 -e BIA_TG_API_HASH=test "bia-build:$ARCH" bash -lc "$1"
}

unit() {
  # the telegram MCP links libntgcalls at run time (built by builders/build.sh)
  builder 'L=$(dirname "$(find /cache/ntgcalls-dylib-* -name "libntgcalls.so*" 2>/dev/null | head -1)" 2>/dev/null);
           export NTGCALLS_LIB_DIR=/cache/ntgcalls-dylib-'"$ARCH"' NTGCALLS_DYLIB=1 LD_LIBRARY_PATH=${L:-};
           cargo test --workspace 2>&1 | grep -E "^test |^test result|panicked|^error" ; exit ${PIPESTATUS[0]}'
}

web_mcp() {
  builder 'cargo build -q -p bia-mcp-web && python3 -I tests/integration/web-mcp-e2e.py "$CARGO_TARGET_DIR/debug/bia-mcp-web"'
}

# two bias on the web bus: trust, messages and files both ways, store and forward, roles, bans, a copy refused
peer_link() {
  builder 'cargo build -q -p bia-mcp-web && python3 -I tests/integration/bia-link-e2e.py "$CARGO_TARGET_DIR/debug/bia-mcp-web"'
}

# bia-bridge (CLI, daemon, MCP mode) against the real web MCP
bridge() {
  builder 'cargo build -q -p bia-mcp-web -p bia-bridge && python3 -I tests/integration/bridge-e2e.py "$CARGO_TARGET_DIR/debug/bia-mcp-web" "$CARGO_TARGET_DIR/debug/bia-bridge"'
}

# messenger + instagram MCPs with the real bia-meta, no Meta account (builds bia-meta with the cached Go)
meta_mcp() {
  builder 'ARCH='"$ARCH"'; . /work/builders/go-env.sh; ensure_go && OUT=/tmp/meta-e2e && mkdir -p $OUT &&
           (cd source/go/bia-meta && CGO_ENABLED=0 go test ./... && CGO_ENABLED=0 go build -o $OUT/bia-meta .) &&
           cargo build -q -p bia-mcp-meta && python3 -I tests/integration/meta-mcp-e2e.py "$CARGO_TARGET_DIR/debug" $OUT/bia-meta'
}

# max MCP with the real bia_max.py and PyMax against a fake MAX server, on the guest's Python (trixie)
max_mcp() {
  builder 'cargo build -q -p bia-mcp-meta' || return 1
  docker run --rm --platform "$PLATFORM" -v "$ROOT":/work:ro -v "bia-cache-$ARCH":/cache -w /work debian:trixie-slim bash -c \
    "apt-get update -qq >/dev/null && apt-get install -y -qq --no-install-recommends python3 python3-venv python3-gi gir1.2-gstreamer-1.0 gir1.2-gst-plugins-base-1.0 gir1.2-gst-plugins-bad-1.0 gstreamer1.0-plugins-base gstreamer1.0-plugins-good gstreamer1.0-plugins-bad gstreamer1.0-nice >/dev/null 2>&1 &&
     python3 -m venv /tmp/v && /tmp/v/bin/pip install -q --no-deps --require-hashes -r builders/max-requirements.txt &&
     BIA_MAX_SITE=\$(/tmp/v/bin/python -c 'import site; print(site.getsitepackages()[0])') python3 -I tests/integration/max-mcp-e2e.py /cache/target-$ARCH-test/debug/bia-mcp-max source/flash/MCPs/max/bia_max.py"
}

# .biaconfig: the real web MCP makes one, the real chrome MCP opens it into a real NSS database
biaconfig() {
  builder 'cargo build -q -p bia-mcp-chrome -p bia-mcp-web' || return 1
  docker run --rm --platform "$PLATFORM" -v "$ROOT":/work:ro -v "bia-cache-$ARCH":/cache -w /work debian:trixie-slim bash -c \
    "apt-get update -qq >/dev/null && apt-get install -y -qq --no-install-recommends python3 libnss3-tools openssl ca-certificates curl util-linux procps >/dev/null && \
     python3 -I tests/integration/biaconfig-e2e.py /cache/target-$ARCH-test/debug/bia-mcp-web /cache/target-$ARCH-test/debug/bia-mcp-chrome source/flash/MCPs/chrome"
}

# client certificates: the real chrome MCP and Chromium against an HTTPS server that demands one
chrome_cert() {
  builder 'cargo build -q -p bia-mcp-chrome' || return 1
  docker run --rm --platform "$PLATFORM" -v "$ROOT":/work:ro -v "bia-cache-$ARCH":/cache -w /work debian:trixie-slim bash -c \
    "apt-get update -qq >/dev/null && apt-get install -y -qq --no-install-recommends python3 python3-gi gir1.2-atspi-2.0 at-spi2-core dbus-daemon xvfb openbox chromium chromium-l10n libnss3-tools openssl ca-certificates curl wmctrl xclip xdotool util-linux procps fonts-dejavu-core >/dev/null 2>&1 &&
     python3 -I tests/integration/chrome-cert-e2e.py /cache/target-$ARCH-test/debug/bia-mcp-chrome source/flash/MCPs/chrome source/flash/MCPs/screen/bia_desktop.py"
}

# passkey approval: the real extension in Chromium; nothing is signed until the model approves
chrome_passkey() {
  builder 'cargo build -q -p bia-mcp-chrome' || return 1
  docker run --rm --platform "$PLATFORM" -v "$ROOT":/work:ro -v "bia-cache-$ARCH":/cache -w /work debian:trixie-slim bash -c \
    "apt-get update -qq >/dev/null && apt-get install -y -qq --no-install-recommends python3 dbus-daemon xvfb openbox chromium libnss3-tools openssl ca-certificates curl wmctrl xdotool util-linux procps fonts-dejavu-core >/dev/null 2>&1 &&
     python3 -I tests/integration/chrome-passkey-e2e.py /cache/target-$ARCH-test/debug/bia-mcp-chrome source/flash/MCPs/chrome"
}

# an installed add-on moves to a new version by itself (takes minutes: Chrome's own update check)
chrome_update() {
  builder 'cargo build -q -p bia-mcp-chrome' || return 1
  docker run --rm --platform "$PLATFORM" -v "$ROOT":/work:ro -v "bia-cache-$ARCH":/cache -w /work debian:trixie-slim bash -c \
    "apt-get update -qq >/dev/null && apt-get install -y -qq --no-install-recommends python3 dbus-daemon xvfb openbox chromium libnss3-tools openssl ca-certificates curl wmctrl xdotool util-linux procps fonts-dejavu-core >/dev/null 2>&1 &&
     python3 -I tests/integration/chrome-update-e2e.py /cache/target-$ARCH-test/debug/bia-mcp-chrome source/flash/MCPs/chrome"
}

# bia's default venv, and an MCP's own venv next to it
venv_default() {
  docker run --rm --platform "$PLATFORM" -v "$ROOT":/work:ro debian:trixie-slim bash /work/tests/integration/venv-e2e.sh
}

# the screen MCP's desktop tools: Xvfb, Openbox, AT-SPI and a real GTK app, on the guest's Debian
screen_desktop() {
  builder 'cargo build -q -p bia-mcp-screen' || return 1
  docker run --rm --platform "$PLATFORM" -v "$ROOT":/work:ro -v "bia-cache-$ARCH":/cache -w /work debian:trixie-slim bash -c \
    "apt-get update -qq >/dev/null && apt-get install -y -qq --no-install-recommends python3 python3-gi gir1.2-atspi-2.0 at-spi2-core dbus-daemon xvfb openbox mousepad wmctrl xclip xdotool scrot util-linux >/dev/null 2>&1 &&
     python3 -I tests/integration/screen-desktop-e2e.py /cache/target-$ARCH-test/debug/bia-mcp-screen source/flash/MCPs/screen/bia_desktop.py"
}

# ignored chats: rules, unread and wake-ups through the messenger MCP with a fake bia-meta
ignore_mcp() {
  builder 'cargo build -q -p bia-mcp-meta && python3 -I tests/integration/ignore-e2e.py "$CARGO_TARGET_DIR/debug"'
}

for stage in "$@"; do
  case $stage in
    unit)
      run unit-cargo unit ;;
    integration)
      run web-mcp-https web_mcp
      run peer-link peer_link
      run bia-bridge bridge
      run meta-mcp meta_mcp
      run ignore-mcp ignore_mcp
      run max-mcp max_mcp
      run screen-desktop screen_desktop
      run chrome-cert chrome_cert
      run chrome-passkey chrome_passkey
      run chrome-update chrome_update
      run venv venv_default
      run biaconfig biaconfig
      run release-publish bash tests/integration/release-publish-test.sh
      run web-markdown node tests/integration/web-md-test.js
      run web-link node tests/integration/web-link-test.js
      # the web app in headless Chrome, on this host (skips when there is no Chrome here)
      run web-app bash -c 'cargo build -q -p bia-mcp-web && python3 -I tests/integration/web-app-e2e.py target/debug/bia-mcp-web'
      run update-internet-stage env ARCH="$ARCH" tests/integration/update-net-test.sh ;;
    e2e)
      run update-e2e env ARCH="$ARCH" bash -c 'tests/e2e/update-e2e.sh images && tests/e2e/update-e2e.sh patch && tests/e2e/update-e2e.sh all internet disk flash rescue fsck twin'
      run peer-e2e env ARCH="$ARCH" tests/e2e/peer-e2e.sh all
      run land-e2e env ARCH="$ARCH" tests/e2e/land-e2e.sh all
      run img-e2e-uefi env ARCH="$ARCH" FIRMWARE=uefi tests/e2e/img-e2e.sh all ;;
    takeover)
      run takeover-e2e env ARCH="$ARCH" tests/e2e/takeover-e2e.sh all
      run takeover-onlink env ARCH="$ARCH" NET=onlink tests/e2e/takeover-e2e.sh run
      run takeover-land env ARCH="$ARCH" LAND=1 tests/e2e/takeover-e2e.sh all
      run takeover-agent env ARCH="$ARCH" LAND=agent tests/e2e/takeover-e2e.sh all ;;
    bios)
      run img-e2e-bios env ARCH=x86_64 FIRMWARE=bios tests/e2e/img-e2e.sh all
      run bios-e2e env ARCH=x86_64 tests/e2e/bios-e2e.sh all
      run bios-update-e2e env ARCH=x86_64 bash -c 'tests/e2e/update-e2e.sh images && tests/e2e/update-e2e.sh patch && python3 tests/e2e/bios-update-e2e.py'
      run takeover-x86-uefi env ARCH=x86_64 MEM=6G tests/e2e/takeover-e2e.sh all
      run takeover-x86-bios env ARCH=x86_64 MEM=6G FIRMWARE=bios tests/e2e/takeover-e2e.sh run ;;
    *) echo "unknown stage $stage (unit integration e2e takeover bios)" >&2; exit 2 ;;
  esac
done

echo
fails=0
for r in "${RESULTS[@]}"; do
  echo "  $r"
  [[ $r == FAIL* ]] && fails=$((fails + 1))
done
echo "== ${#RESULTS[@]} run, $fails failed"
[ $fails -eq 0 ]
