#!/usr/bin/env bash
# App Store screenshots of bia deploy, into installers/dist/appstore/<device>/: a simulator of
# each size boots (made fresh, deleted after), the debug build opens the saved bias and every wizard step with
# made-up data (Demo.swift, debug builds only: launched with -demo <scene>). The chat scenes (chat, call,
# attach) show bia's real web app with a made-up conversation from installers/demo-chat/server.py.
#
#   iphone-6.3   iPhone 17 Pro (1206×2622; App Store's 6.1"/6.3" slot)
#   iphone-duo   iPhone Duo
#   ipad-13      iPad Pro 13-inch (M5) (2064×2752)
#
# RUNTIME=<simctl runtime id> picks the iOS version (default: the newest installed); a device
# that needs a newer one is skipped with a note. ONLY="ipad-13 …" makes only those devices,
# PICK="chat call …" only those scenes (the others' screenshots stay).
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$ROOT_DIR/../.." && pwd)"
OUT="$REPO/installers/dist/appstore"
APP_ID=biadeploy.ios
APP="$ROOT_DIR/build/derivedData/Build/Products/Debug-iphonesimulator/BiaDeploy.app"
SCENES=(keys brain login newkey confirm running installed chat call attach)
CHAT_SCENES=" chat call attach "
DEMO_PORT=18443
DEVICES=(
    "iphone-6.3|com.apple.CoreSimulator.SimDeviceType.iPhone-17-Pro"
    "iphone-duo|com.apple.CoreSimulator.SimDeviceType.iPhone-Duo"
    "ipad-13|com.apple.CoreSimulator.SimDeviceType.iPad-Pro-13-inch-M5-12GB"
)

die() {
    echo "error: $*" >&2
    exit 1
}

