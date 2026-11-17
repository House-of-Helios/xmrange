//! The optimizer: a trials sweep over every relevant setting, and three throttles selected from
//! its results (THESIS 7, 11, 14, 15, 16). A throttle never decides which trials run.
//!
//! - The sweep measures threads 1..=all unclamped and 1..=E-cores with the E-core clamp, in the
//!   current Low Power Mode state, then asks the user to flip Low Power Mode and runs the same
//!   setups in the other state (`driver`).
//! - The throttles are pure selections over the measured trials (`picks`): light, efficient, max.
//!
//! Pure: no I/O and no clock reads, so every rule runs in tests fed by real `runs/` numbers.
pub mod driver;

pub use crate::levers::Setup;

use crate::ledger::TrialRecord;
use crate::settings::Measure;
use crate::trial::Score;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// This Mac's shape: logical threads, and the E-cluster's core count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    pub threads: u32,
    pub e_cores: u32,
}

impl Limits {
    /// The sweep's setups, in order: 1..=threads unclamped, then the background band (quiet's
    /// candidates): 1..=E-cores on Apple Silicon, or up to a quarter of the threads on a Mac with
    /// no E-cores (Intel), where `taskpolicy -b` still lowers xmrig to background priority.
    pub fn plan(&self) -> Vec<Setup> {
        let open = (1..=self.threads.max(1)).map(Setup::threads);
        let band = if self.e_cores > 0 { self.e_cores } else { (self.threads / 4).max(1) };
        let clamped = (1..=band.min(self.threads)).map(|n| Setup { threads: n, e_core_clamp: true, utility: false });
        // Utility QoS on half the threads and up: balanced's polite candidates (owner, 2026-10-03).
        let utility = (self.threads.div_ceil(2).max(1)..=self.threads).map(|n| Setup { threads: n, e_core_clamp: false, utility: true });
        open.chain(utility).chain(clamped).collect()
    }
}

/// Hold a throttle (the optimizer applies it and keeps it) or manual (the user's own threads).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Hold,
    #[default]
    Manual,
}

/// The three throttles. Settings saved before the sweep named them quiet and balanced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Profile {
    /// "quiet": the best-H/W background-band (clamped) setup: E-cores on Apple Silicon.
    #[serde(alias = "quiet")]
    Light,
    /// "balanced": about half the cores (40–60% of the threads, no clamp), the best H/W among them
    /// across Low Power Mode states, on fewer threads only when they tie on clearly fewer watts.
    #[default]
    #[serde(alias = "balanced")]
    Efficient,
    /// "max": every thread at full power (Low Power Mode off).
    Max,
}

/// A profile's name on screen (owner, 2026-10-03): how much of the Mac it takes.
pub fn profile_name(p: Profile) -> &'static str {
    match p {
        Profile::Light => "quiet",
        Profile::Efficient => "balanced",
        Profile::Max => "max",
    }
}

/// One clean measurement: the setup, the Low Power Mode state it ran in, and its score.
#[derive(Debug, Clone, PartialEq)]
pub struct Measured {
    pub setup: Setup,
    pub low_power: Option<bool>,
    pub score: Score,
}

/// What a throttle runs: the setup, the Low Power Mode state it was measured in, and its figures
/// on the denominator it was picked by.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pick {
    pub setup: Setup,
    pub low_power: Option<bool>,
    pub hw: Option<f64>,
    pub hs: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Picks {
    pub light: Option<Pick>,
    pub efficient: Option<Pick>,
    pub max: Option<Pick>,
}

impl Picks {
    pub fn get(&self, p: Profile) -> Option<&Pick> {
        match p {
            Profile::Light => self.light.as_ref(),
            Profile::Efficient => self.efficient.as_ref(),
            Profile::Max => self.max.as_ref(),
        }
    }
}

/// A sweep in progress, kept in settings.json so it resumes after a restart: its trials carry
/// `id` as their Ledger label, and a resume skips every setup already measured under it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SweepRef {
    pub id: String,
    /// The user skipped the Low Power Mode flip: the sweep ends with one state.
    #[serde(default)]
    pub skipped: bool,
}

