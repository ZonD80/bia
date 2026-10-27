#!/bin/bash
# Replace this running Linux (a VPS, a server) with bia.
#
#   curl -fsSL https://github.com/zond80/bia/releases/latest/download/takeover.sh | sudo bash
#   curl -fsSL …/takeover.sh | sudo bash -s -- --config https://example.com/bia.json
#
# Downloads the Linux bia-deploy for this machine from the same release, checks it against
# the release manifest (bia-os.json) and runs `bia-deploy takeover` with these arguments.
# It asks for whatever the configs and arguments leave open (API key, language, setup
# instructions for bia, the disk to replace) and changes nothing before you confirm.
# `bia-deploy takeover --help` lists every option; --dump-config shows the plan only.
#
# BIA_RELEASE_URL: a directory with bia-os.json and the files instead of the GitHub
# release (tests); bia-deploy honours it too.
set -euo pipefail

REL=${BIA_RELEASE_URL:-https://github.com/zond80/bia/releases/latest/download}
REL=${REL%/}

die() { echo "takeover: $*" >&2; exit 1; }

[ "$(id -u)" = 0 ] || die "run as root: curl -fsSL …/takeover.sh | sudo bash"
[ "$(uname -s)" = Linux ] || die "this replaces a running Linux; for a USB stick use bia deploy instead"
case $(uname -m) in
  x86_64|amd64) ARCH=x86_64 ;;
  aarch64|arm64) ARCH=arm64 ;;
  *) die "bia runs on x86_64 and arm64, not $(uname -m)" ;;
esac
# Never on bia itself: it would erase this agent. A misquoted `ssh root@host curl … | bash`
# runs the pipe here, not on the server. bia-deploy takeover checks the same files.
for f in /etc/bia-release /etc/bia-shipped; do
  [ -e "$f" ] && die "this machine already runs bia ($f); a takeover would erase it. Nothing was changed.
  To update bia: carry_update (or /update on the console).
  To take over another server, run this on that server, with the whole pipe inside the quotes:
  ssh root@<server> 'curl -fsSL …/takeover.sh | bash -s -- …'"
done

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
TOOL=bia-deploy-linux-$ARCH.tar.gz

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
[ -x "$WORK/bia-deploy/bia-deploy" ] || die "$TOOL holds no bia-deploy"

# The tool asks on /dev/tty, so this works with the script piped into bash.
"$WORK/bia-deploy/bia-deploy" takeover "$@"
