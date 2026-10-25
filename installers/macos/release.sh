#!/usr/bin/env bash
# Build, codesign, and notarize bia-flash (GUI .app + CLI) for macOS.
# Signing / notarization match ltw-macos/release.sh (AENGIX Developer ID).
#
# Usage:
#   installers/macos/release.sh              # build, sign, notarize, staple
#   installers/macos/release.sh --no-notarize
#   installers/macos/release.sh --cli-only
#   installers/macos/release.sh --app-only
#
# Credentials (repo-root .env):
#   APPLE_ID
#   APPLE_APP_SPECIFIC_PASSWORD
#   APPLE_TEAM_ID           (optional; default A59M2YXS84)
#   CODESIGN_IDENTITY       (optional; default AENGIX Developer ID)
#
# Artifacts → installers/dist/macos-<arch>/:
#   bia flash.app
#   bia-flash-<version>-macos-<arch>.dmg   (notarized + stapled when enabled)
#   bia-flash                              (CLI binary, signed)
#   bia-flash-cli-<version>-macos-<arch>.zip  (notarized)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MACOS_DIR="$REPO_ROOT/installers/macos"
INFO_PLIST="$MACOS_DIR/Info.plist"
ENTITLEMENTS="$MACOS_DIR/entitlements.plist"
DEFAULT_TEAM_ID="A59M2YXS84"
DEFAULT_CODESIGN_IDENTITY="Developer ID Application: AENGIX S.L. (A59M2YXS84)"

NOTARIZE=1
DO_APP=1
DO_CLI=1
UNIVERSAL=0

usage() {
  cat <<'EOF'
Build and notarize bia-flash for macOS (GUI app + CLI).

Usage:
  installers/macos/release.sh [options]

Options:
  --notarize       Submit to Apple notarization and staple (default)
  --no-notarize    Skip notarization (still codesign)
  --app-only       Only the .app / DMG
  --cli-only       Only the CLI zip
  --universal      One binary for Apple Silicon and Intel (arch "universal")
  -h, --help       Show this help

Environment (.env at repo root):
  APPLE_ID                     Apple ID email (required for notarization)
  APPLE_APP_SPECIFIC_PASSWORD  App-specific password (required for notarization)
  APPLE_TEAM_ID                Team ID (default: A59M2YXS84)
  CODESIGN_IDENTITY            Defaults to:
                               Developer ID Application: AENGIX S.L. (A59M2YXS84)

Requires: cargo, codesign, xcrun notarytool, stapler.
DMG packaging uses create-dmg when available (npm install -g create-dmg).
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --notarize) NOTARIZE=1; shift ;;
    --no-notarize|--skip-notarize) NOTARIZE=0; shift ;;
    --app-only) DO_CLI=0; shift ;;
    --cli-only) DO_APP=0; shift ;;
    --universal) UNIVERSAL=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *)
      echo "Unknown option: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

load_env() {
  local file="$1"
  if [[ -f "$file" ]]; then
    echo "Loading env: $file"
    set -a
    # shellcheck disable=SC1090
    source "$file"
    set +a
  fi
}

load_env "$REPO_ROOT/.env"
load_env "$MACOS_DIR/.env"

APPLE_TEAM_ID="${APPLE_TEAM_ID:-$DEFAULT_TEAM_ID}"
CODESIGN_IDENTITY="${CODESIGN_IDENTITY:-$DEFAULT_CODESIGN_IDENTITY}"

require_notarize_credentials() {
  local missing=0
  if [[ -z "${APPLE_ID:-}" ]]; then
    echo "error: APPLE_ID is not set in .env" >&2
    missing=1
  fi
  if [[ -z "${APPLE_APP_SPECIFIC_PASSWORD:-}" ]]; then
    echo "error: APPLE_APP_SPECIFIC_PASSWORD is not set in .env" >&2
    missing=1
  fi
  if [[ "$missing" -ne 0 ]]; then
    exit 1
  fi
}

if [[ "$(uname -s)" != Darwin ]]; then
  echo "error: macOS release script must run on Darwin" >&2
  exit 1
fi

arch="$(uname -m)"
case $arch in
  x86_64|amd64) arch=x86_64 ;;
  aarch64|arm64) arch=arm64 ;;
esac
[[ "$UNIVERSAL" -eq 1 ]] && arch=universal
DIST_DIR="$REPO_ROOT/installers/dist/macos-${arch}"
mkdir -p "$DIST_DIR"

VERSION="$(
  /usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$INFO_PLIST" 2>/dev/null \
    || true
)"
if [[ -z "$VERSION" ]]; then
  VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$REPO_ROOT/Cargo.toml" | head -1)"
fi
VERSION="${VERSION:-1.4.1}"

