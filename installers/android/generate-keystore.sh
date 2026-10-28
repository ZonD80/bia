#!/usr/bin/env bash
# The Google Play upload key of bia deploy: its own keystore, not shared with other apps.
# Use it with Play App Signing (Google keeps the app signing key; a lost upload key can be
# reset). Back up ~/biaDeployPlayKeys.jks and keystore.properties somewhere safe.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
KEYSTORE="${KEYSTORE:-$HOME/biaDeployPlayKeys.jks}"
KEYSTORE_PROPS="$ROOT_DIR/keystore.properties"
ALIAS="${ALIAS:-bia-deploy-upload}"

die() {
    echo "error: $*" >&2
    exit 1
}

find_keytool() {
    local candidate
    for candidate in \
        "/Applications/Android Studio.app/Contents/jbr/Contents/Home/bin/keytool" \
        "$HOME/Applications/Android Studio.app/Contents/jbr/Contents/Home/bin/keytool" \
        "${JAVA_HOME:-}/bin/keytool"; do
        if [[ -x "$candidate" ]]; then
            echo "$candidate"
            return 0
        fi
    done
    command -v keytool || die "keytool not found (install Android Studio or set JAVA_HOME)"
}

KEYTOOL="$(find_keytool)"

[[ ! -f "$KEYSTORE" ]] || die "$KEYSTORE already exists; bia deploy's upload key is made once (delete it only if it was never used)"
[[ ! -f "$KEYSTORE_PROPS" ]] || die "$KEYSTORE_PROPS already exists"

# PKCS12 keystores use one password for the store and the key
password="$(openssl rand -base64 48 | tr -d '/+=\n' | head -c 32)"
umask 077

"$KEYTOOL" -genkeypair -v \
    -storetype PKCS12 \
    -keystore "$KEYSTORE" \
    -alias "$ALIAS" \
    -keyalg RSA \
    -keysize 4096 \
    -validity 10000 \
    -storepass "$password" \
    -keypass "$password" \
    -dname "CN=bia deploy, OU=Mobile, O=AENGIX S.L., L=Barcelona, ST=Barcelona, C=ES"

cat > "$KEYSTORE_PROPS" <<PROPS
storePassword=$password
keyPassword=$password
keyAlias=$ALIAS
PROPS

echo
echo "Keystore: $KEYSTORE"
echo "Alias:    $ALIAS"
echo "Passwords: $KEYSTORE_PROPS (gitignored)"
echo
echo "Upload certificate fingerprint (Play Console shows the same under App integrity):"
"$KEYTOOL" -list -v -keystore "$KEYSTORE" -alias "$ALIAS" -storepass "$password" 2>/dev/null | grep -E 'SHA256:'
echo
echo "Back up $KEYSTORE and $KEYSTORE_PROPS (a password manager or an encrypted disk)."
