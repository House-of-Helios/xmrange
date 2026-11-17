use crate::context::Context;
use crate::telemetry::{PowerSample, PowerSource};
use crate::units::{c_to_f, hw};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};

/// xmrig's hs10 is a 10-second moving average, so consecutive 1 Hz ticks are not independent
/// samples; batch means (see `ci95_batch_means`) de-correlate before computing the 95% CI.
const CI_BLOCK_TICKS: usize = 10;

/// A trial is noisy when other apps' CPU sat this far above its own baseline for
/// `busy::SUSTAIN_S` seconds in a row (ruling P4-C18, which replaced an absolute 25% that an idle
/// Mac's daemons always exceeded). A quality flag only: a noisy trial is still recorded.
pub const NOISE_PCT: f64 = crate::busy::EXCESS_PCT;
/// A trial whose last 60 ticks carry no hashrate ends unscored: xmrig isn't hashing, and waiting
/// longer measures nothing.
const NO_HASH_S: usize = 60;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tick {
    pub power: PowerSample,
    pub hs10: Option<f64>,
    /// Other apps' CPU as `ps` sums it (percent of one core; xmrig and XMRange left out).
    pub other_cpu_pct: Option<f64>,
    /// How far `other_cpu_pct` sits above its rolling baseline (`busy::CpuBaseline`); None while
    /// the baseline seeds or with no reading.
    #[serde(default)]
    pub other_cpu_excess: Option<f64>,
}

/// A sweep trial is a fixed standard (owner, 2026-09-29): a fixed warm-up that is never scored,
/// then a fixed measure window. No early stops, so every trial runs exactly `TRIAL_S` and the
/// countdown is exact. 4 min warm-up covers the longest settle seen on the M3 (3:39, full power).
pub const WARMUP_S: u64 = 240;
pub const MEASURE_S: u64 = 240;
pub const TRIAL_S: u64 = WARMUP_S + MEASURE_S;

#[derive(Debug, Clone, Copy)]
pub struct SettleRule {
    pub window: usize,
    pub max_temp_slope_f_per_min: f64,
    pub max_hs_cv: f64,
    pub min_s: u64,
    pub max_s: u64,
}

impl Default for SettleRule {
    fn default() -> Self {
        Self { window: 60, max_temp_slope_f_per_min: 1.0, max_hs_cv: 0.02, min_s: 60, max_s: 360 }
    }
}

