# Shared by build-apk.sh and build-aab.sh (sourced): Java, SDK and NDK; the Rust core for
# the phone ABIs (cargo-ndk) and its Kotlin bindings (uniffi-bindgen); signing checks.
# BIA_SKIP_CORE=1 keeps the core of the previous build (build-aab.sh right after build-apk.sh).
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$ROOT_DIR/../.." && pwd)"
KEYSTORE="$HOME/biaDeployPlayKeys.jks"
KEYSTORE_PROPS="$ROOT_DIR/keystore.properties"
NDK_VERSION="${NDK_VERSION:-29.0.14206865}"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$REPO/Cargo.toml" | head -1)"
DIST="$REPO/installers/dist/android"

die() {
    echo "error: $*" >&2
    exit 1
}

find_android_studio_jdk() {
    local candidate
    for candidate in \
        "/Applications/Android Studio.app/Contents/jbr/Contents/Home" \
        "$HOME/Applications/Android Studio.app/Contents/jbr/Contents/Home" \
        /Applications/Android\ Studio*.app/Contents/jbr/Contents/Home; do
        if [[ -x "$candidate/bin/java" ]]; then
            echo "$candidate"
            return 0
        fi
    done
    return 1
}

setup_java() {
    local jdk
    if jdk="$(find_android_studio_jdk)"; then
        :
    elif [[ -n "${JAVA_HOME:-}" && -x "${JAVA_HOME}/bin/java" ]]; then
        jdk="$JAVA_HOME"
    else
        die "Android Studio JDK not found; install Android Studio or set JAVA_HOME to JDK 17+"
    fi
    export JAVA_HOME="$jdk"
    export PATH="$JAVA_HOME/bin:$PATH"
    echo "Using Java: $JAVA_HOME"
}

setup_sdk() {
    export ANDROID_HOME="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
    [[ -d "$ANDROID_HOME/platforms" ]] || die "Android SDK not found at $ANDROID_HOME (Android Studio installs it)"
    export ANDROID_NDK_HOME="$ANDROID_HOME/ndk/$NDK_VERSION"
    [[ -d "$ANDROID_NDK_HOME" ]] || die "NDK $NDK_VERSION missing: Android Studio › SDK Manager › SDK Tools › NDK, or: sdkmanager \"ndk;$NDK_VERSION\""
    [[ -f "$ROOT_DIR/local.properties" ]] || echo "sdk.dir=$ANDROID_HOME" > "$ROOT_DIR/local.properties"
}

check_signing() {
    [[ -f "$KEYSTORE" ]] || die "keystore not found at $KEYSTORE (run ./generate-keystore.sh)"
    [[ -f "$KEYSTORE_PROPS" ]] || die "keystore.properties not found at $KEYSTORE_PROPS (run ./generate-keystore.sh)"
    local key
    for key in storePassword keyPassword keyAlias; do
        grep -q "^${key}=" "$KEYSTORE_PROPS" || die "keystore.properties is missing '$key'"
    done
}

# The Rust core (installers/bia-deploy-mobile) as libbia_deploy_mobile.so per ABI, and the
# Kotlin it is called through.
build_core() {
    if [[ -n "${BIA_SKIP_CORE:-}" && -d "$ROOT_DIR/app/src/main/jniLibs" && -d "$ROOT_DIR/app/src/main/java/uniffi" ]]; then
        echo "== Rust core: kept from the previous build (BIA_SKIP_CORE)"
        return
    fi
    command -v cargo-ndk >/dev/null || die "cargo-ndk missing: cargo install cargo-ndk"
    rustup target add aarch64-linux-android x86_64-linux-android >/dev/null
    echo "== Rust core for arm64-v8a and x86_64"
    rm -rf "$ROOT_DIR/app/src/main/jniLibs"
    # no build machine's home path in the libraries (release.sh scans for it)
    (cd "$REPO" && RUSTFLAGS="--remap-path-prefix=$HOME=/home --remap-path-prefix=$REPO=/bia" \
        cargo ndk -t arm64-v8a -t x86_64 -P 26 -o "$ROOT_DIR/app/src/main/jniLibs" \
        build --profile release-mobile -p bia-deploy-mobile --lib)
    echo "== Kotlin bindings"
    # from an unstripped host build: the phone libraries carry no symbols to read them from
    local ext=so
    [[ "$(uname)" == Darwin ]] && ext=dylib
    rm -rf "$ROOT_DIR/app/src/main/java/uniffi"
    (cd "$REPO" && cargo build -q -p bia-deploy-mobile --lib && \
        cargo run -q -p bia-deploy-mobile --features bindgen --bin uniffi-bindgen -- generate \
        --library "$REPO/target/debug/libbia_deploy_mobile.$ext" \
        --language kotlin --no-format --out-dir "$ROOT_DIR/app/src/main/java")
}

prepare() {
    setup_java
    setup_sdk
    check_signing
    build_core
    mkdir -p "$DIST"
    cd "$ROOT_DIR"
}
