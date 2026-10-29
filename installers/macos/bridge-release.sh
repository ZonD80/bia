#!/usr/bin/env bash
# bia-bridge for macOS as a release asset: the signed + notarized universal build that the
# bias serve (tools/bridge-macos.sh), zipped:
#   installers/macos/bridge-release.sh [--no-notarize]
# → installers/dist/macos-universal/bia-bridge-macos-universal.zip
# Credentials as for release.sh (repo-root .env: APPLE_ID, APPLE_APP_SPECIFIC_PASSWORD;
# APPLE_TEAM_ID / CODESIGN_IDENTITY default to the bia Developer ID).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MACOS_DIR="$REPO_ROOT/installers/macos"
NOTARIZE=1
[[ "${1:-}" == "--no-notarize" ]] && NOTARIZE=0
for f in "$REPO_ROOT/.env" "$MACOS_DIR/.env"; do
  if [[ -f "$f" ]]; then set -a; source "$f"; set +a; fi
done
# the same identity as bia-deploy (release.sh)
APPLE_TEAM_ID="${APPLE_TEAM_ID:-$(sed -n 's/^DEFAULT_TEAM_ID="\(.*\)"/\1/p' "$MACOS_DIR/release.sh")}"
CODESIGN_IDENTITY="${CODESIGN_IDENTITY:-$(sed -n 's/^DEFAULT_CODESIGN_IDENTITY="\(.*\)"/\1/p' "$MACOS_DIR/release.sh")}"
[[ "$(uname -s)" == Darwin ]] || { echo "error: run on macOS" >&2; exit 1; }

cd "$REPO_ROOT"
# the build the bias serve (signed + notarized there), so GitHub and the bias carry one file
if [[ "$NOTARIZE" -eq 1 ]]; then tools/bridge-macos.sh; else tools/bridge-macos.sh --no-notarize; fi
DIST="$REPO_ROOT/installers/dist/macos-universal"
STAGE="$DIST/bia-bridge"
rm -rf "$STAGE"
mkdir -p "$STAGE"
cp source/flash/MCPs/web/bridge/bia-bridge-macos "$STAGE/bia-bridge"
cp "$REPO_ROOT/source/crates/bia-bridge/README.md" "$STAGE/README.md"
ZIP="$DIST/bia-bridge-macos-universal.zip"
rm -f "$ZIP"
(cd "$DIST" && ditto -c -k --keepParent bia-bridge "$(basename "$ZIP")")
echo "$ZIP"
