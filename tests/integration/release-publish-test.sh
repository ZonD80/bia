#!/usr/bin/env bash
# tools/release.sh --publish against a fake gh: a release becomes latest only when every package
# is there, a failed upload is retried and holds back bia-os.json, a re-run sends only what is
# missing or different, and an older version never takes latest back.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
FAKE=$WORK/fake
STATE=$WORK/state
REPO=$WORK/repo
mkdir -p "$FAKE" "$STATE/assets" "$REPO/tools" "$REPO/source/flash/MCPs/one"

cat >"$FAKE/gh" <<'EOF'
#!/usr/bin/env python3
import hashlib, os, shutil, sys
state = os.environ["FAKE_GH_STATE"]
assets = os.path.join(state, "assets")
a = sys.argv[1:]
def path(n): return os.path.join(state, n)
def log(line):
    with open(path("log"), "a") as f: f.write(line + "\n")
def names(): return sorted(os.listdir(assets))
if a[:2] == ["auth", "status"]: sys.exit(0)
if a[:2] == ["release", "view"]:
    if not os.path.exists(path("release")): sys.exit(1)
    print("https://example/release" if "url" in a else "\n".join(names()))
elif a[:2] == ["release", "create"]:
    log("create " + ("not-latest" if "--latest=false" in a else "LATEST"))
    open(path("release"), "w").close()
    if "--latest=false" not in a: open(path("latest"), "w").write(a[2])
elif a[:2] == ["release", "upload"]:
    f = a[-1]; n = os.path.basename(f); fail = path("fail-" + n)
    if os.path.exists(fail):
        left = int(open(fail).read())
        if left > 0:
            open(fail, "w").write(str(left - 1))
            log("upload-failed " + n)
            sys.exit(1)
    shutil.copy(f, os.path.join(assets, n))
    log("upload " + n)
elif a[:2] == ["release", "edit"]:
    open(path("latest"), "w").write(a[2]); log("latest " + a[2])
elif a[:2] == ["release", "download"]:
    src = os.path.join(assets, a[a.index("-p") + 1]); out = a[a.index("-O") + 1]
    if out == "-": sys.stdout.buffer.write(open(src, "rb").read())
    else: shutil.copy(src, out)
elif a[0] == "api" and a[1].endswith("/releases/latest"):
    if not os.path.exists(path("latest")): sys.exit(1)
    print(open(path("latest")).read())
elif a[0] == "api":
    for n in names():
        print(n + "\tsha256:" + hashlib.sha256(open(os.path.join(assets, n), "rb").read()).hexdigest())
else:
    sys.exit("fake gh: " + " ".join(a))
EOF
chmod +x "$FAKE/gh"
export PATH="$FAKE:$PATH" FAKE_GH_STATE=$STATE RELEASE_UPLOAD_WAIT=0

cp "$ROOT/tools/release.sh" "$REPO/tools/release.sh"
echo '{}' >"$REPO/source/flash/MCPs/one/mcp.json"
cd "$REPO"
git init -q .
git -c user.name=t -c user.email=t@t commit -q --allow-empty -m init

fail() { echo "FAIL: $*"; echo "--- log"; cat "$STATE/log" 2>/dev/null || true; echo "--- output"; cat "$WORK/out" 2>/dev/null || true; exit 1; }
set_version() { # version: a fresh release folder, as on a new publish runner
  VER=$1 OUT=$REPO/dist/release/v$1
  printf '[workspace.package]\nversion = "%s"\n' "$1" >Cargo.toml
  printf '## %s\n\n### Changes\n\n- test\n' "$1" >RELEASE_NOTES.md
  rm -rf "$REPO/dist"
  mkdir -p "$OUT"
}
built() { # file…: one .built-* record with these files (content follows the name unless it exists)
  local rec f
  rec=$OUT/.built-$RANDOM
  for f in "$@"; do
    [[ -f "$OUT/$f" ]] || echo "content of $f" >"$OUT/$f"
    echo "file $f" >>"$rec"
    case $f in
      bia-x86_64.img.xz) echo '{"image": {}}' >"$OUT/os-x86_64.json"; echo "os x86_64" >>"$rec" ;;
      bia-arm64.img.xz) echo '{"image": {}}' >"$OUT/os-arm64.json"; echo "os arm64" >>"$rec" ;;
      bia-deploy-linux-*.tar.gz) echo '{}' >"$OUT/tool-$f.json"; echo "tool $f" >>"$rec" ;;
      mcp-one.tar.gz) echo '{}' >"$OUT/mcpentry-one.json"; echo "mcp one" >>"$rec" ;;
    esac
  done
}
publish() { bash tools/release.sh --publish --allow-dirty >"$WORK/out" 2>&1; }
uploads() { grep -c "^upload $1\$" "$STATE/log" || true; }
is_latest() { [[ "$(cat "$STATE/latest" 2>/dev/null)" == "$1" ]]; }

