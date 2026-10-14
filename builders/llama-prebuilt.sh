#!/bin/bash
# Official llama.cpp Ubuntu builds, unpacked into the Debian root.
# https://github.com/ggml-org/llama.cpp/releases
# The install doc (conda, brew, nix) ships one CLI. The release assets ship a
# llama-server per backend. The agent execs these binaries (CPU/Vulkan/CUDA/…)
# as a child on 127.0.0.1 so GPU stacks stay out of the agent link.
#
#   LLAMA_CPP_BUILD=b11228
#   LLAMA_BACKENDS=cpu,vulkan,cuda,sycl   (x86_64 default)
#   LLAMA_BACKENDS=cpu,vulkan,cuda        (arm64 default)
#   LLAMA_BACKENDS=none                   CPU server only
# The CPU tarball is always unpacked.
set -euo pipefail
ARCH=${1:?arch arm64|x86_64}
ROOT=${2:?debian root}
BUILD=${LLAMA_CPP_BUILD:-b11228}
CACHE=${CACHE:-/cache}/llama-cpp
case $ARCH in
  x86_64) PKG=x64; DEFAULT=cpu,vulkan,cuda,sycl ;;
  arm64)  PKG=arm64; DEFAULT=cpu,vulkan,cuda ;;
  *) echo "llama-prebuilt: unknown arch $ARCH"; exit 1 ;;
esac
BACKENDS=${LLAMA_BACKENDS:-$DEFAULT}
if [ -z "$BACKENDS" ] || [ "$BACKENDS" = none ]; then
  BACKENDS=cpu
fi
case ",$BACKENDS," in
  *,cpu,*) ;;
  *) BACKENDS="cpu,$BACKENDS" ;;
esac
mkdir -p "$CACHE" "$ROOT/usr/local/lib/bia/llama"

asset_for() {
  case $1 in
    cpu)    echo "llama-${BUILD}-bin-ubuntu-${PKG}.tar.gz" ;;
    vulkan) echo "llama-${BUILD}-bin-ubuntu-vulkan-${PKG}.tar.gz" ;;
    cuda)   echo "llama-${BUILD}-bin-ubuntu-cuda-13.4-${PKG}.tar.gz" ;;
    sycl)
      [ "$PKG" = x64 ] || { echo "sycl prebuilt is x86_64 only"; return 1; }
      echo "llama-${BUILD}-bin-ubuntu-sycl-fp16-x64.tar.gz"
      ;;
    rocm)
      [ "$PKG" = x64 ] || { echo "rocm prebuilt is x86_64 only"; return 1; }
      echo "llama-${BUILD}-bin-ubuntu-rocm-10.0-x64.tar.gz"
      ;;
    *) echo "unknown llama backend '$1' (cpu, vulkan, cuda, sycl, rocm)"; return 1 ;;
  esac
}

fetch() {
  local name=$1 dest=$2
  if [ -s "$dest" ]; then
    echo "   cached $name"
    return
  fi
  local url="https://github.com/ggml-org/llama.cpp/releases/download/${BUILD}/${name}"
  echo "   get $url"
  curl -fL --retry 3 --retry-delay 2 -o "$dest.partial" "$url"
  mv "$dest.partial" "$dest"
}

# llama-server is sometimes at the archive root, sometimes one directory down.
hoist() {
  local dest=$1
  [ -f "$dest/llama-server" ] && return 0
  local bin src tmp
  bin=""
  while IFS= read -r f; do
    bin=$f
    break
  done < <(find "$dest" -type f -name llama-server)
  [ -n "$bin" ] || return 1
  src=$(dirname "$bin")
  [ "$src" = "$dest" ] && return 0
  tmp=$(mktemp -d)
  mv "$src"/* "$tmp/"
  find "$dest" -mindepth 1 -maxdepth 1 -exec rm -rf {} +
  mv "$tmp"/* "$dest/"
  rmdir "$tmp"
}

IFS=',' read -ra list <<< "$BACKENDS"
for b in "${list[@]}"; do
  b=${b// /}
  [ -n "$b" ] || continue
  echo "== llama.cpp $b ($BUILD)"
  name=$(asset_for "$b")
  fetch "$name" "$CACHE/$name"
  dest="$ROOT/usr/local/lib/bia/llama/$b"
  rm -rf "$dest"
  mkdir -p "$dest"
  tar -xzf "$CACHE/$name" -C "$dest"
  hoist "$dest" || { echo "no llama-server in $name"; exit 1; }
  if [ "$b" = cuda ] && ! find "$dest" -name 'libcudart.so*' | grep -q .; then
    crt="cudart-llama-${BUILD}-bin-ubuntu-cuda-13.4-${PKG}.tar.gz"
    echo "   cuda build has no libcudart; fetching $crt"
    fetch "$crt" "$CACHE/$crt"
    stage=$(mktemp -d)
    tar -xzf "$CACHE/$crt" -C "$stage"
    find "$stage" -name 'libcudart.so*' -exec cp -a {} "$dest/" \;
    rm -rf "$stage"
    find "$dest" -name 'libcudart.so*' | grep -q . || { echo "libcudart missing from $crt"; exit 1; }
  fi
  chmod 755 "$dest/llama-server"
  echo "   installed $dest/llama-server"
done
