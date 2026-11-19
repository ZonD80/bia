#!/bin/sh
# The web MCP's ACME client against a local Pebble (Let's Encrypt's test CA): an IP
# certificate with the shortlived profile, http-01 answered on a temporary port 80, then a
# renewal that names the first certificate (ARI `replaces`).
#
#   tests/manual/acme-pebble.sh            # builds Pebble with Go into $WORK, runs the test
#
# Needs Go and a free port 80 (macOS lets ordinary users bind it; on Linux run as root or
# set net.ipv4.ip_unprivileged_port_start=80).
set -eu
here=$(cd "$(dirname "$0")/../.." && pwd)
WORK=${WORK:-$(mktemp -d)}
VER=${PEBBLE_VERSION:-v2.10.3}

GOBIN="$WORK/bin" go install "github.com/letsencrypt/pebble/v2/cmd/pebble@$VER"
src="$(go env GOMODCACHE)/github.com/letsencrypt/pebble/v2@$VER"
python3 - "$src" "$WORK" <<'EOF'
import json, sys
src, work = sys.argv[1], sys.argv[2]
c = json.load(open(f"{src}/test/config/pebble-config.json"))
p = c["pebble"]
p["certificate"] = f"{src}/test/certs/localhost/cert.pem"
p["privateKey"] = f"{src}/test/certs/localhost/key.pem"
p["httpPort"] = 80
json.dump(c, open(f"{work}/config.json", "w"), indent=1)
EOF
cat "$src/test/certs/pebble.minica.pem" >"$WORK/ca.pem.tmp" && mv -f "$WORK/ca.pem.tmp" "$WORK/ca.pem"

PEBBLE_VA_NOSLEEP=1 "$WORK/bin/pebble" -config "$WORK/config.json" -dnsserver 127.0.0.1:1 >"$WORK/pebble.log" 2>&1 &
pid=$!
trap 'kill $pid 2>/dev/null || true' EXIT
sleep 2

cd "$here/source"
BIA_ACME_DIRECTORY=https://127.0.0.1:14000/dir BIA_ACME_CA="$WORK/ca.pem" \
    cargo test -p bia-mcp-web pebble_issues_and_renews_an_ip_certificate -- --ignored --nocapture
echo "pebble log: $WORK/pebble.log"
