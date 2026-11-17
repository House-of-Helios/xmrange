#!/bin/bash
# Reproducible xmrig-mo build: pinned commit + XMRange patches (donate 0, private API reload).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/vendor/xmrig-mo"
PIN="0ce9664d3f540b72624e3733a592ddfdf02662a5"   # MoneroOcean xmrig 6.26.0-mo5
PATCH="$ROOT/patches/xmrig-mo-xmrange.patch"

if [ ! -d "$SRC/.git" ]; then
  git clone --filter=blob:none https://github.com/MoneroOcean/xmrig.git "$SRC"
fi
if [ "$(git -C "$SRC" rev-parse HEAD)" != "$PIN" ]; then
  git -C "$SRC" fetch --depth 1 origin "$PIN"
  git -C "$SRC" checkout --quiet "$PIN"
fi
if git -C "$SRC" apply --reverse --check "$PATCH" 2>/dev/null; then
  echo "patches already applied"
else
  git -C "$SRC" apply "$PATCH"
  echo "patches applied"
fi
grep -q "kMinimumDonateLevel = 0" "$SRC/src/donate.h"
grep -q "XMRange privacy patch" "$SRC/src/base/kernel/Base.cpp"

# WITH_HWLOC=OFF: hwloc's own NUMA/topology smarts aren't needed on Apple Silicon (one uniform
# package), and Homebrew's libhwloc.dylib wouldn't exist on Helios's other Macs anyway. Without it,
# `otool -L` shows only system libraries and /usr/lib (P5-R6). CMAKE_OSX_DEPLOYMENT_TARGET=11.3 is
# the declared floor (P5-R6): every bundled binary is built for macOS 11.3, never newer.
cmake -S "$SRC" -B "$SRC/build" -DCMAKE_BUILD_TYPE=Release \
  -DWITH_HWLOC=OFF -DCMAKE_OSX_DEPLOYMENT_TARGET=11.3 \
  -DOPENSSL_ROOT_DIR="$(brew --prefix openssl@3)" >/dev/null
cmake --build "$SRC/build" -j "$(sysctl -n hw.ncpu)" >/dev/null
"$SRC/build/xmrig" --version | head -1
