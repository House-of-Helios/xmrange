// Run through the Playwright MCP (browser_run_code_unsafe filename=...). file:// is fine, no server.
async (page) => {
  const ROOT = process.env.XMRANGE_ROOT || process.cwd();
  const jobs = [["about-this-mac", 880, 420], ["open-anyway", 960, 620]];
  const browser = page.context().browser();
  for (const [n, w, h] of jobs) {
    const ctx = await browser.newContext({ viewport: { width: w, height: h }, deviceScaleFactor: 2, colorScheme: "light" });
    const p = await ctx.newPage();
    await p.goto(`file://${ROOT}/tools/docs-shots/macos/${n}.html`);
    await p.screenshot({ path: `${ROOT}/docs/site/img/${n}.png` });
    await ctx.close();
  }
  return "ok";
}