RUNTIME=${RUNTIME:-$(xcrun simctl list runtimes -j | python3 -c '
import json, sys
rs = [r for r in json.load(sys.stdin)["runtimes"] if r.get("platform") == "iOS" and r.get("isAvailable")]
rs.sort(key=lambda r: [int(x) for x in r["version"].split(".")])
print(rs[-1]["identifier"] if rs else "")')}
[[ -n "$RUNTIME" ]] || die "no iOS simulator runtime installed (Xcode › Settings › Components)"

echo "== debug build (demo scenes)"
"$ROOT_DIR/build-core.sh"
xcodebuild -project "$ROOT_DIR/BiaDeploy.xcodeproj" -scheme BiaDeploy -sdk iphonesimulator -configuration Debug \
    -derivedDataPath "$ROOT_DIR/build/derivedData" CODE_SIGNING_ALLOWED=NO -quiet build
[[ -d "$APP" ]] || die "no app at $APP"

echo "== demo chat (localhost:$DEMO_PORT)"
DEMO_KEYS=$(mktemp -d)
python3 "$REPO/installers/demo-chat/server.py" "$DEMO_KEYS" "$DEMO_PORT" >"$DEMO_KEYS/log" 2>&1 &
DEMO_PID=$!
for _ in $(seq 1 50); do grep -q READY "$DEMO_KEYS/log" 2>/dev/null && break; sleep 0.2; done
grep -q READY "$DEMO_KEYS/log" || { cat "$DEMO_KEYS/log" >&2; kill "$DEMO_PID" 2>/dev/null; die "the demo chat did not start"; }
stop_demo() { kill "$DEMO_PID" 2>/dev/null || true; rm -rf "$DEMO_KEYS"; }
trap stop_demo EXIT

for entry in "${DEVICES[@]}"; do
    name=${entry%%|*}
    type=${entry#*|}
    if [[ -n "${ONLY:-}" && " $ONLY " != *" $name "* ]]; then
        continue
    fi
    # a device newer than the runtime (iPhone Duo needs iOS 27.1): say so and go on
    need=$(xcrun simctl list -j devicetypes | python3 -c '
import json, sys
t = [d for d in json.load(sys.stdin)["devicetypes"] if d["identifier"] == sys.argv[1]]
print(t[0].get("minRuntimeVersionString", "") if t else "missing")' "$type")
    have=$(xcrun simctl list -j runtimes | python3 -c '
import json, sys
print(next((r["version"] for r in json.load(sys.stdin)["runtimes"] if r["identifier"] == sys.argv[1]), ""))' "$RUNTIME")
    if [[ "$need" == missing ]] || ! python3 -c 'import sys; v = lambda s: [int(x) for x in s.split(".")]; sys.exit(0 if v(sys.argv[2]) >= v(sys.argv[1]) else 1)' "$need" "$have"; then
        echo "== $name skipped: $type needs iOS ${need} or newer, this runtime is $have (install it: Xcode › Settings › Components)"
        continue
    fi
    # a new enough runtime may still lack the device (iOS 27.2 has no iPhone Duo): its own list decides
    if ! xcrun simctl list -j runtimes | python3 -c '
import json, sys
r = next((r for r in json.load(sys.stdin)["runtimes"] if r["identifier"] == sys.argv[1]), {})
sys.exit(0 if any(t["identifier"] == sys.argv[2] for t in r.get("supportedDeviceTypes", [])) else 1)' "$RUNTIME" "$type"; then
        echo "== $name skipped: runtime $RUNTIME does not support $type (another runtime may: RUNTIME=…)"
        continue
    fi
    echo "== $name ($type, $RUNTIME)"
    udid=$(xcrun simctl create "bia-shots-$name" "$type" "$RUNTIME")
    trap 'xcrun simctl shutdown "$udid" >/dev/null 2>&1 || true; xcrun simctl delete "$udid" >/dev/null 2>&1 || true; stop_demo' EXIT
    xcrun simctl boot "$udid"
    xcrun simctl bootstatus "$udid" -b >/dev/null
    xcrun simctl ui "$udid" appearance light
    # the status bar App Store screenshots usually show
    xcrun simctl status_bar "$udid" override --time 9:41 --dataNetwork wifi --wifiMode active --wifiBars 3 \
        --cellularMode active --cellularBars 4 --operatorName "" --batteryState charged --batteryLevel 100
    xcrun simctl install "$udid" "$APP"
    dir="$OUT/$name"
    [[ -n "${PICK:-}" ]] || rm -rf "$dir"
    mkdir -p "$dir"
    i=0
    warm=
    for scene in "${SCENES[@]}"; do
        i=$((i + 1))
        [[ -z "${PICK:-}" || " $PICK " == *" $scene "* ]] || continue
        xcrun simctl terminate "$udid" "$APP_ID" >/dev/null 2>&1 || true
        wait=4
        if [[ "$CHAT_SCENES" == *" $scene "* ]]; then
            # the demo chat's keys, where the app keeps a bia's (after the wizard scenes: keys lists them)
            keys="$(xcrun simctl get_app_container "$udid" "$APP_ID" data)/Library/Application Support/bia-deploy/servers/localhost:$DEMO_PORT"
            mkdir -p "$keys"
            cp "$DEMO_KEYS/ca.crt" "$DEMO_KEYS/bia-root.p12" "$keys/"
            # the first page load of a fresh device is slow
            if [[ -z "${warm:-}" ]]; then wait=25; warm=1; else wait=12; fi
        fi
        xcrun simctl launch "$udid" "$APP_ID" -demo "$scene" >/dev/null
        sleep "$wait"
        xcrun simctl io "$udid" screenshot --type=png "$dir/$i-$scene.png" >/dev/null 2>&1
    done
    xcrun simctl shutdown "$udid" >/dev/null 2>&1 || true
    xcrun simctl delete "$udid" >/dev/null 2>&1 || true
    trap stop_demo EXIT
done

echo
echo "App Store screenshots in $OUT:"
for f in "$OUT"/*/*.png; do
    printf '  %s  %s\n' "${f#$OUT/}" "$(sips -g pixelWidth -g pixelHeight "$f" | awk '/pixel/ {printf "%s ", $2}')"
done
