#!/usr/bin/env bash
# App Store screenshots of bia deploy, into installers/dist/appstore/<device>/: a simulator of
# each size boots (made fresh, deleted after), the debug build opens every wizard step with
# made-up data (Demo.swift, debug builds only: launched with -demo <scene>).
#
#   iphone-6.3   iPhone 17 Pro (1206×2622; App Store's 6.1"/6.3" slot)
#   iphone-duo   iPhone Duo
#   ipad-13      iPad Pro 13-inch (M5) (2064×2752)
#
# RUNTIME=<simctl runtime id> picks the iOS version (default: the newest installed); a device
# that needs a newer one is skipped with a note. ONLY="ipad-13 …" makes only those.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$ROOT_DIR/../.." && pwd)"
OUT="$REPO/installers/dist/appstore"
APP_ID=biadeploy.ios
APP="$ROOT_DIR/build/derivedData/Build/Products/Debug-iphonesimulator/BiaDeploy.app"
SCENES=(brain login newkey confirm running installed keys)
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
    echo "== $name ($type, $RUNTIME)"
    udid=$(xcrun simctl create "bia-shots-$name" "$type" "$RUNTIME")
    trap 'xcrun simctl shutdown "$udid" >/dev/null 2>&1 || true; xcrun simctl delete "$udid" >/dev/null 2>&1 || true' EXIT
    xcrun simctl boot "$udid"
    xcrun simctl bootstatus "$udid" -b >/dev/null
    xcrun simctl ui "$udid" appearance light
    # the status bar App Store screenshots usually show
    xcrun simctl status_bar "$udid" override --time 9:41 --dataNetwork wifi --wifiMode active --wifiBars 3 \
        --cellularMode active --cellularBars 4 --operatorName "" --batteryState charged --batteryLevel 100
    xcrun simctl install "$udid" "$APP"
    dir="$OUT/$name"
    rm -rf "$dir"
    mkdir -p "$dir"
    i=1
    for scene in "${SCENES[@]}"; do
        xcrun simctl terminate "$udid" "$APP_ID" >/dev/null 2>&1 || true
        xcrun simctl launch "$udid" "$APP_ID" -demo "$scene" >/dev/null
        sleep 4
        xcrun simctl io "$udid" screenshot --type=png "$dir/$i-$scene.png" >/dev/null 2>&1
        i=$((i + 1))
    done
    xcrun simctl shutdown "$udid" >/dev/null 2>&1 || true
    xcrun simctl delete "$udid" >/dev/null 2>&1 || true
    trap - EXIT
done

echo
echo "App Store screenshots in $OUT:"
for f in "$OUT"/*/*.png; do
    printf '  %s  %s\n' "${f#$OUT/}" "$(sips -g pixelWidth -g pixelHeight "$f" | awk '/pixel/ {printf "%s ", $2}')"
done
