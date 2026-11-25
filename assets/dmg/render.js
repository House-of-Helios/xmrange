// Run through the Playwright MCP (browser_run_code_unsafe filename=assets/dmg/render.js), like tools/docs-shots/shoot.js.
// Needs: python3 -m http.server 3600 --bind 127.0.0.1 in the XMRange repo root.
// Makes background.png (660x400), its 2x Retina twin, and preview.png (2x, with icons and labels).
async (page) => {
  const ROOT = process.env.XMRANGE_ROOT || process.cwd(); // the XMRange repo root
  const browser = page.context().browser();
  for (const [scale, preview] of [[1, false], [2, false], [2, true]]) {
    const name = preview ? "preview.png" : `background${scale === 2 ? "@2x" : ""}.png`;
    const ctx = await browser.newContext({ viewport: { width: 660, height: 400 }, deviceScaleFactor: scale });
    const pg = await ctx.newPage();
    await pg.goto("http://127.0.0.1:3600/assets/dmg/background.html"); await pg.waitForLoadState("networkidle");
    if (preview) await pg.evaluate(() => document.getElementById("c").classList.add("p"));
    await pg.evaluate(() => document.fonts.ready); await pg.waitForTimeout(300);
    await pg.screenshot({ path: `${ROOT}/assets/dmg/${name}` }); await ctx.close();
  }
}