impl SettleRule {
    /// Roadmap carried item: a configurable rule whose minimum is shorter than its window could
    /// call a trial settled on fewer ticks than the window it judges.
    pub fn validate(&self) -> Result<()> {
        if self.window < 2 {
            bail!("the settle window must be at least 2 ticks");
        }
        if self.min_s < self.window as u64 {
            bail!("settle min_s ({}) must be at least the window ({})", self.min_s, self.window);
        }
        if self.max_s < self.min_s {
            bail!("settle max_s ({}) must be at least min_s ({})", self.max_s, self.min_s);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Score {
    pub n: usize,
    pub hs: f64,
    pub cpu_w: f64,
    pub ram_w: f64,
    pub sys_w: Option<f64>,
    pub temp_c: Option<f64>,
    pub hw_sys: Option<f64>,
    pub hw_cpu: Option<f64>,
    pub hw_sys_ci95: Option<f64>,
    pub other_cpu_pct: Option<f64>,
    pub noisy: bool,
    /// 95% batch-means CI on the mean hashrate (H/s): what "most hashes" ranks by.
    #[serde(default)]
    pub hs_ci95: Option<f64>,
    /// 95% batch-means CI on H/W against CPU watts: what "cpu only" ranks by.
    #[serde(default)]
    pub hw_cpu_ci95: Option<f64>,
    /// The worst macOS thermal pressure level seen while measuring (0 nominal … 4 sleeping).
    #[serde(default)]
    pub thermal_max: Option<u8>,
}

fn mean(v: &[f64]) -> Option<f64> {
    (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
}

fn sd(v: &[f64]) -> Option<f64> {
    let m = mean(v)?;
    (v.len() >= 2).then(|| (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (v.len() - 1) as f64).sqrt())
}

/// Least-squares slope of CPU temperature in °F per minute.
pub fn temp_slope_f_per_min(ticks: &[Tick]) -> Option<f64> {
    let pts: Vec<(f64, f64)> = ticks.iter().filter_map(|k| Some((k.power.t, c_to_f(k.power.temp_c?)))).collect();
    if pts.len() < 2 {
        return None;
    }
    let mx = pts.iter().map(|p| p.0).sum::<f64>() / pts.len() as f64;
    let my = pts.iter().map(|p| p.1).sum::<f64>() / pts.len() as f64;
    let sxx: f64 = pts.iter().map(|p| (p.0 - mx).powi(2)).sum();
    let sxy: f64 = pts.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
    (sxx > 0.0).then(|| sxy / sxx * 60.0)
}

/// Coefficient of variation of the 10 s hashrate.
pub fn hs_cv(ticks: &[Tick]) -> Option<f64> {
    let v: Vec<f64> = ticks.iter().filter_map(|k| k.hs10).collect();
    let m = mean(&v)?;
    (m > 0.0).then(|| sd(&v).map(|s| s / m)).flatten()
}

pub fn is_settled(ticks: &[Tick], rule: &SettleRule) -> bool {
    if ticks.len() < rule.window {
        return false;
    }
    let w = &ticks[ticks.len() - rule.window..];
    matches!((temp_slope_f_per_min(w), hs_cv(w)),
        (Some(s), Some(cv)) if s.abs() <= rule.max_temp_slope_f_per_min && cv <= rule.max_hs_cv)
}

/// Two-tailed 95% Student-t critical value at `df` degrees of freedom. Exact standard-table
/// values for df 1..=30; above that a Cornish-Fisher-style approximation
/// (`z + z(z²+1)/(4·df)`, z = 1.96), continuity-corrected so it lands on the table's df = 30
/// anchor (2.042) at the df = 30/31 boundary instead of jumping to the bare approximation. The
/// correction is scaled by `30/df`, so it fades to 0 and the curve still converges to z as
/// df → ∞ (ruling P4-C1: a trial measures only a handful of batch-mean blocks, and the fixed
/// z = 1.96 is anti-conservative there — a CI stop built on it would chase noise instead of
/// settling). `None` only at `df == 0`, which no caller reaches: `ci95_batch_means` always
/// passes `n_blocks - 1` after checking `n_blocks >= 2`.
fn t_quantile_975(df: usize) -> Option<f64> {
    const TABLE: [f64; 30] = [
        12.706, 4.303, 3.182, 2.776, 2.571, 2.447, 2.365, 2.306, 2.262, 2.228, 2.201, 2.179, 2.160, 2.145, 2.131,
        2.120, 2.110, 2.101, 2.093, 2.086, 2.080, 2.074, 2.069, 2.064, 2.060, 2.056, 2.052, 2.048, 2.045, 2.042,
    ];
    if df == 0 {
        return None;
    }
    if let Some(&v) = TABLE.get(df - 1) {
        return Some(v);
    }
    const Z: f64 = 1.96;
    const TABLE_DF30: f64 = 2.042;
    // f(30), the bare approximation at the table's last anchor.
    const F30: f64 = Z + Z * (Z * Z + 1.0) / (4.0 * 30.0);
    const CORRECTION: f64 = TABLE_DF30 - F30;
    let d = df as f64;
    Some(Z + Z * (Z * Z + 1.0) / (4.0 * d) + CORRECTION * 30.0 / d)
}

/// 95% CI on a per-tick series via batch means: split the values (in order) into consecutive
/// `CI_BLOCK_TICKS`-tick blocks (dropping the incomplete tail), mean each block, then take
/// `t · sd(block means) / sqrt(n_blocks)`, where `t` is the Student-t 97.5th-percentile quantile
/// at `n_blocks - 1` degrees of freedom (P4-C1). `None` below 2 blocks. Plain
/// `1.96 * sd(values) / sqrt(n)` would treat autocorrelated 1 Hz ticks as independent samples
/// and come out 2.6–3.7× too narrow; a fixed z = 1.96 on top of that would also be
/// anti-conservative at the handful of blocks a trial actually measures.
pub(crate) fn ci95_batch_means(values: &[f64]) -> Option<f64> {
    let block_means: Vec<f64> = values
        .chunks(CI_BLOCK_TICKS)
        .filter(|c| c.len() == CI_BLOCK_TICKS)
        .map(|c| mean(c).expect("non-empty block"))
        .collect();
    (block_means.len() >= 2)
        .then(|| {
            sd(&block_means).map(|s| {
                let t = t_quantile_975(block_means.len() - 1).expect("block_means.len() >= 2, so df >= 1");
                t * s / (block_means.len() as f64).sqrt()
            })
        })
        .flatten()
}

/// `noise_threshold_pct` is an excess over the baseline (`Tick::other_cpu_excess`), not an absolute.
/// Everything but `other_cpu_pct` is computed over the same tick set: ticks where `hs10` is
/// present. That way a mid-trial xmrig crash can't pull idle-power ticks into the denominator
/// and inflate H/W (see `ensure_alive`, which is the other half of this fix).
pub fn score(ticks: &[Tick], noise_threshold_pct: f64) -> Option<Score> {
    let hashing: Vec<&Tick> = ticks.iter().filter(|k| k.hs10.is_some()).collect();
    if hashing.is_empty() {
        return None;
    }
    let hs_vals: Vec<f64> = hashing.iter().filter_map(|k| k.hs10).collect();
    let hs = mean(&hs_vals)?;
    let cpu_w = mean(&hashing.iter().map(|k| k.power.cpu_w).collect::<Vec<_>>())?;
    let ram_w = mean(&hashing.iter().map(|k| k.power.ram_w).collect::<Vec<_>>())?;
    let sys_w = mean(&hashing.iter().filter_map(|k| k.power.sys_w).collect::<Vec<_>>());
    let temp_c = mean(&hashing.iter().filter_map(|k| k.power.temp_c).collect::<Vec<_>>());
    let ratios: Vec<f64> = hashing.iter().filter_map(|k| hw(k.hs10, k.power.sys_w)).collect();
    let hw_sys_ci95 = ci95_batch_means(&ratios);
    let cpu_ratios: Vec<f64> = hashing.iter().filter_map(|k| hw(k.hs10, Some(k.power.cpu_w))).collect();
    let thermal_max = hashing.iter().filter_map(|k| k.power.thermal).map(|p| p.level()).max();
    let other_cpu_pct = mean(&ticks.iter().filter_map(|k| k.other_cpu_pct).collect::<Vec<_>>());
    // P4-C18: noisy when the excess held at the threshold for `SUSTAIN_S` seconds in a row. An
    // unknown excess breaks the run: it flags nothing rather than guess.
    let mut run = 0usize;
    let noisy = ticks.iter().any(|k| {
        run = if k.other_cpu_excess.is_some_and(|e| e >= noise_threshold_pct) { run + 1 } else { 0 };
        run >= crate::busy::SUSTAIN_S
    });
    Some(Score {
        n: ticks.len(),
        hs,
        cpu_w,
        ram_w,
        sys_w,
        temp_c,
        hw_sys: hw(Some(hs), sys_w),
        hw_cpu: hw(Some(hs), Some(cpu_w)),
        hw_sys_ci95,
        other_cpu_pct,
        noisy,
        hs_ci95: ci95_batch_means(&hs_vals),
        hw_cpu_ci95: ci95_batch_means(&cpu_ratios),
        thermal_max,
    })
}

/// Below this many watts added, a marginal H/W is undefined rather than huge (or infinite).
pub const MIN_MARGINAL_W: f64 = 0.05;

/// Hashes gained per whole-Mac watt added between two trials (THESIS 15's marginal rule).
pub fn marginal_hw_sys(prev: &Score, cur: &Score) -> Option<f64> {
    let dw = cur.sys_w? - prev.sys_w?;
    (dw > MIN_MARGINAL_W).then(|| (cur.hs - prev.hs) / dw)
}

/// Anything a trial can drive: the real Miner, or a fake in tests.
pub trait Rig {
    fn threads(&self) -> u32;
    fn set_threads(&mut self, n: u32) -> Result<()>;
    fn hs10(&self) -> Option<f64>;
    fn other_cpu_pct(&self) -> Option<f64>;
    /// Errors when the underlying xmrig process has already exited. Called once per tick,
    /// before sampling, so a mid-trial crash aborts the trial instead of silently coasting on
    /// stale/idle power samples.
    fn ensure_alive(&mut self) -> Result<()>;
    /// Denominator context (Low Power Mode, power source, brightness) at this instant.
    fn context(&self) -> Context;
}

/// How long a trial measures (spec §3): at least `min_s`. With `ci_frac`, it stops as soon as
/// every batch-means CI is within that fraction of its mean, and at `max_s` at the latest.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeasureRule {
    pub min_s: u64,
    pub max_s: u64,
    pub ci_frac: Option<f64>,
}

impl MeasureRule {
    pub fn fixed(s: u64) -> Self {
        Self { min_s: s, max_s: s, ci_frac: None }
    }

    /// A sweep trial's rule: exactly `MEASURE_S`, no early stop.
    pub fn seeker() -> Self {
        Self::fixed(MEASURE_S)
    }

    pub fn validate(&self) -> Result<()> {
        if self.min_s == 0 || self.max_s < self.min_s {
            bail!("measure needs 1 ≤ min_s ≤ max_s (got {} and {})", self.min_s, self.max_s);
        }
        if matches!(self.ci_frac, Some(f) if !(f.is_finite() && f > 0.0)) {
            bail!("ci_frac must be a positive fraction");
        }
        Ok(())
    }
}

/// Every CI the score has is within `frac` of its mean. A figure with no CI yet (under two
/// blocks) is not within anything.
fn ci_within(s: &Score, frac: f64) -> bool {
    let ok = |v: Option<f64>, ci: Option<f64>| match (v, ci) {
        (None, _) => true,
        (Some(v), Some(c)) => c <= frac * v.abs(),
        (Some(_), None) => false,
    };
    ok(Some(s.hs), s.hs_ci95) && ok(s.hw_sys, s.hw_sys_ci95) && ok(s.hw_cpu, s.hw_cpu_ci95)
}

fn no_hash(ticks: &[Tick]) -> bool {
    ticks.len() >= NO_HASH_S && ticks[ticks.len() - NO_HASH_S..].iter().all(|k| k.hs10.is_none())
}

#[derive(Debug, Clone)]
pub struct TrialSpec {
    pub threads: u32,
    pub measure: MeasureRule,
    pub settle: SettleRule,
    pub noise_threshold_pct: f64,
}

impl TrialSpec {
    pub fn new(threads: u32, measure_s: u64) -> Self {
        Self { threads, measure: MeasureRule::fixed(measure_s), settle: SettleRule::default(), noise_threshold_pct: NOISE_PCT }
    }

    /// One sweep trial: exactly `WARMUP_S` of warm-up (its steadiness is still recorded as
    /// `settled`), then exactly `MEASURE_S` measured.
    pub fn seeker(threads: u32) -> Self {
        let settle = SettleRule { min_s: WARMUP_S, max_s: WARMUP_S, ..SettleRule::default() };
        Self { measure: MeasureRule::seeker(), settle, ..Self::new(threads, MEASURE_S) }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TrialStage {
    Settling,
    Measuring,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TrialStep {
    /// Keep feeding ticks.
    Running(TrialStage),
    /// Measuring is over (enough ticks, tight CIs, or no hashing): call `finish`.
    Complete,
    /// Low Power Mode flipped mid-trial. The trial measured two machines, so it is dropped,
    /// never scored (Review Focus 5).
    LowPowerChanged { from: bool, to: bool },
}

/// One trial, one tick at a time: settle → measure → score. The engine feeds it from its 1 Hz
/// loop; `run_trial` feeds it from a blocking loop for the CLI.
pub struct TrialRun {
    spec: TrialSpec,
    context_start: Context,
    low_power: Option<bool>,
    settle: Vec<Tick>,
    ticks: Vec<Tick>,
    settled: bool,
    stage: TrialStage,
}

impl TrialRun {
    pub fn new(spec: TrialSpec, context_start: Context) -> Result<TrialRun> {
        spec.settle.validate()?;
        spec.measure.validate()?;
        let low_power = context_start.low_power_mode;
        Ok(TrialRun { spec, context_start, low_power, settle: Vec::new(), ticks: Vec::new(), settled: false, stage: TrialStage::Settling })
    }

    pub fn stage(&self) -> TrialStage {
        self.stage
    }

    /// Ticks so far, settle and measure together (one per second).
    pub fn elapsed_s(&self) -> u64 {
        (self.settle.len() + self.ticks.len()) as u64
    }

    /// Seconds left by the trial's own rules, never an average: while settling, the rest of the
    /// minimum settle plus the full measure cap; while measuring, the rest of the measure cap. A
    /// tight score can end it sooner; it never runs past this.
    pub fn left_s(&self) -> u64 {
        let m = self.spec.measure.max_s;
        match self.stage {
            TrialStage::Settling => self.spec.settle.min_s.saturating_sub(self.settle.len() as u64) + m,
            TrialStage::Measuring => m.saturating_sub(self.ticks.len() as u64),
        }
    }

    /// One 1 Hz tick. `low_power` is Low Power Mode at this tick; `None` (unknown) is never a flip.
    pub fn push(&mut self, tick: Tick, low_power: Option<bool>) -> TrialStep {
        match (self.low_power, low_power) {
            (Some(from), Some(to)) if from != to => return TrialStep::LowPowerChanged { from, to },
            (None, Some(_)) => self.low_power = low_power,
            _ => {}
        }
        match self.stage {
            TrialStage::Settling => {
                self.settle.push(tick);
                if no_hash(&self.settle) {
                    return TrialStep::Complete;
                }
                let n = self.settle.len() as u64;
                if n >= self.spec.settle.min_s && is_settled(&self.settle, &self.spec.settle) {
                    self.settled = true;
                    self.stage = TrialStage::Measuring;
                } else if n >= self.spec.settle.max_s {
                    self.stage = TrialStage::Measuring;
                }
                TrialStep::Running(self.stage)
            }
            TrialStage::Measuring => {
                self.ticks.push(tick);
                let n = self.ticks.len() as u64;
                let m = self.spec.measure;
                let done = n >= m.max_s
                    || no_hash(&self.ticks)
                    || (n >= m.min_s
                        && match m.ci_frac {
                            None => true,
                            Some(f) => score(&self.ticks, self.spec.noise_threshold_pct).is_some_and(|s| ci_within(&s, f)),
                        });
                if done { TrialStep::Complete } else { TrialStep::Running(TrialStage::Measuring) }
            }
        }
    }

    pub fn finish(self, context_end: Context) -> TrialResult {
        let partial = (self.ticks.len() as u64) < self.spec.measure.min_s;
        TrialResult {
            threads: self.spec.threads,
            settled: self.settled,
            settle_s: self.settle.len() as u64,
            score: score(&self.ticks, self.spec.noise_threshold_pct),
            ticks: self.ticks,
            context_start: self.context_start,
            context_end,
            partial,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TrialResult {
    pub threads: u32,
    pub settled: bool,
    pub settle_s: u64,
    pub score: Option<Score>,
    pub ticks: Vec<Tick>,
    pub context_start: Context,
    pub context_end: Context,
    /// True when fewer ticks were measured than `measure_s` asked for (stopped mid-measure).
    pub partial: bool,
}

fn take_tick(rig: &dyn Rig, source: &mut dyn PowerSource, base: &mut crate::busy::CpuBaseline) -> Result<Tick> {
    let power = source.sample(1000)?;
    let other = rig.other_cpu_pct();
    // A CLI trial seeds its own baseline: its first `BASELINE_SEED_S` seconds can't be noisy.
    Ok(Tick { power, hs10: rig.hs10(), other_cpu_pct: other, other_cpu_excess: base.push(other) })
}

/// Apply threads → settle (fix the instrument: slope + CV rule) → measure → score.
pub fn run_trial(
    rig: &mut dyn Rig,
    source: &mut dyn PowerSource,
    spec: &TrialSpec,
    stop: &AtomicBool,
    on_tick: &mut dyn FnMut(&Tick),
) -> Result<TrialResult> {
    if rig.threads() != spec.threads {
        rig.set_threads(spec.threads)?;
    }
    let mut run = TrialRun::new(spec.clone(), rig.context())?;
    let mut base = crate::busy::CpuBaseline::default();
    while !stop.load(Ordering::Relaxed) {
        rig.ensure_alive()?;
        let t = take_tick(rig, source, &mut base)?;
        on_tick(&t);
        if run.push(t, None) == TrialStep::Complete {
            break;
        }
    }
    Ok(run.finish(rig.context()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::ThermalPressure;
    use crate::telemetry::{PowerSample, PowerSource};

    fn tick(t: f64, temp_c: f64, hs: f64, sys: Option<f64>, other: f64) -> Tick {
        Tick {
            power: PowerSample { t, cpu_w: 3.8, ram_w: 0.8, sys_w: sys, temp_c: Some(temp_c), ..Default::default() },
            hs10: Some(hs),
            other_cpu_pct: Some(other),
            other_cpu_excess: Some(other),
        }
    }

    #[test]
    fn flat_and_steady_is_settled() {
        let v: Vec<Tick> = (0..60).map(|i| tick(i as f64, 55.0, 1826.0 + (i % 3) as f64, Some(9.1), 1.0)).collect();
        assert!(is_settled(&v, &SettleRule::default()));
    }

    #[test]
    fn heating_is_not_settled() {
        // +3 °F per minute = +1/60 °C per second · 9/5 … use 3 °F/min → (3/1.8)/60 °C per s
        let v: Vec<Tick> = (0..60).map(|i| tick(i as f64, 50.0 + i as f64 * (3.0 / 1.8) / 60.0, 1826.0, Some(9.1), 1.0)).collect();
        assert!(temp_slope_f_per_min(&v).unwrap() > 2.9);
        assert!(!is_settled(&v, &SettleRule::default()));
    }

    #[test]
    fn wobbling_hashrate_is_not_settled() {
        let v: Vec<Tick> = (0..60).map(|i| tick(i as f64, 55.0, if i % 2 == 0 { 1700.0 } else { 1950.0 }, Some(9.1), 1.0)).collect();
        assert!(hs_cv(&v).unwrap() > 0.02);
        assert!(!is_settled(&v, &SettleRule::default()));
    }

    #[test]
    fn too_few_ticks_is_not_settled() {
        let v: Vec<Tick> = (0..10).map(|i| tick(i as f64, 55.0, 1826.0, Some(9.1), 1.0)).collect();
        assert!(!is_settled(&v, &SettleRule::default()));
    }

    #[test]
    fn busy_machine_marks_trial_noisy() {
        // P4-C18: the excess over the baseline, held `SUSTAIN_S` seconds in a row.
        let v: Vec<Tick> = (0..60).map(|i| tick(i as f64, 55.0, 1826.0, Some(9.1), if i >= 20 && i < 50 { 150.0 } else { 3.0 })).collect();
        assert!(score(&v, NOISE_PCT).unwrap().noisy, "30 s at +150%");
        let spike: Vec<Tick> = (0..60).map(|i| tick(i as f64, 55.0, 1826.0, Some(9.1), if i >= 20 && i < 49 { 150.0 } else { 3.0 })).collect();
        assert!(!score(&spike, NOISE_PCT).unwrap().noisy, "29 s is a spike");
        let mut unknown = v.clone();
        unknown.iter_mut().for_each(|k| k.other_cpu_excess = None);
        assert!(!score(&unknown, NOISE_PCT).unwrap().noisy, "no baseline yet: nothing to flag");
    }

    #[test]
    fn score_without_sys_power_has_no_hw_sys() {
        let v: Vec<Tick> = (0..30).map(|i| tick(i as f64, 55.0, 1826.0, None, 1.0)).collect();
        let s = score(&v, NOISE_PCT).unwrap();
        assert_eq!(s.sys_w, None);
        assert_eq!(s.hw_sys, None);
        assert!(s.hw_cpu.unwrap().is_finite());
    }

    #[test]
    fn marginal_uses_whole_mac_deltas() {
        let a = score(&(0..10).map(|i| tick(i as f64, 55.0, 1363.0, Some(8.90), 1.0)).collect::<Vec<_>>(), NOISE_PCT).unwrap();
        let b = score(&(0..10).map(|i| tick(i as f64, 55.0, 1825.0, Some(9.11), 1.0)).collect::<Vec<_>>(), NOISE_PCT).unwrap();
        assert!((marginal_hw_sys(&a, &b).unwrap() - 2200.0).abs() < 50.0, "THESIS 14: the free 5th thread");
        let c = score(&(0..10).map(|i| tick(i as f64, 55.0, 1830.0, Some(9.11), 1.0)).collect::<Vec<_>>(), NOISE_PCT).unwrap();
        assert_eq!(marginal_hw_sys(&b, &c), None, "ΔW ≈ 0 → undefined, not inf");
    }

    struct FakeRig { threads: u32, sets: Vec<u32>, other: f64, die_after: Option<u32>, alive_calls: u32 }
    impl Rig for FakeRig {
        fn threads(&self) -> u32 { self.threads }
        fn set_threads(&mut self, n: u32) -> anyhow::Result<()> { self.threads = n; self.sets.push(n); Ok(()) }
        fn hs10(&self) -> Option<f64> { Some(1826.0) }
        fn other_cpu_pct(&self) -> Option<f64> { Some(self.other) }
        fn ensure_alive(&mut self) -> anyhow::Result<()> {
            self.alive_calls += 1;
            if self.die_after.is_some_and(|n| self.alive_calls > n) {
                anyhow::bail!("xmrig exited mid-trial (fake death)");
            }
            Ok(())
        }
        fn context(&self) -> Context { Context::default() }
    }
    struct FakePower { t: f64 }
    impl PowerSource for FakePower {
        fn sample(&mut self, _ms: u32) -> anyhow::Result<PowerSample> {
            self.t += 1.0;
            Ok(PowerSample { t: self.t, cpu_w: 3.8, ram_w: 0.8, sys_w: Some(9.1), temp_c: Some(55.0), ..Default::default() })
        }
    }

    #[test]
    fn run_trial_settles_then_measures() {
        let mut rig = FakeRig { threads: 8, sets: vec![], other: 2.0, die_after: None, alive_calls: 0 };
        let mut src = FakePower { t: 0.0 };
        let stop = std::sync::atomic::AtomicBool::new(false);
        let mut seen = 0;
        let r = run_trial(&mut rig, &mut src, &TrialSpec::new(5, 30), &stop, &mut |_| seen += 1).unwrap();
        assert_eq!(rig.sets, vec![5], "threads applied once");
        assert!(r.settled);
        assert_eq!(r.settle_s, 60, "min settle");
        assert_eq!(r.ticks.len(), 30);
        assert_eq!(seen, 90);
        assert!(!r.partial);
        assert!((r.score.unwrap().hw_sys.unwrap() - 200.66).abs() < 0.1);
    }

    #[test]
    fn run_trial_honours_stop() {
        let mut rig = FakeRig { threads: 5, sets: vec![], other: 2.0, die_after: None, alive_calls: 0 };
        let mut src = FakePower { t: 0.0 };
        let stop = std::sync::atomic::AtomicBool::new(true);
        let r = run_trial(&mut rig, &mut src, &TrialSpec::new(5, 30), &stop, &mut |_| {}).unwrap();
        assert!(r.ticks.is_empty() && r.score.is_none());
        assert!(r.partial, "0 measured ticks < measure_s");
    }

    #[test]
    fn run_trial_errors_when_rig_dies() {
        let mut rig = FakeRig { threads: 5, sets: vec![], other: 2.0, die_after: Some(3), alive_calls: 0 };
        let mut src = FakePower { t: 0.0 };
        let stop = std::sync::atomic::AtomicBool::new(false);
        let e = run_trial(&mut rig, &mut src, &TrialSpec::new(5, 30), &stop, &mut |_| {}).unwrap_err().to_string();
        assert!(e.contains("exited"), "{e}");
    }

    #[test]
    fn score_ignores_idle_ticks_after_hashing_stops() {
        let mut v: Vec<Tick> = (0..20).map(|i| tick(i as f64, 55.0, 1826.0, Some(9.1), 1.0)).collect();
        v.extend((20..30).map(|i| Tick {
            power: PowerSample { t: i as f64, cpu_w: 3.8, ram_w: 0.8, sys_w: Some(4.5), temp_c: Some(55.0), ..Default::default() },
            hs10: None,
            other_cpu_pct: Some(1.0),
            other_cpu_excess: Some(1.0),
        }));
        let s = score(&v, NOISE_PCT).unwrap();
        assert!((s.sys_w.unwrap() - 9.1).abs() < 1e-9, "idle-watt ticks after hashing stops must not lower sys_w: {:?}", s.sys_w);
        assert!((s.hw_sys.unwrap() - 1826.0 / 9.1).abs() < 0.5, "{:?}", s.hw_sys);
    }

    #[test]
    fn batch_means_ci_is_wider_than_naive_for_autocorrelated_ticks() {
        // xmrig's hs10 is a 10 s moving average: simulate that by repeating each of 10
        // independent-ish samples 10× (100 autocorrelated 1 Hz ticks).
        let base = [1780.0, 1810.0, 1840.0, 1800.0, 1860.0, 1790.0, 1820.0, 1850.0, 1795.0, 1830.0];
        let v: Vec<Tick> = base
            .iter()
            .flat_map(|&hs| std::iter::repeat(hs).take(10))
            .enumerate()
            .map(|(i, hs)| tick(i as f64, 55.0, hs, Some(9.1), 1.0))
            .collect();
        let ratios: Vec<f64> = v.iter().filter_map(|k| hw(k.hs10, k.power.sys_w)).collect();
        let naive = 1.96 * sd(&ratios).unwrap() / (ratios.len() as f64).sqrt();
        let batch = ci95_batch_means(&ratios).unwrap();
        assert!(batch > naive, "batch-means CI {batch} should be wider than the naive CI {naive} for autocorrelated ticks");
    }

    #[test]
    fn score_carries_three_cis_and_the_worst_thermal_level() {
        // Four 10-tick blocks at 1800, 1820, 1840, 1860 H/s: every batch-means CI is above zero.
        let mut v: Vec<Tick> = (0..40).map(|i| tick(i as f64, 55.0, 1800.0 + (i / 10) as f64 * 20.0, Some(9.0), 1.0)).collect();
        v[25].power.thermal = Some(ThermalPressure::Moderate);
        v[30].power.thermal = Some(ThermalPressure::Nominal);
        let s = score(&v, NOISE_PCT).unwrap();
        assert!(s.hs_ci95.unwrap() > 0.0 && s.hw_cpu_ci95.unwrap() > 0.0 && s.hw_sys_ci95.unwrap() > 0.0, "{s:?}");
        assert_eq!(s.thermal_max, Some(1), "the worst level seen while measuring");

        let flat: Vec<Tick> = (0..40).map(|i| tick(i as f64, 55.0, 1800.0, Some(9.0), 1.0)).collect();
        let f = score(&flat, NOISE_PCT).unwrap();
        assert_eq!((f.hs_ci95, f.thermal_max), (Some(0.0), None), "no thermal reading is None, never 0");
    }

    #[test]
    fn a_score_saved_before_plan_4_still_loads() {
        let old = r#"{"n":180,"hs":1826.0,"cpu_w":3.8,"ram_w":0.8,"sys_w":9.1,"temp_c":55.0,"hw_sys":200.6,"hw_cpu":480.5,"hw_sys_ci95":0.5,"other_cpu_pct":2.0,"noisy":false}"#;
        let s: Score = serde_json::from_str(old).unwrap();
        assert_eq!((s.hs_ci95, s.hw_cpu_ci95, s.thermal_max), (None, None, None));
    }

    /// P4-C1: the batch-means CI uses a Student-t quantile at n_blocks - 1 degrees of freedom,
    /// not the fixed z = 1.96, because a trial only ever measures a handful of blocks.
    #[test]
    fn the_ci_quantile_is_student_t_not_a_fixed_z() {
        let t = |df| t_quantile_975(df).unwrap();
        assert!((t(9) - 2.262).abs() < 0.001, "10 blocks: {}", t(9));
        assert!((t(100_000) - 1.96).abs() < 0.001, "as df -> infinity: {}", t(100_000));
        assert!(t(9) > t(29) && t(29) > t(100_000), "narrows with more blocks");
    }

    /// Review fix round 1: the table (df <= 30) and the Cornish-Fisher tail (df > 30) must meet
    /// without a jump or an overshoot at the boundary.
    #[test]
    fn the_ci_quantile_is_continuous_and_monotone_past_the_table() {
        let t = |df| t_quantile_975(df).unwrap();
        assert!((t(31) - 2.0395).abs() < 0.002, "t(31) = {}", t(31));
        assert!(t(30) - t(31) < 0.003, "boundary step t(30)={} t(31)={}", t(30), t(31));
        for df in 1..200 {
            assert!(t(df) > t(df + 1), "not strictly decreasing at df={}: t(df)={} t(df+1)={}", df, t(df), t(df + 1));
        }
    }

    #[test]
    fn the_ci_quantile_is_undefined_at_zero_degrees_of_freedom() {
        assert_eq!(t_quantile_975(0), None);
    }

    fn steady(i: usize) -> Tick {
        tick(i as f64, 55.0, 1826.0, Some(9.1), 1.0)
    }

    #[test]
    fn a_settle_rule_shorter_than_its_window_is_refused() {
        let bad = SettleRule { min_s: 30, ..SettleRule::default() };
        let e = bad.validate().unwrap_err().to_string();
        assert!(e.contains("at least the window"), "{e}");
        SettleRule::default().validate().unwrap();
        assert!(TrialRun::new(TrialSpec { settle: bad, ..TrialSpec::seeker(5) }, Context::default()).is_err());
        assert!(MeasureRule { min_s: 0, max_s: 10, ci_frac: None }.validate().is_err());
    }

    #[test]
    fn a_sweep_trial_is_exactly_its_fixed_warm_up_then_its_fixed_measure() {
        let mut run = TrialRun::new(TrialSpec::seeker(5), Context::default()).unwrap();
        let mut i = 0;
        while run.push(steady(i), Some(true)) != TrialStep::Complete {
            i += 1;
            assert!(i < 2000, "never completed");
        }
        let r = run.finish(Context::default());
        assert_eq!((r.settle_s, r.ticks.len() as u64), (WARMUP_S, MEASURE_S), "no early stop either side");
        assert!(r.settled && !r.partial);
    }

    #[test]
    fn low_power_flip_mid_trial_drops_the_trial() {
        let ctx = Context { low_power_mode: Some(true), ..Default::default() };
        let mut run = TrialRun::new(TrialSpec::seeker(8), ctx.clone()).unwrap();
        for i in 0..100 {
            assert!(matches!(run.push(steady(i), Some(true)), TrialStep::Running(_)));
        }
        assert_eq!(run.push(steady(100), Some(false)), TrialStep::LowPowerChanged { from: true, to: false });
        let mut run = TrialRun::new(TrialSpec::seeker(8), ctx).unwrap();
        assert!(matches!(run.push(steady(0), None), TrialStep::Running(_)), "an unknown reading is never a flip");
    }

    #[test]
    fn a_miner_that_stops_hashing_ends_the_trial_unscored_in_a_minute() {
        let mut run = TrialRun::new(TrialSpec::seeker(5), Context::default()).unwrap();
        let mut n = 0;
        loop {
            let mut t = steady(n);
            t.hs10 = None;
            n += 1;
            if run.push(t, None) == TrialStep::Complete {
                break;
            }
        }
        assert_eq!(n, 60);
        let r = run.finish(Context::default());
        assert!(r.partial && r.score.is_none());
    }

    #[test]
    fn left_s_counts_down_by_the_rules_never_below_zero() {
        let r = TrialRun::new(TrialSpec::seeker(4), Context::default()).unwrap();
        assert_eq!(r.left_s(), TRIAL_S, "a fresh sweep trial has its whole fixed length left");
    }

}
