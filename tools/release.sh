#!/bin/bash
# One GitHub release of XMRange (the github-publish skill, stage 3), in order:
#   1. refuse: a dirty tree, an existing release of this version, a signing key that doesn't match
#      RELEASE_PUBKEY in update.rs (a mismatch would strand every installed app)
#   2. tools/publish.sh: the filtered `public` branch, scanned, pushed to GitHub's main
#   3. for each kind of Mac: tools/check-release.sh, tools/build-release-app.sh, a ditto zip, its
#      SHA-256, an Ed25519 signature over "<version>\n<sha256>", verified against the app's key;
#      and a DMG for people (the zip is for the app's updater only)
#   4. appcast.json (the feed the app reads), then the GitHub release, tagged on the `public` commit
#   5. verify by download: checksums, signatures, codesign, version, DMG contents, from GitHub itself
#   6. clean up: target/ and the local release files go (GitHub holds them)
#
# While the repo is private, the app reads releases with a read-only, single-repo token compiled in
# from _devdata/github-read-token (owner ruling 2026-11-16). Once the repo is public, no token is
# compiled in, and a build that carries one is refused.
#
# Usage:  NOTES="what changed, one line" tools/release.sh
set -euo pipefail
export PATH=/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
die() { echo "❌ $*" >&2; exit 1; }

REPO="House-of-Helios/xmrange"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
TAG="v$VERSION"
NOTES="${NOTES:?NOTES=\"what changed\" tools/release.sh}"
KEY="$ROOT/_devdata/release-signing.key"
TOKEN_FILE="$ROOT/_devdata/github-read-token"
sign() { swift tools/release-sign.swift "$@"; }

# 1. Refusals.
[ -z "$(git status --porcelain)" ] || die "uncommitted changes — commit first"
[ -f "$KEY" ] || die "no signing key at _devdata/release-signing.key"
PUB_APP="$(sed -n 's/^pub const RELEASE_PUBKEY: &str = "\([0-9a-f]*\)";/\1/p' crates/xmrange-core/src/update.rs)"
[ -n "$PUB_APP" ] && [ "$(sign public "$KEY")" = "$PUB_APP" ] || die "the signing key does not match RELEASE_PUBKEY in update.rs"
! gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1 || die "$TAG is already released"
VIS="$(gh repo view "$REPO" --json visibility --jq .visibility)"
if [ "$VIS" = PRIVATE ]; then
  [ -s "$TOKEN_FILE" ] || die "private repo: put a read-only token for $REPO in _devdata/github-read-token"
  XMRANGE_GITHUB_READ_TOKEN="$(tr -d '[:space:]' < "$TOKEN_FILE")"
  export XMRANGE_GITHUB_READ_TOKEN
else
  unset XMRANGE_GITHUB_READ_TOKEN
fi

# 2. The code.
tools/publish.sh "XMRange $VERSION"
PUBLIC_SHA="$(git rev-parse public)"

# 3. The apps.
OUT="$ROOT/_devdata/release/$VERSION"
mkdir -p "$OUT"
ASSETS=()
FEED_ASSETS=""
for arch in arm64 x86_64; do
  name=$([ "$arch" = arm64 ] && echo apple-silicon || echo intel)
  ARCH=$arch tools/check-release.sh
  APP="$(ARCH=$arch tools/build-release-app.sh)"
  if [ "$VIS" != PRIVATE ] && grep -aqE '(github_pat|gh[pousr])_[A-Za-z0-9_]{20,}' "$APP/Contents/MacOS/"*; then
    die "public repo, but the $arch build carries a GitHub token"
  fi
  ZIP="$OUT/XMRange-$VERSION-$name.zip"
  rm -f "$ZIP"
  ditto -c -k --sequesterRsrc --keepParent "$APP" "$ZIP"
  SHA="$(shasum -a 256 "$ZIP" | cut -d' ' -f1)"
  (cd "$OUT" && shasum -a 256 "$(basename "$ZIP")" > "$(basename "$ZIP").sha256")
  SIG="$(sign sign "$KEY" "$VERSION" "$SHA")"
  sign verify "$PUB_APP" "$VERSION" "$SHA" "$SIG" >/dev/null || die "$arch: signature does not verify with the app's key"
  ASSETS+=("$ZIP" "$ZIP.sha256")
  # For people: a disk image with the app and an Applications shortcut to drag it onto. No version
  # in its name, so releases/latest/download/XMRange-<kind>.dmg is a link that never changes. The
  # DMG stays unsigned (Archivist's lesson): Gatekeeper refuses a disk image signed by an identity
  # Apple doesn't know, but mounts an unsigned one. Only the app inside is signed.
  DMG="$OUT/XMRange-$name.dmg"
  STAGE="$(mktemp -d "$OUT/dmg.XXXX")"
  ditto "$APP" "$STAGE/XMRange.app"
  ln -s /Applications "$STAGE/Applications"
  hdiutil create -volname "XMRange" -srcfolder "$STAGE" -ov -format UDZO "$DMG" >/dev/null
  rm -rf "$STAGE"
  (cd "$OUT" && shasum -a 256 "$(basename "$DMG")" > "$(basename "$DMG").sha256")
  ASSETS+=("$DMG" "$DMG.sha256")
  FEED_ASSETS="$FEED_ASSETS\"$arch\":{\"file\":\"$(basename "$ZIP")\",\"sha256\":\"$SHA\",\"sig\":\"$SIG\"},"
  echo "release: $ZIP"
