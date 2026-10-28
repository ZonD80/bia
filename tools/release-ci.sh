#!/usr/bin/env bash
# Release through GitHub Actions: commit everything as "<version>" (the workspace version in
# Cargo.toml), tag v<version> and push both. The pushed tag starts
# .github/workflows/release.yml, which builds every package and publishes the release
# (tools/release.sh on GitHub's runners).
#
#   tools/release-ci.sh [-y] [--dry-run] [--retag]
#
#   -y         do not ask before committing and pushing
#   --dry-run  show what would be committed, tagged and pushed; change nothing
#   --retag    the tag exists already: move it to the new commit (deletes it locally and on
#              GitHub first) to run the release again for the same version
set -euo pipefail
cd "$(dirname "$0")/.."

YES=0 DRY=0 RETAG=0
die() { echo "error: $*" >&2; exit 1; }
while [[ $# -gt 0 ]]; do
  case $1 in
    -y|--yes) YES=1 ;;
    --dry-run) DRY=1 ;;
    --retag) RETAG=1 ;;
    -h|--help) awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' "$0"; exit 0 ;;
    *) die "unknown option $1 (see --help)" ;;
  esac
  shift
done

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
[[ -n "$VERSION" ]] || die "no version in Cargo.toml"
TAG="v$VERSION"
BRANCH=$(git rev-parse --abbrev-ref HEAD)
[[ "$BRANCH" != HEAD ]] || die "not on a branch (detached HEAD)"

# the release text bia and GitHub show comes from here
grep -qx "## $VERSION" RELEASE_NOTES.md || die "RELEASE_NOTES.md has no '## $VERSION' section"

git fetch -q origin --tags
if git rev-parse -q --verify "refs/tags/$TAG" >/dev/null || git ls-remote --exit-code --tags origin "refs/tags/$TAG" >/dev/null 2>&1; then
  [[ $RETAG -eq 1 ]] || die "tag $TAG exists already (bump the version in Cargo.toml, or --retag to move it)"
fi

# what goes into the commit: everything not ignored
CHANGES=$(git status --porcelain)
if [[ -n "$CHANGES" ]]; then
  # keys and credentials never go in, even when a .gitignore misses them
  bad=$(git status --porcelain --untracked-files=all | cut -c4- | grep -E '(^|/)(\.env|fmm\.json|keystore\.properties|chatgpt-client\.json)$|\.(p12|pfx|jks|keystore|mobileprovision|pem|key)$' || true)
  [[ -z "$bad" ]] || die "these look like keys or credentials; ignore or remove them first:
$bad"
fi

echo "== release $TAG from $BRANCH"
if [[ -n "$CHANGES" ]]; then
  echo "   commit \"$VERSION\" with:"
  git status --short --untracked-files=all | sed 's/^/     /'
else
  echo "   nothing to commit: tagging $(git rev-parse --short HEAD)"
fi
[[ $RETAG -eq 1 ]] && echo "   move tag $TAG (delete it locally and on origin, tag again)"
echo "   push $BRANCH and $TAG to origin (starts the release workflow)"
[[ $DRY -eq 0 ]] || exit 0

if [[ $YES -eq 0 ]]; then
  [[ -t 0 ]] || die "not a terminal: pass -y to go on without asking"
  read -r -p "Go on? [y/N] " a
  [[ $a == y || $a == Y ]] || die "stopped"
fi

if [[ -n "$CHANGES" ]]; then
  git add -A
  git commit -q -m "$VERSION"
fi
git push origin "$BRANCH"
if [[ $RETAG -eq 1 ]]; then
  git tag -d "$TAG" 2>/dev/null || true
  git push origin ":refs/tags/$TAG" 2>/dev/null || true
fi
git tag "$TAG"
git push origin "$TAG"

echo "== pushed $TAG ($(git rev-parse --short HEAD)); the release workflow starts on GitHub:"
url=$(git remote get-url origin | sed -E 's#^git@github.com:#https://github.com/#; s#\.git$##')
echo "   $url/actions/workflows/release.yml"
if command -v gh >/dev/null && gh auth status >/dev/null 2>&1; then
  sleep 5
  gh run list --workflow release.yml -L 1 2>/dev/null || true
fi
