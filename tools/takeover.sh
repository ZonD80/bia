#!/bin/bash
# Replace this running Linux (a VPS, a server) with bia.
#
#   curl -fsSL https://github.com/zond80/bia/releases/latest/download/takeover.sh | sudo bash
#   curl -fsSL …/takeover.sh | sudo bash -s -- --config https://example.com/bia.json
#
# Downloads the Linux bia-flash for this machine from the same release, checks it against
# the release manifest (bia-os.json) and runs `bia-flash takeover` with these arguments.
# It asks for whatever the configs and arguments leave open (API key, language, setup
# instructions for bia, the disk to replace) and changes nothing before you confirm.
# `bia-flash takeover --help` lists every option; --dump-config shows the plan only.
#
# BIA_RELEASE_URL: a directory with bia-os.json and the files instead of the GitHub
# release (tests); bia-flash honours it too.
set -euo pipefail

REL=${BIA_RELEASE_URL:-https://github.com/zond80/bia/releases/latest/download}
REL=${REL%/}

die() { echo "takeover: $*" >&2; exit 1; }

[ "$(id -u)" = 0 ] || die "run as root: curl -fsSL …/takeover.sh | sudo bash"
[ "$(uname -s)" = Linux ] || die "this replaces a running Linux; for a USB stick use bia flash instead"
case $(uname -m) in
  x86_64|amd64) ARCH=x86_64 ;;
  aarch64|arm64) ARCH=arm64 ;;
  *) die "bia runs on x86_64 and arm64, not $(uname -m)" ;;
esac

fetch() { # url out
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL --retry 3 -o "$2" "$1"
  elif command -v wget >/dev/null 2>&1; then
    wget -q -O "$2" "$1"
  else
    die "curl or wget is needed"
  fi
}

WORK=$(mktemp -d /root/bia-takeover-tool.XXXXXX)
trap 'rm -rf "$WORK"' EXIT
TOOL=bia-flash-linux-$ARCH.tar.gz

echo "== bia release manifest from $REL"
fetch "$REL/bia-os.json" "$WORK/bia-os.json" || die "cannot download $REL/bia-os.json"
# "tools": {"<file>": {"file": …, "size": …, "sha256": "<hex>"}}
if command -v python3 >/dev/null 2>&1; then
  WANT=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("tools",{}).get(sys.argv[2],{}).get("sha256",""))' "$WORK/bia-os.json" "$TOOL")
else
  WANT=$(tr -d '\n ' < "$WORK/bia-os.json" | grep -o "\"$TOOL\":{[^}]*}" | grep -o '"sha256":"[0-9a-f]*"' | cut -d'"' -f4 || true)
fi
[ -n "$WANT" ] || die "this release has no $TOOL (bia 1.1.3 or newer is needed)"

echo "== $TOOL"
fetch "$REL/$TOOL" "$WORK/$TOOL" || die "cannot download $REL/$TOOL"
GOT=$(sha256sum "$WORK/$TOOL" | awk '{ print $1 }')
[ "$GOT" = "$WANT" ] || die "$TOOL has sha256 $GOT, the release says $WANT; not running it"
tar -C "$WORK" -xzf "$WORK/$TOOL"
[ -x "$WORK/bia-flash/bia-flash" ] || die "$TOOL holds no bia-flash"

# The tool asks on /dev/tty, so this works with the script piped into bash.
"$WORK/bia-flash/bia-flash" takeover "$@"
