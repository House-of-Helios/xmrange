#!/bin/bash
# Thread/core-class sweep, rx-family only, 13 min each, 90 s idle cool-down between.
cd "$(dirname "$0")/.."
run() { python3 tools/xmrange_run.py --duration 780 --warmup 240 "$@"; sleep 90; }
run --config configs/rx0-only-8t.json --label rx-8t
run --config configs/rx0-only-6t.json --label rx-6t
run --config configs/rx0-only-4t.json --label rx-4t
run --config configs/rx0-only-4t.json --label rx-4t-ecore --taskpolicy background
