// Run through the Playwright MCP (browser_run_code_unsafe filename=...) from the repo root, after
// make-report-compare.py. Needs python3 -m http.server 3600 --bind 127.0.0.1 in crates/xmrange-app/ui.
async (page) => {
  // The MCP sandbox has no `process`: run a copy with __ROOT__ replaced (kept in _devdata/, gitignored).
  const ROOT = typeof process === "undefined" ? "__ROOT__" : process.env.XMRANGE_ROOT || process.cwd();
  const OUT = `${ROOT}/docs/site/img`;
  const browser = page.context().browser();
  const mk = async (w, h, dsf) => {
    const ctx = await browser.newContext({ viewport: { width: w, height: h }, deviceScaleFactor: dsf, colorScheme: "light" });
    await ctx.addInitScript({ path: `${ROOT}/tools/docs-shots/report-fixture.js` });
    await ctx.addInitScript({ path: `${ROOT}/tools/docs-shots/stub.js` });
    return ctx;
  };
  let ctx = await mk(1400, 800, 1.5);
  let p = await ctx.newPage();
  await p.goto(`file://${ROOT}/tools/docs-shots/report-compare.html`);
  await p.evaluate(() => document.fonts.ready);
  await p.waitForTimeout(500);
  await p.screenshot({ path: `${OUT}/report-compare.png`, fullPage: true });
  await ctx.close();

  ctx = await mk(1280, 1300, 2);
  p = await ctx.newPage();
  await p.goto("http://127.0.0.1:3600/index.html?s=mining");
  await p.waitForTimeout(900);
  await p.evaluate(() => window.__shots.feed(240, { tape: window.__shots.tape }));
  await p.waitForTimeout(500);
  await p.locator("#gear-btn").click();
  await p.locator("#fb-open").click();
  await p.waitForTimeout(700);
  await p.locator("#fb-note").fill("the hashrate fell after sleep.");
  await p.locator("#fb-text").evaluate((e) => { e.style.height = "560px"; });
  await p.waitForTimeout(400);
  await p.locator("#fb-text").evaluate((e) => { e.focus(); e.setSelectionRange(0, 0); e.scrollTop = 0; });
  await p.locator("#feedback .sheet__card").screenshot({ path: `${OUT}/report-sheet.png` });
  await ctx.close();

  ctx = await mk(1280, 970, 2);
  p = await ctx.newPage();
  await p.goto("http://127.0.0.1:3600/index.html?s=crash");
  await p.waitForTimeout(900);
  await p.evaluate(() => window.__shots.feed(240, { tape: window.__shots.tape }));
  await p.waitForTimeout(800);
  await p.screenshot({ path: `${OUT}/crash-offer.png` });
  await ctx.close();
}
