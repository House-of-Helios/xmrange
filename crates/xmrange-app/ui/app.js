// XMRange shell: state, events, tab / pool / theme logic and the render loop.
// Card kinds and the module registry live in modules.js; styles in styles.css.

import {
  buildView,
  isRunning,
  liveCommand,
  lowPowerGuideText,
  MISMATCH_TICKS,
  modulesFor,
  num,
  PERIODS,
  POWER_MAX,
  PROFILES,
  renderGrid,
  runStateOf,
  SERIES_MAX,
  shownClamp,
  shownThreads,
  toF,
  walletHint,
} from "./modules.js";

const invoke = (cmd, args) => window.__TAURI__.core.invoke(cmd, args);
const listen = (event, fn) => window.__TAURI__.event.listen(event, fn);
const $ = (id) => document.getElementById(id);

const TABS = ["mining", "machine", "tuning"];
const POOLS = ["mo", "p2"];
const THEMES = ["system", "light", "dark"];
const MEASURES = ["sys", "cpu"];
const TAPE_MAX = 50;
/** Three days of hourly lines. */
const HOURS_MAX = 72;
const NODE_POLL_MS = 10 * 1000;

export const state = {
  info: null, // app_info
  settings: null, // get_settings
  snap: null, // latest xmrange://snapshot payload
  idlePower: null, // latest xmrange://idle power sample (machine tab while nothing mines)
  mismatch: { wallet: 0, threads: 0 }, // snapshots in a row where xmrig runs something else than the cards say
  series: [], // last SERIES_MAX hs10 values, for the sparkline
  powerSeries: [], // last POWER_MAX {w: whole-mac watts, f: cpu °F}, for the power chart
  lowPower: null, // Low Power Mode, one source: the 2 s watch (snapshot or idle sample `low_power`)
  payouts: null, // p2pool mode: all-time payouts to this wallet, from the Ledger ({ count, atomic })
  node: null, // p2pool mode: the local Monero node, from node_status ({ state, height, target })
  context: null, // last known context (low power, source, thermal); rides on 1 snapshot in 30
  tape: [], // accumulated tape lines (each snapshot carries only the new ones)
  hours: [], // the engine's hourly lines, kept for the whole session (the tape keeps 50 lines)
  trials: null, // list_trials: the tuning tab's measured setups
  trialsRecorded: null, // snapshot.optimizer.trials_recorded last seen; a change reloads the trials
  tab: "mining",
  pool: "mo",
  busy: false, // a live-light command is in flight
  lastRunCommand: null, // the last command liveClick sent (start_mining, pause_mining, …)
  lastError: null, // the last snapshot.error already put on the tape
  engineGen: null, // generation start_mining returned; xmrange://stopped carries it too
  price: null, // the price feed's latest fetch ({ usd, source, error, t }), from xmrange://price
  update: null, // update_status: { version, notes } when GitHub offers a newer signed build, else null
  updateBusy: false, // install_update is in flight (the pill is disabled)
  ui: {
    walletEditing: false,
    walletDraft: "",
    walletError: null, // backend error, shown verbatim
    walletBusy: false,
    settingsError: null, // save_settings refusal from the settings sheet, shown there verbatim
    workerError: null, // backend error, shown verbatim
    workerDraft: null, // what the user typed when the backend refused it
    noticeDismissed: false, // settings.notice_dismissed, read at init
    startedThisSession: false,
    lowPowerAsk: null, // "true" / "false" while a Low Power Mode ask is open, else null
    optError: null, // { at: "profile" | "seeker", msg }: a refused optimizer command, verbatim
  },
};

/** Card actions, keyed by a button's data-act. The machine and tuning modules add theirs later. */
export const actions = {};

const errText = (err) => (typeof err === "string" ? err : String(err?.message ?? err));

function clock() {
  const d = new Date();
  return [d.getHours(), d.getMinutes(), d.getSeconds()].map((n) => String(n).padStart(2, "0")).join(":");
}

function pushTape(lines) {
  for (const ln of lines) {
    if (!ln || typeof ln !== "object") continue;
    state.tape.push({ ts: String(ln.ts ?? ""), tag: String(ln.tag ?? "miner"), msg: String(ln.msg ?? "") });
    if (ln.tag === "hour") state.hours.push(`${ln.ts} ${ln.msg}`);
  }
  if (state.tape.length > TAPE_MAX) state.tape.splice(0, state.tape.length - TAPE_MAX);
  if (state.hours.length > HOURS_MAX) state.hours.splice(0, state.hours.length - HOURS_MAX);
}

/** A UI-side failure lands on the tape as an error line, where the engine's own errors go. */
function tapeError(msg) {
  pushTape([{ ts: clock(), tag: "error", msg }]);
}

// ── first-run notice (what the pool sees; the default wallet), before the first start ──

/**
 * Dismissal is saved in settings.json (the app's own data dir), never in the
 * webview's storage. If the save fails it still holds for this session.
 */
async function dismissNotice() {
  state.ui.noticeDismissed = true;
  renderNotice();
  if (state.settings) await saveSettings({ ...state.settings, notice_dismissed: true });
}

/** Doctrine §1: the first run says what the pool sees, and names the default wallet while it is in use. */
export function noticeShown() {
  return state.info !== null && !state.ui.noticeDismissed && !state.ui.startedThisSession;
}

/** Show or hide the notice; its default-wallet line shows only while that wallet is in use. */
/** What the pool sees follows the Tor setting (on by default). */
const NOTICE_POOL = {
  tor: "Your pool sees your wallet address, the worker name you typed and your hashrate. Tor is on, so it sees a Tor exit, not your IP address. Nothing else leaves this Mac.",
  direct: "Your pool sees your wallet address, your IP address, the worker name you typed and your hashrate. Nothing else leaves this Mac.",
};

export function setNoticeVisible(show) {
  $("notice").hidden = !show;
  $("notice-pool").textContent = state.settings?.tor === false ? NOTICE_POOL.direct : NOTICE_POOL.tor;
  $("notice-wallet").hidden = state.info?.wallet?.is_default !== true;
  $("board").classList.toggle("has-notice", !!show || !$("crash-offer").hidden);
}

function renderNotice() {
  setNoticeVisible(noticeShown());
}

// ── theme ──────────────────────────────────────────────────────────────────

function applyTheme(theme) {
  document.documentElement.dataset.theme = THEMES.includes(theme) ? theme : "system";
}

