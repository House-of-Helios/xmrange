// Self-check harness: lets an agent verify the window actually works
// (IPC, events, fonts, bundled assets) without seeing it rendered.
// xmrange-dev-harness: dev builds only. It lives outside ui/ (the frontend a release
// embeds) and main.rs serves it via include_bytes! behind dev-diagnostics.
// tools/check-release.sh fails on this marker in any binary or in ui/.

import {
  buildView, CHART_H, NOT_APPLIED, p2poolView, trialLabelHtml, COST_WINDOW_NOTE, curveView, EARNED_CAP, EMOJI, HW_CAP, linePath, lowPowerGainView, lowPowerGuideText, modulesFor, profileView, picksFor, PROFILES, renderGrid,
  TAPE_LEGEND, TAPE_LINES, trialsProgressView, trialsView,
} from "./modules.js";

/** The approved mockup's own figures (2026-09-23 runs), used to prove every card still fits with real-length text. */
/** The throttle pills' blurbs on the measured M3 Air: h/w, then the chosen setup. */
const MOCK_PICKS = {
  light: { blurb: "107 h/w sys", blurb2: "4 e-cores · lpm" },
  efficient: { blurb: "200 h/w sys", blurb2: "5 threads · lpm" },
  max: { blurb: "143 h/w sys", blurb2: "8 threads · full" },
};

function mockupView(base, walletEditing) {
  const hs = [1834, 1832, 1824, 1822, 1828, 1827, 1828, 1826, 1808, 1801, 1818, 1822, 1831, 1828, 1825, 1818, 1812, 1819, 1829, 1830, 1825, 1830, 1834, 1828, 1832, 1828];
  const watts = [7.48, 8.9, 8.86, 8.9, 8.86, 8.87, 8.87, 8.88, 8.89, 8.91, 8.92, 8.97, 9.03, 8.92, 8.91, 8.91, 8.92, 8.95, 8.97];
  const temps = [106, 113, 116, 117, 119, 119, 120, 121, 121, 122, 122, 122, 122, 122, 123, 123, 123, 124, 126];
  const curve = [279, 728, 1095, 1363, 1825, 1830, 1743, 1858];
  return {
    ...base,
    stats: {
      ...base.stats,
      hw: { value: "200", sub: HW_CAP.sys },
      difficulty: { value: "45,401" },
      uptime: { value: "2 h 14 m" },
      shares: { value: "93 / 0" },
      chip: { value: "apple m3", sub: "4 performance + 4 efficiency" },
      cpu: { value: "5 of 8 busy", sub: "p 1,923 · e 2,554 mhz", bar: 62 },
      temperature: { value: "131°f", bar: 48 },
      memory: { value: "12.5 / 16 gb", bar: 78 },
      lowpower: { value: "on" },
      awake: { value: "kept awake" },
    },
    hashrate: { value: "1,826", sub: "h/s · 10 s average · peak 1,966", series: hs, lo: 1760, hi: 1860 },
    threads: { text: "5 of 8", split: "4 performance · 1 efficiency" },
    earned: { value: "0.000144" },
    cost: { value: "$0.035" },
    worker: "xmrange-m3air",
    wallet: walletEditing
      ? { ...base.wallet, editing: true, draft: "4" + "B".repeat(27), hint: "28 / 95 characters", hintState: "" }
      : base.wallet,
    tape: [
      { ts: "14:00:45", tag: "cpu", msg: "accepted (91/0) diff 45401 (67 ms)" },
      { ts: "14:00:55", tag: "cpu", msg: "accepted (92/0) diff 45401 (106 ms)" },
      { ts: "14:00:59", tag: "cpu", msg: "accepted (93/0) diff 45401 (104 ms)" },
      { ts: "14:01:00", tag: "net", msg: "new job diff 45401 algo rx/0 height 3768915" },
      { ts: "14:01:03", tag: "miner", msg: "speed 10s/60s 1826.6 1855.7 h/s max 1966.2" },
    ],
    power: {
      value: "9.1 w", sub: "whole mac · cpu 3.8 · ram 0.8", source: "outlet", sourceSub: "plugged in", sourceIcon: "hoh_source_outlet",
      watts, wLo: 4, wHi: 10, temps, tLo: 90, tHi: 140,
    },
    thermal: { word: "nominal", level: 1 },
    profile: {
      ...base.profile, value: "efficient", sub: "5 threads · low power mode", sub2: "holding: 5 threads · low power", selected: "efficient",
      choices: base.profile.choices.map((c) => ({ ...c, disabled: false, ...MOCK_PICKS[c.id] })),
    },
    optimizer: {
      value: "done", sub: "all trials done", buttons: [{ act: "trials-go", label: "run trials again" }],
      log: [
        { ts: "14:00:00", tag: "trial", msg: "started 5 threads · low power mode" },
        { ts: "14:09:00", tag: "trial", msg: "settled · measuring" },
        { ts: "14:12:00", tag: "trial", msg: "measured 5 threads · low power mode · 200.4 h/w" },
      ],
    },
    lowPowerGain: {
      ready: true,
      rows: [
        { what: "whole mac", off: "143", on: "220", gain: "+54%" },
        { what: "cpu only", off: "274", on: "600", gain: "+119%" },
      ],
    },
    curve: {
      maxThreads: 8,
      cap: "best h/s added per watt · low power mode",
      bars: curve.map((v, i) => ({ n: String(i + 1), hs: v.toLocaleString("en-US"), h: Math.round((v / 1900) * 160), best: i === 4 })),
      foot: "the 5th core added +462 h/s for +0.21 w",
      footMuted: "threads 6–8 add almost nothing",
    },
    trials: {
      value: "19",
      sub: "2.7 h most efficient h/w",
      rows: [
        { label: "8 threads · low power mode", hw: "220", best: true },
        { label: "5 threads · low power mode", hw: "200" },
        { label: "6 threads · low power mode", hw: "187" },
        { label: "8 threads · full power", hw: "143" },
        { label: "6 threads · full power", hw: "142" },
        { label: "4 threads · full power", hw: "122" },
        { label: "4 efficiency · background", hw: "106" },
      ],
    },
  };
}

function waitForHeartbeat(timeoutMs) {
  return new Promise((resolve) => {
    let settled = false;
    let unlisten;
    const timer = setTimeout(() => {
      if (settled) return;
      settled = true;
      if (unlisten) unlisten();
      resolve(false);
    }, timeoutMs);

    window.__TAURI__.event
      .once("xmrange://heartbeat", () => {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        resolve(true);
      })
      .then((fn) => {
        unlisten = fn;
      })
      .catch(() => {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        resolve(false);
      });
  });
}

function loadEmoji(path) {
  return new Promise((resolve) => {
    const img = new Image();
    img.onload = () => resolve(img.naturalWidth > 0);
    img.onerror = () => resolve(false);
    img.src = path;
  });
}

/** Card count on the live grid. */
function cardCount() {
  return document.querySelectorAll("#grid .card").length;
}

function textIsClean() {
  return !/NaN|Infinity/.test(document.body.innerText);
}

const inkCtx = document.createElement("canvas").getContext("2d");

/**
 * No text clips, descenders included. Two ways a line gets cut:
 *  - its box is squeezed: an element's scrollHeight/Width outgrows its clientHeight/Width
 *    (a flex row shrunk to fit: low power's caption, the chip's "16 gb");
 *  - its glyphs' ink (canvas measureText: the real ascent/descent of that string in
 *    that font, so a "g", "p" or "y" counts) leaves the card's inner box (inside its
 *    padding) or the box of any ancestor that clips (overflow other than visible).
 * The tape is a log window whose long lines clip by design; screen-reader text is
 * clipped to 1px on purpose; text inputs scroll.
 */
function textClips(card) {
  const bad = [];
  const id = card.dataset.mod;
  const cr = card.getBoundingClientRect();
  const cs = getComputedStyle(card);
  const inner = { top: cr.top + parseFloat(cs.paddingTop), bottom: cr.bottom - parseFloat(cs.paddingBottom) };
  const skip = (el) => !!el.closest(".tape, .sr-only, svg") || el.matches("input");
  card.querySelectorAll("*").forEach((el) => {
    if (skip(el)) return;
    if (el.clientHeight > 0 && el.scrollHeight > el.clientHeight + 1) bad.push(`${id}:${el.className || el.tagName.toLowerCase()}:squeezed ${el.scrollHeight}>${el.clientHeight}${el.matches(".trial, .trial *") ? ` [${getComputedStyle(el).fontSize} / ${getComputedStyle(el).lineHeight}]` : ""}`);
  });
  const walker = document.createTreeWalker(card, NodeFilter.SHOW_TEXT);
  const range = document.createRange();
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    const text = n.textContent.trim();
    const el = n.parentElement;
    if (!text || !el || skip(el)) continue;
    const s = getComputedStyle(el);
    if (s.visibility === "hidden" || s.display === "none") continue;
    inkCtx.font = `${s.fontStyle} ${s.fontWeight} ${s.fontSize} ${s.fontFamily}`;
    const m = inkCtx.measureText(text);
    if (![m.fontBoundingBoxDescent, m.actualBoundingBoxAscent, m.actualBoundingBoxDescent].every(Number.isFinite)) {
      bad.push(`${id}:no text metrics`);
      continue;
    }
    const where = `${id}:"${text.slice(0, 24)}"`;
    range.selectNodeContents(n);
    for (const r of range.getClientRects()) {
      if (r.width === 0) continue;
      // The text box is the font's ascent + descent; its baseline sits the descent above its bottom.
      const baseline = r.bottom - m.fontBoundingBoxDescent;
      const ink = { top: baseline - m.actualBoundingBoxAscent, bottom: baseline + m.actualBoundingBoxDescent };
      if (ink.top < inner.top - 0.5 || ink.bottom > inner.bottom + 0.5) {
        bad.push(`${where}:ink ${ink.top.toFixed(1)}–${ink.bottom.toFixed(1)} outside the card's inner box ${inner.top.toFixed(1)}–${inner.bottom.toFixed(1)}`);
        continue;
      }
      for (let a = el; a && a !== card; a = a.parentElement) {
        const o = getComputedStyle(a);
        if (o.overflowX === "visible" && o.overflowY === "visible") continue;
        const ar = a.getBoundingClientRect();
        if (ink.top < ar.top - 0.5 || ink.bottom > ar.bottom + 0.5) {
          bad.push(`${where}:ink ${ink.top.toFixed(1)}–${ink.bottom.toFixed(1)} cut by ${a.className || a.tagName.toLowerCase()} ${ar.top.toFixed(1)}–${ar.bottom.toFixed(1)}`);
          break;
        }
      }
    }
  }
  return bad;
}

/** Every card's text on the live grid (see textClips). */
function clippedText() {
  return [...document.querySelectorAll("#grid .card")].flatMap(textClips);
}

/**
 * Text-fit rule: nothing inside a card is clipped. Every card's content fits
 * its box, every single-line text (values, captions, pills) fits its width, and
 * no text is cut, descenders included (textClips).
 * Returns the offending module ids (empty = all fit).
 */
function overflowingCards() {
  const bad = [];
  document.querySelectorAll("#grid .card").forEach((card) => {
    const id = card.dataset.mod;
    if (card.scrollHeight > card.clientHeight + 1 || card.scrollWidth > card.clientWidth + 1) {
      bad.push(`${id}:box`);
      return;
    }
    bad.push(...textClips(card));
    const cr = card.getBoundingClientRect();
    card.querySelectorAll("*").forEach((el) => {
      if (el.closest(".tape")) return; // the tape is a log window; long lines clip by design
      if (el.closest(".sr-only")) return; // screen-reader text is clipped to 1px on purpose
      const r = el.getBoundingClientRect();
      if (r.width === 0 && r.height === 0) return;
      if (r.right > cr.right - 15 || r.left < cr.left + 15 || r.bottom > cr.bottom - 13) {
        bad.push(`${id}:${el.className || el.tagName.toLowerCase()}`);
      }
      // Text inputs scroll by nature; everything else must show its whole line.
      if (el.matches('input[type="text"]')) return;
      if (el.children.length === 0 && el.scrollWidth > el.clientWidth + 1 && el.clientWidth > 0) {
        bad.push(`${id}:${el.className || el.tagName.toLowerCase()}:clipped ${el.scrollWidth}>${el.clientWidth} "${el.textContent}"`);
      }
    });
  });
  return bad;
}

const is1x1 = (card) => !/\b(c2|c3|c4|r2)\b/.test(card.className);

/**
 * The 1×1 anatomy on the live grid (Helios, 2026-09-24: "the bottom line below the
 * description text always the same"). Measured from each card's top edge:
 *  - caption: every 1×1 card, and every 1×1-aligned column (logs, trials, profile),
 *    has one bottom row (.card__base), and all their bottoms sit within 1px of each
 *    other, and of the threads card's caption where it shows;
 *  - bars: every bar (and thermal's level bar) in a 1×1 card sits at one height, clear
 *    of the value above it and the caption below it.
 */
function anatomy() {
  const capBad = [];
  const barBad = [];
  const caps = [];
  const bars = [];
  document.querySelectorAll("#grid .card").forEach((card) => {
    const id = card.dataset.mod;
    const top = card.getBoundingClientRect().top;
    const hosts = is1x1(card) ? [card] : [...card.querySelectorAll(":scope > .joined > .joined__left")].filter((c) => !c.closest(".spark__top"));
    for (const host of hosts) {
      const bases = host.querySelectorAll(":scope > .card__base");
      if (bases.length !== 1) {
        capBad.push(`${id}:${bases.length} caption rows`);
        continue;
      }
      // P5: the progress panel became a 2-row card, so its caption no longer rides the shared
      // 1-row baseline.
      if (id === "optimizer") continue;
      caps.push({ id, y: bases[0].getBoundingClientRect().bottom - top });
    }
    if (!is1x1(card)) return;
    card.querySelectorAll(":scope > .bar, :scope > .levels").forEach((b) => {
      const r = b.getBoundingClientRect();
      const val = card.querySelector(":scope > .card__value")?.getBoundingClientRect();
      const base = card.querySelector(":scope > .card__base")?.getBoundingClientRect();
      bars.push({ id, y: r.top - top });
      if (!val || !base || r.top < val.bottom || r.bottom > base.top) barBad.push(`${id}:bar overlaps value or caption`);
    });
  });
  const spread = (list) => (list.length ? Math.max(...list.map((c) => c.y)) - Math.min(...list.map((c) => c.y)) : 0);
  if (spread(caps) > 1) capBad.push(`caption bottoms ${caps.map((c) => `${c.id}=${c.y.toFixed(1)}`).join(" ")}`);
  const threads = caps.find((c) => c.id === "threads");
  if (threads && caps.some((c) => Math.abs(c.y - threads.y) > 1)) capBad.push(`not on the threads caption line (${threads.y.toFixed(1)})`);
  if (spread(bars) > 1) barBad.push(`bar tops ${bars.map((c) => `${c.id}=${c.y.toFixed(1)}`).join(" ")}`);
  return { capBad, barBad, caps: caps.length, bars: bars.length };
}

function rect(id) {
  const el = document.querySelector(`#grid [data-mod="${id}"]`);
  if (!el) return null;
  const r = el.getBoundingClientRect();
  return { w: Math.round(r.width), h: Math.round(r.height) };
}

/**
 * Shell checks: switch tabs/pools programmatically and count cards against the
 * approved mockup (mining 11, machine 8 since the display card left, tuning 6, p2pool mining 11), check the
 * grid geometry, and scan the page for NaN/Infinity. Runs synchronously between
 * switch and restore, so nothing paints in between. Never starts mining.
 */
async function shellChecks(ui, checks, detail) {
  if (!ui) {
    checks.ui_ready = false;
    return;
  }
  try {
    await ui.ready;
    checks.ui_ready = document.querySelectorAll("#grid .card").length > 0;
  } catch {
    checks.ui_ready = false;
    return;
  }

  try {
    await document.fonts.ready;
  } catch {
    // measured with fallback fonts; geometry checks still hold
  }

  const prev = { tab: ui.state.tab, pool: ui.state.pool };
  let clean = true;
  const overflow = [];
  const mockOverflow = [];
  const capLine = [];
  const barLine = [];
  const clips = [];
  const seen = { caps: 0, bars: 0 };
  const layout = (board) => {
    const a = anatomy();
    capLine.push(...a.capBad.map((b) => `${board}:${b}`));
    barLine.push(...a.barBad.map((b) => `${board}:${b}`));
    clips.push(...clippedText().map((b) => `${board}:${b}`));
    seen.caps += a.caps;
    seen.bars += a.bars;
  };
  try {
    ui.setPool("mo");
    ui.setTab("mining");
    checks.cards_mining = cardCount() === 11;
    overflow.push(...overflowingCards());
    layout("live mining");
    clean = clean && textIsClean();
    // One 1×1 cell is (896 − 3·16) / 4 = 212 px wide; rows are 160 px; gaps 16 px.
    const hr = rect("hashrate");
    const tape = rect("tape");
    const hw = rect("hw");
    checks.grid_geometry =
      !!hr && hr.w === 440 && hr.h === 336 &&
      !!tape && tape.w === 668 && tape.h === 160 &&
      !!hw && hw.w === 212 && hw.h === 160;

    ui.setTab("machine");
    checks.cards_machine = cardCount() === 8;
    // No hole where the display card was: the 8 cards cover all 12 cells of the 4 × 3 board, and
    // low power is the 2×1 on row 3 (columns 3–4), under the chip and cpu column.
    {
      const g = document.getElementById("grid").querySelector(".card").getBoundingClientRect();
      const cells = [...document.querySelectorAll("#grid .card")].map((c) => {
        const r = c.getBoundingClientRect();
        return { id: c.dataset.mod, col: Math.round((r.left - g.left) / 228), row: Math.round((r.top - g.top) / 176), cs: Math.round((r.width + 16) / 228), rs: Math.round((r.height + 16) / 176) };
      });
      const lp = cells.find((c) => c.id === "lowpower");
      const area = cells.reduce((k, c) => k + c.cs * c.rs, 0);
      const rows = Math.max(...cells.map((c) => c.row + c.rs));
      checks.machine_grid_no_hole = area === 12 && rows === 3 && !!lp && lp.col === 2 && lp.row === 2 && lp.cs === 2 && lp.rs === 1 &&
        !document.querySelector('#grid [data-mod="display"]');
      if (!checks.machine_grid_no_hole) detail.machine_grid = cells;
    }
    overflow.push(...overflowingCards());
    layout("live machine");
    clean = clean && textIsClean();

    ui.setTab("tuning");
    checks.cards_tuning = cardCount() === 6;
    overflow.push(...overflowingCards());
    layout("live tuning");
    const joined = rect("trials");
    checks.grid_joined_2x1 = !!joined && joined.w === 440 && joined.h === 160;
    clean = clean && textIsClean();

    ui.setPool("p2");
    ui.setTab("mining");
    checks.cards_p2pool_mining = cardCount() === 11;
    // The last row is moneroocean's: logs 3×1 beside one stat (payouts here), the same size on both pools.
    const p2tape = rect("tape");
    checks.p2pool_logs_like_mining = !!p2tape && !!tape && p2tape.w === tape.w && p2tape.h === tape.h && !!rect("payouts");
    overflow.push(...overflowingCards());
    layout("live p2pool");
    clean = clean && textIsClean();

    // p2pool's figures, from p2pool's own files (the M1's real numbers, 2026-10-02): earned is the
    // solo-average from the network's difficulty and reward, never moneroocean's rate; payouts
    // come from the Ledger, the share wait from the share difficulty over this Mac's hashrate.
    const p2State = {
      ...ui.state, pool: "p2", settings: { ...ui.state.settings, period: "day" },
      node: { state: "ready", height: 3775602 }, payouts: { count: 12, atomic: 4123456789 },
      snap: synthSnap({ hs15: 1240, hs60: 1240, hs10: 1240, p2pool: { peers: 10, shares_found: 0, net_difficulty: 753538665462, net_reward: 613199000000, share_difficulty: 120673173 } }),
    };
    const p2vm = buildView(p2State);
    renderGrid(document.getElementById("grid"), modulesFor("mining", "p2"), p2vm);
    const p2Seen = { earned: p2vm.earned.value, cap: p2vm.earned.cap, payouts: p2vm.stats.payouts, node: p2vm.stats.node.sub, clip: clippedText(), over: overflowingCards() };
    detail.p2poolFigures = p2Seen;
    checks.p2pool_figures =
      p2Seen.earned === "0.000087" && p2Seen.cap === "xmr · expected" &&
      p2Seen.payouts.value === "0.0041" && p2Seen.payouts.sub === "12 payouts · 1 share per ~27 h" &&
      p2Seen.node === "10 peers · direct" && p2Seen.clip.length === 0 && p2Seen.over.length === 0;
    const noNet = buildView({ ...p2State, snap: synthSnap({ hs15: 1240, p2pool: { peers: 10 } }) });
    checks.p2pool_earned_never_mo_rate = noNet.earned.value === "—";
    // An xmr price turns earned's caption into money: typed, or the feed's (usd only).
    const typed = buildView({ ...p2State, settings: { ...p2State.settings, currency: "$", xmr_price: 542.62, price_feed: false } });
    const fed = buildView({ ...p2State, settings: { ...p2State.settings, currency: "$", xmr_price: 1, price_feed: true }, price: { usd: 542.62, source: "kraken", t: 0 } });
    const feedNoUsd = buildView({ ...p2State, settings: { ...p2State.settings, currency: "€", xmr_price: 500, price_feed: true }, price: { usd: 542.62 } });
    detail.price = { typed: typed.earned.cap, fed: fed.earned.cap, eur: feedNoUsd.earned.cap };
    checks.earned_in_money = typed.earned.cap === "≈ $0.047 expected" && fed.earned.cap === typed.earned.cap && feedNoUsd.earned.cap === "≈ €0.044 expected";
    // The cores card's e pill: by hand, the light profile's setup (e-cores only, threads ≤ e-cores).
    const eState = { ...ui.state, settings: { ...ui.state.settings, threads: 3, e_cores_only: true, optimizer: { ...(ui.state.settings?.optimizer || {}), mode: "manual" } }, snap: null };
    const eView = buildView(eState).threads;
    renderGrid(document.getElementById("grid"), modulesFor("mining", "mo"), buildView(eState));
    const ePill = document.querySelector('#grid [data-mod="threads"] [data-act="ecores-toggle"]');
    detail.eCores = { text: eView.text, split: eView.split, pill: !!ePill, on: ePill?.classList.contains("is-on") };
    checks.cores_card_e_cores = eView.clamp === true && /^3 of \d$/.test(eView.text) && eView.split === "3 efficiency · background" && !!ePill && ePill.classList.contains("is-on") && clippedText().length === 0;
    // A Mac without Low Power Mode (an M1 iMac) never shows its cards.
    checks.no_low_power_cards_without_it = ["machine", "tuning"].every((t) => !modulesFor(t, "mo", false).some((m) => m.id === "lowpower") && modulesFor(t, "mo", true).some((m) => m.id === "lowpower"));

    // Same board, filled with the approved mockup's real-length figures.
    const grid = document.getElementById("grid");
    const views = [mockupView(buildView(ui.state), false), mockupView(buildView(ui.state), true)];
    // A user's own wallet: "your wallet" + change → + use default. The same
    // board carries the longest machine strings (on battery, double-digit
    // watts) and the run pills' longest label.
    const userWallet = mockupView(buildView(ui.state), false);
    userWallet.wallet = { ...userWallet.wallet, isDefault: false, masked: "4Q…AAAA", note: "your wallet", action: "change →" };
    userWallet.power = { ...userWallet.power, value: "18.8 w", sub: "whole mac · cpu 12.4 · ram 1.2", source: "battery", sourceSub: "100% left", sourceIcon: "hoh_source_battery" };
    userWallet.stats = { ...userWallet.stats, cpu: { value: "10 of 10 busy", sub: "p 4,056 · e 2,748 mhz", bar: 100 }, memory: { value: "23.9 / 24 gb", bar: 99 } };
    userWallet.thermal = { word: "trapping", level: 4 };
    userWallet.runPills = { primary: { glyph: "play", label: "resume", aria: "resume mining", title: "", disabled: false }, stop: { disabled: false } };
    views.push(userWallet);
    // A user's wallet in edit mode, at its fullest: save · cancel · use default, under a
    // three-line backend refusal.
    const userEdit = mockupView(buildView(ui.state), false);
    userEdit.wallet = {
      ...userWallet.wallet, editing: true, draft: "4abc", hintState: "bad",
      hint: "not a Monero primary address or subaddress (95 base58 characters starting with 4 or 8)",
    };
    views.push(userEdit);
    const boards = [["mining", "mo"], ["machine", "mo"], ["tuning", "mo"], ["mining", "p2"]];
    let useDefault = false;
    views.forEach((vm, v) => {
      for (const [tab, pool] of boards) {
        renderGrid(grid, modulesFor(tab, pool), vm);
        mockOverflow.push(...overflowingCards());
        layout(`mock${v} ${tab} ${pool}`);
        clean = clean && textIsClean();
        if (vm === userEdit && tab === "mining" && pool === "mo") {
          const b = document.querySelector('#grid [data-mod="wallet"] [data-act="wallet-clear"]');
          useDefault = !!b && b.textContent.trim() === "use default";
        }
      }
    });
    // A user's wallet goes back to the default from its edit mode (the card's view has no links).
    checks.wallet_use_default_in_edit = useDefault;

    // Dark tokens resolve.
    const root = document.documentElement;
    const theme = root.dataset.theme;
    root.dataset.theme = "dark";
    checks.theme_dark_tokens = getComputedStyle(document.body).backgroundColor === "rgb(27, 32, 37)";
    root.dataset.theme = theme;
  } catch {
    clean = false;
  } finally {
    ui.setPool(prev.pool);
    ui.setTab(prev.tab);
  }
  checks.no_nan_infinity = clean;
  checks.cards_fit = overflow.length === 0;
  if (overflow.length) detail.overflow = overflow;
  checks.cards_fit_mockup_values = mockOverflow.length === 0;
  if (mockOverflow.length) detail.overflow_mockup = [...new Set(mockOverflow)];
  // Every visible 1×1 caption on one line, per tab (live and mockup figures).
  checks.caption_line_aligned = capLine.length === 0 && seen.caps > 0;
  if (!checks.caption_line_aligned) detail.caption_line = capLine.length ? [...new Set(capLine)] : "no captions measured";
  checks.bar_row_aligned = barLine.length === 0 && seen.bars > 0;
  if (!checks.bar_row_aligned) detail.bar_row = barLine.length ? [...new Set(barLine)] : "no bars measured";
  checks.text_no_clip = clips.length === 0;
  if (clips.length) detail.text_clips = [...new Set(clips)];

  const bg = getComputedStyle(document.body).backgroundColor;
  checks.theme_ground_token = bg === "rgb(250, 246, 238)" || bg === "rgb(27, 32, 37)";
}

