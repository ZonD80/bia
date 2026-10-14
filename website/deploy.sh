#!/usr/bin/env bash
# Build static site and deploy to Cloudflare Pages (production branch).
# Uses Wrangler OAuth (wrangler login). Optional env in .env.example.
set -euo pipefail

cd "$(dirname "$0")"

if [[ -f .env ]]; then
  set -a
  # shellcheck disable=SC1091
  source .env
  set +a
fi

exec ../scripts/deploy_website_cloudflare "$@"