/// What the optimizer keeps between launches, inside settings.json. Nothing secret.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OptimizerPrefs {
    pub profile: Profile,
    pub mode: Mode,
    pub sweep: Option<SweepRef>,
    /// The throttle's setup last applied: a launch in hold starts on it. (Settings from before the
    /// sweep kept a `held` list of Seeker winners; it is ignored.)
    pub holding: Option<Pick>,
}

impl OptimizerPrefs {
    /// A hand-edited settings.json can't make a launch run 12 threads on an 8-thread Mac.
    pub fn sanitize(&mut self, max_threads: u32) {
        if self.holding.as_ref().is_some_and(|h| !(1..=max_threads).contains(&h.setup.threads)) {
            self.holding = None;
        }
    }
}

/// H/W over the chosen denominator ("h/w counts": whole mac or cpu only).
pub fn hw(s: &Score, m: Measure) -> Option<f64> {
    match m {
        Measure::Sys => s.hw_sys,
        Measure::Cpu => s.hw_cpu,
    }
}

/// Two trials closer than this fraction of the larger H/W are a tie, unless their own CIs are
/// wider still. THESIS 15: under Low Power Mode, threads 6–8 "add under 2%".
pub const BAND_FRAC: f64 = 0.02;

/// A setup with fewer threads must save at least this share of the watts to be the efficient
/// pick over the best-H/W setup it ties.
pub const WATT_SAVING: f64 = 0.05;

fn watts(s: &Score, m: Measure) -> Option<f64> {
    match m {
        Measure::Sys => s.sys_w,
        Measure::Cpu => Some(s.cpu_w),
    }
}

/// `x` ties `best` on H/W: closer than the noise band (`BAND_FRAC`, or either trial's CI if wider).
fn tied(x: &Score, best: &Score, m: Measure) -> bool {
    let (Some(a), Some(b)) = (hw(x, m), hw(best, m)) else { return false };
    let ci = |s: &Score| match m {
        Measure::Sys => s.hw_sys_ci95,
        Measure::Cpu => s.hw_cpu_ci95,
    };
    let band = [ci(x), ci(best)].into_iter().flatten().fold(BAND_FRAC * b.abs(), f64::max);
    b - a <= band
}

/// The efficient setup of one curve (one Low Power Mode state, one clamp setting): the best H/W,
/// unless fewer threads tie it while drawing at least `WATT_SAVING` fewer watts, then the fewest
/// such. A core that costs no watts stays on. (Owner, 2026-10-03: on the M1 iMac the 8th core
/// added 1.6% H/W at the same 25.6 W, and the old marginal-gain knee dropped it, leaving
/// "efficient" one core short of "max" for nothing.)
fn efficient_of<'a>(curve: Vec<&'a Measured>, m: Measure) -> Option<&'a Measured> {
    let best = best_hw(curve.iter().copied(), m)?;
    let limit = watts(&best.score, m)? * (1.0 - WATT_SAVING);
    curve
        .iter()
        .copied()
        .filter(|x| x.setup.threads < best.setup.threads && tied(&x.score, &best.score, m) && watts(&x.score, m).is_some_and(|w| w <= limit))
        .min_by_key(|x| x.setup.threads)
        .or(Some(best))
}

fn pick(x: &Measured, m: Measure) -> Pick {
    Pick { setup: x.setup, low_power: x.low_power, hw: hw(&x.score, m), hs: x.score.hs }
}

fn best_hw<'a>(xs: impl Iterator<Item = &'a Measured>, m: Measure) -> Option<&'a Measured> {
    xs.filter(|x| hw(&x.score, m).is_some()).max_by(|a, b| hw(&a.score, m).unwrap().total_cmp(&hw(&b.score, m).unwrap()))
}