APP_BUNDLE_NAME="bia flash.app"
APP_PATH="$DIST_DIR/$APP_BUNDLE_NAME"
CLI_NAME="bia-flash"
CLI_PATH="$DIST_DIR/$CLI_NAME"
STAMPED_DMG_NAME="bia-flash-${VERSION}-macos-${arch}.dmg"
STAMPED_DMG_PATH="$DIST_DIR/$STAMPED_DMG_NAME"
CLI_ZIP_NAME="bia-flash-cli-${VERSION}-macos-${arch}.zip"
CLI_ZIP_PATH="$DIST_DIR/$CLI_ZIP_NAME"

echo "==> Using signing identity"
echo "    identity: $CODESIGN_IDENTITY"
echo "    team:     $APPLE_TEAM_ID"
echo "    arch:     $arch"
echo "    version:  $VERSION"
echo "    dist:     $DIST_DIR"

if [[ "$NOTARIZE" -eq 1 ]]; then
  require_notarize_credentials
fi

if [[ ! -f "$ENTITLEMENTS" ]]; then
  echo "error: entitlements not found at $ENTITLEMENTS" >&2
  exit 1
fi

echo "==> Building release binaries (gui feature)"
(
  cd "$REPO_ROOT"
  # Keep the builder's home directory and checkout path out of shipped binaries.
  export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$REPO_ROOT=/bia --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/cargo --remap-path-prefix=${RUSTUP_HOME:-$HOME/.rustup}=/rustup"
  # The workspace release profile keeps debug=1; shipped installers carry none
  # (on macOS the debug map would list every object file by absolute path).
  export CARGO_PROFILE_RELEASE_DEBUG=0 CARGO_PROFILE_RELEASE_STRIP=debuginfo
  if [[ "$UNIVERSAL" -eq 1 ]]; then
    for t in aarch64-apple-darwin x86_64-apple-darwin; do
      rustup target list --installed | grep -qx "$t" || rustup target add "$t"
      cargo build --release -p bia-flash --features gui --target "$t"
    done
  else
    cargo build --release -p bia-flash --features gui
  fi
)

target_dir="$(
  cd "$REPO_ROOT"
  cargo metadata --format-version=1 --no-deps \
    | python3 -c 'import sys,json; print(json.load(sys.stdin)["target_directory"])'
)"
REL="$target_dir/release"
if [[ "$UNIVERSAL" -eq 1 ]]; then
  REL="$target_dir/universal-apple-darwin/release"
  mkdir -p "$REL"
  for b in bia-flash bia-flash-gui; do
    lipo -create -output "$REL/$b" \
      "$target_dir/aarch64-apple-darwin/release/$b" "$target_dir/x86_64-apple-darwin/release/$b"
    lipo -info "$REL/$b"
  done
fi
GUI_BIN="$REL/bia-flash-gui"
CLI_BIN="$REL/bia-flash"
[[ -x "$GUI_BIN" ]] || { echo "error: missing $GUI_BIN" >&2; exit 1; }
[[ -x "$CLI_BIN" ]] || { echo "error: missing $CLI_BIN" >&2; exit 1; }

sign_bin() {
  local path="$1"
  echo "    codesign $(basename "$path")"
  codesign --force --options runtime --timestamp \
    --entitlements "$ENTITLEMENTS" \
    --sign "$CODESIGN_IDENTITY" \
    "$path"
  codesign --verify --strict --verbose=2 "$path"
}

