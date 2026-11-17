// Renders tools/docs-shots/diagrams/*.html to docs/site/img/*.png at 2x.
// Run through Playwright MCP (browser_run_code_unsafe filename=...). `process` is absent there:
// copy this to _devdata/, set ROOT to the repo's absolute path, run the copy.
async (page) => {
  const ROOT = "REPO_ROOT";
  const names = ["trial-math", "trial-flow", "privacy-map"];
  const browser = page.context().browser();
  for (const n of names) {
    const ctx = await browser.newContext({ deviceScaleFactor: 2, viewport: { width: 1200, height: 700 } });
    const p = await ctx.newPage();
    await p.goto(`file://${ROOT}/tools/docs-shots/diagrams/${n}.html`);
    const b = await p.evaluate(() => ({ w: document.body.offsetWidth, h: document.body.offsetHeight }));
    await p.screenshot({ path: `${ROOT}/docs/site/img/${n}.png`, clip: { x: 0, y: 0, width: b.w, height: b.h } });
    await ctx.close();
  }
}
