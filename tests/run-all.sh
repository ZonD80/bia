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

# live peer links between web MCPs: trust, messages, files, disconnect, bans, bye, a copy refused
peer_link() {
  builder 'cargo build -q -p bia-mcp-web && python3 -I tests/integration/peer-link-e2e.py "$CARGO_TARGET_DIR/debug/bia-mcp-web"'
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
      run web-markdown node tests/integration/web-md-test.js
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