// ── mining tab: synthetic snapshots through the real render path ───────────

const TYPO_ADDRESS = "4Q" + "A".repeat(93);
const $q = (sel) => document.querySelector(sel);
const textOf = (sel) => ($q(sel)?.textContent ?? "").trim();
const cardVal = (id) => textOf(`#grid [data-mod="${id}"] ${id === "hashrate" ? ".spark__value" : ".card__value"}`);
const errMsg = (err) => (typeof err === "string" ? err : String(err?.message ?? err));

/** A snapshot shaped like the engine's, every live figure empty unless overridden. */
function synthSnap(over) {
  return {
    t: 0, state: "mining", error: null, threads: 5, worker: "selfcheck", wallet_masked: "4Q…AAAA",
    hs10: null, hs60: null, hs15: null, hs_max: null, shares_good: 0, shares_rejected: 0, diff: 0, algo: null,
    uptime_s: 0, power: null, sys_w_60: null, cpu_w_60: null, sys_w_900: null, hw_sys: null, hw_cpu: null, context: null, tape: [], ...over,
  };
}

function cardOverflow(id) {
  return overflowingCards().filter((b) => b.startsWith(`${id}:`));
}

/**
 * Mining tab, live: feeds synthetic snapshots through ui.applySnapshot (the
 * path the real xmrange://snapshot event takes), walks the wallet, worker,
 * period and notice flows against the real backend where they are safe
 * (nothing valid is ever saved), then restores the app's state. Never mines.
 */
async function miningChecks(ui, checks, detail, info, typoMsg) {
  const st = ui.state;
  const saved = {
    snap: st.snap, series: st.series.slice(), tape: st.tape.slice(), lastError: st.lastError,
    powerSeries: st.powerSeries.slice(), context: st.context,
    tab: st.tab, pool: st.pool, started: st.ui.startedThisSession,
  };
  const failed = [];
  const check = (name, ok, info2) => {
    checks[name] = !!ok;
    if (!ok && info2 !== undefined) failed.push({ [name]: info2 });
  };
  try {
    ui.setPool("mo");
    ui.setTab("mining");
    const settings = st.settings || {};
    const mult = { day: 1, week: 7, month: 30 }[settings.period] ?? 1;

    // 1. power: null and hs10: null → every live figure is "—", no NaN/Infinity.
    st.powerSeries = [];
    ui.applySnapshot(synthSnap({}));
    const nulls = ["hashrate", "hw", "earned", "cost"].map(cardVal);
    check(
      "snapshot_null_dashes",
      nulls.every((v) => v === "—") && textOf('#grid [data-mod="hashrate"] .spark__cap') === "h/s · 10 s average · peak —" && textIsClean(),
      nulls,
    );

    // 2. Real-shaped figures render as the mockup formats them.
    const power = { cpu_w: 3.8, ram_w: 0.8, sys_w: 9.1, temp_c: 52.4, p_mhz: 1923, e_mhz: 2554, p_active: 62, e_active: 60, mem_used_gb: 12.5, mem_total_gb: 16 };
    st.powerSeries = [];
    const base = { power, sys_w_900: 9.1, hs_max: 1966.2, hw_sys: 199.6, hw_cpu: 479.1, diff: 45401, shares_rejected: 0 };
    ui.applySnapshot(synthSnap({ ...base, hs10: 1820.2, uptime_s: 8039, shares_good: 92, tape: [{ ts: "14:00:55", tag: "cpu", msg: "accepted (92/0) diff 45401 (106 ms)" }] }));
    ui.applySnapshot(synthSnap({
      ...base, hs10: 1826.6, uptime_s: 8040, shares_good: 93,
      tape: [
        { ts: "14:00:59", tag: "cpu", msg: "accepted (93/0) diff 45401 (104 ms)" },
        { ts: "14:01:03", tag: "miner", msg: "speed 10s/60s 1826.6 1855.7 h/s max 1966.2" },
      ],
    }));
    const earned = 1826.6 * 7.875e-8 * mult;
    const cost = ((9.1 * 24) / 1000) * settings.rate_per_kwh * mult;
    const want = {
      hashrate: "1,827",
      hw: settings.measure === "cpu" ? "479" : "200",
      earned: earned.toFixed(earned < 0.001 ? 6 : 5),
      cost: `${settings.currency || "$"}${cost.toFixed(mult === 1 ? 3 : 2)}`,
      difficulty: "45,401",
      shares: "93 / 0",
    };
    const got = Object.fromEntries(Object.keys(want).map((k) => [k, cardVal(k)]));
    const lines = document.querySelectorAll('#grid [data-mod="tape"] .tape__line');
    const last = lines[lines.length - 1];
    const spark = $q('#grid svg[data-chart="hashrate"] path.chart__line');
    check(
      "snapshot_values_render",
      Object.keys(want).every((k) => want[k] === got[k]) &&
        textOf('#grid [data-mod="hashrate"] .spark__cap') === "h/s · 10 s average · peak 1,966" &&
        /^\d+ (s|m)$|^\d+ h \d+ m$/.test(cardVal("uptime")) &&
        lines.length === Math.min(TAPE_LINES, saved.tape.length + 3) &&
        !!last && last.textContent.includes("speed 10s/60s 1826.6") && !!last.querySelector(".tag--miner") &&
        !!spark && /^M/.test(spark.getAttribute("d") || "") &&
        textIsClean(),
      { want, got, lines: lines.length, spark: !!spark },
    );

    // 2b. Ruling P5-O2: the cards read the engine's rolling fields, never the instant ones. h/w is
    // hw_sys / hw_cpu (hs60 over the same 60 s of watts); earned takes hs15, else the longest
    // average xmrig has; cost is the mean of the power chart's whole-mac watts, mining or not.
    const earnedOf = (hs) => {
      const e = hs * 7.875e-8 * mult;
      return e.toFixed(e < 0.001 ? 6 : 5);
    };
    const costOf = (w) => `${settings.currency || "$"}${(((w * 24) / 1000) * settings.rate_per_kwh * mult).toFixed(mult === 1 ? 3 : 2)}`;
    st.powerSeries = [{ w: 8.0 }, { w: 8.8 }];
    const roll = { ...base, power: null, hs10: 1826.6, hs60: 1855.7, sys_w_60: 9.3, cpu_w_60: 3.9, sys_w_900: 8.4 };
    ui.applySnapshot(synthSnap({ ...roll, hs15: 1840.2 }));
    const r15 = { hw: cardVal("hw"), earned: cardVal("earned"), cost: cardVal("cost") };
    ui.applySnapshot(synthSnap(roll));
    const r60 = { earned: cardVal("earned") };
    st.powerSeries = [];
    ui.applySnapshot(synthSnap(roll));
    const rNoCost = { cost: cardVal("cost") };
    ui.applySnapshot(synthSnap({ ...roll, hw_sys: null, hw_cpu: null }));
    const rNoHw = { hw: cardVal("hw") };
    const rollWant = {
      hw: settings.measure === "cpu" ? "479" : "200", earned15: earnedOf(1840.2), cost900: costOf(8.4), earned60: earnedOf(1855.7),
    };
    check(
      "rolling_fields_drive_cards",
      r15.hw === rollWant.hw && r15.earned === rollWant.earned15 && r15.cost === rollWant.cost900 &&
        r60.earned === rollWant.earned60 && rNoCost.cost === "—" && rNoHw.hw === "—" && textIsClean(),
      { want: rollWant, r15, r60, rNoCost, rNoHw },
    );

    // 2c. Each rolling card names its window in its caption, and every caption fits (both h/w
    // denominators). Cost's caption row is full, so its window rides in the row's tooltip and
    // screen-reader text.
    const capsSeen = {};
    let capsOk = true;
    const s0 = st.settings;
    for (const m of ["sys", "cpu"]) {
      st.settings = { ...s0, measure: m };
      ui.render();
      ui.applySnapshot(synthSnap({ ...roll, hs15: 1840.2 }));
      const hwCap = textOf('#grid [data-mod="hw"] .card__base');
      const earnedRow = $q('#grid [data-mod="earned"] .card__base');
      const earnedVisible = textOf('#grid [data-mod="earned"] .card__base span[title]').replace(textOf('#grid [data-mod="earned"] .card__base span[title] .sr-only'), "").trim();
      const costRow = $q('#grid [data-mod="cost"] .card__base');
      const seen = {
        hwCap, earnedVisible, earnedTip: earnedRow?.querySelector("span[title]")?.title,
        costTip: costRow?.title, costSr: textOf('#grid [data-mod="cost"] .card__base > .sr-only'),
        fit: [...cardOverflow("hw"), ...cardOverflow("earned"), ...cardOverflow("cost")],
      };
      capsSeen[m] = seen;
      capsOk = capsOk && hwCap === HW_CAP[m] && hwCap.endsWith("· 60 s average") &&
        earnedVisible === EARNED_CAP && EARNED_CAP.endsWith("· 15 min average") && /15 min average/.test(seen.earnedTip || "") &&
        (seen.costTip || "").startsWith(COST_WINDOW_NOTE) && seen.costSr === COST_WINDOW_NOTE && /13 min average/.test(COST_WINDOW_NOTE) &&
        seen.fit.length === 0;
    }
    st.settings = s0;
    ui.render();
    check("rolling_windows_captions", capsOk, capsSeen);

    // 3. A launch failure (snapshot.error, no tape line of its own) reaches the tape.
    ui.applySnapshot(synthSnap({ state: "error", error: "self-check: xmrig did not launch" }));
    const errLine = [...document.querySelectorAll('#grid [data-mod="tape"] .tape__line')].pop();
    check(
      "snapshot_error_to_tape",
      !!errLine && !!errLine.querySelector(".tag--error") && errLine.textContent.includes("self-check: xmrig did not launch") &&
        document.body.dataset.run === "error",
    );

    // 3b. I1, in the real order: the engine's error snapshot, then xmrange://stopped with the same
    // generation. "error" and the backend's reason stay (light, tooltip, hashrate card) until the
    // next start; a stale stopped event from an older engine changes nothing.
    const reasons = ["xmrig stopped unexpectedly (signal: 9 (SIGKILL))", "port 3620 is busy (is another xmrig still running?)"];
    const errSeen = [];
    const gen0 = st.engineGen;
    let errOk = true;
    for (const reason of reasons) {
      st.engineGen = 41;
      ui.applySnapshot(synthSnap({ state: "mining", hs10: 1800 }));
      ui.applySnapshot(synthSnap({ state: "error", error: reason }));
      ui.applyStopped(40); // stale: ignored
      ui.applyStopped(41);
      const pills = readPills();
      const seen = {
        light: document.body.dataset.run,
        value: cardVal("hashrate"),
        cap: textOf('#grid [data-mod="hashrate"] .spark__cap--err'),
        primary: pills.primary,
        stopDisabled: pills.stop?.disabled,
        fit: cardOverflow("hashrate"),
      };
      errSeen.push(seen);
      errOk = errOk && seen.light === "error" &&
        seen.value === "error" && seen.cap === reason && seen.primary?.label === "start" && !seen.primary.disabled &&
        seen.stopDisabled === true && seen.fit.length === 0 && textIsClean();
    }
    st.engineGen = gen0;
    check("error_survives_stopped", errOk, errSeen);

    // 4. xmrange://stopped after a normal run: the light says stopped, hashrate goes to "—", and cost
    //    (the Mac's electricity, mining or not) keeps its figure.
    ui.applySnapshot(synthSnap({ state: "mining", hs10: 1800, power: { cpu_w: 3.8, ram_w: 0.8, sys_w: 9.1, temp_c: 52, p_mhz: 1, e_mhz: 1, p_active: 0.5, e_active: 0.5 } }));
    ui.applyStopped();
    check("stopped_clears_rates", document.body.dataset.run === "stopped" && cardVal("hashrate") === "—" && cardVal("cost") !== "—");
  } catch (err) {
    failed.push({ synthetic: String(err) });
    checks.snapshot_values_render = false;
  } finally {
    st.snap = saved.snap;
    st.series = saved.series;
    st.powerSeries = saved.powerSeries;
    st.context = saved.context;
    st.tape = saved.tape;
    st.lastError = saved.lastError;
    ui.refresh();
  }

  try {
    // 5. The default wallet reads as GhostDrip's dev wallet (never XMRange's): the disclosure is
    // the caption, word for word, and "use yours →" has left the card (the pencil pill edits).
    const wtext = ($q('#grid [data-mod="wallet"]')?.innerText ?? "").replace(/\s+/g, " ");
    const wcap = textOf('#grid [data-mod="wallet"] .card__base');
    check(
      "wallet_card_label",
      (info?.wallet?.is_default
        ? wcap === "default · ghostdrip dev wallet" && !/xmrange|project/i.test(wtext)
        : wcap === "your wallet · click to edit") &&
        !wtext.includes("use yours") && !wtext.includes("change →") &&
        textOf('#grid [data-mod="wallet"] .card__value') === (st.info?.wallet?.masked ?? "—"),
      { wtext, wcap },
    );
    // Its emoji is every 1×1 card's: 40px, at the same height as the h/w card's.
    {
      const card = $q('#grid [data-mod="wallet"]').getBoundingClientRect();
      const icon = $q('#grid [data-mod="wallet"] > img')?.getBoundingClientRect();
      const ref = $q('#grid [data-mod="hw"]').getBoundingClientRect();
      const refIcon = $q('#grid [data-mod="hw"] > img').getBoundingClientRect();
      const got = icon && { w: icon.width, h: icon.height, top: icon.top - card.top };
      const want = { w: refIcon.width, h: refIcon.height, top: refIcon.top - ref.top };
      check("wallet_icon_standard", !!got && got.w === want.w && got.h === want.h && Math.abs(got.top - want.top) <= 0.5 && want.w === 40, { got, want });
    }

    // 5b. The first-run notice's own "use yours →" still opens the wallet's edit mode, focused.
    ui.setTab("machine");
    document.getElementById("notice-use").click();
    const noticeEdit = {
      tab: st.tab, editing: st.ui.walletEditing,
      focused: document.activeElement === $q('#grid [data-field="wallet"]'),
    };
    ui.actions["wallet-cancel"]();
    check("notice_opens_wallet_edit", noticeEdit.tab === "mining" && noticeEdit.editing && noticeEdit.focused, noticeEdit);

    // 6. Wallet edit flow: live hint, then the backend's refusal shown verbatim.
    const input = () => $q('#grid [data-field="wallet"]');
    const hintEl = () => $q("#grid .wallet__hint");
    const type = (v) => {
      const el = input();
      el.value = v;
      el.dispatchEvent(new Event("input", { bubbles: true }));
    };
    ui.actions["wallet-edit"]();
    const h0 = textOf("#grid .wallet__hint");
    type("4abc");
    const h1 = textOf("#grid .wallet__hint");
    type(TYPO_ADDRESS);
    const h2 = textOf("#grid .wallet__hint");
    await ui.actions["wallet-save"]();
    const h3 = textOf("#grid .wallet__hint");
    const h3bad = !!hintEl()?.classList.contains("is-bad");
    const fit1 = cardOverflow("wallet");
    type("4abc");
    await ui.actions["wallet-save"]();
    const h4 = textOf("#grid .wallet__hint");
    const fit2 = cardOverflow("wallet");
    const flow = { h0, h1, h2, h3, h4 };
    check(
      "wallet_edit_flow",
      h0 === "95 characters · starts with 4 or 8" && h1 === "4 / 95 characters" && h2 === "format is correct" &&
        !!typoMsg && h3 === typoMsg && h3bad && h4.length > 0 && h4 !== "4 / 95 characters" &&
        st.info?.wallet?.is_default === info?.wallet?.is_default,
      flow,
    );
    check("wallet_error_fits", fit1.length === 0 && fit2.length === 0, [...fit1, ...fit2]);
    detail.wallet_errors = [h3, h4];
    ui.actions["wallet-cancel"]();
    check("wallet_cancel", !!$q('#grid [data-mod="wallet"] .wallet__value') && !input());
    // The logs legend replaces the caption: each tag word in its tape colour and bold, then what it carries.
    {
      const legend = $q('#grid [data-mod="tape"] .joined__left > .card__base');
      const cells = legend ? [...legend.children] : [];
      const rows = [];
      for (let i = 0; i + 1 < cells.length; i += 2) rows.push([cells[i], cells[i + 1]]);
      // The colour a tape line's tag word gets, from a probe inside the tape itself.
      const tapeEl = $q('#grid [data-mod="tape"] .tape');
      const tapeColour = (tag) => {
        const probe = document.createElement("span");
        probe.className = `tag--${tag}`;
        tapeEl.appendChild(probe);
        const c = getComputedStyle(probe).color;
        probe.remove();
        return c;
      };
      const got = rows.map(([t, w]) => {
        const ts = getComputedStyle(t);
        return {
          tag: t.textContent, what: w.textContent, cls: t.className, weight: Number(ts.fontWeight),
          colour: ts.color, lineTop: Math.round(t.getBoundingClientRect().top), refColour: tapeColour(t.textContent),
          muted: getComputedStyle(w).color,
        };
      });
      const tops = [...new Set(got.map((g) => g.lineTop))];
      check(
        "logs_legend",
        rows.length === 4 && cells.length === 8 &&
          JSON.stringify(got.map((g) => [g.tag, g.what])) === JSON.stringify(TAPE_LEGEND) &&
          got.every((g) => g.cls === `tag--${g.tag}` && g.weight >= 600 && g.refColour === g.colour && g.colour !== g.muted) &&
          tops.length === 4 && !$q('#grid [data-mod="tape"] .card__cap'),
        got,
      );
    }
  } catch (err) {
    failed.push({ wallet: String(err) });
    checks.wallet_edit_flow = false;
  }

  // Captured before the try, so a failure mid-cycle still puts the user's period back.
  const p0 = st.settings?.period;
  try {
    // 7. Worker: the backend's refusal shows inline, verbatim; the saved name is untouched.
    const savedWorker = st.settings.worker;
    let backendMsg = null;
    try {
      await window.__TAURI__.core.invoke("set_worker", { worker: "bad name!" });
    } catch (err) {
      backendMsg = errMsg(err);
    }
    await ui.commitWorker("bad name!");
    const err = textOf('#grid [data-mod="worker"] .card__err');
    const fit = cardOverflow("worker");
    await ui.commitWorker(savedWorker);
    const cleared = !$q('#grid [data-mod="worker"] .card__err') && $q('#grid [data-field="worker"]')?.value === savedWorker;
    check("worker_error_inline", !!backendMsg && err === backendMsg && cleared && st.settings.worker === savedWorker, { err, backendMsg, cleared });
    check("worker_error_fits", fit.length === 0, fit);

    // 8. The shared period pill cycles day → week → month → day, on both cards, and saves.
    const order = ["day", "week", "month"];
    const seen = [];
    let synced = true;
    for (let i = 0; i < 3; i++) {
      await ui.actions.period();
      seen.push(textOf('#grid [data-mod="earned"] .period'));
      synced = synced && textOf('#grid [data-mod="cost"] .period') === seen[i];
    }
    const expect = [1, 2, 3].map((k) => `per ${order[(order.indexOf(p0) + k) % 3]} ▾`);
    const stored = await window.__TAURI__.core.invoke("get_settings");
    check("period_cycles", synced && JSON.stringify(seen) === JSON.stringify(expect) && stored.period === p0, { seen, expect });

    // 9. Threads: "{n} of {max}", 36px round stepper buttons.
    const max = info?.max_threads ?? 8;
    const btn = $q('#grid [data-mod="threads"] .stepper__btn')?.getBoundingClientRect();
    check(
      "threads_card",
      textOf('#grid [data-mod="threads"] .stepper__value') === `${st.settings.threads} of ${max}` &&
        !!btn && Math.round(btn.width) === 36 && Math.round(btn.height) === 36,
    );

    // 10. The live light's click map (pure; never invoked here).
    const lc = ui.liveCommand;
    check(
      "live_light_mapping",
      lc("stopped", false) === "start_mining" && lc("error", false) === "start_mining" &&
        lc("mining", false) === "pause_mining" && lc("paused", false) === "resume_mining" &&
        lc("starting", false) === null && lc("mining", true) === "stop_mining" &&
        lc("paused", true) === "stop_mining" && lc("stopped", true) === null,
    );

    // 10a. The trials card's rows from `list_trials` (pure): best first by the chosen h/w; a noisy
    // trial is labelled and never crowned best.
    {
      const t = { count: 3, rows: [
        { label: "8 threads · full power", hw_sys: 143, hw_cpu: 274, noisy: false, t_end: 1, threads: 8, e_core_clamp: false, low_power: false },
        { label: "8 threads · low power mode", hw_sys: 220, hw_cpu: 600, noisy: false, t_end: 2, threads: 8, e_core_clamp: false, low_power: true },
        { label: "5 threads · low power mode", hw_sys: 230, hw_cpu: 479, noisy: true, t_end: 3, threads: 5, e_core_clamp: false, low_power: true },
      ] };
      const sys = trialsView(t, "sys");
      const cpu = trialsView(t, "cpu");
      check("trials_view_from_ledger",
        sys.value === "3" && sys.rows[0].hw === "230" && sys.rows[0].label === "5 threads · low power mode" && sys.rows[0].best && !sys.rows[1].best &&
        sys.rows[0].threads === 5 && sys.rows[0].low_power === true &&
        cpu.rows[0].label === "8 threads · low power mode" && cpu.rows[0].best &&
        trialsView(null, "sys").rows.length === 0,
        { sys, cpu });
    }
    {
      const six = { threads: 6, e_core_clamp: false };
      const sweepOpt = { mode: "hold", profile: "max", sweep: { testing: six, stage: "measuring", done: 2, planned: 12, elapsed_s: 90, left_s: 390, trial_s: 480 } };
      const o = trialsProgressView(sweepOpt, {}, "mining", null, []);
      const paused = trialsProgressView(sweepOpt, {}, "paused", null, []);
      const a = trialsProgressView({ mode: "hold", profile: "max", sweep: { testing: null, stage: null, done: 12, planned: 12 }, ask_low_power: true }, {}, "starting", null, []);
      // A stale sweep/ask survives a stop (the engine no longer runs): the not-running branch
      // reads `progress` instead, never the snapshot's own leftover sweep/ask.
      const stale = trialsProgressView({ mode: "hold", sweep: { testing: six, stage: "measuring", done: 2, planned: 12 }, ask_low_power: true }, {}, "stopped", [12, 12], []);
      const mid = trialsProgressView(null, {}, "stopped", [4, 12], []);
      const ready = trialsProgressView(null, {}, "stopped", [0, 12], []);
      const readyNull = trialsProgressView(null, {}, "stopped", null, []);
      const log = [
        { ts: "12:00:00", tag: "trial", msg: "started 6 threads" },
        { ts: "12:00:01", tag: "miner", msg: "not a trial line" },
      ];
      const withLog = trialsProgressView({ mode: "hold", sweep: { testing: six, stage: "settling", done: 0, planned: 12 } }, {}, "mining", null, log);
      check("trials_progress_view_states",
        o.value === "trial 3 of 12" && o.sub === "6 cores - 06:30 mins left" &&
        JSON.stringify(o.buttons) === JSON.stringify([{ act: "trials-pause", label: "pause" }, { act: "trials-stop", label: "stop" }]) && o.button === "pause" &&
        paused.sub === "paused" &&
        JSON.stringify(paused.buttons) === JSON.stringify([{ act: "trials-resume", label: "continue" }, { act: "trials-stop", label: "stop" }]) &&
        a.value === "low power mode" && a.sub === "turn it on in battery settings" &&
        JSON.stringify(a.buttons) === JSON.stringify([{ act: "trials-skip", label: "skip" }, { act: "trials-stop", label: "stop" }]) && a.button === "skip" &&
        /^trial \d+ of \d+$/.test(stale.value) && stale.buttons[0].act === "trials-go" && stale.button === "run trials again" &&
        mid.value === "trial 5 of 12" && mid.sub === "4 of 12 trials done" && mid.button === "continue trials" &&
        /^trial 1( of \d+)?$/.test(ready.value) && ready.sub === "tests each setup on this mac" && ready.button === "start trials" &&
        /^trial 1( of \d+)?$/.test(readyNull.value) && readyNull.button === "start trials" &&
        withLog.log.length === 1 && withLog.log[0].msg === "started 6 threads",
        { o, paused, a, stale, mid, ready, readyNull, withLog });
    }
    {
      // Owner, 2026-10-04: while a profile holds, the first caption already names what runs, so the
      // second is empty (a "holding: <setup>" line repeated it); manual says so.
      const picks = { efficient: { setup: { threads: 5, e_core_clamp: false }, low_power: true, hw: 200.4, hs: 1825 } };
      const held = profileView({ mode: "hold", profile: "efficient", held: { setup: { threads: 5, e_core_clamp: false }, low_power: true, hw: 200.4, hs: 1825 } }, {}, picks, "sys", true);
      const manual = profileView({ mode: "manual" }, {}, {}, "sys", true);
      check("profile_view_holding_caption",
        held.sub2 === "" && held.sub === "5 cores · low power mode" && manual.sub2 === "manual · your own cores",
        { held, manual });
    }
    {
      const hs = [279, 728, 1095, 1363, 1825, 1830, 1743, 1858];
      const c = curveView({ low_power: true, points: hs.map((v, i) => ({ threads: i + 1, hs: v })), standout: [5, 462, 0.208], flat_from: 5 }, 8, 5);
      check("curve_view_from_trials",
        c.cap === "best h/s added per watt · low power mode" && c.foot === "the 5th core added +462 h/s for +0.21 w" &&
        c.footMuted === "cores 6–8 add less than 2%" && c.bars[4].best && c.bars[4].hs === "1,825" && c.bars[7].h === 160 &&
        curveView(null, 8, 5).bars === null,
        c);
    }

    // 11. First-run notice (doctrine §1): what the pool sees, always; the default wallet while it
    // is in use. Shown exactly when due, word for word, and at the designed 970px height the whole
    // grid still shows without scrolling.
    const notice = document.getElementById("notice");
    const due = !!st.info && !st.ui.noticeDismissed && !st.ui.startedThisSession;
    const lineText = (id) => document.getElementById(id).textContent.replace(/\s+/g, " ").trim();
    const poolWords = "Your pool sees your wallet address, the worker name you typed and your hashrate. Tor is on, so it sees a Tor exit, not your IP address. Nothing else leaves this Mac.";
    const walletWords = "Mining pays the default wallet (GhostDrip dev wallet) until you add yours: use yours → on the wallet card.";
    const walletLineShown = !document.getElementById("notice-wallet").hidden;
    check(
      "notice_state",
      notice.hidden === !due && lineText("notice-pool") === poolWords && lineText("notice-wallet") === walletWords &&
        walletLineShown === (st.info?.wallet?.is_default === true),
      { hidden: notice.hidden, due, pool: lineText("notice-pool"), wallet: lineText("notice-wallet"), walletLineShown },
    );
    ui.setPool("mo");
    ui.setTab("mining");
    // The tallest case: both lines.
    ui.setNoticeVisible(true);
    document.getElementById("notice-wallet").hidden = false;
    const gridEl = document.getElementById("grid");
    const scrollH = document.documentElement.scrollHeight;
    const lines = [...notice.querySelectorAll(".notice__line")].map((l) => l.scrollWidth <= l.clientWidth + 1 && l.getBoundingClientRect().width <= 896);
    const tall = window.innerHeight >= 970;
    const fitsInfo = { scrollH, innerH: window.innerHeight, gridScroll: gridEl.scrollHeight, gridClient: gridEl.clientHeight, lines };
    const fits = scrollH <= window.innerHeight && lines.every(Boolean) && (!tall || gridEl.scrollHeight <= gridEl.clientHeight + 1);
    ui.setNoticeVisible(due);
    check("notice_fits", fits, fitsInfo);

    // 12. Dismissing it is saved in settings.json (M12), not in the webview's storage; then put back.
    if (due) {
      const s0 = { ...st.settings };
      document.getElementById("notice-dismiss").click();
      await settle(80);
      const stored = await window.__TAURI__.core.invoke("get_settings");
      const hidden = notice.hidden;
      st.ui.noticeDismissed = false;
      st.settings = s0;
      await window.__TAURI__.core.invoke("save_settings", { settings: s0 });
      ui.refresh();
      check("notice_dismissal_saved", stored.notice_dismissed === true && hidden && !notice.hidden, { stored: stored.notice_dismissed, hidden });
    } else {
      check("notice_dismissal_saved", st.settings?.notice_dismissed === true || st.ui.startedThisSession, { note: "not due; dismissal state came from settings" });
    }
  } catch (err) {
    failed.push({ controls: String(err) });
    checks.period_cycles = false;
  } finally {
    if (p0 !== undefined && st.settings && st.settings.period !== p0) {
      st.settings = { ...st.settings, period: p0 };
      try {
        await window.__TAURI__.core.invoke("save_settings", { settings: st.settings });
      } catch {
        // reported through period_cycles already
      }
    }
    st.ui.startedThisSession = saved.started;
    ui.setPool(saved.pool);
    ui.setTab(saved.tab);
    ui.refresh();
  }
  if (failed.length) detail.mining = failed;
}

