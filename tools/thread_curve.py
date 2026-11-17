#!/usr/bin/env python3
"""Marginal thread curve: one xmrig process, thread count stepped via API hot-reload.

Each step: PUT cpu.rx = [-1]*n → settle → measure. Reports per-step H/s, W, H/W and the
*marginal* ΔH/ΔW vs the previous step. Also counts RandomX dataset inits in the log to answer
"does a thread change via API re-init the 2 GB dataset?".

  tools/thread_curve.py --config configs/rx0-only-8t.json --label lpm-curve --steps 1,2,3,4,5,6,7,8
  tools/thread_curve.py ... --taskpolicy background --steps 1,2,3,4      (E-core curve)
"""
import argparse, json, os, secrets, shutil, signal, subprocess, sys, threading, time
from datetime import datetime

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from xmrange_run import (ROOT, XMRIG, Recorder, api_poller, api_request, avg, context,
                         context_poller, hw, macmon_reader, now)


def step_stats(rows, t0, t1):
    m = [r for r in rows if r["src"] == "macmon" and t0 <= r["t"] <= t1]
    a = [r for r in rows if r["src"] == "api" and t0 <= r["t"] <= t1 and r.get("hs10")]
    out = {k: avg(m, k) for k in ("cpu_w", "ram_w", "sys_w", "temp_c", "p_mhz", "e_mhz", "p_active", "e_active")}
    out["hs"] = avg(a, "hs10")
    out["n_power"], out["n_hash"] = len(m), len(a)
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--config", required=True)
    ap.add_argument("--label", required=True)
    ap.add_argument("--steps", default="1,2,3,4,5,6,7,8")
    ap.add_argument("--first-settle", type=int, default=240, help="settle for step 1 (includes dataset init)")
    ap.add_argument("--settle", type=int, default=150)
    ap.add_argument("--measure", type=int, default=180)
    ap.add_argument("--taskpolicy", choices=["none", "background"], default="none")
    a = ap.parse_args()
    steps = [int(x) for x in a.steps.split(",")]

    run = os.path.join(ROOT, "runs", f"{datetime.now():%Y%m%d-%H%M%S}-{a.label}")
    os.makedirs(run)
    cfg_path = os.path.join(run, "config.json")
    cfg = json.load(open(a.config))
    token = secrets.token_hex(16)
    cfg["http"].update({"enabled": True, "host": "127.0.0.1", "restricted": False, "access-token": token})
    cfg["cpu"]["rx"] = [-1] * steps[0]
    json.dump(cfg, open(cfg_path, "w"), indent=2)
    os.chmod(cfg_path, 0o600)
    log = os.path.join(run, "xmrig.log")
    rec = Recorder(os.path.join(run, "samples.jsonl"))
    stop = threading.Event()

    mm = subprocess.Popen(["macmon", "pipe", "-i", "1000"], stdout=subprocess.PIPE, text=True, stderr=subprocess.DEVNULL)
    threading.Thread(target=macmon_reader, args=(mm, rec, stop), daemon=True).start()
    time.sleep(3)
    xm = subprocess.Popen([XMRIG, "-c", cfg_path, f"--log-file={log}", "--no-color"],
                          stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, stdin=subprocess.DEVNULL)
    if a.taskpolicy == "background":
        subprocess.run(["taskpolicy", "-b", "-p", str(xm.pid)], check=False)
    caf = subprocess.Popen(["caffeinate", "-i", "-w", str(xm.pid)])
    threading.Thread(target=api_poller, args=(rec, stop, 5.0, token), daemon=True).start()
    rec.add("ctx", context())
    threading.Thread(target=context_poller, args=(rec, stop), daemon=True).start()
    print(f"▶ {a.label}: steps {steps}, pid {xm.pid}, run {run}", flush=True)

    results = []
    try:
        for i, n in enumerate(steps):
            if i > 0:
                with api_request("/2/config", token) as r:
                    live = json.load(r)
                live["cpu"]["rx"] = [-1] * n
                with api_request("/2/config", token, "PUT", live) as r:
                    assert r.status in (200, 204), r.status
            t_apply = now()
            rec.add("step", {"threads": n, "event": "apply"})
            time.sleep(a.first_settle if i == 0 else a.settle)
            t0 = now()
            time.sleep(a.measure)
            t1 = now()
            st = step_stats(rec.rows, t0, t1) | {"threads": n, "t_apply": t_apply, "t0": t0, "t1": t1}
            results.append(st)
            print(f"  {n}t  H/s {st['hs'] or 0:7.1f}  sysW {st['sys_w']:5.2f}  cpuW {st['cpu_w']:5.2f}  "
                  f"T {st['temp_c'] * 9 / 5 + 32:.0f}F  H/W sys {hw(st['hs'], st['sys_w'])}", flush=True)
            if xm.poll() is not None:
                print("xmrig exited early", flush=True)
                break
    finally:
        if xm.poll() is None:
            xm.send_signal(signal.SIGINT)
            try:
                xm.wait(10)
            except subprocess.TimeoutExpired:
                xm.kill()
        stop.set(); mm.terminate(); caf.terminate()

    prev = None
    for st in results:
        st["hw_sys"], st["hw_cpu"] = hw(st["hs"], st["sys_w"]), hw(st["hs"], st["cpu_w"])
        if prev and st["hs"] and prev["hs"]:
            dh, dw = st["hs"] - prev["hs"], st["sys_w"] - prev["sys_w"]
            st["marginal_hs"], st["marginal_sys_w"] = round(dh, 1), round(dw, 2)
            st["marginal_hw_sys"] = round(dh / dw, 1) if dw > 0.05 else None  # ~free if dW ≈ 0
            st["thread_helps_avg"] = (st["hw_sys"] or 0) > (prev["hw_sys"] or 0)
        prev = st
    text = open(log, errors="replace").read() if os.path.exists(log) else ""
    summary = {"label": a.label, "taskpolicy": a.taskpolicy, "steps": results,
               "dataset_inits": text.count("init dataset"), "dataset_ready": text.count("dataset ready"),
               "context_end": context()}
    json.dump(summary, open(os.path.join(run, "summary.json"), "w"), indent=2)
    print(f"dataset inits: {summary['dataset_inits']} (1 = reused across thread changes)")


if __name__ == "__main__":
    main()
