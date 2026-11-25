#!/bin/bash
# Deletes everything in vendor/ except the finished helper binaries the app bundles (and the two
# p2pool source files its tests read). Every helper build script re-downloads its pinned source
# when the folder holds no git checkout, so source and build scratch are temporary: run this after
# a helper build (owner, 2026-11-25: vendor/ had grown to 7.5 GB for about 80 MB of helpers).
set -euo pipefail
V="$(cd "$(dirname "$0")/.." && pwd)/vendor"
HOLD="$V/.prune-hold"
KEEP=(
  xmrig-mo/build/xmrig xmrig-mo/build-x86_64/xmrig
  p2pool/build/p2pool p2pool-x86_64/build/p2pool
  p2pool/src/params.cpp p2pool/src/p2p_server.cpp   # read by crates/xmrange-core/src/p2pool.rs tests
  arti/target/release/arti arti/target/x86_64-apple-darwin/release/arti
  monero/build-out/arm64/monerod monero/build-out/x86_64/monerod
  universal/house-report                            # Bookkeeper copies it from here
)

if pgrep -xq 'cargo|rustc|cmake|make|clang'; then echo "a build is running: wait for it" >&2; exit 1; fi
# A stopped earlier run left the kept files here: never delete them, put them back by hand.
[ ! -e "$HOLD" ] || { echo "$HOLD exists (an earlier prune stopped): move its files back into vendor/ first" >&2; exit 1; }
for k in "${KEEP[@]}"; do
  [ -f "$V/$k" ] || { echo "missing $k: build it before pruning (nothing deleted)" >&2; exit 1; }
done

for k in "${KEEP[@]}"; do mkdir -p "$HOLD/$(dirname "$k")"; mv "$V/$k" "$HOLD/$k"; done
find "$V" -mindepth 1 -maxdepth 1 ! -name .prune-hold -exec rm -rf {} +
find "$HOLD" -mindepth 1 -maxdepth 1 -exec mv {} "$V/" \;
rmdir "$HOLD"

for k in "${KEEP[@]}"; do [ -f "$V/$k" ] || { echo "LOST $k" >&2; exit 1; }; done
du -sh "$V"
