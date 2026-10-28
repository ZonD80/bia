#!/usr/bin/env bash
# bia deploy for the App Store: the Rust core (build-core.sh), then an archive and an .ipa
# exported for App Store Connect with automatic signing (team A59M2YXS84; Xcode must be
# signed in to that account). Nothing is uploaded: upload installers/dist/ios/*.ipa with
# Xcode's Organizer or Transporter. BIA_BUILD_NUMBER=n sets the build number (default 1;
# raise it for a second upload of the same version).
#
# IOS_UNSIGNED=1 (GitHub Actions, release.sh): no signing at all; the archived app is packed
# as installers/dist/ios/bia-deploy-<version>-unsigned.ipa (sideloading tools sign it).
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$ROOT_DIR/../.." && pwd)"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$REPO/Cargo.toml" | head -1)"
TEAM_ID="${IOS_TEAM_ID:-A59M2YXS84}"
ARCHIVE="$ROOT_DIR/build/BiaDeploy.xcarchive"
DIST="$REPO/installers/dist/ios"

die() {
    echo "error: $*" >&2
    exit 1
}

"$ROOT_DIR/build-core.sh"

SIGN_ARGS=(-allowProvisioningUpdates)
[[ -n "${IOS_UNSIGNED:-}" ]] && SIGN_ARGS=(CODE_SIGNING_ALLOWED=NO CODE_SIGNING_REQUIRED=NO "CODE_SIGN_IDENTITY=")

echo "== Archive ($VERSION, build ${BIA_BUILD_NUMBER:-1})"
rm -rf "$ARCHIVE"
xcodebuild \
    -project "$ROOT_DIR/BiaDeploy.xcodeproj" \
    -scheme BiaDeploy \
    -configuration Release \
    -destination 'generic/platform=iOS' \
    -derivedDataPath "$ROOT_DIR/build/derivedData" \
    -archivePath "$ARCHIVE" \
    "${SIGN_ARGS[@]}" \
    archive

if [[ -n "${IOS_UNSIGNED:-}" ]]; then
    echo "== Unsigned IPA"
    PAY="$ROOT_DIR/build/unsigned"
    rm -rf "$PAY"
    mkdir -p "$PAY/Payload" "$DIST"
    cp -R "$ARCHIVE/Products/Applications/BiaDeploy.app" "$PAY/Payload/"
    rm -f "$DIST/bia-deploy-$VERSION-unsigned.ipa"
    (cd "$PAY" && zip -qry "$DIST/bia-deploy-$VERSION-unsigned.ipa" Payload)
    echo
    echo "Unsigned IPA ready:"
    ls -lh "$DIST/bia-deploy-$VERSION-unsigned.ipa"
    exit 0
fi

PLIST="$(mktemp "${TMPDIR:-/tmp}/bia-export.XXXXXX")"
trap 'rm -f "$PLIST"' EXIT

cat >"$PLIST" <<PL
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>method</key>
	<string>app-store-connect</string>
	<key>destination</key>
	<string>export</string>
	<key>signingStyle</key>
	<string>automatic</string>
	<key>teamID</key>
	<string>${TEAM_ID}</string>
	<key>stripSwiftSymbols</key>
	<true/>
	<key>uploadSymbols</key>
	<true/>
</dict>
</plist>
PL

echo "== Export for App Store Connect"
rm -rf "$ROOT_DIR/build/export"
xcodebuild -exportArchive \
    -archivePath "$ARCHIVE" \
    -exportPath "$ROOT_DIR/build/export" \
    -exportOptionsPlist "$PLIST" \
    -allowProvisioningUpdates

IPA="$(find "$ROOT_DIR/build/export" -maxdepth 1 -name '*.ipa' | head -1)"
[[ -n "$IPA" ]] || die "export finished but no .ipa in $ROOT_DIR/build/export"
mkdir -p "$DIST"
cp -f "$IPA" "$DIST/bia-deploy-$VERSION.ipa"
echo
echo "IPA ready (upload it with Xcode's Organizer or Transporter):"
ls -lh "$DIST/bia-deploy-$VERSION.ipa"
