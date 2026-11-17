// Docs screenshots: a fake window.__TAURI__ with test data only (no real wallet, IP or host).
// Scenario comes from the page URL: ?s=mining | first-run | wallet | tuning | update.
// Helpers for the runner: window.__shots.feed(n, opts) pushes n snapshots; emit(event, payload).
(() => {
  const S = new URLSearchParams(location.search).get("s") || "mining";
  let seed = 7;
  // Uptime counts from page load: the runner shifts the clock after load (window.__shots.age).
  const realNow = Date.now.bind(Date); let skew = 0; Date.now = () => realNow() + skew;
  const rnd = () => ((seed = (seed * 16807) % 2147483647) / 2147483647) - 0.5; // -0.5..0.5, repeatable
  const FAKE_MASK = "4Tst…Wx7Q"; // fake; format-plausible
  const machine = { model: "Mac14,9", chip: "Apple M2 Pro", p_cores: 6, e_cores: 4, memory_gb: 16 };
  const dflt = S === "first-run";
  const info = {
    version: "0.0.1", tagline: "hashes per watt · every watt, weighed", machine, max_threads: 10,
    low_power: false, low_power_supported: true, form: "laptop", price: null,
    wallet: { masked: dflt ? "47C4…zPB" : FAKE_MASK, is_default: dflt, label: dflt ? "default · ghostdrip dev wallet" : "your wallet" },
    crashed_last_time: S === "crash", dev_build: false, self_check: false, self_check_live: false, startup_note: null,
  };
  const pick5 = { setup: { threads: 5, e_core_clamp: false, utility: false }, low_power: false, hw: 220.53, hs: 3330 };
  const settings = {
    rate_per_kwh: 0.16, currency: "$", theme: new URLSearchParams(location.search).get("t") || "light", measure: "sys", period: "day", worker: "studio-mac", threads: 5,
    update_folder: null, wallet: null, notice_dismissed: !dflt,
    optimizer: { profile: "efficient", mode: "hold", sweep: null, holding: pick5 },
    pool: "mo", tor: true, xmr_price: 160, price_feed: false, guard: { pause_above_f: null, only_on_ac: false },
    profile_log: [], e_cores_only: false, utility_manual: false, p2pool_chain: "nano", node: "gui", node_dir: null,
  };
  // The measured thread curve: threads → H/s and whole-Mac watts.
  const HS = [690, 1370, 2040, 2700, 3330, 3930, 4250, 4520, 4720, 4860];
  const W = [7.5, 9.4, 11.3, 13.2, 15.1, 17.0, 19.3, 21.8, 24.4, 27.2];
  const row = (n, clamp, hs, w, t) => ({
    label: `${n} cores`, hw_sys: +(hs / w).toFixed(1), hw_cpu: +(hs / (w * 0.62)).toFixed(1), noisy: false, t_end: t,
    threads: n, e_core_clamp: clamp, utility: false, low_power: false,
  });
  const mid = S === "tuning";
  const nShown = mid ? 6 : 10;
  const rows = [];
  for (let i = 0; i < nShown; i++) rows.push(row(i + 1, false, HS[i], W[i], 1000 + i));
  rows.push(row(4, true, 1250, 7.1, 990));
  const trials = {
    count: rows.length, rows,
    curve: {
      low_power: false,
      points: HS.slice(0, nShown).map((hs, i) => ({ threads: i + 1, hs, sys_w: W[i], hw_sys: +(hs / W[i]).toFixed(1), hw_cpu: +(hs / (W[i] * 0.62)).toFixed(1) })),
      standout: [5, 630, 1.9], flat_from: mid ? null : 6,
    },
    picks: {
      sys: {
        light: { setup: { threads: 4, e_core_clamp: true, utility: false }, low_power: false, hw: 176.1, hs: 1250 },
        efficient: pick5,
        max: { setup: { threads: mid ? 6 : 10, e_core_clamp: false, utility: false }, low_power: false, hw: mid ? 231.2 : 178.7, hs: mid ? 3930 : 4860 },
      },
      cpu: {},
    },
    low_power_gain: { sys: null, cpu: null },
    progress: mid ? [6, 12] : [12, 12],
  };
  const updateOffer = S === "update" ? { version: "0.0.2", notes: "Faster start." } : null;
  const netLog = {
    now: ["tor → 4 tor relays (port 443, 9001)", "xmrig → the pool through tor"],
    allowed: [
      "moneroocean pool (gulf.moneroocean.stream), through tor",
      "the tor network (tor relays only)",
      "updates: github (house-of-helios/xmrange), at launch and every 6 h, through tor",
    ],
  };
  const handlers = {};
  const cmds = {
    app_info: () => info, get_settings: () => settings, list_trials: () => trials, update_status: () => updateOffer,
    node_status: () => ({ state: "missing" }), payout_totals: () => ({ count: 0, atomic: 0 }), network_log: () => netLog,
    set_wallet: ({ address }) => ({ masked: FAKE_MASK, is_default: false, label: "your wallet" }),
    save_settings: () => null, copy_text: () => null,
    report_text: () => window.__REPORT_TEXT || "",
  };
  const noop = () => null;
  window.__TAURI__ = {
    core: { invoke: async (cmd, args) => (cmds[cmd] || noop)(args) },
    event: { listen: async (ev, fn) => { (handlers[ev] ||= []).push(fn); return () => {}; } },
    dialog: { open: async () => null, save: async () => null, ask: async () => false },
  };
  const emit = (ev, payload) => (handlers[ev] || []).forEach((f) => f({ payload }));

  const tape = [
    ["14:31:12", "net", "connected to pool"], ["14:31:12", "net", "new job algo rx/0"],
    ["14:36:40", "cpu", "accepted (38/0) diff 45401"], ["14:38:55", "cpu", "accepted (39/0) diff 45401"],
    ["14:41:07", "cpu", "accepted (40/0) diff 45401"], ["14:44:19", "cpu", "accepted (41/0) diff 45401"],
    ["14:47:30", "cpu", "accepted (42/0) diff 45401"],
  ].map(([ts, tag, msg]) => ({ ts, tag, msg }));
  const trialTape = [
    "started 1 core", "measured 1 core · 92.0 h/w", "started 2 cores", "measured 2 cores · 145.7 h/w", "started 3 cores",
    "measured 3 cores · 180.5 h/w", "started 4 cores", "measured 4 cores · 204.5 h/w", "started 5 cores", "measured 5 cores · 220.5 h/w",
    "started 6 cores", "measured 6 cores · 231.2 h/w", "started 7 cores", "measuring 7 cores",
  ].map((msg, i) => ({ ts: `14:${String(2 + i * 4).padStart(2, "0")}:00`, tag: "trial", msg })).slice(-5);

  const opt = (over) => ({
    profile: "efficient", mode: "hold", measure: "sys", target: null, sweep: null, held: pick5, ask_low_power: null,
    note: null, trials_recorded: trials.count, ...over,
  });
  const sweep = { testing: { threads: 7, e_core_clamp: false, utility: false }, stage: "measuring", done: 6, planned: 12, elapsed_s: 230, left_s: 290, trial_s: 520 };

  /** Push n snapshots of a healthy Mac; the last one carries the tape and the context. */
  function feed(n = 240, o = {}) {
    let a = 0, b = 0; // slow wobble, so the charts look like a live Mac and not static
    for (let i = 0; i < n; i++) {
      a = a * 0.9 + rnd() * 0.012; b = b * 0.93 + rnd() * 0.02;
      const last = i === n - 1;
      const hs10 = 3340 * (1 + a + rnd() * 0.006);
      const sysw = 15.1 * (1 + b + rnd() * 0.01);
      emit("xmrange://snapshot", {
        t: i, state: "mining", error: null, threads: 5, clamp: false, worker: settings.worker, wallet_masked: FAKE_MASK,
        hs10, hs60: 3335, hs15: 3328, hs_max: 3492, shares_good: 42, shares_rejected: 0, diff: 45401, algo: "rx/0",
        uptime_s: 2 * 3600 + 14 * 60 + 8 + i,
        power: { t: i, cpu_w: sysw * 0.62, ram_w: 1.3, sys_w: sysw, temp_c: 56 + rnd() * 2, p_mhz: 3420, e_mhz: 1150, p_active: 0.84, e_active: 0.06, mem_used_gb: 10.4, mem_total_gb: 16 },
        sys_w_60: 15.1, cpu_w_60: 9.4, sys_w_900: 15.2, hw_sys: 220.5, hw_cpu: 354.8,
        context: i === 0 || last ? { low_power_mode: false, on_ac: true, battery_pct: 100, brightness_pct: 62, thermal: "nominal" } : null,
        low_power: false, tape: last ? o.tape || [] : [], optimizer: o.optimizer ?? opt(), prefs: null, p2pool: null, tor: true, guard_note: null,
      });
    }
  }
  window.__shots = { age: (ms) => { skew = ms; }, feed, emit, tape, trialTape, opt, sweep, scenario: S };
})();