const PAUSE_TIP = "keeps the 2 GB dataset in memory · resume is immediate";
const STOP_TIP = "stops xmrig and releases its memory";
const settle = (ms = 30) => new Promise((r) => setTimeout(r, ms));

/** The hashrate card's run pills as the user sees them. */
function readPills() {
  const a = $q('#grid [data-mod="hashrate"] [data-act="run-primary"]');
  const b = $q('#grid [data-mod="hashrate"] [data-act="run-stop"]');
  return {
    primary: a ? { label: a.textContent.trim(), disabled: a.disabled, title: a.title } : null,
    stop: b ? { label: b.textContent.trim(), disabled: b.disabled, title: b.title } : null,
  };
}

/** Snapshot the state every synthetic check touches, and put it back. */
function saveState(st) {
  return {
    snap: st.snap, series: st.series.slice(), powerSeries: st.powerSeries.slice(), context: st.context,
    tape: st.tape.slice(), lastError: st.lastError, busy: st.busy, tab: st.tab, pool: st.pool,
  };
}

function restoreState(ui, saved) {
  const st = ui.state;
  st.snap = saved.snap;
  st.series = saved.series;
  st.powerSeries = saved.powerSeries;
  st.context = saved.context;
  st.tape = saved.tape;
  st.lastError = saved.lastError;
  st.busy = saved.busy;
  ui.setPool(saved.pool);
  ui.setTab(saved.tab);
  ui.refresh();
}

/**
 * Run pills (ruling P2-R8): for synthetic stopped / mining / paused / starting
 * snapshots, and a command in flight, the labels, tooltips and disabled states
 * are right, and the pills fit the title row at any allowed window width (960 px and up).
 * Never clicks them: nothing here can start mining.
 */
function runPillChecks(ui, checks, detail) {
  const st = ui.state;
  const saved = saveState(st);
  const seen = {};
  try {
    ui.setPool("mo");
    ui.setTab("mining");
    const want = {
      stopped: { p: "start", pd: false, s: true },
      mining: { p: "pause", pd: false, s: false },
      paused: { p: "resume", pd: false, s: false },
      starting: { p: "start", pd: true, s: false },
      error: { p: "start", pd: false, s: true },
    };
    let ok = true;
    for (const run of Object.keys(want)) {
      ui.applySnapshot(synthSnap({ state: run }));
      const got = readPills();
      seen[run] = got;
      const w = want[run];
      ok = ok && !!got.primary && !!got.stop &&
        got.primary.label === w.p && got.primary.disabled === w.pd &&
        got.stop.label === "stop" && got.stop.disabled === w.s &&
        got.stop.title === STOP_TIP &&
        got.primary.title === (run === "mining" ? PAUSE_TIP : "");
    }
    checks.run_pills_states = ok;

    // Keyboard focus on a pill survives the next snapshot rebuilding the card.
    ui.applySnapshot(synthSnap({ state: "mining", hs10: 1800 }));
    $q('#grid [data-mod="hashrate"] [data-act="run-stop"]').focus();
    ui.applySnapshot(synthSnap({ state: "mining", hs10: 1900 }));
    const focused = document.activeElement;
    checks.run_pills_keep_focus = !!focused && focused.dataset?.act === "run-stop" && cardVal("hashrate") === "1,900";
    focused?.blur();

    // Keyboard focus on a pill that turns disabled while a command is in flight
    // comes back when it is enabled again; after a stop it lands on start, never on stop.
    ui.applySnapshot(synthSnap({ state: "mining" }));
    $q('#grid [data-mod="hashrate"] [data-act="run-primary"]').focus();
    st.busy = true;
    ui.render();
    const lostWhileBusy = document.activeElement === document.body || !document.activeElement;
    st.busy = false;
    ui.render();
    const back = document.activeElement?.dataset?.act;
    $q('#grid [data-mod="hashrate"] [data-act="run-stop"]').focus();
    st.busy = true;
    ui.render();
    ui.applySnapshot(synthSnap({ state: "stopped" }));
    st.busy = false;
    ui.render();
    const afterStop = document.activeElement?.dataset?.act;
    document.activeElement?.blur();
    seen.focus = { lostWhileBusy, back, afterStop };
    checks.run_pills_focus_after_busy = lostWhileBusy && back === "run-primary" && afterStop === "run-primary";

    // A command in flight disables both.
    ui.applySnapshot(synthSnap({ state: "mining" }));
    st.busy = true;
    ui.render();
    const busy = readPills();
    st.busy = false;
    ui.render();
    seen.busy = busy;
    checks.run_pills_busy = !!busy.primary && busy.primary.disabled && !!busy.stop && busy.stop.disabled;

    // Profile layout: stacked in the right column, clear of the left column, inside the card, at the fixed window width.
    let fitOk = true;
    let mineOk = true;
    const fit = {};
    const mine = {};
    for (const run of ["stopped", "mining", "paused", "error"]) {
      ui.applySnapshot(synthSnap(run === "error" ? { state: "error", error: "port 3620 is busy (is another xmrig still running?)" } : { state: run }));
      const q = (sel) => $q(`#grid [data-mod="hashrate"] ${sel}`);
      const pick = q(".run__col > img");
      const volt = q(".joined__left > img");
      const p = pick?.getBoundingClientRect();
      const v = volt?.getBoundingClientRect();
      const stop = q('[data-act="run-stop"]').getBoundingClientRect();
      const primary = q('[data-act="run-primary"]').getBoundingClientRect();
      const capEl = q(".spark__cap").getBoundingClientRect();
      const r = {
        src: pick?.getAttribute("src"),
        size: p && [p.width, p.height],
        dTop: p && v ? +(p.top - v.top).toFixed(2) : null,
        pickToPill: p ? +(primary.top - p.bottom).toFixed(1) : null,
        stopBottom: +stop.bottom.toFixed(1),
        capBottom: +capEl.bottom.toFixed(1),
        pillH: [primary.height, stop.height],
      };
      mine[run] = r;
      mineOk = mineOk && r.src === "assets/emoji/hoh_mine.svg" && !!p && !!v && p.width === v.width && p.height === v.height &&
        Math.abs(r.dTop) <= 0.5 && Math.abs(r.pickToPill - 12) <= 0.5 && stop.bottom <= capEl.bottom + 0.5 &&
        primary.height === 26 && stop.height === 26;
    }
    // "mine" · ⛏️ at the ⚡'s size and height, 12px of air under it, then the 26px pills; the stop
    // pill never sits lower than the hashrate caption.
    checks.mine_icon_and_pills = mineOk;
    if (!mineOk) detail.mine = mine;
    for (const run of ["stopped", "mining", "paused"]) {
      ui.applySnapshot(synthSnap({ state: run }));
      const card = $q('#grid [data-mod="hashrate"]').getBoundingClientRect();
      const head = $q('#grid [data-mod="hashrate"] .spark__top');
      const t = head.querySelector(".joined__left").getBoundingClientRect();
      const below = [...document.querySelectorAll("#grid .card")].map((c) => c.getBoundingClientRect()).filter((b) => b.top >= card.bottom - 1 && b.width < card.width / 2 + 1);
      const centreOver = (x) => below.some((b) => Math.abs((b.left + b.right) / 2 - x) <= 1.5);
      const pillEls = [...head.querySelectorAll(".choice__pill")];
      const pills = pillEls.map((b) => b.getBoundingClientRect());
      const bad = cardOverflow("hashrate");
      // Inner padding: pill edge → emoji on the left, label's text end → pill edge on the right.
      const pads = pillEls.map((b, i) => {
        const em = b.querySelector("img").getBoundingClientRect();
        const label = b.querySelector(".run__face > span");
        const range = document.createRange();
        range.selectNodeContents(label);
        const tr = range.getBoundingClientRect();
        return [+(em.left - pills[i].left).toFixed(2), +(pills[i].right - tr.right).toFixed(2)];
      });
      // The widest label sets the width: "resume" fills its pill with the same padding both sides.
      const mineTitle = head.querySelector(".run__col > .card__title").getBoundingClientRect();
      const mineX = (mineTitle.left + mineTitle.right) / 2;
      const r = {
        win: window.innerWidth,
        widths: pills.map((p) => +p.width.toFixed(2)),
        pads,
        mineOff: pills.map((p) => +((p.left + p.right) / 2 - mineX).toFixed(2)),
        gapToTitle: Math.round(t.left - pills[0].right),
        centred: centreOver((pills[0].left + pills[0].right) / 2) && centreOver((t.left + t.right) / 2),
        rightInset: Math.round(card.right - pills[pills.length - 1].right),
        headClip: head.scrollWidth > head.clientWidth + 1,
        bad,
      };
      fit[run] = r;
      fitOk = fitOk && r.win >= 960 && Math.abs(r.widths[0] - r.widths[1]) <= 0.5 && r.gapToTitle >= 8 &&
        r.pads.every(([l, rr]) => Math.abs(l - rr) <= 1) && r.mineOff.every((o) => Math.abs(o) <= 1) &&
        r.rightInset >= 15 && r.centred && !r.headClip && bad.length === 0 &&
        Math.abs(pills[0].left - pills[1].left) < 1 && pills[1].top >= pills[0].bottom;
    }
    checks.run_pills_fit = fitOk;
    {
      const q = (sel) => $q(`#grid [data-mod="hashrate"] ${sel}`).getBoundingClientRect();
      const card = $q('#grid [data-mod="hashrate"]');
      detail.hashrate_space = {
        topToChart: Math.round(q(".chart").top - q(".spark__top").bottom),
        capToChart: Math.round(q(".chart").top - q(".spark__cap").bottom),
        overflow: card.scrollHeight - card.clientHeight,
      };
    }
    if (!ok || !checks.run_pills_busy || !fitOk || !checks.run_pills_focus_after_busy) detail.run_pills = { seen, fit };
  } catch (err) {
    checks.run_pills_states = false;
    detail.run_pills = { error: String(err), seen };
  } finally {
    restoreState(ui, saved);
  }
}

/** A path's y coordinates (the "M x y" / "L x y" pairs drawCharts writes). */
function pathYs(el) {
  const nums = (el?.getAttribute("d") || "").replace(/[MLZ]/g, " ").trim().split(/\s+/).map(Number);
  return nums.filter((_, i) => i % 2 === 1);
}

/**
 * The 2×2 anatomy (Helios, 2026-09-24: "noticeably jumps when I switch from mining to
 * machine"): hashrate (mining) and power (machine) share one anatomy. Measured from each
 * card's top, the hashrate column and the power column put their title, emoji, value,
 * caption, chart top and chart bottom at the same y (±1px); every column in both top halves
 * shares the title and emoji line; each column centres over the 1×1 card below it; the
 * top-half caption sits on the row-1 1×1 caption line (h/w, chip / cpu), in the same type;
 * both charts end on the card's inner bottom, the row-2 1×1 caption line. The power legend
 * floats inside the chart's top-left corner, above both lines. The logs window shows
 * TAPE_LINES lines with even gaps above and below. Every mining-tab card wears the owner's icon.
 */