done

# 4. The feed, then the release.
NOTES_JSON="$(python3 -c 'import json,sys; print(json.dumps(sys.argv[1]))' "$NOTES")"
printf '{"version":"%s","notes":%s,"assets":{%s}}\n' "$VERSION" "$NOTES_JSON" "${FEED_ASSETS%,}" > "$OUT/appcast.json"
python3 -c 'import json,sys; json.load(open(sys.argv[1]))' "$OUT/appcast.json" || die "appcast.json is not valid JSON"
BODY="$OUT/notes.md"
cat > "$BODY" <<EOF
$NOTES

- **Apple silicon Macs:** \`XMRange-apple-silicon.dmg\`
- **Intel Macs:** \`XMRange-intel.dmg\`

Open the DMG and drag XMRange onto Applications. Each file has a \`.sha256\` beside it; the zips are for the app's own updater. The app is ad-hoc signed (no Apple Developer account), so macOS asks once: see the README's "open anyway" steps.
EOF
gh release create "$TAG" --repo "$REPO" --target "$PUBLIC_SHA" --title "XMRange $VERSION" --notes-file "$BODY" \
  "${ASSETS[@]}" "$OUT/appcast.json"

# 5. Verify by download, from GitHub itself.
DL="$(mktemp -d "$ROOT/_devdata/release/verify-$VERSION.XXXX")"
gh release download "$TAG" --repo "$REPO" --dir "$DL"
(cd "$DL" && shasum -a 256 -c ./*.sha256)
for arch in arm64 x86_64; do
  read -r FILE SHA SIG < <(python3 -c 'import json,sys; a=json.load(open(sys.argv[1]))["assets"][sys.argv[2]]; print(a["file"],a["sha256"],a["sig"])' "$DL/appcast.json" "$arch")
  [ "$(shasum -a 256 "$DL/$FILE" | cut -d' ' -f1)" = "$SHA" ] || die "$arch: downloaded zip does not match the feed"
  sign verify "$PUB_APP" "$VERSION" "$SHA" "$SIG" >/dev/null || die "$arch: downloaded feed's signature does not verify"
  mkdir "$DL/$arch"
  ditto -x -k "$DL/$FILE" "$DL/$arch"
  codesign --verify --deep --strict "$DL/$arch/XMRange.app" || die "$arch: codesign does not verify"
  V="$(/usr/libexec/PlistBuddy -c 'Print CFBundleShortVersionString' "$DL/$arch/XMRange.app/Contents/Info.plist")"
  [ "$V" = "$VERSION" ] || die "$arch: bundle says $V, not $VERSION"
  name=$([ "$arch" = arm64 ] && echo apple-silicon || echo intel)
  hdiutil attach -nobrowse -readonly -mountpoint "$DL/dmg-$arch" "$DL/XMRange-$name.dmg" >/dev/null
  ok=1
  [ -d "$DL/dmg-$arch/XMRange.app" ] && [ -L "$DL/dmg-$arch/Applications" ] \
    && codesign --verify --deep --strict "$DL/dmg-$arch/XMRange.app" || ok=0
  hdiutil detach "$DL/dmg-$arch" >/dev/null
  [ "$ok" = 1 ] || die "$arch: the DMG does not hold a verified XMRange.app and the Applications shortcut"
done
echo "✅ $TAG released and verified from GitHub: https://github.com/$REPO/releases/tag/$TAG"

# 6. Clean up (owner, 2026-11-25): build leftovers are temporary. GitHub now holds the release
#    files, and every target/ folder is rebuilt by the next build. A failed run keeps its files for
#    a look; this step runs only after the download check passed.
rm -rf "$ROOT/target" "$OUT" "$DL"
echo "🧹 removed target/ and the local release files"
