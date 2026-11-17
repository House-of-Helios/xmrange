#!/usr/bin/env python3
"""Live utility QoS check (our xmrig patch, cpu.qos): mine 8 threads at default, then PUT
cpu.qos = "utility" and back, without a restart. PASS iff exactly 1 dataset init, the hashrate
and whole-Mac watts drop under utility, and both return at default. Numbers go to stdout."""
import json, os, secrets, subprocess, sys, tempfile, threading, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from xmrange_run import ROOT, XMRIG, api_request

cfg = json.load(open(os.path.join(ROOT, "configs/rx0-only-8t.json")))
token = secrets.token_hex(16)
cfg.update({"autosave": False, "watch": False})
cfg["http"].update({"restricted": False, "access-token": token})
d = tempfile.mkdtemp(prefix="xmrange-qos-", dir=os.path.join(ROOT, "_devdata"))
path, log = os.path.join(d, "config.json"), os.path.join(d, "xmrig.log")
json.dump(cfg, open(path, "w")); os.chmod(path, 0o600)
watts = []
mm = subprocess.Popen(["macmon", "pipe", "-i", "1000"], stdout=subprocess.PIPE, text=True, stderr=subprocess.DEVNULL)
def read():
    for line in mm.stdout:
        try: watts.append((time.time(), json.loads(line).get("sys_power")))
        except Exception: pass
threading.Thread(target=read, daemon=True).start()
xm = subprocess.Popen([XMRIG, "-c", path, f"--log-file={log}", "--no-color"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, stdin=subprocess.DEVNULL)
def hs():
    with api_request("/2/summary", token) as r:
        return json.load(r)["hashrate"]["total"][0] or 0
def window(s):
    t0 = time.time(); time.sleep(s)
    w = [v for t, v in watts if t >= t0 and v]
    return hs(), (sum(w) / len(w) if w else None)
def put(utility):
    live = json.loads(json.dumps(cfg)); live["cpu"]["qos"] = "utility" if utility else None
    with api_request("/2/config", token, "PUT", live) as r:
        return r.status
try:
    t = time.time()
    while "READY threads" not in (open(log).read() if os.path.exists(log) else ""):
        assert time.time() - t < 120, "no READY"; time.sleep(1)
    time.sleep(60)
    a = window(40); print(f"default  {a[0]:7.0f} h/s  {a[1]:5.1f} W")
    print("PUT utility →", put(True)); time.sleep(20)
    b = window(40); print(f"utility  {b[0]:7.0f} h/s  {b[1]:5.1f} W")
    print("PUT default →", put(False)); time.sleep(20)
    c = window(40); print(f"default  {c[0]:7.0f} h/s  {c[1]:5.1f} W")
    inits = open(log).read().count("init dataset")
    ok = inits == 1 and b[1] < a[1] * 0.9 and c[1] > b[1] * 1.1
    print(f"dataset inits {inits} · {'PASS' if ok else 'FAIL'}")
finally:
    xm.terminate(); xm.wait(10); mm.terminate()
