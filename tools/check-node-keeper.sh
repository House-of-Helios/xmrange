#!/bin/bash
# XMRange's own Monero node, through the app (Plan 6 M3): a dev build with p2pool, the own node,
# Tor on and a chain folder in _devdata. The app's keeper must start monerod on its own, monerod
# must find the chain through Tor with no connection of its own off this Mac, and a SIGTERM to the
# app must stop monerod and Arti cleanly and leave no peer list. Hidden window (P2-R10). Signals
# only the app it started. Prints counts only. The chain folder is deleted at the end.
set -uo pipefail
export PATH=/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
BASE="$ROOT/_devdata/node-keeper"
DATA="$BASE/data"; CHAIN="$BASE/chain"; LOG="$BASE/app.log"
fail=0
pass() { echo "PASS  $1"; }
bad() { echo "FAIL  $1"; fail=1; }

cargo build -q -p xmrange-app --features dev-diagnostics || { echo "FAIL: dev build"; exit 1; }
for p in xmrange-app xmrig monerod; do
  pgrep -x "$p" >/dev/null && { echo "BLOCKED: a $p is running (maybe the owner's); try later"; exit 3; }
done
rm -rf "$BASE"; mkdir -p "$DATA" "$CHAIN"
printf '{"pool":"p2pool","node":"own","node_dir":"%s","tor":true,"notice_dismissed":true}\n' "$CHAIN" > "$DATA/settings.json"

XMRANGE_AUTOSTART=1 XMRANGE_DATA_DIR="$DATA" "$ROOT/target/debug/xmrange-app" >"$LOG" 2>&1 &
APP=$!
mine() { pgrep -P "$APP" -x "$1" | head -1; }

node=""; for _ in $(seq 1 120); do node="$(mine monerod)"; [ -n "$node" ] && break; sleep 1; done
[ -n "$node" ] && pass "the keeper started monerod (a child of the app)" || bad "no monerod after 120 s"
target=0
for _ in $(seq 1 40); do
  target="$(curl -s -m 2 http://127.0.0.1:3632/get_info | python3 -c 'import json,sys; print(json.load(sys.stdin).get("target_height",0))' 2>/dev/null || echo 0)"
  [ "${target:-0}" -gt 3000000 ] && break; sleep 3
done
[ "${target:-0}" -gt 3000000 ] && pass "monerod found the chain through tor (target $target)" || bad "no chain height from peers"
if [ -n "$node" ]; then
  off="$(lsof -nP -a -p "$node" -i 2>/dev/null | grep -- '->' | grep -vc -- '->127.0.0.1:' || true)"
  [ "$off" -eq 0 ] && pass "monerod holds no connection off this mac (all through arti)" || bad "monerod has $off direct connection(s)"
  lsof -nP -a -p "$node" -iTCP -sTCP:LISTEN 2>/dev/null | awk 'NR>1{print $9}' | grep -v '^127.0.0.1:' | grep -q . && bad "monerod listens beyond 127.0.0.1" || pass "monerod listens on 127.0.0.1 only"
fi
arti="$(mine arti)"; [ -n "$arti" ] && pass "one shared arti runs" || bad "no arti"

t0=$(date +%s)
kill -TERM "$APP"
for _ in $(seq 1 150); do kill -0 "$APP" 2>/dev/null || break; sleep 1; done
gone=1
kill -0 "$APP" 2>/dev/null && { bad "the app still runs after 150 s"; gone=0; }
[ -n "$node" ] && kill -0 "$node" 2>/dev/null && { bad "monerod still runs"; gone=0; }
[ -n "$arti" ] && kill -0 "$arti" 2>/dev/null && { bad "arti still runs"; gone=0; }
[ $gone -eq 1 ] && pass "SIGTERM stopped the app, monerod and arti in $(( $(date +%s) - t0 )) s"
[ -e "$CHAIN/monero-chain/p2pstate.bin" ] && bad "monerod's peer list stayed at rest" || pass "no peer list at rest"
rm -rf "$CHAIN"
[ $fail -eq 0 ] && echo "node keeper check: PASS" || echo "node keeper check: FAIL (log: $LOG)"
exit $fail
