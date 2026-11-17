#!/bin/bash
# The one release bundle recipe: builds xmrange-app WITHOUT dev-diagnostics, remapped paths (as
# build-release.sh) and MACOSX_DEPLOYMENT_TARGET=11.3 (P5-R6, the declared floor), then assembles
# XMRange.app with the release bundle id, the workspace version, and the patched xmrig, p2pool and
# Arti beside the app binary (paths.rs' release lookups use Contents/MacOS, beside itself).
# Uses only macOS built-ins (sips, iconutil, codesign) plus cargo. No sudo, installs nothing.
#
# Prints the built bundle's path on stdout.
set -euo pipefail
export PATH=/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# ARCH=x86_64: the Intel Mac app (Stage 3), from the x86_64 helpers (tools/build-x86_64.sh), for
# macOS 10.15. Default: Apple Silicon, macOS 11.3.
ARCH="${ARCH:-arm64}"
if [ "$ARCH" = x86_64 ]; then
  XMRIG="$ROOT/vendor/xmrig-mo/build-x86_64/xmrig"
  P2POOL="$ROOT/vendor/p2pool-x86_64/build/p2pool"
  ARTI="$ROOT/vendor/arti/target/x86_64-apple-darwin/release/arti"
  MONEROD="$ROOT/vendor/monero/build-out/x86_64/monerod"
  MIN_OS=10.15
  CARGO_TARGET=(--target x86_64-apple-darwin)
  BIN_DIR=x86_64-apple-darwin/release
else
  XMRIG="$ROOT/vendor/xmrig-mo/build/xmrig"
  P2POOL="$ROOT/vendor/p2pool/build/p2pool"
  ARTI="$ROOT/vendor/arti/target/release/arti"
  MONEROD="$ROOT/vendor/monero/build-out/arm64/monerod"
  MIN_OS=11.3
  CARGO_TARGET=()
  BIN_DIR=release
fi
for b in "$XMRIG:build-xmrig (x86_64: build-x86_64)" "$P2POOL:build-p2pool (x86_64: build-x86_64)" "$ARTI:build-arti (x86_64: build-x86_64)" "$MONEROD:build-monerod"; do
  if [ ! -x "${b%%:*}" ]; then
    echo "${b%%:*} not built: run tools/${b##*:}.sh first" >&2
    exit 1
  fi
done

TARGET="$ROOT/target/release-app-$ARCH"   # own target dir: never thrashes the dev-app or plain release builds
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
APP="$TARGET/XMRange.app"

MACOSX_DEPLOYMENT_TARGET=$MIN_OS \
  RUSTFLAGS="--remap-path-prefix=$HOME=~ --remap-path-prefix=$ROOT=." \
  cargo build -q --release -p xmrange-app --target-dir "$TARGET" ${CARGO_TARGET[@]+"${CARGO_TARGET[@]}"}

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$TARGET/$BIN_DIR/xmrange-app" "$APP/Contents/MacOS/xmrange-app"
cp "$XMRIG" "$APP/Contents/MacOS/xmrig"
# Stage 2 (doctrine §1a): p2pool and Arti (Tor) beside xmrig, where paths.rs' release `bundled` looks.
cp "$P2POOL" "$APP/Contents/MacOS/p2pool"
cp "$ARTI" "$APP/Contents/MacOS/arti"
# Stage 2 (Plan 6): XMRange's own Monero node, for users who don't run the Monero GUI.
cp "$MONEROD" "$APP/Contents/MacOS/monerod"

# Icon: the app icon PNG → .icns through an iconset (same recipe as make-dev-app.sh).
ICONSET="$TARGET/XMRange.iconset"
rm -rf "$ICONSET" && mkdir -p "$ICONSET"
SRC="crates/xmrange-app/icons/icon.png"
for s in 16 32 128 256 512; do
  sips -z $s $s "$SRC" --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
  sips -z $((s * 2)) $((s * 2)) "$SRC" --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/XMRange.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>XMRange</string>
  <key>CFBundleDisplayName</key><string>XMRange</string>
  <key>CFBundleIdentifier</key><string>house.helios.xmrange</string>
  <key>CFBundleExecutable</key><string>xmrange-app</string>
  <key>CFBundleIconFile</key><string>XMRange</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>LSMinimumSystemVersion</key><string>${MIN_OS}</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.utilities</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

# Ad-hoc sign: no Developer ID until Stage 4 (owner decision, 2026-09-27).
codesign --force --deep --sign - "$APP" >/dev/null 2>&1
echo "$APP"
