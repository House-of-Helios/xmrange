//! The profile playground (owner, 2026-10-03, after 0.2.18 shipped balanced = every core): the
//! owner's profile rules as checks, run against many Mac shapes and hundreds of varied trial
//! tables, including the pattern that broke it (utility on every core scoring the best H/W).
//!
//! The rules, as the owner stated them:
//! - **max** is every thread at default QoS, no clamp, at full power when that was measured.
//!   Nothing else may use every thread.
//! - **balanced** is a core budget: 40–60% of the threads (the count nearest half when the band is
//!   empty), never the clamp, never every thread.
//! - **quiet** is the background band: the clamp, at most the E-cores (or a quarter of the threads
//!   on a Mac with none).
//! - The three are different setups.
use super::{picks, score_of, Limits, Measured, Picks, Setup};
use crate::settings::Measure;

#[derive(Debug, Clone, Copy)]
pub(crate) struct Shape {
    pub name: &'static str,
    pub threads: u32,
    pub e_cores: u32,
    /// A laptop sweeps both Low Power Mode states; a desktop has none (unknown).
    pub laptop: bool,
}

pub(crate) const SHAPES: [Shape; 14] = [
    Shape { name: "M1 iMac (4P+4E)", threads: 8, e_cores: 4, laptop: false },
    Shape { name: "M3 Air (4P+4E)", threads: 8, e_cores: 4, laptop: true },
    Shape { name: "M1 Pro (8P+2E)", threads: 10, e_cores: 2, laptop: true },
    Shape { name: "M2 Pro mini (8P+4E)", threads: 12, e_cores: 4, laptop: false },
    Shape { name: "M3 Pro (5P+6E)", threads: 11, e_cores: 6, laptop: true },
    Shape { name: "M4 (4P+6E)", threads: 10, e_cores: 6, laptop: true },
    Shape { name: "M1 Ultra (16P+4E)", threads: 20, e_cores: 4, laptop: false },
    Shape { name: "Intel 2c/4t", threads: 4, e_cores: 0, laptop: true },
    Shape { name: "Intel 4c/8t", threads: 8, e_cores: 0, laptop: false },
    Shape { name: "Intel 6c/12t", threads: 12, e_cores: 0, laptop: true },
    Shape { name: "2 threads", threads: 2, e_cores: 0, laptop: false },
    // The owner's 2018 Mac minis (Macmini8,1): i3-8100B, i5-8500B (no Hyper-Threading), i7-8700B.
    Shape { name: "Mac mini 2018 i3 4c/4t", threads: 4, e_cores: 0, laptop: false },
    Shape { name: "Mac mini 2018 i5 6c/6t", threads: 6, e_cores: 0, laptop: false },
    Shape { name: "Mac mini 2018 i7 6c/12t", threads: 12, e_cores: 0, laptop: false },
];

/// A small deterministic generator, so every case reruns identically.
struct Lcg(u64);

impl Lcg {
    fn f(&mut self) -> f64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
    /// 1 ± `spread`.
    fn jit(&mut self, spread: f64) -> f64 {
        1.0 + (self.f() - 0.5) * 2.0 * spread
    }
}

/// One plausible trial table for `shape`: every setup the sweep plans, in every power state, with
/// per-Mac variation. Utility often wins H/W on every thread (the 0.2.18 failure), CPU watts often
/// flatten at the top (power-capped chips), and E-cores barely draw.
pub(crate) fn synth(shape: Shape, seed: u64) -> Vec<Measured> {
    let mut r = Lcg(seed.wrapping_mul(2654435761).wrapping_add(shape.threads as u64));
    let n = shape.threads;
    let p = if shape.e_cores > 0 { n - shape.e_cores } else { n };
    let p_hs = 480.0 * r.jit(0.2);
    let e_hs = 110.0 * r.jit(0.2);
    let base = if shape.laptop { 4.0 } else { 9.5 } * r.jit(0.15);
    let cap = 12.0 * r.jit(0.4);
    let sat = 0.03 * r.jit(0.5);
    let utility_hs = 0.88 * r.jit(0.08);
    let utility_w = 0.6 * r.jit(0.2);
    let states: &[Option<bool>] = if shape.laptop { &[Some(false), Some(true)] } else { &[None] };
    let mut out = Vec::new();
    for &lp in states {
        let (lp_hs, lp_w) = if lp == Some(true) { (0.62, 0.3) } else { (1.0, 1.0) };
        let open = |t: u32, r: &mut Lcg| {
            let hs: f64 = (0..t).map(|i| if i < p { p_hs } else { e_hs }).sum::<f64>() / (1.0 + sat * (t as f64 - 1.0));
            let cpu = ((0..t).map(|i| if i < p { 3.0 } else { 0.15 }).sum::<f64>()).min(cap);
            (hs * lp_hs * r.jit(0.04), cpu * lp_w * r.jit(0.04))
        };
        for s in (Limits { threads: n, e_cores: shape.e_cores }).plan() {
            let (hs, cpu) = if s.e_core_clamp {
                let per = if shape.e_cores > 0 { e_hs } else { p_hs * 0.5 };
                let w = if shape.e_cores > 0 { 0.1 } else { 1.5 };
                (per * s.threads as f64 * lp_hs * r.jit(0.04), w * s.threads as f64 * r.jit(0.1))
            } else if s.utility {
                let (hs, cpu) = open(s.threads, &mut r);
                (hs * utility_hs, cpu * utility_w)
            } else {
                open(s.threads, &mut r)
            };
            let sys = base + cpu + 0.7;
            out.push(Measured { setup: s, low_power: lp, score: score_of(hs, sys, cpu) });
        }
    }
    out
}