async function heroChecks(ui, checks, detail) {
  const st = ui.state;
  const prev = { tab: st.tab, pool: st.pool };
  const d = {};
  const measure = (id, colSel, refIds) => {
    const card = $q(`#grid [data-mod="${id}"]`);
    const cr = card.getBoundingClientRect();
    const y = (el, edge = "top") => +(el.getBoundingClientRect()[edge] - cr.top).toFixed(2);
    const col = card.querySelector(colSel);
    const cols = [...card.querySelectorAll(":scope > .hero > .hero__col")];
    const cap = col.querySelector(":scope > .hero__cap");
    const cs = getComputedStyle(cap);
    const svg = card.querySelector("svg.chart");
    // The 1×1 cards in the row under the 2×2.
    const below = [...document.querySelectorAll("#grid .card")].map((c) => c.getBoundingClientRect())
      .filter((b) => Math.abs(b.top - (cr.bottom + 16)) <= 1 && b.left < cr.right);
    const colX = cols.map((c) => { const r = c.getBoundingClientRect(); return (r.left + r.right) / 2; });
    const refs = refIds.map((r) => {
      const el = $q(`#grid [data-mod="${r}"]`);
      const base = el.querySelector(":scope > .card__base:not(.card__caps), :scope > .card__caps > .card__cap:last-child");
      const top = el.getBoundingClientRect().top;
      return { id: r, capBottom: +(base.getBoundingClientRect().bottom - top).toFixed(2), font: getComputedStyle(base).fontSize, lh: getComputedStyle(base).lineHeight };
    });
    const row2 = [...document.querySelectorAll("#grid .card")].filter((c) => is1x1(c) && Math.abs(c.getBoundingClientRect().top - (cr.top + 176)) <= 1)
      .map((c) => +(c.querySelector(":scope > .card__base, :scope > .card__caps")?.getBoundingClientRect().bottom - cr.top).toFixed(2));
    return {
      title: y(col.querySelector(":scope > .card__title")),
      icon: y(col.querySelector(":scope > img")),
      iconSize: col.querySelector(":scope > img").getBoundingClientRect().width,
      value: y(col.querySelector(":scope > .hero__value")),
      valueFont: getComputedStyle(col.querySelector(":scope > .hero__value")).fontSize,
      caption: y(cap, "bottom"),
      capFont: cs.fontSize, capLh: cs.lineHeight,
      chartTop: y(svg), chartBottom: y(svg, "bottom"),
      innerBottom: +(cr.height - parseFloat(getComputedStyle(card).paddingBottom)).toFixed(2),
      colTitles: cols.map((c) => y(c.querySelector(":scope > .card__title"))),
      colIcons: cols.map((c) => y(c.querySelector(":scope > img"))),
      colCentred: colX.every((x) => below.some((b) => Math.abs((b.left + b.right) / 2 - x) <= 1)),
      refs, row2,
    };
  };
  let heroOk = false;
  let capOk = false;
  let chartOk = false;
  let legendOk = false;
  let tapeOk = false;
  let iconOk = false;
  let machineIconOk = false;
  let tuningIconOk = false;
  let themeIconOk = false;
  try {
    ui.setPool("mo");
    ui.setTab("mining");
    const hr = measure("hashrate", ":scope > .hero > .joined__left", ["hw", "threads"]);
    ui.setTab("machine");
    const pw = measure("power", ":scope > .hero > .hero__col", ["chip", "cpu"]);
    d.mining = hr;
    d.machine = pw;
    const near = (a, b) => Math.abs(a - b) <= 1;
    const keys = ["title", "icon", "value", "caption", "chartTop", "chartBottom"];
    heroOk = keys.every((k) => near(hr[k], pw[k])) && hr.valueFont === pw.valueFont && hr.iconSize === pw.iconSize &&
      [hr, pw].every((m) => m.colTitles.every((t) => near(t, m.title)) && m.colIcons.every((t) => near(t, m.icon)) && m.colCentred);
    capOk = [hr, pw].every((m) => m.refs.every((r) => near(r.capBottom, m.caption)) &&
      m.refs.some((r) => r.font === m.capFont && r.lh === m.capLh));
    chartOk = [hr, pw].every((m) => near(m.chartBottom, m.innerBottom) && m.row2.length > 0 && m.row2.every((b) => near(b, m.chartBottom))) &&
      hr.chartBottom - hr.chartTop === CHART_H;

    // The power legend, with real-shaped lines under it (the mockup's figures).
    const grid = document.getElementById("grid");
    const vm = mockupView(buildView(st), false);
    vm.tape = Array.from({ length: TAPE_LINES + 2 }, (_, i) => ({ ts: `14:0${i}:00`, tag: ["cpu", "net", "miner", "error"][i % 4], msg: `line ${i + 1} · accepted (9${i}/0) diff 45401` }));
    renderGrid(grid, modulesFor("machine", "mo"), vm);
    {
      const svg = $q('#grid svg[data-chart="power"]');
      const sr = svg.getBoundingClientRect();
      const lg = $q('#grid [data-mod="power"] .legend').getBoundingClientRect();
      const ys = [...svg.querySelectorAll("path.chart__line, path.chart__temp")].flatMap(pathYs);
      d.legend = { dx: +(lg.left - sr.left).toFixed(1), dy: +(lg.top - sr.top).toFixed(1), bottom: +(lg.bottom - sr.top).toFixed(1), lineTop: Math.min(...ys) };
      legendOk = ys.length > 0 && d.legend.dx >= 0 && d.legend.dx <= 12 && d.legend.dy >= 0 && d.legend.dy <= 6 &&
        d.legend.bottom < d.legend.lineTop && lg.right <= sr.right;
    }
    renderGrid(grid, modulesFor("mining", "mo"), vm);
    {
      const tape = $q('#grid [data-mod="tape"] .tape');
      const tr = tape.getBoundingClientRect();
      const lines = [...tape.querySelectorAll(".tape__line")];
      const first = lines[0].getBoundingClientRect();
      const last = lines[lines.length - 1].getBoundingClientRect();
      d.tape = { lines: lines.length, top: +(first.top - tr.top).toFixed(2), bottom: +(tr.bottom - last.bottom).toFixed(2) };
      tapeOk = lines.length === TAPE_LINES && TAPE_LINES === 7 && Math.abs(d.tape.top - d.tape.bottom) <= 1 && d.tape.bottom >= 0;
    }
    // Every card wears the owner's icon wherever the owner drew one (the rest keep their Fluent
    // emoji: awake's clock, profile's compass; the curve has none). A list entry may name
    // alternatives: the source half shows the outlet or the battery, whichever powers the Mac.
    const OWNER_ICONS = {
      mining: {
        hashrate: ["hoh_mine", "hoh_hashrate"], hw: ["hoh_hashwatt"], threads: ["hoh_cores"], earned: ["hoh_earned"], cost: ["hoh_cost"],
        wallet: ["hoh_wallet"], difficulty: ["hoh_difficulty"], uptime: ["hoh_uptime"], worker: ["hoh_worker"], tape: ["hoh_logs"], shares: ["hoh_shares"],
      },
      machine: {
        power: ["hoh_power", ["hoh_source_outlet", "hoh_source_battery"]], chip: ["hoh_chip"], cpu: ["hoh_cpu"], temperature: ["hoh_temp"],
        thermal: ["hoh_thermal"], memory: ["hoh_memory"], awake: ["alarm_clock"], lowpower: ["hoh_low_power"],
      },
      tuning: {
        profile: ["compass", "hoh_profile_quiet", "hoh_profile_balanced", "hoh_profile_max"], optimizer: ["hoh_seeker"],
        curve: [], trials: ["hoh_trials"], lowpower: ["hoh_low_power"], measure: ["hoh_power"],
      },
    };
    const src = (n) => `assets/emoji/${n}.svg`;
    const iconOkFor = {};
    const owned = new Set();
    d.icons = {};
    // Pinned: the live watch reports this Mac's real Low Power Mode, which adds the seeker's icon.
    const lpWas = ui.lowPowerDetected();
    ui.applyLowPower(false);
    for (const [tab, want] of Object.entries(OWNER_ICONS)) {
      ui.setTab(tab);
      const icons = {};
      document.querySelectorAll("#grid .card").forEach((c) => {
        icons[c.dataset.mod] = [...c.querySelectorAll("img")].filter((i) => !i.closest(".run__pill")).map((i) => i.getAttribute("src"));
      });
      Object.values(want).flat(2).filter((n) => n.startsWith("hoh_")).forEach((n) => owned.add(n));
      const match = (got, names) => !!got && got.length === names.length && names.every((n, i) => (Array.isArray(n) ? n.map(src) : [src(n)]).includes(got[i]));
      iconOkFor[tab] = JSON.stringify(Object.keys(icons).sort()) === JSON.stringify(Object.keys(want).sort()) &&
        Object.entries(want).every(([id, names]) => match(icons[id], names));
      d.icons[tab] = icons;
    }
    // The theme toggle: the owner's light and dark mode icons; the low power guide: the owner's lever icon.
    const themeIcons = [...document.querySelectorAll("#theme-btn img")].map((i) => i.getAttribute("src"));
    const guideIcon = $q("#guide .sheet__head img")?.getAttribute("src");
    ["hoh_mode_light", "hoh_mode_dark"].forEach((n) => owned.add(n));
    const loads = await Promise.all([...owned].map((n) => loadEmoji(src(n))));
    ui.setTab("mining");
    Object.assign(d.icons, { themeIcons, guideIcon, loads: loads.every(Boolean), coresTitle: textOf('#grid [data-mod="threads"] .card__title') });
    const ownedOk = [...owned].every((n) => EMOJI[n] === n) && loads.every(Boolean);
    iconOk = iconOkFor.mining && ownedOk && d.icons.coresTitle === "cores";
    machineIconOk = iconOkFor.machine && ownedOk;
    tuningIconOk = iconOkFor.tuning && ownedOk;
    ui.applyLowPower(lpWas);
    themeIconOk = JSON.stringify(themeIcons) === JSON.stringify([src("hoh_mode_light"), src("hoh_mode_dark")]) && guideIcon === src("hoh_low_power") && ownedOk;
  } catch (err) {
    d.error = String(err);
  } finally {
    ui.setPool(prev.pool);
    ui.setTab(prev.tab);
    ui.refresh();
  }
  checks.hero_anatomy_shared = heroOk;
  checks.hero_caption_on_row1_line = capOk;
  checks.hero_charts_bottom_line = chartOk;
  checks.power_legend_overlay = legendOk;
  checks.tape_even_gaps = tapeOk;
  checks.mining_owner_icons = iconOk;
  checks.machine_owner_icons = machineIconOk;
  checks.tuning_owner_icons = tuningIconOk;
  checks.theme_owner_icons = themeIconOk;
  if (!(heroOk && capOk && chartOk && legendOk && tapeOk && iconOk && machineIconOk && tuningIconOk && themeIconOk)) detail.hero = d;
}

/**
 * Corner pills (Helios, 2026-09-24): wallet and worker carry a pencil, low power an
 * arrow. Each is a real round <button> with its aria-label, at the card's top right,
 * clear of the title, which stays centred. Each opens what the card's own control
 * does: the wallet's edit mode, the worker's focused field, the low power guide (or
 * the walkthrough Plan 5 hooks in). Nothing is saved.
 */
function cornerPillChecks(ui, checks, detail) {
  const st = ui.state;
  const prev = { tab: st.tab, pool: st.pool };
  const d = { shape: {} };
  let shapeOk = true;
  let actOk = false;
  let realLowPowerHook;
  const want = [
    ["mining", "wallet", "wallet-edit", "edit wallet"],
    ["mining", "worker", "worker-edit", "edit worker"],
    ["machine", "lowpower", "guide", "about low power mode"],
    ["tuning", "lowpower", "guide", "about low power mode"],
    ["mining", "threads", "ecores-toggle", "e-cores only"],
  ];
  try {
    ui.setPool("mo");
    for (const [tab, id, act, label] of want) {
      ui.setTab(tab);
      const card = $q(`#grid [data-mod="${id}"]`);
      const pills = card.querySelectorAll(":scope > .card__pill");
      const pill = pills[0];
      const cr = card.getBoundingClientRect();
      const pr = pill?.getBoundingClientRect();
      // A 2×1 (machine low power) centres its title in its 1×1-aligned left column.
      const host = card.querySelector(":scope > .joined > .joined__left") || card;
      const tr = host.querySelector(":scope > .card__title").getBoundingClientRect();
      const hr = host.getBoundingClientRect();
      const r = pill && {
        tag: pill.tagName, type: pill.type, act: pill.dataset.act, label: pill.getAttribute("aria-label"),
        size: [pr.width, pr.height], radius: parseFloat(getComputedStyle(pill).borderTopLeftRadius),
        inset: [+(cr.right - pr.right).toFixed(1), +(pr.top - cr.top).toFixed(1)],
        clearOfTitle: +(pr.left - tr.right).toFixed(1),
        titleOffCentre: +Math.abs((tr.left + tr.right) / 2 - (hr.left + hr.right) / 2).toFixed(2),
        glyph: !!pill.querySelector("svg path"),
      };
      d.shape[`${tab}/${id}`] = r || "missing";
      shapeOk = shapeOk && pills.length === 1 && !!r && r.tag === "BUTTON" && r.type === "button" && r.act === act && r.label === label &&
        r.size[0] >= 24 && r.size[0] <= 26 && r.size[0] === r.size[1] && r.radius >= r.size[0] / 2 &&
        r.inset[0] >= 15 && r.inset[0] <= 20 && r.inset[1] >= 10 && r.inset[1] <= 16 &&
        r.clearOfTitle >= 8 && r.titleOffCentre <= 1 && (r.glyph || act === "ecores-toggle"); // the cores card's pill is the letter e
    }
    // Only these cards carry one, and none leaves a card's text-fit box.
    ui.setTab("mining");
    const miningPills = [...document.querySelectorAll("#grid .card__pill")].map((b) => b.closest(".card").dataset.mod);
    d.miningPills = miningPills;
    shapeOk = shapeOk && JSON.stringify(miningPills) === JSON.stringify(["threads", "wallet", "worker"]);

    // What they open.
    $q('#grid [data-mod="wallet"] .card__pill').click();
    const walletOpen = st.ui.walletEditing && document.activeElement === $q('#grid [data-field="wallet"]');
    ui.actions["wallet-cancel"]();
    const walletBack = !st.ui.walletEditing && !!$q('#grid [data-mod="wallet"] .card__pill');
    $q('#grid [data-mod="wallet"] .wallet__value').click();
    const valueOpen = st.ui.walletEditing;
    ui.actions["wallet-cancel"]();
    const worker0 = st.settings?.worker;
    $q('#grid [data-mod="worker"] .card__pill').click();
    const w = $q('#grid [data-field="worker"]');
    const workerOpen = document.activeElement === w && w.selectionStart === w.value.length;
    w.blur();
    const workerKept = st.settings?.worker === worker0;
    ui.setTab("machine");
    // The real hook (Plan 5's walkthrough) is registered at init(); save it so the temp stub below
    // can be restored afterward, rather than deleted (the rest of this run, and the owner's app
    // once it resumes, still need the real one wired).
    realLowPowerHook = ui.guideHooks.lowpower;
    $q('#grid [data-mod="lowpower"] .card__pill').click();
    const guideOpen = dialogShown("guide") && textOf("#guide-why") === lowPowerGuideText(st.trials);
    pressEsc();
    let hooked = 0;
    ui.guideHooks.lowpower = () => {
      hooked += 1;
    };
    $q('#grid [data-mod="lowpower"] .card__pill').click();
    ui.guideHooks.lowpower = realLowPowerHook;
    const hookRan = hooked === 1 && document.getElementById("guide").hidden;
    d.acts = { walletOpen, walletBack, valueOpen, workerOpen, workerKept, guideOpen, hookRan };
    actOk = walletOpen && walletBack && valueOpen && workerOpen && workerKept && guideOpen && hookRan;
  } catch (err) {
    d.error = String(err);
    shapeOk = false;
  } finally {
    if (typeof realLowPowerHook === "function") ui.guideHooks.lowpower = realLowPowerHook;
    ui.closeGuide();
    if (st.ui.walletEditing) ui.actions["wallet-cancel"]();
    document.activeElement?.blur?.();
    ui.setPool(prev.pool);
    ui.setTab(prev.tab);
  }
  checks.corner_pills_shape = shapeOk;
  checks.corner_pills_open = actOk;
  if (!shapeOk || !actOk) detail.corner_pills = d;
}

/**
 * Review fixes, all synthetic, never mining:
 *  - p2_stop_alt: on the p2pool view with a (synthetic) running engine, ⌥-click (the hashrate
 *    card's stop pill's own route) reaches stop_mining, and a plain click pauses, as on moneroocean.
 *  - cost_never_infinite: an absurd rate held only in memory shows "—".
 *  - denominator_tips: cpu "busy" and "+54% h/w" say what they count.
 *  - chart_gap_aligned: a missing sample leaves a gap in place.
 */
async function fixChecks(ui, checks, detail) {
  const st = ui.state;
  const saved = saveState(st);
  const d = {};
  try {
    ui.setPool("mo");
    ui.setTab("mining");
    ui.setPool("p2"); // the pool switches only while stopped
    ui.applySnapshot(synthSnap({ state: "mining" }));
    // window.__TAURI__.core is frozen, so the route is read from liveClick's own record.
    // ⌥-click on the "stage 2" light takes the hashrate card's stop pill's own path.
    ui.applySnapshot(synthSnap({ state: "paused" }));
    st.lastRunCommand = null;
    await ui.liveClick({ altKey: true });
    const altRoute = st.lastRunCommand;
    // A plain click on p2 pauses a mining engine, the same as on moneroocean.
    ui.applySnapshot(synthSnap({ state: "mining" }));
    st.lastRunCommand = null;
    await ui.liveClick({ altKey: false });
    const plainRoute = st.lastRunCommand;
    const light = document.body.dataset.run;
    // One outbound set at a time: while mining, the other pool's pill is locked.
    const moLocked = !!document.querySelector('#pools [data-pool="mo"]')?.disabled;
    d.p2_stop = { altRoute, plainRoute, light, moLocked };
    checks.p2_stop_alt = altRoute === "stop_mining" && plainRoute === "pause_mining";
    checks.p2_light_real_state = light === "mining" && moLocked && !document.getElementById("stage-chip");
    ui.applyStopped();
  } catch (err) {
    d.p2_error = String(err);
    checks.p2_stop_alt = false;
  }

  try {
    ui.setPool("mo");
    ui.setTab("mining");
    const settings0 = st.settings;
    st.settings = { ...settings0, rate_per_kwh: 1e308, period: "month" };
    ui.applySnapshot(synthSnap({ sys_w_900: 9.1, power: { cpu_w: 3.8, ram_w: 0.8, sys_w: 9.1, temp_c: 55, p_mhz: 1, e_mhz: 1, p_active: 0.5, e_active: 0.5 } }));
    const cost = cardVal("cost");
    const clean = textIsClean();
    st.settings = settings0;
    ui.render();
    d.cost = cost;
    checks.cost_never_infinite = cost === "—" && clean;

    // Never a hard-coded figure (P5): the tooltip is this Mac's own measured gain, the same on
    // the machine tab's lever and the tuning tab's stat.
    const savedTrialsForTips = st.trials;
    st.trials = { low_power_gain: { sys: { off: 143, on: 220, pct: 53.8 }, cpu: { off: 274, on: 600, pct: 119.2 } } };
    ui.render();
    ui.setTab("machine");
    const cpuTip = $q('#grid [data-mod="cpu"] .card__value')?.title;
    const lpTip = $q('#grid [data-mod="lowpower"] .card__cap')?.title;
    ui.setTab("tuning");
    const lpTip2 = $q('#grid [data-mod="lowpower"] .card__cap')?.title;
    d.tips = { cpuTip, lpTip, lpTip2 };
    checks.denominator_tips =
      cpuTip === "busy cores on the whole mac, not only for xmrig" &&
      /whole mac h\/w .*\+54%/.test(lpTip || "") && /cpu only h\/w .*\+119%/.test(lpTip || "") && lpTip2 === lpTip;
    st.trials = savedTrialsForTips;
    ui.render();

    const full = linePath([5, 6, 7, 8, 9], 0, 10, 396, 118);
    const gap = linePath([5, 6, null, 8, 9], 0, 10, 396, 118);
    const lastX = (p) => p.d.trim().split(/\s+/).slice(-2)[0].replace(/^[ML]/, "");
    const moves = (p) => (p.d.match(/M/g) || []).length;
    d.chart = { full: full?.d, gap: gap?.d };
    checks.chart_gap_aligned = !!full && !!gap && moves(full) === 1 && moves(gap) === 2 && lastX(full) === lastX(gap) && lastX(gap) === "394.0";
  } catch (err) {
    d.error = String(err);
    checks.cost_never_infinite = false;
  } finally {
    restoreState(ui, saved);
  }
  const names = ["p2_stop_alt", "p2_light_real_state", "cost_never_infinite", "denominator_tips", "chart_gap_aligned"];
  if (names.some((n) => !checks[n])) detail.fixes = d;
}

/**
 * Machine tab from synthetic snapshots through ui.applySnapshot: every figure
 * is "—" with power: null, then real-shaped figures render as the brief words
 * them, the context outlives the snapshots that don't carry it, and a Mac on
 * battery still fits. Never mines.
 */
