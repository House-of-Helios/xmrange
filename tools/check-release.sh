#!/bin/bash
# Release builds must carry no dev code paths, no repo/home paths, no dev-only commands.
# Scans both release binaries, the CLI (xmrange) and the desktop app (xmrange-app), and the app's
# frontendDist tree (crates/xmrange-app/ui/): Tauri embeds that tree compressed, so the binary scan
# can't see into it, and scanning the source bytes it embeds covers it.
#
# Usage: tools/check-release.sh                        build both via tools/build-release.sh, scan
#        tools/check-release.sh <cli> <app> [<ui-dir>] scan these instead (no build), e.g. a copy
#                                                      with an injected needle, to prove LEAK fires
#
# Optional: XMRANGE_CHECK_WALLET=<address> adds your own wallet as a needle. It is never printed
# and never written anywhere; keep it out of the repo.
#
# NOTE on the grep-pipe fix below: `strings -a "$BIN" | grep -qF "$needle"` under
# `set -euo pipefail` is broken — `grep -q` exits as soon as it finds a match, `strings` then
# gets SIGPIPE (exit 141), pipefail fails the whole pipeline, and the `if` takes the no-leak
# branch even when the needle IS present. Capture the text once and grep the capture.
#
# NOTE on the byte scan: macOS `strings -a` reads only a Mach-O's sections, so bytes outside
# them are never seen (Task 11's negative run: a needle appended to a copied binary passed).
# Every printable run of every byte in the file is captured instead, a superset of `strings`.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# frontendDist, read from tauri.conf.json: exactly the tree a release build embeds.
DIST="$(grep -oE '"frontendDist": *"[^"]+"' "$ROOT/crates/xmrange-app/tauri.conf.json" | sed -E 's/.*"([^"]+)"$/\1/')"
UI="$ROOT/crates/xmrange-app/$DIST"

