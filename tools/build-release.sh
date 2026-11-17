#!/bin/bash
# The one release-build recipe: remapped paths (no repo/home paths baked into the binaries), so
# the binaries tools/check-release.sh inspects are exactly the binaries any shipping script ships.
# Builds the CLI and the desktop app, both WITHOUT dev-diagnostics.
# Prints the built binaries' paths on stdout, one per line: the CLI first, then the app.
set -euo pipefail
export PATH=/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
# ARCH=x86_64 builds the Intel Mac binaries (Stage 3: a separate Intel app), for macOS 10.15.
if [ "${ARCH:-arm64}" = x86_64 ]; then
  MACOSX_DEPLOYMENT_TARGET=10.15 RUSTFLAGS="--remap-path-prefix=$HOME=~ --remap-path-prefix=$ROOT=." \
    cargo build -q --release --target x86_64-apple-darwin -p xmrange-cli -p xmrange-app
  OUT="$ROOT/target/x86_64-apple-darwin/release"
else
  RUSTFLAGS="--remap-path-prefix=$HOME=~ --remap-path-prefix=$ROOT=." cargo build -q --release -p xmrange-cli -p xmrange-app
  OUT="$ROOT/target/release"
fi
echo "$OUT/xmrange"
echo "$OUT/xmrange-app"
