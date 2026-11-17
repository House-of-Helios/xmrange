#!/usr/bin/env python3
"""Coexistence test: how much does the miner slow a user's work, per politeness setting?

For each mode: start xmrig (8 threads unless --threads), settle, measure idle H/W, then run a fixed
stand-in user workload and time it. Baseline = same workload with no miner.
Workloads: 'single' = 1-thread CPU loop (UI/app-like latency), 'quad' = 4 parallel CPU loops (build-like).

Modes: default | nice19 (xmrig priority 0) | utility (taskpolicy -c utility) | background (taskpolicy -c background)
A "-noyield" suffix (or the mode "noyield") also turns xmrig's cpu.yield off.
"qos-utility" sets xmrig's cpu.qos (our patch): utility QoS per mining thread, the shipping lever.
"""
import argparse, json, os, secrets, signal, subprocess, sys, threading, time
from datetime import datetime

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from xmrange_run import ROOT, XMRIG, Recorder, api_poller, avg, context, hw, macmon_reader, now

WORK = "import time;t=time.perf_counter();x=0\nfor i in range(60_000_000): x+=i*i\nprint(time.perf_counter()-t)"


def workload(kind):
    n = 1 if kind == "single" else 4
    t = now()
    procs = [subprocess.Popen([sys.executable, "-c", WORK], stdout=subprocess.PIPE, text=True) for _ in range(n)]
    for p in procs:
        p.wait()
    return now() - t


def window(rows, t0, t1):
    m = [r for r in rows if r["src"] == "macmon" and t0 <= r["t"] <= t1]
    a = [r for r in rows if r["src"] == "api" and t0 <= r["t"] <= t1 and r.get("hs10")]
    return {"hs": avg(a, "hs10"), "sys_w": avg(m, "sys_w"), "cpu_w": avg(m, "cpu_w"), "temp_c": avg(m, "temp_c")}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--config", default="configs/rx0-only-8t.json")
    ap.add_argument("--label", default="coexist")
    ap.add_argument("--modes", default="none,default,nice19,utility,background")
    ap.add_argument("--threads", type=int, default=8)
    ap.add_argument("--settle", type=int, default=150)
    ap.add_argument("--measure", type=int, default=90)
    ap.add_argument("--reps", type=int, default=2)
    a = ap.parse_args()
    run = os.path.join(ROOT, "runs", f"{datetime.now():%Y%m%d-%H%M%S}-{a.label}")
    os.makedirs(run)
    rec = Recorder(os.path.join(run, "samples.jsonl"))
    stop = threading.Event()
    mm = subprocess.Popen(["macmon", "pipe", "-i", "1000"], stdout=subprocess.PIPE, text=True, stderr=subprocess.DEVNULL)
    threading.Thread(target=macmon_reader, args=(mm, rec, stop), daemon=True).start()
    results = {"context": context(), "modes": {}}
    try:
        for mode in a.modes.split(","):
            r = {"mode": mode}
            xm = caf = None
            if mode != "none":
                cfg = json.load(open(a.config))
                token = secrets.token_hex(16)
                cfg["http"].update({"restricted": True, "access-token": token})
                cfg["cpu"]["rx"] = [-1] * a.threads
                cfg["cpu"]["priority"] = 0 if mode == "nice19" else None
                cfg["cpu"]["yield"] = "noyield" not in mode
                # "qos-utility": our xmrig patch's per-thread utility QoS (the shipping lever).
                cfg["cpu"]["qos"] = "utility" if mode.startswith("qos-utility") else None
                path = os.path.join(run, f"config-{mode}.json")
                json.dump(cfg, open(path, "w"), indent=2); os.chmod(path, 0o600)
                cmd = [XMRIG, "-c", path, f"--log-file={os.path.join(run, f'xmrig-{mode}.log')}", "--no-color"]
                policy = mode.split("-")[0]
                if policy in ("utility", "background"):
                    cmd = ["taskpolicy", "-c", policy] + cmd
                xm = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, stdin=subprocess.DEVNULL)
                caf = subprocess.Popen(["caffeinate", "-i", "-w", str(xm.pid)])
                pstop = threading.Event()
                threading.Thread(target=api_poller, args=(rec, pstop, 5.0, token), daemon=True).start()
                time.sleep(a.settle)
                t0 = now(); time.sleep(a.measure)
                r["idle"] = window(rec.rows, t0, now())
                r["idle"]["hw_sys"] = hw(r["idle"]["hs"], r["idle"]["sys_w"])
            else:
                time.sleep(20)
            for kind in ("single", "quad"):
                ts = []
                for _ in range(a.reps):
                    t0 = now(); ts.append(workload(kind)); r[f"{kind}_window"] = window(rec.rows, t0, now())
                r[f"{kind}_s"] = round(sum(ts) / len(ts), 2)
            print(f"{mode:10} single {r['single_s']:6.2f}s  quad {r['quad_s']:6.2f}s  "
                  + (f"idle H/s {r['idle']['hs'] or 0:6.0f}  sysW {r['idle']['sys_w']:5.2f}  H/W {r['idle']['hw_sys']}" if 'idle' in r else ""), flush=True)
            results["modes"][mode] = r
            if xm:
                pstop.set(); xm.send_signal(signal.SIGINT)
                try: xm.wait(10)
                except subprocess.TimeoutExpired: xm.kill()
                caf.terminate()
                time.sleep(60)
    finally:
        stop.set(); mm.terminate()
    base = results["modes"].get("none")
    if base:
        for m, r in results["modes"].items():
            r["single_slowdown_pct"] = round(100 * (r["single_s"] / base["single_s"] - 1), 1)
            r["quad_slowdown_pct"] = round(100 * (r["quad_s"] / base["quad_s"] - 1), 1)
    json.dump(results, open(os.path.join(run, "summary.json"), "w"), indent=2)
    for m, r in results["modes"].items():
        print(f"{m:10} user slowdown: single {r.get('single_slowdown_pct')}%  quad {r.get('quad_slowdown_pct')}%")


if __name__ == "__main__":
    main()
