#!/bin/bash
# Fetch sherpa-onnx (STT/TTS), whisper-tiny, Silero VAD, GTCRN, the English Piper
# voice, and ntgcalls libs for tgcalls. Other languages are installed on demand by
# the lang MCP into /home/bia/files/speech.
# ffmpeg stays in the Debian root via rootfs apt (required by tgcalls Media::audio).
#
#   builders/speech-prebuilt.sh <arm64|x86_64> <debian-root>
set -euo pipefail
ARCH=${1:?arch arm64|x86_64}
ROOT=${2:?debian root}
CACHE=${CACHE:-/cache}/speech
SHERPA_VER=${SHERPA_VER:-v1.13.6}
# Must match vendor/tgcalls ntgcalls crate pin.
NTGCALLS_VER=${NTGCALLS_VER:-v3.0.0}
DEST="$ROOT/usr/local/lib/bia/speech"
mkdir -p "$CACHE" "$DEST/bin" "$DEST/lib" "$DEST/models"

case $ARCH in
  arm64)
    NTG_ARCH=arm64
    SHERPA_ASSET="sherpa-onnx-${SHERPA_VER}-linux-aarch64-shared-cpu.tar.bz2"
    SHERPA_URL="https://github.com/k2-fsa/sherpa-onnx/releases/download/${SHERPA_VER}/${SHERPA_ASSET}"
    ;;
  x86_64)
    NTG_ARCH=x86_64
    SHERPA_ASSET="sherpa-onnx-${SHERPA_VER}-linux-x64-shared.tar.bz2"
    SHERPA_URL="https://github.com/k2-fsa/sherpa-onnx/releases/download/${SHERPA_VER}/${SHERPA_ASSET}"
    ;;
  *) echo "speech-prebuilt: unknown arch $ARCH"; exit 1 ;;
esac

NTG_SHARED_ASSET="ntgcalls.linux-${NTG_ARCH}-shared_libs.zip"
NTG_BASE="https://github.com/pytgcalls/ntgcalls/releases/download/${NTGCALLS_VER}"
# Multilingual whisper-tiny: transcription for calls and language identification
# (it knows every Whisper language, so a language that is not installed is still
# detected). Hashes match source/flash/MCPs/lang/catalog.json.
WHISPER_ASSET="sherpa-onnx-whisper-tiny.tar.bz2"
WHISPER_URL="https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/${WHISPER_ASSET}"
WHISPER_SHA=c46116994e539aa165266d96b325252728429c12535eb9d8b6a2b10f129e66b1
VAD_URL="https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx"
VAD_SHA=9e2449e1087496d8d4caba907f23e0bd3f78d91fa552479bb9c23ac09cbb1fd6
GTCRN_URL="https://github.com/k2-fsa/sherpa-onnx/releases/download/speech-enhancement-models/gtcrn_simple.onnx"
GTCRN_SHA=e77603ac0c23dac3227dd2d7135b3a585cbee2679048aecfa886657d3ae1b534
TTS_ASSET="vits-piper-en_US-lessac-medium-int8.tar.bz2"
TTS_URL="https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/${TTS_ASSET}"

fetch() {
  local url=$1 dest=$2
  if [ -s "$dest" ]; then
    echo "   cached $(basename "$dest")"
    return
  fi
  echo "   get $url"
  curl -fL --retry 3 --retry-delay 2 -A bia-build -o "$dest.partial" "$url"
  mv "$dest.partial" "$dest"
}

verify() {
  local file=$1 want=$2 got
  got=$(sha256sum "$file" | cut -d' ' -f1)
  if [ "$got" != "$want" ]; then
    echo "speech-prebuilt: sha256 mismatch for $file: $got (want $want)"
    rm -f "$file"
    exit 1
  fi
}

echo "== speech: sherpa-onnx $SHERPA_VER ($ARCH)"
fetch "$SHERPA_URL" "$CACHE/$SHERPA_ASSET"
rm -rf "$CACHE/sherpa-unpack"
mkdir -p "$CACHE/sherpa-unpack"
tar -xjf "$CACHE/$SHERPA_ASSET" -C "$CACHE/sherpa-unpack"
SHERPA_ROOT=$(find "$CACHE/sherpa-unpack" -type d -name bin | head -1 | xargs dirname)
cp -a "$SHERPA_ROOT/bin/." "$DEST/bin/"
cp -a "$SHERPA_ROOT/lib/." "$DEST/lib/"
chmod 755 "$DEST/bin/"* || true
ln -sfn sherpa-onnx-offline "$DEST/bin/bia-stt"
if [ -x "$DEST/bin/sherpa-onnx-offline-tts" ]; then
  ln -sfn sherpa-onnx-offline-tts "$DEST/bin/bia-tts"
fi