function machineChecks(ui, checks, detail, info) {
  const st = ui.state;
  const saved = saveState(st);
  const val = (id) => textOf(`#grid [data-mod="${id}"] .card__value`);
  const cap = (id) => textOf(`#grid [data-mod="${id}"] .card__cap`);
  const power = () => [...document.querySelectorAll('#grid [data-mod="power"] .power__value')].map((e) => e.textContent.trim());
  const powerCaps = () => [...document.querySelectorAll('#grid [data-mod="power"] .power__cap')].map((e) => e.textContent.trim());
  const sourceIcon = () => $q('#grid [data-mod="power"] .hero__col:last-child > img')?.getAttribute("src");
  const fails = [];
  try {
    ui.setPool("mo");
    ui.setTab("machine");

    // 1. power: null, no context → "—" everywhere a live figure goes; no NaN / Infinity.
    st.context = null;
    st.powerSeries = [];
    ui.applyLowPower(null);
    ui.applySnapshot(synthSnap({ power: null, context: null }));
    const nulls = { power: power(), cpu: val("cpu"), temperature: val("temperature"), memory: val("memory"), thermal: val("thermal"), lowpower: val("lowpower"), sourceIcon: sourceIcon() };
    const nullOk =
      nulls.power[0] === "—" && nulls.power[1] === "—" && powerCaps()[1] === "power source" &&
      ["cpu", "temperature", "memory", "thermal", "lowpower"].every((k) => nulls[k] === "—") &&
      cap("cpu") === "p — · e — mhz" && powerCaps()[0] === "whole mac · cpu — · ram —" &&
      document.querySelectorAll('#grid [data-mod="thermal"] .levels__seg.is-on').length === 0 &&
      !!$q('#grid svg[data-chart="power"] path.chart__base') && textIsClean();
    checks.machine_null_dashes = nullOk;
    if (!nullOk) fails.push({ nulls });

    // 2. Real-shaped figures (two samples, so the chart has a line).
    const pw = { cpu_w: 3.8, ram_w: 0.8, sys_w: 9.1, temp_c: 55, p_mhz: 1923, e_mhz: 2554, p_active: 0.62, e_active: 0.6, mem_used_gb: 12.5, mem_total_gb: 16 };
    const ctx = { low_power_mode: true, on_ac: true, battery_pct: 100, thermal: "nominal" };
    ui.applyLowPower(true);
    ui.applySnapshot(synthSnap({ power: { ...pw, sys_w: 8.9, temp_c: 54 }, context: ctx }));
    ui.applySnapshot(synthSnap({ power: pw, context: null }));
    const m = info?.machine;
    const P = m?.p_cores;
    const E = m?.e_cores;
    const cpuWant = typeof P === "number" && typeof E === "number"
      ? `${Math.max(0, Math.min(P + E, Math.round(0.62 * P + 0.6 * E)))} of ${P + E} busy`
      : "—";
    const want = {
      power: ["9.1 w", "outlet"],
      powerCaps: ["whole mac · cpu 3.8 · ram 0.8", "plugged in"],
      sourceIcon: "assets/emoji/hoh_source_outlet.svg",
      cpu: cpuWant,
      cpuCap: cpuWant === "—" ? "p — · e — mhz" : "p 1,923 · e 2,554 mhz",
      temperature: "131°f",
      memory: "12.5 / 16 gb",
      memoryCap: "dataset uses 2.3 gb",
      thermal: "nominal",
      thermalSegs: 1,
      lowpower: "on",
      lowpowerCap: "run trials to measure",
      awake: "kept awake",
      chip: m ? String(m.chip).toLowerCase() : "—",
    };
    const got = {
      power: power(), powerCaps: powerCaps(), sourceIcon: sourceIcon(), cpu: val("cpu"), cpuCap: cap("cpu"), temperature: val("temperature"),
      memory: val("memory"), memoryCap: cap("memory"), thermal: val("thermal"),
      thermalSegs: document.querySelectorAll('#grid [data-mod="thermal"] .levels__seg.is-on').length,
      lowpower: val("lowpower"), lowpowerCap: cap("lowpower"), awake: val("awake"), chip: val("chip"),
    };
    const chart = {
      line: !!$q('#grid svg[data-chart="power"] path.chart__line'),
      temp: !!$q('#grid svg[data-chart="power"] path.chart__temp'),
      legend: textOf('#grid [data-mod="power"] .legend'),
    };
    const tempBar = $q('#grid [data-mod="temperature"] .bar__fill')?.dataset.w;
    const valuesOk = JSON.stringify(want) === JSON.stringify(got) && chart.line && chart.temp &&
      chart.legend === "━ watts ┅ °f · last 13 min" && Math.abs(Number(tempBar) - ((131 - 90) / 122) * 100) < 0.01 && textIsClean();
    checks.machine_values_render = valuesOk;
    if (!valuesOk) fails.push({ want, got, chart, tempBar });

    // 3. The context rode on the first snapshot only; the second (context: null) kept it.
    checks.machine_context_kept = got.lowpower === "on" && got.power[1] === "outlet";

    // 4. On battery, and a stopped engine: "battery" with the battery icon, "57% left", "normal
    // sleep", everything fits; with no charge reading, "charge unknown"; back on the outlet, the
    // outlet icon and "plugged in" (a desktop Mac has no battery at all).
    ui.applySnapshot(synthSnap({ state: "stopped", power: pw, context: { ...ctx, on_ac: false, battery_pct: 57, thermal: "heavy" } }));
    const batt = { power: power(), caps: powerCaps(), icon: sourceIcon(), awake: val("awake"), segs: document.querySelectorAll('#grid [data-mod="thermal"] .levels__seg.is-on').length };
    const battFit = overflowingCards();
    ui.applySnapshot(synthSnap({ state: "stopped", power: pw, context: { ...ctx, on_ac: false, battery_pct: null } }));
    batt.unknown = powerCaps()[1];
    ui.applySnapshot(synthSnap({ state: "stopped", power: pw, context: { ...ctx, on_ac: true, battery_pct: null } }));
    batt.desktop = [power()[1], powerCaps()[1], sourceIcon()];
    checks.machine_battery = batt.power[1] === "battery" && batt.caps[1] === "57% left" && batt.icon === "assets/emoji/hoh_source_battery.svg" &&
      batt.unknown === "charge unknown" && JSON.stringify(batt.desktop) === JSON.stringify(["outlet", "plugged in", "assets/emoji/hoh_source_outlet.svg"]) &&
      batt.awake === "normal sleep" && batt.segs === 3 && battFit.length === 0;
    if (!checks.machine_battery) fails.push({ batt, battFit });

    // 5. Low power, 2×1: the 1×1 anatomy on the left, the efficient profile's own measured gain
    // on the right (this Mac's own trials, never a hard-coded figure), clear of the corner pill
    // above it; "run trials to measure" before both Low Power Mode states are measured.
    {
      const card = $q('#grid [data-mod="lowpower"]');
      const pill = card.querySelector(":scope > .card__pill").getBoundingClientRect();
      const savedTrials = st.trials;
      st.trials = { low_power_gain: { sys: { off: 143, on: 220, pct: 53.8 }, cpu: { off: 274, on: 600, pct: 119.2 } } };
      ui.render();
      const lever = card.querySelector(".lever");
      const cells = [...lever.children].map((e) => e.textContent.trim());
      const gain = lowPowerGainView(st.trials);
      const wantCells = ["off → on · balanced profile", ...gain.rows.flatMap((g) => [g.what, `${g.off} → ${g.on}`, g.gain])];
      const first = lever.children[1].getBoundingClientRect();
      const lv = { cells, pillBottom: +pill.bottom.toFixed(1), noteTop: +lever.children[0].getBoundingClientRect().top.toFixed(1), rowTop: +first.top.toFixed(1), fit: cardOverflow("lowpower") };
      st.trials = { low_power_gain: {} };
      ui.render();
      const empty = card.querySelector(".lever")?.textContent.trim();
      st.trials = savedTrials;
      ui.render();
      checks.machine_lowpower_lever = JSON.stringify(cells) === JSON.stringify(wantCells) && lv.noteTop >= lv.pillBottom && lv.fit.length === 0 &&
        gain.ready && gain.rows.length === 2 && empty === "not yet measured in both states";
      if (!checks.machine_lowpower_lever) fails.push({ lever: lv, empty });
    }
  } catch (err) {
    fails.push({ error: String(err) });
    checks.machine_values_render = false;
  } finally {
    restoreState(ui, saved);
  }
  if (fails.length) detail.machine = fails;
}

/** Throttle picks on the measured M3 Air, as list_trials sends them (sys and cpu alike here). */
const SYNTH_PICKS = {
  light: { setup: { threads: 4, e_core_clamp: true }, low_power: true, hw: 107.4, hs: 528 },
  efficient: { setup: { threads: 5, e_core_clamp: false }, low_power: true, hw: 200.4, hs: 1825 },
  max: { setup: { threads: 8, e_core_clamp: false }, low_power: false, hw: 143.4, hs: 3029 },
};
const SYNTH_TRIALS = { count: 24, rows: [], curve: {}, picks: { sys: SYNTH_PICKS, cpu: SYNTH_PICKS }, progress: [24, 24] };

/**
 * Tuning tab: the throttle and optimizer cards, the curve's empty state, and the h/w counts toggle
 * saves settings.measure (clicked for real, then clicked back). The throttle pills are disabled
 * with "run trials first" before any trials, and show each pick's h/w and setup once trials exist.
 */
async function tuningChecks(ui, checks, detail) {
  const st = ui.state;
  const prev = { tab: st.tab, pool: st.pool };
  const m0 = st.settings?.measure;
  const info = {};
  try {
    ui.setPool("mo");
    ui.setTab("tuning");
    const seekBtn = $q('#grid [data-mod="optimizer"] [data-act="trials-go"]');
    info.live = {
      profileCard: !$q('#grid [data-mod="profile"]')?.classList.contains("is-disabled"),
      optimizerCard: !$q('#grid [data-mod="optimizer"]')?.classList.contains("is-disabled"),
      seekOn: seekBtn?.disabled === false, // "trials-go" always works: stopped, it starts mining first
      seekLabel: seekBtn?.textContent.trim(),
      title: textOf('#grid [data-mod="optimizer"] .card__title'),
    };
    const l = info.live;
    checks.tuning_seeker_live = l.profileCard && l.optimizerCard && l.seekOn && l.title === "seeker" &&
      ["start trials", "continue trials", "run trials again"].includes(l.seekLabel);

    // The three throttles, in order, each with its own icon: before any trials, disabled, with
    // their 2–3 word blurbs and "run trials first"; with trials, each pick's h/w and setup.
    const grid = document.getElementById("grid");
    const mods = modulesFor("tuning", "mo");
    const sysState = { ...st, settings: { ...st.settings, measure: "sys", optimizer: { ...(st.settings?.optimizer ?? {}), profile: "efficient" } } };
    const choices = () => [...document.querySelectorAll('#grid [data-mod="profile"] .choice')].map((c) => {
      const b = c.querySelector("button");
      const blurb = c.querySelector(".choice__blurb");
      return [b.dataset.val, b.textContent.trim(), blurb.innerText.replace(/\n/g, " | ").trim(), b.querySelector("img").getAttribute("src"), b.disabled, blurb.getAttribute("title") ?? ""];
    });
    renderGrid(grid, mods, buildView({ ...sysState, trials: null }));
    info.before = { choices: choices(), cap: textOf('#grid [data-mod="profile"] .card__cap'), title: textOf('#grid [data-mod="profile"] .card__title'), overflow: cardOverflow("profile") };
    renderGrid(grid, mods, buildView({ ...sysState, trials: SYNTH_TRIALS }));
    info.after = { choices: choices(), value: textOf('#grid [data-mod="profile"] .card__value'), cap: textOf('#grid [data-mod="profile"] .card__cap'), overflow: cardOverflow("profile") };
    const wantBefore = [
      ["light", "quiet", "background e-cores", "assets/emoji/hoh_profile_quiet.svg", true, ""],
      ["efficient", "balanced", "half the cores", "assets/emoji/hoh_profile_balanced.svg", true, ""],
      ["max", "max", "all cores", "assets/emoji/hoh_profile_max.svg", true, ""],
    ];
    const wantAfter = [
      ["light", "quiet", "107 h/w | 4 e-cores", "assets/emoji/hoh_profile_quiet.svg", false, "4 e-cores · low power mode · quiet: mines on the efficiency cores at background priority. Other apps get the cpu first."],
      ["efficient", "balanced", "200 h/w | 5 cores", "assets/emoji/hoh_profile_balanced.svg", false, "5 cores · low power mode · balanced: uses about half of the cores. The other cores stay free for you."],
      ["max", "max", "143 h/w | 8 cores", "assets/emoji/hoh_profile_max.svg", false, "8 cores · full power · max: uses all of the cores for the most hashes."],
    ];
    checks.tuning_profiles =
      JSON.stringify(info.before.choices) === JSON.stringify(wantBefore) && info.before.cap === "run trials first" && info.before.title === "profile" &&
      JSON.stringify(info.after.choices) === JSON.stringify(wantAfter) && info.after.value === "balanced" && info.after.cap === "5 cores · low power mode" &&
      PROFILES.every((p) => p.blurb.split(" ").length >= 2 && p.blurb.split(" ").length <= 4) &&
      info.before.overflow.length === 0 && info.after.overflow.length === 0;
    ui.setTab("tuning");
    const foot = ($q('#grid [data-mod="curve"] .curve__foot')?.innerText ?? "").replace(/\s+/g, " ").trim();
    info.curve = foot;
    // Empty on a fresh data dir; a dev data dir that already holds trials draws the curve instead.
    checks.tuning_curve_empty = foot === "no curve yet each measured core count shows here" || (st.trials?.curve?.points?.length ?? 0) > 0;

    // The trials card, rendered from synthetic Ledger trials (a copy of state, so a real
    // list_trials answer can't race it), then from an empty Ledger: the direct empty state.
    {
      const grid = document.getElementById("grid");
      const mods = modulesFor("tuning", "mo");
      const trials = { count: 4, rows: [
        { label: "8 threads · full power", hw_sys: 143, hw_cpu: 274, noisy: false, t_end: 1 },
        { label: "8 threads · low power mode", hw_sys: 220, hw_cpu: 600, noisy: false, t_end: 2 },
        { label: "5 threads · low power mode", hw_sys: 230, hw_cpu: 479, noisy: true, t_end: 3 },
      ] };
      const sysState = { ...st, settings: { ...st.settings, measure: "sys" } };
      const rowsOf = () => [...document.querySelectorAll('#grid [data-mod="trials"] .trial')].map((r) =>
        [...r.querySelectorAll("span:not(.trial__star)")].map((x) => x.textContent.trim()).concat(r.classList.contains("is-best") ? ["best"] : []));
      renderGrid(grid, mods, buildView({ ...sysState, trials }));
      info.trials = {
        value: textOf('#grid [data-mod="trials"] .card__value'),
        rows: rowsOf(),
        overflow: cardOverflow("trials"),
      };
      // A fixed-width star column, one per row, so the thread counts line up whether or not a
      // row is best. Measured before the empty-state re-render below clears the rows.
      const starWidths = [...document.querySelectorAll('#grid [data-mod="trials"] .trial .trial__star')].map((s) => s.getBoundingClientRect().width);
      info.trials.starCount = starWidths.length;
      info.trials.starWidthsMatch = starWidths.length > 0 && starWidths.every((w) => Math.abs(w - starWidths[0]) < 0.5);
      renderGrid(grid, mods, buildView({ ...sysState, trials: { count: 0, rows: [] } }));
      info.trials.empty = textOf('#grid [data-mod="trials"] .empty');
      info.trials.emptyRows = rowsOf().length;
      const want = [
        // Icons carry threads (cores), Low Power Mode (turtle) and best (star); text is the count.
        ["5 cores", "230 h/w", "best"],
        ["8 cores", "220 h/w"],
        ["8 cores", "143 h/w"],
      ];
      checks.tuning_trials_card =
        info.trials.value === "4" && JSON.stringify(info.trials.rows) === JSON.stringify(want) && info.trials.overflow.length === 0 &&
        info.trials.empty === "no trials yet · each measured setup lands here, with its h/w" && info.trials.emptyRows === 0 &&
        info.trials.starCount === 3 && info.trials.starWidthsMatch;
      ui.setTab("tuning");
    }

    // h/w counts: click the other side, see it saved everywhere, click back.
    const other = m0 === "cpu" ? "sys" : "cpu";
    $q(`#grid [data-mod="measure"] [data-val="${other}"]`).click();
    await settle();
    const stored1 = await window.__TAURI__.core.invoke("get_settings");
    const cardOn = $q('#grid [data-mod="measure"] .seg__btn.is-on')?.dataset.val;
    const sheetOn = $q("#set-measure .seg__btn.is-on")?.dataset.measure;
    ui.setTab("mining");
    const hwCap = textOf('#grid [data-mod="hw"] .card__cap');
    ui.setTab("tuning");
    $q(`#grid [data-mod="measure"] [data-val="${m0}"]`).click();
    await settle();
    const stored2 = await window.__TAURI__.core.invoke("get_settings");
    info.measure = { m0, stored1: stored1.measure, cardOn, sheetOn, hwCap, stored2: stored2.measure };
    checks.tuning_measure_toggle =
      stored1.measure === other && cardOn === other && sheetOn === other &&
      hwCap === HW_CAP[other] &&
      stored2.measure === m0 && st.settings.measure === m0;
  } catch (err) {
    info.error = String(err);
    checks.tuning_measure_toggle = false;
  } finally {
    if (m0 && st.settings && st.settings.measure !== m0) await ui.setMeasure(m0);
    ui.setPool(prev.pool);
    ui.setTab(prev.tab);
  }
  if (!checks.tuning_seeker_live || !checks.tuning_profiles || !checks.tuning_curve_empty || !checks.tuning_trials_card || !checks.tuning_measure_toggle) detail.tuning = info;
}

/**
 * Tuning tab, live, all synthetic and never mining: the throttle pills follow the chosen throttle;
 * the trials half shows the sweep's progress, the keel half the held throttle; a snapshot whose
 * trials_recorded moves reloads the trials (and a steady one doesn't); the Low Power Mode ask opens
 * the guide once through guideHooks; a refused command shows its reason inline (the real
 * `stop_seek`, refused while nothing mines, P4-C19); and no card clips, at the longest strings
 * these states make.
 */
