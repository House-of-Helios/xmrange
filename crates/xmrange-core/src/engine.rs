//! The engine: a background thread that owns the `Miner` and the macmon power sampler, accepts
//! `Command`s, and emits one `Snapshot` per second. The tape (`TapeState`) is a pure,
//! IP-free summary of events synthesized from xmrig's API — never the raw connection details.
use crate::api::Summary;
use crate::config::{mask_wallet, MinerConfig, API_PORT};
use crate::context::{self, Context};
use crate::ledger::{LedgerHandle, TrialRecord};
use crate::levers::Setup;
use crate::lowpower;
use crate::miner::{max_threads, LaunchOptions, Miner};
use crate::optimizer::driver::{setup_note_label, Action, OptCommand, OptTick, Optimizer, OptimizerView};
use crate::trial::TrialStage;
use crate::optimizer::{Limits, OptimizerPrefs};
use crate::paths;
use crate::settings::Measure;
use crate::trial::Tick;
use crate::telemetry::{now_secs, PowerSample};
use crate::units::hw;
use serde::Serialize;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// What the engine needs to launch and identify a mining session. Deliberately carries no
/// `Debug`/`Clone`: the wallet is a real address, and this codebase never derives Debug on
/// anything that holds one unmasked (see `MinerConfig`'s manual `Debug`).
pub struct EngineConfig {
    pub wallet: String,
    pub worker: String,
    pub threads: u32,
    pub e_core_clamp: bool,
    /// Launch with the mining threads at utility QoS (the held or manual setup's).
    pub utility: bool,
    pub dev_log: Option<PathBuf>,
    /// Where finished trials go and past ones come from; `None` = no Ledger this session. The
    /// 1 Hz samples stay in memory: the Ledger keeps trials only (ruling P5-O1).
    pub ledger: Option<LedgerHandle>,
    /// The sweep and the throttles; `None` = the engine only mines.
    pub optimizer: Option<OptimizerInit>,
    /// MoneroOcean or p2pool (doctrine §1a).
    pub pool: crate::settings::Pool,
    /// p2pool mode: the share chain, and the node p2pool talks to.
    pub chain: crate::settings::Chain,
    pub node: crate::settings::NodeChoice,
    /// Route the mode's connections through the bundled Tor client.
    pub tor: bool,
    /// The user's limits: pause above a temperature, or on battery (`guard`).
    pub guard: crate::guard::GuardPrefs,
}

/// How many past trials the optimizer reads at start, to select from and to resume a sweep.
pub const HISTORY_TRIALS: u32 = 500;

/// The optimizer's start-up facts. `None` in `EngineConfig` means the engine only mines.
#[derive(Clone)]
pub struct OptimizerInit {
    pub prefs: OptimizerPrefs,
    pub measure: Measure,
    pub chip: Option<String>,
    pub e_cores: u32,
    /// The user's own thread count (settings.json), the one manual runs (minor M4). None: the
    /// launch count is the user's.
    pub own_threads: Option<u32>,
}

#[derive(Clone)]
pub enum Command {
    SetThreads(u32),
    SetIdentity { wallet: String, worker: String },
    /// New limits (`guard`), live.
    SetGuard(crate::guard::GuardPrefs),
    Pause,
    Resume,
    Stop,
    Optimizer(OptCommand),
}

