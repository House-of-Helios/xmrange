//! Other apps' CPU load, with every constant in one place.
//!
//! A trial is marked noisy (`trial::score`) when other apps' CPU sits at least `EXCESS_PCT` (one
//! full core) above its own baseline for `SUSTAIN_S` seconds in a row. The baseline is the rolling
//! `BASELINE_PCTL` percentile of the last `BASELINE_WINDOW_S` seconds, known once
//! `BASELINE_SEED_S` seconds are in. xmrig and XMRange itself are never in the sum (the engine
//! excludes their pids). This is a quality flag only: a noisy trial is still recorded, and it
//! never blocks or delays a sweep.
//!
//! Why a baseline: an idle Mac's background daemons (Spotlight, media analysis, agents) sum to
//! 130–185% on `ps`, so an old absolute 25% never let a trial score clean (Task 11 live smoke).
use std::collections::VecDeque;

/// Other apps' CPU this far above the baseline (percent of one core, as `ps` counts) is load the
/// user put there.
pub const EXCESS_PCT: f64 = 100.0;
/// ...held this many seconds in a row: the trial noisy flag's run length.
pub const SUSTAIN_S: usize = 30;
/// The baseline's window: the last 30 minutes of 1 Hz readings.
pub const BASELINE_WINDOW_S: usize = 30 * 60;
/// The baseline is known after the first 2 minutes of readings.
pub const BASELINE_SEED_S: usize = 2 * 60;
/// The baseline's percentile: low enough that a busy stretch doesn't become the new normal.
pub const BASELINE_PCTL: f64 = 0.20;

/// Other apps' CPU, one reading a second: its rolling baseline, and how far each reading sits above it.
///
/// Readings carry their time, and anything older than `BASELINE_WINDOW_S` seconds is dropped, so
/// the window is the last 30 minutes of the clock, not the last 1,800 readings (T11 re-review N1).
/// Time spent in manual, paused or asleep ages the baseline out, and a later hold seeds afresh.
/// (Clearing it on leaving hold would miss a pause or a sleep while holding.)
#[derive(Debug, Clone, Default)]
pub struct CpuBaseline {
    vals: VecDeque<(f64, f64)>,
    clock: crate::telemetry::ClockGap,
}

impl CpuBaseline {
    /// `push_at`, with ruling P4-C23: after a sleep (wall and monotonic time more than
    /// `SLEEP_GAP_S` apart since the last reading) the baseline seeds again from scratch, since
    /// what ran before the sleep says nothing about what runs after it.
    pub fn push_awake(&mut self, t: f64, mono: f64, other_cpu_pct: Option<f64>) -> Option<f64> {
        if self.clock.step(t, mono).is_some() {
            self.vals.clear();
        }
        self.push_at(t, other_cpu_pct)
    }

    /// One second's `ps` sum (None: `ps` failed) at unix time `t`. Returns the excess over the
    /// baseline, or None when the reading is missing or the baseline is still seeding. Readings
    /// older than the window are dropped first, even when this one is missing.
    pub fn push_at(&mut self, t: f64, other_cpu_pct: Option<f64>) -> Option<f64> {
        while self.vals.front().is_some_and(|&(t0, _)| t - t0 >= BASELINE_WINDOW_S as f64) {
            self.vals.pop_front();
        }
        let v = other_cpu_pct.filter(|v| v.is_finite())?;
        self.vals.push_back((t, v));
        while self.vals.len() > BASELINE_WINDOW_S {
            self.vals.pop_front();
        }
        Some(v - self.baseline()?)
    }

    /// `push_at` on a 1 Hz clock of its own: one second after the last reading. For a run that is
    /// never interrupted (a CLI trial, the tests).
    pub fn push(&mut self, other_cpu_pct: Option<f64>) -> Option<f64> {
        let t = self.vals.back().map_or(0.0, |&(t, _)| t + 1.0);
        self.push_at(t, other_cpu_pct)
    }