async function tuningLiveChecks(ui, checks, detail) {
  const st = ui.state;
  const saved = {
    snap: st.snap, settings: st.settings, trials: st.trials, trialsRecorded: st.trialsRecorded, series: st.series.slice(),
    powerSeries: st.powerSeries.slice(), tape: st.tape.slice(), context: st.context, mismatch: st.mismatch, lastError: st.lastError,
    lowPowerAsk: st.ui.lowPowerAsk, optError: st.ui.optError, tab: st.tab, pool: st.pool,
    guideHook: ui.guideHooks.lowpower, // the real one (Plan 5's walkthrough), restored below rather than deleted
  };
  const d = {};
  const fits = [];
  const grid = document.getElementById("grid");
  const mods = modulesFor("tuning", "mo");
  const opt = (over) => ({ mode: "hold", profile: "efficient", measure: "sys", target: null, sweep: null, held: null, ask_low_power: null, note: null, trials_recorded: 0, ...over });
  const six = { threads: 6, e_core_clamp: false };
  const held = SYNTH_PICKS.efficient;
  const sweeping = { testing: six, stage: "measuring", done: 2, planned: 12 };
  const viewOf = (snap, over = {}) => buildView({ ...st, trials: SYNTH_TRIALS, snap, ...over, ui: { ...st.ui, ...(over.ui || {}) } });
  const paint = (vm, label) => {
    renderGrid(grid, mods, vm);
    for (const id of ["profile", "optimizer", "curve", "trials"]) fits.push(...cardOverflow(id).map((b) => `${label}:${b}`));
  };
  // P5: the optimizer/keel halves became one trials progress panel (value · caption · button ·
  // a small scrolling log); the held setup moved onto the throttle card's 2nd caption line.
  const progress = () => {
    const card = document.querySelector('#grid [data-mod="optimizer"]');
    const btns = [...(card?.querySelectorAll(".seeker__actions button") ?? [])];
    const capEl = card?.querySelector(".card__cap");
    return {
      value: textOf('#grid [data-mod="optimizer"] .card__value'),
      cap: capEl?.textContent.trim(),
      // The 2×1 seeker clamps its caption; the full text rides in the tooltip.
      full: capEl?.getAttribute("title") || capEl?.textContent.trim(),
      tip: capEl?.getAttribute("title") ?? null,
      buttons: btns.map((b) => ({ act: b.dataset.act, label: b.textContent.trim(), disabled: b.disabled })),
      // The first button, for the common single-button not-running states.
      button: btns[0]?.textContent.trim(),
      act: btns[0]?.dataset.act,
      disabled: btns[0]?.disabled,
      log: [...(card?.querySelectorAll(".tape__line") ?? [])].map((l) => l.textContent.replace(/\s+/g, " ").trim()),
    };
  };
  const holding = () => textOf('#grid [data-mod="profile"] .joined__sub2');
  try {
    ui.setPool("mo");
    ui.setTab("tuning");
    await settle(300); // setTab's own list_trials lands first

    // 1. The pills follow the chosen throttle: from settings while stopped, from the snapshot while running.
    const pills = () => [...document.querySelectorAll('#grid [data-mod="profile"] .choice__pill')]
      .filter((b) => b.classList.contains("is-on") && b.getAttribute("aria-pressed") === "true").map((b) => b.dataset.val);
    const card = () => ({ on: pills(), value: textOf('#grid [data-mod="profile"] .card__value'), cap: textOf('#grid [data-mod="profile"] .card__cap') });
    d.profiles = {};
    for (const id of ["light", "efficient", "max"]) {
      paint(viewOf(null, { settings: { ...st.settings, measure: "sys", optimizer: { profile: id } } }), `profile-${id}`);
      d.profiles[id] = card();
    }
    paint(viewOf({ state: "mining", optimizer: opt({ profile: "max" }) }, { settings: { ...st.settings, measure: "sys", optimizer: { profile: "light" } } }), "profile-snap");
    d.profiles.snap = card();
    const capOf = { light: "4 e-cores · low power mode", efficient: "5 cores · low power mode", max: "8 cores · full power" };
    const nameOf = { light: "quiet", efficient: "balanced", max: "max" };
    checks.tuning_profile_selected =
      ["light", "efficient", "max"].every((id) => JSON.stringify(d.profiles[id].on) === JSON.stringify([id]) && d.profiles[id].value === nameOf[id] && d.profiles[id].cap === capOf[id]) &&
      JSON.stringify(d.profiles.snap.on) === '["max"]' && d.profiles.snap.value === "max" && d.profiles.snap.cap === "8 cores · full power";

    // 2. The progress panel while the engine runs: trial N of 12, the setup under test, its stage
    // and an ETA; pause/stop while mining, resume/stop once paused. A stale sweep in the snapshot
    // never shows once the engine stops — the not-running view reads `progress` (from
    // `list_trials`) instead.
    paint(viewOf({ state: "mining", optimizer: opt({ target: six, sweep: { ...sweeping, elapsed_s: 90, left_s: 390, trial_s: 480 } }) }), "sweeping");
    d.sweeping = progress();
    paint(viewOf({ state: "paused", optimizer: opt({ target: six, sweep: { ...sweeping, elapsed_s: 90 } }) }), "sweeping-paused");
    d.sweepingPaused = progress();
    paint(viewOf({ state: "mining", optimizer: opt({ sweep: { ...sweeping, testing: null, stage: null }, note: "waiting for the outlet" }) }), "sweep-waiting");
    d.waiting = progress();
    paint(viewOf({ state: "stopped", optimizer: opt({ sweep: sweeping }) }, { trials: { ...SYNTH_TRIALS, progress: [4, 12] } }), "sweep-stopped");
    d.sweepStopped = progress();
    checks.tuning_seeker_progress =
      d.sweeping.value === "trial 3 of 12" && d.sweeping.cap === "6 cores - 06:30 mins left" &&
      /all trials/.test(d.sweeping.tip || "") &&
      d.sweeping.buttons.length === 2 &&
      d.sweeping.buttons[0].act === "trials-pause" && d.sweeping.buttons[0].label === "pause" &&
      d.sweeping.buttons[1].act === "trials-stop" && d.sweeping.buttons[1].label === "stop" && d.sweeping.disabled === false &&
      d.sweepingPaused.cap === "paused" &&
      d.sweepingPaused.buttons[0].act === "trials-resume" && d.sweepingPaused.buttons[0].label === "continue" &&
      d.waiting.full === "waiting for the outlet" &&
      // Stopped: the stale snapshot sweep is ignored; the panel shows the ledger's own progress.
      d.sweepStopped.value === "trial 5 of 12" && d.sweepStopped.full === "4 of 12 trials done" &&
      d.sweepStopped.buttons.length === 1 && d.sweepStopped.buttons[0].act === "trials-go" &&
      d.sweepStopped.buttons[0].label === "continue trials" && d.sweepStopped.disabled === false;

    // 3. The held setup moved onto the throttle card's 2nd caption (P5); the progress panel shows
    // "done"/"ready" once nothing is testing, from the ledger's own progress.
    paint(viewOf({ state: "mining", optimizer: opt({ target: held.setup, held }) }), "held");
    d.held = { holding: holding(), progress: progress() };
    paint(viewOf({ state: "mining", optimizer: opt({ mode: "manual", note: "you set 6 threads" }) }), "manual");
    d.manual = { holding: holding(), progress: progress() };
    paint(viewOf({ state: "mining", optimizer: opt({ mode: "manual" }) }, { trials: null }), "manual-no-trials");
    d.manualNone = { holding: holding(), progress: progress() };
    checks.tuning_keel_held =
      d.held.holding === "" && /^trial \d+ of \d+$/.test(d.held.progress.value) && d.held.progress.button === "run trials again" &&
      d.manual.holding === "manual · your own cores" && /^trial \d+ of \d+$/.test(d.manual.progress.value) &&
      d.manualNone.holding === "manual · your own cores" && /^trial 1( of \d+)?$/.test(d.manualNone.progress.value) && d.manualNone.progress.button === "start trials";

    // 4. The curve from list_trials' curve with the running count in fill.
    const hsC = [279, 728, 1095, 1363, 1825, 1830, 1743, 1858];
    const curve = { low_power: true, points: hsC.map((v, i) => ({ threads: i + 1, hs: v })), standout: [5, 462, 0.208], flat_from: 5 };
    paint(viewOf({ state: "mining", optimizer: opt({ target: held.setup, held }) }, { trials: { ...SYNTH_TRIALS, curve } }), "curve");
    d.curve = {
      cap: textOf('#grid [data-mod="curve"] .curve__cap'),
      foot: ($q('#grid [data-mod="curve"] .curve__foot')?.innerText ?? "").replace(/\s+/g, " ").trim(),
      best: [...document.querySelectorAll('#grid [data-mod="curve"] .curve__col.is-best .curve__n')].map((e) => e.textContent.trim()),
      hs: [...document.querySelectorAll('#grid [data-mod="curve"] .curve__hs')].map((e) => e.textContent.trim()),
    };
    checks.tuning_curve_live =
      d.curve.cap === "best h/s added per watt · low power mode" && d.curve.foot === "the 5th core added +462 h/s for +0.21 w cores 6–8 add less than 2%" &&
      JSON.stringify(d.curve.best) === '["5"]' && d.curve.hs[4] === "1,825";

    // 5. Live trials: a snapshot whose trials_recorded moves reloads list_trials; a steady one doesn't.
    // The trials card lists what the reload brings (a new row appends).
    ui.setTab("tuning");
    await settle(300);
    const sentinel = { count: 0, rows: [], curve: {}, sentinel: true };
    st.trialsRecorded = 3;
    st.trials = sentinel;
    ui.applySnapshot(synthSnap({ optimizer: opt({ trials_recorded: 3 }) }));
    await settle(300);
    const steady = st.trials === sentinel;
    ui.applySnapshot(synthSnap({ optimizer: opt({ trials_recorded: 4 }) }));
    await settle(400);
    const reloaded = st.trials !== sentinel && st.trialsRecorded === 4;
    const rowsN = () => document.querySelectorAll('#grid [data-mod="trials"] .trial').length;
    const three = { count: 3, rows: [
      { label: "8 threads · low power mode", hw_sys: 220, hw_cpu: 600, noisy: false, t_end: 3 },
      { label: "5 threads · low power mode", hw_sys: 200, hw_cpu: 479, noisy: false, t_end: 2 },
      { label: "8 threads · full power", hw_sys: 143, hw_cpu: 274, noisy: false, t_end: 1 },
    ] };
    paint(viewOf(null, { trials: three }), "trials-3");
    const before = rowsN();
    paint(viewOf(null, { trials: { count: 4, rows: [...three.rows, { label: "6 threads · low power mode", hw_sys: 187, hw_cpu: 450, noisy: false, t_end: 4 }] } }), "trials-4");
    const after = rowsN();
    const value = textOf('#grid [data-mod="trials"] .card__value');
    d.trials = { steady, reloaded, before, after, value };
    checks.tuning_trials_live = steady && reloaded && before === 3 && after === 4 && value === "4";

    // 6. The Low Power Mode ask: the progress panel asks, and the guide opens once per ask through guideHooks.
    const calls = [];
    ui.guideHooks.lowpower = (want) => calls.push(want);
    const asking = (want) => synthSnap({ optimizer: opt({ sweep: { ...sweeping, testing: null, stage: null, done: 12 }, ask_low_power: want }) });
    ui.applySnapshot(asking(true));
    const askOpen = dialogShown("guide");
    const askProgress = progress();
    ui.closeGuide();
    ui.applySnapshot(asking(true));
    const once = calls.length === 1 && document.getElementById("guide").hidden;
    ui.applySnapshot(synthSnap({ optimizer: opt({ sweep: sweeping }) }));
    const cleared = st.ui.lowPowerAsk === null;
    ui.applySnapshot(asking(false));
    const offOpen = dialogShown("guide");
    const offProgress = progress();
    ui.closeGuide();
    delete ui.guideHooks.lowpower;
    d.ask = { calls, askOpen, askProgress, once, cleared, offOpen, offProgress };
    checks.tuning_lowpower_ask =
      JSON.stringify(calls) === "[true,false]" && askOpen && once && cleared && offOpen &&
      askProgress.value === "low power mode" && askProgress.full === "turn it on in battery settings" && askProgress.button === "skip" && askProgress.disabled === false &&
      offProgress.full === "turn it off in battery settings";

    // 7. Stop trials calls the real `stop_seek`, harmless with nothing mining (it ends the run and
    // stops mining): never refused, no error shows, and it logs "stopping" then "stopped" to the
    // trial log. The synthetic snapshot says a sweep runs, so the rendered button is `trials-stop`.
    st.snap = synthSnap({ optimizer: opt({ sweep: sweeping }) });
    st.ui.optError = null;
    ui.render();
    let backendSaid = null;
    try {
      await window.__TAURI__.core.invoke("stop_seek");
    } catch (err) {
      backendSaid = errMsg(err);
    }
    const tapeBefore = st.tape.length;
    await ui.actions["trials-stop"]();
    await settle(200);
    const inline = progress().full;
    const tapeAdded = st.tape.slice(tapeBefore).map((l) => l.msg);
    d.refused = { backendSaid, inline, at: st.ui.optError?.at, tapeAdded };
    checks.tuning_refusal_inline = backendSaid === null && !st.ui.optError && tapeAdded.includes("stopping") && tapeAdded.includes("stopped");
    fits.push(...cardOverflow("optimizer").map((b) => `refused:${b}`));
    st.ui.optError = null;
    st.snap = null;

    // 8. Nothing clips at the longest strings these states make (and the refusals the backend can give).
    const ec = { threads: 12, e_core_clamp: true };
    const longPick = { setup: ec, low_power: true, hw: 1234.5, hs: 9999 };
    const longTrials = { ...SYNTH_TRIALS, picks: { sys: { light: longPick, efficient: longPick, max: { ...longPick, setup: { threads: 12, e_core_clamp: false }, low_power: false } } } };
    const longest = [
      ["long-sweep", { state: "mining", optimizer: opt({ sweep: { testing: ec, stage: "settling", done: 23, planned: 24 } }) }],
      ["long-ask", { state: "mining", optimizer: opt({ sweep: { ...sweeping, testing: null }, ask_low_power: false }) }],
      ["long-held", { state: "mining", optimizer: opt({ target: ec, held: longPick }) }],
      ["long-manual", { state: "mining", optimizer: opt({ mode: "manual", note: "xmrig's api did not answer the thread change within five seconds" }) }],
    ];
    for (const [label, snap] of longest) {
      paint(viewOf(snap, { trials: longTrials, settings: { ...st.settings, measure: "sys" } }), label);
    }
    // A shortened label keeps its full words in the tooltip.
    paint(viewOf(longest[2][1], { trials: longTrials, settings: { ...st.settings, measure: "sys", optimizer: { profile: "light" } } }), "tip-profile");
    // The cores wording (2026-10-04) is short enough that the longest quiet setup fits whole: no
    // tooltip needed, and nothing cut.
    const profileCap = $q('#grid [data-mod="profile"] .card__cap');
    const profileTip = profileCap?.getAttribute("title");
    d.tips = { profileTip, text: profileCap?.textContent.trim() };
    const tipsOk = profileCap?.textContent.trim() === "12 e-cores · low power mode" && !profileTip;
    // P4-C21: every note the optimizer can send (driver.rs), and refusals the commands can give,
    // through every caption each can land in. Each fits whole, or is cut with its full words in
    // the tooltip; nothing clips.
    const NOTES = [
      "you set 1 thread", "you set 16 threads", "waiting for the outlet",
      "trials resumed", "reading past trials", "run trials first",
      // apply_failed passes the engine's error through as the note: any length, no " · " to cut at.
      "xmrig's api did not answer the thread change within five seconds",
    ];
    const REFUSALS = ["start mining to run trials", "no power readings on this mac", "settings.json could not be written: permission denied (os error 13)"];
    const whole = [];
    const cut = [];
    const notes = [];
    const land = (label, text, tip, full) => {
      if (text === full && !tip) whole.push(label);
      else if (tip === full && text.length < full.length) cut.push(label);
      else notes.push({ label, text, tip });
    };
    // A note only ever rides in the sweep's "between trials" caption now (P5): while holding
    // (no sweep), the progress panel is purely `progress`-driven, and holdingCaption (the
    // throttle card's 2nd line) never carries a note either.
    for (const n of NOTES) {
      paint(viewOf({ state: "mining", optimizer: opt({ sweep: { ...sweeping, testing: null, stage: null }, note: n }) }), `note-sweeping:${n}`);
      land(`sweeping:${n}`, progress().cap, progress().tip ?? "", n);
    }
    paint(viewOf({ state: "mining", optimizer: opt({ target: held.setup, held, note: "you set 6 threads" }) }), "note-holding-ignored");
    const holdingIgnoresNote = /^trial \d+ of \d+$/.test(progress().value) && progress().full !== "you set 6 threads";
    for (const r of REFUSALS) {
      for (const at of ["profile", "seeker"]) {
        paint(viewOf(null, { ui: { optError: { at, msg: r } } }), `refusal-${at}:${r}`);
        const el = at === "profile" ? $q('#grid [data-mod="profile"] .card__cap') : null;
        const h = at === "seeker" ? progress() : null;
        land(`${at}:${r}`, el ? el.textContent.trim() : h.cap, (el ? el.getAttribute("title") : h.tip) ?? "", r);
      }
    }
    d.notes = { whole: whole.length, cut, bad: notes, holdingIgnoresNote };
    const notesOk = notes.length === 0 && cut.includes("sweeping:xmrig's api did not answer the thread change within five seconds") && whole.includes("sweeping:waiting for the outlet");
    d.fits = fits;
    checks.tuning_live_fits = fits.length === 0 && tipsOk && notesOk && holdingIgnoresNote;

    // 9. Pick a trial row (P5 "pick a result"): the row carries its exact setup as data attrs,
    // and clicking it calls the real `apply_pick`. Stopped, that saves the exact setup for the
    // next start with no refusal. Any error fails this check (an ACL denial passed it until 0.2.21).
    {
      const rows = { count: 1, rows: [
        { label: "6 threads · low power mode", hw_sys: 210, hw_cpu: 480, noisy: false, t_end: 1, threads: 6, e_core_clamp: false, low_power: true },
      ] };
      const threads0 = (await window.__TAURI__.core.invoke("get_settings")).threads;
      paint(viewOf(null, { trials: rows }), "pick-row");
      st.ui.optError = null;
      const row = $q('#grid [data-mod="trials"] [data-act="apply-trial"]');
      const rowAttrs = { threads: row?.dataset.threads, clamp: row?.dataset.clamp, lp: row?.dataset.lp };
      await ui.actions["apply-trial"](row);
      await settle(200);
      const saved6 = await window.__TAURI__.core.invoke("get_settings");
      // The window's own copy follows (one source per fact): the cores card shows the pick.
      const uiThreads = st.settings.threads;
      const coresCard = buildView(st).threads.text;
      await window.__TAURI__.core.invoke("set_threads", { threads: threads0 }); // put the self-check's settings back
      st.settings = await window.__TAURI__.core.invoke("get_settings");
      d.pick = { rowAttrs, at: st.ui.optError?.at, msg: st.ui.optError?.msg, threads: saved6.threads, mode: saved6.optimizer?.mode, uiThreads, coresCard };
      checks.tuning_pick_trial = rowAttrs.threads === "6" && rowAttrs.clamp === "0" && rowAttrs.lp === "true" &&
        !st.ui.optError && saved6.threads === 6 && saved6.e_cores_only === false && saved6.utility_manual === false && saved6.optimizer?.mode === "manual" &&
        uiThreads === 6 && /^6 of \d+$/.test(coresCard);
      st.ui.optError = null;
    }
  } catch (err) {
    d.error = String(err);
    checks.tuning_live_fits = false;
  } finally {
    if (typeof saved.guideHook === "function") ui.guideHooks.lowpower = saved.guideHook;
    ui.closeGuide();
    Object.assign(st, {
      snap: saved.snap, settings: saved.settings, trials: saved.trials, trialsRecorded: saved.trialsRecorded, series: saved.series,
      powerSeries: saved.powerSeries, tape: saved.tape, context: saved.context, mismatch: saved.mismatch, lastError: saved.lastError,
    });
    st.ui.lowPowerAsk = saved.lowPowerAsk;
    st.ui.optError = saved.optError;
    ui.setPool(saved.pool);
    ui.setTab(saved.tab);
    ui.refresh();
  }
  const names = ["tuning_profile_selected", "tuning_seeker_progress", "tuning_keel_held", "tuning_curve_live", "tuning_trials_live", "tuning_lowpower_ask", "tuning_refusal_inline", "tuning_live_fits", "tuning_pick_trial"];
  for (const n of names) checks[n] = !!checks[n];
  if (names.some((n) => !checks[n])) detail.tuning_live = d;
}

function pressEsc() {
  document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
}

/** A dialog is shown, its card sits inside the window, and no row is clipped. */
function dialogShown(id) {
  const el = document.getElementById(id);
  if (!el || el.hidden) return false;
  const card = el.querySelector(".sheet__card").getBoundingClientRect();
  return card.height > 0 && card.top >= 0 && card.bottom <= window.innerHeight && card.left >= 0 && card.right <= window.innerWidth;
}

function dialogClipped(id) {
  const bad = [];
  document.querySelectorAll(`#${id} .sheet__card *`).forEach((el) => {
    if (el.matches('input[type="text"], input[type="number"]')) return;
    if (el.children.length === 0 && el.scrollWidth > el.clientWidth + 1 && el.clientWidth > 0) bad.push(`${el.className || el.tagName}: "${el.textContent}"`);
  });
  const card = document.querySelector(`#${id} .sheet__card`);
  if (card && card.scrollWidth > card.clientWidth + 1) bad.push("card");
  return bad;
}

/**
 * settings sheet and the low power guide: open and close (Esc, backdrop,
 * close button), the words, the controls, a real theme round-trip, and the
 * currency guard. Nothing valid but the user's own values is ever saved.
 */
