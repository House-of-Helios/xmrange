#!/usr/bin/env python3
"""XMRange test runner: launch xmrig-mo + macmon, pair power with hashrate, report H/W.

Usage:
  tools/xmrange_run.py --config configs/pool-test.json --label rx0-8t --duration 900 --warmup 180
  tools/xmrange_run.py --config configs/pool-all-algos.json --label mo-calibration --until-calibrated

Writes runs/<stamp>-<label>/: config.json (working copy, xmrig may autosave into it),
xmrig.log, samples.jsonl (macmon + API rows), summary.json.
"""
import argparse, json, os, re, shutil, signal, subprocess, sys, threading, time, urllib.request
from datetime import datetime, timezone

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
XMRIG = os.path.join(ROOT, "vendor/xmrig-mo/build/xmrig")
API_PORT = 3620  # PORTS.md: XMRange 36xx band


def now():
    return time.time()


class Recorder:
    def __init__(self, path):
        self.f = open(path, "a", buffering=1)
        self.lock = threading.Lock()
        self.rows = []

    def add(self, src, data):
        row = {"t": now(), "src": src, **data}
        with self.lock:
            self.rows.append(row)
            self.f.write(json.dumps(row) + "\n")


def macmon_reader(proc, rec, stop):
    for line in proc.stdout:
        if stop.is_set():
            break
        try:
            d = json.loads(line)
        except ValueError:
            continue
        rec.add("macmon", {
            "cpu_w": d["cpu_power"], "ram_w": d["ram_power"], "sys_w": d["sys_power"],
            "all_w": d["all_power"], "temp_c": d["temp"]["cpu_temp_avg"],
            "p_mhz": d["pcpu_freq_mhz"], "e_mhz": d["ecpu_freq_mhz"],
            "p_active": d["pcpu_active_ratio"], "e_active": d["ecpu_active_ratio"],
        })


def api_request(path, token=None, method="GET", body=None):
    req = urllib.request.Request(f"http://127.0.0.1:{API_PORT}{path}", method=method,
                                 data=json.dumps(body).encode() if body is not None else None)
    if token:
        req.add_header("Authorization", f"Bearer {token}")
    if body is not None:
        req.add_header("Content-Type", "application/json")
    return urllib.request.urlopen(req, timeout=5)


def api_poller(rec, stop, interval, token=None):
    while not stop.wait(interval):
        try:
            with api_request("/2/summary", token) as r:
                d = json.load(r)
        except Exception:
            continue
        hr = d.get("hashrate", {}).get("total", [None])
        rec.add("api", {
            "algo": d.get("algo"), "hs10": hr[0], "hs60": hr[1] if len(hr) > 1 else None,
            "shares_good": d.get("results", {}).get("shares_good"),
            "diff": d.get("results", {}).get("diff_current"),
        })


def context():
    """Denominator context: Low Power Mode, display brightness, power source. All sudoless."""
    ctx = {}
    try:
        pm = subprocess.run(["pmset", "-g"], capture_output=True, text=True).stdout
        m = re.search(r"lowpowermode\s+(\d)", pm)
        ctx["lowpowermode"] = int(m.group(1)) if m else None
        b = subprocess.run(["pmset", "-g", "batt"], capture_output=True, text=True).stdout
        ctx["on_ac"] = "AC Power" in b
        m = re.search(r"(\d+)%", b)
        ctx["batt_pct"] = int(m.group(1)) if m else None
        io = subprocess.run(["ioreg", "-r", "-c", "AppleARMBacklight", "-d", "1"], capture_output=True, text=True).stdout
        m = re.search(r'"brightness"=\{"min"=(\d+),"max"=(\d+),"value"=(\d+)\}', io)
        ctx["brightness_pct"] = round(100 * int(m.group(3)) / int(m.group(2)), 1) if m else None
    except Exception:
        pass
    return ctx


def context_poller(rec, stop, interval=30):
    while not stop.wait(interval):
        rec.add("ctx", context())


LOG_TS = re.compile(r"^\[(\d{4}-\d\d-\d\d \d\d:\d\d:\d\d\.\d+)\]")
ALGO_START = re.compile(r"Algo (\S+) Starting test")
ALGO_DONE = re.compile(r"Algo (\S+) hashrate: ([\d.]+)")


def parse_calibration(log_path):
    """Return [(algo, t_start, t_end, hashrate)] from MO algo-perf calibration log lines."""
    out, starts = [], {}
    for line in open(log_path, errors="replace"):
        m = LOG_TS.match(line)
        if not m:
            continue
        t = datetime.strptime(m.group(1), "%Y-%m-%d %H:%M:%S.%f").timestamp()
        if (s := ALGO_START.search(line)):
            starts[s.group(1)] = t
        elif (s := ALGO_DONE.search(line)) and s.group(1) in starts:
            out.append((s.group(1), starts.pop(s.group(1)), t, float(s.group(2))))
    return out


def avg(rows, key):
    vals = [r[key] for r in rows if r.get(key) is not None]
    return sum(vals) / len(vals) if vals else None


