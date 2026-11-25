#!/bin/bash
# Reproducible Arti (the Tor Project's Rust Tor client) build: pinned tag, default features
# (onion-service client included: p2pool finds peers through its built-in onion seeds).
# Links only system libraries and frameworks, so it runs on any Apple Silicon Mac.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/vendor/arti"
TAG="arti-v2.6.0"
PIN="a71097fdf7b141b56d1eb3709628ee38d232c9d1"
export PATH=/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH

if [ ! -d "$SRC/.git" ]; then
  rm -rf "$SRC"   # pruned by tools/prune-vendor.sh: only finished binaries were left
  git clone --depth 1 --branch "$TAG" https://gitlab.torproject.org/tpo/core/arti.git "$SRC"
fi
[ "$(git -C "$SRC" rev-parse HEAD)" = "$PIN" ] || { echo "arti is not at $TAG ($PIN)"; exit 1; }

# --remap-path-prefix: no repo or home paths (the username) baked into the shipped binary.
(cd "$SRC" && MACOSX_DEPLOYMENT_TARGET=11.3 RUSTFLAGS="--remap-path-prefix=$SRC=arti --remap-path-prefix=$HOME=~" \
  cargo build --release --locked -p arti >/dev/null 2>&1)
"$SRC/target/release/arti" --version | head -1
if otool -L "$SRC/target/release/arti" | tail -n +2 | grep -v -e "^\s*/usr/lib/" -e "^\s*/System/Library/"; then
  echo "arti links a non-system library"; exit 1
fi
