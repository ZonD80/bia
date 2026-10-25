# Sourced by builders/build.sh and tests/run-all.sh inside bia-build (needs ARCH=arm64|x86_64):
# ensure_go puts a pinned Go toolchain on PATH, fetched once into the /cache volume.
GO_VER=${GO_VER:-1.26.10}
ensure_go() {
  local goarch sha
  case $ARCH in
    arm64)  goarch=arm64; sha=3f4227ce7b206a513e794f1393adb473a07b46e94d18bd15d1c684bbb7319797 ;;
    x86_64) goarch=amd64; sha=ccfe7ae822d2c2f41f4fe7c956199967a4dac22d0ee0d2b2bfdb440aab3b20c5 ;;
  esac
  local root="/cache/go-$GO_VER-$goarch"
  if [ ! -x "$root/bin/go" ]; then
    local tgz="/cache/go$GO_VER.linux-$goarch.tar.gz"
    echo "   fetching go $GO_VER ($goarch)"
    curl -fL --retry 3 -o "$tgz.partial" "https://go.dev/dl/go$GO_VER.linux-$goarch.tar.gz"
    echo "$sha  $tgz.partial" | sha256sum -c - >/dev/null || { echo "go tarball checksum mismatch" >&2; exit 1; }
    rm -rf "$root.tmp" && mkdir -p "$root.tmp"
    tar -C "$root.tmp" --strip-components=1 -xzf "$tgz.partial"
    rm -f "$tgz.partial"
    mv "$root.tmp" "$root"
  fi
  export PATH="$root/bin:$PATH" GOPATH=/cache/gopath GOMODCACHE=/cache/gopath/pkg/mod GOCACHE=/cache/gocache-$ARCH GOTOOLCHAIN=local
}

