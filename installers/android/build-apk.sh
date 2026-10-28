#!/usr/bin/env bash
# Signed release APK of bia deploy (sideloading, testing): installers/dist/android/.
# versionCode is the build time in minutes (BIA_VERSION_CODE=n sets it).
set -euo pipefail
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/build-common.sh"
prepare

echo "== Building signed release APK ($VERSION)"
./gradlew assembleRelease

OUTPUT="$ROOT_DIR/app/build/outputs/apk/release/app-release.apk"
[[ -f "$OUTPUT" ]] || die "build finished but APK not found at $OUTPUT"
cp -f "$OUTPUT" "$DIST/bia-deploy-$VERSION.apk"
echo
echo "APK ready:"
ls -lh "$DIST/bia-deploy-$VERSION.apk"