impl std::fmt::Debug for Command {
    /// Manual impl: `{:?}` must never print the full wallet (see `MinerConfig`'s manual Debug).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Command::SetThreads(n) => f.debug_tuple("SetThreads").field(n).finish(),
            Command::SetIdentity { wallet, worker } => {
                f.debug_struct("SetIdentity").field("wallet", &mask_wallet(wallet)).field("worker", worker).finish()
            }
            Command::SetGuard(g) => f.debug_tuple("SetGuard").field(g).finish(),
            Command::Pause => write!(f, "Pause"),
            Command::Resume => write!(f, "Resume"),
            Command::Stop => write!(f, "Stop"),
            Command::Optimizer(c) => f.debug_tuple("Optimizer").field(c).finish(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RunState {
    Starting,
    Mining,
    Paused,
    Stopped,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TapeLine {
    pub ts: String,
    pub tag: String,
    pub msg: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub t: f64,
    pub state: RunState,
    pub error: Option<String>,
    pub threads: u32,
    /// Whether xmrig runs held to the E-cores right now (the rig's own state, not the request), so
    /// the cores card can tell a clamp that failed. `None` before xmrig runs.
    pub clamp: Option<bool>,
    pub worker: String,
    pub wallet_masked: String,
    pub hs10: Option<f64>,
    pub hs60: Option<f64>,
    /// xmrig's 15-minute average hashrate: the earned card's (ruling P5-O2). `None` in its first
    /// 15 minutes; the card then takes the longest average there is.
    pub hs15: Option<f64>,
    pub hs_max: Option<f64>,
    pub shares_good: u64,
    pub shares_rejected: u64,
    pub diff: u64,
    pub algo: Option<String>,
    pub uptime_s: u64,
    pub power: Option<PowerSample>,
    /// Mean whole-Mac and CPU watts over the last 60 s (`HW_WINDOW`), or over what there is once
    /// that holds 10 s of readings. Display only, like every rolling field (ruling P5-O2).
    pub sys_w_60: Option<f64>,
    pub cpu_w_60: Option<f64>,
    /// Mean whole-Mac watts over the last 15 minutes (`COST_WINDOW`), or what there is past 10 s:
    /// the cost card's.
    pub sys_w_900: Option<f64>,
    /// H/W as a ratio of means over aligned windows: `hs60` over `sys_w_60` / `cpu_w_60`. In
    /// xmrig's first minute, before it has an `hs60`, `hs10` over the last 10 s of watts.
    pub hw_sys: Option<f64>,
    pub hw_cpu: Option<f64>,
    /// Refreshed on the first tick and every 30 ticks thereafter; `None` otherwise (it's a
    /// handful of `pmset`/`ioreg` shell-outs, too slow to justify doing every second).
    pub context: Option<Context>,
    /// Low Power Mode as the 2 s watch last saw it (`lowpower::latest()`); `None` when no watch
    /// runs or the Mac doesn't report it.
    pub low_power: Option<bool>,
    /// Only the NEW tape lines produced this tick.
    pub tape: Vec<TapeLine>,
    /// The optimizer's state for the tuning tab (None when the engine only mines).
    pub optimizer: Option<OptimizerView>,
    /// Set only on ticks where the optimizer's prefs changed: the app saves them to settings.json.
    pub prefs: Option<OptimizerPrefs>,
    /// p2pool mode: p2pool's own figures (its data-api files), read each tick.
    pub p2pool: Option<crate::p2pool::Stats>,
    /// This session's connections go through Tor.
    pub tor: bool,
    /// Why a limit holds mining paused ("paused · on battery"), while it does.
    pub guard_note: Option<String>,
}

fn fmt_opt(v: Option<f64>) -> String {
    v.map(|x| format!("{x:.1}")).unwrap_or_else(|| "n/a".to_string())
}

/// Pure, IP-free synthesis of tape events from consecutive xmrig `Summary` polls. See the tape
/// rules in the Task 5 brief: connect / new-job / difficulty / accepted / rejected / speed.
#[derive(Default)]
pub struct TapeState {
    failures: u64,
    good: u64,
    rejected: u64,
    algo: Option<String>,
    diff: u64,
    ticks: u64,
    connected: bool,
}

impl TapeState {
    pub fn update(&mut self, s: &Summary, ts: &str) -> Vec<TapeLine> {
        let mut out = Vec::new();
        let mut push = |tag: &str, msg: String| out.push(TapeLine { ts: ts.to_string(), tag: tag.to_string(), msg });

        if let Some(algo) = &s.algo {
            if !self.connected {
                self.connected = true;
                push("net", "connected to pool".to_string());
            }
            if self.algo.as_deref() != Some(algo.as_str()) {
                self.algo = Some(algo.clone());
                push("net", format!("new job algo {algo}"));
            }
        }

        // xmrig's own count of pool connections lost after a login (`NetworkState::stop`). A login
        // that never succeeds isn't counted: that shows as "waiting for the pool" instead.
        if s.connection.failures > self.failures {
            self.failures = s.connection.failures;
            push("error", format!("lost the pool connection ({} times)", self.failures));
        }

        let diff = s.results.diff_current;
        if diff != 0 {
            if self.diff == 0 {
                self.diff = diff; // first non-zero value: baseline, no line
            } else if diff != self.diff {
                self.diff = diff;
                push("net", format!("difficulty {diff}"));
            }
        }

        if s.results.shares_good > self.good {
            self.good = s.results.shares_good;
            let rejected_now = s.results.shares_total.saturating_sub(s.results.shares_good);
            push("cpu", format!("accepted ({}/{}) diff {}", self.good, rejected_now, diff));
        }
        let rejected_now = s.results.shares_total.saturating_sub(s.results.shares_good);
        if rejected_now > self.rejected {
            self.rejected = rejected_now;
            push("cpu", format!("rejected ({}/{})", self.good, self.rejected));
        }

        self.ticks += 1;
        if self.ticks % 60 == 0 {
            if let Some(hs10) = s.hs10() {
                push("miner", format!("speed 10s/60s {:.1} {} h/s max {}", hs10, fmt_opt(s.hs60()), fmt_opt(s.hashrate.highest)));
            }
        }

        out
    }
}

/// Trials-sweep tape lines, synthesized from consecutive `OptimizerView`s: started, settling
/// done, waiting (for the outlet) and the Low Power Mode ask. "measured"/"skipped" come from
/// `Action::Record`, where the real score is known (see the engine's main loop).
#[derive(Debug, Default)]
pub struct TrialTapeState {
    testing: Option<Setup>,
    stage: Option<TrialStage>,
    waiting: Option<String>,
    asking: Option<bool>,
}

impl TrialTapeState {
    pub fn update(&mut self, v: &OptimizerView, ts: &str) -> Vec<TapeLine> {
        let mut out = Vec::new();
        let Some(s) = &v.sweep else {
            self.testing = None;
            self.stage = None;
            self.waiting = None;
            return out;
        };
        if s.testing != self.testing {
            self.stage = None;
            if let Some(setup) = s.testing {
                out.push(TapeLine { ts: ts.to_string(), tag: "trial".into(), msg: format!("started {}", setup_note_label(setup, None)) });
            }
            self.testing = s.testing;
        }
        if s.testing.is_some() && s.stage != self.stage {
            if let (Some(TrialStage::Settling), Some(TrialStage::Measuring), Some(setup)) = (self.stage, s.stage, s.testing) {
                out.push(TapeLine { ts: ts.to_string(), tag: "trial".into(), msg: format!("measuring {}", setup_note_label(setup, None)) });
            }
            self.stage = s.stage;
        }
        // Every new optimizer note lands on the log once (a Low Power Mode change, a stop).
        if v.note != self.waiting {
            if let Some(n) = &v.note {
                out.push(TapeLine { ts: ts.to_string(), tag: "trial".into(), msg: n.clone() });
            }
            self.waiting = v.note.clone();
        }
        if v.ask_low_power != self.asking {
            if let Some(want) = v.ask_low_power {
                out.push(TapeLine {
                    ts: ts.to_string(),
                    tag: "trial".into(),
                    msg: format!("low power mode ask: turn it {} in battery settings", if want { "on" } else { "off" }),
                });
            }
            self.asking = v.ask_low_power;
        }
        out
    }
}

/// How many 1-s ticks in a row the API may fail before the live rates are withdrawn (M4).
pub const API_STALE_TICKS: u32 = 5;
/// How long "starting" may wait for a pool job before the tape says so (M5).
pub const POOL_WAIT_TICKS: u64 = 30;
/// The dashboard's rolling windows (ruling P5-O2), in 1 s engine ticks. The shortest is xmrig's
/// 10 s average, and the least power history a rolling figure is shown on.
pub const SHORT_WINDOW: usize = 10;
/// H/W: xmrig's 60 s hashrate over the mean watts of the same 60 s.
pub const HW_WINDOW: usize = 60;
/// Cost and earned: the mean whole-Mac watts over 15 minutes, and xmrig's 15-minute hashrate.
pub const COST_WINDOW: usize = 900;
/// A longer gap than this between two power readings (the Mac slept) starts the windows over, so
/// no window pairs watts from before a sleep with a hashrate from after it.
pub const GAP_S: f64 = 5.0;

/// Watches the engine's own instruments, tick by tick, and says when the live figures can no
/// longer be trusted. Pure, so its rules are unit-tested.
#[derive(Debug, Default)]
pub struct Health {
    api_fails: u32,
    api_flagged: bool,
    pool_flagged: bool,
}

impl Health {
    /// One tick's API poll. Returns the tape line to add at a transition, if any.
    pub fn api(&mut self, answered: bool) -> Option<(&'static str, &'static str)> {
        if answered {
            self.api_fails = 0;
            if self.api_flagged {
                self.api_flagged = false;
                return Some(("miner", "xmrig's api is answering again"));
            }
            return None;
        }
        self.api_fails = self.api_fails.saturating_add(1);
        if self.api_fails >= API_STALE_TICKS && !self.api_flagged {
            self.api_flagged = true;
            return Some(("error", "xmrig's api stopped answering; rates withdrawn"));
        }
        None
    }

    /// True while the API has been silent long enough that the last rates would be frozen numbers.
    pub fn rates_stale(&self) -> bool {
        self.api_flagged
    }

    /// Once per session: still "starting" (no job from the pool) after `POOL_WAIT_TICKS` ticks.
    pub fn pool_wait(&mut self, starting: bool, has_job: bool, ticks: u64) -> Option<(&'static str, &'static str)> {
        if starting && !has_job && ticks >= POOL_WAIT_TICKS && !self.pool_flagged {
            self.pool_flagged = true;
            return Some(("net", "waiting for the pool (check the network)"));
        }
        None
    }
}

/// The last `COST_WINDOW` ticks of watts, a bounded ring with one slot per tick (`None` where the
/// tick had no reading), so a window of n ticks covers the same wall time as the xmrig average it
/// is paired with. Display only: trials and their CIs keep their own windows.
#[derive(Debug, Default)]
pub struct PowerRing {
    /// (whole-Mac W, CPU W), oldest first.
    slots: VecDeque<(Option<f64>, Option<f64>)>,
    last_t: Option<f64>,
}

impl PowerRing {
    pub fn push(&mut self, p: Option<&PowerSample>) {
        if let Some(p) = p {
            if self.last_t.is_some_and(|t| p.t - t > GAP_S) {
                self.slots.clear();
            }
            self.last_t = Some(p.t);
        }
        let finite = |v: Option<f64>| v.filter(|v| v.is_finite());
        self.slots.push_back((finite(p.and_then(|p| p.sys_w)), finite(p.map(|p| p.cpu_w))));
        while self.slots.len() > COST_WINDOW {
            self.slots.pop_front();
        }
    }

    /// Mean whole-Mac watts over the last `n` ticks (or all there are). Unknown readings are
    /// skipped, never read as 0 W; under `SHORT_WINDOW` known readings there is no mean.
    pub fn sys(&self, n: usize) -> Option<f64> {
        self.mean(n, |s| s.0)
    }

    /// `sys`, for the CPU's watts.
    pub fn cpu(&self, n: usize) -> Option<f64> {
        self.mean(n, |s| s.1)
    }

    fn mean(&self, n: usize, pick: impl Fn(&(Option<f64>, Option<f64>)) -> Option<f64>) -> Option<f64> {
        let known: Vec<f64> = self.slots.iter().rev().take(n).filter_map(pick).collect();
        (known.len() >= SHORT_WINDOW).then(|| known.iter().sum::<f64>() / known.len() as f64)
    }
}

/// The Snapshot's rolling figures (ruling P5-O2).
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Rolling {
    pub sys_w_60: Option<f64>,
    pub cpu_w_60: Option<f64>,
    pub sys_w_900: Option<f64>,
    pub hw_sys: Option<f64>,
    pub hw_cpu: Option<f64>,
}

/// H/W as a ratio of means over aligned windows: xmrig's 60 s hashrate over the last 60 s of
/// watts; in xmrig's first minute, before it has one, its 10 s hashrate over the last 10 s. Pure,
/// so the windows are tested without a running engine.
pub fn rolling(ring: &PowerRing, hs10: Option<f64>, hs60: Option<f64>) -> Rolling {
    let (sys_w_60, cpu_w_60) = (ring.sys(HW_WINDOW), ring.cpu(HW_WINDOW));
    let (hs, sys, cpu) = match hs60 {
        Some(h) => (Some(h), sys_w_60, cpu_w_60),
        None => (hs10, ring.sys(SHORT_WINDOW), ring.cpu(SHORT_WINDOW)),
    };
    Rolling { sys_w_60, cpu_w_60, sys_w_900: ring.sys(COST_WINDOW), hw_sys: hw(hs, sys), hw_cpu: hw(hs, cpu) }
}

/// Review T9 #4: with M5, a missing `ps` reading counts as a busy Mac, so the optimizer would wait
/// with no word. One tape line per outage: `read` is this tick's reading (`None` = `ps` failed),
/// and a reading that comes back re-arms the line. Pure, so it's tested without a running engine.
pub fn ps_line(flagged: &mut bool, read: Option<f64>, ts: &str) -> Option<TapeLine> {
    outage_line(flagged, read.is_some(), PS_UNREAD, ts)
}

pub const PS_UNREAD: &str = "cannot read other apps' cpu · trials record without the noise flag";

fn outage_line(flagged: &mut bool, ok: bool, msg: &str, ts: &str) -> Option<TapeLine> {
    if ok {
        *flagged = false;
        None
    } else if !*flagged {
        *flagged = true;
        Some(TapeLine { ts: ts.to_string(), tag: "error".to_string(), msg: msg.to_string() })
    } else {
        None
    }
}

/// A trial the Ledger refused (closed, busy past its wait, or a write error): one tape line per new
/// loss, read off `TrialSink::lost`. An unanswered write may still land, so it is not a loss.
pub fn trial_lost_line(seen: &mut u64, lost: u64, ts: &str) -> Option<TapeLine> {
    if lost > *seen {
        *seen = lost;
        Some(TapeLine { ts: ts.to_string(), tag: "error".to_string(), msg: "history could not save a trial".to_string() })
    } else {
        None
    }
}

/// Who asked for a lever pull: the tape says so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum By {
    Optimizer,
    /// The user's own input (the `SetThreads` hand change, F4): tagged as theirs, not the optimizer's.
    Hand,
    /// A hand change xmrig refused (review M2): the optimizer lets go at the count that really
    /// runs. Not the user's pick, so the tape never says "you set" (T11 re-review N3).
    Fallback,
}

/// The tape and snapshot error for a hand change xmrig refused (T11 re-review N3).
fn hand_change_failed(n: u32, e: &dyn std::fmt::Display) -> String {
    format!("could not set {n} cores · {e}")
}

/// How long the engine, on its way out, waits for queued trials to reach the Ledger's writer.
pub const SINK_JOIN_WAIT: Duration = Duration::from_secs(2);

/// Finished trials go to the Ledger from one thread of their own, in order: the engine only
/// queues them (an unbounded channel, one trial every few minutes), so it never waits on the
/// Ledger. The thread ends once the engine drops its sender and the queue is written.
///
/// Review M1: dropping the sink (every exit from `run`) closes the queue and waits for the thread,
/// `SINK_JOIN_WAIT` at most, so a trial recorded just before a stop is handed to the Ledger's
/// writer before `EngineCtl::stop` returns, and the app's `LedgerWriter::close` then writes it.
struct TrialSink {
    tx: Option<Sender<(TrialRecord, String)>>,
    recorded: Arc<AtomicU64>,
    lost: Arc<AtomicU64>,
    /// Disconnects when the thread ends (its sender is dropped last): a join with a limit.
    ended: Receiver<()>,
}

impl Drop for TrialSink {
    fn drop(&mut self) {
        self.tx.take(); // closes the queue: the thread writes what is queued, then ends
        let _ = self.ended.recv_timeout(SINK_JOIN_WAIT);
    }
}

impl TrialSink {
    fn spawn(ledger: LedgerHandle) -> TrialSink {
        let (tx, rx) = mpsc::channel::<(TrialRecord, String)>();
        let (ended_tx, ended) = mpsc::channel::<()>();
        let recorded = Arc::new(AtomicU64::new(0));
        let lost = Arc::new(AtomicU64::new(0));
        let (n, l) = (recorded.clone(), lost.clone());
        thread::spawn(move || {
            let _ended = ended_tx; // dropped after the last trial is handed over
            for (rec, wallet) in rx {
                // The Ledger stores only a salted tag of the wallet (Plan 3). Never retried: an
                // unconfirmed write may still land, and a retry could store the trial twice.
                match ledger.trial(rec, Some(wallet)) {
                    Ok(_) => n.fetch_add(1, Ordering::Relaxed),
                    Err(e) if e.unconfirmed() => 0,
                    Err(_) => l.fetch_add(1, Ordering::Relaxed),
                };
            }
        });
        TrialSink { tx: Some(tx), recorded, lost, ended }
    }

    fn send(&self, rec: TrialRecord, wallet: String) {
        if let Some(tx) = &self.tx {
            let _ = tx.send((rec, wallet));
        }
    }
}

/// The levers the engine pulls for the optimizer: `Miner` in the app, a fake in tests (review M5).
pub(crate) trait Rigging {
    fn apply(&mut self, s: &Setup) -> anyhow::Result<()>;
    fn setup(&self) -> Setup;
}

impl Rigging for Miner {
    fn apply(&mut self, s: &Setup) -> anyhow::Result<()> {
        Miner::apply(self, s)
    }
    fn setup(&self) -> Setup {
        Miner::setup(self)
    }
}

/// What one pass over the optimizer's actions leaves for the engine to finish.
#[derive(Debug, Default)]
struct Carried {
    tape: Vec<TapeLine>,
    /// `apply_failed`'s actions: carried out on the next tick.
    next: Vec<(Action, By)>,
    /// `Record` and `Prefs`, for the engine (the Ledger sink, the snapshot).
    rest: Vec<Action>,
}

/// Carry out the levers. After every Apply, on both arms, the optimizer hears what really runs
/// (`rig.setup()`: clamp first with an undo, Task 3), and a failure hands control back to the
/// user through `apply_failed`, whose actions wait for the next tick.
fn carry_out<R: Rigging>(rig: &mut R, o: &mut Optimizer, acts: Vec<(Action, By)>, ts: &str) -> Carried {
    let mut c = Carried::default();
    for (a, by) in acts {
        let Action::Apply(s) = a else {
            c.rest.push(a);
            continue;
        };
        let result = rig.apply(&s);
        o.applied(rig.setup());
        match result {
            Ok(()) => {
                let msg = match by {
                    By::Optimizer => format!("optimizer · {}", s.label()),
                    By::Hand => format!("you set {}", s.label()),
                    By::Fallback => format!("manual at {}", s.label()),
                };
                c.tape.push(TapeLine { ts: ts.to_string(), tag: "miner".into(), msg });
            }
            Err(e) => {
                let msg = format!("could not apply {}: {e}", s.label());
                c.tape.push(TapeLine { ts: ts.to_string(), tag: "error".into(), msg: msg.clone() });
                c.next.extend(o.apply_failed(&msg).into_iter().map(|a| (a, By::Optimizer)));
            }
        }
    }
    c
}

/// Review M3: the tape line for a History read that failed (the optimizer starts with no trials).
pub const HISTORY_FAILED: &str = "could not read past trials · starting fresh";

/// Local HH:MM:SS, for tape line timestamps.
pub fn clock_hms() -> String {
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&now, &mut tm) };
    format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
}

