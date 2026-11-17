#!/bin/bash
# Low Power Mode OFF: marginal curves + coexistence (mirror of the LPM-on campaign).
cd "$(dirname "$0")/.."
python3 tools/thread_curve.py --config configs/rx0-only-8t.json --label lpm-off-curve-pe --steps 1,2,3,4,5,6,7,8
sleep 90
python3 tools/thread_curve.py --config configs/rx0-only-8t.json --label lpm-off-curve-e --steps 1,2,3,4 --taskpolicy background
sleep 90
python3 tools/coexist.py --label coexist-lpmoff-8t --threads 8
sleep 60
python3 tools/coexist.py --label coexist-lpmoff-5t --threads 5 --modes none,default,utility