/// The three throttles over clean measurements, ranked on `m`. `max_threads`: this Mac's count.
pub fn picks(items: &[Measured], m: Measure, max_threads: u32) -> Picks {
    let groups = [(Some(false), false), (Some(false), true), (Some(true), false), (Some(true), true), (None, false), (None, true)];
    // Balanced (owner, 2026-10-03): a core budget, about half the Mac (40–60% of the threads, all
    // scheduled by macOS), so the rest stays free for the user; the score picks inside it. Budgets
    // on CPU watts were tried and dropped: CPU watts flatten at the top on every chip measured
    // (M1 iMac: 4 and 8 threads both 12.0 W), so a watt band can't tell balanced from max.
    let (lo, hi) = (((max_threads as f64) * 0.4).ceil() as u32, ((max_threads as f64) * 0.6).floor() as u32);
    // Utility QoS setups compete inside the same core budget, never outside it (owner, 2026-10-03:
    // the M1 iMac's balanced held 8 threads at utility, its best h/w, and 8 cores is max, not
    // balanced). Utility runs above the band stay in the trials, pickable by hand.
    let open: Vec<&Measured> = items.iter().filter(|x| !x.setup.e_core_clamp).collect();
    let mut ranged: Vec<&Measured> = open.iter().copied().filter(|x| (lo.max(1)..=hi).contains(&x.setup.threads)).collect();
    if ranged.is_empty() {
        // Too few threads for the band (a 2-thread Mac): the count nearest half.
        let half = max_threads as f64 / 2.0;
        let near = open.iter().map(|x| x.setup.threads).min_by(|a, b| (*a as f64 - half).abs().total_cmp(&(*b as f64 - half).abs()));
        ranged = open.iter().copied().filter(|x| Some(x.setup.threads) == near).collect();
    }
    let candidates: Vec<&Measured> = groups
        .iter()
        .filter(|(_, clamp)| !clamp)
        .filter_map(|&(lp, _)| efficient_of(ranged.iter().copied().filter(|x| x.low_power == lp).collect(), m))
        .collect();
    let all = Setup::threads(max_threads);
    // Full power first; a Mac measured only under Low Power Mode (or with it unknown) still has a max.
    let max = [Some(false), None, Some(true)].iter().find_map(|&lp| items.iter().find(|x| x.setup == all && x.low_power == lp));
    Picks {
        light: best_hw(items.iter().filter(|x| x.setup.e_core_clamp), m).map(|x| pick(x, m)),
        efficient: best_hw(candidates.into_iter(), m).map(|x| pick(x, m)),
        max: max.map(|x| pick(x, m)),
    }
}

/// The measurements the throttles select from: settled, whole, clean trials on AC from this chip,
/// the newest per setup and Low Power Mode state. `records` come newest first, as the Ledger
/// returns them. Rows that ran yield off (a few Plan 4 Seeker trials) are a different setup: out.
pub fn usable(records: &[TrialRecord], chip: Option<&str>) -> Vec<Measured> {
    let mut seen = HashSet::new();
    records
        .iter()
        .filter(|t| {
            !t.partial
                && t.config.cpu_yield
                && t.chip.as_deref() == chip
        })
        .filter_map(|t| Some(Measured { setup: t.config.setup(), low_power: t.config.low_power_mode, score: t.score.clone()? })) // noisy trials count, flagged
        .filter(|x| seen.insert((x.setup, x.low_power)))
        .collect()
}

/// A clean score from means, with small CIs (the band's 2% dominates). Tests only.
#[cfg(test)]
pub(crate) fn score_of(hs: f64, sys_w: f64, cpu_w: f64) -> Score {
    Score {
        n: 180,
        hs,
        cpu_w,
        ram_w: 0.7,
        sys_w: Some(sys_w),
        temp_c: Some(50.0),
        hw_sys: Some(hs / sys_w),
        hw_cpu: Some(hs / cpu_w),
        hw_sys_ci95: Some(0.5),
        other_cpu_pct: Some(2.0),
        noisy: false,
        hs_ci95: Some(4.0),
        hw_cpu_ci95: Some(2.0),
        thermal_max: None,
    }
}

#[cfg(test)]
pub(crate) mod playground;

#[cfg(test)]
pub(crate) mod testkit {
    use super::{score_of, Measured, Setup};
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct Row {
        low_power: bool,
        e_core_clamp: bool,
        threads: u32,
        hs: f64,
        sys_w: f64,
        cpu_w: f64,
    }

    #[derive(Deserialize)]
    struct Fixture {
        setups: Vec<Row>,
    }