if [ $# -eq 2 ] || [ $# -eq 3 ]; then
  CLI="$1"; APP="$2"; UI="${3:-$UI}"
elif [ $# -eq 0 ]; then
  BINS="$("$ROOT/tools/build-release.sh")"
  CLI="$(sed -n 1p <<<"$BINS")"; APP="$(sed -n 2p <<<"$BINS")"
else
  echo "usage: $0 [<cli-binary> <app-binary> [<ui-dir>]]"; exit 2
fi

# The same leak needles apply to both binaries and to the embedded frontend.
NEEDLES=("_devdata" "test-tmp" "secrets.json" "HouseOfHelios" "$HOME" "XMRANGE_DEV_UNREDACTED"
  "XMRANGE_SELF_CHECK" "XMRANGE_AUTOSTART" "selfcheck-backup" "SELF-CHECK" "xmrange-dev-harness"
  "XMRANGE_DEV_NO_PARENT_GUARD" "XMRANGE_DEV_STOP_QUIT_AFTER" "DEV stop-then-quit"
  "dev.key" "ledger-dev.sqlite" "XMRANGE_LEDGER_CRASH")

# Wallets: the GhostDrip default (DEFAULT_WALLET) is legitimately embedded. Any OTHER Monero
# address-shaped run is a leak. The default is read from the source at check time, so no address
# is written here; matches are counted, never printed.
DEF="$(grep -m1 -oE '"4[1-9A-HJ-NP-Za-km-z]{94}"' "$ROOT/crates/xmrange-core/src/config.rs" | tr -d '"')"
if [ ${#DEF} -ne 95 ]; then echo "CHECKER BROKEN: DEFAULT_WALLET not found in config.rs"; exit 2; fi
ADDRESS_RE='[48][0-9AB][1-9A-HJ-NP-Za-km-z]{93}'
# Self-test: the pattern must match a real address (the default) in full.
if ! grep -xE -- "$ADDRESS_RE" <<<"$DEF" >/dev/null; then echo "CHECKER BROKEN: address pattern misses DEFAULT_WALLET"; exit 2; fi

fail=0

# check <label> <captured text> <self-test needle> [default-wallet-expected]
check() {
  local label="$1" text="$2" known="$3" want_default="${4:-}"
  # Self-test: a needle known to be in every release build of this target. If it isn't found,
  # the checker itself is broken (e.g. the pipefail bug above): never trust a PASS from it.
  if ! grep -F -- "$known" <<<"$text" >/dev/null; then
    echo "CHECKER BROKEN: known-present string not found in the $label"
    exit 2
  fi
  # Self-test for the wallet scan: the binaries carry the default wallet, so it must be seen.
  if [ -n "$want_default" ] && ! grep -F -- "$DEF" <<<"$text" >/dev/null; then
    echo "CHECKER BROKEN: the default wallet not found in the $label"
    exit 2
  fi
  local needle
  for needle in "${NEEDLES[@]}"; do
    if grep -F -- "$needle" <<<"$text" >/dev/null; then
      echo "LEAK: $label contains '$needle'"; fail=1
    fi
  done
  if [ -n "${XMRANGE_CHECK_WALLET:-}" ] && grep -F -- "$XMRANGE_CHECK_WALLET" <<<"$text" >/dev/null; then
    echo "LEAK: $label contains the wallet in XMRANGE_CHECK_WALLET"; fail=1
  fi
  local others
  # A real address is high-entropy base58: runs of one repeated character (padding) are skipped.
  others="$(sed "s/$DEF/ /g" <<<"$text" | { grep -oE "$ADDRESS_RE" || true; } |
    awk '{ n = 0; delete seen; for (i = 1; i <= length($0); i++) { c = substr($0, i, 1); if (!(c in seen)) { seen[c] = 1; n++ } } if (n >= 20) k++ } END { print k + 0 }')"
  if [ "$others" -ne 0 ]; then
    echo "LEAK: $label contains $others Monero-address-shaped string(s) other than the default wallet"; fail=1
  fi
}

# Every printable run of every byte of a file (see the byte-scan note above).
bytes() { LC_ALL=C tr -cs '[:print:]' '\n' <"$1"; }

for bin in "$CLI" "$APP"; do
  if [ ! -f "$bin" ]; then echo "CHECKER BROKEN: binary not found: $bin"; exit 2; fi
done
if [ ! -d "$UI" ] || [ -z "$(find "$UI" -type f -print -quit)" ]; then echo "CHECKER BROKEN: frontend tree not found"; exit 2; fi

check "cli release binary" "$(bytes "$CLI")" "Hashes per watt" default      # clap's `about` text
check "app release binary" "$(bytes "$APP")" "every watt, weighed" default  # the app's TAGLINE const
check "app frontend ($DIST/)" "$(find "$UI" -type f -exec cat {} + | LC_ALL=C tr -cs '[:print:]' '\n')" "every watt, weighed"  # index.html

# The bundled helpers (xmrig, p2pool, Arti): third-party code, so only the path needles apply (no
# repo or home path, i.e. no username), each with its own known-present self-test string.
if [ "${ARCH:-arm64}" = x86_64 ]; then
  HELPERS=("xmrig-mo/build-x86_64/xmrig:XMRig" "p2pool-x86_64/build/p2pool:P2Pool" "arti/target/x86_64-apple-darwin/release/arti:arti" "monero/build-out/x86_64/monerod:Monero" "universal/house-report:house-report")
else
  HELPERS=("xmrig-mo/build/xmrig:XMRig" "p2pool/build/p2pool:P2Pool" "arti/target/release/arti:arti" "monero/build-out/arm64/monerod:Monero" "universal/house-report:house-report")
fi
for h in "${HELPERS[@]}"; do
  bin="$ROOT/vendor/${h%%:*}"; known="${h##*:}"
  if [ ! -f "$bin" ]; then echo "CHECKER BROKEN: bundled helper not built: $bin"; exit 2; fi
  text="$(bytes "$bin")"
  if ! grep -F -- "$known" <<<"$text" >/dev/null; then echo "CHECKER BROKEN: '$known' not found in $bin"; exit 2; fi
  for needle in "HouseOfHelios" "$HOME" "_devdata"; do
    if grep -F -- "$needle" <<<"$text" >/dev/null; then echo "LEAK: bundled $(basename "$bin") contains '$needle'"; fail=1; fi
  done
done

# Every command the app registers must be granted in the capabilities the release is built from:
# 0.2.21 shipped a command (apply_pick) the ACL refused ("not allowed by ACL"). The binary can't
# prove this (a permission's name is compiled in whether or not it is granted, tested 2026-10-04),
# so the check reads the two source files the build reads.
CMDS="$(sed -n '/commands(&\[/,/\])/p' "$ROOT/crates/xmrange-app/build.rs" | grep -oE '"[a-z_]+"' | tr -d '"')"
if [ "$(wc -l <<<"$CMDS")" -lt 20 ]; then echo "CHECKER BROKEN: commands not read from build.rs"; exit 2; fi
CAPS="$(cat "$ROOT/crates/xmrange-app/capabilities/main.json")"
for cmd in $CMDS; do
  perm="allow-${cmd//_/-}"
  if ! grep -F -- "\"$perm\"" <<<"$CAPS" >/dev/null; then echo "MISSING: '$perm' is not granted in capabilities/main.json"; fail=1; fi
done

# `--help` opens no Ledger and reads no secrets: clap prints and exits before any command runs.
HELP="$("$CLI" --help)"
for cmd in wallet ledger; do
  if grep -w "$cmd" <<<"$HELP" >/dev/null; then echo "LEAK: dev-only '$cmd' command in the cli release"; fail=1; fi
done

[ $fail -eq 0 ] && echo "release privacy check: PASS (${ARCH:-arm64}: cli + app + frontend + bundled xmrig, p2pool, arti, monerod, house-report)"
exit $fail
