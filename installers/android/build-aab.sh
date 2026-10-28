#!/usr/bin/env bash
# Signed release app bundle of bia deploy for Google Play (uploaded by hand in Play Console):
# installers/dist/android/. versionCode is the build time in minutes, higher with every
# build (BIA_VERSION_CODE=n sets it).
set -euo pipefail
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/build-common.sh"
prepare

echo "== Building signed release app bundle ($VERSION)"
./gradlew bundleRelease

OUTPUT="$ROOT_DIR/app/build/outputs/bundle/release/app-release.aab"
[[ -f "$OUTPUT" ]] || die "build finished but bundle not found at $OUTPUT"
cp -f "$OUTPUT" "$DIST/bia-deploy-$VERSION.aab"
echo
echo "App bundle ready:"
ls -lh "$DIST/bia-deploy-$VERSION.aab"
