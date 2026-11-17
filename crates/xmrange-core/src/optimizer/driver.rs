//! The optimizer the engine runs, once per 1 Hz tick: the trials sweep, and holding a throttle.
//! Pure: the engine feeds ticks and commands in and carries the actions out. Time arrives only
//! through `Optimizer::new(.., now)` and each tick's `power.t`.
//!
//! - **Sweep** ("run trials"): every setup in `Limits::plan`, one `TrialRun` each, in the current
//!   Low Power Mode state; then the user is asked to flip Low Power Mode and the same setups run
//!   in the other state. A skipped ask keeps one state. The sweep applies its first trial's setup
//!   the moment it starts, never the user's own setup first, and never waits for the user to step
//!   away. No trial starts on battery; waiting for the outlet is free. A restart resumes, skipping
//!   setups already in the Ledger under the sweep's id.
//! - **Hold**: apply the selected throttle's pick (`super::picks`) and keep it. A hand thread
//!   change switches to manual.
use super::{picks, usable, Limits, Measured, Mode, OptimizerPrefs, Pick, Picks, Profile, Setup, SweepRef};
use crate::context::Context;
use crate::ledger::TrialRecord;
use crate::settings::Measure;
use crate::telemetry::ClockGap;
use crate::trial::{Tick, TrialResult, TrialRun, TrialSpec, TrialStage, TrialStep};
use serde::Serialize;

/// An unanswered Low Power Mode ask during a sweep counts as skipped after this long (spec §6.3).
pub const ASK_TIMEOUT_S: u64 = 10 * 60;

/// "5 threads · e-cores · low power mode", for a note (the mockup's own words, see
/// `ledger::trial_label`, which this mirrors without a `TrialConfig` to build one from).
/// A setup in the words the screen uses (ASD-STE100: one word for one thing, "cores"):
/// "5 cores · utility · low power mode", "4 e-cores".
pub(crate) fn setup_note_label(s: Setup, low_power: Option<bool>) -> String {
    let mut t = s.label();
    match low_power {
        Some(true) => t.push_str(" · low power mode"),
        Some(false) => t.push_str(" · full power"),
        None => {}
    }
    t
}

pub const NOTE_NO_TRIALS: &str = "run trials first";

#[derive(Debug, Clone, PartialEq)]
pub enum OptCommand {
    /// "run trials": start a sweep (a no-op while one runs).
    Seek,
    /// "stop": end the sweep and hold the throttle from what was measured.
    StopSeek,
    Hold,
    SetProfile(Profile),
    ManualThreads(u32),
    SkipLowPower,
    SetMeasure(Measure),
    /// Clicking a trials-card row: run that exact setup and switch to manual. The lever always
    /// applies (threads and the clamp are independent of Low Power Mode); a mismatched Low Power
    /// Mode state shows the usual ask instead of silently doing nothing.
    ApplySetup { setup: Setup, low_power: Option<bool> },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Pull the levers: threads by one live reload, the clamp by taskpolicy. Afterwards the engine
    /// reports what is really running through `Optimizer::applied` (both arms).
    Apply(Setup),
    /// Store a finished trial in the Ledger, labelled with the sweep id.
    Record { setup: Setup, label: String, result: TrialResult },
    /// The prefs changed: the app writes them to settings.json.
    Prefs(OptimizerPrefs),
}

