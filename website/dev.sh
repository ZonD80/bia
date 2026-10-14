#!/usr/bin/env bash
# Start the Nuxt website locally.
set -euo pipefail

cd "$(dirname "$0")"

if [[ ! -d node_modules ]]; then
  npm install --no-audit --no-fund
fi

exec npm run dev "$@"