    /// The rolling percentile, once `BASELINE_SEED_S` readings are in.
    pub fn baseline(&self) -> Option<f64> {
        if self.vals.len() < BASELINE_SEED_S {
            return None;
        }
        let mut v: Vec<f64> = self.vals.iter().map(|&(_, v)| v).collect();
        v.sort_by(f64::total_cmp);
        let i = ((v.len() - 1) as f64 * BASELINE_PCTL).round() as usize;
        Some(v[i])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A background that wobbles ±5% around `level` (Spotlight and friends are never flat).
    fn background(level: f64, i: usize) -> f64 {
        level * (1.0 + 0.05 * (((i * 37) % 11) as f64 / 5.0 - 1.0))
    }

    /// P4-C23: after a sleep the baseline seeds again. Before the sleep a 150% background is the
    /// baseline; ten minutes asleep later (inside the 30-minute window, so aging alone would keep
    /// it) the first reading has no baseline, and the excess comes back once 2 minutes are in.
    #[test]
    fn a_sleep_reseeds_the_cpu_baseline() {
        let mut b = CpuBaseline::default();
        for i in 0..600 {
            b.push_awake(1000.0 + i as f64, i as f64, Some(background(150.0, i)));
        }
        assert!(b.baseline().is_some());
        let (wall, mono) = (1000.0 + 600.0 + 600.0, 600.0);
        assert_eq!(b.push_awake(wall, mono, Some(40.0)), None, "seeding again after the wake");
        for i in 1..BASELINE_SEED_S {
            b.push_awake(wall + i as f64, mono + i as f64, Some(40.0));
        }
        assert_eq!(b.baseline(), Some(40.0), "the post-wake load is the new baseline, not the old 150%");
        // Without a sleep (both clocks move), the same stretch keeps the baseline.
        let mut awake = CpuBaseline::default();
        for i in 0..600 {
            awake.push_awake(1000.0 + i as f64, i as f64, Some(background(150.0, i)));
        }
        assert!(awake.push_awake(1600.0, 600.0, Some(40.0)).is_some());
    }

    #[test]
    fn the_baseline_is_a_rolling_20th_percentile_over_30_minutes() {
        let mut b = CpuBaseline::default();
        for _ in 0..BASELINE_SEED_S - 1 {
            assert_eq!(b.push(Some(100.0)), None, "seeding");
        }
        assert_eq!(b.push(Some(100.0)), Some(0.0), "seeded at {BASELINE_SEED_S} s");
        assert_eq!(b.push(None), None, "a missing reading has no excess");
        for _ in BASELINE_SEED_S..10 * 60 {
            b.push(Some(100.0));
        }
        // 20 busy minutes after 10 quiet ones: a third of the window is quiet, so the 20th
        // percentile is still the quiet level and the busy load reads as excess.
        for _ in 0..20 * 60 {
            b.push(Some(400.0));
        }
        assert_eq!(b.baseline(), Some(100.0), "the quiet third still holds the 20th percentile");
        assert_eq!(b.push(Some(400.0)), Some(300.0));
        for _ in 0..BASELINE_WINDOW_S {
            b.push(Some(400.0));
        }
        assert_eq!(b.baseline(), Some(400.0), "30 minutes later the window has rolled");
    }

    /// T11 re-review N1: back to hold after an hour in manual, the baseline seeds afresh from the
    /// Mac as it is now, never from readings an hour old.
    #[test]
    fn an_hour_away_ages_the_baseline_out() {
        let mut b = CpuBaseline::default();
        let t0 = 1_790_000_000.0;
        for i in 0..BASELINE_WINDOW_S {
            b.push_at(t0 + i as f64, Some(100.0));
        }
        assert_eq!(b.baseline(), Some(100.0));
        let back = t0 + BASELINE_WINDOW_S as f64 + 3600.0;
        assert_eq!(b.push_at(back, Some(300.0)), None, "the old readings are gone: seeding again");
        for i in 1..BASELINE_SEED_S - 1 {
            assert_eq!(b.push_at(back + i as f64, Some(300.0)), None, "still seeding at {i} s");
        }
        assert_eq!(b.push_at(back + (BASELINE_SEED_S - 1) as f64, Some(300.0)), Some(0.0), "seeded from now, not from an hour ago");
        // A missing reading still ages the window, and a gap shorter than it keeps what's recent.
        let mut c = CpuBaseline::default();
        for i in 0..BASELINE_SEED_S {
            c.push_at(t0 + i as f64, Some(100.0));
        }
        assert_eq!(c.push_at(t0 + 600.0, Some(150.0)), Some(50.0), "10 minutes paused: the baseline holds");
        assert_eq!(c.push_at(t0 + 7200.0, None), None);
        assert_eq!(c.baseline(), None, "two hours on, a missing reading has still aged it out");
    }
}