/// A live handle to the engine thread. `stop()`/`Drop` both send `Command::Stop` and join;
/// `Drop`'s own send is a harmless no-op once `stop()` already emptied the channel and joined.
pub struct EngineCtl {
    tx: Sender<Command>,
    join: Option<JoinHandle<()>>,
}

impl EngineCtl {
    pub fn send(&self, c: Command) -> anyhow::Result<()> {
        self.tx.send(c).map_err(|e| anyhow::anyhow!("engine thread is gone: {e}"))
    }

    pub fn stop(mut self) {
        let _ = self.tx.send(Command::Stop);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

impl Drop for EngineCtl {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Stop);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

pub fn spawn(cfg: EngineConfig) -> (EngineCtl, Receiver<Snapshot>) {
    let (cmd_tx, cmd_rx) = mpsc::channel::<Command>();
    let (snap_tx, snap_rx) = mpsc::channel::<Snapshot>();
    let join = thread::spawn(move || run(cfg, cmd_rx, snap_tx));
    (EngineCtl { tx: cmd_tx, join: Some(join) }, snap_rx)
}

fn error_snapshot(threads: u32, worker: &str, wallet: &str, last: &Summary, msg: String) -> Snapshot {
    Snapshot {
        t: now_secs(),
        state: RunState::Error,
        error: Some(msg),
        threads,
        clamp: None,
        worker: worker.to_string(),
        wallet_masked: mask_wallet(wallet),
        hs10: last.hs10(),
        hs60: last.hs60(),
        hs15: last.hs15(),
        hs_max: last.hashrate.highest,
        shares_good: last.results.shares_good,
        shares_rejected: last.results.shares_total.saturating_sub(last.results.shares_good),
        diff: last.results.diff_current,
        algo: last.algo.clone(),
        uptime_s: last.uptime,
        power: None,
        sys_w_60: None,
        cpu_w_60: None,
        sys_w_900: None,
        hw_sys: None,
        hw_cpu: None,
        context: None,
        low_power: lowpower::latest(),
        tape: Vec::new(),
        optimizer: None,
        prefs: None,
        p2pool: None,
        tor: false,
        guard_note: None,
    }
}

/// A start-up step on the tape ("connecting to tor", "starting p2pool") while the engine brings
/// its helpers up, before xmrig runs.
fn step_snapshot(cfg: &EngineConfig, msg: &str) -> Snapshot {
    let mut s = error_snapshot(cfg.threads, &cfg.worker, &cfg.wallet, &Summary::default(), String::new());
    s.state = RunState::Starting;
    s.error = None;
    s.tape = vec![TapeLine { ts: clock_hms(), tag: "net".into(), msg: msg.into() }];
    s
}

/// Why p2pool can't start yet, in the node guide's words.
fn node_problem(n: &crate::node::Node, choice: crate::settings::NodeChoice) -> Option<String> {
    use crate::node::Node;
    let own = choice == crate::settings::NodeChoice::Own;
    match n {
        Node::Ready { .. } => None,
        Node::Missing if own => Some("xmrange's monero node is not running: choose its chain folder in settings".into()),
        Node::Missing => Some("no monero node on this mac: open the monero gui and start its local node".into()),
        Node::NoZmq if own => Some("xmrange's monero node is still starting: try again in a minute".into()),
        Node::NoZmq => Some(format!("your monero node needs its zmq feed: add {} to the gui's daemon startup flags", crate::node::ZMQ_FLAG)),
        Node::Syncing { height, target } if own => Some(format!("xmrange's monero node is syncing ({height} of {target}); p2pool starts once it's synced")),
        Node::Syncing { height, target } => Some(format!("your monero node is syncing ({height} of {target}); p2pool starts once it's synced")),
    }
}

/// The Keel's drift check (design §3): while a throttle is held, the rolling 15-minute H/W against
/// the held trial's score. Below by more than `DRIFT_FRAC` for `DRIFT_TICKS` in a row, after the
/// hold's first `DRIFT_WARMUP` ticks, the tape says so once; back within it, the tape says that.
/// It only reports: re-probing on drift is the owner's call (heat or another app can depress a
/// re-measure).
#[derive(Debug, Default)]
struct Drift {
    held: Option<crate::optimizer::Setup>,
    held_ticks: u32,
    below: u32,
    flagged: bool,
}

const DRIFT_FRAC: f64 = 0.05;
const DRIFT_TICKS: u32 = 900;
const DRIFT_WARMUP: u32 = 1200;

impl Drift {
    /// `held`: the held pick (None when not holding). `hw15`: H/W over the last 15 minutes, on
    /// the pick's denominator.
    fn tick(&mut self, held: Option<&crate::optimizer::Pick>, hw15: Option<f64>) -> Option<String> {
        let Some(p) = held else {
            *self = Drift::default();
            return None;
        };
        if self.held != Some(p.setup) {
            *self = Drift { held: Some(p.setup), ..Drift::default() };
        }
        self.held_ticks = self.held_ticks.saturating_add(1);
        let (Some(trial), Some(now)) = (p.hw, hw15) else { return None };
        if self.held_ticks <= DRIFT_WARMUP || trial <= 0.0 {
            return None;
        }
        let under = (trial - now) / trial;
        if under > DRIFT_FRAC {
            self.below += 1;
            if self.below >= DRIFT_TICKS && !self.flagged {
                self.flagged = true;
                return Some(format!(
                    "drift · {} at {now:.1} h/w, {:.0}% under its trial's {trial:.1} · other apps or heat",
                    p.setup.label(),
                    under * 100.0
                ));
            }
        } else {
            self.below = 0;
            if self.flagged {
                self.flagged = false;
                return Some(format!("back on its trial's h/w · {now:.1} of {trial:.1}"));
            }
        }
        None
    }
}

/// One hour of mining, summed tick by tick: the "hour" tape line that shows whether a long hold
/// stayed steady (memory only: live power figures never reach the Ledger, ruling P5-O1).
#[derive(Debug, Default)]
struct HourLog {
    ticks: u32,
    hs: (f64, u32),
    sys_w: (f64, u32),
    cpu_w: (f64, u32),
}

/// Ticks in an hour (one per second).
const HOUR_TICKS: u32 = 3600;

impl HourLog {
    /// One mining tick. Returns the hour's line once `HOUR_TICKS` have passed.
    fn tick(&mut self, hs10: Option<f64>, power: Option<&crate::telemetry::PowerSample>) -> Option<String> {
        let add = |acc: &mut (f64, u32), v: Option<f64>| {
            if let Some(v) = v.filter(|v| v.is_finite()) {
                acc.0 += v;
                acc.1 += 1;
            }
        };
        self.ticks += 1;
        add(&mut self.hs, hs10);
        add(&mut self.sys_w, power.and_then(|p| p.sys_w));
        add(&mut self.cpu_w, power.map(|p| p.cpu_w));
        if self.ticks < HOUR_TICKS {
            return None;
        }
        let mean = |acc: (f64, u32)| (acc.1 > 0).then(|| acc.0 / f64::from(acc.1));
        let (hs, sys, cpu) = (mean(self.hs), mean(self.sys_w), mean(self.cpu_w));
        *self = HourLog::default();
        let w = |label: &str, w: Option<f64>| match (hs, w) {
            (Some(h), Some(w)) if w > 0.0 => format!("{label} {w:.1} w ({:.1} h/w)", h / w),
            (_, Some(w)) => format!("{label} {w:.1} w"),
            _ => format!("{label} n/a"),
        };
        Some(format!("past hour · {} h/s · {} · {}", hs.map_or("n/a".to_string(), |h| format!("{h:.0}")), w("whole mac", sys), w("cpu", cpu)))
    }
}

/// The tape's line for a payout p2pool reported: the amount to the piconero, and its block.
fn payout_line(p: crate::p2pool::Payout) -> String {
    format!("payout {}.{:012} XMR · block {}", p.atomic / 1_000_000_000_000, p.atomic % 1_000_000_000_000, p.height)
}

/// p2pool events for the tape, from p2pool itself: peers found or lost, a share found, the share
/// chain synced (its own figures and words), and its warnings and errors (a repeat of the line
/// just shown is skipped).
#[derive(Default)]
struct P2Tape {
    shares: Option<u64>,
    peers_up: bool,
    synced: bool,
    last_error: Option<String>,
}

impl P2Tape {
    fn update(&mut self, s: &crate::p2pool::Stats, synced: bool, errors: Vec<String>, ts: &str) -> Vec<TapeLine> {
        let mut out = Vec::new();
        for e in errors {
            if self.last_error.as_deref() != Some(e.as_str()) {
                out.push(TapeLine { ts: ts.to_string(), tag: "error".into(), msg: e.clone() });
                self.last_error = Some(e);
            }
        }
        let line = |msg: String| TapeLine { ts: ts.to_string(), tag: "net".into(), msg };
        if synced && !self.synced {
            out.push(line("p2pool synced its share chain".into()));
            self.synced = true;
        }
        let up = s.peers.unwrap_or(0) > 0;
        if up != self.peers_up {
            out.push(line(if up { format!("p2pool connected · {} peers", s.peers.unwrap_or(0)) } else { "p2pool lost its peers".into() }));
            self.peers_up = up;
        }
        if let Some(n) = s.shares_found {
            if self.shares.is_some_and(|was| n > was) {
                out.push(line("share found on the p2pool chain".into()));
            }
            self.shares = Some(n);
        }
        out
    }
}

fn run(cfg: EngineConfig, cmd_rx: Receiver<Command>, snap_tx: Sender<Snapshot>) {
    // Not Send: created here, on the sampling thread, never handed elsewhere.
    let mut power_source = crate::telemetry::open_source().ok().map(|(s, _)| s);

    let miner_config = match MinerConfig::new(&cfg.wallet, &cfg.worker, cfg.threads) {
        Ok(c) => MinerConfig { utility: cfg.utility, ..c },
        Err(e) => {
            let _ = snap_tx.send(error_snapshot(cfg.threads, &cfg.worker, &cfg.wallet, &Summary::default(), e.to_string()));
            return;
        }
    };
    // Tor first, then p2pool, then xmrig. They stop in reverse: `miner` is declared last, so it
    // drops (stops) first on every return path, then p2pool, then Tor.
    // Start-up can take a while (Tor's first bootstrap, p2pool reaching the node): a Stop or a quit
    // meanwhile cancels it at once. Other commands wait in `early` for the main loop.
    let early = std::cell::RefCell::new(std::collections::VecDeque::new());
    let cancel = || loop {
        match cmd_rx.try_recv() {
            Ok(Command::Stop) | Err(TryRecvError::Disconnected) => return true,
            Ok(c) => early.borrow_mut().push_back(c),
            Err(TryRecvError::Empty) => return false,
        }
    };
    let fail = |msg: String| {
        if msg.ends_with(crate::sidecar::CANCELLED) {
            return; // the user stopped: the engine ends quietly
        }
        let _ = snap_tx.send(error_snapshot(cfg.threads, &cfg.worker, &cfg.wallet, &Summary::default(), msg));
    };
    let p2pool_mode = cfg.pool == crate::settings::Pool::P2pool;
    if p2pool_mode {
        if let Err(e) = crate::p2pool::check_wallet(&cfg.wallet) {
            return fail(e.to_string());
        }
        if let Some(why) = node_problem(&crate::node::probe(cfg.node), cfg.node) {
            return fail(why);
        }
    }
    let mut tor = None;
    if cfg.tor {
        let _ = snap_tx.send(step_snapshot(&cfg, "connecting to tor · a first start can take a few minutes"));
        match crate::tor::acquire(&cancel) {
            Ok(t) => {
                let _ = snap_tx.send(step_snapshot(&cfg, "connected through tor"));
                tor = Some(t);
            }
            // Never a quiet fall back to a direct connection: the user chose Tor (or kept the default).
            Err(e) => return fail(format!("tor: {e} · to mine directly, turn tor off in settings")),
        }
    }
    let socks = tor.as_ref().map(|_| crate::tor::socks_addr());
    let mut p2 = None;
    let miner_config = if p2pool_mode {
        let _ = snap_tx.send(step_snapshot(&cfg, &format!("starting p2pool ({} chain)", cfg.chain.name())));
        match crate::p2pool::P2pool::start(&cfg.wallet, socks.as_deref(), cfg.chain, crate::node::ports(cfg.node), &cancel) {
            Ok(p) => p2 = Some(p),
            Err(e) => return fail(format!("p2pool: {e}")),
        }
        miner_config.to_p2pool()
    } else {
        MinerConfig { socks5: socks, ..miner_config }
    };
    let mut p2_tape = P2Tape::default();
    let mut hour_log = HourLog::default();
    let mut drift = Drift::default();
    let opts = LaunchOptions { config: miner_config, api_port: API_PORT, e_core_clamp: cfg.e_core_clamp, qos_utility: false, dev_log: cfg.dev_log.clone() };
    let mut miner = match Miner::launch(&paths::xmrig_binary(), opts) {
        Ok(m) => m,
        Err(e) => {
            let _ = snap_tx.send(error_snapshot(cfg.threads, &cfg.worker, &cfg.wallet, &Summary::default(), e.to_string()));
            return;
        }
    };

    let mut tape = TapeState::default();
    let mut trial_tape = TrialTapeState::default();
    let mut last = Summary::default();
    let mut state = RunState::Starting;
    let mut threads = cfg.threads;
    let mut worker = cfg.worker.clone();
    let mut wallet = cfg.wallet.clone();
    let mut engine_tick: u64 = 0;
    #[cfg(feature = "dev-diagnostics")]
    let mut last_opt_line = String::new();
    let mut health = Health::default();
    let mut ring = PowerRing::default();

    let mut optimizer = cfg.optimizer.as_ref().map(|o| {
        // Minor M2: what xmrig really runs. A launch-time clamp that failed is off here, so the
        // optimizer's first Apply of a clamped setup isn't a no-op.
        let start = miner.setup();
        let mut opt = Optimizer::new(o.prefs.clone(), Limits { threads: max_threads(), e_cores: o.e_cores }, o.chip.clone(), o.measure, start, now_secs());
        if let Some(n) = o.own_threads {
            opt.set_own(Setup::threads(n));
        }
        opt
    });
    let chip = cfg.optimizer.as_ref().and_then(|o| o.chip.clone());
    // The Ledger's past trials, read off this thread: the engine never waits on the Ledger.
    let (hist_tx, hist_rx) = mpsc::channel::<Result<Vec<TrialRecord>, String>>();
    if cfg.optimizer.is_some() {
        let handle = cfg.ledger.clone();
        thread::spawn(move || {
            let recs = match handle {
                Some(h) => h.trials(HISTORY_TRIALS).map(|(_, r)| r),
                None => Ok(Vec::new()),
            };
            let _ = hist_tx.send(recs);
        });
    }
    let sink = cfg.ledger.clone().filter(|_| optimizer.is_some()).map(TrialSink::spawn);
    let mut lost_seen: u64 = 0;
    let mut ps_flagged = false;
    // The trial noise flag: other apps' CPU against its own rolling baseline, read only while
    // a sweep runs.
    let mut cpu_base = crate::busy::CpuBaseline::default();
    let mut on_ac: Option<bool> = None;
    let mut guard = crate::guard::Guard::new(cfg.guard);
    // The user's own pause: a limit never resumes it.
    let mut user_paused = false;
    let mut pending: Vec<(Action, By)> = Vec::new();

    loop {
        let mut cmd_error: Option<String> = None;
        let mut tape_lines: Vec<TapeLine> = Vec::new();
        loop {
            let next = early.borrow_mut().pop_front();
            match next.map(Ok).unwrap_or_else(|| cmd_rx.try_recv()) {
                Ok(Command::Stop) => {
                    let _ = miner.stop();
                    return;
                }
                Ok(Command::SetThreads(n)) => match miner.set_threads(n) {
                    Ok(()) => {
                        threads = n;
                        // Hand input: the optimizer drops to manual and never pulls a lever again
                        // (spec §2). Its one Apply (the clamp off, F4) is the user's own.
                        if let Some(o) = optimizer.as_mut() {
                            pending.extend(o.command(OptCommand::ManualThreads(n)).into_iter().map(|a| (a, By::Hand)));
                        }
                    }
                    Err(e) => {
                        // Review M2: the hand change still means manual (the app saves it so), at
                        // the count that really runs; a held optimizer must not go on steering.
                        if let Some(o) = optimizer.as_mut().filter(|o| o.active()) {
                            pending.extend(o.command(OptCommand::ManualThreads(miner.threads())).into_iter().map(|a| (a, By::Fallback)));
                        }
                        let msg = hand_change_failed(n, &e);
                        tape_lines.push(TapeLine { ts: clock_hms(), tag: "error".to_string(), msg: msg.clone() });
                        cmd_error = Some(msg);
                    }
                },
                Ok(Command::SetIdentity { wallet: w, worker: wk }) => match miner.set_identity(&w, &wk) {
                    Ok(()) => {
                        wallet = w;
                        worker = wk;
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        tape_lines.push(TapeLine { ts: clock_hms(), tag: "error".to_string(), msg: msg.clone() });
                        cmd_error = Some(msg);
                    }
                },
                Ok(Command::SetGuard(g)) => {
                    if let Some(crate::guard::GuardAct::Resume(msg)) = guard.set_prefs(g) {
                        if miner.resume().is_ok() {
                            tape_lines.push(TapeLine { ts: clock_hms(), tag: "miner".into(), msg });
                        }
                    }
                }
                Ok(Command::Pause) => {
                    user_paused = true;
                    if let Err(e) = miner.pause() {
                        let msg = e.to_string();
                        tape_lines.push(TapeLine { ts: clock_hms(), tag: "error".to_string(), msg: msg.clone() });
                        cmd_error = Some(msg);
                    }
                }
                Ok(Command::Resume) => {
                    user_paused = false;
                    guard.user_resumed();
                    if let Err(e) = miner.resume() {
                        let msg = e.to_string();
                        tape_lines.push(TapeLine { ts: clock_hms(), tag: "error".to_string(), msg: msg.clone() });
                        cmd_error = Some(msg);
                    }
                }
                Ok(Command::Optimizer(c)) => {
                    if let Some(o) = optimizer.as_mut() {
                        pending.extend(o.command(c).into_iter().map(|a| (a, By::Optimizer)));
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    let _ = miner.stop();
                    return;
                }
            }
        }

        if let Err(e) = miner.ensure_running() {
            let _ = snap_tx.send(error_snapshot(threads, &worker, &wallet, &last, e.to_string()));
            break;
        }
        if let Some(why) = p2.as_mut().and_then(|p| p.exited()).or_else(|| tor.as_mut().and_then(|t| t.exited())) {
            let _ = snap_tx.send(error_snapshot(threads, &worker, &wallet, &last, why));
            break;
        }

        let power = match power_source.as_mut() {
            Some(src) => src.sample(1000).ok(),
            None => {
                thread::sleep(Duration::from_millis(1000));
                None
            }
        };
        // Read beside the sample's wall time (`power.t`): the pair shows a sleep (P4-C23).
        let mono = crate::telemetry::mono_secs();

        // The user's limits, once a second, never during trials (`guard`).
        let temp_f = power.as_ref().and_then(|p| p.temp_c).map(|c| c * 9.0 / 5.0 + 32.0);
        let sweeping = optimizer.as_ref().is_some_and(|o| o.sweeping());
        match guard.tick(temp_f, on_ac, user_paused, sweeping) {
            Some(crate::guard::GuardAct::Pause(msg)) => {
                if miner.pause().is_ok() {
                    tape_lines.push(TapeLine { ts: clock_hms(), tag: "miner".into(), msg });
                }
            }
            Some(crate::guard::GuardAct::Resume(msg)) => {
                if miner.resume().is_ok() {
                    tape_lines.push(TapeLine { ts: clock_hms(), tag: "miner".into(), msg });
                }
            }
            None => {}
        }
        let summary = miner.summary().ok();
        let ts = clock_hms();
        if let Some(s) = &summary {
            tape_lines.extend(tape.update(s, &ts));
            last = s.clone();
        }
        let line = |(tag, msg): (&str, &str)| TapeLine { ts: ts.clone(), tag: tag.to_string(), msg: msg.to_string() };
        tape_lines.extend(health.api(summary.is_some()).map(line));
        if let Some(w) = health.pool_wait(state == RunState::Starting, last.algo.is_some(), engine_tick) {
            // p2pool mode: p2pool, not the network, hands out the job; it refuses xmrig while it has no peers.
            tape_lines.push(match p2.as_ref() {
                Some(p) => line(("net", &format!("waiting for p2pool's first job · {} peers", p.stats().peers.map_or("no".to_string(), |n| n.to_string())))),
                None => line(w),
            });
        }

        // Starting graduates to Mining/Paused the first time hs10 is known and xmrig has a job
        // (`algo`): with no job xmrig reports 0.0, not null, and never starts hashing. After that, hs10
        // can legitimately go None again (a thread change restarts workers; a paused miner's
        // moving average empties out) without un-graduating: Mining vs. Paused is then governed
        // by `paused` alone.
        if state == RunState::Starting {
            if last.hs10().is_some() && last.algo.is_some() {
                state = if last.paused { RunState::Paused } else { RunState::Mining };
            }
        } else {
            state = if last.paused { RunState::Paused } else { RunState::Mining };
        }

        let want_context = engine_tick == 0 || engine_tick % 30 == 0;
        let context = if want_context { Some(context::read_context()) } else { None };

        // The last rates stay only while the API answers: a silent API must not read as mining.
        let stale = health.rates_stale();
        let hs10 = if stale { None } else { last.hs10() };
        if state == RunState::Mining {
            if let Some(msg) = hour_log.tick(hs10, power.as_ref()) {
                tape_lines.push(TapeLine { ts: clock_hms(), tag: "hour".into(), msg });
            }
        }
        let hs60 = if stale { None } else { last.hs60() };
        let hs15 = if stale { None } else { last.hs15() };
        // Display H/W, cost and earned: ratios of means over aligned windows (ruling P5-O2), as
        // `xmrange trial` divides mean H by mean W. The optimizer's own ticks below are untouched.
        ring.push(power.as_ref());
        let roll = rolling(&ring, hs10, hs60);

        if let Some(c) = &context {
            on_ac = c.on_ac.or(on_ac);
            if let Some(o) = optimizer.as_mut() {
                o.context_read(c.clone());
            }
        }
        let mut prefs_out: Option<OptimizerPrefs> = None;
        if let Some(o) = optimizer.as_mut() {
            if let Ok(read) = hist_rx.try_recv() {
                let recs = read.unwrap_or_else(|_| {
                    tape_lines.push(TapeLine { ts: ts.clone(), tag: "error".into(), msg: HISTORY_FAILED.into() });
                    Vec::new()
                });
                pending.extend(o.history_loaded(recs, lowpower::latest()).into_iter().map(|a| (a, By::Optimizer)));
            }
            match (state, power) {
                (RunState::Mining, Some(p)) => {
                    // `ps` only while a sweep runs: it's the trial noise flag, and nothing else
                    // needs it.
                    let (other, excess) = if o.sweeping() {
                        let read = context::other_cpu_pct(&[std::process::id(), miner.pid()]);
                        tape_lines.extend(ps_line(&mut ps_flagged, read, &ts));
                        // P4-C23: a sleep re-seeds the baseline (wall vs monotonic time).
                        (read, cpu_base.push_awake(now_secs(), mono, read))
                    } else {
                        // Review M4: a later sweep names a still-failing reading again.
                        ps_flagged = false;
                        (None, None)
                    };
                    let tick = Tick { power: p, hs10, other_cpu_pct: other, other_cpu_excess: excess };
                    let acts = o.on_tick(&OptTick { tick, low_power: lowpower::latest(), on_ac, mono });
                    pending.extend(acts.into_iter().map(|a| (a, By::Optimizer)));
                }
                (RunState::Mining, None) => {}
                _ => pending.extend(o.paused().into_iter().map(|a| (a, By::Optimizer))),
            }
        }
        let acts = std::mem::take(&mut pending);
        if let (Some(o), false) = (optimizer.as_mut(), acts.is_empty()) {
            let c = carry_out(&mut miner, o, acts, &ts);
            threads = miner.threads();
            tape_lines.extend(c.tape);
            pending = c.next;
            for a in c.rest {
                match a {
                    Action::Record { setup, label, result } => {
                        let lp = result.context_start.low_power_mode;
                        // A noisy trial is kept and flagged, never skipped (owner, 2026-09-28).
                        // Under the fixed standard only a trial stopped early is skipped (as `usable`).
                        let msg = if !result.partial {
                            match result.score.as_ref() {
                                Some(s) => {
                                    let hw = s.hw_sys.or(s.hw_cpu);
                                    format!("measured {} · {} h/w", setup_note_label(setup, lp), fmt_opt(hw))
                                }
                                None => format!("skipped {} · no reading", setup_note_label(setup, lp)),
                            }
                        } else {
                            format!("skipped {} · stopped early", setup_note_label(setup, lp))
                        };
                        tape_lines.push(TapeLine { ts: ts.clone(), tag: "trial".into(), msg });
                        if let Some(sink) = &sink {
                            sink.send(TrialRecord::from_result(&result, &label, "sweep", chip.as_deref(), setup.e_core_clamp).with_utility(setup.utility), wallet.clone());
                        }
                    }
                    Action::Prefs(p) => prefs_out = Some(p),
                    Action::Apply(_) => {} // carried out above
                }
            }
        }
        if let Some(sink) = &sink {
            tape_lines.extend(trial_lost_line(&mut lost_seen, sink.lost.load(Ordering::Relaxed), &ts));
        }

        let opt_view = optimizer.as_ref().map(|o| o.view());
        if let Some(v) = &opt_view {
            let w = match v.measure {
                crate::settings::Measure::Sys => ring.sys(COST_WINDOW),
                crate::settings::Measure::Cpu => ring.cpu(COST_WINDOW),
            };
            let hw15 = match (hs15, w) {
                (Some(h), Some(w)) if w > 0.0 => Some(h / w),
                _ => None,
            };
            if let Some(msg) = drift.tick(v.held.as_ref().filter(|_| state == RunState::Mining), hw15) {
                tape_lines.push(TapeLine { ts: ts.clone(), tag: "miner".into(), msg });
            }
        }
        if let Some(v) = &opt_view {
            tape_lines.extend(trial_tape.update(v, &ts));
        }
        let p2_stats = p2.as_ref().map(|p| p.stats());
        if let (Some(s), Some(p)) = (&p2_stats, p2.as_ref()) {
            tape_lines.extend(p2_tape.update(s, p.synced(), p.take_errors(), &ts));
            // Payouts go to the Ledger off this thread (the engine never waits on the Ledger).
            for payout in p.take_payouts() {
                tape_lines.push(TapeLine { ts: ts.clone(), tag: "net".into(), msg: payout_line(payout) });
                if let Some(h) = cfg.ledger.clone() {
                    let w = wallet.clone();
                    thread::spawn(move || h.payout(now_secs(), payout, w));
                }
            }
        }

        let snapshot = Snapshot {
            t: now_secs(),
            state,
            error: cmd_error,
            threads,
            clamp: Some(miner.setup().e_core_clamp),
            worker: worker.clone(),
            wallet_masked: mask_wallet(&wallet),
            hs10,
            hs60,
            hs15,
            hs_max: last.hashrate.highest,
            shares_good: last.results.shares_good,
            shares_rejected: last.results.shares_total.saturating_sub(last.results.shares_good),
            diff: last.results.diff_current,
            algo: last.algo.clone(),
            uptime_s: last.uptime,
            power,
            sys_w_60: roll.sys_w_60,
            cpu_w_60: roll.cpu_w_60,
            sys_w_900: roll.sys_w_900,
            hw_sys: roll.hw_sys,
            hw_cpu: roll.hw_cpu,
            context,
            low_power: lowpower::latest(),
            tape: tape_lines,
            optimizer: opt_view.map(|v| OptimizerView {
                trials_recorded: sink.as_ref().map_or(0, |k| k.recorded.load(Ordering::Relaxed)),
                ..v
            }),
            prefs: prefs_out,
            p2pool: p2_stats,
            tor: tor.is_some(),
            guard_note: guard.note().map(String::from),
        };

        #[cfg(feature = "dev-diagnostics")]
        app_log(&snapshot, &mut last_opt_line);

        engine_tick += 1;
        if snap_tx.send(snapshot).is_err() {
            break;
        }
    }

    let _ = miner.stop();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_silent_api_withdraws_the_rates_once_and_recovers() {
        let mut h = Health::default();
        for _ in 1..API_STALE_TICKS {
            assert_eq!(h.api(false), None);
            assert!(!h.rates_stale(), "a short hiccup keeps the rates");
        }
        assert_eq!(h.api(false), Some(("error", "xmrig's api stopped answering; rates withdrawn")));
        assert!(h.rates_stale());
        assert_eq!(h.api(false), None, "one line, not one per tick");
        assert_eq!(h.api(true), Some(("miner", "xmrig's api is answering again")));
        assert!(!h.rates_stale());
        assert_eq!(h.api(true), None);
    }

    #[test]
    fn drift_speaks_once_after_15_minutes_under_and_once_on_recovery() {
        let pick = crate::optimizer::Pick { setup: crate::optimizer::Setup::threads(8), low_power: None, hw: Some(90.8), hs: 2326.0 };
        let mut d = Drift::default();
        for _ in 0..DRIFT_WARMUP {
            assert_eq!(d.tick(Some(&pick), Some(70.0)), None, "the hold's first 20 minutes are its warm-up");
        }
        for _ in 1..DRIFT_TICKS {
            assert_eq!(d.tick(Some(&pick), Some(80.0)), None);
        }
        assert_eq!(d.tick(Some(&pick), Some(80.0)).as_deref(), Some("drift · 8 cores at 80.0 h/w, 12% under its trial's 90.8 · other apps or heat"));
        assert_eq!(d.tick(Some(&pick), Some(80.0)), None, "said once");
        assert_eq!(d.tick(Some(&pick), Some(89.0)).as_deref(), Some("back on its trial's h/w · 89.0 of 90.8"));
        assert_eq!(d.tick(None, Some(50.0)), None, "not holding: nothing to compare");
    }

    #[test]
    fn an_hour_of_mining_sums_to_one_line() {
        let mut h = HourLog::default();
        let p = crate::telemetry::PowerSample { sys_w: Some(24.0), cpu_w: 6.0, ..Default::default() };
        for _ in 1..HOUR_TICKS {
            assert_eq!(h.tick(Some(1200.0), Some(&p)), None);
        }
        assert_eq!(h.tick(Some(1200.0), Some(&p)).as_deref(), Some("past hour · 1200 h/s · whole mac 24.0 w (50.0 h/w) · cpu 6.0 w (200.0 h/w)"));
        for _ in 1..HOUR_TICKS {
            h.tick(Some(1200.0), None);
        }
        assert_eq!(h.tick(None, None).as_deref(), Some("past hour · 1200 h/s · whole mac n/a · cpu n/a"), "an iMac without whole-mac watts says so");
    }

    #[test]
    fn a_payout_reads_to_the_piconero() {
        assert_eq!(payout_line(crate::p2pool::Payout { atomic: 412_345_678, height: 3_775_700 }), "payout 0.000412345678 XMR · block 3775700");
    }

    #[test]
    fn p2pool_errors_reach_the_tape_once_and_sync_is_said_once() {
        let mut t = P2Tape::default();
        let s = crate::p2pool::Stats::default();
        let msgs = |v: Vec<TapeLine>| v.into_iter().map(|l| format!("{} {}", l.tag, l.msg)).collect::<Vec<_>>();
        let e = || vec!["p2pool: get_miner_data RPC failed".to_string()];
        assert_eq!(msgs(t.update(&s, false, e(), "t")), vec!["error p2pool: get_miner_data RPC failed"]);
        assert!(t.update(&s, false, e(), "t").is_empty(), "the same line again is skipped");
        assert_eq!(msgs(t.update(&s, true, vec![], "t")), vec!["net p2pool synced its share chain"]);
        assert!(t.update(&s, true, vec![], "t").is_empty(), "synced is said once");
    }

    #[test]
    fn pool_wait_speaks_once_after_30_ticks_while_starting() {
        let mut h = Health::default();
        assert_eq!(h.pool_wait(true, false, POOL_WAIT_TICKS - 1), None);
        assert_eq!(h.pool_wait(false, false, POOL_WAIT_TICKS), None, "already mining");
        assert_eq!(h.pool_wait(true, true, POOL_WAIT_TICKS), None, "a job arrived");
        assert_eq!(h.pool_wait(true, false, POOL_WAIT_TICKS), Some(("net", "waiting for the pool (check the network)")));
        assert_eq!(h.pool_wait(true, false, POOL_WAIT_TICKS + 5), None, "once per session");
    }

    fn watts(t: f64, sys: Option<f64>, cpu: f64) -> PowerSample {
        PowerSample { t, sys_w: sys, cpu_w: cpu, ..Default::default() }
    }

    /// A ring fed `(sys, cpu)` once a second from t = 1000.
    fn ring_of(vals: &[(Option<f64>, f64)]) -> PowerRing {
        let mut r = PowerRing::default();
        for (i, (s, c)) in vals.iter().enumerate() {
            r.push(Some(&watts(1000.0 + i as f64, *s, *c)));
        }
        r
    }

    fn close(a: Option<f64>, b: f64) -> bool {
        a.is_some_and(|a| (a - b).abs() < 1e-9)
    }

    #[test]
    fn a_window_waits_for_10_s_then_uses_what_there_is() {
        let r = ring_of(&[(Some(10.0), 4.0); 9]);
        assert_eq!((r.sys(HW_WINDOW), r.cpu(HW_WINDOW), r.sys(COST_WINDOW)), (None, None, None), "under 10 s: no figure");
        let mut r = ring_of(&[(Some(10.0), 4.0); 9]);
        r.push(Some(&watts(1009.0, Some(20.0), 6.0)));
        assert!(close(r.sys(HW_WINDOW), 11.0) && close(r.cpu(HW_WINDOW), 4.2), "10 s: the 10 s there are");
        assert!(close(r.sys(COST_WINDOW), 11.0), "the 15-minute window too");
    }

    #[test]
    fn each_window_covers_its_own_last_n_ticks() {
        // 30 s at 20 W, then 60 s at 10 W.
        let mut vals = vec![(Some(20.0), 8.0); 30];
        vals.extend([(Some(10.0), 4.0); 60]);
        let r = ring_of(&vals);
        assert!(close(r.sys(HW_WINDOW), 10.0) && close(r.cpu(HW_WINDOW), 4.0), "the last 60 s only");
        assert!(close(r.sys(COST_WINDOW), (30.0 * 20.0 + 60.0 * 10.0) / 90.0), "all 90 s, short of 15 minutes");
    }

    #[test]
    fn unknown_readings_are_skipped_never_read_as_zero() {
        let mut vals = vec![(Some(10.0), 4.0); 12];
        vals.extend([(None, f64::NAN), (Some(f64::INFINITY), 4.0)]);
        let mut r = ring_of(&vals);
        assert!(close(r.sys(HW_WINDOW), 10.0) && close(r.cpu(HW_WINDOW), 4.0));
        r.push(None); // a tick with no power reading at all
        assert!(close(r.sys(HW_WINDOW), 10.0));
        let only_nine_known = ring_of(&[vec![(Some(10.0), 4.0); 9], vec![(None, 4.0); 1]].concat());
        assert_eq!(only_nine_known.sys(HW_WINDOW), None, "10 ticks but 9 readings: not yet");
    }

    #[test]
    fn the_ring_is_bounded_at_15_minutes() {
        let mut vals = vec![(Some(100.0), 50.0); 500];
        vals.extend(vec![(Some(10.0), 4.0); COST_WINDOW]);
        let r = ring_of(&vals);
        assert_eq!(r.slots.len(), COST_WINDOW);
        assert!(close(r.sys(COST_WINDOW), 10.0), "the oldest 500 s have left the ring");
    }

    #[test]
    fn a_sleep_starts_the_windows_over() {
        let mut r = ring_of(&[(Some(10.0), 4.0); 60]);
        r.push(Some(&watts(1000.0 + 60.0 + 3600.0, Some(30.0), 9.0)));
        assert_eq!(r.sys(HW_WINDOW), None, "one reading since waking");
        for i in 1..10 {
            r.push(Some(&watts(4660.0 + i as f64, Some(30.0), 9.0)));
        }
        assert!(close(r.sys(HW_WINDOW), 30.0), "no watts from before the sleep");
    }

    #[test]
    fn hw_is_a_ratio_of_means_over_aligned_windows() {
        let mut vals = vec![(Some(20.0), 8.0); 30];
        vals.extend([(Some(10.0), 4.0); 60]);
        let r = ring_of(&vals);
        let x = rolling(&r, Some(1000.0), Some(1200.0));
        assert!(close(x.hw_sys, 120.0) && close(x.hw_cpu, 300.0), "hs60 over the last 60 s of watts: {x:?}");
        assert!(close(x.sys_w_60, 10.0) && close(x.cpu_w_60, 4.0));
        assert!(close(x.sys_w_900, (30.0 * 20.0 + 60.0 * 10.0) / 90.0));
        let first_minute = rolling(&ring_of(&[vec![(Some(20.0), 8.0); 5], vec![(Some(10.0), 5.0); 10]].concat()), Some(1000.0), None);
        assert!(close(first_minute.hw_sys, 100.0) && close(first_minute.hw_cpu, 200.0), "no hs60 yet: hs10 over the last 10 s: {first_minute:?}");
        assert_eq!(rolling(&ring_of(&[(Some(10.0), 4.0); 9]), Some(1000.0), Some(1200.0)).hw_sys, None, "under 10 s of watts: no h/w");
        assert_eq!(rolling(&r, None, None).hw_sys, None, "no hashrate (a silent api): no h/w");
    }

    /// Review T9 #4: one line per `ps` outage, never one per tick; a reading re-arms it.
    #[test]
    fn ps_line_speaks_once_per_outage() {
        let mut flagged = false;
        assert_eq!(ps_line(&mut flagged, Some(3.0), "12:00:00"), None);
        let line = ps_line(&mut flagged, None, "12:00:01").expect("the outage");
        assert_eq!((line.tag.as_str(), line.msg.as_str()), ("error", PS_UNREAD));
        assert_eq!(ps_line(&mut flagged, None, "12:00:02"), None, "still out: no repeat line");
        assert_eq!(ps_line(&mut flagged, Some(0.0), "12:00:03"), None, "back: quiet");
        assert!(ps_line(&mut flagged, None, "12:00:04").is_some(), "a new outage speaks again");
    }

    #[test]
    fn trial_lost_line_speaks_per_new_loss() {
        let mut seen = 0;
        assert_eq!(trial_lost_line(&mut seen, 0, "t"), None);
        assert_eq!(trial_lost_line(&mut seen, 1, "t").map(|l| l.msg), Some("history could not save a trial".to_string()));
        assert_eq!(trial_lost_line(&mut seen, 1, "t"), None);
        assert!(trial_lost_line(&mut seen, 2, "t").is_some());
    }

    /// A rig whose reload and clamp-off can fail, over `levers::apply_setup` (clamp first, undo).
    struct FakeRig {
        cur: Setup,
        fail_reload: bool,
        fail_clamp_off: bool,
    }

    impl crate::levers::LeverOps for FakeRig {
        fn setup(&self) -> Setup {
            self.cur
        }
        fn set_clamp(&mut self, on: bool) -> anyhow::Result<()> {
            if !on && self.fail_clamp_off {
                anyhow::bail!("taskpolicy failed");
            }
            self.cur.e_core_clamp = on;
            Ok(())
        }
        fn reload(&mut self, threads: u32, _utility: bool) -> anyhow::Result<()> {
            if self.fail_reload {
                anyhow::bail!("api down");
            }
            self.cur.threads = threads;
            Ok(())
        }
    }

    impl Rigging for FakeRig {
        fn apply(&mut self, s: &Setup) -> anyhow::Result<()> {
            crate::levers::apply_setup(self, s)
        }
        fn setup(&self) -> Setup {
            self.cur
        }
    }

    fn holding(start: Setup) -> Optimizer {
        use crate::optimizer::Mode;
        let prefs = OptimizerPrefs { mode: Mode::Hold, ..Default::default() };
        Optimizer::new(prefs, Limits { threads: 8, e_cores: 4 }, None, Measure::Sys, start, 1_790_000_000.0)
    }

    /// Review M5: the tape names who pulled the lever; Record and Prefs pass through untouched.
    #[test]
    fn carry_out_tags_the_hand_and_the_optimizer() {
        let mut rig = FakeRig { cur: Setup::threads(5), fail_reload: false, fail_clamp_off: false };
        let mut o = holding(Setup::threads(5));
        let e4 = Setup { threads: 4, e_core_clamp: true, utility: false };
        let prefs = OptimizerPrefs::default();
        let acts = vec![(Action::Apply(e4), By::Optimizer), (Action::Prefs(prefs.clone()), By::Optimizer), (Action::Apply(Setup::threads(6)), By::Hand)];
        let c = carry_out(&mut rig, &mut o, acts, "12:00:00");
        let msgs: Vec<&str> = c.tape.iter().map(|l| l.msg.as_str()).collect();
        assert_eq!(msgs, ["optimizer · 4 e-cores", "you set 6 cores"]);
        assert!(c.tape.iter().all(|l| l.tag == "miner"));
        assert_eq!(c.rest, vec![Action::Prefs(prefs)]);
        assert!(c.next.is_empty());
        assert_eq!(rig.cur, Setup::threads(6));
        assert_eq!(o.view().target, Some(Setup::threads(6)), "the optimizer heard what runs");
    }

    /// Review M5: a failed Apply whose clamp undo also fails leaves the clamp on. The optimizer
    /// hears that (`applied` on the Err arm), drops to manual now, and its prefs go out next tick.
    #[test]
    fn a_failed_apply_reports_the_true_rig_and_defers_apply_failed() {
        use crate::optimizer::Mode;
        let mut rig = FakeRig { cur: Setup::threads(5), fail_reload: true, fail_clamp_off: true };
        let mut o = holding(Setup::threads(5));
        let want = Setup { threads: 4, e_core_clamp: true, utility: false };
        let c = carry_out(&mut rig, &mut o, vec![(Action::Apply(want), By::Optimizer)], "12:00:00");
        assert_eq!(rig.cur, Setup { threads: 5, e_core_clamp: true, utility: false }, "the undo failed: the clamp stuck");
        assert_eq!(c.tape.len(), 1);
        assert_eq!(c.tape[0].tag, "error");
        assert!(c.tape[0].msg.starts_with("could not apply 4 e-cores: "), "{}", c.tape[0].msg);
        assert!(c.rest.is_empty());
        assert!(matches!(c.next.as_slice(), [(Action::Prefs(p), By::Optimizer)] if p.mode == Mode::Manual), "{:?}", c.next);
        assert_eq!(o.view().mode, Mode::Manual);
        // `applied` saw the stuck clamp: the hand change to the same count must clear it (F4).
        // Without `applied`, the optimizer would think the clamp is off and pull nothing.
        let acts = o.command(OptCommand::ManualThreads(5));
        assert!(acts.contains(&Action::Apply(Setup::threads(5))), "{acts:?}");
    }

    /// T11 re-review N3: after a refused hand change the tape reports the failure and says the
    /// optimizer let go at what runs, never "you set" a count the user didn't pick.
    #[test]
    fn a_refused_hand_change_is_not_called_the_users_pick() {
        let mut rig = FakeRig { cur: Setup { threads: 5, e_core_clamp: true, utility: false }, fail_reload: false, fail_clamp_off: false };
        let mut o = holding(rig.cur);
        let acts: Vec<(Action, By)> = o.command(OptCommand::ManualThreads(5)).into_iter().map(|a| (a, By::Fallback)).collect();
        let c = carry_out(&mut rig, &mut o, acts, "12:00:00");
        let msgs: Vec<&str> = c.tape.iter().map(|l| l.msg.as_str()).collect();
        assert_eq!(msgs, ["manual at 5 cores"], "the clamp came off at the count that runs");
        assert!(msgs.iter().all(|m| !m.contains("you set")));
        assert_eq!(hand_change_failed(6, &"xmrig refused the reload"), "could not set 6 cores · xmrig refused the reload");
    }

    #[test]
    fn optimizer_command_debug() {
        use crate::optimizer::Profile;
        let d = format!("{:?}", Command::Optimizer(OptCommand::SetProfile(Profile::Light)));
        assert!(d.starts_with("Optimizer(SetProfile("), "{d}");
    }

    #[test]
    fn command_debug_masks_wallet() {
        let wallet = format!("4Q{}", "A".repeat(93));
        let cmd = Command::SetIdentity { wallet: wallet.clone(), worker: "w".into() };
        let d = format!("{cmd:?}");
        assert!(!d.contains(&wallet), "{d}");
        assert!(d.contains('…'), "{d}");
    }
}

/// Dev builds only: every tape line and every optimizer state change, appended to
/// `<data dir>/logs/app.log`, so the whole trial flow can be read back after a run.
#[cfg(feature = "dev-diagnostics")]
fn app_log(snap: &Snapshot, last: &mut String) {
    use std::io::Write;
    let dir = crate::paths::data_dir().join("logs");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("app.log")) else { return };
    for l in &snap.tape {
        let _ = writeln!(f, "{} {} {}", l.ts, l.tag, crate::redact::redact_ips(&l.msg));
    }
    if let Some(o) = &snap.optimizer {
        let line = format!(
            "opt mode={:?} note={:?} sweep={:?} ask_low_power={:?} low_power={:?}",
            o.mode,
            o.note,
            o.sweep.as_ref().map(|s| (s.testing, s.stage, s.done, s.planned)),
            o.ask_low_power,
            snap.low_power
        );
        if line != *last {
            let _ = writeln!(f, "{} {}", clock_hms(), line);
            *last = line;
        }
    }
}