def window_power(rows, t0, t1):
    m = [r for r in rows if r["src"] == "macmon" and t0 <= r["t"] <= t1]
    return {k: avg(m, k) for k in ("cpu_w", "ram_w", "sys_w", "temp_c", "p_mhz", "e_mhz")} | {"n": len(m)}


def hw(hs, w):
    return round(hs / w, 1) if hs and w else None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--config", required=True)
    ap.add_argument("--label", required=True)
    ap.add_argument("--duration", type=int, default=900, help="seconds of mining after start")
    ap.add_argument("--warmup", type=int, default=180, help="seconds excluded from steady-state stats")
    ap.add_argument("--until-calibrated", action="store_true", help="stop after MO calibration + duration")
    ap.add_argument("--taskpolicy", choices=["none", "background"], default="none",
                    help="background = clamp miner to E-cores (taskpolicy -b)")
    ap.add_argument("--api-interval", type=float, default=5.0)
    ap.add_argument("xmrig_args", nargs="*")
    a = ap.parse_args()

    stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
    run = os.path.join(ROOT, "runs", f"{stamp}-{a.label}")
    os.makedirs(run)
    cfg = os.path.join(run, "config.json")
    shutil.copy(a.config, cfg)
    log = os.path.join(run, "xmrig.log")
    rec = Recorder(os.path.join(run, "samples.jsonl"))
    stop = threading.Event()

    mm = subprocess.Popen(["macmon", "pipe", "-i", "1000"], stdout=subprocess.PIPE, text=True,
                          stderr=subprocess.DEVNULL)
    threading.Thread(target=macmon_reader, args=(mm, rec, stop), daemon=True).start()
    time.sleep(3)  # idle baseline samples

    t_start = now()
    xm = subprocess.Popen([XMRIG, "-c", cfg, f"--log-file={log}", "--no-color", *a.xmrig_args],
                          stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, stdin=subprocess.DEVNULL)
    if a.taskpolicy == "background":
        subprocess.run(["taskpolicy", "-b", "-p", str(xm.pid)], check=False)
    caf = subprocess.Popen(["caffeinate", "-i", "-w", str(xm.pid)])
    threading.Thread(target=api_poller, args=(rec, stop, a.api_interval), daemon=True).start()
    ctx_start = context()
    rec.add("ctx", ctx_start)
    threading.Thread(target=context_poller, args=(rec, stop), daemon=True).start()
    print(f"▶ {a.label}: xmrig pid {xm.pid}, run dir {run}", flush=True)

    deadline = t_start + a.duration
    try:
        while now() < deadline and xm.poll() is None:
            if a.until_calibrated and os.path.exists(log) and "CALIBRATION COMPLETE" in open(log, errors="replace").read():
                deadline = min(deadline, now() + a.warmup + 120)
                a.until_calibrated = False
                print("✔ calibration complete; mining briefly before stop", flush=True)
            time.sleep(2)
    except KeyboardInterrupt:
        pass
    finally:
        t_end = now()
        if xm.poll() is None:
            xm.send_signal(signal.SIGINT)
            try:
                xm.wait(10)
            except subprocess.TimeoutExpired:
                xm.kill()
        stop.set()
        mm.terminate()
        caf.terminate()

    rows = rec.rows
    summary = {"label": a.label, "config": a.config, "taskpolicy": a.taskpolicy,
               "t_start": t_start, "t_end": t_end, "seconds": round(t_end - t_start),
               "context_start": ctx_start, "context_end": context()}

    cal = parse_calibration(log) if os.path.exists(log) else []
    if cal:
        summary["calibration"] = []
        for algo, t0, t1, hs in cal:
            p = window_power(rows, t0 + 3, t1)  # skip first seconds of each round (ramp)
            summary["calibration"].append({"algo": algo, "hs": hs, **{k: round(v, 2) if isinstance(v, float) else v for k, v in p.items()},
                                           "hw_cpu": hw(hs, p["cpu_w"]), "hw_sys": hw(hs, p["sys_w"])})

    # steady state: last mining stretch after warmup, excluding calibration
    t_mine = max([c[2] for c in cal], default=t_start) + a.warmup
    api = [r for r in rows if r["src"] == "api" and r["t"] >= t_mine and r.get("hs10")]
    if api:
        p = window_power(rows, t_mine, t_end)
        hs = avg(api, "hs10")
        summary["steady"] = {"from_s": round(t_mine - t_start), "hs": round(hs, 1),
                             "algos": sorted({r["algo"] for r in api if r.get("algo")}),
                             **{k: round(v, 2) if isinstance(v, float) else v for k, v in p.items()},
                             "hw_cpu": hw(hs, p["cpu_w"]), "hw_cpu_ram": hw(hs, (p["cpu_w"] or 0) + (p["ram_w"] or 0)),
                             "hw_sys": hw(hs, p["sys_w"]),
                             "shares_good": api[-1].get("shares_good")}
    json.dump(summary, open(os.path.join(run, "summary.json"), "w"), indent=2)
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
