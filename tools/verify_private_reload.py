#!/usr/bin/env python3
"""Privacy + dataset verification for the Base::reload patch (doctrine §4, module 1g).
Launch with autosave:false/watch:false, delete the 0600 config after READY, step threads via PUT.
PASS iff: config file never reappears, exactly 1 dataset init, READY threads match each step."""
import json, os, re, secrets, signal, subprocess, sys, tempfile, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from xmrange_run import ROOT, XMRIG, api_request

cfg = json.load(open(os.path.join(ROOT, "configs/rx0-only-8t.json")))
token = secrets.token_hex(16)
cfg.update({"autosave": False, "watch": False})
cfg["http"].update({"restricted": False, "access-token": token})
d = tempfile.mkdtemp(prefix="xmrange-", dir=os.path.join(ROOT, "_devdata") if os.path.isdir(os.path.join(ROOT, "_devdata")) else None)
os.chmod(d, 0o700)
path, log = os.path.join(d, "config.json"), os.path.join(d, "xmrig.log")
json.dump(cfg, open(path, "w")); os.chmod(path, 0o600)
xm = subprocess.Popen([XMRIG, "-c", path, f"--log-file={log}", "--no-color"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, stdin=subprocess.DEVNULL)
ok = True
try:
    t = time.time()
    while "READY threads" not in (open(log).read() if os.path.exists(log) else ""):
        assert time.time() - t < 90, "no READY"; time.sleep(1)
    os.remove(path); print("✔ READY, config deleted")
    for n in (4, 8, 2):
        live = dict(cfg); live["cpu"] = dict(cfg["cpu"]); live["cpu"]["rx"] = [-1] * n   # app = source of truth, no GET
        with api_request("/2/config", token, "PUT", live) as r:
            print(f"  PUT threads={n} → HTTP {r.status}")
        time.sleep(8)
        exists = os.path.exists(path)
        print(f"  config file on disk after PUT: {exists}"); ok &= not exists
    text = open(log).read()
    inits = text.count("init dataset"); readies = re.findall(r"READY threads (\d+)/", text)
    print(f"  dataset inits: {inits}   READY thread counts: {readies}")
    ok &= inits == 1 and readies[-3:] == ["4", "8", "2"]
    ok &= "configuration saved" not in text
finally:
    xm.send_signal(signal.SIGINT); xm.wait(10)
    for f in os.listdir(d): os.remove(os.path.join(d, f))
    os.rmdir(d)
print("PASS ✅" if ok else "FAIL ❌"); sys.exit(0 if ok else 1)