if [[ "$DO_APP" -eq 1 ]]; then
  echo "==> Assembling $APP_BUNDLE_NAME"
  rm -rf "$APP_PATH"
  mkdir -p "$APP_PATH/Contents/MacOS" "$APP_PATH/Contents/Resources"
  cp -f "$INFO_PLIST" "$APP_PATH/Contents/Info.plist"
  cp -f "$MACOS_DIR/AppIcon.icns" "$APP_PATH/Contents/Resources/AppIcon.icns"
  /usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $VERSION" "$APP_PATH/Contents/Info.plist"
  cp -f "$GUI_BIN" "$APP_PATH/Contents/MacOS/bia-flash-gui"
  chmod 755 "$APP_PATH/Contents/MacOS/bia-flash-gui"

  echo "==> Signing app with $CODESIGN_IDENTITY"
  codesign --force --deep --options runtime --timestamp \
    --entitlements "$ENTITLEMENTS" \
    --sign "$CODESIGN_IDENTITY" \
    "$APP_PATH"
  codesign --verify --deep --strict --verbose=2 "$APP_PATH"

  echo "==> Packaging DMG"
  rm -f "$DIST_DIR"/*.dmg
  if command -v create-dmg >/dev/null 2>&1; then
    CREATE_DMG_CMD=(create-dmg)
  elif command -v npx >/dev/null 2>&1; then
    CREATE_DMG_CMD=(npx --yes create-dmg)
  else
    CREATE_DMG_CMD=()
  fi

  if [[ ${#CREATE_DMG_CMD[@]} -gt 0 ]]; then
    "${CREATE_DMG_CMD[@]}" \
      --overwrite \
      --identity="$CODESIGN_IDENTITY" \
      --dmg-title="bia flash" \
      "$APP_PATH" \
      "$DIST_DIR"
    CREATED_DMG_PATH="$(ls -t "$DIST_DIR"/*.dmg 2>/dev/null | head -1 || true)"
    if [[ -z "${CREATED_DMG_PATH:-}" || ! -f "$CREATED_DMG_PATH" ]]; then
      echo "error: DMG was not created in $DIST_DIR" >&2
      exit 1
    fi
    if [[ "$CREATED_DMG_PATH" != "$STAMPED_DMG_PATH" ]]; then
      rm -f "$STAMPED_DMG_PATH"
      mv "$CREATED_DMG_PATH" "$STAMPED_DMG_PATH"
    fi
  else
    echo "    create-dmg not found; building UDIF DMG with hdiutil"
    STAGE="$(mktemp -d)"
    cp -R "$APP_PATH" "$STAGE/"
    ln -s /Applications "$STAGE/Applications"
    hdiutil create -volname "bia flash" -srcfolder "$STAGE" -ov -format UDZO "$STAMPED_DMG_PATH"
    rm -rf "$STAGE"
  fi

  echo "==> Signing stamped DMG"
  codesign --force --sign "$CODESIGN_IDENTITY" --timestamp "$STAMPED_DMG_PATH"
  codesign --verify --verbose=2 "$STAMPED_DMG_PATH"

  if [[ "$NOTARIZE" -eq 1 ]]; then
    echo "==> Submitting DMG for notarization"
    xcrun notarytool submit "$STAMPED_DMG_PATH" \
      --apple-id "$APPLE_ID" \
      --password "$APPLE_APP_SPECIFIC_PASSWORD" \
      --team-id "$APPLE_TEAM_ID" \
      --wait
    echo "==> Stapling notarization ticket (DMG + app)"
    xcrun stapler staple "$STAMPED_DMG_PATH"
    xcrun stapler validate "$STAMPED_DMG_PATH"
    # Staple the .app as well when possible (offline Gatekeeper for unpacked app).
    xcrun stapler staple "$APP_PATH" 2>/dev/null || true
  else
    echo "==> Skipping DMG notarization (--no-notarize)"
  fi
fi

if [[ "$DO_CLI" -eq 1 ]]; then
  echo "==> Staging CLI"
  cp -f "$CLI_BIN" "$CLI_PATH"
  chmod 755 "$CLI_PATH"
  sign_bin "$CLI_PATH"

  echo "==> Zipping CLI for notarization"
  rm -f "$CLI_ZIP_PATH"
  (
    cd "$DIST_DIR"
    ditto -c -k --keepParent "$CLI_NAME" "$(basename "$CLI_ZIP_PATH")"
  )

  if [[ "$NOTARIZE" -eq 1 ]]; then
    echo "==> Submitting CLI zip for notarization"
    xcrun notarytool submit "$CLI_ZIP_PATH" \
      --apple-id "$APPLE_ID" \
      --password "$APPLE_APP_SPECIFIC_PASSWORD" \
      --team-id "$APPLE_TEAM_ID" \
      --wait
    echo "    CLI zip notarized (staple not applicable to raw zip; Gatekeeper checks online)"
  else
    echo "==> Skipping CLI notarization (--no-notarize)"
  fi
fi

cat >"$DIST_DIR/README.txt" <<EOF
bia flash — macOS ${arch} (v${VERSION})

GUI:
  Open "bia flash.app", or install from ${STAMPED_DMG_NAME}

CLI:
  ./bia-flash list
  ./bia-flash write --disk /dev/rdiskN --arch x86_64 --yes   # downloads the bia image from GitHub
  Zip: ${CLI_ZIP_NAME}

Signed with Developer ID. Notarization: $([[ "$NOTARIZE" -eq 1 ]] && echo yes || echo skipped).
API keys: use ./fmm.json (gitignored) or --fmm path.
EOF

echo
echo "Done."
if [[ "$DO_APP" -eq 1 ]]; then
  echo "  App:  $APP_PATH"
  echo "  DMG:  $STAMPED_DMG_PATH"
fi
if [[ "$DO_CLI" -eq 1 ]]; then
  echo "  CLI:  $CLI_PATH"
  echo "  Zip:  $CLI_ZIP_PATH"
fi
if [[ "$NOTARIZE" -eq 1 ]]; then
  echo "  Notarization: submitted"
fi
