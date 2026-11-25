#!/bin/bash
# dmgbuild and its two libraries (ds_store, mac_alias), pure Python, from source at pinned tags.
# dmgbuild writes the DMG window's layout (.DS_Store: background picture, icon spots, window size)
# itself, so no script ever drives Finder (that would raise a macOS automation prompt).
# Nothing is installed: tools/release.sh runs it with PYTHONPATH pointing here.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
D="$ROOT/vendor/dmgbuild"
mkdir -p "$D"
get() {  # get <repo> <tag> <pin>
  local src="$D/$1"
  if [ ! -d "$src/.git" ]; then
    rm -rf "$src"
    git clone --quiet --depth 1 --branch "$2" "https://github.com/dmgbuild/$1.git" "$src" 2>/dev/null
  fi
  [ "$(git -C "$src" rev-parse HEAD)" = "$3" ] || { echo "$1 is not at $2 ($3)" >&2; exit 1; }
}
get dmgbuild  v2.0.0 7f80e3e44ea950e27d4fdf66628767dc3396a052
get ds_store  v1.3.4 e94c714c6d1cd25e17eae5597e405eec5ba184b9
get mac_alias v2.2.4 3d53f2e464b02735b030f6f6601ac48ceb596e36
# Each package reads its own version from install metadata (importlib.metadata); without an install,
# a minimal .dist-info per package on the path answers it.
for nv in dmgbuild:2.0.0 ds_store:1.3.4 mac_alias:2.2.4; do
  m="$D/meta/${nv%%:*}-${nv##*:}.dist-info"; mkdir -p "$m"
  printf 'Metadata-Version: 2.1\nName: %s\nVersion: %s\n' "${nv%%:*}" "${nv##*:}" > "$m/METADATA"
done
PYTHONPATH="$D/dmgbuild/src:$D/ds_store/src:$D/mac_alias/src:$D/meta" python3 -c 'import dmgbuild, ds_store, mac_alias; print("dmgbuild ready")'