async function sheetChecks(ui, checks, detail, info) {
  const st = ui.state;
  const d = {};
  const theme0 = st.settings?.theme;
  const cur0 = st.settings?.currency;
  try {
    // Open with the gear, close with Esc; open again, close with a backdrop click.
    document.getElementById("gear-btn").click();
    const open1 = dialogShown("sheet");
    const clipped = dialogClipped("sheet");
    const words = {
      privacy: textOf("#sheet-privacy"),
      wallet: textOf("#set-wallet-note"),
      updateRowGone: !document.getElementById("update-folder"), // retired in 0.0.1: updates come from GitHub
      rate: document.getElementById("set-rate").value,
      currency: document.getElementById("set-currency").value,
      theme: $q("#set-theme .seg__btn.is-on")?.dataset.themePick,
      measure: $q("#set-measure .seg__btn.is-on")?.dataset.measure,
    };
    pressEsc();
    const closedEsc = document.getElementById("sheet").hidden;
    ui.openSheet();
    const open2 = dialogShown("sheet");
    document.getElementById("sheet-scrim").click();
    const closedScrim = document.getElementById("sheet").hidden;
    d.sheet = { open1, open2, closedEsc, closedScrim, clipped, words };
    checks.settings_sheet_open_close = open1 && closedEsc && open2 && closedScrim && clipped.length === 0;

    const walletWant = "saved on this Mac, in XMRange's settings file";
    checks.settings_sheet_contents =
      words.privacy === "XMRange connects only to the pool that you select. It uses Tor unless you turn Tor off. MoneroOcean sees your wallet address, worker name and hashrate. It sees your IP address only when Tor is off. On p2pool, peers see your IP address only when Tor is off. Your payout address is public on the p2pool share chain." &&
      words.wallet === walletWant &&
      words.updateRowGone &&
      words.rate === String(st.settings.rate_per_kwh) && words.currency === st.settings.currency &&
      words.theme === (st.settings.theme || "system") && words.measure === (st.settings.measure === "cpu" ? "cpu" : "sys");

    // Theme: pick another in the sheet, see it applied and saved, pick the original back.
    ui.openSheet();
    const other = theme0 === "dark" ? "light" : "dark";
    $q(`#set-theme [data-theme-pick="${other}"]`).click();
    await settle();
    const applied = document.documentElement.dataset.theme;
    const stored1 = await window.__TAURI__.core.invoke("get_settings");
    const onBtn = $q("#set-theme .seg__btn.is-on")?.dataset.themePick;
    $q(`#set-theme [data-theme-pick="${theme0 || "system"}"]`).click();
    await settle();
    const stored2 = await window.__TAURI__.core.invoke("get_settings");
    d.theme = { theme0, other, applied, stored1: stored1.theme, onBtn, stored2: stored2.theme, now: document.documentElement.dataset.theme };
    checks.settings_theme = applied === other && stored1.theme === other && onBtn === other &&
      stored2.theme === (theme0 || "system") && document.documentElement.dataset.theme === (theme0 || "system");

    // Currency and rate: the backend's Settings::validate refuses, its words show
    // inline in the sheet verbatim, and the saved value comes back. Nothing is written.
    const backendSays = async (over) => {
      try {
        await window.__TAURI__.core.invoke("save_settings", { settings: { ...st.settings, ...over } });
        return null;
      } catch (err) {
        return errMsg(err);
      }
    };
    const errEl = document.getElementById("sheet-err");
    const curEl = document.getElementById("set-currency");
    const curMsg = await backendSays({ currency: "abcd" });
    curEl.value = "abcd";
    const r1 = (await ui.setCurrency(curEl)) === false;
    const shown1 = { text: errEl.textContent, hidden: errEl.hidden, field: curEl.value };
    curEl.value = "";
    const r2 = (await ui.setCurrency(curEl)) === false;
    const shown2 = { text: errEl.textContent, field: curEl.value };
    const rateEl = document.getElementById("set-rate");
    const rate0 = st.settings.rate_per_kwh;
    const rateMsg = await backendSays({ rate_per_kwh: 1e308 });
    rateEl.value = "1e308";
    const r3 = (await ui.setRate(rateEl, true)) === false;
    const shown3 = { text: errEl.textContent, hidden: errEl.hidden, field: rateEl.value };
    const sheetClip = dialogClipped("sheet");
    const stored3 = await window.__TAURI__.core.invoke("get_settings");
    d.inline = { curMsg, rateMsg, shown1, shown2, shown3, sheetClip };
    checks.settings_inline_errors =
      curMsg === "currency must be 1–3 characters, with no spaces or control characters" &&
      rateMsg === "electricity rate must be a number from 0 to 100 per kWh" &&
      r1 && shown1.text === curMsg && !shown1.hidden && shown1.field === cur0 &&
      r2 && shown2.text === curMsg && shown2.field === cur0 &&
      r3 && shown3.text === rateMsg && !shown3.hidden && shown3.field === String(rate0) &&
      sheetClip.length === 0 &&
      stored3.currency === cur0 && stored3.rate_per_kwh === rate0 && st.settings.currency === cur0 && st.settings.rate_per_kwh === rate0;

    // update_folder is pinned to the stored value (P2-R9): an incoming one is ignored.
    // This saves the user's own settings unchanged apart from that field.
    const folder0 = stored3.update_folder ?? null;
    const pinErr = await backendSays({ update_folder: "/somewhere/else" });
    const stored4 = await window.__TAURI__.core.invoke("get_settings");
    d.update_folder = { pinErr, before: folder0, after: stored4.update_folder ?? null };
    checks.settings_update_folder_pinned = pinErr === null && (stored4.update_folder ?? null) === folder0;

    // Reopening the sheet clears an old refusal.
    ui.closeSheet();
    ui.openSheet();
    checks.settings_error_clears = errEl.hidden && errEl.textContent === "";
    ui.closeSheet();

    // Low power guide: opens from the card's corner pill, says why it helps and how to turn it on,
    // closes by Esc and backdrop. "open battery settings" calls the real `open_battery_settings`
    // command, which is stubbed on the backend (self_check_mode() there never really shells out to
    // `/usr/bin/open`) rather than here — `window.__TAURI__.core.invoke` is a read-only property in
    // this build, so it can't be monkey-patched from the frontend. A broken wiring (a missing or
    // misspelled command) would instead push a tape error, which the tape-length check below catches.
    ui.setPool("mo");
    ui.setTab("machine");
    $q('#grid [data-mod="lowpower"] [data-act="guide"]').click();
    const g1 = dialogShown("guide");
    const gwhy = textOf("#guide-why");
    const steps = textOf("#guide-steps");
    const gclip = dialogClipped("guide");
    const wantDetected = ui.lowPowerDetected() === true;
    const detectedMatches = document.getElementById("guide-detected").hidden === !wantDetected;
    const tapeBefore = st.tape.length;
    document.getElementById("guide-open-settings").click();
    await settle();
    const clickedOk = st.tape.length === tapeBefore;
    pressEsc();
    const g2 = document.getElementById("guide").hidden;
    ui.openGuide();
    document.getElementById("guide-scrim").click();
    const g3 = document.getElementById("guide").hidden;
    d.guide = { g1, gwhy, steps, gclip, g2, g3, detectedMatches, clickedOk };
    checks.lowpower_guide =
      g1 && g2 && g3 && gclip.length === 0 && gwhy === lowPowerGuideText(st.trials) &&
      steps.includes("Low Power Mode") && steps.includes("Always or Only on Power Adapter") &&
      detectedMatches && clickedOk;

    // Node guide: the startup flag is selectable and fits, and "copy" reaches the backend's
    // copy_text (stubbed in self-check: the owner's clipboard is never touched).
    ui.closeGuide();
    ui.openGuide("node");
    const n1 = dialogShown("node-guide");
    const nclip = dialogClipped("node-guide");
    const flag = document.getElementById("node-flag");
    const selectable = getComputedStyle(flag).webkitUserSelect === "text" || getComputedStyle(flag).userSelect === "text";
    document.getElementById("node-flag-copy").click();
    await settle();
    const copied = textOf("#node-flag-copy") === "copied";
    pressEsc();
    const stamp = ui.fileStamp(new Date(2026, 9, 2, 14, 3, 7));
    d.nodeGuide = { n1, nclip, flag: flag.textContent, selectable, copied, stamp };
    // The Network Log opens, fits, and lists what this setup may reach (never an address).
    await ui.openNetworkLog();
    await settle(400);
    const nl = { shown: dialogShown("network-log"), clip: dialogClipped("network-log"), now: textOf("#network-now"), allowed: [...document.querySelectorAll("#network-allowed li")].map((l) => l.textContent) };
    pressEsc();
    d.networkLog = nl;
    checks.network_log =
      nl.shown && nl.clip.length === 0 && nl.allowed.length >= 2 && nl.allowed.some((l) => l.startsWith("updates:")) &&
      !/\b\d{1,3}(\.\d{1,3}){3}\b/.test(nl.now + nl.allowed.join(" "));
    // Feedback: the row, the sheet, its controls, and a report text that holds no address (send is never pressed).
    await ui.openFeedback("problem");
    await settle(800);
    const fbText = document.getElementById("fb-text").value;
    const fbKinds = [...document.querySelectorAll("#fb-kind [data-kind]")].filter((b) => !b.hidden).map((b) => b.dataset.kind);
    const fb = { shown: dialogShown("feedback"), clip: dialogClipped("feedback"), kinds: fbKinds, textLen: fbText.length };
    pressEsc();
    await ui.openFeedback("crash");
    await settle(300);
    fb.crashKind = !document.querySelector('#fb-kind [data-kind="crash"]').hidden;
    pressEsc();
    d.feedback = fb;
    checks.feedback =
      !!document.getElementById("fb-open") && ["fb-copy", "fb-save", "fb-send", "fb-note", "feedback-close", "crash-send", "crash-dismiss"].every((id) => !!document.getElementById(id)) &&
      fb.shown && fb.clip.length === 0 && fb.kinds.join() === "problem,idea" && fb.crashKind && fbText.startsWith("XMRange problem report") &&
      !/\b\d{1,3}(\.\d{1,3}){3}\b/.test(fbText) && !/\/Users\//.test(fbText) &&
      document.getElementById("crash-offer").hidden === true;
    // Limits: the temperature row always; the battery row hidden only on a desktop.
    const acRow = document.getElementById("set-ac-row");
    checks.limits_rows = !!document.getElementById("set-pause-above") && acRow.hidden === (ui.state.info?.form === "desktop");
    // The temperature limit through the real settings path: a bad value is refused inline and
    // nothing is saved; a good one is saved; empty turns it off again.
    const pauseInput = document.getElementById("set-pause-above");
    pauseInput.value = "50";
    const badSaved = await ui.setPauseAbove(pauseInput);
    const badErr = st.ui.settingsError || "";
    pauseInput.value = "200";
    const goodSaved = await ui.setPauseAbove(pauseInput);
    const stored200 = (await window.__TAURI__.core.invoke("get_settings")).guard?.pause_above_f;
    pauseInput.value = "";
    const offSaved = await ui.setPauseAbove(pauseInput);
    const storedOff = (await window.__TAURI__.core.invoke("get_settings")).guard?.pause_above_f ?? null;
    d.pauseAbove = { badSaved, badErr, goodSaved, stored200, offSaved, storedOff };
    checks.limits_save_path =
      badSaved === false && /100 to 230/.test(badErr) && goodSaved === true && stored200 === 200 && offSaved === true && storedOff === null;
    // A limit pause says why on the hashrate card.
    const gv = buildView({ ...ui.state, snap: synthSnap({ state: "paused", guard_note: "paused · on battery" }) }).hashrate;
    checks.guard_caption = gv.value === "paused" && gv.sub === "paused · on battery";
    // ASD-STE100 guards: a utility trial row says so, and no visible text uses a contraction.
    const uRow = trialLabelHtml({ threads: 4, e_core_clamp: false, utility: true, low_power: null });
    checks.utility_row_label = /^4 cores · utility </.test(uRow);
    const visible = [document.body.innerText, ...[...document.querySelectorAll("[title]")].map((e) => e.getAttribute("title"))].join(" ");
    const contractions = [...new Set(visible.match(/\b[a-z]+n't\b/gi) ?? [])];
    d.contractions = contractions;
    checks.no_contractions = contractions.length === 0;
    // Every icon on the page loads (a wrong path draws a broken image).
    const broken = [...document.images].filter((i) => i.getAttribute("src") && !(i.complete && i.naturalWidth > 0)).map((i) => i.getAttribute("src"));
    d.brokenImages = broken;
    checks.every_icon_loads = broken.length === 0;
    // The price feed asks before it adds a Tor connection; cancel saves nothing.
    ui.openSheet();
    const feedBefore = st.settings.price_feed === true;
    document.querySelector('#set-price-feed [data-feed="on"]').click();
    const pc1 = dialogShown("price-confirm");
    const pclip = dialogClipped("price-confirm");
    document.getElementById("price-confirm-cancel").click();
    await settle();
    const pc2 = document.getElementById("price-confirm").hidden && (st.settings.price_feed === true) === feedBefore;
    const priceSheetClip = dialogClipped("sheet");
    ui.closeSheet();
    d.priceConfirm = { pc1, pclip, pc2, priceSheetClip };
    checks.price_feed_asks_first = pc1 && pclip.length === 0 && pc2 && priceSheetClip.length === 0;
    checks.node_guide_copy =
      n1 && nclip.length === 0 && flag.textContent === "--zmq-pub tcp://127.0.0.1:18083" && selectable && copied &&
      stamp === "2026-10-02-14-03-07";
  } catch (err) {
    d.error = String(err);
    checks.settings_sheet_open_close = false;
  } finally {
    ui.closeSheet();
    ui.closeGuide();
    if (theme0 && st.settings && st.settings.theme !== theme0) await ui.setTheme(theme0);
    ui.setTab("mining");
    ui.refresh();
  }
  const names = [
    "settings_sheet_open_close", "settings_sheet_contents", "settings_theme", "settings_inline_errors",
    "settings_update_folder_pinned", "settings_error_clears", "lowpower_guide",
  ];
  if (names.some((n) => !checks[n])) detail.sheet = d;
}

/**
 * The top-left update pill: hidden with no offer, shown with the offer's
 * version once `state.update` is set (`update_status`; a dev build never asks GitHub). The real
 * swap (`install_update`) is never run here.
 */
function updatePillChecks(ui, checks, detail) {
  const st = ui.state;
  const prevUpdate = st.update;
  const d = {};
  try {
    st.update = null;
    ui.render();
    const el = document.getElementById("update-pill");
    const hiddenWhenNone = el.hidden && getComputedStyle(el).display === "none";

    st.update = { version: "9.9.9", notes: "test" };
    ui.render();
    const shown = !el.hidden;
    const text = el.textContent;
    d.pill = { hiddenWhenNone, shown, text };
    checks.update_pill_renders = hiddenWhenNone && shown && text === "update 9.9.9 → restart";
  } catch (err) {
    d.error = String(err);
    checks.update_pill_renders = false;
  } finally {
    st.update = prevUpdate;
    ui.render();
  }
  if (!checks.update_pill_renders) detail.updatePill = d;
}

/**
 * I4: idle telemetry (xmrange://idle) fills the machine tab while nothing
 * mines, feeds the power chart one slot per second (two per 2 s sample), and
 * is ignored while an engine runs. M2: a live change xmrig didn't take shows
 * "not applied · see the logs", but only once it outlasts a change in flight.
 */
function idleAndAppliedChecks(ui, checks, detail) {
  const st = ui.state;
  const saved = saveState(st);
  const savedIdle = st.idlePower;
  const savedMismatch = { ...st.mismatch };
  const d = {};
  try {
    ui.setPool("mo");
    ui.setTab("machine");
    st.context = null;
    st.powerSeries = [];
    ui.applyStopped();
    const pw = { cpu_w: 0.6, ram_w: 0.2, sys_w: 4.3, temp_c: 38, p_mhz: 700, e_mhz: 950, p_active: 0.05, e_active: 0.2, mem_used_gb: 9.1, mem_total_gb: 16 };
    ui.applyIdle({ t: 1, power: pw, context: { low_power_mode: false, on_ac: true, battery_pct: 88, thermal: "nominal" } });
    ui.applyLowPower(false);
    const val = (id) => textOf(`#grid [data-mod="${id}"] .card__value`);
    const pv = () => [...document.querySelectorAll('#grid [data-mod="power"] .power__value')].map((e) => e.textContent.trim());
    d.idle = { light: document.body.dataset.run, power: pv(), temperature: val("temperature"), memory: val("memory"), lowpower: val("lowpower"), awake: val("awake"), slots: st.powerSeries.length };
    const idleOk = d.idle.light === "stopped" && d.idle.power[0] === "4.3 w" && d.idle.power[1] === "outlet" &&
      d.idle.temperature === "100°f" && d.idle.memory === "9.1 / 16 gb" && d.idle.lowpower === "off" &&
      d.idle.awake === "normal sleep" && d.idle.slots === 2 && textIsClean();
    // Mining tab while idle: h/w stays "—" (no hashes); cost shows the Mac's electricity anyway.
    ui.setTab("mining");
    d.idle.cost = cardVal("cost");
    d.idle.hw = cardVal("hw");
    // An engine is running: its snapshot owns the tab, and a late idle sample changes nothing.
    ui.applySnapshot(synthSnap({ state: "mining", hs10: 1800, power: { ...pw, sys_w: 9.9 } }));
    ui.applyIdle({ t: 2, power: { ...pw, sys_w: 1.1 } });
    ui.setTab("machine");
    d.idle.whileMining = pv()[0];
    checks.idle_telemetry = idleOk && d.idle.cost !== "—" && d.idle.hw === "—" && d.idle.whileMining === "9.9 w";

    // M2. The cards' intended values vs what xmrig runs.
    ui.setTab("mining");
    const masked = st.info?.wallet?.masked;
    const threads = st.settings?.threads;
    const off = { state: "mining", hs10: 1800, wallet_masked: "8Z…ZZZZ", threads: (threads ?? 1) + 1 };
    const walletNote = () => textOf('#grid [data-mod="wallet"] .wallet__note');
    const threadCap = () => textOf('#grid [data-mod="threads"] .card__cap');
    st.mismatch = { wallet: 0, threads: 0 };
    ui.applySnapshot(synthSnap(off));
    ui.applySnapshot(synthSnap(off));
    d.applied = { early: [walletNote(), threadCap()] };
    ui.applySnapshot(synthSnap(off));
    d.applied.late = [walletNote(), threadCap()];
    d.applied.fit = [...cardOverflow("wallet"), ...cardOverflow("threads")];
    ui.applySnapshot(synthSnap({ ...off, wallet_masked: masked, threads }));
    d.applied.agreed = [walletNote(), threadCap()];
    const na = "not applied · see the logs";
    checks.not_applied_shown =
      typeof masked === "string" && typeof threads === "number" &&
      d.applied.early[0] !== na && d.applied.early[1] !== na &&
      d.applied.late[0] === na && d.applied.late[1] === na && d.applied.fit.length === 0 &&
      d.applied.agreed[0] !== na && d.applied.agreed[1] !== na;
  } catch (err) {
    d.error = String(err);
    checks.idle_telemetry = false;
  } finally {
    st.idlePower = savedIdle;
    st.mismatch = savedMismatch;
    restoreState(ui, saved);
  }
  if (!checks.idle_telemetry || !checks.not_applied_shown) detail.idle_applied = d;
}

/**
 * M10: a short window (a 13" Air, down to the 600px minimum). The header keeps
 * its place, the grid becomes the scroller with the same card geometry, and
 * scrolling it to the end shows the last card whole. Simulated by giving the
 * board the short height (CSSOM, allowed under the CSP); the window itself is
 * not resized.
 */
function shortWindowChecks(ui, checks, detail) {
  const st = ui.state;
  const prev = { tab: st.tab, pool: st.pool };
  const board = document.getElementById("board");
  const grid = document.getElementById("grid");
  const d = {};
  try {
    ui.setPool("mo");
    ui.setTab("mining");
    for (const h of [849, 600]) {
      board.style.height = `${h}px`;
      const status = document.querySelector(".statusrow").getBoundingClientRect();
      const g = grid.getBoundingClientRect();
      const hr = rect("hashrate");
      grid.scrollTop = grid.scrollHeight;
      const last = document.querySelector('#grid [data-mod="difficulty"]').getBoundingClientRect(); // the last card on mining
      const lastWhole = last.bottom <= grid.getBoundingClientRect().bottom + 1 && last.top >= g.top - 1;
      d[h] = {
        statusBottom: Math.round(status.bottom), gridTop: Math.round(g.top), gridBottom: Math.round(g.bottom),
        scrolls: grid.scrollHeight > grid.clientHeight, hr, lastWhole, docScroll: document.documentElement.scrollHeight,
      };
      grid.scrollTop = 0;
    }
    const ok = (h) => {
      const r = d[h];
      return r.statusBottom < r.gridTop && r.gridBottom <= h + 1 && r.scrolls && r.lastWhole &&
        !!r.hr && r.hr.w === 440 && r.hr.h === 336 && r.docScroll <= window.innerHeight;
    };
    checks.short_window_scrolls = ok(849) && ok(600);
  } catch (err) {
    d.error = String(err);
    checks.short_window_scrolls = false;
  } finally {
    board.style.height = "";
    grid.scrollTop = 0;
    ui.setPool(prev.pool);
    ui.setTab(prev.tab);
  }
  if (!checks.short_window_scrolls) detail.short_window = d;
}

/** Waits for one event matching pred; `ready` resolves once the listener is registered. */
function waitEvent(name, pred, ms) {
  let unlisten = null;
  let done = false;
  let resolveFn;
  const promise = new Promise((r) => {
    resolveFn = r;
  });
  const finish = (v) => {
    if (done) return;
    done = true;
    clearTimeout(timer);
    if (unlisten) unlisten();
    resolveFn(v);
  };
  const timer = setTimeout(() => finish(null), ms);
  const ready = window.__TAURI__.event
    .listen(name, (e) => {
      if (pred(e.payload)) finish(e.payload ?? true);
    })
    .then((u) => {
      unlisten = u;
      if (done) u();
    })
    .catch(() => finish(null));
  return { promise, ready };
}

/**
 * Live wiring, end to end (dev builds, XMRANGE_SELF_CHECK_LIVE=1 only): the
 * live light's own click starts real mining with whatever wallet is set, the
 * check waits up to 90 s for a "mining" snapshot with a numeric hs10, reads
 * the hashrate card and the tape, then stops and waits for xmrange://stopped.
 */
async function liveMiningChecks(ui, checks, detail) {
  const live = {};
  ui.setPool("mo");
  ui.setTab("mining");
  const pill = (act) => $q(`#grid [data-mod="hashrate"] [data-act="${act}"]`);
  const isSnap = (state) => (p) => p && p.state === state && typeof p.hs10 === "number" && Number.isFinite(p.hs10) && p.hs10 > 0;
  const t0 = Date.now();
  let idleWhileMining = 0;
  let countIdle = false;
  const idleCounter = await window.__TAURI__.event.listen("xmrange://idle", () => {
    if (countIdle) idleWhileMining += 1;
  });
  const hasPower = (p) => p && p.power && typeof p.power.cpu_w === "number";
  try {
    // I4: before anything mines, the idle sampler's real figures fill the machine tab.
    const wi = waitEvent("xmrange://idle", hasPower, 8000);
    await wi.ready;
    const idle = await wi.promise;
    await settle(50);
    ui.setTab("machine");
    const mvi = (id) => textOf(`#grid [data-mod="${id}"] .card__value`);
    const pvi = [...document.querySelectorAll('#grid [data-mod="power"] .power__value')].map((e) => e.textContent.trim());
    live.idle_before = { event: !!idle, light: document.body.dataset.run, power: pvi, cpu: mvi("cpu"), temperature: mvi("temperature"), memory: mvi("memory") };
    checks.live_idle_machine =
      !!idle && live.idle_before.light === "stopped" && /^\d+\.\d w$/.test(pvi[0] || "") && /^\d+ of \d+ busy$/.test(live.idle_before.cpu) &&
      /^\d+°f$/.test(live.idle_before.temperature) && /^[\d.]+ \/ [\d.]+ gb$/.test(live.idle_before.memory) && textIsClean();
    ui.setTab("mining");

    // Start through the hashrate card's own start pill (the live light's busy-guarded path).
    const before = readPills();
    live.pills_before = before;
    const w = waitEvent("xmrange://snapshot", isSnap("mining"), 90000);
    await w.ready;
    pill("run-primary").click();
    const snap = await w.promise;
    await settle(50); // let the app's own listener paint
    live.seconds_to_mining = Math.round((Date.now() - t0) / 1000);
    checks.live_mining_snapshot = !!snap;
    const hr = cardVal("hashrate");
    const lines = document.querySelectorAll('#grid [data-mod="tape"] .tape__line').length;
    live.hs10 = snap ? Math.round(snap.hs10) : null;
    live.hashrate_card = hr;
    live.tape_lines = lines;
    live.sparkline = !!$q('#grid svg[data-chart="hashrate"] path');
    checks.live_hashrate_number = /^\d{1,3}(,\d{3})*$/.test(hr) && hr !== "0";
    checks.live_tape_lines = lines >= 1;
    checks.live_light_mining = document.body.dataset.run === "mining";
    checks.live_no_nan_infinity = textIsClean();
    const mining = readPills();
    live.pills_mining = mining;

    // Ruling P5-O2, live: 60 s into mining (and until xmrig has its 60 s average, 30 s more at
    // most), the h/w card shows hw_sys / hw_cpu, and that is hs60 over the mean watts of the same
    // 60 s; cost and earned read their 15-minute fields, still filling.
    let ticks = 0;
    const w60 = waitEvent("xmrange://snapshot", (p) => p && p.state === "mining" && ++ticks >= 60, 90000);
    await w60.ready;
    let at60 = await w60.promise;
    if (at60 && typeof at60.hs60 !== "number") {
      const wh = waitEvent("xmrange://snapshot", (p) => p && p.state === "mining" && typeof p.hs60 === "number", 30000);
      await wh.ready;
      at60 = (await wh.promise) || at60;
    }
    await settle(50);
    const m60 = ui.state.settings?.measure === "cpu" ? "cpu" : "sys";
    const rnd = (v, d = 2) => (typeof v === "number" ? Number(v.toFixed(d)) : v ?? null);
    live.at_60s = at60 && {
      seconds_mining: ticks, measure: m60, hw_card: cardVal("hw"), hw_cap: textOf('#grid [data-mod="hw"] .card__base'),
      earned_card: cardVal("earned"), cost_card: cardVal("cost"),
      hs10: rnd(at60.hs10, 1), hs60: rnd(at60.hs60, 1), hs15: rnd(at60.hs15, 1),
      sys_w_60: rnd(at60.sys_w_60), cpu_w_60: rnd(at60.cpu_w_60), sys_w_900: rnd(at60.sys_w_900),
      hw_sys: rnd(at60.hw_sys, 1), hw_cpu: rnd(at60.hw_cpu, 1),
    };
    const shownHw = (v) => (v >= 100 ? Math.round(v).toLocaleString("en-US") : v.toFixed(1)); // modules.js fmtHw
    const ratio = (h, w, got) => typeof h === "number" && typeof w === "number" && typeof got === "number" && Math.abs(h / w - got) < 1e-6 * got;
    checks.live_rolling_60s =
      !!at60 && typeof at60.hs60 === "number" && ratio(at60.hs60, at60.sys_w_60, at60.hw_sys) && ratio(at60.hs60, at60.cpu_w_60, at60.hw_cpu) &&
      typeof at60.sys_w_900 === "number" && live.at_60s.hw_card === shownHw(m60 === "cpu" ? at60.hw_cpu : at60.hw_sys) &&
      live.at_60s.hw_cap === HW_CAP[m60] && live.at_60s.earned_card !== "—" && textIsClean();
    // From here on the engine owns macmon: the idle sampler must stay quiet (a sample already
    // under way when mining began has had 3 s to land).
    await settle(3000);
    countIdle = true;

    // The machine tab, fed by the same real snapshots: watts, cpu, °f, context.
    ui.setTab("machine");
    const mv = (id) => textOf(`#grid [data-mod="${id}"] .card__value`);
    const pv = [...document.querySelectorAll('#grid [data-mod="power"] .power__value')].map((e) => e.textContent.trim());
    live.machine = { power: pv, cpu: mv("cpu"), temperature: mv("temperature"), memory: mv("memory"), thermal: mv("thermal"), lowpower: mv("lowpower"), awake: mv("awake") };
    checks.live_machine_figures =
      /^\d+\.\d w$/.test(pv[0] || "") && /^\d+ of \d+ busy$/.test(live.machine.cpu) && /^\d+°f$/.test(live.machine.temperature) &&
      /^[\d.]+ \/ [\d.]+ gb$/.test(live.machine.memory) && ["on", "off"].includes(live.machine.lowpower) &&
      live.machine.awake === "kept awake" && textIsClean();
    ui.setTab("mining");

    // Pause, then resume, through the same pill.
    const wp = waitEvent("xmrange://snapshot", (p) => p && p.state === "paused", 20000);
    await wp.ready;
    pill("run-primary").click();
    const paused = await wp.promise;
    await settle(50);
    const pausedPills = readPills();
    live.pills_paused = pausedPills;
    const light = document.body.dataset.run;
    const wr = waitEvent("xmrange://snapshot", (p) => p && p.state === "mining", 20000);
    await wr.ready;
    pill("run-primary").click();
    const resumed = await wr.promise;
    live.pause_resume = { paused: !!paused, light, resumed: !!resumed, seconds: Math.round((Date.now() - t0) / 1000) };
    checks.live_run_pills =
      before.primary?.label === "start" && !before.primary.disabled && before.stop?.disabled === true &&
      mining.primary?.label === "pause" && !mining.primary.disabled && mining.stop?.disabled === false &&
      !!paused && light === "paused" && pausedPills.primary?.label === "resume" && !pausedPills.primary.disabled &&
      !!resumed;

    // The cores card while really mining (owner, 2026-10-05): −, +, e on, e off, each by its own
    // button. Each must reach xmrig (a snapshot running the count the card shows, within 30 s),
    // then hold for 5 more snapshots, with no "not applied" painted at any moment, no error line,
    // and the saved settings saying the same.
    const coresBtn = (act) => $q(`#grid [data-mod="threads"] [data-act="${act}"]`);
    const cores0 = await window.__TAURI__.core.invoke("get_settings");
    const steps = [];
    for (const act of ["threads-down", "threads-up", "ecores-toggle", "ecores-toggle"]) {
      const b = coresBtn(act);
      const step = { act, from: ui.state.snap?.threads };
      if (!b || b.disabled) {
        step.skipped = "disabled";
        steps.push(step);
        continue;
      }
      const errs0 = ui.state.tape.filter((l) => l.tag === "error").length;
      let sawNotApplied = false;
      const watch = setInterval(() => {
        if (document.body.innerText.includes(NOT_APPLIED)) sawNotApplied = true;
      }, 200);
      const t = Date.now();
      b.click();
      await settle(300);
      const want = () => (ui.state.snap?.optimizer?.mode === "hold" && ui.state.snap?.optimizer?.target ? ui.state.snap.optimizer.target.threads : ui.state.settings?.threads);
      const wl = waitEvent("xmrange://snapshot", (p) => p && p.state === "mining" && p.threads === want(), 30000);
      await wl.ready;
      const landed = await wl.promise;
      step.seconds = Math.round((Date.now() - t) / 100) / 10;
      let held = 0;
      const wh = waitEvent("xmrange://snapshot", (p) => p && p.state === "mining" && (p.threads === want() ? ++held >= 5 : (held = -99)), 15000);
      await wh.ready;
      await wh.promise;
      clearInterval(watch);
      await settle(50);
      const stored = await window.__TAURI__.core.invoke("get_settings");
      Object.assign(step, {
        to: ui.state.snap?.threads, want: want(), landed: !!landed, held, sawNotApplied,
        card: textOf('#grid [data-mod="threads"] .stepper__value'), clamp: stored.e_cores_only, saved: stored.threads,
        errors: ui.state.tape.filter((l) => l.tag === "error").slice(errs0).map((l) => l.msg),
      });
      const of = stored.e_cores_only ? ui.state.info?.machine?.e_cores : ui.state.info?.max_threads;
      step.ok = step.landed && held >= 5 && !sawNotApplied && step.errors.length === 0 && stored.threads === step.want &&
        step.card === `${step.want} of ${of}`;
      steps.push(step);
    }
    // The cores as the run found them (settings_unchanged compares at the end).
    await window.__TAURI__.core.invoke("apply_pick", { threads: cores0.threads, eCoreClamp: cores0.e_cores_only, utility: cores0.utility_manual, lowPower: null });
    if (cores0.optimizer?.mode === "hold") await window.__TAURI__.core.invoke("set_profile", { profile: cores0.optimizer.profile });
    ui.state.settings = await window.__TAURI__.core.invoke("get_settings");
    live.cores_card = steps;
    checks.live_cores_card_controls = steps.filter((s) => !s.skipped).length >= 3 && steps.every((s) => s.skipped || s.ok);
  } catch (err) {
    live.error = String(err);
    checks.live_mining_snapshot = false;
  } finally {
    // Stop through the stop pill; if it can't be clicked, stop directly. Either way, wait for xmrange://stopped.
    await settle(50);
    countIdle = false; // the engine is about to go: idle samples are due again from here
    const gen = ui.state.engineGen;
    const w2 = waitEvent("xmrange://stopped", () => true, 30000);
    await w2.ready;
    const stopBtn = pill("run-stop");
    live.stopped_by = stopBtn && !stopBtn.disabled ? "pill" : "stopMining";
    if (live.stopped_by === "pill") stopBtn.click();
    else await ui.stopMining();
    let stoppedGen = await w2.promise;
    if (stoppedGen === null) {
      live.stopped_by += "+fallback";
      await ui.stopMining();
    }
    checks.live_stopped = stoppedGen !== null;
    // start_mining's generation comes back on xmrange://stopped, so a stale event can be told apart.
    checks.live_stopped_gen = typeof gen === "number" && stoppedGen === gen;
    live.gen = { started: gen, stopped: stoppedGen };
    await settle(50);
    checks.live_light_stopped = document.body.dataset.run === "stopped";
    const after = readPills();
    live.pills_after = after;
    checks.live_pills_stopped = after.primary?.label === "start" && after.stop?.disabled === true;
    live.idle_while_mining = idleWhileMining;
    checks.live_idle_quiet_while_mining = idleWhileMining === 0;
    // Once stopped, the idle sampler takes the machine tab back.
    const wa = waitEvent("xmrange://idle", hasPower, 8000);
    await wa.ready;
    const idleAfter = await wa.promise;
    live.idle_after_stop = !!idleAfter;
    checks.live_idle_after_stop = !!idleAfter;
    idleCounter();
  }
  detail.live = live;
}

/** An Intel Mac (Stage 3): the Mac mini 2018 i7's facts read as cores and threads, never as
 * performance and efficiency cores, and the cores card has no e pill. Pure: buildView only. */
function intelViewChecks(checks, detail) {
  const info = { max_threads: 12, machine: { model: "Macmini8,1", chip: "intel core i7-8700b", p_cores: 6, e_cores: 0, memory_gb: 16 }, wallet: { is_default: true } };
  const state = {
    info, settings: { threads: 6, optimizer: { mode: "manual" } }, ui: {}, tape: [], series: [], powerSeries: [], mismatch: {}, trials: null, pool: "mo", tab: "mining",
    snap: { state: "mining", threads: 6, power: { p_active: 0.5, e_active: 0, cpu_w: 30, sys_w: 50, temp_c: 60 } },
  };
  const v = buildView(state);
  const words = JSON.stringify([v.threads, v.stats?.chip, v.stats?.cpu]);
  checks.intel_mac_reads_cores_and_threads =
    v.threads.split === "6 cores · 12 threads" && v.threads.ePill === false && v.stats.chip.sub === "6 cores · 12 threads" &&
    v.stats.cpu.value === "6 of 12 busy" && !/efficiency|performance/.test(words);
  if (!checks.intel_mac_reads_cores_and_threads) detail.intel = words;
}

/** XMRange's own node on the p2pool tab's node card: each keeper state says what is happening and
 * what to do, and the gui node's words ("open the monero gui") never show for it. Pure. */
function ownNodeViewChecks(checks, detail) {
  const say = (node, tor = true) => p2poolView(node, null, tor, null, null, true).node;
  const cases = {
    no_folder: say({ state: "no_folder" }),
    starting: say({ state: "starting" }),
    stopping: say({ state: "stopping" }),
    failed: say({ state: "failed", why: "the chain folder is gone · reconnect its drive, then choose the folder again" }),
    peers: say({ state: "syncing", height: 0, target: 0 }),
    syncing: say({ state: "syncing", height: 1500000, target: 3000000 }),
    ready: say({ state: "ready", height: 3000000 }),
    missing: say({ state: "missing" }),
  };
  const gui = p2poolView({ state: "missing" }, null, true, null, null, false).node;
  checks.own_node_card_states =
    cases.no_folder.sub.includes("chain folder") && cases.starting.value === "starting" && cases.stopping.sub.includes("2 min") &&
    cases.failed.sub.includes("reconnect") && cases.peers.sub === "finding peers · through tor" && cases.syncing.value === "50%" &&
    cases.ready.value === "ready" && !JSON.stringify(cases).includes("monero gui") && gui.sub === "open the monero gui";
  if (!checks.own_node_card_states) detail.own_node = cases;
}

/** The words Tauri itself uses when a call can't reach its command: a wiring fault, never a refusal. */
const IPC_FAULT = /not allowed by ACL|missing required key|invalid args|command \S+ not found|Unhandled Promise|is not a function|Cannot read propert/i;

/**
 * Every control, clicked (owner, 2026-10-05: "are you not simulating using all the buttons and
 * seeing what they do"). Stopped, on both pools and every tab: every enabled card button, the
 * header pills, the notice, and every button and field in the settings sheet. A click passes when
 * it adds no error line to the tape, no refusal in a card or the sheet, no uncaught error, no
 * Tauri wiring fault anywhere on screen, and no "not applied". Skipped here, each covered
 * elsewhere: start (the live run clicks it), run trials (the live run of the trials card), the
 * wallet's clear (settings checks), copy (it would overwrite the owner's clipboard), fetch through
 * tor (network). Open battery settings is clicked: self-check mode never shells out. Settings go
 * back after.
 */
async function controlSweepChecks(ui, checks, detail) {
  const st = ui.state;
  const saved = saveState(st);
  const settings0 = await window.__TAURI__.core.invoke("get_settings");
  const hooks0 = { ...ui.dialogHooks };
  ui.dialogHooks.confirmClear = async () => false;
  ui.dialogHooks.pickSave = async () => null;
  ui.dialogHooks.pickFolder = async () => null;
  const uncaught = [];
  const onErr = (e) => uncaught.push(String(e?.message ?? e?.reason?.message ?? e?.reason ?? e));
  window.addEventListener("error", onErr);
  window.addEventListener("unhandledrejection", onErr);
  const SKIP = new Set(["run-primary", "run-stop", "trials-go", "wallet-clear"]);
  const SKIP_IDS = new Set(["node-flag-copy", "price-confirm-ok", "update-pill", "fb-send"]);
  const clicked = [];
  const bad = [];
  const errLines = () => st.tape.filter((l) => l.tag === "error").map((l) => l.msg);
  const closeAll = async () => {
    for (let i = 0; i < 3; i++) pressEsc();
    ui.closeGuide();
    for (const id of ["network-log", "price-confirm", "node-guide", "feedback"]) {
      const d = document.getElementById(id);
      if (d && !d.hidden) document.getElementById(`${id}-close`)?.click() ?? document.getElementById(`${id}-cancel`)?.click();
    }
    await settle(50);
  };
  const judge = async (name, fire, wait = 400) => {
    const tape0 = errLines().length;
    const unc0 = uncaught.length;
    st.ui.optError = null;
    st.ui.settingsError = null;
    await fire();
    await settle(wait);
    const body = document.body.innerText;
    const fault = [];
    const newErr = errLines().slice(tape0);
    if (newErr.length) fault.push(`tape: ${newErr.join(" | ")}`);
    if (st.ui.optError) fault.push(`card: ${st.ui.optError.msg}`);
    if (st.ui.settingsError) fault.push(`sheet: ${st.ui.settingsError}`);
    if (uncaught.length > unc0) fault.push(`uncaught: ${uncaught.slice(unc0).join(" | ")}`);
    if (IPC_FAULT.test(body)) fault.push(`screen: ${body.match(IPC_FAULT)[0]}`);
    if (body.includes(NOT_APPLIED)) fault.push("screen: not applied");
    clicked.push(name);
    if (fault.length) bad.push({ control: name, fault });
  };
  try {
    await closeAll();
    // Card buttons, on each pool and tab, re-found after every click (each one repaints).
    for (const pool of ["mo", "p2"]) {
      ui.setPool(pool);
      for (const tab of ["mining", "machine", "tuning"]) {
        ui.setTab(tab);
        await settle(600); // the tuning tab loads its trials
        // A button's key is its action, value and place among buttons with the same action, so a
        // repaint that disables or adds another button never shifts it.
        const keyed = () => {
          const seen = {};
          return [...document.querySelectorAll("#grid [data-act]")]
            .filter((b) => !SKIP.has(b.dataset.act))
            .map((b) => {
              const base = `${pool}/${tab}/${b.dataset.act}${b.dataset.val ? `=${b.dataset.val}` : ""}`;
              seen[base] = (seen[base] ?? 0) + 1;
              return { key: `${base}#${seen[base]}`, b };
            });
        };
        const keys = keyed().filter((k) => !k.b.disabled).map((k) => k.key);
        for (const key of keys) {
          const b = keyed().find((k) => k.key === key)?.b;
          if (!b) {
            bad.push({ control: key, fault: ["gone before its click (a click before it repainted the card without it)"] });
            continue;
          }
          if (b.disabled) continue; // a click before it disabled it (a limit reached): nothing to judge
          await judge(key, async () => b.click());
          await closeAll();
          if (st.ui.walletEditing) ui.actions["wallet-cancel"]?.();
        }
      }
    }
    ui.setPool(saved.pool);
    ui.setTab("mining");
    await settle(300);
    // The cores card in both of its states, whatever order the pass above left it in: −, + with
    // the e pill off (set_threads), then on (apply_pick), then off again.
    const core = (act) => document.querySelector(`#grid [data-mod="threads"] [data-act="${act}"]`);
    const eOn = () => core("ecores-toggle")?.getAttribute("aria-pressed") === "true";
    if (eOn()) await judge("cores: e off", async () => core("ecores-toggle").click());
    for (const pass of ["e off", "e on"]) {
      if (pass === "e on") await judge("cores: e on", async () => core("ecores-toggle").click());
      await judge(`cores: − (${pass})`, async () => core("threads-down").click());
      await judge(`cores: + (${pass})`, async () => core("threads-up").click());
    }
    await judge("cores: e off again", async () => core("ecores-toggle").click());
    if (eOn()) bad.push({ control: "cores: e off again", fault: ["the e pill stayed on"] });
    // Header and notice.
    await judge("tabs", async () => {
      for (const t of ["machine", "tuning", "mining"]) document.querySelector(`#tabs [data-tab="${t}"]`).click();
    });
    await judge("theme pill (twice)", async () => {
      document.getElementById("theme-btn").click();
      await settle(150);
      document.getElementById("theme-btn").click();
    });
    if (!document.getElementById("notice").hidden) {
      await judge("notice: use yours", async () => document.getElementById("notice-use").click());
      await closeAll();
      if (st.ui.walletEditing) ui.actions["wallet-cancel"]?.();
    }
    // The settings sheet: every segment both ways, every field set to its own value, every button.
    await judge("gear", async () => document.getElementById("gear-btn").click());
    for (const [box, attr] of [["set-theme", "themePick"], ["set-measure", "measure"], ["set-tor", "tor"], ["set-ac", "ac"], ["set-price-feed", "feed"], ["set-chain", "chain"], ["set-node", "node"]]) {
      const opts = [...document.querySelectorAll(`#${box} button`)];
      const on = opts.find((b) => b.getAttribute("aria-pressed") === "true");
      for (const b of [...opts.filter((x) => x !== on), on].filter(Boolean)) {
        if (b.disabled) continue;
        await judge(`sheet ${box}=${b.dataset[attr]}`, async () => b.click());
        if (!document.getElementById("price-confirm").hidden) document.getElementById("price-confirm-cancel").click();
      }
    }
    for (const id of ["set-rate", "set-currency", "set-price", "set-pause-above"]) {
      const el = document.getElementById(id);
      if (!el || el.disabled) continue;
      await judge(`sheet ${id} (its own value)`, async () => el.dispatchEvent(new Event("change", { bubbles: true })));
    }
    for (const b of document.querySelectorAll("#sheet button[id]")) {
      if (SKIP_IDS.has(b.id) || b.hidden || b.disabled || b.id === "sheet-close") continue;
      await judge(`sheet #${b.id}`, async () => b.click(), b.id === "set-network" ? 2500 : 400);
      if (b.id === "set-network") {
        const now = [...document.querySelectorAll("#network-now li")].map((l) => l.textContent);
        if (now[0] === "reading…") bad.push({ control: "sheet #set-network", fault: ["the connection list never filled"] });
      }
      await closeAll();
      if (!document.getElementById("sheet").hidden) continue;
      document.getElementById("gear-btn").click();
      await settle(100);
    }
    await judge("sheet close", async () => document.getElementById("sheet-close").click());
    // Every dialog's close button, opened through its own control.
    await judge("node guide close", async () => {
      ui.openGuide("node");
      await settle(100);
      document.getElementById("node-guide-close").click();
    });
    await judge("low power guide: open battery settings, close", async () => {
      ui.openGuide("lowpower");
      await settle(100);
      document.getElementById("guide-open-settings").click();
      await settle(300);
      document.getElementById("guide-close").click();
    });
    // The chain folder: a folder that isn't there is refused in the sheet, verbatim, and nothing is saved.
    ui.openSheet();
    ui.dialogHooks.pickFolder = async () => "/definitely/not/a/folder/xmrange";
    await ui.chooseNodeFolder();
    await settle(100);
    const nodeErr = document.getElementById("sheet-err").textContent;
    const storedNode = (await window.__TAURI__.core.invoke("get_settings")).node_dir;
    checks.node_folder_refusal_inline = nodeErr.includes("existing folder") && storedNode === settings0.node_dir;
    if (!checks.node_folder_refusal_inline) detail.node_folder = { nodeErr, storedNode };
    st.ui.settingsError = null;
    ui.dialogHooks.pickFolder = async () => null;
    ui.closeSheet();
  } catch (err) {
    bad.push({ control: "sweep", fault: [String(err)] });
  } finally {
    window.removeEventListener("error", onErr);
    window.removeEventListener("unhandledrejection", onErr);
    Object.assign(ui.dialogHooks, hooks0);
    // save_settings keeps the cores and the profile as they are (they have their own commands).
    await window.__TAURI__.core.invoke("save_settings", { settings: settings0 });
    const s0 = settings0;
    await window.__TAURI__.core.invoke("apply_pick", { threads: s0.threads, eCoreClamp: s0.e_cores_only, utility: s0.utility_manual, lowPower: null });
    if (s0.optimizer?.mode === "hold") await window.__TAURI__.core.invoke("set_profile", { profile: s0.optimizer.profile });
    st.settings = await window.__TAURI__.core.invoke("get_settings");
    await closeAll();
    restoreState(ui, saved);
  }
  checks.every_control_clicked_clean = bad.length === 0 && clicked.length >= 25;
  detail.control_sweep = { clicked: clicked.length, bad, names: bad.length ? clicked : undefined };
}

export async function runSelfCheck(ui) {
  const checks = {};
  const detail = {};

  try {
    checks.ping = (await window.__TAURI__.core.invoke("ping")) === "pong";
  } catch {
    checks.ping = false;
  }

  checks.heartbeat = await waitForHeartbeat(3000);

  try {
    await document.fonts.load("600 20px Fraunces");
    checks.font_fraunces_600 = document.fonts.check("600 20px Fraunces");
  } catch {
    checks.font_fraunces_600 = false;
  }

  try {
    await document.fonts.load("400 20px Jost");
    checks.font_jost_400 = document.fonts.check("400 20px Jost");
  } catch {
    checks.font_jost_400 = false;
  }

  try {
    await document.fonts.load('400 20px "JetBrains Mono"');
    checks.font_jetbrains_mono_400 = document.fonts.check(
      '400 20px "JetBrains Mono"',
    );
  } catch {
    checks.font_jetbrains_mono_400 = false;
  }

  checks.emoji = await loadEmoji("assets/emoji/high_voltage.svg");

  let info = null;
  try {
    info = await window.__TAURI__.core.invoke("app_info");
    checks.app_info_tagline =
      info.tagline === "hashes per watt · every watt, weighed";
    checks.app_info_wallet_label =
      typeof info.wallet?.label === "string" &&
      info.wallet.label.includes("ghostdrip");
  } catch {
    checks.app_info_tagline = false;
    checks.app_info_wallet_label = false;
  }

  try {
    const settings = await window.__TAURI__.core.invoke("get_settings");
    checks.get_settings_rate_per_kwh = typeof settings?.rate_per_kwh === "number";
  } catch {
    checks.get_settings_rate_per_kwh = false;
  }

  // Format-valid (95 chars, starts with '4', all base58) but not a real
  // address, so it fails the Monero checksum rather than any real wallet.
  let typoMsg = null;
  try {
    const address = TYPO_ADDRESS;
    await window.__TAURI__.core.invoke("set_wallet", { address });
    checks.set_wallet_rejects_typo = false;
  } catch (err) {
    typoMsg = typeof err === "string" ? err : String(err?.message ?? err);
    checks.set_wallet_rejects_typo = typoMsg.includes("checksum");
  }

  // Settings as the run found them; compared again at the end.
  let settingsAtStart = null;
  try {
    settingsAtStart = JSON.stringify(await window.__TAURI__.core.invoke("get_settings"));
  } catch {
    settingsAtStart = null;
  }

  // Shell and mining-tab checks switch tabs and feed synthetic snapshots, so
  // they only run under XMRANGE_SELF_CHECK=1, never under a user's hands.
  if (info?.self_check) {
    await shellChecks(ui, checks, detail);
    await miningChecks(ui, checks, detail, info, typoMsg);
    runPillChecks(ui, checks, detail);
    cornerPillChecks(ui, checks, detail);
    await heroChecks(ui, checks, detail);
    machineChecks(ui, checks, detail, info);
    await tuningChecks(ui, checks, detail);
    await tuningLiveChecks(ui, checks, detail);
    await sheetChecks(ui, checks, detail, info);
    updatePillChecks(ui, checks, detail);
    await fixChecks(ui, checks, detail);
    idleAndAppliedChecks(ui, checks, detail);
    shortWindowChecks(ui, checks, detail);
    await controlSweepChecks(ui, checks, detail);
    intelViewChecks(checks, detail);
    ownNodeViewChecks(checks, detail);
  }

  // Dev builds with XMRANGE_SELF_CHECK_LIVE=1 only: a short real mining run.
  if (info?.self_check && info?.self_check_live) {
    await liveMiningChecks(ui, checks, detail);
  }

  // Every setting the run touched is back as it found it. (If the run dies
  // first, the backend's settings.selfcheck-backup.json puts it back.)
  if (info?.self_check) {
    try {
      const end = JSON.stringify(await window.__TAURI__.core.invoke("get_settings"));
      checks.settings_unchanged = settingsAtStart !== null && end === settingsAtStart;
      if (!checks.settings_unchanged) detail.settings = { start: settingsAtStart, end };
    } catch {
      checks.settings_unchanged = false;
    }
  }

  // P2-R10 evidence: a test run's window should be hidden and unfocused.
  detail.window = { visibility: document.visibilityState, focused: document.hasFocus() };

  const ok = Object.values(checks).every(Boolean);
  const report = Object.keys(detail).length ? { ok, checks, detail } : { ok, checks };

  try {
    await window.__TAURI__.core.invoke("self_check_report", { report });
  } catch {
    // If the backend command itself fails, there's nothing more to report.
  }

  return report;
}
