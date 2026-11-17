#!/bin/bash
# Marginal thread curves with Low Power Mode ON (Helios toggled). One process per curve, API-stepped.
cd "$(dirname "$0")/.."
python3 tools/thread_curve.py --config configs/rx0-only-8t.json --label lpm-on-curve-pe --steps 1,2,3,4,5,6,7,8
sleep 90
python3 tools/thread_curve.py --config configs/rx0-only-8t.json --label lpm-on-curve-e --steps 1,2,3,4 --taskpolicy background
