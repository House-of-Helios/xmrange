#!/usr/bin/env python3
"""One-off: turn a real thread-curve run into Tick JSONL (measure window of one step) for Rust tests."""
import json, sys
run, threads, out = sys.argv[1], int(sys.argv[2]), sys.argv[3]
rows = [json.loads(l) for l in open(f"{run}/samples.jsonl")]
step = next(s for s in json.load(open(f"{run}/summary.json"))["steps"] if s["threads"] == threads)
hs = None
with open(out, "w") as f:
    for r in rows:
        if r["src"] == "api" and r.get("hs10"):
            hs = r["hs10"]
        if r["src"] == "macmon" and step["t0"] <= r["t"] <= step["t1"] and hs:
            f.write(json.dumps({"power": {"t": r["t"], "cpu_w": r["cpu_w"], "ram_w": r["ram_w"], "sys_w": r["sys_w"],
                                          "temp_c": r["temp_c"], "p_mhz": int(r["p_mhz"]), "e_mhz": int(r["e_mhz"]),
                                          "p_active": r["p_active"], "e_active": r["e_active"]},
                                "hs10": hs, "other_cpu_pct": None}) + "\n")
print(out)
