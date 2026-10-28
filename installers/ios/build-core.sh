#!/usr/bin/env bash
# The Rust core of bia deploy for iOS (installers/bia-deploy-mobile): static libraries for
# devices and the simulator in BiaDeployCore.xcframework, the Swift that calls them
# (BiaDeploy/Generated), and BiaDeploy.xcodeproj (XcodeGen). Run before opening the project
# in Xcode the first time, and after the core changes. build-ipa.sh runs it too.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$ROOT_DIR/../.." && pwd)"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$REPO/Cargo.toml" | head -1)"
LIB=libbia_deploy_mobile.a

die() {
    echo "error: $*" >&2
    exit 1
}

command -v xcodegen >/dev/null || die "xcodegen missing: brew install xcodegen"
command -v xcodebuild >/dev/null || die "Xcode is needed"
rustup target add aarch64-apple-ios aarch64-apple-ios-sim >/dev/null

echo "== Rust core for iOS devices and the simulator"
cd "$REPO"
export IPHONEOS_DEPLOYMENT_TARGET=16.0
for t in aarch64-apple-ios aarch64-apple-ios-sim; do
    # no build machine's home path in the library (release.sh scans for it)
    RUSTFLAGS="--remap-path-prefix=$HOME=/home --remap-path-prefix=$REPO=/bia" \
        cargo build --release -p bia-deploy-mobile --lib --target "$t"
done

echo "== Swift bindings"
GEN="$ROOT_DIR/build/bindings"
rm -rf "$GEN" "$ROOT_DIR/BiaDeploy/Generated"
cargo run -q -p bia-deploy-mobile --features bindgen --bin uniffi-bindgen -- generate \
    --library "target/aarch64-apple-ios/release/$LIB" --language swift --out-dir "$GEN"
mkdir -p "$ROOT_DIR/BiaDeploy/Generated" "$GEN/headers"
mv "$GEN/bia_deploy_mobile.swift" "$ROOT_DIR/BiaDeploy/Generated/"
mv "$GEN/bia_deploy_mobileFFI.h" "$GEN/headers/"
mv "$GEN/bia_deploy_mobileFFI.modulemap" "$GEN/headers/module.modulemap"

echo "== BiaDeployCore.xcframework"
rm -rf "$ROOT_DIR/BiaDeployCore.xcframework"
xcodebuild -create-xcframework \
    -library "target/aarch64-apple-ios/release/$LIB" -headers "$GEN/headers" \
    -library "target/aarch64-apple-ios-sim/release/$LIB" -headers "$GEN/headers" \
    -output "$ROOT_DIR/BiaDeployCore.xcframework" >/dev/null

echo "== BiaDeploy.xcodeproj ($VERSION)"
cd "$ROOT_DIR"
BIA_VERSION="$VERSION" BIA_BUILD_NUMBER="${BIA_BUILD_NUMBER:-1}" xcodegen generate --quiet
echo "ready: open $ROOT_DIR/BiaDeploy.xcodeproj"
