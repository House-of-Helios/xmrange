// Card kinds + module registry — a port of the approved desktop mockup
// (_Research/design/2026-09-23-dashboard-mockup/Main.dc.html).
//
// Rules this file keeps:
//  - every dynamic value that reaches HTML goes through esc();
//  - no inline style attributes (CSP style-src 'self'): widths/heights ride on
//    data-w / data-h and applyDynamics() sets them through the CSSOM after render;
//  - a card whose input has focus is never re-rendered under the user's cursor.

// ── helpers ────────────────────────────────────────────────────────────────

const ESC = { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" };

export function esc(value) {
  if (value === null || value === undefined) return "";
  return String(value).replace(/[&<>"']/g, (c) => ESC[c]);
}

export const DASH = "—";

/** Finite number or null. Guards every figure against NaN/Infinity reaching the page. */
export function num(v) {
  return typeof v === "number" && Number.isFinite(v) ? v : null;
}

/**
 * Local emoji, one flat SVG each: Fluent Emoji Flat (bundled), and the owner's own set (hoh_*,
 * drawn for XMRange; sources in assets/emoji/) on the mining, machine and tuning tabs and the
 * theme toggle.
 */
export const EMOJI = {
  volt: "high_voltage",
  herb: "herb",
  thread: "thread",
  bull: "bullseye",
  coin: "coin",
  purse: "purse",
  die: "game_die",
  watch: "stopwatch",
  label: "label",
  scroll: "scroll",
  laptop: "laptop",
  heart: "anatomical_heart",
  plug: "electric_plug",
  therm: "thermometer",
  fire: "fire",
  brain: "brain",
  turtle: "turtle",
  battery: "battery",
  compass: "compass",
  scope: "telescope",
  anchor: "anchor",
  trophy: "trophy",
  chart: "chart_increasing",
  tube: "test_tube",
  chains: "chains",
  hourglass: "hourglass_done",
  hands: "handshake",
  link: "link",
  window: "window",
  bag: "money_bag",
  cabinet: "file_cabinet",
  sat: "satellite_antenna",
  bulb: "light_bulb",
  screen: "desktop_computer",
  clock: "alarm_clock",
  sun: "sun",
  moon: "crescent_moon",
  star: "star",
  wood: "wood",
  pick: "pick",
  gear: "gear",
  // the owner's mining-tab set
  hoh_cores: "hoh_cores",
  hoh_cost: "hoh_cost",
  hoh_difficulty: "hoh_difficulty",
  hoh_earned: "hoh_earned",
  hoh_hashrate: "hoh_hashrate",
  hoh_hashwatt: "hoh_hashwatt",
  hoh_logs: "hoh_logs",
  hoh_mine: "hoh_mine",
  hoh_shares: "hoh_shares",
  hoh_uptime: "hoh_uptime",
  hoh_wallet: "hoh_wallet",
  hoh_worker: "hoh_worker",
  // the owner's machine, tuning and theme set
  hoh_chip: "hoh_chip",
  hoh_cpu: "hoh_cpu",
  hoh_display: "hoh_display",
  hoh_low_power: "hoh_low_power",
  hoh_memory: "hoh_memory",
  hoh_mode_dark: "hoh_mode_dark",
  hoh_mode_light: "hoh_mode_light",
  hoh_power: "hoh_power",
  hoh_profile_balanced: "hoh_profile_balanced",
  hoh_profile_max: "hoh_profile_max",
  hoh_profile_quiet: "hoh_profile_quiet",
  hoh_seeker: "hoh_seeker",
  hoh_source_battery: "hoh_source_battery",
  hoh_source_outlet: "hoh_source_outlet",
  hoh_temp: "hoh_temp",
  hoh_thermal: "hoh_thermal",
  hoh_trials: "hoh_trials",
};

export function emojiSrc(name) {
  return `assets/emoji/${EMOJI[name] ?? "gear"}.svg`;
}

function img(name, cls) {
  return `<img class="${esc(cls)}" src="${esc(emojiSrc(name))}" alt="" />`;
}

function pct(v) {
  const n = num(v);
  return n === null ? 0 : Math.max(0, Math.min(100, n));
}

// ── live figures ───────────────────────────────────────────────────────────

/** Earned estimate: XMR per day per h/s at the sep 23 rate. */
export const XMR_PER_HS_DAY = 7.875e-8;
export const PERIODS = ["day", "week", "month"];
export const PERIOD_MULT = { day: 1, week: 7, month: 30 };
/**
 * The window each rolling figure is averaged over (ruling P5-O2), named in its card's caption.
 * The engine computes them: h/w is xmrig's 60 s hashrate over the same 60 s of watts (hw_sys,
 * hw_cpu); earned takes xmrig's 15-minute hashrate (hs15) and cost the whole-mac watts of the
 * last 15 minutes (sys_w_900). Before a window fills, each uses the part there is.
 */
export const HW_CAP = { sys: "whole-mac watts · 60 s average", cpu: "cpu watts · 60 s average" };
export const EARNED_CAP = "xmr · 15 min average";
/** Cost's caption row is full (the period pill and the rate field), so its window rides in the row's tooltip and in screen-reader text. */
export const COST_WINDOW_NOTE = "whole-mac watts · 13 min average";
/** The sparkline keeps the last 180 hs10 values (three minutes at one snapshot a second). */
export const SERIES_MAX = 180;
/** The power chart keeps the last 780 samples (13 minutes at one snapshot a second). */
export const POWER_MAX = 780;
/** The app session's start: uptime counts from here, mining or not. */
const APP_START = Date.now();
/**
 * Both 2×2 charts fill the bottom grid row's inner box: 160 − 2·14 = 132px tall, ending on
 * the card's inner bottom, the row-2 1×1 caption line.
 */
export const CHART_H = 132;
/** The power chart's legend floats in its top 22px; the lines stay under it. */
const LEGEND_PAD = 22;
/**
 * Lines the logs window shows: seven 11px mono lines (17px each, 119px) in the full 132px
 * card height, with the 13px left over split evenly above and below.
 */
export const TAPE_LINES = 7;
/** The temperature bar runs from 90 °F to boiling. */
const TEMP_BAR_F = [90, 212];
const THERMAL_WORDS = ["nominal", "moderate", "heavy", "trapping", "sleeping"];

// ── run state: the one map both the live light and the run pills use ──────

export const RUN_STATES = ["starting", "mining", "paused", "stopped", "error"];

/** The engine's state from the latest snapshot; no snapshot means stopped. */
export function runStateOf(snap) {
  return RUN_STATES.includes(snap?.state) ? snap.state : "stopped";
}

export const isRunning = (s) => s === "starting" || s === "mining" || s === "paused";

/**
 * What a click on the live light (or the run pill) does: stopped/error → start,
 * mining → pause, paused → resume, starting → nothing yet. ⌥-click (or the stop
 * pill) stops a running engine.
 */
export function liveCommand(run, alt) {
  if (alt) return isRunning(run) ? "stop_mining" : null;
  if (run === "mining") return "pause_mining";
  if (run === "paused") return "resume_mining";
  if (run === "starting") return null;
  return "start_mining";
}

/** Snapshots in a row that must disagree with the intended value before a card says "not applied" (M2). */
export const MISMATCH_TICKS = 3;
/** What a card says when xmrig runs something other than what it shows. */
export const NOT_APPLIED = "not applied · see the logs";

export const PAUSE_TIP = "keeps the 2 GB dataset in memory · resume is immediate";
export const STOP_TIP = "stops xmrig and releases its memory";

/** °C → °F; anything not finite → null. */
export function toF(c) {
  const n = num(c);
  return n === null ? null : (n * 9) / 5 + 32;
}

/** 16.0 → "16", 12.46 → "12.5". */
function fmtGb(v) {
  const r = Math.round(v * 10) / 10;
  return Number.isInteger(r) ? String(r) : r.toFixed(1);
}

/** 1826.6 → "1,827"; anything not finite → "—". */
export function fmtInt(v) {
  const n = num(v);
  return n === null ? DASH : Math.round(n).toLocaleString("en-US");
}

/** h/w: whole numbers from 100 up, one decimal below. */
function fmtHw(v) {
  const n = num(v);
  if (n === null) return DASH;
  return n >= 100 ? fmtInt(n) : n.toFixed(1);
}

/** Rows the trials card lists (the approved mockup shows six). */
export const TRIALS_SHOWN = 6;

/**
 * The trials card from `list_trials`: every measured setup, best first by the h/w the
 * "h/w counts" toggle chose.
 */
export function trialsView(t, measure) {
  const rows = (t?.rows ?? [])
    .map((r) => ({
      label: r.label,
      v: num(measure === "cpu" ? r.hw_cpu : r.hw_sys),
      threads: num(r.threads),
      e_core_clamp: !!r.e_core_clamp,
      utility: !!r.utility,
      low_power: r.low_power === true || r.low_power === false ? r.low_power : null,
    }))
    .filter((r) => r.v !== null)
    .sort((a, b) => b.v - a.v)
    .slice(0, TRIALS_SHOWN);
  const best = rows.length ? 0 : -1;
  return {
    value: t?.count ? fmtInt(t.count) : DASH,
    sub: "most efficient h/w",
    rows: rows.map((r, i) => ({
      label: r.label,
      hw: fmtHw(r.v),
      best: i === best,
      threads: r.threads,
      e_core_clamp: r.e_core_clamp,
      utility: r.utility,
      low_power: r.low_power,
    })),
  };
}

/** "5 cores · utility · low power mode", "4 e-cores": a setup in the trials card's words. One
 * word for one thing (ASD-STE100): the screen counts cores; xmrig runs one thread on each. */
export function setupLabel(s, lowPower) {
  if (!s || typeof s !== "object") return DASH;
  const n = num(s.threads);
  const word = s.e_core_clamp ? "e-core" : "core";
  let t = `${n ?? DASH} ${word}${n === 1 ? "" : "s"}`;
  if (s.utility) t += " · utility";
  if (lowPower === true) t += " · low power mode";
  else if (lowPower === false) t += " · full power";
  return t;
}

/**
 * Characters a single-line caption holds with room to spare (the self-check proves it at the
 * limit): a 1×1-aligned caption (12.5px in 180px) and a half's caption (11.5px in 196px).
 */
const CAP_CHARS = 34;
const HALF_CAP_CHARS = 40;

/**
 * A setup label that fits `max` characters: "low power mode" becomes "low power", then "e-cores"
 * goes, then the power mode. The full label rides in the tooltip whenever anything was cut.
 */
export function setupFit(s, lowPower, max) {
  const full = setupLabel(s, lowPower);
  if (!s || typeof s !== "object" || full.length <= max) return { text: full, tip: "" };
  const lean = [full.replace(" · low power mode", " · low power"), setupLabel({ ...s, e_core_clamp: false }, lowPower), setupLabel({ ...s, e_core_clamp: false }, null)];
  return { text: lean.find((t) => t.length <= max) ?? lean[lean.length - 1], tip: full };
}

/**
 * Backend free text (an optimizer note, a refusal) that fits `max` characters: it keeps as many
 * leading " · " parts as fit, else cuts at a word with "…". The full text rides in the tooltip
 * whenever anything was cut (P4-C21).
 */
export function fitText(t, max) {
  const full = typeof t === "string" ? t : "";
  if (full.length <= max) return { text: full, tip: "" };
  const parts = full.split(" · ");
  for (let n = parts.length - 1; n >= 1; n--) {
    const head = parts.slice(0, n).join(" · ");
    if (head.length <= max) return { text: head, tip: full };
  }
  const cut = full.slice(0, max - 1);
  const sp = cut.lastIndexOf(" ");
  return { text: `${(sp > max / 2 ? cut.slice(0, sp) : cut).trimEnd()}…`, tip: full };
}

/** The threads the rig runs: the optimizer's target while holding, else the user's own count. */
/** Whether the rig runs on the E-cores only: the held setup's clamp, else the user's own choice. */
export function shownClamp(state) {
  const opt = state?.snap?.optimizer;
  return opt?.mode === "hold" && opt?.target ? opt.target.e_core_clamp === true : state?.settings?.e_cores_only === true;
}

export function shownThreads(state) {
  const opt = state?.snap?.optimizer;
  return opt?.mode === "hold" && opt?.target ? num(opt.target.threads) : num(state?.settings?.threads);
}

/** The throttle picks for the chosen denominator, from list_trials (null before any trials). */
export function picksFor(trials, measure) {
  return trials?.picks?.[measure === "cpu" ? "cpu" : "sys"] ?? {};
}

/** "5 cores · utility", "4 e-cores · low power": a pick's setup in a pill blurb's few characters (the full words ride in its tooltip). */
function shortSetup(p) {
  const n = num(p?.setup?.threads);
  let t = `${n ?? DASH} ${p?.setup?.e_core_clamp ? "e-core" : "core"}${n === 1 ? "" : "s"}`;
  if (p?.setup?.utility) t += " · utility";
  return t; // Low Power Mode shows as the turtle beside it (profileView's `lpm`), as on the trials rows
}

/**
 * The held setup, as a caption line under the throttle card ("holding: 5 threads · low power
 * mode", or "manual"): what the keel half used to say, moved onto the throttle card (P5).
 */
/** The profile card's second line: only what the first line can't say (owner, 2026-10-04: a
 * "holding: <setup>" line repeated the setup above it and could show a stale one). Holding a
 * profile, the first line already names what runs, so this line is empty. */
function holdingCaption(opt, settings, running) {
  const mode = opt?.mode ?? settings?.optimizer?.mode ?? "manual";
  if (mode !== "hold") return { text: "manual · your own cores", tip: "a profile pill or a trials row sets the cores again" };
  if (opt?.sweep) return { text: "trials running", tip: "" };
  return { text: "", tip: "" };
}

/**
 * throttle card: light · efficient · max. Once trials exist each pill's blurb is its chosen setup
 * and h/w (full label in the tooltip); before any, the pills are disabled and the caption says so.
 * A second caption line says what's actually held right now, or "manual" (P5: moved off the
 * removed keel half).
 */
export function profileView(opt, settings, picks, measure, running) {
  const id = opt?.profile ?? settings?.optimizer?.profile ?? "efficient";
  const selected = picks[id] ?? null;
  const fit = selected ? setupFit(selected.setup, selected.low_power, CAP_CHARS) : { text: "run trials first", tip: "" };
  const holding = holdingCaption(opt, settings, running);
  return {
    value: PROFILES.find((p) => p.id === id)?.label ?? DASH,
    sub: fit.text,
    subTip: fit.tip,
    sub2: holding.text,
    sub2Tip: holding.tip,
    selected: id,
    choices: PROFILES.map((c) => {
      const p = picks[c.id];
      if (!p) return { ...c, disabled: true };
      return { ...c, blurb: `${fmtHw(p.hw)} h/w`, blurb2: shortSetup(p), lpm: p.low_power === true, tip: `${setupLabel(p.setup, p.low_power)}${PROFILE_TIPS[c.id] ?? ""}` };
    }),
  };
}

/** What each throttle takes of the Mac, after its setup in the pill's tooltip (owner, 2026-10-03). */
const PROFILE_TIPS = {
  light: " · quiet: mines on the efficiency cores at background priority. Other apps get the cpu first.",
  efficient: " · balanced: uses about half of the cores. The other cores stay free for you.",
  max: " · max: uses all of the cores for the most hashes.",
};

/** Rows the trials progress panel's log shows (the approved mockup keeps this small). */
export const TRIAL_LOG_LINES = 8;

/**
 * The trials progress panel (P5, replaced the optimizer/keel halves): live setup under test,
 * stage + elapsed, "trial N of M", and an estimated time per trial and total remaining, from the
 * settle + measure limits and this session's own completed trials. The button always works:
 * stopped, "start trials" / "continue trials" start mining first (`trialsGo`).
 */
export function trialsProgressView(opt, settings, run, progress, tape, lowPower = null, fallbackPlanned = 0) {
  const live = run === "mining" || run === "paused" || run === "starting";
  // A sweep only exists while the engine runs; a stale snapshot after stop never shows one.
  const s = live ? opt?.sweep ?? null : null;
  const ask = live ? opt?.ask_low_power : null;
  const note = live && typeof opt?.note === "string" && opt.note ? fitText(opt.note, HALF_CAP_CHARS) : null;
  // bar: the current trial's progress, 0–100, the same bar as the temp and cpu cards.
  let value, sub, tip = "", bar = 0, buttons;
  if (ask === true || ask === false) {
    value = "low power mode";
    sub = `turn it ${ask ? "on" : "off"} in battery settings`;
    buttons = [{ act: "trials-skip", label: "skip" }, { act: "trials-stop", label: "stop" }];
  } else if (s) {
    const done = num(s.done) ?? 0;
    const planned = num(s.planned) ?? 0;
    value = `trial ${Math.min(done + 1, planned)} of ${planned}`;
    buttons = [run === "paused" ? { act: "trials-resume", label: "continue" } : { act: "trials-pause", label: "pause" }, { act: "trials-stop", label: "stop" }];
    if (run === "paused") {
      sub = "paused";
    } else if (s.testing) {
      // Stage, elapsed time and an ETA from this session's own completed trials, never hard-coded.
      // Every trial is a fixed length (trial_s: warm-up + measure), so both figures are exact.
      const left = num(s.left_s) ?? 0;
      const trialS = num(s.trial_s) ?? 0;
      const remainingS = Math.max(0, planned - done - 1) * trialS + left;
      bar = trialS > 0 ? Math.min((trialS - left) / trialS, 0.99) * 100 : 0;
      // "4 cores - 08:20 mins left"; the sweep's total rides in the tooltip.
      sub = `${trialLabelHtml(s.testing).replace(/<[^>]*>/g, "").trim()} - ${fmtCountdown(left)} left`;
      tip = `${fmtSpan(remainingS)} left for all trials`;
    } else {
      sub = note?.text ?? "between trials";
      tip = note?.tip ?? "";
    }
  } else {
    // The value is always the next trial ("trial 1 of 12" before any), never "ready".
    const [done, planned] = Array.isArray(progress) ? progress.map((n) => num(n) ?? 0) : [0, fallbackPlanned];
    if (planned > 0 && done >= planned) {
      value = `trial ${planned} of ${planned}`;
      sub = "all trials done";
      bar = 100;
      buttons = [{ act: "trials-go", label: "run trials again" }];
    } else if (done > 0) {
      value = `trial ${done + 1} of ${planned}`;
      sub = `${done} of ${planned} trials done`;
      buttons = [{ act: "trials-go", label: "continue trials" }];
    } else {
      value = planned > 0 ? `trial 1 of ${planned}` : "trial 1";
      sub = "tests each setup on this mac";
      buttons = [{ act: "trials-go", label: "start trials" }];
    }
  }
  const log = (tape || []).filter((l) => l && l.tag === "trial").slice(-TRIAL_LOG_LINES);
  // The Low Power Mode icon beside the seeker's value whenever the Mac is in Low Power Mode.
  const turtle = lowPower === true;
  return { value, sub, tip, buttons, button: buttons[0].label, log, bar, turtle };
}

const ordinal = (k) => `${k}${k % 100 >= 11 && k % 100 <= 13 ? "th" : { 1: "st", 2: "nd", 3: "rd" }[k % 10] ?? "th"}`;

/** thread curve 2×2 from list_trials' curve: H/s by thread count, the running count in fill. */
export function curveView(c, maxThreads) {
  const pts = Array.isArray(c?.points) ? c.points.filter((p) => num(p?.hs) !== null) : [];
  if (!pts.length) return { maxThreads, bars: null };
  const last = Math.max(...pts.map((p) => p.threads));
  const top = Math.max(...pts.map((p) => p.hs), 1);
  // The highlighted bar is the one the caption names: the core that added the most h/s per watt.
  const st = Array.isArray(c.standout) ? c.standout : null;
  const bars = Array.from({ length: Math.max(maxThreads, last) }, (_, i) => {
    const p = pts.find((q) => q.threads === i + 1);
    return { n: String(i + 1), hs: p ? fmtInt(p.hs) : DASH, h: p ? Math.round((p.hs / top) * 160) : 4, best: !!st && i + 1 === num(st[0]) };
  });
  const mode = c.low_power === true ? " · low power mode" : c.low_power === false ? " · full power" : "";
  const flat = num(c.flat_from);
  return {
    maxThreads,
    bars,
    cap: `best h/s added per watt${mode}`,
    foot: st ? `the ${ordinal(st[0])} core added +${fmtInt(st[1])} h/s for +${Number(st[2]).toFixed(2)} w` : "",
    footMuted: flat === null || flat >= last ? "" : flat + 1 === last ? `core ${last} adds less than 2%` : `cores ${flat + 1}–${last} add less than 2%`,
  };
}

/**
 * p2pool's cards: the node from node_status (polled, mining or not), everything else from p2pool's
 * own figures in the snapshot (`snap.p2pool`), so each value has one source. `tor`: this session's
 * route while mining, else the saved setting.
 */
export function p2poolView(node, p, tor, payouts, hs, own = false) {
  const n = node?.state;
  const pct = n === "syncing" && num(node.target) ? Math.floor((num(node.height) / num(node.target)) * 100) : null;
  // XMRange's own node: the keeper's states, then the same probe as the gui's node.
  const ownNode =
    n === "no_folder" ? { value: "no folder", sub: "choose its chain folder in settings" }
    : n === "off" ? { value: "off", sub: "starts when p2pool is selected" }
    : n === "starting" ? { value: "starting", sub: "opening the chain" }
    : n === "stopping" ? { value: "stopping", sub: "closing the chain · up to 2 min" }
    : n === "failed" ? { value: "stopped", sub: String(node.why ?? "") }
    : n === "missing" ? { value: "not running", sub: "it runs while p2pool is selected" }
    : n === "no_zmq" ? { value: "starting", sub: "opening the chain" }
    : n === "syncing" && !num(node.target) ? { value: "syncing", sub: `finding peers · ${tor ? "through tor" : "direct"}` }
    : n === "syncing" ? { value: pct === null ? "syncing" : `${pct}%`, sub: `block ${fmtInt(node.height)} of ${fmtInt(node.target)} · p2pool starts when done` }
    : null;
  const peers = num(p?.peers);
  const route = tor ? "through tor" : "direct";
  // Gupaxx's "your p2pool share mean": hashes one share takes ÷ this Mac's hashrate.
  const shareDiff = num(p?.share_difficulty);
  const shareS = shareDiff !== null && hs ? shareDiff / hs : null;
  const count = num(payouts?.count);
  const atomic = num(payouts?.atomic);
  const paid = count === null ? DASH : count === 1 ? "1 payout" : `${fmtInt(count)} payouts`;
  return {
    node:
      own && ownNode ? ownNode
      : n === "ready" ? { value: "ready", sub: peers === null ? `block ${fmtInt(node.height)}` : `${fmtInt(peers)} peers · ${route}` }
      : n === "syncing" ? { value: pct === null ? "syncing" : `${pct}%`, sub: "syncing · p2pool starts when done" }
      : n === "no_zmq" ? { value: "needs zmq", sub: "add one startup flag · see the guide" }
      : n === "missing" ? { value: "not found", sub: "open the monero gui" }
      : { value: DASH },
    payouts: {
      value: atomic === null ? DASH : fmtXmr(atomic / 1e12),
      sub: shareS === null ? `${paid} · all time` : `${paid} · 1 share per ~${fmtWait(shareS)}`,
      subTip: shareS === null ? "" : SHARE_WAIT_TIP,
    },
  };
}

/** Money to the cents, or to a tenth of a cent below one: 0.0472 → "0.047", 1.234 → "1.23". */
export function fmtMoney(x) {
  return x < 1 ? x.toFixed(3) : x.toFixed(2);
}

/** XMR to the digits that matter: 0 → "0", 0.000412345 → "0.000412", 1.25 → "1.2500". */
export function fmtXmr(x) {
  if (x === 0) return "0";
  return x < 0.001 ? x.toFixed(6) : x.toFixed(4);
}

/** An average wait, rounded the way people say it: "~45 min", "~27 h", "~3 days". */
export function fmtWait(s) {
  const h = s / 3600;
  if (h < 1) return `${Math.max(1, Math.round(s / 60))} min`;
  if (h < 48) return `${Math.round(h)} h`;
  return `${Math.round(h / 24)} days`;
}

/** The payouts card's tooltips. */
const PAYOUTS_TIP =
  "xmr that p2pool paid to this wallet while xmrange ran. the encrypted ledger keeps the record. p2pool pays your wallet directly when it finds a monero block. each of your shares counts for about 18 hours. there is no minimum and no withdrawal. you can spend a payout after 60 blocks (about 2 hours).";
const SHARE_WAIT_TIP = "average time for this mac to find one p2pool share at its current hashrate. each share earns part of every block that p2pool finds in the next 18 hours.";

/** 8040 → "2 h 14 m", 2760 → "46 m", 38 → "38 s". */
/** A log line's time as HH:MM; the tape keeps HH:MM:SS for the ETA math. */
const hm = (ts) => (/^\d{2}:\d{2}:\d{2}$/.test(ts || "") ? ts.slice(0, 5) : ts || "");

const pad2 = (n) => String(n).padStart(2, "0");

/** A live countdown: "08:20 mins" / "00:45 secs" under an hour, "1hr 23 mins" from an hour up. */
export function fmtCountdown(s) {
  const t = Math.max(0, Math.ceil(num(s) ?? 0));
  if (t >= 3600) return `${Math.floor(t / 3600)}hr ${Math.floor((t % 3600) / 60)} mins`;
  return `${pad2(Math.floor(t / 60))}:${pad2(t % 60)} ${t >= 60 ? "mins" : "secs"}`;
}

/** An estimate's span, minutes only: "42 mins", "1hr 5 mins". */
function fmtSpan(s) {
  const m = Math.max(0, Math.round((num(s) ?? 0) / 60));
  return m >= 60 ? `${Math.floor(m / 60)}hr ${m % 60} mins` : `${m} mins`;
}

export function fmtUptime(s) {
  const n = num(s);
  if (n === null || n < 0) return DASH;
  const t = Math.floor(n);
  if (t < 60) return `${t} s`;
  const m = Math.floor(t / 60);
  if (m < 60) return `${m} m`;
  return `${Math.floor(m / 60)} h ${m % 60} m`;
}

/** Wallet edit hint, as in the mockup; a backend error replaces it verbatim. */
export function walletHint(draft, error) {
  if (error) return { hint: String(error), hintState: "bad" };
  const d = draft || "";
  if (!d) return { hint: "95 characters · starts with 4 or 8", hintState: "" };
  if (d.length === 95 && /^[48]/.test(d)) return { hint: "format is correct", hintState: "good" };
  return { hint: `${d.length} / 95 characters`, hintState: "" };
}

/** y-range for the sparkline: the line rides in the upper part of the chart, as in the mockup. */
function sparkRange(series) {
  if (!series.length) return { lo: 0, hi: 1 };
  const max = Math.max(...series);
  const min = Math.min(...series);
  const span = Math.max(max - min, max * 0.03, 1);
  return { lo: Math.max(0, min - span * 1.2), hi: max + span * 0.3 };
}

/**
 * Port of the mockup's line(): an SVG path (and its filled area) through vals.
 * x is the sample's index over the whole array, so a missing sample (null)
 * leaves a gap in place instead of shifting the rest of the line: the line
 * breaks into subpaths there, and two series of the same length stay aligned.
 * The line runs from 8px above the bottom up to `top` px below the chart's top.
 */
export function linePath(vals, lo, hi, W, H, top = 12) {
  const all = vals || [];
  const n = all.length;
  if (n < 2 || !(hi > lo)) return null;
  const segs = [];
  let cur = null;
  all.forEach((v, i) => {
    const y = num(v);
    if (y === null) {
      cur = null;
      return;
    }
    const c = Math.max(lo, Math.min(hi, y));
    const p = [(i / (n - 1)) * (W - 4) + 2, H - 8 - ((c - lo) / (hi - lo)) * (H - 8 - top)];
    if (!cur) segs.push((cur = []));
    cur.push(p);
  });
  if (segs.reduce((k, sg) => k + sg.length, 0) < 2) return null;
  const f = (x) => x.toFixed(1);
  const d = segs.map((sg) => sg.map((p, i) => (i ? "L" : "M") + f(p[0]) + " " + f(p[1])).join(" ")).join(" ");
  const area = segs
    .filter((sg) => sg.length > 1)
    .map((sg) => `${sg.map((p, i) => (i ? "L" : "M") + f(p[0]) + " " + f(p[1])).join(" ")} L${f(sg[sg.length - 1][0])} ${H} L${f(sg[0][0])} ${H} Z`)
    .join(" ");
  return { d, area };
}

// ── module registry (desktop board, grid order = mockup order) ─────────────
//
// kind decides the renderer; cs/rs are column/row spans on the 4-column grid.

/** Tooltips that say what a figure counts (every H/W figure names its denominator). */
const CPU_BUSY_TIP = "busy cores on the whole mac, not only for xmrig";

/**
 * The low power card's gain, never a hard-coded figure (Helios: "we don't want to hard code my
 * own computer into the app"): the efficient profile's own H/W, this mac's own trials, Low Power
 * Mode off vs on, from `list_trials`' `low_power_gain`. `null` (either denominator not yet
 * measured in both states) means the card falls back to "run trials to measure".
 */
export function lowPowerGainView(trials) {
  const g = trials?.low_power_gain || {};
  const row = (key, what) => {
    const x = g[key];
    const off = num(x?.off);
    const on = num(x?.on);
    if (off === null || on === null) return null;
    const p = num(x?.pct);
    return { what, off: fmtHw(off), on: fmtHw(on), gain: p === null ? DASH : `${p >= 0 ? "+" : ""}${Math.round(p)}%` };
  };
  const rows = [row("sys", "whole mac"), row("cpu", "cpu only")].filter(Boolean);
  return { ready: rows.length > 0, rows };
}

/** The low power walkthrough's one line of "why": the same measured gain, or no number at all. */
export function lowPowerGuideText(trials) {
  const g = lowPowerGainView(trials);
  if (!g.ready) return "low power mode changes how your mac runs cpu work. run trials with it on and off to see its effect on your h/w.";
  return "low power mode off vs on, from your balanced profile's trials: " + g.rows.map((r) => `${r.what} h/w ${r.off} → ${r.on} (${r.gain})`).join(" · ");
}

/** The low power card's corner pill: opens the guide (Plan 5 hooks its walkthrough in by this topic). */
const LOW_POWER_PILL = { act: "guide", val: "lowpower", glyph: "arrow", label: "about low power mode" };
const NODE_PILL = { act: "guide", val: "node", glyph: "arrow", label: "set up your monero node" };

const stat = (id, title, icon, sub, extra = {}) => ({ id, kind: "stat", title, icon, sub, cs: 1, rs: 1, ...extra });
const mod = (id, kind, title, icon, cs, rs, extra = {}) => ({ id, kind, title, icon, cs, rs, ...extra });

export const MODULES = {
  // moneroocean · mining — 11 modules in the approved final mockup.
  mining_mo: [
    // The owner's icons throughout. "threads" keeps its id; its title is "cores" (one xmrig thread per core).
    mod("hashrate", "spark", "hashrate", "hoh_hashrate", 2, 2, { icon2: "hoh_mine" }),
    stat("hw", "h/w", "hoh_hashwatt", HW_CAP.sys),
    mod("threads", "threads", "cores", "hoh_cores", 1, 1),
    mod("earned", "earned", "earned", "hoh_earned", 1, 1),
    mod("cost", "cost", "cost", "hoh_cost", 1, 1),
    mod("wallet", "wallet", "wallet", "hoh_wallet", 1, 1),
    mod("worker", "worker", "worker", "hoh_worker", 1, 1),
    stat("uptime", "uptime", "hoh_uptime", "this session"),
    stat("shares", "shares", "hoh_shares", "accepted / rejected"),
    mod("tape", "tape", "logs", "hoh_logs", 3, 1),
    stat("difficulty", "difficulty", "hoh_difficulty", "set by the pool"),
  ],
  // p2pool · mining (stage 2): moneroocean's four rows. The node takes the shares card's place and
  // payouts take difficulty's; earned is the expected solo-average from p2pool's own network
  // figures (owner, 2026-10-02: the figures Gupaxx's calculator shows, fewer cards).
  mining_p2: [
    mod("hashrate", "spark", "hashrate", "hoh_hashrate", 2, 2, { icon2: "hoh_mine" }),
    stat("hw", "h/w", "hoh_hashwatt", HW_CAP.sys),
    mod("threads", "threads", "cores", "hoh_cores", 1, 1),
    mod("earned", "earned", "earned", "hoh_earned", 1, 1),
    mod("cost", "cost", "cost", "hoh_cost", 1, 1),
    mod("wallet", "wallet", "wallet", "hoh_wallet", 1, 1),
    mod("worker", "worker", "worker", "hoh_worker", 1, 1),
    stat("uptime", "uptime", "hoh_uptime", "this session"),
    stat("node", "node", "cabinet", "monero gui · this mac", { pill: NODE_PILL }),
    mod("tape", "tape", "logs", "hoh_logs", 3, 1),
    stat("payouts", "payouts", "bag", "all time", { valueTip: PAYOUTS_TIP }),
  ],
  // The owner's icons throughout ("awake" keeps its clock: the owner's set has none for it).
  // No display card: macOS never reported a brightness change back (Helios, 2026-09-27). The
  // 1×1s under the power card keep its two columns centred over them; low power takes the
  // freed cell as a 2×1 (lever), its measured gain written out beside it.
  machine: [
    mod("power", "power", "power", "hoh_power", 2, 2, { title2: "source" }),
    stat("chip", "chip", "hoh_chip", DASH),
    stat("cpu", "cpu", "hoh_cpu", "p — · e — mhz", { bar: true, valueTip: CPU_BUSY_TIP }),
    stat("temperature", "temperature", "hoh_temp", "cpu average", { bar: true }),
    mod("thermal", "thermal", "thermal", "hoh_thermal", 1, 1),
    stat("memory", "memory", "hoh_memory", "dataset uses 2.3 gb", { bar: true }),
    stat("awake", "awake", "clock", "mac stays awake while mining"),
    mod("lowpower", "lever", "low power", "hoh_low_power", 2, 1, { pill: LOW_POWER_PILL }),
  ],
  // trials progress and the throttles selected from what's measured.
  tuning: [
    mod("profile", "joined", "profile", "compass", 2, 1, { right: "choices" }),
    mod("optimizer", "progress", "seeker", "hoh_seeker", 2, 1, {}),
    mod("curve", "curve", "core curve", "chart", 2, 2),
    mod("trials", "joined", "trials", "hoh_trials", 2, 1, { right: "list" }),
    stat("lowpower", "low power", "hoh_low_power", "", { pill: LOW_POWER_PILL }),
    // h/w counts: whole mac or cpu only, the power card's question, so the power card's icon.
    mod("measure", "measure", "watts counted", "hoh_power", 1, 1),
  ],
};

/** `lowPower`: false on a Mac without Low Power Mode (app_info), which never shows its cards. */
export function modulesFor(tab, pool, lowPower = true) {
  const mods = tab === "machine" ? MODULES.machine : tab === "tuning" ? MODULES.tuning : pool === "p2" ? MODULES.mining_p2 : MODULES.mining_mo;
  return lowPower ? mods : mods.filter((m) => m.id !== "lowpower");
}

// ── view model ─────────────────────────────────────────────────────────────
//
// buildView() turns app state into display strings: settings and app_info, the
// latest snapshot (mining and machine tabs), the last known context (it rides
// on one snapshot in 30), and the UI's own edit state. Any figure that is
// missing or not finite shows "—".

/** The three throttles (Helios, 2026-09-28): light · efficient · max, each with a 2–3 word blurb. */
export const PROFILES = [
  { id: "light", label: "quiet", blurb: "background e-cores", icon: "hoh_profile_quiet" },
  { id: "efficient", label: "balanced", blurb: "half the cores", icon: "hoh_profile_balanced" },
  { id: "max", label: "max", blurb: "all cores", icon: "hoh_profile_max" },
];
/** The default wallet's disclosure (the backend's label); it stays on the wallet card, word for word. */
const DEFAULT_WALLET_NOTE = "default · ghostdrip dev wallet";

export function buildView(state) {
  const settings = state.settings || {};
  const info = state.info || {};
  const machine = info.machine || null;
  const maxThreads = num(info.max_threads) ?? 8;
  const threads = shownThreads(state);
  const measure = settings.measure === "cpu" ? "cpu" : "sys";
  const period = PERIODS.includes(settings.period) ? settings.period : "day";
  const mult = PERIOD_MULT[period];
  const currency = typeof settings.currency === "string" && settings.currency ? settings.currency : "$";
  const rate = num(settings.rate_per_kwh);
  const wallet = info.wallet || null;
  const ui = state.ui || {};
  const snap = state.snap || null;
  const series = Array.isArray(state.series) ? state.series : [];

  // Live figures from the latest snapshot. The machine tab also takes the idle
  // sampler's figures (xmrange://idle) while no engine sends any (I4).
  const hs10 = num(snap?.hs10);
  const power = snap?.power || null;
  const machinePower = power || (isRunning(runStateOf(snap)) ? null : state.idlePower || null);
  // The rolling figures (see HW_CAP): earned on the longest hashrate average xmrig has, up to 15 min.
  const hsEarned = num(snap?.hs15) ?? num(snap?.hs60) ?? hs10;
  // Cost is the Mac's electricity, mining or not: the mean whole-mac watts of the power chart's
  // own series (one slot per second, both samplers feed it), the same numbers the chart draws.
  const ws = (Array.isArray(state.powerSeries) ? state.powerSeries : []).map((p) => num(p?.w)).filter((w) => w !== null);
  const sysW = ws.length ? ws.reduce((a, b) => a + b, 0) / ws.length : null;
  const hw = measure === "cpu" ? snap?.hw_cpu : snap?.hw_sys;
  // num() again on the results: whatever the inputs, "$Infinity" or NaN never reaches the page.
  // p2pool: the expected solo-mining average from p2pool's own network figures (p2pool pays it, no
  // fee): h/s × 86,400 s × block reward ÷ monero difficulty. Never moneroocean's rate.
  const p2 = state.pool === "p2";
  const net = snap?.p2pool;
  const xmrPerHsDay = p2
    ? num(net?.net_reward) !== null && num(net?.net_difficulty) ? (num(net.net_reward) / 1e12) * 86400 / num(net.net_difficulty) : null
    : XMR_PER_HS_DAY;
  const earned = hsEarned === null || xmrPerHsDay === null ? null : num(hsEarned * xmrPerHsDay * mult);
  const price = settings.price_feed && currency === "$" ? num(state.price?.usd) : num(settings.xmr_price);
  const fiat = earned === null || price === null ? null : num(earned * price);
  const cost = sysW === null || rate === null ? null : num(((sysW * 24) / 1000) * rate * mult);
  const good = num(snap?.shares_good);
  const rejected = num(snap?.shares_rejected);
  const diff = num(snap?.diff);

  const isDefault = wallet ? !!wallet.is_default : true;
  const draft = typeof ui.walletDraft === "string" ? ui.walletDraft : "";
  const workerError = ui.workerError || null;

  let split = DASH;
  const clamp = shownClamp(state);
  // An Intel Mac: no E-cores, and with Hyper-Threading two threads per core.
  const intel = !!machine && num(machine.e_cores) === 0;
  const intelCores = intel ? `${num(machine.p_cores) ?? DASH} cores · ${maxThreads} threads` : null;
  if (intel) {
    split = intelCores;
  } else if (machine && threads !== null) {
    const p = Math.min(threads, num(machine.p_cores) ?? threads);
    const e = Math.max(0, threads - p);
    split = clamp ? `${threads} efficiency · background` : `${p} performance` + (e ? ` · ${e} efficiency` : "");
  }

  // The core split; memory has its own card.
  const chip = machine
    ? {
        value: String(machine.chip || DASH).toLowerCase(),
        sub: intelCores ?? `${num(machine.p_cores) ?? DASH} performance + ${num(machine.e_cores) ?? DASH} efficiency`,
      }
    : { value: DASH, sub: DASH };

  // ── machine tab: the power sample, the last known context, app_info.machine
  const run = runStateOf(snap);
  const ctx = state.context || null;
  const P = num(machine?.p_cores);
  const E = num(machine?.e_cores);
  const mp = machinePower;
  const pAct = num(mp?.p_active);
  const eAct = num(mp?.e_active);
  const pMhz = num(mp?.p_mhz);
  const eMhz = num(mp?.e_mhz);
  let cpu = { value: DASH, sub: `p ${DASH} · e ${DASH} mhz`, bar: 0 };
  if (intel && mp && pAct !== null) {
    // Intel: the whole CPU's busy share over its threads; no clock reading without root.
    const raw = pAct * maxThreads;
    cpu = { value: `${Math.max(0, Math.min(maxThreads, Math.round(raw)))} of ${maxThreads} busy`, sub: intelCores, bar: pAct * 100 };
  } else if (mp && P !== null && E !== null && P + E > 0 && pAct !== null && eAct !== null) {
    const raw = pAct * P + eAct * E;
    const busyCores = Math.max(0, Math.min(P + E, Math.round(raw)));
    cpu = { value: `${busyCores} of ${P + E} busy`, sub: `p ${fmtInt(pMhz)} · e ${fmtInt(eMhz)} mhz`, bar: (raw / (P + E)) * 100 };
  }
  const tempF = toF(mp?.temp_c);
  const memUsed = num(mp?.mem_used_gb);
  const memTotal = num(mp?.mem_total_gb);
  const thermalIdx = THERMAL_WORDS.indexOf(ctx?.thermal);
  const lowPower = state.lowPower ?? null;
  const battery = num(ctx?.battery_pct);
  // Source is one word, outlet or battery (Helios, 2026-09-27), with its own icon; the caption
  // says "plugged in" on the outlet and the charge on battery.
  let source = DASH;
  let sourceSub = "power source";
  let sourceIcon = "hoh_source_outlet";
  if (ctx?.on_ac === true) {
    source = "outlet";
    sourceSub = "plugged in";
  } else if (ctx?.on_ac === false) {
    source = "battery";
    sourceSub = battery === null ? "charge unknown" : `${Math.round(battery)}% left`;
    sourceIcon = "hoh_source_battery";
  }
  const machineW = num(mp?.sys_w);
  const cpuW = num(mp?.cpu_w);
  const ramW = num(mp?.ram_w);
  const samples = Array.isArray(state.powerSeries) ? state.powerSeries : [];
  const watts = samples.map((s) => num(s?.w));
  const temps = samples.map((s) => num(s?.f));
  const wVals = watts.filter((v) => v !== null);
  const tVals = temps.filter((v) => v !== null);

  // ── tuning: a refused optimizer command (P4-C19) shows its reason, verbatim, in place of the
  // caption of the card it came from, until the next command or start.
  const picks = picksFor(state.trials, measure);
  const hasTrials = Object.values(picks).some(Boolean);
  const profile = profileView(snap?.optimizer, settings, picks, measure, isRunning(run));
  // Before any sweep: every thread count plus the E-core-only ones (the sweep's own plan).
  const fallbackPlanned = (num(info?.max_threads) ?? 0) + (num(info?.machine?.e_cores) ?? 0);
  const optimizer = trialsProgressView(snap?.optimizer, settings, run, state.trials?.progress, state.tape, lowPower, fallbackPlanned);
  const optErr = typeof ui.optError?.msg === "string" ? ui.optError : null;
  if (optErr) {
    const fit = fitText(optErr.msg, optErr.at === "profile" ? CAP_CHARS : HALF_CAP_CHARS);
    if (optErr.at === "profile") Object.assign(profile, { sub: fit.text, subTip: fit.tip });
    else if (optErr.at === "seeker") Object.assign(optimizer, { sub: fit.text, tip: fit.tip });
  }

  // ── run pills on the hashrate card: the same map as the live light
  const busy = !!state.busy;
  const primaryCmd = liveCommand(run, false);
  const runPills = {
    primary: {
      glyph: run === "mining" ? "pause" : "play",
      label: run === "mining" ? "pause" : run === "paused" ? "resume" : "start",
      aria: run === "mining" ? "pause mining" : run === "paused" ? "resume mining" : "start mining",
      title: run === "mining" ? PAUSE_TIP : "",
      disabled: busy || primaryCmd === null,
    },
    stop: { disabled: busy || liveCommand(run, true) === null },
  };

  const lpGain = lowPowerGainView(state.trials);
  const lpGainTip = lpGain.ready ? lpGain.rows.map((r) => `${r.what} h/w ${r.off} → ${r.on} (${r.gain})`).join(" · ") : "";
  const p2v = p2poolView(state.node, snap?.p2pool, isRunning(run) ? snap?.tor === true : settings.tor === true, state.payouts, hsEarned, settings?.node === "own");

  return {
    stats: {
      ...p2v,
      hw: { value: fmtHw(hw), sub: HW_CAP[measure] },
      difficulty: { value: diff === null || diff <= 0 ? DASH : fmtInt(diff) },
      uptime: { value: fmtUptime((Date.now() - APP_START) / 1000) },
      shares: { value: good === null || rejected === null ? DASH : `${fmtInt(good)} / ${fmtInt(rejected)}` },
      chip,
      cpu,
      temperature: { value: tempF === null ? DASH : `${Math.round(tempF)}°f`, bar: tempF === null ? 0 : ((tempF - TEMP_BAR_F[0]) / (TEMP_BAR_F[1] - TEMP_BAR_F[0])) * 100 },
      memory: {
        value: memUsed === null || memTotal === null ? DASH : `${fmtGb(memUsed)} / ${fmtGb(memTotal)} gb`,
        bar: memUsed === null || !memTotal ? 0 : (memUsed / memTotal) * 100,
      },
      lowpower: {
        value: lowPower === true ? "on" : lowPower === false ? "off" : DASH,
        sub: lpGain.ready ? "off → on · balanced profile" : "run trials to measure",
        subTip: lpGainTip,
      },
      awake: { value: isRunning(run) ? "kept awake" : "normal sleep" },
    },
    run,
    runPills,
    // I1: an engine that ended in an error says so, with the backend's reason verbatim, until the next start.
    hashrate:
      run === "error"
        ? { value: "error", sub: String(snap?.error || "xmrig stopped"), isError: true, series, ...sparkRange(series) }
        : run === "paused" && snap?.guard_note
        ? { value: "paused", sub: String(snap.guard_note), series, ...sparkRange(series) }
        : { value: fmtInt(hs10), sub: `h/s · 10 s average · peak ${fmtInt(snap?.hs_max)}`, series, ...sparkRange(series) },
    threads: {
      text: threads === null ? DASH : `${threads} of ${clamp ? num(machine?.e_cores) ?? maxThreads : maxThreads}`,
      clamp,
      // The e pill needs the E-core count (machine info, from the power readings) and the
      // optimizer that carries the clamp: on a Mac without them it would only be refused.
      ePill: (num(machine?.e_cores) ?? 0) > 0 || clamp,
      split: (state.mismatch?.threads ?? 0) >= MISMATCH_TICKS ? NOT_APPLIED : split,
      notApplied: (state.mismatch?.threads ?? 0) >= MISMATCH_TICKS,
    },
    period,
    currency,
    rate,
    earned: {
      value: earned === null ? DASH : earned.toFixed(earned < 0.001 ? 6 : 5),
      // With an xmr price (typed, or the feed's), the caption is the money figure; the unit and
      // window move to the tooltip.
      cap: fiat === null ? (p2 ? EARNED_CAP_P2 : EARNED_CAP) : `≈ ${currency}${fmtMoney(fiat)}${p2 ? " expected" : ""}`,
      note: (p2 ? EARNED_NOTE_P2 : EARNED_NOTE) + (price === null ? "" : ` · at ${currency}${price.toFixed(2)} per xmr`),
    },
    cost: { value: cost === null ? DASH : `${currency}${cost.toFixed(mult === 1 ? 3 : 2)}` },
    wallet: {
      editing: !!ui.walletEditing,
      busy: !!ui.walletBusy,
      isDefault,
      masked: wallet?.masked ?? DASH,
      // The default wallet's caption is its disclosure, word for word; the pencil pill says "edit".
      note: (state.mismatch?.wallet ?? 0) >= MISMATCH_TICKS ? NOT_APPLIED : isDefault ? wallet?.label ?? DEFAULT_WALLET_NOTE : `${wallet?.label ?? "your wallet"} · click to edit`,
      notApplied: (state.mismatch?.wallet ?? 0) >= MISMATCH_TICKS,
      draft,
      ...walletHint(draft, ui.walletError),
    },
    worker: workerError && typeof ui.workerDraft === "string" ? ui.workerDraft : typeof settings.worker === "string" ? settings.worker : "",
    workerError,
    tape: Array.isArray(state.tape) ? state.tape : [],
    power: {
      value: machineW === null ? DASH : `${machineW.toFixed(1)} w`,
      sub: `whole mac · cpu ${cpuW === null ? DASH : cpuW.toFixed(1)} · ram ${ramW === null ? DASH : ramW.toFixed(1)}`,
      source,
      sourceSub,
      sourceIcon,
      watts,
      temps,
      wLo: wVals.length ? Math.max(0, Math.min(...wVals) * 0.5) : 0,
      wHi: wVals.length ? Math.max(...wVals) * 1.1 + 0.5 : 1,
      tLo: TEMP_BAR_F[0],
      tHi: tVals.length ? Math.max(140, Math.max(...tVals) + 10) : 140,
    },
    // nominal fills one segment, moderate two, heavy three, trapping (and sleeping) four.
    thermal: { word: thermalIdx < 0 ? DASH : THERMAL_WORDS[thermalIdx], level: thermalIdx < 0 ? 0 : Math.min(4, thermalIdx + 1) },
    measure,
    measureNote: measure === "cpu" ? "cpu only · compares setups" : "whole mac from the outlet",
    profile,
    optimizer,
    lowPowerGain: lowPowerGainView(state.trials),
    curve: curveView(state.trials?.curve, maxThreads),
    // "reset trials" (erase every trial; nothing starts) sits under the list; clearing is
    // refused while mining, so it shows only when stopped.
    trials: { ...trialsView(state.trials, measure), clear: num(state.trials?.count) > 0 && !isRunning(run) },
  };
}

// ── card kinds ─────────────────────────────────────────────────────────────

const title = (t) => `<div class="card__title">${esc(t)}</div>`;
const tip = (t) => (t ? ` title="${esc(t)}"` : "");
const value = (v, extra = "", t = "") => `<div class="card__value${extra}"${tip(t)}>${esc(v ?? DASH)}</div>`;
const cap = (c, extra = "", t = "") => `<div class="card__cap${extra}"${tip(t)}>${esc(c ?? DASH)}</div>`;
/**
 * The caption as the card's bottom row (.card__base): pinned to the card's bottom edge, so every
 * 1×1 card's caption sits on one line, the threads card's. A bar sits between value and caption.
 */
const base = (c, extra = "", t = "") => cap(c, " card__base" + extra, t);
/** Corner-pill glyphs, drawn as small inline shapes (as the run pills' glyphs once were). */
const PILL_GLYPHS = {
  pencil:
    `<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M3 10.5 L10 3.5 L12.5 6 L5.5 13 Z M2.6 11.1 L4.9 13.4 L1.8 14.2 Z ` +
    `M10.8 2.7 L11.7 1.8 A1.77 1.77 0 0 1 14.2 4.3 L13.3 5.2 Z"></path></svg>`,
  arrow: `<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M2.5 7.1 H10.2 L7.4 4.3 L8.6 3.1 L13.5 8 L8.6 12.9 L7.4 11.7 L10.2 8.9 H2.5 Z"></path></svg>`,
};
/**
 * A small round pill at the card's top right, for a card that is edited (pencil) or explained
 * (arrow). A real button: it opens the same edit mode, or the same guide, the card's own
 * controls do. It never moves the centred title.
 */
const cornerPill = (p) =>
  `<button class="card__pill" type="button" data-act="${esc(p.act)}"${p.val ? ` data-val="${esc(p.val)}"` : ""} ` +
  `aria-label="${esc(p.label)}" title="${esc(p.label)}">${PILL_GLYPHS[p.glyph] ?? ""}</button>`;
const WALLET_PILL = { act: "wallet-edit", glyph: "pencil", label: "edit wallet" };
const WORKER_PILL = { act: "worker-edit", glyph: "pencil", label: "edit worker" };
/** The cores card's e pill: light's setup, chosen by hand. */
const ECORES_TIP = "e-cores only: mines on the efficiency cores at background priority. other apps get the cpu first. the hashrate is much lower.";
/** The cores card counts cores in use; xmrig runs one thread on each. */
const CORES_TIP = "cores that mine · xmrig runs one thread on each core";
const bar = (w) => `<div class="bar"><div class="bar__fill" data-w="${esc(pct(w))}"></div></div>`;
/**
 * Earned caption beside the period pill: the unit and its window (EARNED_CAP). "Estimate" and
 * the rate it rests on don't fit a 1×1 row beside the pill (180px), so they ride in the tooltip
 * and a visually hidden span.
 */
const EARNED_NOTE = "estimate from xmrig's 15 min average hashrate, at the sep 23 rate: 0.00000007875 xmr per h/s per day";
/** p2pool's earned caption and tooltip: expected, from the live network figures. */
export const EARNED_CAP_P2 = "xmr · expected";
const EARNED_NOTE_P2 =
  "expected long-run average: xmrig's 15 min hashrate × block reward ÷ monero difficulty. p2pool supplies both values. p2pool pays the solo-mining average with no fee. a payment comes when p2pool finds a block.";
const walletHintClass = (state) => `wallet__hint${state ? " is-" + esc(state) : ""}`;
/**
 * The logs legend: each tape tag in its colour, and what the engine tags with it (xmrange-core
 * engine.rs: TapeState and Health). cpu = accepted / rejected shares; net = pool connect, new
 * job, difficulty, pool wait; miner = the 60 s speed report and xmrig's api coming back;
 * error = failed commands, launch failures, a silent api.
 */
export const TAPE_LEGEND = [
  ["cpu", "shares"],
  ["net", "pool"],
  ["miner", "speed · api"],
  ["error", "faults"],
];
const tapeLegend =
  `<div class="card__base legend--tape" aria-label="tape tags">` +
  TAPE_LEGEND.map(([tag, what]) => `<span class="tag--${esc(tag)}">${esc(tag)}</span><span>${esc(what)}</span>`).join("") +
  `</div>`;
/** Run-pill emoji, in a 20px slot: start/resume 🔥, pause ⏳, stop 🌙. */
const RUN_EMOJI = { play: "fire", pause: "hourglass", stop: "moon" };
const periodPill = (period) =>
  `<button class="period" type="button" data-act="period" aria-label="change estimate period">per ${esc(period)} ▾</button>`;

const KINDS = {
  /**
   * The 1×1 anatomy: title · emoji 40 · Fraunces 21 value · optional bar (right under the value,
   * at one height in every bar card) · caption pinned to the bottom (one or two lines), and an
   * optional corner pill.
   */
  stat(m, vm) {
    const s = vm.stats?.[m.id] || {};
    const sub = s.sub ?? m.sub;
    const subTip = s.subTip ?? m.subTip;
    const caption = s.sub2
      ? `<div class="card__base card__caps">${cap(sub, "", subTip)}${cap(s.sub2)}</div>`
      : base(sub, "", subTip);
    return (
      title(m.title) +
      (m.pill ? cornerPill(m.pill) : "") +
      img(m.icon, "card__icon") +
      value(s.value ?? m.value, "", m.valueTip) +
      (m.bar ? bar(s.bar) : "") +
      caption
    );
  },

  /**
   * hashrate 2×2, on the shared 2×2 anatomy (see .hero): the top half's left column is
   * "mine" · its emoji · the run pills (start / pause / resume, then stop; the stop pill
   * never sits lower than the hashrate caption), the right column is the hashrate
   * (title · emoji · rate · caption). The sparkline fills the bottom row's inner box;
   * drawCharts() draws its paths through DOM APIs.
   */
  spark(m, vm) {
    const h = vm.hashrate;
    const rp = vm.runPills || { primary: { glyph: "play", label: "start", aria: "start mining", title: "", disabled: false }, stop: { disabled: true } };
    const pill = (act, glyph, label, aria, tip, disabled) =>
      `<div class="choice"><button class="choice__pill run__pill" type="button" data-act="${esc(act)}" aria-label="${esc(aria)}"` +
      `${tip ? ` title="${esc(tip)}"` : ""}${disabled ? " disabled" : ""}><span class="run__face">${img(RUN_EMOJI[glyph], "")}<span>${esc(label)}</span></span></button></div>`;
    const heroCol =
      `<div class="joined__left hero__col">` +
      title(m.title) +
      img(m.icon, "hero__icon spark__icon") +
      `<div class="hero__value spark__value${h.isError ? " is-error" : ""}">${esc(h.value)}</div>` +
      `<div class="card__cap hero__cap spark__cap${h.isError ? " spark__cap--err" : ""}"${h.isError ? ' role="status"' : ""}>${esc(h.sub)}</div>` +
      `</div>`;
    return (
      `<div class="joined spark__top hero"><div class="run__col hero__col">${title("mine")}${img(m.icon2, "hero__icon spark__icon")}<div class="run__pills">` +
      pill("run-primary", rp.primary.glyph, rp.primary.label, rp.primary.aria, rp.primary.title, rp.primary.disabled) +
      pill("run-stop", "stop", "stop", "stop mining", STOP_TIP, rp.stop.disabled) +
      `</div></div>${heroCol}</div>` +
      `<div class="chart__box"><svg class="chart" data-chart="hashrate" width="396" height="${CHART_H}" viewBox="0 0 396 ${CHART_H}" aria-label="hashrate over the last three minutes"></svg></div>`
    );
  },

  /**
   * power | source, on the shared 2×2 anatomy (see .hero): two titled columns on the
   * 180px / 48px column grid, and the watts / °f chart in the bottom row's inner box, its
   * legend floating at the chart's top left, above the lines. The chart's paths (last 780
   * samples) are drawn by drawCharts() through DOM APIs.
   */
  power(m, vm) {
    const p = vm.power;
    const col = (t, icon, v, c) =>
      `<div class="hero__col">${title(t)}${img(icon, "hero__icon power__icon")}<div class="hero__value power__value">${esc(v)}</div>` +
      `<div class="card__cap hero__cap power__cap">${esc(c)}</div></div>`;
    return (
      `<div class="hero">` +
      col(m.title, m.icon, p.value, p.sub) +
      col(m.title2, p.sourceIcon || "hoh_source_outlet", p.source, p.sourceSub) +
      `</div>` +
      `<div class="chart__box"><svg class="chart" data-chart="power" width="396" height="${CHART_H}" viewBox="0 0 396 ${CHART_H}" aria-label="whole-mac watts and cpu temperature over the last 13 minutes"></svg>` +
      `<div class="legend"><span class="legend__swatch">━</span> watts ┅ °f · last 13 min</div></div>`
    );
  },

  /**
   * tape 3×1, titled "logs": a 1×1-wide left column (title · emoji · the tag legend, its last
   * line on the 1×1 caption line), and the log window from the wallet card's right edge to
   * the card's end, full card height: the newest TAPE_LINES xmrig lines, mono.
   */
  tape(m, vm) {
    const lines = (vm.tape || []).slice(-TAPE_LINES);
    const body = lines.length
      ? lines
          .map((ln) => {
            const tag = ["cpu", "net", "miner", "error"].includes(ln.tag) ? ln.tag : "miner";
            return `<div class="tape__line"><span class="tape__ts">${esc(hm(ln.ts))}</span> <span class="tag--${tag}">${esc(ln.tag)}</span> ${esc(ln.msg)}</div>`;
          })
          .join("")
      : `<div class="tape__empty">the tape starts when xmrig does</div>`;
    return (
      `<div class="joined tape__wrap"><div class="joined__left">${title(m.title)}${img(m.icon, "card__icon")}${tapeLegend}</div>` +
      `<div class="joined__right"><div class="tape" aria-label="live xmrig log">${body}</div></div></div>`
    );
  },

  /** Joined 2×1: a 1×1-aligned left column (title · emoji · value · caption, an optional 2nd
   * caption line) + content across the join. A "list" row is a button (P5: tap to run that exact
   * setup) when it carries a `threads` count, else plain text. */
  joined(m, vm) {
    const d = vm[m.id] || {};
    // sub2 (the throttle card's "holding: …") stays after the caption in DOM order — so a plain
    // ".card__cap" lookup still finds the one true caption first — but CSS `order` renders it
    // above that caption, which stays pinned to the shared baseline (`anatomy()`'s rule).
    const left =
      `<div class="joined__left">${title(m.title)}${img(m.icon, "card__icon")}` +
      value(d.value) +
      base(d.sub, "", d.subTip) +
      (d.sub2 ? cap(d.sub2, " joined__sub2", d.sub2Tip) : "") +
      `</div>`;
    let right;
    if (m.right === "choices") {
      right =
        `<div class="joined__right joined__right--choices">` +
        (d.choices || [])
          .map((c) => {
            const on = d.selected === c.id;
            const turtle = c.lpm ? ` <img class="trial__icon" src="${esc(emojiSrc("hoh_low_power"))}" alt="low power mode" />` : "";
            const blurb = c.blurb2 ? `${esc(c.blurb)}<br>${esc(c.blurb2)}${turtle}` : esc(c.blurb);
            return (
              `<div class="choice"><button class="choice__pill${on ? " is-on" : ""}" type="button" data-act="profile" data-val="${esc(c.id)}" aria-pressed="${on}"${m.disabled || c.disabled ? " disabled" : ""}>` +
              `${img(c.icon, "")}${esc(c.label)}</button><span class="choice__blurb"${tip(c.tip)}>${blurb}</span></div>`
            );
          })
          .join("") +
        `</div>`;
    } else {
      const rows = d.rows || [];
      right =
        `<div class="joined__right joined__right--list"><div class="trials__rows">` +
        (rows.length
          ? rows
              .map((r, i) => {
                // Three columns, every one vertically centred: star (fixed width, so the counts
                // line up), "3 cores" + icons, and the h/w.
                const star = `<span class="trial__star">${r.best ? `<img class="trial__icon" src="${esc(emojiSrc("star"))}" alt="best" />` : ""}</span>`;
                const inner =
                  `${star}<span class="trial__label" title="${esc(r.best ? "best · " + r.label : r.label)}">${trialLabelHtml(r)}</span>` +
                  `<span class="trial__hw">${esc(r.hw)} h/w</span>`;
                if (num(r.threads) === null) return `<div class="trial${r.best ? " is-best" : ""}">${inner}</div>`;
                const lp = r.low_power === true ? "true" : r.low_power === false ? "false" : "";
                return (
                  `<button class="trial trial--pick${r.best ? " is-best" : ""}" type="button" data-act="apply-trial" data-i="${i}" ` +
                  `data-threads="${esc(r.threads)}" data-clamp="${r.e_core_clamp ? "1" : "0"}" data-utility="${r.utility ? "1" : "0"}" data-lp="${lp}" ` +
                  `aria-label="run ${esc(r.label)}">${inner}</button>`
                );
              })
              .join("")
          : `<div class="empty">no trials yet · each measured setup lands here, with its h/w</div>`) +
        `</div>` +
        (d.clear
          ? `<div class="seeker__actions"><button class="btn btn--sm btn--quiet" type="button" data-act="trials-clear">reset trials</button></div>`
          : "") +
        `</div>`;
    }
    return `<div class="joined">${left}${right}</div>`;
  },

  /**
   * lever 2×1 (low power on the machine tab): the 1×1 anatomy in a 1×1-aligned left column
   * (title · emoji · value · caption on the 1×1 caption line), the corner pill at the card's top
   * right, and the efficient profile's own measured gain written out on the right — this Mac's
   * own trials, never a hard-coded figure. "run trials to measure" until both Low Power Mode
   * states are measured.
   */
  lever(m, vm) {
    const s = vm.stats?.[m.id] || {};
    const g = vm.lowPowerGain || { ready: false, rows: [] };
    const sub = g.ready ? "measured on this mac" : "run trials to measure";
    const body = g.ready
      ? g.rows
          .map(
            (r) =>
              `<span class="lever__what">${esc(r.what)}</span><span class="lever__hw">${esc(`${r.off} → ${r.on}`)}</span>` +
              `<span class="lever__gain">${esc(r.gain)}</span>`,
          )
          .join("")
      : `<span class="lever__note">not yet measured in both states</span>`;
    return (
      (m.pill ? cornerPill(m.pill) : "") +
      `<div class="joined"><div class="joined__left">${title(m.title)}${img(m.icon, "card__icon")}` +
      value(s.value ?? m.value) +
      base(sub, "", s.subTip || "") +
      `</div><div class="joined__right lever" aria-label="measured h/w, low power mode off then on">` +
      (g.ready ? `<span class="lever__note">off → on · balanced profile</span>${body}` : body) +
      `</div></div>`
    );
  },

  /**
   * trials progress panel (2×1, P5: replaced the optimizer/keel halves): a 1×1-aligned left
   * column (title · emoji · value · caption · the run/stop button) + a small scrolling log of
   * trial events across the join, in the tape's own styling.
   */
  progress(m, vm) {
    const o = vm.optimizer;
    const lines = o.log || [];
    const body = lines.length
      ? lines.map((ln) => `<div class="tape__line"><span class="tape__ts">${esc(hm(ln.ts))}</span> ${trialMsgHtml(ln.msg)}</div>`).join("")
      : `<div class="tape__empty">trial events land here once you run trials</div>`;
    return (
      `<div class="joined tape__wrap"><div class="joined__left">${title(m.title)}${img(m.icon, "card__icon")}` +
      `<div class="card__value seeker__value">${esc(o.value ?? DASH)}${o.turtle ? `<img class="seeker__turtle" src="${esc(emojiSrc("hoh_low_power"))}" alt="low power mode" title="low power mode trials" />` : ""}</div>` +
      bar(o.bar) +
      base(fitText(o.sub, 28).text, "", o.tip || fitText(o.sub, 28).tip) +
      `</div><div class="joined__right"><div class="tape tape--scroll" aria-label="trial events">${body}</div>` +
      `<div class="seeker__actions">${o.buttons.map((b, i) => `<button class="btn btn--sm${i ? " btn--quiet" : ""}" type="button" data-act="${esc(b.act)}" title="close other apps for the best results"${m.disabled ? " disabled" : ""}>${esc(b.label)}</button>`).join("")}</div></div></div>`
    );
  },

  /**
   * wallet, like the worker card: view = title · pencil pill · emoji · masked address
   * (a button: tap to edit) · caption. The default wallet's caption is its disclosure
   * ("default · ghostdrip dev wallet"), a user's reads "your wallet · tap to edit".
   * Edit = mono input · live hint · save / cancel, plus "use default" once a user
   * wallet is set. A backend error takes the hint's place, verbatim.
   */
  wallet(m, vm) {
    const w = vm.wallet;
    if (w.editing) {
      return (
        title(m.title) +
        `<input class="wallet__input" type="text" data-field="wallet" value="${esc(w.draft)}" placeholder="your monero address" aria-label="your monero wallet address" spellcheck="false" autocomplete="off" />` +
        `<div class="${walletHintClass(w.hintState)}" role="status">${esc(w.hint)}</div>` +
        `<div class="wallet__actions card__base"><button class="btn btn--sm" type="button" data-act="wallet-save"${w.busy ? " disabled" : ""}>save</button>` +
        `<button class="btn btn--sm btn--quiet" type="button" data-act="wallet-cancel">cancel</button>` +
        (w.isDefault ? "" : `<button class="wallet__clear" type="button" data-act="wallet-clear">use default</button>`) +
        `</div>`
      );
    }
    return (
      title(m.title) +
      cornerPill(WALLET_PILL) +
      img(m.icon, "card__icon") +
      `<button class="card__value wallet__value" type="button" data-act="wallet-edit" title="tap to edit">${esc(w.masked ?? DASH)}</button>` +
      base(w.note, w.notApplied ? " wallet__note is-warn" : " wallet__note")
    );
  },

  threads(m, vm) {
    return (
      title(m.title) +
      img(m.icon, "card__icon") +
      (vm.threads.ePill
        ? `<button class="card__pill card__pill--text${vm.threads.clamp ? " is-on" : ""}" type="button" data-act="ecores-toggle" aria-pressed="${vm.threads.clamp}" ` +
          `aria-label="e-cores only" title="${esc(ECORES_TIP)}">e</button>`
        : "") +
      `<div class="stepper">` +
      `<button class="stepper__btn" type="button" data-act="threads-down" aria-label="one fewer core">−</button>` +
      value(vm.threads.text, " stepper__value", CORES_TIP) +
      `<button class="stepper__btn" type="button" data-act="threads-up" aria-label="one more core">+</button>` +
      `</div>` +
      base(vm.threads.split, vm.threads.notApplied ? " is-warn" : "")
    );
  },

  /** worker: inline input (the pencil pill focuses it); blur or Enter saves. A backend error replaces the caption, verbatim. */
  worker(m, vm) {
    const err = vm.workerError;
    return (
      title(m.title) +
      cornerPill(WORKER_PILL) +
      img(m.icon, err ? "card__icon card__icon--sm" : "card__icon") +
      `<input class="worker__input" type="text" data-field="worker" value="${esc(vm.worker)}" aria-label="worker name" spellcheck="false" autocomplete="off" maxlength="64" />` +
      (err ? `<div class="card__err card__base" role="status">${esc(err)}</div>` : base(vm.workerNote || "worker name · click to edit"))
    );
  },

  cost(m, vm) {
    const rate = vm.rate === null ? "" : String(vm.rate);
    return (
      title(m.title) +
      img(m.icon, "card__icon") +
      value(vm.cost.value) +
      `<div class="card__row card__base" title="${esc(COST_WINDOW_NOTE)} × 24 h × your rate">${periodPill(vm.period)}` +
      `<span class="sr-only">${esc(COST_WINDOW_NOTE)}</span>` +
      `<label class="rate">${esc(vm.currency)}<input class="rate__input" type="number" step="0.01" min="0" data-field="rate" value="${esc(rate)}" aria-label="electricity rate per kilowatt hour" />/kwh</label>` +
      `</div>`
    );
  },

  earned(m, vm) {
    return (
      title(m.title) +
      img(m.icon, "card__icon") +
      value(vm.earned.value) +
      `<div class="card__row card__base">${periodPill(vm.period)}<span title="${esc(vm.earned.note)}">${esc(vm.earned.cap)}<span class="sr-only"> (${esc(vm.earned.note)})</span></span></div>`
    );
  },

  /** thermal: pressure word + a 4-step level bar (nominal, moderate, heavy, trapping). */
  thermal(m, vm) {
    const level = Math.max(0, Math.min(4, Math.round(num(vm.thermal.level) ?? 0)));
    const segs = [1, 2, 3, 4].map((i) => `<div class="levels__seg${i <= level ? " is-on" : ""}"></div>`).join("");
    return (
      title(m.title) +
      img(m.icon, "card__icon") +
      value(vm.thermal.word) +
      `<div class="levels" role="img" aria-label="thermal pressure level ${esc(level)} of 4">${segs}</div>` +
      base("thermal pressure from macos")
    );
  },

  /** h/w counts: whole mac | cpu only. */
  measure(m, vm) {
    const btn = (id, label) => {
      const on = vm.measure === id;
      return `<button class="seg__btn${on ? " is-on" : ""}" type="button" data-act="measure" data-val="${esc(id)}" aria-pressed="${on}">${esc(label)}</button>`;
    };
    return (
      title(m.title) +
      img(m.icon, "card__icon") +
      `<div class="seg seg--small">${btn("sys", "whole mac")}${btn("cpu", "cpu only")}</div>` +
      base(vm.measureNote)
    );
  },

  /** thread curve 2×2: h/s by thread count; the core that added the most h/s per watt in fill. */
  curve(m, vm) {
    const c = vm.curve;
    const n = Math.max(1, Math.min(16, num(c.maxThreads) ?? 8));
    const bars = c.bars || Array.from({ length: n }, (_, i) => ({ n: String(i + 1), hs: DASH, h: 4, best: false }));
    const cols = bars
      .map(
        (b) =>
          `<div class="curve__col${b.best ? " is-best" : ""}"><div class="curve__hs">${esc(b.hs)}</div>` +
          `<div class="curve__bar" data-h="${esc(Math.max(0, Math.min(160, num(b.h) ?? 0)))}"></div>` +
          `<div class="curve__n">${esc(b.n)}</div></div>`,
      )
      .join("");
    const foot = c.bars
      ? `${esc(c.foot || "")}${c.footMuted ? `<br><span>${esc(c.footMuted)}</span>` : ""}`
      : `no curve yet<br><span>each measured core count shows here</span>`;
    return (
      title(m.title) +
      `<div class="curve__cap">${esc(c.cap || "most h/s per watt added")}</div>` +
      `<div class="curve__bars">${cols}</div>` +
      `<div class="curve__foot">${foot}</div>`
    );
  },

  /** p2pool "your own node" 2×2 explainer. */
  note(m) {
    return (
      title(m.title) +
      img(m.icon, "note__icon") +
      `<div class="note__value">${esc(m.value)}</div>` +
      `<div class="note__cap">${esc(m.sub)}</div>` +
      `<div class="chip">${esc(m.chip)}</div>`
    );
  },
};

export const CARD_KINDS = Object.keys(KINDS);

function cardClass(m) {
  return `card card--${m.kind}${m.cs > 1 ? " c" + m.cs : ""}${m.rs > 1 ? " r" + m.rs : ""}${m.disabled ? " is-disabled" : ""}`;
}

function cardInner(m, vm) {
  const render = KINDS[m.kind];
  return render ? render(m, vm) : "";
}

// ── render ─────────────────────────────────────────────────────────────────

/** Set CSSOM-driven sizes (data-w → width %, data-h → height px). */
export function applyDynamics(root) {
  root.querySelectorAll("[data-w]").forEach((el) => {
    el.style.width = `${pct(Number(el.dataset.w))}%`;
  });
  root.querySelectorAll("[data-h]").forEach((el) => {
    const h = num(Number(el.dataset.h));
    el.style.height = `${h === null ? 0 : h}px`;
  });
}

function hasFocusedInput(el) {
  const a = document.activeElement;
  return !!a && el.contains(a) && (a.tagName === "INPUT" || a.tagName === "TEXTAREA");
}

/**
 * A card is "held" while it has a focused input or a pointer is pressed on it
 * (app.js sets el.__pressed on pointerdown). Replacing a held card's DOM would
 * move the text under the cursor or swallow the click that is in flight.
 */
function isHeld(el) {
  return !!el.__pressed || hasFocusedInput(el);
}

const SVG_NS = "http://www.w3.org/2000/svg";

/** Replace an <svg>'s paths through DOM APIs (createElementNS), never an HTML string. */
function drawPaths(svg, paths) {
  const key = paths.map((p) => p.cls + p.d).join("|");
  if (svg.__key === key && svg.firstChild) return;
  svg.__key = key;
  while (svg.firstChild) svg.removeChild(svg.firstChild);
  for (const p of paths) {
    const el = document.createElementNS(SVG_NS, "path");
    el.setAttribute("class", p.cls);
    el.setAttribute("d", p.d);
    svg.appendChild(el);
  }
}

/** Draw the live charts (hashrate sparkline, power + °f) from the view model. Runs on every render. */
export function drawCharts(root, vm) {
  const svg = root.querySelector('svg[data-chart="hashrate"]');
  if (svg) {
    const h = vm.hashrate || {};
    const p = linePath(h.series, h.lo ?? 0, h.hi ?? 1, 396, CHART_H);
    drawPaths(
      svg,
      p
        ? [
            { cls: "chart__area", d: p.area },
            { cls: "chart__line", d: p.d },
          ]
        : [{ cls: "chart__base", d: `M2 ${CHART_H - 8} L394 ${CHART_H - 8}` }],
    );
  }
  const psvg = root.querySelector('svg[data-chart="power"]');
  if (psvg) {
    const p = vm.power || {};
    // The legend floats in the chart's top LEGEND_PAD px, so both lines stay below it.
    const w = linePath(p.watts, p.wLo ?? 0, p.wHi ?? 1, 396, CHART_H, LEGEND_PAD);
    const t = linePath(p.temps, p.tLo ?? 0, p.tHi ?? 1, 396, CHART_H, LEGEND_PAD);
    const paths = [];
    if (w) paths.push({ cls: "chart__area", d: w.area }, { cls: "chart__line", d: w.d });
    if (t) paths.push({ cls: "chart__temp", d: t.d });
    drawPaths(psvg, paths.length ? paths : [{ cls: "chart__base", d: `M2 ${CHART_H - 8} L394 ${CHART_H - 8}` }]);
  }
}

/**
 * Keyboard focus that a rebuild could not put back, because its control came
 * back disabled (a run pill while a command is in flight). The next renders
 * return it as soon as that control is enabled again. If it stays disabled
 * (stop, once stopped), focus goes to the card's first enabled control, but
 * never onto stop itself; with no such control yet, it keeps waiting. It is
 * forgotten once the user focuses anything else, or the tab changes.
 */
let lostFocus = null;

function findAct(el, act, val) {
  return [...el.querySelectorAll("[data-act]")].find((b) => b.dataset.act === act && (b.dataset.val ?? null) === (val ?? null));
}

function restoreLostFocus(grid) {
  if (!lostFocus) return;
  const a = document.activeElement;
  if (a && a !== document.body) {
    lostFocus = null;
    return;
  }
  const card = [...grid.children].find((c) => c.dataset.mod === lostFocus.id);
  if (!card) return;
  const exact = findAct(card, lostFocus.act, lostFocus.val);
  if (exact && !exact.disabled) {
    exact.focus();
    lostFocus = null;
    return;
  }
  // Nothing to take it yet (a command in flight, or "starting"): wait for the next render.
  const next = [...card.querySelectorAll("[data-act]")].find((b) => !b.disabled && b.dataset.act !== "run-stop");
  if (!next) return;
  next.focus();
  lostFocus = null;
}

/** Scrollable lists inside a card: the seeker's log (newest at the bottom) and the trials rows. */
const SCROLLERS = ".tape--scroll, .trials__rows";

function scrollState(el) {
  return [...el.querySelectorAll(SCROLLERS)].map((s) => ({ top: s.scrollTop, pinned: s.scrollTop + s.clientHeight >= s.scrollHeight - 2 }));
}

/** A card re-render keeps each list's place; a log at its bottom (or new) stays pinned there. */
function restoreScroll(el, saved) {
  el.querySelectorAll(SCROLLERS).forEach((s, i) => {
    const st = saved[i];
    const log = s.classList.contains("tape--scroll");
    s.scrollTop = log && (!st || st.pinned) ? s.scrollHeight : st ? st.top : 0;
  });
}

/**
 * Render the grid for a module list. A new module list (tab/pool change)
 * rebuilds the grid; the same list patches card by card, skipping unchanged
 * cards and any held card (focused input or pressed pointer).
 */
export function renderGrid(grid, mods, vm) {
  const key = mods.map((m) => m.id).join("|");
  if (grid.dataset.key !== key) {
    const inner = mods.map((m) => cardInner(m, vm));
    grid.innerHTML = mods
      .map((m, i) => `<article class="${cardClass(m)}" data-mod="${esc(m.id)}">${inner[i]}</article>`)
      .join("");
    grid.dataset.key = key;
    lostFocus = null;
    Array.from(grid.children).forEach((el, i) => {
      el.__src = inner[i];
    });
    restoreScroll(grid, []);
    applyDynamics(grid);
    drawCharts(grid, vm);
    return;
  }
  const els = grid.children;
  mods.forEach((m, i) => {
    const el = els[i];
    if (!el || isHeld(el)) return;
    const html = cardInner(m, vm);
    if (el.__src === html) return;
    // A focused button (a run pill, the period pill) is rebuilt with the card;
    // put keyboard focus back on its successor so a snapshot never steals it.
    const a = document.activeElement;
    const act = a && el.contains(a) ? a.dataset?.act : null;
    const val = act ? a.dataset?.val : null;
    const scrolls = scrollState(el);
    el.innerHTML = html;
    el.__src = html;
    restoreScroll(el, scrolls);
    applyDynamics(el);
    if (act) {
      const again = findAct(el, act, val);
      if (again && !again.disabled) again.focus();
      else lostFocus = { id: m.id, act, val };
    }
  });
  restoreLostFocus(grid);
  drawCharts(grid, vm);
}

/** A trial log line, compact: thread counts as the cores icon, Low Power Mode as the turtle,
 * full power left unsaid. Everything else stays text (escaped). */
export function trialMsgHtml(msg) {
  let t = esc(String(msg ?? ""));
  t = t.replace(/ · full power/g, "");
  t = t.replace(/ · low power mode/g, ` <img class="tape__icon" src="${esc(emojiSrc("hoh_low_power"))}" alt="low power" />`);
  t = t.replace(/(\d+) (?:threads?|cores?)\b/g, (_, n) => `${n} ${n === "1" ? "core" : "cores"} <img class="tape__icon" src="${esc(emojiSrc("hoh_cores"))}" alt="" />`);
  return t;
}

/** A trials-list row's setup: "3 cores" (or "2 e-cores") with the cores icon, plus the turtle for
 * Low Power Mode. Full power is left unsaid. */
export function trialLabelHtml(r) {
  const n = num(r?.threads);
  if (n === null) return trialMsgHtml(r?.label ?? "");
  const word = r.e_core_clamp ? (n === 1 ? "e-core" : "e-cores") : n === 1 ? "core" : "cores";
  const turtle = r.low_power === true ? `<img class="trial__icon" src="${esc(emojiSrc("hoh_low_power"))}" alt="low power" />` : "";
  return `${n} ${word}${r.utility ? " · utility" : ""} <img class="trial__icon" src="${esc(emojiSrc("hoh_cores"))}" alt="" />${turtle}`;
}
