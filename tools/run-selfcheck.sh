#!/bin/bash
# The dev app's self-check, hidden (ruling P2-R10), in its own data dir (_devdata/selfcheck/data),
# so it never shares settings, secrets, the run dir or the single-instance lock with the owner's
# app. Static by default; `live` also mines for about a minute through the real UI path.
#
# Usage: [ARCH=x86_64] tools/run-selfcheck.sh [static|live] [<out file>]   (default out: _devdata/selfcheck-<mode>.out)
# Waits (polling only, up to 20 min) while any xmrange-app or xmrig runs, then reports BLOCKED.
# Never signals a process it didn't start. No sudo, no AppleScript.
set -uo pipefail
export PATH=/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
MODE="${1:-static}"
OUT="${2:-$ROOT/_devdata/selfcheck-$MODE.out}"
DATA="$ROOT/_devdata/selfcheck/data"
LIMIT=$([ "$MODE" = live ] && echo 300 || echo 180)
# ARCH=x86_64: the Intel app's code, run here under Rosetta (Stage 3); its own output file.
if [ "${ARCH:-arm64}" = x86_64 ]; then
  APP_BIN="$ROOT/target/x86_64-apple-darwin/debug/xmrange-app"
  RUN=(arch -x86_64 "$APP_BIN")
  OUT="${2:-$ROOT/_devdata/selfcheck-$MODE-x86_64.out}"
  cargo build -q -p xmrange-app --features dev-diagnostics --target x86_64-apple-darwin || { echo "FAIL: dev build (x86_64)"; exit 1; }
else
  APP_BIN="$ROOT/target/debug/xmrange-app"
  RUN=("$APP_BIN")
  cargo build -q -p xmrange-app --features dev-diagnostics || { echo "FAIL: dev build"; exit 1; }
fi
mkdir -p "$DATA"

waited=0
while pgrep -x xmrange-app >/dev/null || pgrep -x xmrig >/dev/null; do
  [ $waited -ge 1200 ] && { echo "BLOCKED: another xmrange-app or xmrig still running after 20 min"; exit 3; }
  [ $waited -eq 0 ] && echo "waiting: another xmrange-app or xmrig is running (up to 20 min)"
  sleep 10; waited=$((waited + 10))
done

FRONT="$(lsappinfo front 2>/dev/null)"
if [ "$MODE" = live ]; then
  XMRANGE_SELF_CHECK=1 XMRANGE_SELF_CHECK_LIVE=1 XMRANGE_DATA_DIR="$DATA" "${RUN[@]}" >"$OUT" 2>&1 &
else
  XMRANGE_SELF_CHECK=1 XMRANGE_DATA_DIR="$DATA" "${RUN[@]}" >"$OUT" 2>&1 &
fi
PID=$!
START=$(date +%s); changed=0
while kill -0 $PID 2>/dev/null; do
  [ "$(lsappinfo front 2>/dev/null)" != "$FRONT" ] && changed=1
  if [ $(( $(date +%s) - START )) -ge $LIMIT ]; then
    echo "WATCHDOG: stopping after $LIMIT s" >>"$OUT"
    [ "$(ps -o comm= -p $PID 2>/dev/null | xargs basename 2>/dev/null)" = xmrange-app ] && kill -TERM $PID
    sleep 15
    [ "$(ps -o comm= -p $PID 2>/dev/null | xargs basename 2>/dev/null)" = xmrange-app ] && kill -KILL $PID
    break
  fi
  sleep 1
done
wait $PID 2>/dev/null; code=$?
echo "exit=$code elapsed=$(( $(date +%s) - START ))s front_app_changed=$changed" >>"$OUT"
python3 - "$OUT" <<'PY'
import json, sys
text = open(sys.argv[1]).read()
line = next((l for l in text.splitlines() if l.startswith("SELF-CHECK {")), None)
if not line:
    print("NO REPORT"); print(text[-2000:]); sys.exit(1)
rep = json.loads(line[len("SELF-CHECK "):])
checks = rep["checks"]
bad = [k for k, v in checks.items() if not v]
print(f"self-check: {'PASS' if rep['ok'] else 'FAIL'} ({sum(1 for v in checks.values() if v)}/{len(checks)})")
if bad:
    print("failed:", ", ".join(bad))
    print(json.dumps(rep.get("detail", {}), indent=1)[:6000])
print(text.strip().splitlines()[-1])
sys.exit(0 if rep["ok"] else 1)
PY
