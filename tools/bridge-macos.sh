#!/usr/bin/env bash
# bia-bridge for macOS (one universal binary) into source/flash/MCPs/web/bridge/, where the
# bia serves it to agents next to the Linux and Windows builds (builders/build.sh).
# Signed with the bia Developer ID (hardened runtime) and notarized, like bia-deploy:
# credentials from the repo-root .env (APPLE_ID, APPLE_APP_SPECIFIC_PASSWORD; APPLE_TEAM_ID
# and CODESIGN_IDENTITY default to installers/macos/release.sh's). A bare binary cannot be
# stapled; Gatekeeper finds its ticket online. The notarized build's sha256 goes to
# bia-bridge-macos.notarized, which tools/release.sh checks.
#   tools/bridge-macos.sh [--no-notarize]    sign only (quick local builds)
#   BRIDGE_ADHOC=1 tools/bridge-macos.sh     no Developer ID at all (machines without it)
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$(pwd)
NOTARIZE=1
[[ "${1:-}" == "--no-notarize" ]] && NOTARIZE=0
[[ "$(uname -s)" == Darwin ]] || { echo "tools/bridge-macos.sh: needs macOS" >&2; exit 1; }
for f in "$ROOT/.env" "$ROOT/installers/macos/.env"; do
  if [[ -f "$f" ]]; then set -a; source "$f"; set +a; fi
done
MAC=installers/macos/release.sh
APPLE_TEAM_ID="${APPLE_TEAM_ID:-$(sed -n 's/^DEFAULT_TEAM_ID="\(.*\)"/\1/p' "$MAC")}"
CODESIGN_IDENTITY="${CODESIGN_IDENTITY:-$(sed -n 's/^DEFAULT_CODESIGN_IDENTITY="\(.*\)"/\1/p' "$MAC")}"

export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$ROOT=/bia --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/cargo --remap-path-prefix=${RUSTUP_HOME:-$HOME/.rustup}=/rustup"
export CARGO_PROFILE_RELEASE_DEBUG=0 CARGO_PROFILE_RELEASE_STRIP=symbols
for t in aarch64-apple-darwin x86_64-apple-darwin; do
  rustup target list --installed | grep -qx "$t" || rustup target add "$t"
  cargo build -q --release -p bia-bridge --target "$t"
done
target_dir=$(cargo metadata --format-version=1 --no-deps | python3 -c 'import sys,json; print(json.load(sys.stdin)["target_directory"])')
out=source/flash/MCPs/web/bridge
bin="$out/bia-bridge-macos"
mkdir -p "$out"
fresh=$(mktemp -d)/bia-bridge
lipo -create -output "$fresh" \
  "$target_dir/aarch64-apple-darwin/release/bia-bridge" "$target_dir/x86_64-apple-darwin/release/bia-bridge"
lipo -info "$fresh"
# the same code as the build already notarized: keep that one (signing changes the file,
# so the unsigned build's hash is what tells)
unsigned=$(shasum -a 256 "$fresh" | cut -d' ' -f1)
if [[ "${BRIDGE_ADHOC:-0}" != 1 && -f "$bin" && -f "$bin.notarized" && -f "$bin.unsigned" ]] \
  && [[ "$(cat "$bin.unsigned")" == "$unsigned" && "$(shasum -a 256 "$bin" | cut -d' ' -f1)" == "$(cat "$bin.notarized")" ]]; then
  echo "$bin: unchanged and already notarized"
  exit 0
fi
mv "$fresh" "$bin"
rm -f "$bin.unsigned" "$bin.notarized"
if [[ "${BRIDGE_ADHOC:-0}" == 1 ]]; then
  codesign --force --sign - "$bin"
  echo "$bin (ad hoc signed: not for release)"
  exit 0
fi
codesign --force --options runtime --timestamp --sign "$CODESIGN_IDENTITY" "$bin"
codesign --verify --strict --verbose=2 "$bin"
if [[ "$NOTARIZE" -eq 1 ]]; then
  [[ -n "${APPLE_ID:-}" && -n "${APPLE_APP_SPECIFIC_PASSWORD:-}" ]] || { echo "error: APPLE_ID / APPLE_APP_SPECIFIC_PASSWORD not set (.env); --no-notarize for a local build" >&2; exit 1; }
  zip=$(mktemp -d)/bia-bridge-macos.zip
  ditto -c -k --keepParent "$bin" "$zip"
  log=$(mktemp)
  xcrun notarytool submit "$zip" --apple-id "$APPLE_ID" --password "$APPLE_APP_SPECIFIC_PASSWORD" --team-id "$APPLE_TEAM_ID" --wait | tee "$log"
  grep -q "status: Accepted" "$log" || { echo "error: notarization was not accepted" >&2; exit 1; }
  shasum -a 256 "$bin" | cut -d' ' -f1 > "$bin.notarized"
  echo "$unsigned" > "$bin.unsigned"
  rm -rf "$(dirname "$zip")" "$log"
  echo "$bin (Developer ID signed, notarized)"
else
  echo "$bin (Developer ID signed, not notarized)"
fi