function effectiveTheme() {
  const t = document.documentElement.dataset.theme;
  if (t === "light" || t === "dark") return t;
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

/** The light/dark pill (the owner's mode icons) flips what you see (light ↔ dark) and saves it; "system" is chosen in the settings sheet. */
async function toggleTheme() {
  await setTheme(effectiveTheme() === "dark" ? "light" : "dark");
}

// ── header pieces ──────────────────────────────────────────────────────────

function renderNav() {
  document.querySelectorAll("#tabs [data-tab]").forEach((b) => {
    const on = b.dataset.tab === state.tab;
    b.classList.toggle("is-on", on);
    b.setAttribute("aria-pressed", String(on));
  });
  document.querySelectorAll("#pools [data-pool]").forEach((b) => {
    const on = b.dataset.pool === state.pool;
    b.classList.toggle("is-on", on);
    b.setAttribute("aria-pressed", String(on));
  });
}

function runState() {
  return runStateOf(state.snap);
}

/**
 * Run-state chrome: the state on <body data-run> and the p2pool "stage 2" chip. (The status-row
 * live light was retired for the hashrate card's "mine" pills, Helios 2026-09-23; the settings
 * sheet's own stop button followed it, Helios 2026-09-28 — the hashrate card's pills handle
 * stopping everywhere, p2pool included.)
 */
function renderLive() {
  const run = runState();
  // The run state rides on <body data-run>; the hashrate card's pills and caption show it.
  document.body.dataset.run = run;
  // One outbound set at a time (doctrine §1a): the pool and Tor change only while stopped.
  const locked = isRunning(run);
  document.querySelectorAll("#pools [data-pool], #set-tor [data-tor], #set-chain [data-chain], #set-node [data-node], #node-folder-choose").forEach((b) => {
    b.disabled = locked && !b.classList.contains("is-on");
    b.title = b.disabled ? "stop mining to switch" : "";
  });
}

// ── grid ───────────────────────────────────────────────────────────────────

export function render() {
  renderGrid($("grid"), modulesFor(state.tab, state.pool, state.info?.low_power_supported !== false), buildView(state));
  renderUpdatePill();
  renderLowPowerDetected();
  $("guide-why").textContent = lowPowerGuideText(state.trials);
}

// ── snapshots: the one path every xmrange://snapshot takes ─────────────────

/** Fold one snapshot into state (sparkline series, accumulated tape) and re-render. */
export function applySnapshot(snap) {
  if (!snap || typeof snap !== "object") return;
  state.snap = snap;
  const hs = num(snap.hs10);
  if (hs !== null) {
    state.series.push(hs);
    if (state.series.length > SERIES_MAX) state.series.splice(0, state.series.length - SERIES_MAX);
  }
  const p = snap.power;
  if (p && typeof p === "object") {
    state.powerSeries.push({ w: num(p.sys_w), f: toF(p.temp_c) });
    if (state.powerSeries.length > POWER_MAX) state.powerSeries.splice(0, state.powerSeries.length - POWER_MAX);
  }
  if (snap.context && typeof snap.context === "object") state.context = snap.context;
  trackMismatch(snap);
  const lines = Array.isArray(snap.tape) ? snap.tape : [];
  pushTape(lines);
  // A launch failure arrives as snapshot.error with no tape line of its own.
  if (snap.error && !lines.some((l) => l && l.tag === "error" && l.msg === snap.error) && snap.error !== state.lastError) {
    tapeError(String(snap.error));
  }
  state.lastError = snap.error || null;
  if (snap.prefs && state.settings) state.settings = { ...state.settings, optimizer: snap.prefs };
  const rec = num(snap.optimizer?.trials_recorded);
  if (rec !== null && rec !== state.trialsRecorded) {
    state.trialsRecorded = rec;
    if (rec > 0) loadTrials(); // a trial just landed in the Ledger
  }
  followAsk(snap.optimizer?.ask_low_power);
  renderLive();
  render();
}

/**
 * M2: while xmrig runs, compare what it runs (the snapshot) with what the
 * cards say (app_info wallet, settings threads). A difference that outlasts
 * MISMATCH_TICKS snapshots (a failed live change, never the one-tick lag of
 * a change in flight) makes the card say "not applied · see tape".
 */
function trackMismatch(snap) {
  const running = isRunning(runStateOf(snap));
  const walletOff = running && typeof snap.wallet_masked === "string" && typeof state.info?.wallet?.masked === "string" &&
    snap.wallet_masked !== state.info.wallet.masked;
  // The cores card says a count and, by its e pill, a clamp: either one differing is "not applied"
  // (a clamp macOS refused leaves the count right and the pill wrong otherwise).
  const threadsOff = running && ((num(snap.threads) !== null && shownThreads(state) !== null && snap.threads !== shownThreads(state)) ||
    (typeof snap.clamp === "boolean" && state.settings !== null && snap.clamp !== shownClamp(state)));
  state.mismatch = {
    wallet: walletOff ? state.mismatch.wallet + 1 : 0,
    threads: threadsOff ? state.mismatch.threads + 1 : 0,
  };
}

export const notApplied = (kind) => (state.mismatch?.[kind] ?? 0) >= MISMATCH_TICKS;

/**
 * The engine ended: the light goes to "stopped" and rate figures to "—" (counts
 * stay for the session). If it ended in an error (xmrig died, or never
 * launched), "error" and its reason stay until the next start (I1): the
 * stopped event that always follows an error snapshot must not wipe it. An
 * event from an older engine (its generation is not the one this UI started
 * last) is stale and ignored.
 */
export function applyStopped(gen) {
  if (typeof gen === "number" && typeof state.engineGen === "number" && gen !== state.engineGen) return;
  if (state.ui.startAfterStop) {
    const cmd = state.ui.startAfterStop;
    state.ui.startAfterStop = null;
    startMining(cmd);
    return;
  }
  const prev = state.snap || {};
  const failed = prev.state === "error";
  state.snap = {
    ...prev,
    state: failed ? "error" : "stopped",
    error: failed ? prev.error ?? null : null,
    hs10: null, hs60: null, hs15: null, power: null, sys_w_60: null, cpu_w_60: null, sys_w_900: null, hw_sys: null, hw_cpu: null, tape: [],
    // The engine's live optimizer ends with it: no mode, profile, target or sweep, so the profile
    // pill and the cores card read the saved settings from here (a pill clicked while stopped
    // moves at once), and those are read back, since the engine saved its last prefs.
    optimizer: prev.optimizer ? { ...prev.optimizer, mode: undefined, profile: undefined, target: null, sweep: null } : prev.optimizer,
  };
  state.mismatch = { wallet: 0, threads: 0 };
  state.ui.lowPowerAsk = null; // the next session's ask opens the guide again
  renderLive();
  render();
  syncSettings();
}

/**
 * I4: whole-Mac telemetry while nothing mines (xmrange://idle, every 2 s).
 * Ignored while an engine runs: its snapshots own the machine tab then. Each
 * idle sample fills two slots of the power chart, so the chart keeps one slot
 * per second and its "last 13 min" stays true.
 */
export function applyIdle(sample) {
  if (!sample || typeof sample !== "object" || isRunning(runState())) return;
  const p = sample.power && typeof sample.power === "object" ? sample.power : null;
  state.idlePower = p;
  if (p) {
    const slot = { w: num(p.sys_w), f: toF(p.temp_c) };
    state.powerSeries.push(slot, slot);
    if (state.powerSeries.length > POWER_MAX) state.powerSeries.splice(0, state.powerSeries.length - POWER_MAX);
  }
  if (sample.context && typeof sample.context === "object") state.context = sample.context;
  // Not mining: the hashrate chart keeps moving at 0 (two 1 s slots per 2 s idle sample).
  if (state.series.length) {
    state.series.push(0, 0);
    if (state.series.length > SERIES_MAX) state.series.splice(0, state.series.length - SERIES_MAX);
  }
  render();
}

// ── mining controls ────────────────────────────────────────────────────────

export async function startMining(cmd = "start_mining") {
  // After an error the dead engine keeps its slot while it shuts xmrig, p2pool and tor down, and
  // a start in that window gets the dead engine's generation back: start again once it stopped.
  const deadGen = state.snap?.state === "error" ? state.engineGen : null;
  state.ui.startedThisSession = true;
  state.ui.optError = null;
  state.series = [];
  state.idlePower = null;
  state.mismatch = { wallet: 0, threads: 0 };
  state.snap = { state: "starting" };
  state.lastError = null;
  renderNotice();
  renderLive();
  render();
  try {
    const gen = await invoke(cmd);
    state.engineGen = typeof gen === "number" ? gen : null;
    state.ui.startAfterStop = gen !== null && gen === deadGen ? cmd : null;
  } catch (err) {
    tapeError(errText(err));
    state.snap = { state: "error", error: errText(err) };
    state.lastError = state.snap.error;
    renderLive();
    render();
  }
}

export async function stopMining() {
  state.ui.startAfterStop = null; // a stop always wins over a pending start
  try {
    await invoke("stop_mining");
  } catch (err) {
    tapeError(errText(err));
  }
  applyStopped();
}

async function sendMining(cmd) {
  try {
    await invoke(cmd);
  } catch (err) {
    tapeError(errText(err));
    render();
  }
}

/**
 * The one busy-guarded run command path: the live light's click (⌥-click
 * stops), the hashrate card's run pills and the sheet's stop mining all come
 * here. The same on both pools: the engine brings p2pool (and Tor) up itself.
 * While a command is in flight the pills render disabled.
 */
export async function liveClick(e) {
  if (state.busy) return;
  const alt = !!e?.altKey;
  const cmd = liveCommand(runState(), alt);
  if (!cmd) return;
  state.lastRunCommand = cmd;
  state.busy = true;
  render();
  try {
    if (cmd === "start_mining") await startMining();
    else if (cmd === "stop_mining") await stopMining();
    else await sendMining(cmd);
  } finally {
    state.busy = false;
    renderLive();
    render();
  }
}

// ── card actions ───────────────────────────────────────────────────────────

/**
 * Save through save_settings, which validates every field (Settings::validate).
 * A refusal rolls the change back and shows verbatim: inline in the settings
 * sheet when the change came from there (`inline`), else on the tape.
 */
async function saveSettings(next, inline = false) {
  const prev = state.settings;
  state.settings = next;
  if (inline) state.ui.settingsError = null;
  render();
  renderSheet(inline);
  try {
    await invoke("save_settings", { settings: next });
    return true;
  } catch (err) {
    state.settings = prev;
    if (inline) state.ui.settingsError = errText(err);
    else tapeError(errText(err));
    render();
    renderSheet(inline);
    return false;
  }
}

/** Mine on the E-cores only, by hand (the light profile's setup): `apply_pick` keeps the clamp. */
async function setECores(on, n) {
  const prev = state.settings;
  state.settings = { ...state.settings, threads: n, e_cores_only: on, utility_manual: false };
  if (state.snap?.optimizer?.mode === "hold") state.snap = { ...state.snap, optimizer: { ...state.snap.optimizer, mode: "manual", target: null } };
  render();
  try {
    await invoke("apply_pick", { threads: n, eCoreClamp: on, utility: false, lowPower: null });
  } catch (err) {
    state.settings = prev;
    tapeError(errText(err));
    render();
  }
  await syncSettings();
}

function toggleECores() {
  if (!state.settings) return;
  const on = !shownClamp(state);
  const eCores = num(state.info?.machine?.e_cores) ?? 4;
  const cur = shownThreads(state) ?? 1;
  setECores(on, on ? Math.min(cur, eCores) : cur);
}

async function stepThreads(delta) {
  if (!state.settings) return;
  if (shownClamp(state)) {
    const eCores = num(state.info?.machine?.e_cores) ?? 4;
    const cur = shownThreads(state) ?? 1;
    const n = Math.min(eCores, Math.max(1, cur + delta));
    if (n !== cur) setECores(true, n);
    return;
  }
  const max = num(state.info?.max_threads) ?? 8;
  // Steps from what the rig runs (the throttle's count while holding); a refusal puts back the
  // user's own saved count, never the throttle's (F9).
  const cur = shownThreads(state) ?? 1;
  const prev = num(state.settings.threads);
  const n = Math.min(max, Math.max(1, cur + delta));
  if (n === cur) return;
  state.settings = { ...state.settings, threads: n };
  // Hand input: the optimizer drops to manual (spec §2); show it before the next snapshot does.
  if (state.snap?.optimizer?.mode === "hold") state.snap = { ...state.snap, optimizer: { ...state.snap.optimizer, mode: "manual", target: null } };
  render();
  try {
    await invoke("set_threads", { threads: n });
  } catch (err) {
    // Roll back only if no later click has moved the count since.
    if (state.settings.threads === n) state.settings = { ...state.settings, threads: prev };
    tapeError(errText(err));
    render();
    return;
  }
  await syncSettings(); // the backend also cleared the e-core and utility choices
}

/** The shared per-period pill (earned + cost): day → week → month → day. */
async function cyclePeriod() {
  if (!state.settings) return;
  const i = PERIODS.indexOf(state.settings.period);
  const period = PERIODS[(i + 1) % PERIODS.length];
  await saveSettings({ ...state.settings, period });
}

/**
 * Electricity rate (cost card and the settings sheet). Text that isn't a number
 * puts the saved rate back; the range (0 to 100 per kWh) is the backend's rule,
 * and its refusal shows verbatim.
 */
export async function setRate(input, inline = false) {
  if (!state.settings) return false;
  const v = parseFloat(input.value);
  if (!Number.isFinite(v)) {
    input.value = String(state.settings.rate_per_kwh ?? "");
    return false;
  }
  if (v === state.settings.rate_per_kwh) return true;
  const ok = await saveSettings({ ...state.settings, rate_per_kwh: v }, inline);
  if (!ok) input.value = String(state.settings.rate_per_kwh ?? "");
  return ok;
}

/** The temperature limit (settings sheet), °F; empty turns it off. The range is the backend's. */
export async function setPauseAbove(input) {
  if (!state.settings) return false;
  const raw = String(input.value ?? "").trim();
  const v = raw === "" ? null : parseFloat(raw);
  const guard = state.settings.guard ?? {};
  if (v !== null && !Number.isFinite(v)) {
    input.value = guard.pause_above_f ?? "";
    return false;
  }
  if (v === (guard.pause_above_f ?? null)) return true;
  const ok = await saveSettings({ ...state.settings, guard: { ...guard, pause_above_f: v } }, true);
  if (!ok) input.value = state.settings.guard?.pause_above_f ?? "";
  return ok;
}

function setOnlyOnAc(on) {
  if (!state.settings) return;
  const guard = state.settings.guard ?? {};
  if ((guard.only_on_ac === true) === on) return;
  saveSettings({ ...state.settings, guard: { ...guard, only_on_ac: on } }, true);
}

/** The typed XMR price (settings sheet), in the electricity currency. Empty clears it. */
export async function setPrice(input) {
  if (!state.settings) return false;
  const raw = String(input.value ?? "").trim();
  const v = raw === "" ? null : parseFloat(raw);
  if (v !== null && !Number.isFinite(v)) {
    input.value = state.settings.xmr_price ?? "";
    return false;
  }
  if (v === (state.settings.xmr_price ?? null)) return true;
  const ok = await saveSettings({ ...state.settings, xmr_price: v }, true);
  if (!ok) input.value = state.settings.xmr_price ?? "";
  return ok;
}

/** The Network Log (doctrine §2): read live each time it opens. */
export async function openNetworkLog() {
  const fill = (id, lines) => {
    $(id).replaceChildren(...lines.map((l) => Object.assign(document.createElement("li"), { textContent: l })));
  };
  fill("network-now", ["reading…"]);
  fill("network-allowed", []);
  openDialog("network-log", "network-log-close");
  try {
    const log = await invoke("network_log");
    fill("network-now", log.now ?? []);
    fill("network-allowed", log.allowed ?? []);
  } catch (err) {
    fill("network-now", [errText(err)]);
  }
}

/** typed | fetch: turning the feed on asks first (it adds a Tor connection, doctrine §1b). */
function setPriceFeed(on) {
  if (!state.settings || state.settings.price_feed === on) return;
  if (on) {
    openDialog("price-confirm", "price-confirm-cancel");
    return;
  }
  saveSettings({ ...state.settings, price_feed: false }, true);
}

async function confirmPriceFeed() {
  closeDialog("price-confirm");
  if (state.settings) await saveSettings({ ...state.settings, price_feed: true }, true);
}

/** Currency symbol (settings sheet). The rule (1–3 characters, no spaces or control characters) is the backend's; a refusal shows verbatim. */
export async function setCurrency(input) {
  if (!state.settings) return false;
  const v = String(input.value ?? "").trim();
  if (v === state.settings.currency) {
    input.value = v;
    return true;
  }
  const ok = await saveSettings({ ...state.settings, currency: v }, true);
  if (!ok) input.value = state.settings.currency ?? "";
  return ok;
}

/** h/w counts: whole mac | cpu only (tuning card and the settings sheet). Drives every H/W figure. */
export async function setMeasure(measure, inline = false) {
  if (!state.settings || !MEASURES.includes(measure) || state.settings.measure === measure) return;
  await saveSettings({ ...state.settings, measure }, inline);
}

/** Theme (settings sheet): system | light | dark, applied at once and saved. */
export async function setTheme(theme, inline = false) {
  if (!THEMES.includes(theme)) return;
  applyTheme(theme);
  if (!state.settings || state.settings.theme === theme) {
    renderSheet();
    return;
  }
  if (!(await saveSettings({ ...state.settings, theme }, inline))) applyTheme(state.settings.theme);
}

/** Worker: blur or Enter saves through set_worker; a refusal shows inline, verbatim. */
export async function commitWorker(value) {
  if (!state.settings) return;
  const v = String(value ?? "").trim();
  if (v === state.settings.worker) {
    state.ui.workerError = null;
    state.ui.workerDraft = null;
    render();
    return;
  }
  // Optimistic, as stepThreads is: a settings save sent while this is in flight
  // (the period pill, the theme) already carries the new name (M3).
  const prev = state.settings.worker;
  state.settings = { ...state.settings, worker: v };
  try {
    await invoke("set_worker", { worker: v });
    state.ui.workerError = null;
    state.ui.workerDraft = null;
  } catch (err) {
    if (state.settings.worker === v) state.settings = { ...state.settings, worker: prev };
    state.ui.workerError = errText(err);
    state.ui.workerDraft = String(value ?? "");
  }
  render();
}

function walletInput() {
  return document.querySelector('#grid [data-field="wallet"]');
}

/**
 * The wallet card holds its DOM while its input has focus, so the live hint
 * (and a save error) is written straight into the hint element as well.
 */
function patchWalletHint() {
  const el = document.querySelector("#grid .wallet__hint");
  if (!el) return;
  const h = walletHint(state.ui.walletDraft, state.ui.walletError);
  el.textContent = h.hint;
  el.className = `wallet__hint${h.hintState ? " is-" + h.hintState : ""}`;
}

/** The worker card's pencil pill: the same edit mode a tap on the name gives (the input, focused, caret at the end). */
export function workerEdit() {
  const el = document.querySelector('#grid [data-field="worker"]');
  if (!el) return;
  el.focus();
  el.setSelectionRange(el.value.length, el.value.length);
}

/** p2pool reads the wallet only at start (main.rs P2POOL_WALLET_LOCKED): the editor says so up front. */
const P2POOL_WALLET_LOCKED = "stop mining to change the wallet · p2pool reads it only at start";

export function walletEdit() {
  state.ui.walletEditing = true;
  state.ui.walletDraft = "";
  state.ui.walletError = state.pool === "p2" && isRunning(runState()) ? P2POOL_WALLET_LOCKED : null;
  render();
  walletInput()?.focus();
}

function walletType(value) {
  state.ui.walletDraft = String(value).trim();
  state.ui.walletError = null;
  patchWalletHint();
}

function walletCancel() {
  state.ui.walletEditing = false;
  state.ui.walletDraft = "";
  state.ui.walletError = null;
  walletInput()?.blur();
  render();
}

/** Every address goes through the backend's set_wallet (full Monero checksum); its error shows verbatim. */
async function walletSave() {
  if (state.ui.walletBusy) return;
  const address = state.ui.walletDraft;
  if (!address) {
    walletInput()?.focus();
    return;
  }
  state.ui.walletBusy = true;
  const saveBtn = document.querySelector('#grid [data-act="wallet-save"]');
  if (saveBtn) saveBtn.disabled = true; // the card may be held (focused input), so set it directly too
  render();
  try {
    const view = await invoke("set_wallet", { address });
    state.info = { ...state.info, wallet: view };
    state.ui.walletEditing = false;
    state.ui.walletDraft = "";
    state.ui.walletError = null;
    walletInput()?.blur();
  } catch (err) {
    state.ui.walletError = errText(err);
    patchWalletHint();
  } finally {
    state.ui.walletBusy = false;
    if (saveBtn) saveBtn.disabled = false;
  }
  renderNotice();
  render();
}

/** "use default": one wallet edit at a time, the same busy guard as walletSave (T3 review m1). */
async function walletClear() {
  if (state.ui.walletBusy) return;
  state.ui.walletBusy = true;
  render();
  try {
    const view = await invoke("clear_wallet");
    state.info = { ...state.info, wallet: view };
  } catch (err) {
    tapeError(errText(err));
  } finally {
    state.ui.walletBusy = false;
  }
  renderNotice();
  render();
}

Object.assign(actions, {
  "threads-down": () => stepThreads(-1),
  "threads-up": () => stepThreads(1),
  "ecores-toggle": () => toggleECores(),
  period: cyclePeriod,
  "wallet-edit": walletEdit,
  "worker-edit": workerEdit,
  "wallet-cancel": walletCancel,
  "wallet-save": walletSave,
  "wallet-clear": walletClear,
  // Run pills on the hashrate card: the live light's own busy-guarded path.
  "run-primary": () => liveClick({ altKey: false }),
  "run-stop": () => liveClick({ altKey: true }),
  measure: (b) => setMeasure(b.dataset.val),
  guide: (b) => openGuide(b?.dataset?.val),
  profile: (b) => pickProfile(b.dataset.val),
  "trials-go": () => trialsGo(),
  "trials-pause": () => trialsCmd("pause_mining"),
  "trials-resume": () => trialsCmd("resume_mining"),
  "trials-stop": () => trialsStop(),
  "trials-clear": () => trialsReset(),
  "trials-skip": () => trialsCmd("skip_low_power"),
  "apply-trial": (b) => applyTrial(b),
});

/** Repaint everything from state (the self-check uses it after restoring state). */
export function refresh() {
  renderNav();
  renderLive();
  renderNotice();
  render();
  renderSheet();
}

export function setTab(tab) {
  if (!TABS.includes(tab)) return;
  state.tab = tab;
  if (tab === "tuning") loadTrials();
  renderNav();
  render();
}

/** The pool pill is the saved `pool` setting, applied at the next start; locked while mining. */
export function setPool(pool) {
  if (!POOLS.includes(pool) || pool === state.pool) return;
  if (isRunning(runState())) return;
  const prev = state.pool;
  state.pool = pool;
  if (state.settings) {
    // A refused save puts the pill back with the settings, so the tab never shows a pool that won't start.
    saveSettings({ ...state.settings, pool: pool === "p2" ? "p2pool" : "mo" }).then((ok) => {
      if (ok || state.pool !== pool) return;
      state.pool = prev;
      renderNav();
      renderLive();
      render();
    });
  }
  renderNav();
  renderLive();
  render();
  pollNode();
}

/** The user's own Monero node (p2pool mode), one source: the node_status probe. The payouts card
 * reads the Ledger on the same beat (p2pool's payouts land there as they happen). */
async function pollNode() {
  if (state.pool !== "p2") return;
  try {
    state.node = await invoke("node_status");
  } catch {
    state.node = null;
  }
  try {
    state.payouts = await invoke("payout_totals");
  } catch {
    state.payouts = null;
  }
  renderNodeReady();
  render();
}

function renderNodeReady() {
  const el = $("node-guide-ready");
  if (el) el.hidden = state.node?.state !== "ready";
}

/** Local date and time for a file name, "2026-10-02-14-32-07", so two saves a day never share one. */
export function fileStamp(d) {
  const p = (n) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}-${p(d.getHours())}-${p(d.getMinutes())}-${p(d.getSeconds())}`;
}

/** The node guide's copy button: the flag as shown, onto the clipboard (the webview can't copy). */
async function copyNodeFlag() {
  const btn = $("node-flag-copy");
  try {
    await invoke("copy_text", { text: $("node-flag").textContent.trim() });
    btn.textContent = "copied";
  } catch {
    btn.textContent = "copy failed";
  }
  setTimeout(() => (btn.textContent = "copy"), 1500);
}

/** ⚙️ save diagnostics: the user picks where (save dialog); the on-screen tape plus app facts,
 * scrubbed in Rust (no wallet, no IPs, no names), are written there. */
async function saveDiagnostics() {
  const note = $("diag-note");
  const path = await dialogHooks.pickSave(`xmrange-diagnostics-${fileStamp(new Date())}.txt`);
  if (!path) return;
  try {
    const tape = state.tape.map((l) => `${l.ts} ${l.tag} ${l.msg}`);
    note.textContent = `saved ${await invoke("save_diagnostics", { tape, hours: state.hours, path })}`;
  } catch (err) {
    note.textContent = errText(err);
  }
}

/** 📮 feedback (doctrine §1d): the report is built in Rust, shown whole and editable, and goes
 * out only when the user presses send. `edited` keeps their edits when the kind or note changes. */
const feedback = { kind: "problem", edited: false, busy: false };

function fbSay(text) {
  $("fb-status").textContent = text;
}

async function fbBuild() {
  fbSay("");
  try {
    const tape = state.tape.map((l) => `${l.ts} ${l.tag} ${l.msg}`);
    $("fb-text").value = await invoke("report_text", { kind: feedback.kind, note: $("fb-note").value, tape, hours: state.hours });
    feedback.edited = false;
  } catch (err) {
    fbSay(errText(err));
  }
}

function fbKind(kind) {
  feedback.kind = kind;
  for (const b of $("fb-kind").querySelectorAll("[data-kind]")) b.setAttribute("aria-pressed", String(b.dataset.kind === kind));
}

/** Opens the sheet; `kind` "crash" (from the crash offer) shows the crash choice. */
export async function openFeedback(kind = "problem") {
  $("fb-kind").querySelector('[data-kind="crash"]').hidden = kind !== "crash";
  fbKind(kind);
  $("fb-note").value = "";
  feedback.busy = false;
  $("fb-send").disabled = false;
  openDialog("feedback", "fb-note");
  await fbBuild();
}

async function fbCopy() {
  try {
    await invoke("copy_text", { text: $("fb-text").value });
    fbSay("copied");
  } catch (err) {
    fbSay(errText(err));
  }
}

async function fbSave() {
  const path = await dialogHooks.pickSave(`xmrange-report-${fileStamp(new Date())}.txt`, "save report");
  if (!path) return;
  try {
    fbSay(`saved ${await invoke("save_report", { text: $("fb-text").value, path })}`);
  } catch (err) {
    fbSay(errText(err));
  }
}

async function fbSend() {
  if (feedback.busy) return;
  feedback.busy = true;
  $("fb-send").disabled = true;
  fbSay("sending through tor");
  try {
    const r = await invoke("send_report", { kind: feedback.kind, text: $("fb-text").value });
    fbSay(`sent to ${r.sent} relay${r.sent === 1 ? "" : "s"}`);
  } catch (err) {
    const m = String(err?.message ?? err);
    if (m.startsWith("rescrubbed:\n")) {
      $("fb-text").value = m.slice("rescrubbed:\n".length);
      fbSay("scrubbed more: read the text, then press send again");
    } else {
      fbSay("couldn't send — copy instead");
    }
  }
  feedback.busy = false;
  $("fb-send").disabled = false;
}

/** ⚙️ tor: route the mode's connections through the bundled Tor client (next start). */
async function setTor(on) {
  if (!state.settings || isRunning(runState()) || state.settings.tor === on) return;
  await saveSettings({ ...state.settings, tor: on }, true);
  renderNotice(); // what the pool sees follows Tor
  renderLive();
}

// ── dialogs: the settings sheet and the low power guide ────────────────

const DIALOGS = ["sheet", "guide", "node-guide", "price-confirm", "network-log", "feedback"];
const dialogReturnFocus = {};

function openDialog(id, focusId) {
  const el = $(id);
  if (!el.hidden) return;
  dialogReturnFocus[id] = document.activeElement;
  el.hidden = false;
  $(focusId)?.focus();
}

function closeDialog(id) {
  const el = $(id);
  if (el.hidden) return;
  el.hidden = true;
  const back = dialogReturnFocus[id];
  if (back && typeof back.focus === "function") back.focus();
  dialogReturnFocus[id] = null;
}

/** Esc closes whichever dialog is open. */
function closeOpenDialog() {
  for (const id of DIALOGS) closeDialog(id);
}

export function openSheet() {
  state.ui.settingsError = null;
  renderSheet(true);
  openDialog("sheet", "sheet-close");
}

export function closeSheet() {
  closeDialog("sheet");
}

/**
 * Guided walkthroughs, by topic. `guideHooks.lowpower` (below) fills and opens the sheet; a card's
 * guide pill and the Low Power Mode ask (`followAsk`) both call it through `openGuide`/directly.
 */
export const guideHooks = {};

guideHooks.node = () => {
  pollNode();
  openDialog("node-guide", "node-guide-close");
};

export function openGuide(topic = "lowpower") {
  const hook = guideHooks[topic];
  if (typeof hook === "function") {
    hook();
    return;
  }
  openDialog("guide", "guide-close");
}

export function closeGuide() {
  closeDialog("guide");
}

/** Low Power Mode has one source: the app's 2 s watch (xmrange://lowpower), mining or not. */
export function applyLowPower(v) {
  const next = v === true || v === false ? v : null;
  const changed = next !== state.lowPower;
  state.lowPower = next;
  renderLowPowerDetected();
  render();
  // The curve, trial counts and profiles are per Low Power Mode state: re-read them on a change.
  if (changed) loadTrials();
}

/** Low Power Mode as last seen; `null` before the watch reports. */
export function lowPowerDetected() {
  return state.lowPower ?? null;
}

/** The guide's "✓ detected" chip: shown only once Low Power Mode reads on. */
function renderLowPowerDetected() {
  const el = $("guide-detected");
  if (!el) return;
  el.hidden = lowPowerDetected() !== true;
}

/** Fills the walkthrough (static in index.html beyond the live chip) and opens it. */
guideHooks.lowpower = () => {
  renderLowPowerDetected();
  openDialog("guide", "guide-close");
};

// ── tuning: throttle pills, the trials progress panel, the Low Power Mode ask ──

/**
 * The Low Power Mode ask (design §6.2), from the sweep or a held throttle: open the guide once per ask. Plan 5's
 * walkthrough, once registered, fills the sheet for this direction first.
 */
function followAsk(want) {
  const key = want === true || want === false ? String(want) : null;
  if (key === state.ui.lowPowerAsk) return;
  state.ui.lowPowerAsk = key;
  if (key === null) return;
  guideHooks.lowpower?.(want);
  openDialog("guide", "guide-close");
}

const PROFILE_IDS = PROFILES.map((p) => p.id);

/** A refused optimizer command shows inline where it was pressed (P4-C19), and on the tape. */
function optRefused(at, err) {
  const msg = errText(err);
  state.ui.optError = { at, msg };
  tapeError(msg);
  render();
}

/** A throttle pill: the backend applies its pick and holds it. */
async function pickProfile(id) {
  if (!PROFILE_IDS.includes(id) || !state.settings) return;
  const prev = state.settings;
  state.settings = { ...prev, optimizer: { ...(prev.optimizer ?? {}), profile: id, mode: "hold" } };
  state.ui.optError = null;
  render();
  try {
    await invoke("set_profile", { profile: id });
  } catch (err) {
    state.settings = prev;
    optRefused("profile", err);
    return;
  }
  await syncSettings();
}

/** Start / continue / run trials again: mining first if stopped, then the sweep. */
async function trialsGo() {
  // A second click while the first is still starting xmrig would reach `seek` before the engine
  // exists and be refused: one click at a time, and none while starting.
  if (state.ui.trialsBusy || runState() === "starting") return;
  state.ui.trialsBusy = true;
  state.ui.optError = null;
  render();
  try {
    if (!isRunning(runState())) {
      // Launches xmrig straight onto the next trial's setup and resumes the sweep (start_trials).
      await startMining("start_trials");
      return;
    }
    await invoke("seek");
  } catch (err) {
    optRefused("seeker", err);
  } finally {
    state.ui.trialsBusy = false;
  }
}

async function trialsCmd(cmd) {
  state.ui.optError = null;
  try {
    await invoke(cmd);
  } catch (err) {
    optRefused("seeker", err);
  }
  render();
}

/** Stop trials = stop mining; the log says so, and the trials list reloads for "continue". */
async function trialsStop() {
  state.ui.optError = null;
  state.tape.push({ ts: clockNow(), tag: "trial", msg: "stopping" });
  render();
  try {
    await invoke("stop_seek");
    state.tape.push({ ts: clockNow(), tag: "trial", msg: "stopped" });
    await loadTrials();
  } catch (err) {
    optRefused("seeker", err);
  }
  render();
}

const clockNow = () => new Date().toTimeString().slice(0, 8);

/** A trials-card row, clicked (P5 "pick a result"): run that exact measured setup and switch to
 * manual. A mismatched Low Power Mode state shows the usual ask (`followAsk`, off the next
 * snapshot) instead of doing nothing. */
async function applyTrial(b) {
  const threads = Number(b.dataset.threads);
  if (!Number.isFinite(threads) || threads < 1) return;
  const eCoreClamp = b.dataset.clamp === "1";
  const utility = b.dataset.utility === "1";
  const lp = b.dataset.lp;
  const lowPower = lp === "true" ? true : lp === "false" ? false : null;
  state.ui.optError = null;
  // The same facts the backend saves, at once, so the cores card never reads "not applied" for a
  // pick that worked (owner, 2026-10-04: one source per fact, synced everywhere).
  state.settings = { ...state.settings, threads, e_cores_only: eCoreClamp, utility_manual: utility, optimizer: { ...(state.settings?.optimizer ?? {}), mode: "manual" } };
  if (state.snap?.optimizer?.mode === "hold") state.snap = { ...state.snap, optimizer: { ...state.snap.optimizer, mode: "manual", target: null } };
  render();
  try {
    await invoke("apply_pick", { threads, eCoreClamp, utility, lowPower });
  } catch (err) {
    optRefused("seeker", err);
  }
  await syncSettings();
}

/** The window's settings, read back from the backend after a command that saved them: the
 * backend's settings.json is the one source. */
/** The fields only backend commands change (main.rs accept_settings keeps them): the read-back
 * takes these, so a theme or period click made while it was in flight is never undone. */
const BACKEND_OWNED = ["threads", "e_cores_only", "utility_manual", "optimizer", "worker", "wallet", "profile_log", "update_folder", "node_dir"];

async function syncSettings() {
  let fresh;
  try {
    fresh = await invoke("get_settings");
  } catch {
    return;
  }
  const next = { ...(state.settings ?? fresh) };
  for (const k of BACKEND_OWNED) next[k] = fresh[k];
  state.settings = next;
  render();
  renderSheet();
}

/** Where the wallet lives, plain text in `settings.json` — one place, one build, never a claim
 * that changes. */
export const WALLET_STORE_NOTE = "saved on this Mac, in XMRange's settings file";

/** Sync the sheet's controls from state. A focused field is left alone unless `force`. */
function renderSheet(force = false) {
  const s = state.settings || {};
  const keep = (el) => !force && document.activeElement === el;
  const rate = $("set-rate");
  if (!keep(rate)) rate.value = num(s.rate_per_kwh) === null ? "" : String(s.rate_per_kwh);
  const cur = $("set-currency");
  if (!keep(cur)) cur.value = typeof s.currency === "string" ? s.currency : "";
  const theme = THEMES.includes(s.theme) ? s.theme : "system";
  document.querySelectorAll("#set-theme [data-theme-pick]").forEach((b) => {
    const on = b.dataset.themePick === theme;
    b.classList.toggle("is-on", on);
    b.setAttribute("aria-pressed", String(on));
  });
  const measure = s.measure === "cpu" ? "cpu" : "sys";
  document.querySelectorAll("#set-measure [data-measure]").forEach((b) => {
    const on = b.dataset.measure === measure;
    b.classList.toggle("is-on", on);
    b.setAttribute("aria-pressed", String(on));
  });
  document.querySelectorAll("#set-tor [data-tor]").forEach((b) => {
    const on = (b.dataset.tor === "on") === (s.tor === true);
    b.classList.toggle("is-on", on);
    b.setAttribute("aria-pressed", String(on));
  });
  const chain = s.p2pool_chain ?? "nano";
  document.querySelectorAll("#set-chain [data-chain]").forEach((b) => {
    const on = b.dataset.chain === chain;
    b.classList.toggle("is-on", on);
    b.setAttribute("aria-pressed", String(on));
  });
  const node = s.node === "own" ? "own" : "gui";
  document.querySelectorAll("#set-node [data-node]").forEach((b) => {
    const on = b.dataset.node === node;
    b.classList.toggle("is-on", on);
    b.setAttribute("aria-pressed", String(on));
  });
  $("node-folder-row").hidden = node !== "own";
  $("node-folder-name").textContent = s.node_dir ? folderBaseName(s.node_dir) : "none";
  $("node-folder-name").title = s.node_dir ?? "";
  const guard = s.guard ?? {};
  const pause = $("set-pause-above");
  if (!keep(pause)) pause.value = guard.pause_above_f ?? "";
  // Battery means nothing on a desktop (the spec table says which this Mac is).
  $("set-ac-row").hidden = state.info?.form === "desktop";
  document.querySelectorAll("#set-ac [data-ac]").forEach((b) => {
    const on = (b.dataset.ac === "on") === (guard.only_on_ac === true);
    b.classList.toggle("is-on", on);
    b.setAttribute("aria-pressed", String(on));
  });
  const usd = s.currency === "$";
  const feedOn = s.price_feed === true && usd;
  document.querySelectorAll("#set-price-feed [data-feed]").forEach((b) => {
    const on = (b.dataset.feed === "on") === feedOn;
    b.classList.toggle("is-on", on);
    b.setAttribute("aria-pressed", String(on));
    b.disabled = b.dataset.feed === "on" && !usd;
    b.title = b.disabled ? "the feed is in us dollars: set the currency to $" : "";
  });
  $("set-price-cur").textContent = typeof s.currency === "string" ? s.currency : "$";
  const price = $("set-price");
  price.disabled = feedOn;
  if (!keep(price)) price.value = feedOn ? (num(state.price?.usd) === null ? "" : num(state.price.usd).toFixed(2)) : s.xmr_price ?? "";
  $("set-price-note").textContent = feedOn ? priceNote(state.price) : "";
  $("set-wallet-note").textContent = WALLET_STORE_NOTE;
  const err = $("sheet-err");
  err.textContent = state.ui.settingsError || "";
  err.hidden = !state.ui.settingsError;
}

/** The fetched price's line: "last updated 14:32", or that it's on its way or will retry. */
function priceNote(feed) {
  if (!feed) return "updating…";
  if (num(feed.usd) === null) return "update failed · next try in 5 min";
  const d = new Date(num(feed.t) * 1000);
  return `last updated ${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

// ── the update pill (top-left corner): the backend asks GitHub through Tor ──────────────────

/** The backend's latest verified GitHub offer (doctrine §1c), read at launch; later changes arrive
 * as xmrange://update. */
export async function checkForUpdate() {
  try {
    state.update = await invoke("update_status");
  } catch {
    state.update = null;
  }
  render();
}

function renderUpdatePill() {
  const el = $("update-pill");
  if (!el) return;
  const u = state.update;
  el.hidden = !u;
  el.disabled = state.updateBusy;
  el.textContent = !u ? "" : state.updateBusy ? `update ${u.version} · downloading…` : `update ${u.version} → restart`;
}

/** Stops mining, swaps the bundle in, relaunches and exits (backend `install_update`/`run_install`).
 * On success the app is gone; a refusal (still mining after 20 s, a bad copy, an unsigned bundle,
 * dev build) shows on the tape and the pill stays put. */
async function installUpdate() {
  if (state.updateBusy || !state.update) return;
  state.updateBusy = true;
  renderUpdatePill();
  try {
    await invoke("install_update");
  } catch (err) {
    tapeError(errText(err));
  } finally {
    state.updateBusy = false;
    renderUpdatePill();
  }
}

/** The last segment of a folder path, for the sheet's display name (the full path stays in its tooltip). */
export function folderBaseName(path) {
  const parts = String(path).split("/").filter(Boolean);
  return parts.length ? parts[parts.length - 1] : String(path);
}

/**
 * The macOS folder picker (tauri-plugin-dialog), called through this indirection so the self-check
 * can stub it (`window.__TAURI__` itself is frozen in that build). The open panel grants access to
 * whatever the user picks and triggers no TCC prompt.
 */
export const dialogHooks = {
  pickFolder: (title) => window.__TAURI__.dialog.open({ directory: true, multiple: false, title }),
  pickSave: (name, title = "save diagnostics") => window.__TAURI__.dialog.save({ title, defaultPath: name, filters: [{ name: "Text", extensions: ["txt"] }] }),
  confirmClear: () =>
    window.__TAURI__.dialog.ask("Delete all trials? The next run starts at trial 1.", { title: "reset trials", kind: "warning", okLabel: "reset", cancelLabel: "cancel" }),
};

/** Trials card "reset trials": a native confirm, then every trial is deleted. Nothing starts. */
async function trialsReset() {
  if (!(await dialogHooks.confirmClear())) return;
  try {
    await invoke("clear_trials");
    await loadTrials();
    await syncSettings(); // the backend dropped the held setup and any sweep with them
  } catch (err) {
    state.ui.optError = { at: "seeker", msg: errText(err) };
  }
  render();
}

/** p2pool's chain and node: saved settings, read at the next start (locked while mining). */
async function setChain(chain) {
  if (!state.settings || isRunning(runState()) || (state.settings.p2pool_chain ?? "nano") === chain) return;
  await saveSettings({ ...state.settings, p2pool_chain: chain }, true);
}

async function setNode(node) {
  if (!state.settings || isRunning(runState()) || (state.settings.node ?? "gui") === node) return;
  await saveSettings({ ...state.settings, node }, true);
  pollNode();
}

/** "choose folder…" for the chain: the native folder picker, then set_node_folder checks the
 * drive (format, write access, free space); a refusal shows in the sheet, verbatim. */
export async function chooseNodeFolder() {
  let picked;
  try {
    picked = await dialogHooks.pickFolder("choose the folder for the monero chain");
  } catch (err) {
    state.ui.settingsError = errText(err);
    renderSheet(true);
    return false;
  }
  if (typeof picked !== "string" || !picked) return true; // cancelled
  state.ui.settingsError = null;
  try {
    await invoke("set_node_folder", { folder: picked });
  } catch (err) {
    state.ui.settingsError = errText(err);
    renderSheet(true);
    return false;
  }
  await syncSettings();
  return true;
}

// ── wiring ─────────────────────────────────────────────────────────────────

/** Mark the card under a pressed pointer, so no render swaps it out mid-click. */
function wirePress(grid) {
  let held = null;
  const release = () => {
    if (!held) return;
    held.__pressed = false;
    held = null;
    setTimeout(render, 0); // after the click that this press becomes
  };
  grid.addEventListener("pointerdown", (e) => {
    const card = e.target.closest(".card");
    if (!card) return;
    held = card;
    card.__pressed = true;
  });
  document.addEventListener("pointerup", release);
  document.addEventListener("pointercancel", release);
  window.addEventListener("blur", release);
}

function wireInputs(grid) {
  grid.addEventListener("input", (e) => {
    if (e.target.dataset.field === "wallet") walletType(e.target.value);
  });
  grid.addEventListener("change", (e) => {
    if (e.target.dataset.field === "rate") setRate(e.target);
  });
  grid.addEventListener("focusout", (e) => {
    if (e.target.dataset.field === "worker") commitWorker(e.target.value);
  });
  grid.addEventListener("keydown", (e) => {
    const field = e.target.dataset?.field;
    if (field === "wallet") {
      if (e.key === "Enter") walletSave();
      else if (e.key === "Escape") {
        e.stopPropagation();
        walletCancel();
      }
    } else if (field === "worker" || field === "rate") {
      if (e.key === "Enter") e.target.blur();
      else if (e.key === "Escape" && field === "worker") {
        e.stopPropagation();
        e.target.value = state.settings?.worker ?? "";
        state.ui.workerError = null;
        state.ui.workerDraft = null;
        e.target.blur();
      }
    }
  });
}

function wire() {
  $("tabs").addEventListener("click", (e) => {
    const b = e.target.closest("[data-tab]");
    if (b) setTab(b.dataset.tab);
  });
  $("pools").addEventListener("click", (e) => {
    const b = e.target.closest("[data-pool]");
    if (b) setPool(b.dataset.pool);
  });
  $("theme-btn").addEventListener("click", toggleTheme);
  $("gear-btn").addEventListener("click", openSheet);
  $("sheet-scrim").addEventListener("click", closeSheet);
  $("sheet-close").addEventListener("click", closeSheet);
  $("guide-scrim").addEventListener("click", closeGuide);
  $("guide-close").addEventListener("click", closeGuide);
  $("guide-open-settings").addEventListener("click", async () => {
    try {
      await invoke("open_battery_settings");
    } catch (err) {
      tapeError(errText(err));
    }
  });
  $("update-pill").addEventListener("click", installUpdate);
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") closeOpenDialog();
  });
  $("set-rate").addEventListener("change", (e) => setRate(e.target, true));
  $("set-currency").addEventListener("change", (e) => setCurrency(e.target));
  $("set-price").addEventListener("change", (e) => setPrice(e.target));
  $("set-pause-above").addEventListener("change", (e) => setPauseAbove(e.target));
  $("set-ac").addEventListener("click", (e) => {
    const b = e.target.closest("[data-ac]");
    if (b) setOnlyOnAc(b.dataset.ac === "on");
  });
  $("set-price-feed").addEventListener("click", (e) => {
    const b = e.target.closest("[data-feed]");
    if (b && !b.disabled) setPriceFeed(b.dataset.feed === "on");
  });
  $("price-confirm-ok").addEventListener("click", confirmPriceFeed);
  $("set-network").addEventListener("click", openNetworkLog);
  $("network-log-close").addEventListener("click", () => closeDialog("network-log"));
  $("network-log-scrim").addEventListener("click", () => closeDialog("network-log"));
  $("price-confirm-cancel").addEventListener("click", () => closeDialog("price-confirm"));
  $("price-confirm-scrim").addEventListener("click", () => closeDialog("price-confirm"));
  $("set-theme").addEventListener("click", (e) => {
    const b = e.target.closest("[data-theme-pick]");
    if (b) setTheme(b.dataset.themePick, true);
  });
  $("diag-save").addEventListener("click", saveDiagnostics);
  $("fb-open").addEventListener("click", () => openFeedback("problem"));
  $("feedback-close").addEventListener("click", () => closeDialog("feedback"));
  $("feedback-scrim").addEventListener("click", () => closeDialog("feedback"));
  $("fb-kind").addEventListener("click", (e) => {
    const b = e.target.closest("[data-kind]");
    if (!b || b.dataset.kind === feedback.kind) return;
    fbKind(b.dataset.kind);
    if (!feedback.edited) fbBuild();
  });
  $("fb-note").addEventListener("change", () => !feedback.edited && fbBuild());
  $("fb-text").addEventListener("input", () => (feedback.edited = true));
  $("fb-copy").addEventListener("click", fbCopy);
  $("fb-save").addEventListener("click", fbSave);
  $("fb-send").addEventListener("click", fbSend);
  $("crash-send").addEventListener("click", () => {
    $("crash-offer").hidden = true;
    openFeedback("crash");
  });
  $("crash-dismiss").addEventListener("click", () => ($("crash-offer").hidden = true));
  $("set-tor").addEventListener("click", (e) => {
    const b = e.target.closest("[data-tor]");
    if (b) setTor(b.dataset.tor === "on");
  });
  $("node-guide-scrim").addEventListener("click", () => closeDialog("node-guide"));
  $("node-guide-close").addEventListener("click", () => closeDialog("node-guide"));
  $("node-flag-copy").addEventListener("click", copyNodeFlag);
  $("set-measure").addEventListener("click", (e) => {
    const b = e.target.closest("[data-measure]");
    if (b) setMeasure(b.dataset.measure, true);
  });
  $("set-chain").addEventListener("click", (e) => {
    const b = e.target.closest("[data-chain]");
    if (b && !b.disabled) setChain(b.dataset.chain);
  });
  $("set-node").addEventListener("click", (e) => {
    const b = e.target.closest("[data-node]");
    if (b && !b.disabled) setNode(b.dataset.node);
  });
  $("node-folder-choose").addEventListener("click", chooseNodeFolder);
  $("sheet").addEventListener("keydown", (e) => {
    if (["set-rate", "set-currency", "set-price", "set-pause-above"].includes(e.target.id) && e.key === "Enter") e.target.blur();
  });
  $("grid").addEventListener("click", (e) => {
    const b = e.target.closest("[data-act]");
    if (!b || b.disabled) return;
    const fn = actions[b.dataset.act];
    if (fn) fn(b, e);
  });
  wirePress($("grid"));
  wireInputs($("grid"));
  $("notice-dismiss").addEventListener("click", dismissNotice);
  $("notice-use").addEventListener("click", () => {
    setPool("mo");
    setTab("mining");
    walletEdit();
  });
}

