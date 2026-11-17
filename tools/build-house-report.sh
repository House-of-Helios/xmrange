#!/bin/bash
# house-report (crates/house-report): the House's one feedback sender, shared by XMRange and
# Bookkeeper. Universal (arm64 + x86_64), --locked, no repo or home paths in the binary, system
# libraries only. Output: vendor/universal/house-report (Bookkeeper copies it from there).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
export PATH=/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH
export MACOSX_DEPLOYMENT_TARGET=10.15
export RUSTFLAGS="--remap-path-prefix=$ROOT=xmrange --remap-path-prefix=$HOME=~"
for t in aarch64-apple-darwin x86_64-apple-darwin; do
  cargo build -q --release --locked -p house-report --target "$t"
done
OUT="vendor/universal/house-report"
lipo -create -output "$OUT.tmp" target/aarch64-apple-darwin/release/house-report target/x86_64-apple-darwin/release/house-report
codesign --force --sign - "$OUT.tmp" 2>/dev/null
if otool -L "$OUT.tmp" | grep -E "^\s+/" | grep -v -e "^\s*/usr/lib/" -e "^\s*/System/Library/"; then
  echo "❌ house-report links a non-system library"; exit 1
fi
if strings -a "$OUT.tmp" | grep -qE "/Users/|$(whoami)"; then echo "❌ house-report carries a home path"; exit 1; fi
mv -f "$OUT.tmp" "$OUT"
echo "📮 $OUT ($(lipo -archs "$OUT")) sha256 $(shasum -a 256 "$OUT" | cut -c1-16)"