fn band(n: u32) -> (u32, u32) {
    (((n as f64) * 0.4).ceil() as u32, ((n as f64) * 0.6).floor() as u32)
}

/// Every owner rule, on one set of picks. Err names the broken rule.
pub(crate) fn check(shape: Shape, p: &Picks) -> Result<(), String> {
    let n = shape.threads;
    let max = p.max.as_ref().ok_or("no max")?;
    if max.setup != Setup::threads(n) {
        return Err(format!("max is not every thread at default QoS: {:?}", max.setup));
    }
    if shape.laptop && max.low_power == Some(true) {
        return Err("max ran under Low Power Mode though full power was measured".into());
    }
    let bal = p.efficient.as_ref().ok_or("no balanced")?;
    let (lo, hi) = band(n);
    let in_band = if lo.max(1) <= hi { (lo.max(1)..=hi).contains(&bal.setup.threads) } else { bal.setup.threads == (n / 2).max(1) };
    if bal.setup.e_core_clamp || !in_band || (n > 1 && bal.setup.threads >= n) {
        return Err(format!("balanced left its core budget {lo}..={hi}: {:?}", bal.setup));
    }
    let quiet = p.light.as_ref().ok_or("no quiet")?;
    let qband = if shape.e_cores > 0 { shape.e_cores } else { (n / 4).max(1) };
    if !quiet.setup.e_core_clamp || quiet.setup.threads > qband || quiet.setup.utility {
        return Err(format!("quiet left the background band (≤{qband}): {:?}", quiet.setup));
    }
    if (quiet.setup, quiet.low_power) == (bal.setup, bal.low_power) || (bal.setup, bal.low_power) == (max.setup, max.low_power) {
        return Err("two profiles share a setup".into());
    }
    Ok(())
}

#[test]
fn every_shape_and_three_hundred_tables_keep_the_owners_rules_on_both_denominators() {
    let mut failures = Vec::new();
    for shape in SHAPES {
        for seed in 0..300 {
            let table = synth(shape, seed);
            for m in [Measure::Sys, Measure::Cpu] {
                if let Err(e) = check(shape, &picks(&table, m, shape.threads)) {
                    failures.push(format!("{} seed {seed} {m:?}: {e}", shape.name));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{} broken cases, first: {:?}", failures.len(), failures.iter().take(5).collect::<Vec<_>>());
}

/// The playground must contain the failure that shipped: utility on every thread with the best
/// H/W, often, or the rules above prove nothing.
#[test]
fn the_tables_include_utility_on_every_thread_winning_hw() {
    let mut hits = 0;
    for seed in 0..300 {
        let t = synth(SHAPES[0], seed);
        let best = t.iter().filter(|x| x.score.hw_sys.is_some()).max_by(|a, b| a.score.hw_sys.unwrap().total_cmp(&b.score.hw_sys.unwrap())).unwrap();
        if best.setup == (Setup { threads: 8, e_core_clamp: false, utility: true }) {
            hits += 1;
        }
    }
    assert!(hits > 100, "only {hits} of 300 tables have utility on every thread as the best h/w");
}

/// The real tables: the M1 iMac's 17 trials (diagnostics 2026-10-03) and the M3 Air's thesis set.
#[test]
fn the_real_macs_keep_the_rules() {
    let row = |t: u32, c: bool, u: bool, hs: f64, sys: f64, cpu: f64| Measured { setup: Setup { threads: t, e_core_clamp: c, utility: u }, low_power: None, score: score_of(hs, sys, cpu) };
    let imac = vec![
        row(8, false, true, 2159.0, 20.4, 7.6), row(7, false, true, 2078.0, 20.0, 7.6), row(6, false, true, 1980.0, 19.6, 7.5),
        row(5, false, true, 1794.0, 19.3, 7.5), row(4, false, true, 1699.0, 23.2, 11.2),
        row(4, true, false, 425.0, 10.2, 0.4), row(3, true, false, 363.0, 10.1, 0.3), row(2, true, false, 261.0, 9.9, 0.3), row(1, true, false, 128.0, 9.7, 0.2),
        row(8, false, false, 2326.0, 25.6, 12.0), row(7, false, false, 2288.0, 25.6, 12.3), row(6, false, false, 2211.0, 25.5, 12.5),
        row(5, false, false, 2006.0, 25.0, 12.4), row(4, false, false, 1734.0, 24.2, 12.0), row(3, false, false, 1308.0, 21.4, 9.5),
        row(2, false, false, 933.0, 18.0, 6.7), row(1, false, false, 483.0, 14.2, 3.5),
    ];
    for m in [Measure::Sys, Measure::Cpu] {
        let p = picks(&imac, m, 8);
        check(SHAPES[0], &p).unwrap_or_else(|e| panic!("iMac {m:?}: {e}"));
        assert_eq!(p.max.unwrap().setup, Setup::threads(8));
    }
    assert_eq!(picks(&imac, Measure::Sys, 8).efficient.unwrap().setup, Setup { threads: 4, e_core_clamp: false, utility: true });
    let m3 = super::testkit::m3_all();
    for m in [Measure::Sys, Measure::Cpu] {
        check(SHAPES[1], &picks(&m3, m, 8)).unwrap_or_else(|e| panic!("M3 {m:?}: {e}"));
    }
}
