#!/bin/bash
# Build release bia-flash (+ optional GUI) for the host OS.
#   tools/build-flasher.sh           # CLI only
#   tools/build-flasher.sh gui       # CLI + bia-flash-gui
# Output: installers/dist/<os>-<arch>/
#
# On macOS prefer: installers/macos/release.sh  (Developer ID + notarization)
#   make flasher
#   NOTARIZE=0 make flasher
set -euo pipefail
cd "$(dirname "$0")/.."
if [ "$(uname -s)" = Darwin ] && [ "${1:-}" = gui ] && [ "${FORCE_PLAIN_FLASHER:-0}" != 1 ]; then
  echo "note: on macOS use installers/macos/release.sh (or make flasher) for notarized app+CLI"
fi
WITH_GUI=${1:-}
features=()
bins=(bia-flash)
if [ "$WITH_GUI" = gui ] || [ "$WITH_GUI" = --gui ]; then
  features=(--features gui)
  bins+=(bia-flash-gui)
fi
# Keep the builder's home directory and checkout path out of shipped binaries.
export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$PWD=/bia --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/cargo --remap-path-prefix=${RUSTUP_HOME:-$HOME/.rustup}=/rustup"
# The workspace release profile keeps debug=1; shipped installers carry none
# (on macOS the debug map would list every object file by absolute path).
export CARGO_PROFILE_RELEASE_DEBUG=0 CARGO_PROFILE_RELEASE_STRIP=debuginfo
echo "== cargo build --release -p bia-flash ${features[*]:-}"
cargo build --release -p bia-flash "${features[@]+"${features[@]}"}"

target_dir=$(cargo metadata --format-version=1 --no-deps 2>/dev/null \
  | python3 -c 'import sys,json; print(json.load(sys.stdin)["target_directory"])')
rel="$target_dir/release"

host=$(uname -s | tr '[:upper:]' '[:lower:]')
case $host in
  darwin) host=macos ;;
  mingw*|msys*|cygwin*) host=windows ;;
  linux) host=linux ;;
esac
arch=$(uname -m)
case $arch in
  x86_64|amd64) arch=x86_64 ;;
  aarch64|arm64) arch=arm64 ;;
esac
out="installers/dist/${host}-${arch}"
mkdir -p "$out"

for b in "${bins[@]}"; do
  src="$rel/$b"
  [ -f "$src" ] || src="$rel/$b.exe"
  [ -f "$src" ] || { echo "missing $b in $rel"; exit 1; }
  cp -f "$src" "$out/"
  echo "   $out/$(basename "$src")"
done
cat >"$out/README.txt" <<EOF
bia-flash — write a bootable bia USB stick

CLI:
  bia-flash list
  # Keys from local gitignored fmm.json (or --fmm path):
  cp fmm.json.example fmm.json   # edit api_key
  bia-flash write --disk /dev/… --root dist/<arch>/root.img --esp source/flash --arch x86_64 --yes
  # Local model from Hugging Face (HF_TOKEN for gated repos):
  bia-flash search qwen3 4b instruct
  bia-flash files unsloth/Qwen3-4B-Instruct-2507-GGUF
  bia-flash write … --model unsloth/Qwen3-4B-Instruct-2507-GGUF:Qwen3-4B-Instruct-2507-Q4_K_M.gguf

GUI (if built with gui):
  bia-flash-gui

Use your ChatGPT plan instead of a key (browser sign-in; one per stick):
  bia-flash chatgpt                       # lists your models
  bia-flash chatgpt --model <id>          # saves fmm.json for write --fmm
Where to get an API key (links per provider):
  bia-flash keys
List the models an FMM key can use (provider detected from the key):
  bia-flash models claude
Wake bia on the FMM model (write does this first; --no-test skips):
  bia-flash test

FMM keys are written to EFI/bia/fmm.json on the FAT32 ESP. The agent seeds
them into /home/bia/files/state/fmm.json on first boot when state is empty.
Never commit fmm.json — see .gitignore.

The system partition takes the whole stick; it grows on first boot. A model is
streamed onto the stick and moved into /home/bia/model on first boot.

Host: ${host}-${arch}
EOF
echo "== flasher binaries in $out"