SOME=(bia-x86_64.img.xz bia-arm64.img.xz bia-land-x86_64.iso.xz bia-land-arm64.iso.xz
  bia-deploy-macos-universal.dmg bia-deploy-cli-macos-universal.zip
  bia-deploy-linux-x86_64.tar.gz bia-deploy-linux-arm64.tar.gz bia-deploy-windows-x86_64.zip
  takeover.sh mcp-one.tar.gz bia-deploy-android.apk bia-deploy-android.aab
  bia-deploy-ios-unsigned.ipa bia-bridge-macos-universal.zip bia-bridge-linux-x86_64.tar.gz
  bia-bridge-linux-arm64.tar.gz)
LAST=bia-bridge-windows-x86_64.zip

# 1. a package is missing: published, but not latest; the older latest stays
set_version 2.0.0
echo v1.9.9 >"$STATE/latest"
built "${SOME[@]}"
publish || fail "partial publish should succeed"
grep -q '^create not-latest$' "$STATE/log" || fail "release was not created as not-latest"
is_latest v1.9.9 || fail "a partial release took latest"
grep -q "NOT latest; still missing: bridge-windows" "$WORK/out" || fail "missing package not named"
echo "ok partial release is not latest"

# 2. a file fails every try: nothing more happens, bia-os.json waits, not latest
set_version 2.0.0
: >"$STATE/log"
echo 3 >"$STATE/fail-$LAST"
built "$LAST"
if publish; then fail "publish should fail when a file cannot be uploaded"; fi
[[ $(grep -c "^upload-failed $LAST\$" "$STATE/log") == 3 ]] || fail "expected three tries"
is_latest v1.9.9 || fail "failed upload took latest"
echo "ok failed upload stops short of latest"

# 3. a file fails twice, then goes up; the release is complete, so it becomes latest
set_version 2.0.0
: >"$STATE/log"
echo 2 >"$STATE/fail-$LAST"
built "$LAST"
publish || fail "publish should recover after two failed tries"
[[ $(uploads "$LAST") == 1 ]] || fail "file not uploaded after retries"
is_latest v2.0.0 || fail "complete release did not become latest"
echo "ok retry recovers, complete release is latest"

# 4. a re-run sends only what differs
set_version 2.0.0
: >"$STATE/log"
built takeover.sh
publish || fail "re-run of identical file"
[[ $(uploads takeover.sh) == 0 ]] || fail "identical file was uploaded again"
set_version 2.0.0
: >"$STATE/log"
echo changed >"$OUT/takeover.sh"
built takeover.sh
publish || fail "re-run of changed file"
[[ $(uploads takeover.sh) == 1 ]] || fail "changed file was not uploaded"
echo "ok re-run uploads only what differs"

# 5. bia-os.json goes up after every other file
set_version 2.0.0
: >"$STATE/log"
echo changed-too >"$OUT/bia-x86_64.img.xz"
built bia-x86_64.img.xz bia-land-arm64.iso.xz
echo changed-again >"$OUT/bia-land-arm64.iso.xz"
publish || fail "publish of image"
[[ "$(grep '^upload ' "$STATE/log" | tail -1)" == "upload bia-os.json" ]] || fail "bia-os.json was not last"
echo "ok bia-os.json is uploaded last"

# 6. completing an older version does not take latest back
echo v2.0.0 >"$STATE/latest"
set_version 1.9.5
rm -f "$STATE/release" "$STATE"/assets/* "$STATE"/fail-*
built "${SOME[@]}" "$LAST"
publish || fail "publish of older version"
is_latest v2.0.0 || fail "older version took latest"
grep -q "latest stays v2.0.0" "$WORK/out" || fail "no note about the newer latest"
echo "ok older version leaves latest alone"

# 7. 2.0.10 is newer than 2.0.9
echo v2.0.9 >"$STATE/latest"
set_version 2.0.10
rm -f "$STATE/release" "$STATE"/assets/*
built "${SOME[@]}" "$LAST"
publish || fail "publish of 2.0.10"
is_latest v2.0.10 || fail "2.0.10 did not replace 2.0.9"
echo "ok versions compare as versions"
echo PASS
