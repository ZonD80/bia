#!/bin/bash
# Guest smoke checklist for ferogram+tgcalls telegram MCP.
# Run inside the guest (or via agent tools) after flash boot.
#
# Expects: bia-mcp-telegram on PATH / MCP enabled, ffmpeg + speech env present.
set -euo pipefail

echo "== env"
command -v ffmpeg >/dev/null && echo "ffmpeg: $(command -v ffmpeg)" || echo "ffmpeg: MISSING"
test -x /usr/local/lib/bia/speech/bin/bia-tts && echo "bia-tts: ok" || echo "bia-tts: missing"
test -x /usr/local/lib/bia/speech/bin/bia-stt && echo "bia-stt: ok" || echo "bia-stt: missing"
echo "session path: \${BIA_FILES:-/home/bia/files}/telegram/ferogram.session"

cat <<'EOF'

Manual MCP smoke (via agent or mcp client):
  1. telegram_status          → stack=ferogram+tgcalls, auth=waiting_phone|ready
  2. telegram_login(phone)    → waiting_code
  3. telegram_submit_code     → ready (or waiting_password → submit_password)
  4. telegram_send / search / history
  5. telegram_call_start or accept → inbound [call ready]
  6. telegram_call_say        → audible TTS via Media::audio
  7. telegram_call_hangup     → [call ended]

EOF