async function loadTrials() {
  try {
    state.trials = await invoke("list_trials");
  } catch {
    state.trials = null;
  }
  render();
}

async function init() {
  wire();
  renderNav();
  renderLive();

  // Listen before reading app_info so no watch change falls between the two.
  await listen("xmrange://lowpower", (event) => applyLowPower(event.payload));
  const [info, settings] = await Promise.all([
    invoke("app_info").catch(() => null),
    invoke("get_settings").catch(() => null),
  ]);
  state.info = info;
  state.price = info?.price ?? null;
  state.settings = settings;
  state.pool = settings?.pool === "p2pool" ? "p2" : "mo";
  renderNav();
  if (state.lowPower === null) state.lowPower = info?.low_power === true || info?.low_power === false ? info.low_power : null;
  state.ui.noticeDismissed = settings?.notice_dismissed === true;
  if (typeof info?.startup_note === "string" && info.startup_note) pushTape([{ ts: clock(), tag: "miner", msg: info.startup_note }]);
  applyTheme(settings?.theme);
  renderNotice();
  $("crash-offer").hidden = info?.crashed_last_time !== true; // offered once: the marker is rewritten at launch
  renderNotice();
  render();
  renderSheet();
  loadTrials();

  // Grid and live light only; the header stays put.
  await listen("xmrange://snapshot", (event) => applySnapshot(event.payload));
  await listen("xmrange://stopped", (event) => applyStopped(event.payload));
  await listen("xmrange://idle", (event) => applyIdle(event.payload));

  // p2pool mode: the local node and the payouts, looked at now and every 10 s while the p2pool tab shows.
  pollNode();
  setInterval(pollNode, NODE_POLL_MS);

  // Updates: the backend asks GitHub (release builds) and says when a new build is offered.
  // The XMR price feed (opt-in, through tor): each fetch's outcome.
  await listen("xmrange://price", (event) => {
    state.price = event.payload ?? null;
    render();
    renderSheet();
  });
  await listen("xmrange://update", (event) => {
    state.update = event.payload ?? null;
    render();
  });
  checkForUpdate();
}

