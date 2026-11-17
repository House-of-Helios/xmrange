#!/usr/bin/env python3
"""Draw docs/site/examples/report-input.txt beside report-sent.txt as HTML (private things
struck through in the first, their stand-ins marked in the second), and write the fixture the
sheet screenshot uses. Both examples come from `cargo dev report-sample`, so the picture is
made from what the code really does. Render report-compare.html with shoot-report.js."""
import html, json, pathlib, re
root = pathlib.Path(__file__).resolve().parents[2]
ex = root / "docs/site/examples"
before = (ex / "report-input.txt").read_text()
after = (ex / "report-sent.txt").read_text()
planted = ["4123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz123456789ABCDEFGHJKLMNPQRSTUVWXYZabc",
           "203.0.113.7", "2001:db8:85a3::8a2e:370:7334", "Someones-MacBook-Pro.local", "/Users/someone",
           "someones-rig"]
def mark_before(t):
    t = html.escape(t)
    for p in planted:
        t = t.replace(html.escape(p), f'<mark class="out">{html.escape(p)}</mark>')
    return t
def mark_after(t):
    return re.sub(r"(\[wallet removed\]|ip hidden|\[private\]|~(?=/Library))", r'<mark class="in">\1</mark>', html.escape(t))
fonts = "../../assets/fonts"
page = f"""<!doctype html><meta charset="utf-8"><title>Report before and after</title>
<style>
@font-face{{font-family:Fraunces;font-weight:400;src:url({fonts}/fraunces-latin-400-normal.woff2)}}
@font-face{{font-family:Jost;font-weight:400;src:url({fonts}/jost-latin-400-normal.woff2)}}
@font-face{{font-family:Jost;font-weight:500;src:url({fonts}/jost-latin-500-normal.woff2)}}
@font-face{{font-family:JB;src:url({fonts}/jetbrains-mono-latin-400-normal.woff2)}}
*{{box-sizing:border-box;margin:0}}
body{{width:1400px;background:#faf6ee;color:#2b2a28;font:15px Jost,sans-serif;padding:40px 44px}}
h1{{font:400 28px Fraunces,serif;margin-bottom:22px}}
.cols{{display:grid;grid-template-columns:1fr 1fr;gap:28px}}
h2{{font:500 13px Jost;letter-spacing:.14em;text-transform:uppercase;color:#7a756c;margin-bottom:10px}}
pre{{background:#fffdf8;border:1px solid #e3ddd1;border-radius:14px;padding:18px 20px;font:12.5px/1.6 JB,ui-monospace,Menlo,monospace;white-space:pre-wrap;word-break:break-all}}
mark.out{{background:#fbd5cc;color:#9b2c14;border-radius:4px;padding:0 2px;text-decoration:line-through}}
mark.in{{background:#dcebcf;color:#2f5d1e;border-radius:4px;padding:0 2px}}
p{{margin-top:18px;color:#5d5a55}}
</style>
<h1>One report, before and after XMRange cleans it</h1>
<div class="cols"><div><h2>What the app saw</h2><pre>{mark_before(before)}</pre></div>
<div><h2>What send hands the sender, byte for byte</h2><pre>{mark_after(after)}</pre></div></div>
<p>Made by XMRange's own code from <code>docs/site/examples/</code>; a test fails if this example and the code ever disagree.</p>
"""
(root / "tools/docs-shots/report-compare.html").write_text(page)
(root / "tools/docs-shots/report-fixture.js").write_text("window.__REPORT_TEXT = " + json.dumps(after) + ";\n")