#[derive(Debug, Clone, PartialEq)]
pub struct OptTick {
    pub tick: Tick,
    pub low_power: Option<bool>,
    pub on_ac: Option<bool>,
    /// Monotonic seconds at this tick (`telemetry::mono_secs`). Set against `tick.power.t` it
    /// shows a sleep (ruling P4-C23).
    pub mono: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SweepView {
    pub testing: Option<Setup>,
    pub stage: Option<TrialStage>,
    /// Setups measured so far in the current Low Power Mode state, out of `planned`.
    pub done: u32,
    pub planned: u32,
    /// Seconds into the trial being tested now (settle + measure together); `None` between trials.
    pub elapsed_s: Option<u64>,
    /// Seconds left in the trial by its rules (`TrialRun::left_s`); `None` between trials.
    pub left_s: Option<u64>,
    /// Every sweep trial's fixed length (`trial::TRIAL_S`), for an exact sweep total.
    pub trial_s: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OptimizerView {
    pub profile: Profile,
    pub mode: Mode,
    pub measure: Measure,
    /// What the optimizer runs while holding; `None` in manual (the user's own count rules).
    pub target: Option<Setup>,
    pub sweep: Option<SweepView>,
    /// The throttle held (hold mode, no sweep running).
    pub held: Option<Pick>,
    /// `Some(want)` while the user is asked to flip Low Power Mode.
    pub ask_low_power: Option<bool>,
    /// One plain line: "you set 6 cores", "waiting for the outlet".
    pub note: Option<String>,
    /// Trials written to the Ledger this session; the UI reloads the trials card when it moves.
    pub trials_recorded: u64,
}

pub struct Optimizer {
    prefs: OptimizerPrefs,
    limits: Limits,
    chip: Option<String>,
    measure: Measure,
    /// What xmrig runs now, as far as the optimizer knows (`applied` corrects it).
    current: Setup,
    /// Clean measurements to select from; `None` until the Ledger's trials arrive.
    measured: Option<Vec<Measured>>,
    /// Setups this sweep has measured, with the Low Power Mode state each ran in.
    swept: Vec<(Setup, Option<bool>)>,
    /// The newest sweep in the Ledger and the setups it measured: "run trials" continues it when
    /// it is unfinished, even after a restart or an app version that forgot its place.
    last_sweep: Option<(String, Vec<(Setup, Option<bool>)>)>,
    pending_sweep: bool,
    trial: Option<(Setup, TrialRun)>,
    low_power: Option<bool>,
    on_ac: Option<bool>,
    asking: Option<bool>,
    asked_s: u64,
    note: Option<String>,
    now: f64,
    /// The user's own setup, the one manual runs: what the engine launched in manual, or the last
    /// hand change.
    own: Option<Setup>,
    clock: ClockGap,
    /// The engine's last full context read, for the trials it records.
    ctx: Context,
}

/// Whether `setup` was measured in Low Power Mode state `lp`. An unknown state on either side
/// (not yet read at launch, or not recorded) matches, so a known trial is never re-run just
/// because the power mode wasn't read yet.
fn measured_in(done: &[(Setup, Option<bool>)], setup: Setup, lp: Option<bool>) -> bool {
    done.iter().any(|(s, l)| *s == setup && (l.is_none() || lp.is_none() || *l == lp))
}

impl Optimizer {
    pub fn new(prefs: OptimizerPrefs, limits: Limits, chip: Option<String>, measure: Measure, current: Setup, now: f64) -> Optimizer {
        let own = (prefs.mode == Mode::Manual).then_some(current);
        Optimizer {
            prefs,
            limits,
            chip,
            measure,
            current,
            measured: None,
            swept: Vec::new(),
            last_sweep: None,
            pending_sweep: false,
            trial: None,
            low_power: None,
            on_ac: None,
            asking: None,
            asked_s: 0,
            note: None,
            now,
            own,
            clock: ClockGap::default(),
            ctx: Context::default(),
        }
    }

    /// The setup to launch xmrig with: the held throttle while holding, else the user's own.
    pub fn launch_setup(prefs: &OptimizerPrefs, manual: Setup) -> Setup {
        match (prefs.mode, &prefs.holding) {
            (Mode::Hold, Some(h)) => h.setup,
            _ => manual,
        }
    }

    /// Holding or sweeping: a hand change xmrig refused must still let go.
    pub fn active(&self) -> bool {
        self.prefs.mode == Mode::Hold
    }

    /// A sweep runs: the engine reads `ps` and the input idle time for the busy gate.
    pub fn sweeping(&self) -> bool {
        self.prefs.mode == Mode::Hold && self.prefs.sweep.is_some()
    }

    /// What the rig really runs after an `Apply`, read back by the engine on both arms.
    pub fn applied(&mut self, actual: Setup) {
        self.current = actual;
    }

    /// The user's own setup from settings.json, so a sweep that ends with nothing to hold returns
    /// to the user's thread count.
    pub fn set_own(&mut self, own: Setup) {
        self.own = Some(own);
    }

    /// The engine's latest `read_context()`, stored with each trial.
    pub fn context_read(&mut self, c: Context) {
        self.ctx = c;
    }

    fn context(&self) -> Context {
        Context { low_power_mode: self.low_power, on_ac: self.on_ac, ..self.ctx.clone() }
    }

    fn apply(&mut self, s: Setup) -> Vec<Action> {
        if s == self.current {
            return vec![];
        }
        self.current = s;
        vec![Action::Apply(s)]
    }

    fn prefs_out(&self) -> Action {
        Action::Prefs(self.prefs.clone())
    }

    pub fn picks(&self) -> Picks {
        picks(self.measured.as_deref().unwrap_or(&[]), self.measure, self.limits.threads)
    }

    fn go_manual(&mut self, note: String) {
        self.prefs.mode = Mode::Manual;
        // A hand change stops the sweep; its trials stay in the Ledger and "run trials" continues it.
        self.prefs.sweep = None;
        self.trial = None;
        self.asking = None;
        self.pending_sweep = false;
        self.note = Some(note);
    }

    /// Hold the selected throttle: apply its pick, and ask for its Low Power Mode state if the Mac
    /// is in the other one. With no trials there is nothing to hold: manual, on the user's own.
    fn hold(&mut self) -> Vec<Action> {
        if self.measured.is_none() {
            self.prefs.mode = Mode::Hold;
            self.note = Some("reading past trials".into());
            return vec![self.prefs_out()];
        }
        let mut out = Vec::new();
        match self.picks().get(self.prefs.profile).cloned() {
            Some(p) => {
                self.prefs.mode = Mode::Hold;
                self.note = None;
                self.asking = match (p.low_power, self.low_power) {
                    (Some(want), Some(now)) if want != now => Some(want),
                    _ => None,
                };
                out.extend(self.apply(p.setup));
                self.prefs.holding = Some(p);
            }
            None => {
                self.go_manual(NOTE_NO_TRIALS.into());
                let own = self.own.unwrap_or(Setup::threads(self.current.threads));
                out.extend(self.apply(own));
            }
        }
        out.push(self.prefs_out());
        out
    }

    pub fn history_loaded(&mut self, history: Vec<TrialRecord>, low_power: Option<bool>) -> Vec<Action> {
        if low_power.is_some() {
            self.low_power = low_power;
        }
        let newest = history.iter().filter(|t| t.label.starts_with("sweep-")).max_by(|a, b| a.t_end.total_cmp(&b.t_end)).map(|t| t.label.clone());
        self.last_sweep = newest.map(|id| {
            let done = history.iter().filter(|t| t.label == id).map(|t| (t.config.setup(), t.config.low_power_mode)).collect();
            (id, done)
        });
        if let Some(r) = self.prefs.sweep.as_ref() {
            self.swept = history.iter().filter(|t| t.label == r.id).map(|t| (t.config.setup(), t.config.low_power_mode)).collect();
        }
        self.measured = Some(usable(&history, self.chip.as_deref()));
        if std::mem::take(&mut self.pending_sweep) {
            return self.start_sweep();
        }
        match (self.prefs.mode, &self.prefs.sweep) {
            (Mode::Manual, _) => vec![],
            (Mode::Hold, Some(_)) => {
                self.note = Some("trials resumed".into());
                vec![]
            }
            (Mode::Hold, None) => self.hold(),
        }
    }

    fn start_sweep(&mut self) -> Vec<Action> {
        if self.measured.is_none() {
            self.pending_sweep = true;
            self.prefs.mode = Mode::Hold;
            self.note = Some("reading past trials".into());
            return vec![];
        }
        if self.prefs.mode == Mode::Manual {
            self.own = Some(self.current);
        }
        self.prefs.mode = Mode::Hold;
        // The Ledger is the only record of progress: continue the newest sweep if it still has
        // setups left in this power mode; otherwise start a fresh one.
        match () {
            _ if self.last_sweep.as_ref().is_some_and(|(_, done)| {
                self.limits.plan().into_iter().any(|s| !measured_in(done, s, self.low_power))
            }) =>
            {
                let (id, done) = self.last_sweep.clone().expect("checked");
                self.prefs.sweep = Some(SweepRef { id, skipped: false });
                self.swept = done;
            }
            _ => {
                self.prefs.sweep = Some(SweepRef { id: format!("sweep-{}", self.now.max(0.0) as u64), skipped: false });
                self.swept.clear();
            }
        }
        self.trial = None;
        self.asking = None;
        self.note = None;
        // Apply the first trial's setup right away: a sweep never mines at the user's own setup
        // first, and never waits for a settle tick to switch down to it.
        let mut out = vec![self.prefs_out()];
        out.extend(self.advance());
        out
    }

    fn end_sweep(&mut self) -> Vec<Action> {
        self.prefs.sweep = None;
        self.last_sweep = None; // finished: the next "run trials" starts a fresh sweep
        self.trial = None;
        self.asking = None;
        self.hold()
    }

    pub fn command(&mut self, c: OptCommand) -> Vec<Action> {
        match c {
            OptCommand::Seek if self.prefs.sweep.is_some() && self.prefs.mode == Mode::Hold => vec![],
            OptCommand::Seek => self.start_sweep(),
            OptCommand::StopSeek if self.prefs.sweep.is_some() => {
                // Stop ends this run and pulls no lever (the app stops mining). The trials measured
                // so far are in the Ledger and `last_sweep`: "run trials" continues after them.
                self.prefs.sweep = None;
                self.trial = None;
                self.asking = None;
                vec![self.prefs_out()]
            }
            OptCommand::StopSeek => vec![],
            OptCommand::Hold if self.prefs.sweep.is_some() => vec![],
            OptCommand::Hold => self.hold(),
            OptCommand::SetProfile(p) => {
                self.prefs.profile = p;
                if self.prefs.sweep.is_some() && self.prefs.mode == Mode::Hold {
                    return vec![self.prefs_out()]; // held once the sweep ends
                }
                if self.prefs.mode == Mode::Manual {
                    self.own = Some(self.current);
                }
                self.hold()
            }
            OptCommand::ManualThreads(n) => {
                // The engine already set the thread count. Manual means the user's own setup,
                // which carries no clamp: one Apply of the user's own input, then nothing more.
                let was = Setup { threads: n, ..self.current };
                self.go_manual(format!("you set {n} core{}", if n == 1 { "" } else { "s" }));
                let own = Setup::threads(n);
                self.own = Some(own);
                self.current = was;
                let mut out = self.apply(own);
                out.push(self.prefs_out());
                out
            }
            OptCommand::SkipLowPower => {
                self.asking = None;
                self.asked_s = 0;
                match self.prefs.sweep.as_mut() {
                    Some(r) => {
                        r.skipped = true;
                        vec![self.prefs_out()]
                    }
                    None => vec![],
                }
            }
            OptCommand::SetMeasure(m) => {
                self.measure = m;
                if self.prefs.mode == Mode::Hold && self.prefs.sweep.is_none() && self.measured.is_some() {
                    return self.hold();
                }
                vec![]
            }
            OptCommand::ApplySetup { setup, low_power } => {
                self.go_manual(format!("you picked {}", setup_note_label(setup, low_power)));
                self.own = Some(setup);
                let mut out = self.apply(setup);
                self.asking = match (low_power, self.low_power) {
                    (Some(want), Some(now)) if want != now => Some(want),
                    _ => None,
                };
                out.push(self.prefs_out());
                out
            }
        }
    }

    /// A paused miner measures nothing: the running trial is dropped and runs again on resume.
    pub fn paused(&mut self) -> Vec<Action> {
        self.trial = None;
        vec![]
    }

    /// A lever the engine couldn't pull: stop steering and say so.
    pub fn apply_failed(&mut self, err: &str) -> Vec<Action> {
        self.go_manual(err.to_string());
        vec![self.prefs_out()]
    }

    fn planned_in(&self, lp: Option<bool>) -> impl Iterator<Item = Setup> + '_ {
        self.limits.plan().into_iter().filter(move |s| !measured_in(&self.swept, *s, lp))
    }

    pub fn on_tick(&mut self, t: &OptTick) -> Vec<Action> {
        self.now = t.tick.power.t;
        let slept = self.clock.step(t.tick.power.t, t.mono).is_some();
        if t.low_power.is_some() {
            if self.sweeping() && self.low_power.is_some() && t.low_power != self.low_power {
                // Low Power Mode changed (macOS turns it on by itself on battery if set to "only on
                // battery"): each power mode has its own set of trials and its own count. Say so.
                self.note = Some(format!("low power mode {} · it has its own trials", if t.low_power == Some(true) { "on" } else { "off" }));
            }
            self.low_power = t.low_power;
        }
        if t.on_ac.is_some() {
            self.on_ac = t.on_ac;
        }
        if self.prefs.mode == Mode::Manual {
            return vec![]; // never fight the user (spec §2)
        }
        if self.asking.is_some() && self.asking == self.low_power {
            self.asking = None; // the user flipped it
        }
        if !self.sweeping() || self.measured.is_none() {
            return vec![];
        }
        if slept {
            // The trial across a sleep measured two machines (the chip cooled asleep): dropped.
            self.trial = None;
        }

        if let Some((setup, mut run)) = self.trial.take() {
            return match run.push(t.tick.clone(), t.low_power) {
                TrialStep::Running(_) => {
                    self.trial = Some((setup, run));
                    vec![]
                }
                TrialStep::LowPowerChanged { .. } => vec![], // dropped; the new state's setups come next
                TrialStep::Complete => {
                    let result = run.finish(self.context());
                    let lp = result.context_start.low_power_mode;
                    self.swept.push((setup, lp));
                    // Keep `last_sweep` current, so stop + run trials continues without a restart.
                    if let Some(r) = &self.prefs.sweep {
                        self.last_sweep = Some((r.id.clone(), self.swept.clone()));
                    }
                    // Every whole trial counts, noisy or not, settled or not: under the fixed standard
                    // the warm-up is time, never a gate (0.1.34's `usable` rule, the same here).
                    let clean = result.score.clone().filter(|_| !result.partial);
                    if let (Some(score), Some(m)) = (clean, self.measured.as_mut()) {
                        m.retain(|x| !(x.setup == setup && x.low_power == lp));
                        m.push(Measured { setup, low_power: lp, score });
                    }
                    let label = self.prefs.sweep.as_ref().map_or_else(String::new, |r| r.id.clone());
                    vec![Action::Record { setup, label, result }]
                }
            };
        }

        self.advance()
    }

    /// Start (or resume) the next trial: the next unmeasured setup in the current Low Power Mode
    /// state, applied right away — no wait for the user to be quiet, only for the outlet. Once a
    /// state is fully measured, ask for the other one (or end the sweep). Called both from
    /// `start_sweep` (so a sweep begins at its first trial's setup, never the user's own) and from
    /// `on_tick` once a trial finishes.
    fn advance(&mut self) -> Vec<Action> {
        let lp = self.low_power;
        let next = self.planned_in(lp).next();
        if let Some(setup) = next {
            self.asking = None;
            self.asked_s = 0;
            return match TrialRun::new(TrialSpec::seeker(setup.threads), self.context()) {
                Ok(run) => {
                    self.trial = Some((setup, run));
                    self.apply(setup)
                }
                Err(e) => self.apply_failed(&e.to_string()),
            };
        }
        // This state is done. Ask for the other one, unless it is done, skipped or unknowable.
        let skipped = self.prefs.sweep.as_ref().is_some_and(|r| r.skipped);
        let other = lp.map(|l| !l);
        if other.is_some() && !skipped && self.planned_in(other).next().is_some() {
            self.asking = other;
            self.asked_s += 1;
            if self.asked_s >= ASK_TIMEOUT_S {
                return self.command(OptCommand::SkipLowPower);
            }
            return vec![];
        }
        self.end_sweep()
    }

    pub fn view(&self) -> OptimizerView {
        let sweeping = self.sweeping();
        OptimizerView {
            profile: self.prefs.profile,
            mode: self.prefs.mode,
            measure: self.measure,
            target: (self.prefs.mode == Mode::Hold).then_some(self.current),
            sweep: sweeping.then(|| {
                let planned = self.limits.plan().len() as u32;
                SweepView {
                    testing: self.trial.as_ref().map(|t| t.0),
                    stage: self.trial.as_ref().map(|t| t.1.stage()),
                    done: planned - self.planned_in(self.low_power).count() as u32,
                    planned,
                    elapsed_s: self.trial.as_ref().map(|t| t.1.elapsed_s()),
                    left_s: self.trial.as_ref().map(|t| t.1.left_s()),
                    trial_s: crate::trial::TRIAL_S,
                }
            }),
            held: (self.prefs.mode == Mode::Hold && !sweeping).then(|| self.prefs.holding.clone()).flatten(),
            ask_low_power: self.asking,
            note: self.note.clone(),
            trials_recorded: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::optimizer::testkit::m3_all;
    use crate::telemetry::PowerSample;

    const M3: Limits = Limits { threads: 8, e_cores: 4 };

    /// A Mac that runs whatever the optimizer applies, reading the measured M3 (an unmeasured
    /// setup reads as the nearest measured thread count, so every trial scores).
    struct Rig {
        setup: Setup,
        lp: Option<bool>,
        on_ac: Option<bool>,
        t: f64,
        recorded: Vec<(Setup, Option<bool>)>,
        /// A whole trial table to score from (the playground's), exact setup and state; else the M3 fixture.
        table: Option<Vec<crate::optimizer::Measured>>,
    }

    impl Rig {
        fn new() -> Rig {
            Rig { setup: Setup::threads(5), lp: Some(true), on_ac: Some(true), t: 1_790_000_000.0, recorded: vec![], table: None }
        }

        fn tick(&self) -> OptTick {
            let all = self.table.clone().unwrap_or_else(m3_all);
            let row = match &self.table {
                Some(t) => t.iter().find(|x| x.setup == self.setup && x.low_power == self.lp).unwrap_or_else(|| panic!("no row for {:?} {:?}", self.setup, self.lp)),
                None => all
                    .iter()
                    .filter(|x| x.low_power == self.lp && x.setup.e_core_clamp == self.setup.e_core_clamp)
                    .min_by_key(|x| x.setup.threads.abs_diff(self.setup.threads))
                    .unwrap(),
            };
            let s = &row.score;
            OptTick {
                tick: Tick {
                    power: PowerSample { t: self.t, cpu_w: s.cpu_w, ram_w: 0.7, sys_w: s.sys_w, temp_c: s.temp_c, ..Default::default() },
                    hs10: Some(s.hs),
                    other_cpu_pct: Some(150.0),
                    other_cpu_excess: Some(0.0),
                },
                low_power: self.lp,
                on_ac: self.on_ac,
                mono: self.t,
            }
        }

        fn apply_acts(&mut self, acts: &[Action]) {
            for a in acts {
                match a {
                    Action::Apply(s) => self.setup = *s,
                    Action::Record { setup, result, .. } => self.recorded.push((*setup, result.context_start.low_power_mode)),
                    Action::Prefs(_) => {}
                }
            }
        }

        fn step(&mut self, o: &mut Optimizer) -> Vec<Action> {
            let acts = o.on_tick(&self.tick());
            self.t += 1.0;
            self.apply_acts(&acts);
            acts
        }

        /// A command whose actions (an `Apply` in particular, now that a sweep can apply its first
        /// trial's setup right away) must land on the rig, same as a tick's.
        fn command(&mut self, o: &mut Optimizer, c: OptCommand) -> Vec<Action> {
            let acts = o.command(c);
            self.apply_acts(&acts);
            acts
        }

        /// Ticks until `done` holds (at most `max` ticks).
        fn run_until(&mut self, o: &mut Optimizer, max: u32, done: impl Fn(&Optimizer) -> bool) {
            for _ in 0..max {
                if done(o) {
                    return;
                }
                self.step(o);
            }
            panic!("not within {max} ticks: {:?}", o.view());
        }
    }

    fn fresh(prefs: OptimizerPrefs, history: Vec<TrialRecord>) -> Optimizer {
        let mut o = Optimizer::new(prefs, M3, Some("Apple M3".into()), Measure::Sys, Setup::threads(5), 1_790_000_000.0);
        o.history_loaded(history, Some(true));
        o
    }

    const TRIAL_MAX: u32 = 20 * 60;

    /// The playground end to end: for every Mac shape, the real optimizer sweeps a varied table
    /// (both power states on a laptop), then each profile it holds keeps the owner's rules.
    #[test]
    fn every_mac_shape_sweeps_then_holds_profiles_that_keep_the_owners_rules() {
        use crate::optimizer::playground::{check, synth, SHAPES};
        for shape in SHAPES {
            for seed in [3, 41, 77] {
                let limits = Limits { threads: shape.threads, e_cores: shape.e_cores };
                let lp0 = if shape.laptop { Some(false) } else { None };
                let mut o = Optimizer::new(OptimizerPrefs::default(), limits, Some("Apple M3".into()), Measure::Sys, Setup::threads(1), 1_790_000_000.0);
                o.history_loaded(vec![], lp0);
                let mut rig = Rig { setup: Setup::threads(1), lp: lp0, table: Some(synth(shape, seed)), ..Rig::new() };
                rig.command(&mut o, OptCommand::Seek);
                let budget = 60 * TRIAL_MAX;
                rig.run_until(&mut o, budget, |o| o.view().ask_low_power.is_some() || !o.sweeping());
                if o.view().ask_low_power.is_some() {
                    rig.lp = Some(true);
                    rig.run_until(&mut o, budget, |o| !o.sweeping());
                }
                let per_state = limits.plan().len();
                assert_eq!(rig.recorded.len(), per_state * if shape.laptop { 2 } else { 1 }, "{}: every setup measured", shape.name);
                check(shape, &o.picks()).unwrap_or_else(|e| panic!("{} seed {seed}: {e}", shape.name));
                for profile in [Profile::Light, Profile::Efficient, Profile::Max] {
                    rig.command(&mut o, OptCommand::SetProfile(profile));
                    let held = o.view().held.unwrap_or_else(|| panic!("{}: nothing held for {profile:?}", shape.name));
                    assert_eq!(Some(&held), o.picks().get(profile), "{} {profile:?}: holds its pick", shape.name);
                    assert_eq!(rig.setup, held.setup, "{} {profile:?}: the rig runs the held setup", shape.name);
                    if profile != Profile::Max {
                        assert!(held.setup.threads < shape.threads || shape.threads == 1, "{} {profile:?} uses every thread: {:?}", shape.name, held.setup);
                    }
                }
            }
        }
    }

    /// The sweep runs all 17 setups in the current state, asks for the other, runs 17 more there,
    /// then holds the efficient throttle, asking to restore its Low Power Mode state.
    #[test]
    fn a_sweep_measures_every_setup_in_both_states_then_holds_efficient() {
        let mut o = fresh(OptimizerPrefs::default(), vec![]);
        let mut rig = Rig::new();
        let seek = rig.command(&mut o, OptCommand::Seek);
        assert!(matches!(seek.as_slice(), [Action::Prefs(p), Action::Apply(s)] if p.sweep.is_some() && p.mode == Mode::Hold && *s == Setup::threads(1)), "{seek:?}");
        rig.run_until(&mut o, 17 * TRIAL_MAX, |o| o.view().ask_low_power.is_some());
        assert_eq!(o.view().ask_low_power, Some(false), "asks to turn it off");
        assert_eq!(rig.recorded, M3.plan().into_iter().map(|s| (s, Some(true))).collect::<Vec<_>>());
        rig.lp = Some(false); // the user flips it
        rig.run_until(&mut o, 17 * TRIAL_MAX, |o| !o.sweeping());
        assert_eq!(rig.recorded.len(), 34);
        let v = o.view();
        let held = v.held.unwrap();
        assert_eq!((v.mode, v.profile, held.setup, held.low_power), (Mode::Hold, Profile::Efficient, held.setup, Some(true)));
        assert_eq!(held.setup.threads, 4, "balanced is half the cores; the rig scores utility like default, so either 4-thread setup may win");
        assert_eq!(rig.setup, held.setup);
        assert_eq!(v.ask_low_power, Some(true), "efficient ran under Low Power Mode: ask to turn it back on");
    }

    #[test]
    fn a_skipped_flip_keeps_one_state() {
        let mut o = fresh(OptimizerPrefs::default(), vec![]);
        let mut rig = Rig::new();
        rig.command(&mut o, OptCommand::Seek);
        rig.run_until(&mut o, 17 * TRIAL_MAX, |o| o.view().ask_low_power.is_some());
        o.command(OptCommand::SkipLowPower);
        rig.step(&mut o);
        assert!(!o.sweeping() && rig.recorded.len() == 17);
        assert_eq!(o.view().held.unwrap().setup.threads, 4);
    }

    /// A restart resumes the sweep by its id: setups already in the Ledger under it are skipped.
    #[test]
    fn a_resumed_sweep_skips_what_it_measured() {
        let mut o = fresh(OptimizerPrefs::default(), vec![]);
        let mut rig = Rig::new();
        rig.command(&mut o, OptCommand::Seek);
        let id = o.prefs.sweep.clone().unwrap().id;
        rig.run_until(&mut o, 3 * TRIAL_MAX, |o| rig_done(o, 3));
        let history: Vec<TrialRecord> = rig
            .recorded
            .iter()
            .map(|(s, lp)| TrialRecord {
                id: None, t_start: 0.0, t_end: 1.0, label: id.clone(), source: "sweep".into(), chip: Some("Apple M3".into()),
                config: crate::ledger::TrialConfig { threads: s.threads, e_core_clamp: s.e_core_clamp, low_power_mode: *lp, cpu_yield: true, utility: false },
                settled: true, settle_s: 60, partial: false, score: None,
                context_start: Context::default(), context_end: Context::default(), wallet_tag: None,
            })
            .collect();
        let mut again = fresh(o.prefs.clone(), history);
        assert_eq!(again.view().sweep.unwrap().done, 3);
        let mut rig2 = Rig { t: rig.t, ..Rig::new() };
        rig2.run_until(&mut again, TRIAL_MAX, |o| o.view().sweep.is_some_and(|s| s.testing.is_some()));
        assert_eq!(again.view().sweep.unwrap().testing, Some(Setup::threads(4)), "picks up at the 4th setup");
    }

    /// No paused sweep in settings (an older version forgot it): "run trials" still continues the
    /// newest unfinished sweep in the Ledger, after its last measured setup.
    #[test]
    fn run_trials_continues_the_newest_unfinished_sweep_in_the_ledger() {
        let rec = |threads: u32| TrialRecord {
            id: None, t_start: 0.0, t_end: 1.0, label: "sweep-100".into(), source: "sweep".into(), chip: Some("Apple M3".into()),
            config: crate::ledger::TrialConfig { threads, e_core_clamp: false, low_power_mode: Some(true), cpu_yield: true, utility: false },
            settled: true, settle_s: 60, partial: false, score: None,
            context_start: Context::default(), context_end: Context::default(), wallet_tag: None,
        };
        let mut o = fresh(OptimizerPrefs::default(), vec![rec(1)]);
        o.low_power = None; // the power mode not read yet at launch must not restart the sweep
        let acts = o.command(OptCommand::Seek);
        assert!(acts.contains(&Action::Apply(Setup::threads(2))), "continues at trial 2: {acts:?}");
        assert_eq!(o.prefs.sweep.as_ref().map(|r| r.id.as_str()), Some("sweep-100"));
    }

    /// Stop trials pauses and pulls no lever (the app stops mining).
    #[test]
    fn stop_trials_applies_nothing() {
        let mut o = fresh(OptimizerPrefs::default(), vec![]);
        o.command(OptCommand::Seek);
        let acts = o.command(OptCommand::StopSeek);
        assert!(!acts.iter().any(|a| matches!(a, Action::Apply(_))), "{acts:?}");
    }

    fn rig_done(o: &Optimizer, n: u32) -> bool {
        o.view().sweep.is_some_and(|s| s.done >= n)
    }

    /// The owner's bug: "run trials" must apply trial 1's setup before mining starts or the
    /// instant the sweep begins, never mine at the user's own setup first, and never gate the
    /// first trial on a busy-user reading.
    #[test]
    fn a_sweep_starts_at_its_own_setup_before_any_settle_with_no_busy_wait() {
        let mut o = fresh(OptimizerPrefs::default(), vec![]);
        // The user is at the keyboard, mining their own 5 threads, when they press "run trials".
        let acts = o.command(OptCommand::Seek);
        assert!(acts.contains(&Action::Apply(Setup::threads(1))), "applies trial 1's setup immediately: {acts:?}");
        let v = o.view();
        let sweep = v.sweep.expect("sweeping");
        assert_eq!(sweep.testing, Some(Setup::threads(1)));
        assert_eq!(sweep.elapsed_s, Some(0), "no settle ticks have run yet");
        assert_ne!(v.note.as_deref(), Some("waiting for a quiet mac · trials run when you step away"), "never waits for the user to step away");
    }

    /// Stop keeps the sweep's place: "run trials" resumes at the next setup, not trial 1.
    #[test]
    fn stop_then_run_trials_resumes_where_it_left_off() {
        let mut o = fresh(OptimizerPrefs::default(), vec![]);
        let mut rig = Rig::new();
        rig.command(&mut o, OptCommand::Seek);
        let id = o.prefs.sweep.clone().expect("sweeping").id;
        rig.run_until(&mut o, TRIAL_MAX, |o| rig_done(o, 1)); // trial 1 measured for real
        o.command(OptCommand::StopSeek);
        assert!(o.prefs.sweep.is_none(), "stopped");
        let acts = o.command(OptCommand::Seek);
        assert!(acts.contains(&Action::Apply(Setup::threads(2))), "resumes at trial 2: {acts:?}");
        assert_eq!(o.prefs.sweep.as_ref().map(|r| r.id.clone()), Some(id), "same sweep");
    }

    /// Applying a throttle holds it: nothing moves on its own afterwards.
    #[test]
    fn a_throttle_holds_without_re_probing() {
        let mut o = fresh(OptimizerPrefs::default(), vec![]);
        o.measured = Some(m3_all());
        let a = o.command(OptCommand::SetProfile(Profile::Max));
        assert!(a.contains(&Action::Apply(Setup::threads(8))), "{a:?}");
        assert_eq!(o.view().ask_low_power, Some(false), "max runs at full power: ask to turn Low Power Mode off");
        let mut rig = Rig { setup: Setup::threads(8), ..Rig::new() };
        for _ in 0..(24 * 3600) {
            assert!(rig.step(&mut o).is_empty());
        }
        let light = o.command(OptCommand::SetProfile(Profile::Light));
        assert!(light.contains(&Action::Apply(Setup { threads: 4, e_core_clamp: true, utility: false })), "{light:?}");
    }

    #[test]
    fn a_hand_thread_change_switches_to_manual() {
        let mut o = fresh(OptimizerPrefs::default(), vec![]);
        o.command(OptCommand::Seek);
        let a = o.command(OptCommand::ManualThreads(6));
        assert!(matches!(a.last(), Some(Action::Prefs(p)) if p.mode == Mode::Manual && p.sweep.is_none()));
        let mut rig = Rig::new();
        for _ in 0..600 {
            assert!(rig.step(&mut o).is_empty());
        }
        let v = o.view();
        assert_eq!((v.mode, v.note.as_deref(), v.target), (Mode::Manual, Some("you set 6 cores"), None));
    }

    #[test]
    fn holding_with_no_trials_says_run_trials_first() {
        let mut o = fresh(OptimizerPrefs::default(), vec![]);
        o.command(OptCommand::Hold);
        let v = o.view();
        assert_eq!((v.mode, v.note.as_deref()), (Mode::Manual, Some(NOTE_NO_TRIALS)));
    }
}
