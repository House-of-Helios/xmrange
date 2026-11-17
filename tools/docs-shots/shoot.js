// Run through the Playwright MCP (browser_run_code_unsafe filename=...) or adapt for node playwright.
// Needs: python3 -m http.server 3600 --bind 127.0.0.1 in crates/xmrange-app/ui
async (page) => {
  const ROOT = process.env.XMRANGE_ROOT || process.cwd(); // the XMRange repo root
  const OUT = `${ROOT}/docs/site/img`;
  const browser = page.context().browser();
  const errs = [];
  async function open(q, { w = 1280, h = 970, dsf = 2, scheme = "light" } = {}) {
    const ctx = await browser.newContext({ viewport: { width: w, height: h }, deviceScaleFactor: dsf, colorScheme: scheme });
    await ctx.addInitScript({ path: `${ROOT}/tools/docs-shots/stub.js` });
    const p = await ctx.newPage();
    p.on("pageerror", (e) => errs.push(`${q}: ${e}`));
    p.on("console", (m) => m.type() === "error" && !/favicon|Failed to load resource/.test(m.text()) && errs.push(`${q}: ${m.text()}`));
    await p.goto(`http://127.0.0.1:3600/index.html?${q}`);
    await p.waitForTimeout(700);
    await settle(p);
    return { ctx, p };
  }
  // The dev server drops a few requests under load: retry any visible image that did not load.
  const settle = async (p) => {
    for (let k = 0; k < 8; k++) {
      await p.waitForLoadState("networkidle");
      const bad = await p.evaluate((k) => {
        const miss = [...document.images].filter((i) => i.offsetParent && !(i.complete && i.naturalWidth > 0));
        miss.forEach((i) => { const u = new URL(i.src); u.searchParams.set("r", k); i.src = u.href; });
        return miss.length;
      }, k);
      if (!bad) return;
      await p.waitForTimeout(400);
    }
    throw new Error("images still broken");
  };
  const mining = async (p, opts) => {
    await p.evaluate((o) => { window.__shots.feed(240, o.tape ? { tape: window.__shots.tape } : {}); window.__shots.age(8048000); }, { tape: true, ...opts });
    await p.waitForTimeout(1500); // uptime ticks
    await settle(p);
  };
  const clipOf = async (p, sel, pad = 16) => {
    const b = await p.locator(sel).first().boundingBox();
    return { x: Math.max(0, b.x - pad), y: Math.max(0, b.y - pad), width: b.width + pad * 2, height: b.height + pad * 2 };
  };

  // mining tab, running (overview and mining tab)
  let { ctx, p } = await open("s=mining");
  await mining(p);
  await p.screenshot({ path: `${OUT}/overview.png` });
  await p.screenshot({ path: `${OUT}/mining-tab.png` });
  await ctx.close();

  // first run notice (idle)
  ({ ctx, p } = await open("s=first-run"));
  await p.waitForTimeout(500);
  await p.screenshot({ path: `${OUT}/first-run-notice.png` });
  await ctx.close();

  // the wallet card while the default (GhostDrip dev wallet) is in use
  ({ ctx, p } = await open("s=first-run"));
  await p.waitForTimeout(500);
  await p.screenshot({ path: `${OUT}/default-wallet.png`, clip: await clipOf(p, "#grid [data-mod=wallet]", 24) });
  await ctx.close();

  // wallet edit with a format-correct fake address
  ({ ctx, p } = await open("s=wallet"));
  await p.locator('#grid [data-act="wallet-edit"]').first().click();
  await p.locator("#grid [data-mod=wallet] input").fill("4" + "Tst".repeat(30) + "Wx7Q");
  await p.waitForTimeout(300);
  await p.screenshot({ path: `${OUT}/wallet-edit.png`, clip: await clipOf(p, "#grid [data-mod=wallet]", 24) });
  await ctx.close();

  // tuning tab mid-trial
  ({ ctx, p } = await open("s=tuning"));
  await p.evaluate(() => {
    const s = window.__shots;
    s.feed(240, { optimizer: s.opt({ sweep: s.sweep, trials_recorded: 6 }), tape: s.trialTape });
    s.age(8048000);
  });
  await p.locator('[data-tab="tuning"]').click();
  await p.waitForTimeout(1200);
  await settle(p);
  await p.screenshot({ path: `${OUT}/tuning-trial.png` });
  await ctx.close();

  // network sheet
  ({ ctx, p } = await open("s=mining"));
  await mining(p);
  await p.locator("#gear-btn").click();
  await p.locator("#set-network").click();
  await p.waitForTimeout(500);
  await p.screenshot({ path: `${OUT}/network-sheet.png` });
  await ctx.close();

  // update pill
  ({ ctx, p } = await open("s=update"));
  await mining(p);
  await p.waitForTimeout(500);
  await p.screenshot({ path: `${OUT}/update-pill.png`, clip: { x: 56, y: 4, width: 260, height: 84 } });
  await ctx.close();

  // site card: 1600x1000 layout at scale 0.9 = exactly 1440x900
  for (const [t, name] of [["dark", "card.png"], ["light", "card-light.png"]]) {
    ({ ctx, p } = await open(`s=mining&t=${t}`, { w: 1600, h: 1000, dsf: 0.9, scheme: t }));
    await mining(p);
    await p.screenshot({ path: `${OUT}/${name}` });
    await ctx.close();
  }
  return errs;
}
