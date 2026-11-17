#!/bin/bash
# Lifecycle + network check for the desktop app: a dev build, really mining, once per quit signal.
#
# For SIGTERM, then SIGINT:
#   1. launch target/debug/xmrange-app with XMRANGE_AUTOSTART=1 (dev-only: mining starts on setup;
#      the window stays hidden and the app is never activated, ruling P2-R10) and its own data dir
#      (_devdata/lifecycle/data), so its settings and handoff dirs are this script's alone
#   2. wait for its xmrig (vendor/xmrig-mo/build/xmrig, a child of the app) and the pool connection
#   3. network: the app has 0 sockets with a non-loopback peer; xmrig has exactly 1 non-loopback
#      ESTABLISHED TCP connection (the pool)
#   4. send the signal to the app; within 15 s there is no app, no xmrig, no `caffeinate -i -w
#      <xmrig pid>`, and the data dir's run/ holds no handoff-* dir
#   5. the Ledger (Plan 3 Task 9): the app logged its close, no -wal is left beside the dev
#      Ledger (a clean close checkpoints it away), SQLite's and SQLCipher's integrity checks pass
#      (`xmrange ledger`, dev CLI), and the round kept every trial (the Ledger keeps trials only
#      since schema v3, ruling P5-O1: no 1 s samples to count). STOPQUIT checks the same.
#
# Then three crash rounds (final review I2, I3):
#   ORPHAN  launch with the parent guard off (XMRANGE_DEV_NO_PARENT_GUARD=1, dev only), `kill -9` the
#           app, confirm xmrig and its caffeinate were left running with the run record; relaunch
#           and confirm the new app stops them at startup (and says so), then mines normally
#   GUARD   launch with the guard on, `kill -9` the app, confirm the guard stops xmrig on its own
#   STOPQUIT  XMRANGE_DEV_STOP_QUIT_AFTER: the app begins a stop and quits in the same instant (as
#           a stop click followed at once by ⌘Q), guard off; confirm nothing is left running
#
# Prints PASS or FAIL per assertion. Prints counts only, never an address.
# The owner may be running XMRange.app:
#   - if any xmrange-app or xmrig is running before a round, this waits up to 20 min for it to
#     finish, then reports BLOCKED (exit 3);
#   - if one appears mid-round, the round stops its own app cleanly and reports INCONCLUSIVE (exit 4).
# It only ever signals processes it started: a pid is forgotten once seen dead, and checked for its
# parent and command before any kill. The app's own dead-man's switch (background mode) stops it
# if this script dies without cleaning up. No sudo, no AppleScript, no System Events.
set -uo pipefail
export PATH=/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

APP_BIN="$ROOT/target/debug/xmrange-app"
CLI_BIN="$ROOT/target/debug/xmrange"
XMRIG_PAT="vendor/xmrig-mo/build/xmrig"
LOG_DIR="$ROOT/_devdata/lifecycle"
DATA_DIR="$LOG_DIR/data"
RUN_DIR="$DATA_DIR/run"
LEDGER="$DATA_DIR/ledger-dev.sqlite"
MINE_SECS="${XMRANGE_LIFECYCLE_MINE_SECS:-20}" # real mining after the pool connects, per round
QUIT_SECS=15
QUIET_WAIT_SECS=1200

fails=0
inconclusive=0
pass() { echo "PASS  $*"; }
fail() { echo "FAIL  $*"; fails=$((fails + 1)); }

# Alive = present and not a zombie (a dead child stays a zombie until its parent reaps it).
alive() {
  local st
  [ -n "$1" ] || return 1
  st="$(ps -p "$1" -o stat= 2>/dev/null)"
  [ -n "$st" ] && [[ "$st" != Z* ]]
}

