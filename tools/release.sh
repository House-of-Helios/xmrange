#!/bin/bash
# One GitHub release of XMRange (the github-publish skill, stage 3), in order:
#   1. refuse: a dirty tree, an existing release of this version, a signing key that doesn't match
#      RELEASE_PUBKEY in update.rs (a mismatch would strand every installed app)
#   2. tools/publish.sh: the filtered `public` branch, scanned, pushed to GitHub's main
#   3. tools/check-release.sh for both kinds of Mac, then one universal app
#      (tools/build-release-app.sh, ARCH=universal) in one unsigned DMG, XMRange.dmg: the file people
#      download and the file the app updater takes. Its SHA-256 and an Ed25519 signature over
#      "<version>\n<sha256>", verified against the app key
#   4. appcast.json (the feed the app reads), then the GitHub release, tagged on the `public` commit
#   5. verify by download: checksum, signature, DMG contents, codesign, both arches, version
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

# 3. The app: one universal bundle, one DMG (owner, 2026-11-25: one app, DMG only).
OUT="$ROOT/_devdata/release/$VERSION"
rm -rf "$OUT" && mkdir -p "$OUT"
for arch in arm64 x86_64; do ARCH=$arch tools/check-release.sh; done
APP="$(ARCH=universal tools/build-release-app.sh)"
if [ "$VIS" != PRIVATE ] && grep -aqE '(github_pat|gh[pousr])_[A-Za-z0-9_]{20,}' "$APP/Contents/MacOS/"*; then
  die "public repo, but the build carries a GitHub token"
fi
# A disk image with the app and an Applications shortcut to drag it onto. No version in its name,
# so releases/latest/download/XMRange.dmg never changes. The DMG stays unsigned (Archivist's
# lesson): Gatekeeper refuses a disk image signed by an identity Apple doesn't know, but mounts an
# unsigned one. Only the app inside is signed.
DMG="$OUT/XMRange.dmg"
STAGE="$(mktemp -d "$OUT/dmg.XXXX")"
ditto "$APP" "$STAGE/XMRange.app"
ln -s /Applications "$STAGE/Applications"
hdiutil create -volname "XMRange" -srcfolder "$STAGE" -ov -format UDZO "$DMG" >/dev/null
rm -rf "$STAGE"
(cd "$OUT" && shasum -a 256 XMRange.dmg > XMRange.dmg.sha256)
SHA="$(cut -d' ' -f1 "$DMG.sha256")"
SIG="$(sign sign "$KEY" "$VERSION" "$SHA")"
sign verify "$PUB_APP" "$VERSION" "$SHA" "$SIG" >/dev/null || die "the signature does not verify with the app's key"
ASSET="{\"file\":\"XMRange.dmg\",\"sha256\":\"$SHA\",\"sig\":\"$SIG\"}"
echo "release: $DMG"

# 4. The feed, then the release. Both kinds of Mac point at the one universal DMG.
NOTES_JSON="$(python3 -c 'import json,sys; print(json.dumps(sys.argv[1]))' "$NOTES")"
printf '{"version":"%s","notes":%s,"assets":{"arm64":%s,"x86_64":%s}}\n' "$VERSION" "$NOTES_JSON" "$ASSET" "$ASSET" > "$OUT/appcast.json"
python3 -c 'import json,sys; json.load(open(sys.argv[1]))' "$OUT/appcast.json" || die "appcast.json is not valid JSON"
BODY="$OUT/notes.md"
cat > "$BODY" <<EOF
$NOTES

**Install:** download **XMRange.dmg** (every Mac, Apple silicon and Intel), open it, drag XMRange onto Applications. The app is ad-hoc signed (no Apple Developer account), so macOS asks once: see the README's "open anyway" steps.
EOF
gh release create "$TAG" --repo "$REPO" --target "$PUBLIC_SHA" --title "XMRange $VERSION" --notes-file "$BODY" \
  "$DMG" "$DMG.sha256" "$OUT/appcast.json"

# 5. Verify by download, from GitHub itself.
DL="$(mktemp -d "$ROOT/_devdata/release/verify-$VERSION.XXXX")"
gh release download "$TAG" --repo "$REPO" --dir "$DL"
(cd "$DL" && shasum -a 256 -c XMRange.dmg.sha256)
for arch in arm64 x86_64; do
  read -r FILE FSHA FSIG < <(python3 -c 'import json,sys; a=json.load(open(sys.argv[1]))["assets"][sys.argv[2]]; print(a["file"],a["sha256"],a["sig"])' "$DL/appcast.json" "$arch")
  [ "$FILE" = XMRange.dmg ] && [ "$(shasum -a 256 "$DL/$FILE" | cut -d' ' -f1)" = "$FSHA" ] || die "$arch: the feed does not match the downloaded DMG"
  sign verify "$PUB_APP" "$VERSION" "$FSHA" "$FSIG" >/dev/null || die "$arch: downloaded feed's signature does not verify"
done
hdiutil attach -nobrowse -readonly -mountpoint "$DL/dmg" "$DL/XMRange.dmg" >/dev/null
ok=1
M="$DL/dmg/XMRange.app"
[ -d "$M" ] && [ -L "$DL/dmg/Applications" ] && codesign --verify --deep --strict "$M" || ok=0
V="$(/usr/libexec/PlistBuddy -c 'Print CFBundleShortVersionString' "$M/Contents/Info.plist" 2>/dev/null || true)"
[ "$V" = "$VERSION" ] || ok=0
for f in "$M/Contents/MacOS/"*; do lipo "$f" -verify_arch arm64 x86_64 || ok=0; done
hdiutil detach "$DL/dmg" >/dev/null
[ "$ok" = 1 ] || die "the downloaded DMG does not hold a verified universal XMRange.app $VERSION and the Applications shortcut"
echo "✅ $TAG released and verified from GitHub: https://github.com/$REPO/releases/tag/$TAG"

# 6. Clean up (owner, 2026-11-25): build leftovers are temporary. GitHub now holds the release
#    files, and every target/ folder is rebuilt by the next build. A failed run keeps its files for
#    a look; this step runs only after the download check passed.
rm -rf "$ROOT/target" "$OUT" "$DL"
echo "🧹 removed target/ and the local release files"