echo "== speech: whisper-tiny multilingual (int8)"
fetch "$WHISPER_URL" "$CACHE/$WHISPER_ASSET"
verify "$CACHE/$WHISPER_ASSET" "$WHISPER_SHA"
rm -rf "$CACHE/whisper-unpack"
mkdir -p "$CACHE/whisper-unpack"
tar -xjf "$CACHE/$WHISPER_ASSET" -C "$CACHE/whisper-unpack"
WROOT=$(find "$CACHE/whisper-unpack" -name 'tiny-encoder.int8.onnx' | head -1 | xargs dirname)
rm -rf "$DEST/models/whisper-tiny.en"
mkdir -p "$DEST/models/whisper-tiny"
cp -a "$WROOT/tiny-encoder.int8.onnx" "$DEST/models/whisper-tiny/encoder.onnx"
cp -a "$WROOT/tiny-decoder.int8.onnx" "$DEST/models/whisper-tiny/decoder.onnx"
cp -a "$WROOT/tiny-tokens.txt" "$DEST/models/whisper-tiny/tokens.txt"

echo "== speech: Silero VAD + GTCRN denoiser"
fetch "$VAD_URL" "$CACHE/silero_vad.onnx"
verify "$CACHE/silero_vad.onnx" "$VAD_SHA"
mkdir -p "$DEST/models/silero-vad" "$DEST/models/gtcrn"
cp -a "$CACHE/silero_vad.onnx" "$DEST/models/silero-vad/silero_vad.onnx"
fetch "$GTCRN_URL" "$CACHE/gtcrn_simple.onnx"
verify "$CACHE/gtcrn_simple.onnx" "$GTCRN_SHA"
cp -a "$CACHE/gtcrn_simple.onnx" "$DEST/models/gtcrn/gtcrn_simple.onnx"

echo "== speech: piper en_US-lessac-medium int8 (via sherpa-tts)"
fetch "$TTS_URL" "$CACHE/$TTS_ASSET"
rm -rf "$CACHE/tts-unpack"
mkdir -p "$CACHE/tts-unpack"
tar -xjf "$CACHE/$TTS_ASSET" -C "$CACHE/tts-unpack"
TROOT=$(find "$CACHE/tts-unpack" -name 'en_US-lessac-medium.onnx' | head -1 | xargs dirname)
mkdir -p "$DEST/models/piper-en_US-lessac-medium"
cp -a "$TROOT/." "$DEST/models/piper-en_US-lessac-medium/"

echo "== speech: ntgcalls $NTGCALLS_VER (shared lib for guest + build cache)"
fetch "$NTG_BASE/$NTG_SHARED_ASSET" "$CACHE/$NTGCALLS_VER-$NTG_SHARED_ASSET"
rm -rf "$CACHE/ntg-shared-unpack"
mkdir -p "$CACHE/ntg-shared-unpack"
unzip -qo "$CACHE/$NTGCALLS_VER-$NTG_SHARED_ASSET" -d "$CACHE/ntg-shared-unpack"
NTG_SO=$(find "$CACHE/ntg-shared-unpack" -name 'libntgcalls.so' | head -1 || true)
[ -n "$NTG_SO" ] || { echo "libntgcalls.so missing from $NTG_SHARED_ASSET"; exit 1; }
cp -a "$NTG_SO" "$DEST/lib/libntgcalls.so"
# Mirror for cargo link (build.sh also fetches into /cache/ntgcalls-dylib-$ARCH).
mkdir -p "/cache/ntgcalls-dylib-$ARCH" "/cache/ntgcalls-dylib-$ARCH-$NTGCALLS_VER"
cp -a "$NTG_SO" "/cache/ntgcalls-dylib-$ARCH/libntgcalls.so"
cp -a "$NTG_SO" "/cache/ntgcalls-dylib-$ARCH-$NTGCALLS_VER/libntgcalls.so"
NTG_H=$(find "$CACHE/ntg-shared-unpack" -name 'ntgcalls.h' | head -1 || true)
if [ -n "$NTG_H" ]; then
  cp -a "$NTG_H" "$DEST/lib/ntgcalls.h"
  cp -a "$NTG_H" "/cache/ntgcalls-dylib-$ARCH/"
  cp -a "$NTG_H" "/cache/ntgcalls-dylib-$ARCH-$NTGCALLS_VER/"
fi

# Wrapper env file for the speech MCPs (models come from the language registry,
# /home/bia/files/speech/languages.json, managed by the lang MCP).
cat > "$DEST/env.sh" <<EOF
export BIA_SPEECH_DIR=/usr/local/lib/bia/speech
export PATH="/usr/local/lib/bia/speech/bin:\$PATH"
export LD_LIBRARY_PATH="/usr/local/lib/bia/speech/lib:\${LD_LIBRARY_PATH:-}"
EOF

echo "== speech installed under $DEST (no bia-call-media; tgcalls links ntgcalls)"
ls -la "$DEST/bin" | head -30
du -sh "$DEST" "$DEST/models" "$DEST/lib" 2>/dev/null || true