# ours <pid> <command name> <args substring> <allowed parent pid>...
# True only if the pid is alive and is still the process this script started: same command name,
# the expected arguments, and one of the expected parents. Guards every kill against pid reuse.
ours() {
  local pid="$1" name="$2" args="$3" ppid comm cmdline p
  shift 3
  alive "$pid" || return 1
  ppid="$(ps -o ppid= -p "$pid" 2>/dev/null | tr -d ' ')"
  comm="$(ps -o comm= -p "$pid" 2>/dev/null)"
  cmdline="$(ps -o args= -p "$pid" 2>/dev/null)"
  [ "$(basename "$comm")" = "$name" ] || return 1
  [[ "$cmdline" == *"$args"* ]] || return 1
  for p in "$@"; do [ "$ppid" = "$p" ] && return 0; done
  return 1
}

# Sockets of pid $1 (lsof filters in $2..) whose peer is not loopback. Prints a count only.
remote_count() {
  local pid="$1"
  shift
  lsof -nP -a -p "$pid" "$@" -F n 2>/dev/null | awk '
    /^n.*->/ { peer = $0; sub(/^n.*->/, "", peer)
               if (peer !~ /^(127\.|\[::1\]|\[::ffff:127\.|localhost)/) n++ }
    END { print n + 0 }'
}

# Any xmrange-app or xmrig that isn't this round's own. Prints a count only.
others_running() {
  local p n=0
  for p in $(pgrep -x xmrange-app) $(pgrep -x xmrig); do
    [ "$p" = "$APP_PID" ] || [ "$p" = "$XPID" ] || n=$((n + 1))
  done
  echo "$n"
}

# The owner's XMRange.app, or any other miner, has priority: wait, then give up.
wait_for_quiet() {
  local waited=0
  while pgrep -x xmrange-app >/dev/null || pgrep -x xmrig >/dev/null; do
    if [ $waited -ge $QUIET_WAIT_SECS ]; then
      echo "BLOCKED: another xmrange-app or xmrig is still running after ${QUIET_WAIT_SECS} s; not interfering"
      return 1
    fi
    [ $waited -eq 0 ] && echo "waiting: another xmrange-app or xmrig is running (up to ${QUIET_WAIT_SECS} s)"
    sleep 5
    waited=$((waited + 5))
  done
}

# This round's processes. A pid is cleared the moment it is seen dead, so it is never signalled
# after the system could have reused it. LAUNCHED keeps the app pid for the parent checks.
# XSEEN keeps the xmrig pid for caffeinate's `-w <pid>` match. APP_CODE is the app's exit code.
APP_PID="" XPID="" CAFF_PID="" LAUNCHED="" XSEEN="" APP_CODE="" PREV_LAUNCHED=""
forget_dead() {
  if [ -n "$APP_PID" ] && ! alive "$APP_PID"; then wait "$APP_PID" 2>/dev/null; APP_CODE=$?; APP_PID=""; fi
  alive "$XPID" || XPID=""
  alive "$CAFF_PID" || CAFF_PID=""
}
cleanup() {
  forget_dead
  if ours "$APP_PID" xmrange-app "$APP_BIN" "$$"; then kill -KILL "$APP_PID" 2>/dev/null; fi
  if ours "$XPID" xmrig "$XMRIG_PAT" "$LAUNCHED" ${PREV_LAUNCHED:-1} 1; then kill -KILL "$XPID" 2>/dev/null; fi
  if ours "$CAFF_PID" caffeinate "-w $XSEEN" "$LAUNCHED" ${PREV_LAUNCHED:-1} 1; then kill -KILL "$CAFF_PID" 2>/dev/null; fi
  [ -n "$APP_PID" ] && wait "$APP_PID" 2>/dev/null
  APP_PID="" XPID="" CAFF_PID="" LAUNCHED="" XSEEN=""
}
trap 'cleanup; exit 130' INT TERM
trap cleanup EXIT

# Stop this round's app the normal way (SIGTERM) and wait for it, for an INCONCLUSIVE round.
stop_own_app() {
  ours "$APP_PID" xmrange-app "$APP_BIN" "$$" && kill -TERM "$APP_PID" 2>/dev/null
  local start=$SECONDS
  while alive "$APP_PID" && [ $((SECONDS - start)) -lt $QUIT_SECS ]; do sleep 0.5; done
  cleanup
}

round() {
  local sig="$1" i
  echo "== $sig"
  wait_for_quiet || exit 3

  local front_before trials_before
  front_before="$(lsappinfo front 2>/dev/null)"
  trials_before="$(ledger_trials)"
  APP_CODE=""
  XMRANGE_AUTOSTART=1 XMRANGE_DATA_DIR="$DATA_DIR" "$APP_BIN" >"$LOG_DIR/app-$sig.log" 2>&1 &
  APP_PID=$!
  LAUNCHED=$APP_PID

  # 2. xmrig, as a child of this app (never someone else's)
  for i in $(seq 1 60); do
    XPID="$(pgrep -P "$APP_PID" -f "$XMRIG_PAT" | head -1)"
    [ -n "$XPID" ] && break
    alive "$APP_PID" || break
    sleep 1
  done
  if [ -z "$XPID" ]; then
    fail "[$sig] xmrig started by the app (none within 60 s)"
    stop_own_app
    return
  fi
  XSEEN=$XPID
  pass "[$sig] xmrig started by the app"

  local pool=0
  for i in $(seq 1 60); do
    pool="$(remote_count "$XPID" -iTCP -sTCP:ESTABLISHED)"
    [ "$pool" -ge 1 ] && break
    alive "$XPID" || break
    sleep 1
  done
  sleep "$MINE_SECS"
  CAFF_PID="$(pgrep -P "$APP_PID" -f "caffeinate -i -w ${XPID}\$" | head -1)"
  if [ -n "$CAFF_PID" ]; then pass "[$sig] caffeinate -i -w <xmrig> keeps the Mac awake while mining"; else fail "[$sig] caffeinate -i -w <xmrig> running while mining"; fi

  # The owner opened XMRange.app (or started a miner) mid-round: it shares port 3620, so this
  # round's results would be mixed with theirs. Stop our own app and say so.
  if [ "$(others_running)" -ne 0 ]; then
    echo "INCONCLUSIVE  [$sig] another xmrange-app or xmrig appeared mid-round; stopped this round's app"
    inconclusive=$((inconclusive + 1))
    stop_own_app
    return
  fi

  # P2-R10, informational (the owner may switch apps themselves): the front app is unchanged.
  if [ "$(lsappinfo front 2>/dev/null)" = "$front_before" ]; then echo "INFO  [$sig] front app unchanged while mining (P2-R10)"; else echo "INFO  [$sig] front app changed while mining (P2-R10: check it wasn't this app)"; fi

  # 3. network shape, counts only. The app's loopback polls of the xmrig API are short-lived, so
  # it is sampled 10 times over 5 s: the most non-loopback peers seen, and the most sockets seen.
  local app_remote=0 app_inet=0 r n xmrig_remote
  for i in $(seq 1 10); do
    r="$(remote_count "$APP_PID" -i)"
    n="$(lsof -nP -a -p "$APP_PID" -i -F f 2>/dev/null | grep -c '^f')"
    [ "$r" -gt "$app_remote" ] && app_remote=$r
    [ "$n" -gt "$app_inet" ] && app_inet=$n
    sleep 0.5
  done
  xmrig_remote="$(remote_count "$XPID" -iTCP -sTCP:ESTABLISHED)"
  # Self-test: lsof must see the app at all, or its "0 connections" would mean nothing.
  local app_fds
  app_fds="$(lsof -nP -a -p "$APP_PID" -F f 2>/dev/null | grep -c '^f')"
  if [ "$app_fds" -gt 0 ]; then pass "[$sig] lsof sees the app ($app_fds open files)"; else fail "[$sig] lsof sees no open files for the app (instrument broken)"; fi
  if [ "$app_remote" -eq 0 ]; then pass "[$sig] app: 0 non-loopback connections in 10 samples (at most $app_inet loopback sockets)"; else fail "[$sig] app: $app_remote non-loopback connections (want 0)"; fi
  if [ "$xmrig_remote" -eq 1 ]; then pass "[$sig] xmrig: exactly 1 non-loopback ESTABLISHED TCP (the pool)"; else fail "[$sig] xmrig: $xmrig_remote non-loopback ESTABLISHED TCP (want exactly 1)"; fi

  # 4. quit by signal
  ours "$APP_PID" xmrange-app "$APP_BIN" "$$" && kill -"$sig" "$APP_PID"
  local start=$SECONDS
  while [ $((SECONDS - start)) -lt $QUIT_SECS ]; do
    forget_dead
    [ -z "$APP_PID" ] && [ -z "$XPID" ] && [ -z "$CAFF_PID" ] && break
    sleep 0.5
  done
  local took=$((SECONDS - start))
  if [ -n "$APP_PID" ]; then fail "[$sig] app still running after ${QUIT_SECS} s"; else pass "[$sig] app exited (code $APP_CODE) in ${took} s"; fi
  if [ -n "$XPID" ]; then fail "[$sig] xmrig still running after ${QUIT_SECS} s"; else pass "[$sig] no xmrig left"; fi
  if pgrep -f "caffeinate -i -w ${XSEEN}\$" >/dev/null; then fail "[$sig] caffeinate -i -w <xmrig> left behind"; else pass "[$sig] no caffeinate -i -w <xmrig> left"; fi
  local handoffs
  handoffs="$(find "$RUN_DIR" -maxdepth 1 -name 'handoff-*' 2>/dev/null | wc -l | tr -d ' ')"
  if [ "$handoffs" -eq 0 ]; then pass "[$sig] no handoff-* dir in the run dir"; else fail "[$sig] $handoffs handoff-* dir(s) left in the run dir"; fi
  if pgrep -f "xmrange-guard $LAUNCHED " >/dev/null; then fail "[$sig] parent guard left behind"; else pass "[$sig] no parent guard left"; fi
  if [ ! -f "$RUN_DIR/xmrig.pid" ]; then pass "[$sig] run record removed"; else fail "[$sig] run record left after a clean quit"; fi
  [ -z "$APP_PID" ] && ledger_closed "$sig" "$LOG_DIR/app-$sig.log" "$trials_before"

  cleanup
}

# Trials in this data dir's dev Ledger (0 if there is none yet). Counts only.
ledger_trials() {
  local n
  n="$(XMRANGE_DATA_DIR="$DATA_DIR" "$CLI_BIN" ledger 2>/dev/null | sed -nE 's/.* trials ([0-9]+) .*/\1/p')"
  echo "${n:-0}"
}

# ledger_closed <label> <app log> <trials before the round>: the quit closed the Ledger whole.
ledger_closed() {
  local label="$1" log="$2" before="$3" stats after
  if grep -q "^ledger closed" "$log"; then pass "[$label] the app closed the Ledger on the way out"; else fail "[$label] no Ledger close in the app log"; fi
  if [ ! -e "$LEDGER-wal" ]; then pass "[$label] no -wal left beside the Ledger (a clean close)"; else fail "[$label] a -wal was left beside the Ledger"; fi
  stats="$(XMRANGE_DATA_DIR="$DATA_DIR" "$CLI_BIN" ledger 2>&1)"
  if grep -q "integrity ok" <<<"$stats"; then pass "[$label] Ledger integrity ok (SQLite + SQLCipher pages)"; else fail "[$label] Ledger integrity not ok"; fi
  after="$(sed -nE 's/.* trials ([0-9]+) .*/\1/p' <<<"$stats")"
  if [ -n "$after" ] && [ "$after" -ge "$before" ]; then pass "[$label] the Ledger kept its trials ($before before, $after after)"; else fail "[$label] trials lost or unreadable (${before} before, ${after:-none} after)"; fi
}

# Launch this round's app hidden with autostart (extra env first), and wait for its xmrig and its
# caffeinate. Sets APP_PID, LAUNCHED, XPID, XSEEN, CAFF_PID. Returns 1 if xmrig never appeared.
launch_mining() {
  local log="$1" i
  shift
  APP_CODE=""
  env "$@" XMRANGE_AUTOSTART=1 XMRANGE_DATA_DIR="$DATA_DIR" "$APP_BIN" >"$log" 2>&1 &
  APP_PID=$!
  LAUNCHED=$APP_PID
  XPID=""
  for i in $(seq 1 60); do
    XPID="$(pgrep -P "$APP_PID" -f "$XMRIG_PAT" | head -1)"
    [ -n "$XPID" ] && break
    alive "$APP_PID" || break
    sleep 1
  done
  [ -n "$XPID" ] || return 1
  XSEEN=$XPID
  CAFF_PID=""
  for i in $(seq 1 30); do
    CAFF_PID="$(pgrep -P "$APP_PID" -f "caffeinate -i -w ${XPID}\$" | head -1)"
    [ -n "$CAFF_PID" ] && break
    sleep 1
  done
  return 0
}

# I2: a kill -9'd app leaves xmrig mining (guard off for this test); the next launch reaps it.
round_orphan() {
  echo "== ORPHAN (kill -9, then relaunch)"
  wait_for_quiet || exit 3
  rm -f "$RUN_DIR/xmrig.pid"
  if ! launch_mining "$LOG_DIR/app-orphan-1.log" XMRANGE_DEV_NO_PARENT_GUARD=1; then
    fail "[ORPHAN] xmrig started by the app (none within 60 s)"; stop_own_app; return
  fi
  sleep 10
  if [ -f "$RUN_DIR/xmrig.pid" ]; then pass "[ORPHAN] run record written while mining"; else fail "[ORPHAN] no run record while mining"; fi
  if pgrep -f "xmrange-guard $APP_PID " >/dev/null; then fail "[ORPHAN] parent guard running although disabled for this test"; else pass "[ORPHAN] parent guard off for this test (dev env)"; fi
  local old_x=$XPID old_c=$CAFF_PID
  ours "$APP_PID" xmrange-app "$APP_BIN" "$$" && kill -KILL "$APP_PID"
  sleep 5
  forget_dead
  if [ -z "$APP_PID" ]; then pass "[ORPHAN] app gone after kill -9"; else fail "[ORPHAN] app survived kill -9"; fi
  local xppid
  xppid="$(ps -o ppid= -p "$old_x" 2>/dev/null | tr -d ' ')"
  if alive "$old_x" && [ "$xppid" = 1 ]; then pass "[ORPHAN] xmrig left running, reparented to launchd (the orphan to recover)"; else fail "[ORPHAN] no orphaned xmrig to recover (alive=$(alive "$old_x" && echo yes || echo no) ppid=$xppid)"; fi
  if [ -n "$old_c" ] && alive "$old_c"; then pass "[ORPHAN] its caffeinate left running too"; else echo "INFO  [ORPHAN] caffeinate not left running"; fi
  if [ -f "$RUN_DIR/xmrig.pid" ]; then pass "[ORPHAN] run record left behind for the next launch"; else fail "[ORPHAN] run record missing after the crash"; fi

  # Relaunch (guard on). The orphan still counts as "another xmrig" to wait_for_quiet, so no wait here.
  PREV_LAUNCHED=$LAUNCHED
  local t0=$SECONDS gone=""
  env XMRANGE_AUTOSTART=1 XMRANGE_DATA_DIR="$DATA_DIR" "$APP_BIN" >"$LOG_DIR/app-orphan-2.log" 2>&1 &
  APP_PID=$!
  LAUNCHED=$APP_PID
  for i in $(seq 1 40); do
    if ! alive "$old_x" && { [ -z "$old_c" ] || ! alive "$old_c"; }; then gone=1; break; fi
    sleep 0.5
  done
  if [ -n "$gone" ]; then pass "[ORPHAN] relaunch stopped the orphaned xmrig and caffeinate in $((SECONDS - t0)) s"; else fail "[ORPHAN] orphan still running $((SECONDS - t0)) s after relaunch"; fi
  XPID=""
  for i in $(seq 1 60); do
    grep -q "stopped a miner left running by a previous session" "$LOG_DIR/app-orphan-2.log" && XPID="$(pgrep -P "$APP_PID" -f "$XMRIG_PAT" | head -1)"
    [ -n "$XPID" ] && break
    alive "$APP_PID" || break
    sleep 1
  done
  if grep -q "stopped a miner left running by a previous session" "$LOG_DIR/app-orphan-2.log"; then pass "[ORPHAN] the relaunched app says so (tape note)"; else fail "[ORPHAN] no reap note from the relaunched app"; fi
  if [ -n "$XPID" ]; then pass "[ORPHAN] relaunched app mines again (port 3620 was freed)"; else fail "[ORPHAN] relaunched app could not start xmrig"; fi
  XSEEN=$XPID
  CAFF_PID="$(pgrep -P "$APP_PID" -f "caffeinate -i -w ${XPID}\$" | head -1)"
  sleep 5
  ours "$APP_PID" xmrange-app "$APP_BIN" "$$" && kill -TERM "$APP_PID"
  local start=$SECONDS
  while [ $((SECONDS - start)) -lt $QUIT_SECS ]; do
    forget_dead
    [ -z "$APP_PID" ] && [ -z "$XPID" ] && [ -z "$CAFF_PID" ] && break
    sleep 0.5
  done
  if [ -z "$APP_PID" ] && [ -z "$XPID" ] && [ -z "$CAFF_PID" ]; then pass "[ORPHAN] relaunched app quit cleanly (no xmrig, no caffeinate)"; else fail "[ORPHAN] something left after quitting the relaunched app"; fi
  if [ ! -f "$RUN_DIR/xmrig.pid" ]; then pass "[ORPHAN] run record removed by the clean quit"; else fail "[ORPHAN] run record left after a clean quit"; fi
  cleanup
  # If the relaunch failed to reap it, the orphan is still this script's to stop.
  if ours "$old_x" xmrig "$XMRIG_PAT" 1; then kill -KILL "$old_x" 2>/dev/null; fi
  if [ -n "$old_c" ] && ours "$old_c" caffeinate "-w $old_x" 1; then kill -KILL "$old_c" 2>/dev/null; fi
  PREV_LAUNCHED=""
}

# The parent guard: with it on, a kill -9'd app's xmrig stops within seconds, with no relaunch.
round_guard() {
  echo "== GUARD (kill -9, guard on)"
  wait_for_quiet || exit 3
  if ! launch_mining "$LOG_DIR/app-guard.log"; then
    fail "[GUARD] xmrig started by the app (none within 60 s)"; stop_own_app; return
  fi
  sleep 10
  if pgrep -f "xmrange-guard $APP_PID $XPID " >/dev/null; then pass "[GUARD] parent guard running beside xmrig"; else fail "[GUARD] no parent guard"; fi
  local old_x=$XPID old_c=$CAFF_PID app=$APP_PID t0 gone=""
  ours "$APP_PID" xmrange-app "$APP_BIN" "$$" && kill -KILL "$APP_PID"
  t0=$SECONDS
  for i in $(seq 1 40); do
    forget_dead
    if ! alive "$old_x" && { [ -z "$old_c" ] || ! alive "$old_c"; }; then gone=1; break; fi
    sleep 0.5
  done
  if [ -n "$gone" ]; then pass "[GUARD] xmrig and caffeinate stopped $((SECONDS - t0)) s after the app was killed"; else fail "[GUARD] xmrig still running $((SECONDS - t0)) s after the app was killed"; fi
  sleep 1
  if pgrep -f "xmrange-guard $app " >/dev/null; then fail "[GUARD] guard still running after its work"; else pass "[GUARD] guard exited after its work"; fi
  cleanup
}

# I3: a stop in flight when the app quits (stop, then ⌘Q at once) still stops xmrig.
round_stopquit() {
  echo "== STOPQUIT (stop, then quit at once)"
  wait_for_quiet || exit 3
  local trials_before
  trials_before="$(ledger_trials)"
  # Guard off: only the quit path's wait may stop xmrig here, or the guard would hide an I3 regression.
  if ! launch_mining "$LOG_DIR/app-stopquit.log" XMRANGE_DEV_STOP_QUIT_AFTER=25 XMRANGE_DEV_NO_PARENT_GUARD=1; then
    fail "[STOPQUIT] xmrig started by the app (none within 60 s)"; stop_own_app; return
  fi
  local start=$SECONDS
  while [ $((SECONDS - start)) -lt 60 ]; do
    forget_dead
    [ -z "$APP_PID" ] && [ -z "$XPID" ] && [ -z "$CAFF_PID" ] && break
    sleep 0.5
  done
  if grep -q "DEV stop-then-quit: stop in flight, quitting now" "$LOG_DIR/app-stopquit.log"; then pass "[STOPQUIT] stop began and quit was requested in the same instant"; else fail "[STOPQUIT] the hook never fired (no stop in flight)"; fi
  if [ -z "$APP_PID" ]; then pass "[STOPQUIT] app exited (code $APP_CODE)"; else fail "[STOPQUIT] app still running"; fi
  if [ -z "$XPID" ]; then pass "[STOPQUIT] no xmrig left"; else fail "[STOPQUIT] xmrig left running"; fi
  if pgrep -f "caffeinate -i -w ${XSEEN}\$" >/dev/null; then fail "[STOPQUIT] caffeinate left"; else pass "[STOPQUIT] no caffeinate left"; fi
  if [ ! -f "$RUN_DIR/xmrig.pid" ]; then pass "[STOPQUIT] run record removed (the stop completed)"; else fail "[STOPQUIT] run record left (the stop did not complete)"; fi
  [ -z "$APP_PID" ] && ledger_closed STOPQUIT "$LOG_DIR/app-stopquit.log" "$trials_before"
  cleanup
}

mkdir -p "$DATA_DIR"
# This script checks the app's own lifecycle and xmrig's one direct pool socket, so it pins Tor off
# in its own data dir (Tor is on by default since 0.2.21; its own checks live in the release audit).
printf '{"tor": false}\n' > "$DATA_DIR/settings.json"
wait_for_quiet || exit 3
echo "building the dev app (dev-diagnostics)"
cargo build -q -p xmrange-app --features dev-diagnostics || { echo "FAIL  dev build"; exit 1; }
cargo build -q -p xmrange-cli --features dev-diagnostics || { echo "FAIL  dev CLI build (the Ledger checks use it)"; exit 1; }

ROUNDS="${XMRANGE_LIFECYCLE_ROUNDS:-TERM INT ORPHAN GUARD STOPQUIT}"
for r in $ROUNDS; do
  case "$r" in
    TERM|INT) round "$r" ;;
    ORPHAN) round_orphan ;;
    GUARD) round_guard ;;
    STOPQUIT) round_stopquit ;;
  esac
done

if [ $fails -ne 0 ]; then echo "app lifecycle check: FAIL ($fails)"; exit 1; fi
if [ $inconclusive -ne 0 ]; then echo "app lifecycle check: INCONCLUSIVE ($inconclusive round(s) disturbed; rerun when the Mac is quiet)"; exit 4; fi
echo "app lifecycle check: PASS"