    /// Every setup measured on the M3 Air (tests/fixtures/m3-setups.json).
    pub fn m3_all() -> Vec<Measured> {
        serde_json::from_str::<Fixture>(include_str!("../../tests/fixtures/m3-setups.json"))
            .unwrap()
            .setups
            .into_iter()
            .map(|r| Measured {
                setup: Setup { threads: r.threads, e_core_clamp: r.e_core_clamp, utility: false },
                low_power: Some(r.low_power),
                score: score_of(r.hs, r.sys_w, r.cpu_w),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::testkit::m3_all;
    use super::*;

    fn e(threads: u32) -> Setup {
        Setup { threads, e_core_clamp: true, utility: false }
    }

    #[test]
    fn the_plan_is_every_thread_count_then_utility_then_every_e_core_count_clamped() {
        let plan = Limits { threads: 8, e_cores: 4 }.plan();
        assert_eq!(plan.len(), 17);
        assert_eq!(plan[..8], (1..=8).map(Setup::threads).collect::<Vec<_>>()[..]);
        assert_eq!(plan[8..13], (4..=8).map(|t| Setup { threads: t, e_core_clamp: false, utility: true }).collect::<Vec<_>>()[..]);
        assert_eq!(plan[13..], [e(1), e(2), e(3), e(4)]);
    }

    /// Balanced on the M3 Air: 40–60% of 8 threads is exactly 4. Under Low Power Mode 4 threads
    /// score 153.2 system H/W against 122.1 at full power: 4 threads with Low Power Mode.
    #[test]
    fn balanced_is_about_half_the_cores() {
        let p = picks(&m3_all(), Measure::Sys, 8);
        let eff = p.efficient.unwrap();
        assert_eq!((eff.setup, eff.low_power), (Setup::threads(4), Some(true)));
        assert!((eff.hw.unwrap() - 153.2).abs() < 0.1, "{:?}", eff.hw);
        let cpu = picks(&m3_all(), Measure::Cpu, 8).efficient.unwrap();
        assert_eq!(cpu.setup, Setup::threads(4), "the budget holds on either denominator");
        // A 10-thread Mac's band is 4–6; a 12-thread Mac's 5–7; a 2-thread Mac takes 1.
        let curve = |n: u32| (1..=n).map(|t| Measured { setup: Setup::threads(t), low_power: None, score: score_of(100.0 * t as f64, 10.0 + t as f64, t as f64) }).collect::<Vec<_>>();
        assert_eq!(picks(&curve(10), Measure::Sys, 10).efficient.unwrap().setup, Setup::threads(6));
        assert_eq!(picks(&curve(12), Measure::Sys, 12).efficient.unwrap().setup, Setup::threads(7));
        assert_eq!(picks(&curve(2), Measure::Sys, 2).efficient.unwrap().setup, Setup::threads(1));
    }

    /// The M1 iMac overnight (2026-10-03): 8 threads at utility scored 105.9 h/w, its best, but
    /// balanced is a core budget: 4 threads, where utility (73.1) edges default (71.6).
    #[test]
    fn utility_competes_for_balanced_only_inside_the_core_budget() {
        let row = |t: u32, u: bool, hs: f64, sys: f64| Measured { setup: Setup { threads: t, e_core_clamp: false, utility: u }, low_power: None, score: score_of(hs, sys, sys / 2.0) };
        let imac = vec![
            row(3, false, 1308.0, 21.4), row(4, false, 1734.0, 24.2), row(8, false, 2326.0, 25.6),
            row(4, true, 1699.0, 23.2), row(5, true, 1794.0, 19.3), row(8, true, 2159.0, 20.4),
        ];
        let p = picks(&imac, Measure::Sys, 8);
        assert_eq!(p.efficient.unwrap().setup, Setup { threads: 4, e_core_clamp: false, utility: true });
        assert_eq!(p.max.unwrap().setup, Setup::threads(8), "max stays every core at default QoS");
    }

    /// Intel (no E-cores): quiet's candidates are up to a quarter of the threads at background priority.
    #[test]
    fn a_mac_without_e_cores_still_sweeps_a_background_band() {
        let plan = Limits { threads: 8, e_cores: 0 }.plan();
        assert_eq!(plan.len(), 15);
        assert_eq!(plan[13..], [e(1), e(2)]);
    }

    #[test]
    fn light_is_the_best_e_only_setup_and_max_is_every_thread_at_full_power() {
        let p = picks(&m3_all(), Measure::Sys, 8);
        let light = p.light.unwrap();
        assert_eq!((light.setup, light.low_power), (e(4), Some(true)), "107 H/W sys (THESIS 18)");
        let max = p.max.unwrap();
        assert_eq!((max.setup, max.low_power), (Setup::threads(8), Some(false)));
        assert!((max.hs - 3028.6).abs() < 0.1);
        assert_eq!(picks(&m3_all(), Measure::Cpu, 8).light.unwrap().setup, e(2));
        // Swept under Low Power Mode only: max is all threads in the state measured.
        let lpm: Vec<Measured> = m3_all().into_iter().filter(|x| x.low_power == Some(true)).collect();
        assert_eq!(picks(&lpm, Measure::Sys, 8).max.unwrap().low_power, Some(true));
        assert_eq!(picks(&[], Measure::Sys, 8), Picks::default(), "no trials, no throttles");
    }

    /// The M1 iMac's 12 trials (diagnostics 2026-10-03, no Low Power Mode): quiet is 4 e-cores,
    /// balanced 4 cores (half the Mac), max 8.
    #[test]
    fn the_imac_gets_quiet_balanced_and_max_by_core_budget() {
        let rows: [(u32, bool, f64, f64, f64); 12] = [
            (1, false, 483.0, 14.2, 3.5), (2, false, 933.0, 18.0, 6.7), (3, false, 1308.0, 21.4, 9.5), (4, false, 1734.0, 24.2, 12.0),
            (5, false, 2006.0, 25.0, 12.4), (6, false, 2211.0, 25.5, 12.5), (7, false, 2288.0, 25.6, 12.3), (8, false, 2326.0, 25.6, 12.0),
            (1, true, 128.0, 9.7, 0.2), (2, true, 261.0, 9.9, 0.3), (3, true, 363.0, 10.1, 0.3), (4, true, 425.0, 10.2, 0.4),
        ];
        let imac: Vec<Measured> = rows
            .iter()
            .map(|&(t, c, hs, sys, cpu)| Measured { setup: Setup { threads: t, e_core_clamp: c, utility: false }, low_power: None, score: score_of(hs, sys, cpu) })
            .collect();
        let p = picks(&imac, Measure::Sys, 8);
        assert_eq!(p.efficient.as_ref().unwrap().setup, Setup::threads(4));
        assert_eq!(p.max.as_ref().unwrap().setup, Setup::threads(8));
        assert_eq!(p.light.unwrap().setup, e(4), "light stays the best e-core setup: 41.7 h/w against 90.8");
        // Inside a wider band, fewer threads that tie on clearly fewer watts win (a 10-thread Mac: 4–6).
        let ten: Vec<Measured> = [(4, 1700.0, 20.0), (5, 2100.0, 23.3), (6, 2300.0, 25.6)]
            .iter()
            .map(|&(t, hs, sys)| Measured { setup: Setup::threads(t), low_power: None, score: score_of(hs, sys, sys / 2.0) })
            .collect();
        assert_eq!(picks(&ten, Measure::Sys, 10).efficient.unwrap().setup, Setup::threads(5), "90.1 h/w vs 89.8, on 9% fewer watts");
    }

    #[test]
    fn usable_takes_the_newest_clean_trial_per_setup_and_state() {
        use crate::context::Context;
        use crate::ledger::TrialConfig;
        let rec = |threads: u32, hs: f64, yield_on: bool, noisy: bool| {
            let mut score = score_of(hs, 9.0, 3.0);
            score.noisy = noisy;
            TrialRecord {
                id: None, t_start: 0.0, t_end: 1.0, label: "t".into(), source: "sweep".into(), chip: Some("Apple M3".into()),
                config: TrialConfig { threads, e_core_clamp: false, low_power_mode: Some(true), cpu_yield: yield_on, utility: false },
                settled: true, settle_s: 60, partial: false, score: Some(score),
                context_start: Context::default(), context_end: Context::default(), wallet_tag: None,
            }
        };
        let mut other_chip = rec(7, 1000.0, true, false);
        other_chip.chip = Some("Apple M2".into());
        let got = usable(&[rec(5, 1800.0, true, false), rec(5, 1500.0, true, false), rec(6, 1900.0, true, true), rec(8, 1900.0, false, false), other_chip], Some("Apple M3"));
        // Noisy trials count too (flagged), so the noisy 6-thread row is usable.
        assert_eq!(got.len(), 2, "{got:?}");
        assert_eq!(got.iter().find(|m| m.setup.threads == 5).map(|m| m.score.hs), Some(1800.0), "the newest");
    }
}