const ready = init().catch((err) => {
  console.error("xmrange ui failed to start", err);
  render();
});

export const ui = {
  ready,
  state,
  actions,
  setTab,
  setPool,
  openSheet,
  closeSheet,
  openGuide,
  fileStamp,
  openNetworkLog,
  setPauseAbove,
  closeGuide,
  guideHooks,
  lowPowerDetected,
  applyLowPower,
  walletEdit,
  workerEdit,
  setMeasure,
  setTheme,
  setCurrency,
  setRate,
  chooseNodeFolder,
  folderBaseName,
  dialogHooks,
  checkForUpdate,
  render,
  refresh,
  applySnapshot,
  applyStopped,
  applyIdle,
  notApplied,
  liveClick,
  liveCommand,
  stopMining,
  commitWorker,
  noticeShown,
  setNoticeVisible,
  openFeedback,
};

// Dev self-check only: the harness is fetched only when app_info.self_check is
// true, which only a dev build in self-check mode reports. A release build
// doesn't contain the harness at all (it lives in ui-dev/, outside this
// frontend). A load failure is still reported, so a self-check run always ends.
ready.then(async () => {
  const info = state.info ?? (await invoke("app_info").catch(() => null));
  if (!info?.self_check) return;
  try {
    const { runSelfCheck } = await import("./selfcheck.js");
    await runSelfCheck(ui);
  } catch (err) {
    const report = { ok: false, checks: { selfcheck_loaded: false }, detail: { error: String(err) } };
    await invoke("self_check_report", { report }).catch(() => null);
  }
});
