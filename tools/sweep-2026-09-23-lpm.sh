#!/bin/bash
# Low Power Mode ON sweep — same configs as the LPM-off thread sweep for apples-to-apples.
cd "$(dirname "$0")/.."
run() { python3 tools/xmrange_run.py --duration 780 --warmup 240 "$@"; sleep 90; }
run --config configs/rx0-only-8t.json --label lpm-rx-8t
run --config configs/rx0-only-6t.json --label lpm-rx-6t
