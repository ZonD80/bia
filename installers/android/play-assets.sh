#!/usr/bin/env bash
# Google Play listing assets of bia deploy, into installers/dist/play/:
#   icon-512.png          app icon (from the iOS 1024 px icon)
#   feature-1024x500.png  feature graphic (rendered with headless Chrome, like tools/make-og.sh)
#   phone/, tablet7/, tablet10/  screenshots: each AVD boots headless, the debug build opens
#                         every wizard step with made-up data (Demo.kt, debug builds only)
#
# AVDs (PHONE_AVD, TABLET7_AVD, TABLET10_AVD; 9:16 portrait): losdias-play-phone (1080×1920),
# losdias-play-tablet7 (1080×1920), losdias-play-tablet10 (1440×2560). ONLY=graphics or
# ONLY=screenshots does one half.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$ROOT_DIR/../.." && pwd)"
OUT="$REPO/installers/dist/play"
SDK="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
ADB="$SDK/platform-tools/adb"
EMU="$SDK/emulator/emulator"
CHROME=${CHROME:-"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"}
APP=biadeploy.android
SCENES=(brain login newkey confirm running installed keys)

die() {
    echo "error: $*" >&2
    exit 1
}

graphics() {
    [[ -x "$CHROME" ]] || die "need Google Chrome (set CHROME=…)"
    mkdir -p "$OUT"
    echo "== icon-512.png"
    sips -z 512 512 "$REPO/installers/ios/BiaDeploy/Assets.xcassets/AppIcon.appiconset/icon-1024.png" --out "$OUT/icon-512.png" >/dev/null
    echo "== feature-1024x500.png"
    local tmp
    tmp=$(mktemp -d)
    cp "$REPO/installers/ios/BiaDeploy/Assets.xcassets/AppIcon.appiconset/icon-1024.png" "$tmp/icon.png"
    cat >"$tmp/card.html" <<'HTML'
<!doctype html><html><head><meta charset="utf-8">
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Fraunces:opsz,wght@9..144,600;9..144,700&family=Manrope:wght@500;700&display=block">
<style>
html, body { margin: 0; width: 1024px; height: 500px; overflow: hidden; }
body { background: linear-gradient(135deg, #fbf8f2 0%, #f1e9da 100%); display: flex; align-items: center; gap: 56px; padding: 0 72px; box-sizing: border-box; font-family: Manrope, sans-serif; color: #1d2126; }
img { width: 300px; height: 300px; border-radius: 66px; box-shadow: 0 24px 60px rgba(38, 92, 160, 0.25); flex: none; }
h1 { font-family: Fraunces, serif; font-size: 92px; line-height: 1; margin: 0; font-weight: 700; }
h1 span { color: #265ca0; }
p { font-size: 33px; line-height: 1.3; margin: 22px 0 0; font-weight: 500; max-width: 520px; }
small { display: block; margin-top: 22px; font-size: 22px; color: #6b6f75; font-weight: 700; letter-spacing: 0.02em; }
</style></head><body>
<img src="icon.png" alt="">
<div><h1><span>bia</span> deploy</h1>
<p>Put bia, a helpful AI personality, on a server from your phone.</p>
<small>Any Linux VPS · its key stays with you</small></div>
</body></html>
HTML
    "$CHROME" --headless=new --disable-gpu --hide-scrollbars --force-device-scale-factor=1 --virtual-time-budget=8000 \
        --window-size=1024,500 --screenshot="$tmp/feature.png" "file://$tmp/card.html" >/dev/null 2>&1
    cp "$tmp/feature.png" "$OUT/feature-1024x500.png"
    rm -rf "$tmp"
}

wait_boot() { # serial
    local s=$1 i
    "$ADB" -s "$s" wait-for-device
    for i in $(seq 1 180); do
        [[ "$("$ADB" -s "$s" shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" == 1 ]] && return 0
        sleep 2
    done
    die "$s did not finish booting"
}

# a clean status bar: 10:00, full battery, Wi-Fi with internet, nothing else (sent before each
# shot: the first ones can arrive before System UI listens)
clean_bar() { # serial
    local d=(shell am broadcast -a com.android.systemui.demo -e command)
    "$ADB" -s "$1" "${d[@]}" enter >/dev/null
    "$ADB" -s "$1" "${d[@]}" clock -e hhmm 1000 >/dev/null
    "$ADB" -s "$1" "${d[@]}" battery -e level 100 -e plugged false >/dev/null
    "$ADB" -s "$1" "${d[@]}" network -e wifi show -e level 4 -e fully true >/dev/null
    "$ADB" -s "$1" "${d[@]}" network -e mobile hide -e nosim hide -e airplane hide >/dev/null
    "$ADB" -s "$1" "${d[@]}" notifications -e visible false >/dev/null
    "$ADB" -s "$1" "${d[@]}" status -e volume hide -e bluetooth hide -e location hide -e alarm hide -e sync hide -e tty hide -e eri hide -e mute hide -e speakerphone hide -e managed_profile hide -e zen hide >/dev/null
}

shots() { # avd dir port
    local avd=$1 dir="$OUT/$2" port=$3 s="emulator-$3" scene i=1
    echo "== $2 ($avd)"
    "$EMU" -avd "$avd" -port "$port" -no-window -no-audio -no-boot-anim -no-snapshot -gpu swiftshader_indirect >/dev/null 2>&1 &
    local pid=$!
    wait_boot "$s"
    "$ADB" -s "$s" uninstall "$APP" >/dev/null 2>&1 || true
    "$ADB" -s "$s" install -r "$ROOT_DIR/app/build/outputs/apk/debug/app-debug.apk" >/dev/null
    "$ADB" -s "$s" shell cmd uimode night no >/dev/null
    "$ADB" -s "$s" shell settings put global sysui_demo_allowed 1
    rm -rf "$dir"
    mkdir -p "$dir"
    for scene in "${SCENES[@]}"; do
        "$ADB" -s "$s" shell am start -S -W -n "$APP/.MainActivity" --es demo "$scene" >/dev/null
        clean_bar "$s"
        sleep 4
        "$ADB" -s "$s" exec-out screencap -p >"$dir/$i-$scene.png"
        i=$((i + 1))
    done
    "$ADB" -s "$s" emu kill >/dev/null 2>&1 || true
    wait "$pid" 2>/dev/null || true
}

screenshots() {
    [[ -x "$EMU" ]] || die "Android emulator not found in $SDK"
    pgrep -f qemu-system >/dev/null && die "an emulator or QEMU is already running; stop it first"
    echo "== debug build (demo scenes)"
    (cd "$ROOT_DIR" && . ./build-common.sh && setup_java && setup_sdk && ./gradlew -q assembleDebug)
    shots "${PHONE_AVD:-losdias-play-phone}" phone 5560
    shots "${TABLET7_AVD:-losdias-play-tablet7}" tablet7 5562
    shots "${TABLET10_AVD:-losdias-play-tablet10}" tablet10 5564
}

case "${ONLY:-all}" in
    graphics) graphics ;;
    screenshots) screenshots ;;
    all) graphics; screenshots ;;
    *) die "ONLY is graphics or screenshots" ;;
esac
echo
echo "Play assets in $OUT:"
find "$OUT" -name '*.png' | sort | sed "s|$OUT/|  |"
