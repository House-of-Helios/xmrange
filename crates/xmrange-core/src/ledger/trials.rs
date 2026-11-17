//! Trials (spec §3–4). Roadmap item "settle Score as the Ledger trials row": the row IS
//! `trial::Score`, column for column (°C kept), plus the trial's config, settle facts and
//! context. The wallet appears only as a salted tag (doctrine §3).
use super::{sql, Ledger, LedgerError};
use crate::context::Context;
use crate::telemetry::now_secs;
use crate::trial::{Score, TrialResult};
use rusqlite::Row;
use serde::{Deserialize, Serialize};
use sha3::{Digest, Sha3_256};

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrialConfig {
    pub threads: u32,
    pub e_core_clamp: bool,
    pub low_power_mode: Option<bool>,
    /// xmrig's `cpu.yield`. Always on now (THESIS 5: a no-op on Apple Silicon); a few Plan 4
    /// Seeker rows ran it off, and they stay marked. Rows before Plan 4 have none: yield on.
    #[serde(default = "yes")]
    pub cpu_yield: bool,
    /// Mining threads at utility QoS (2026-10-03). Older rows have none: off.
    #[serde(default)]
    pub utility: bool,
}

impl TrialConfig {
    pub fn setup(&self) -> crate::levers::Setup {
        crate::levers::Setup { threads: self.threads, e_core_clamp: self.e_core_clamp, utility: self.utility }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrialRecord {
    pub id: Option<i64>,
    pub t_start: f64,
    pub t_end: f64,
    pub label: String,
    pub source: String,
    pub chip: Option<String>,
    pub config: TrialConfig,
    pub settled: bool,
    pub settle_s: u64,
    pub partial: bool,
    pub score: Option<Score>,
    pub context_start: Context,
    pub context_end: Context,
    pub wallet_tag: Option<String>,
}

impl TrialRecord {
    /// `t_start`/`t_end` are the first and last measured ticks (the settle, `settle_s` long,
    /// comes before). A trial stopped before measuring gets now for both.
    pub fn from_result(r: &TrialResult, label: &str, source: &str, chip: Option<&str>, e_core_clamp: bool) -> TrialRecord {
        let t_start = r.ticks.first().map(|k| k.power.t).unwrap_or_else(now_secs);
        let t_end = r.ticks.last().map(|k| k.power.t).unwrap_or(t_start);
        TrialRecord {
            id: None,
            t_start,
            t_end,
            label: label.to_string(),
            source: source.to_string(),
            chip: chip.map(String::from),
            config: TrialConfig { threads: r.threads, e_core_clamp, low_power_mode: r.context_start.low_power_mode, cpu_yield: true, utility: false },
            settled: r.settled,
            settle_s: r.settle_s,
            partial: r.partial,
            score: r.score.clone(),
            context_start: r.context_start.clone(),
            context_end: r.context_end.clone(),
            wallet_tag: None,
        }
    }
}

impl TrialRecord {
    /// The same record, marked as run at utility QoS (or not).
    pub fn with_utility(mut self, on: bool) -> TrialRecord {
        self.config.utility = on;
        self
    }
}

/// One row of the tuning tab's trials card. How many rows show is a UI concern (the JS
/// `TRIALS_SHOWN`), so this view returns every distinct setup.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrialView {
    pub label: String,
    pub hw_sys: Option<f64>,
    pub hw_cpu: Option<f64>,
    pub noisy: bool,
    pub t_end: f64,
    /// The exact setup this row measured, so clicking it can apply it (never re-derived from the
    /// label text).
    pub threads: u32,
    pub e_core_clamp: bool,
    pub utility: bool,
    pub low_power: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct TrialsView {
    pub count: u64,
    pub rows: Vec<TrialView>,
    pub curve: CurveView,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CurvePoint {
    pub threads: u32,
    pub hs: f64,
    pub sys_w: Option<f64>,
    pub hw_sys: Option<f64>,
    pub hw_cpu: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct CurveView {
    pub low_power: Option<bool>,
    pub points: Vec<CurvePoint>,
    /// The step with the most H/s per whole-Mac watt added: (threads, +H/s, +W).
    pub standout: Option<(u32, f64, f64)>,
    /// From this count on, no further thread adds 2% more H/s (THESIS 15's words).
    pub flat_from: Option<u32>,
}

/// The thread curve card (spec §3: the Seeker "walks the per-thread curve"). It shows the newest
/// clean trial per thread count, with the clamp off and yield on, in one Low Power Mode state:
/// the given one, else the newest trial's. `records` come newest first, as `Ledger::trials` returns them.
pub fn curve_view(records: &[TrialRecord], low_power: Option<bool>) -> CurveView {
    let lp = low_power.or_else(|| records.first().and_then(|r| r.config.low_power_mode));
    let mut by = std::collections::BTreeMap::new();
    for r in records {
        let c = &r.config;
        if r.partial || c.e_core_clamp || !c.cpu_yield || c.low_power_mode != lp {
            continue;
        }
        let Some(s) = r.score.as_ref() else { continue }; // noisy trials count, flagged
        by.entry(c.threads).or_insert(CurvePoint { threads: c.threads, hs: s.hs, sys_w: s.sys_w, hw_sys: s.hw_sys, hw_cpu: s.hw_cpu });
    }
    let points: Vec<CurvePoint> = by.into_values().collect();
    let standout = points
        .windows(2)
        .filter(|w| w[1].threads == w[0].threads + 1)
        .filter_map(|w| {
            let dw = w[1].sys_w? - w[0].sys_w?;
            (dw > 0.05).then(|| (w[1].threads, w[1].hs - w[0].hs, dw))
        })
        .max_by(|a, b| (a.1 / a.2).total_cmp(&(b.1 / b.2)));
    let flat_from = points
        .iter()
        .enumerate()
        .find(|&(i, p)| i + 1 < points.len() && points[i + 1..].iter().all(|q| q.hs < p.hs * 1.02))
        .map(|(_, p)| p.threads);
    CurveView { low_power: lp, points, standout, flat_from }
}

/// The mockup's wording: "8 threads · low power mode", "4 threads · e-cores · full power".
pub fn trial_label(c: &TrialConfig) -> String {
    let mut s = c.setup().label();
    if !c.cpu_yield {
        s.push_str(" · no yield");
    }
    match c.low_power_mode {
        Some(true) => s.push_str(" · low power mode"),
        Some(false) => s.push_str(" · full power"),
        None => {}
    }
    s
}

/// `records` newest first (as `Ledger::trials` returns them). Keeps scored, complete trials,
/// the newest one per setup, so the card lists real configurations, each once.
pub fn trial_views(count: u64, records: &[TrialRecord]) -> TrialsView {
    let mut seen = std::collections::HashSet::new();
    let rows = records
        .iter()
        .filter(|r| !r.partial)
        .filter_map(|r| r.score.as_ref().map(|s| (r, s, trial_label(&r.config))))
        .filter(|(_, _, label)| seen.insert(label.clone()))
        .map(|(r, s, label)| TrialView {
            label,
            hw_sys: s.hw_sys,
            hw_cpu: s.hw_cpu,
            noisy: s.noisy,
            t_end: r.t_end,
            threads: r.config.threads,
            e_core_clamp: r.config.e_core_clamp,
            utility: r.config.utility,
            low_power: r.config.low_power_mode,
        })
        .collect();
    TrialsView { count, rows, curve: CurveView::default() }
}

fn json<T: Serialize>(v: &T) -> Result<String, LedgerError> {
    serde_json::to_string(v).map_err(|e| LedgerError::Sql(e.to_string()))
}

/// Closures can't be generic, so this is a free function. `i` only labels which column failed
/// in the conversion error; it is informational and needn't be that column's exact position.
fn parse<T: for<'de> Deserialize<'de>>(i: usize, s: String) -> rusqlite::Result<T> {
    serde_json::from_str(&s).map_err(|e| rusqlite::Error::FromSqlConversionFailure(i, rusqlite::types::Type::Text, Box::new(e)))
}

fn from_row(r: &Row) -> rusqlite::Result<TrialRecord> {
    let hs: Option<f64> = r.get("hs")?;
    let score = match hs {
        None => None,
        Some(hs) => Some(Score {
            n: r.get::<_, i64>("n")? as usize,
            hs,
            cpu_w: r.get("cpu_w")?,
            ram_w: r.get("ram_w")?,
            sys_w: r.get("sys_w")?,
            temp_c: r.get("temp_c")?,
            hw_sys: r.get("hw_sys")?,
            hw_cpu: r.get("hw_cpu")?,
            hw_sys_ci95: r.get("hw_sys_ci95")?,
            other_cpu_pct: r.get("other_cpu_pct")?,
            noisy: r.get::<_, i64>("noisy")? != 0,
            hs_ci95: r.get("hs_ci95")?,
            hw_cpu_ci95: r.get("hw_cpu_ci95")?,
            thermal_max: r.get::<_, Option<i64>>("thermal_max")?.map(|v| v as u8),
        }),
    };
    Ok(TrialRecord {
        id: Some(r.get("id")?),
        t_start: r.get("t_start")?,
        t_end: r.get("t_end")?,
        label: r.get("label")?,
        source: r.get("source")?,
        chip: r.get("chip")?,
        config: parse(6, r.get("config")?)?,
        settled: r.get::<_, i64>("settled")? != 0,
        settle_s: r.get::<_, i64>("settle_s")? as u64,
        partial: r.get::<_, i64>("partial")? != 0,
        score,
        context_start: parse(22, r.get("context_start")?)?,
        context_end: parse(23, r.get("context_end")?)?,
        wallet_tag: r.get("wallet_tag")?,
    })
}

impl Ledger {
    pub fn insert_trial(&mut self, rec: &TrialRecord) -> Result<i64, LedgerError> {
        let s = rec.score.as_ref();
        self.conn
            .execute(
                "INSERT INTO trials (t_start, t_end, label, source, chip, config, settled, settle_s, partial,
                    n, hs, cpu_w, ram_w, sys_w, temp_c, hw_sys, hw_cpu, hw_sys_ci95, other_cpu_pct, noisy,
                    context_start, context_end, wallet_tag, hs_ci95, hw_cpu_ci95, thermal_max)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26)",
                rusqlite::params![
                    rec.t_start, rec.t_end, rec.label, rec.source, rec.chip, json(&rec.config)?,
                    rec.settled as i64, rec.settle_s as i64, rec.partial as i64,
                    s.map(|s| s.n as i64), s.map(|s| s.hs), s.map(|s| s.cpu_w), s.map(|s| s.ram_w),
                    s.and_then(|s| s.sys_w), s.and_then(|s| s.temp_c), s.and_then(|s| s.hw_sys), s.and_then(|s| s.hw_cpu),
                    s.and_then(|s| s.hw_sys_ci95), s.and_then(|s| s.other_cpu_pct), s.map(|s| s.noisy as i64),
                    json(&rec.context_start)?, json(&rec.context_end)?, rec.wallet_tag,
                    s.and_then(|s| s.hs_ci95), s.and_then(|s| s.hw_cpu_ci95), s.and_then(|s| s.thermal_max.map(i64::from)),
                ],
            )
            .map_err(sql)?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn trials(&self, limit: u32) -> Result<Vec<TrialRecord>, LedgerError> {
        let mut st = self.conn.prepare_cached("SELECT * FROM trials ORDER BY t_end DESC, id DESC LIMIT ?1").map_err(sql)?;
        let rows = st.query_map([limit], from_row).map_err(sql)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(|e| LedgerError::Corrupt(e.to_string()))
    }

    /// Deletes every trial and gives the space back.
    pub fn clear_trials(&self) -> Result<(), LedgerError> {
        self.conn.execute_batch("DELETE FROM trials;").map_err(sql)?;
        let _ = self.conn.execute_batch("VACUUM;");
        Ok(())
    }

    /// Deletes every trial with this exact label (a stray CLI run, say); returns how many.
    pub fn delete_label(&self, label: &str) -> Result<u64, LedgerError> {
        self.conn.execute("DELETE FROM trials WHERE label = ?1", [label]).map(|n| n as u64).map_err(sql)
    }

    pub fn trial_count(&self) -> Result<u64, LedgerError> {
        self.conn.query_row("SELECT count(*) FROM trials", [], |r| r.get::<_, i64>(0)).map(|n| n as u64).map_err(sql)
    }

    /// Final review M7: trials that are complete and scored, the only ones `trial_views` ever
    /// lists. `trial_count` above still counts every row (partial ones included) for callers that
    /// want that, e.g. the CLI's stats; the trials card's big number must agree with its own list,
    /// so it asks this one instead.
    pub fn scored_trial_count(&self) -> Result<u64, LedgerError> {
        self.conn
            .query_row("SELECT count(*) FROM trials WHERE partial = 0 AND hs IS NOT NULL", [], |r| r.get::<_, i64>(0))
            .map(|n| n as u64)
            .map_err(sql)
    }

    /// 16 hex chars of SHA3-256(salt ‖ wallet). The salt is derived from this Ledger's key
    /// (`Ledger::open`), so no table stores it (P3-R4). Rows can say "this wallet" without the
    /// address, and the tag means nothing outside this Ledger. Because it is keyed by the Ledger
    /// key, guessing a known wallet (e.g. the default) gives no oracle without that key.
    pub fn wallet_tag(&self, wallet: &str) -> String {
        let mut h = Sha3_256::new();
        h.update(self.tag_salt);
        h.update(wallet.as_bytes());
        h.finalize()[..8].iter().map(|b| format!("{b:02x}")).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::{assert_no_plaintext, open_test};
    use crate::telemetry::PowerSample;
    use crate::trial::Tick;

    fn result(threads: u32, t0: f64, hw: Option<f64>) -> TrialResult {
        let ticks: Vec<Tick> = (0..180)
            .map(|i| Tick {
                power: PowerSample { t: t0 + i as f64, cpu_w: 3.8, ram_w: 0.8, sys_w: Some(9.1), temp_c: Some(55.0), ..Default::default() },
                hs10: Some(1826.0),
                other_cpu_pct: Some(2.0),
                other_cpu_excess: Some(0.0),
            })
            .collect();
        TrialResult {
            threads,
            settled: true,
            settle_s: 75,
            partial: hw.is_none(),
            score: hw.map(|h| Score {
                n: 180,
                hs: 1826.0,
                cpu_w: 3.8,
                ram_w: 0.8,
                sys_w: Some(9.1),
                temp_c: Some(55.0),
                hw_sys: Some(h),
                hw_cpu: Some(480.5),
                hw_sys_ci95: Some(3.2),
                other_cpu_pct: Some(2.0),
                noisy: false,
                hs_ci95: Some(9.1),
                hw_cpu_ci95: Some(1.2),
                thermal_max: Some(1),
            }),
            ticks,
            context_start: Context { low_power_mode: Some(true), ..Default::default() },
            context_end: Context { low_power_mode: Some(true), ..Default::default() },
        }
    }

    #[test]
    fn score_is_the_row_and_comes_back_whole() {
        let (mut l, _, _) = open_test("trials");
        let rec = TrialRecord::from_result(&result(8, 1000.0, Some(220.0)), "curve", "cli", Some("Apple M3"), false);
        assert_eq!((rec.t_start, rec.t_end), (1000.0, 1179.0));
        assert_eq!(rec.config, TrialConfig { threads: 8, e_core_clamp: false, low_power_mode: Some(true), cpu_yield: true, utility: false });
        let id = l.insert_trial(&rec).unwrap();
        let back = l.trials(10).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0], TrialRecord { id: Some(id), ..rec }, "every Score field survives, °C included");
    }

    #[test]
    fn an_unscored_trial_is_kept_as_partial() {
        let (mut l, _, _) = open_test("partial");
        l.insert_trial(&TrialRecord::from_result(&result(5, 1.0, None), "trial", "cli", None, true)).unwrap();
        let back = &l.trials(1).unwrap()[0];
        assert!(back.partial && back.score.is_none() && back.config.e_core_clamp);
    }

    #[test]
    fn newest_trials_come_first_and_the_limit_holds() {
        let (mut l, _, _) = open_test("order");
        for t0 in [100.0, 300.0, 200.0] {
            l.insert_trial(&TrialRecord::from_result(&result(4, t0, Some(150.0)), "t", "cli", None, false)).unwrap();
        }
        let got: Vec<f64> = l.trials(2).unwrap().iter().map(|r| r.t_start).collect();
        assert_eq!(got, [300.0, 200.0]);
        assert_eq!(l.trial_count().unwrap(), 3);
    }

    /// Final review M7: `trial_count` (every row) and `scored_trial_count` (what the card's list
    /// actually shows) disagree on purpose once a partial trial is in the mix.
    #[test]
    fn scored_trial_count_excludes_partial_and_unscored_rows() {
        let (mut l, _, _) = open_test("scored-count");
        l.insert_trial(&TrialRecord::from_result(&result(4, 100.0, Some(150.0)), "t", "cli", None, false)).unwrap();
        l.insert_trial(&TrialRecord::from_result(&result(5, 200.0, Some(160.0)), "t", "cli", None, false)).unwrap();
        l.insert_trial(&TrialRecord::from_result(&result(6, 300.0, None), "trial", "cli", None, true)).unwrap(); // partial
        assert_eq!(l.trial_count().unwrap(), 3);
        assert_eq!(l.scored_trial_count().unwrap(), 2);
    }

    #[test]
    fn wallet_tag_is_salted_stable_and_opaque() {
        let (l, p, k) = open_test("tag");
        // Synthetic addresses only (project hard rule): never the real default wallet, even in tests.
        let a = format!("4{}", "A".repeat(94));
        let b = format!("4Q{}", "A".repeat(93));
        let ta = l.wallet_tag(&a);
        assert_eq!(ta.len(), 16);
        assert!(ta.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(ta, l.wallet_tag(&b));
        assert!(!a.contains(&ta) && !ta.contains(&a[..6]));
        drop(l);
        assert_eq!(Ledger::open(&p, &k).unwrap().wallet_tag(&a), ta, "same key, same salt, same tag");
        let (other, _, _) = open_test("tag2");
        assert_ne!(other.wallet_tag(&a), ta, "another Ledger (another key), another salt");
    }

    #[test]
    fn trial_text_is_not_readable_on_disk() {
        let (mut l, p, _) = open_test("trialtext");
        l.insert_trial(&TrialRecord::from_result(&result(8, 1.0, Some(200.0)), "CANARY-TRIAL-LABEL", "cli", Some("CANARY-CHIP"), false)).unwrap();
        let dir = p.parent().unwrap().to_path_buf();
        for f in std::fs::read_dir(&dir).unwrap().flatten() {
            assert_no_plaintext(&f.path(), &["CANARY-TRIAL-LABEL", "CANARY-CHIP", "low_power_mode"]);
        }
    }

    #[test]
    fn labels_follow_the_mockup() {
        let c = |threads, e_core_clamp, lpm| TrialConfig { threads, e_core_clamp, low_power_mode: lpm, cpu_yield: true, utility: false };
        assert_eq!(trial_label(&c(8, false, Some(true))), "8 cores · low power mode");
        assert_eq!(trial_label(&c(8, false, Some(false))), "8 cores · full power");
        assert_eq!(trial_label(&c(4, true, Some(false))), "4 e-cores · full power");
        assert_eq!(trial_label(&c(1, false, None)), "1 core");
        assert_eq!(trial_label(&TrialConfig { cpu_yield: false, ..c(6, false, Some(true)) }), "6 cores · no yield · low power mode");
    }

    #[test]
    fn a_config_saved_before_plan_4_ran_with_yield_on() {
        let c: TrialConfig = serde_json::from_str(r#"{"threads":8,"e_core_clamp":false,"low_power_mode":true}"#).unwrap();
        assert!(c.cpu_yield, "xmrig's default, and every Plan 1–3 trial");
    }

    #[test]
    fn the_card_shows_the_newest_scored_trial_per_setup() {
        let mut recs = vec![
            TrialRecord::from_result(&result(8, 300.0, Some(221.0)), "t", "cli", None, false), // newest 8 · lpm
            TrialRecord::from_result(&result(8, 100.0, Some(207.0)), "t", "cli", None, false), // older, same setup
            TrialRecord::from_result(&result(5, 200.0, Some(200.0)), "t", "cli", None, false),
            TrialRecord::from_result(&result(4, 400.0, None), "t", "cli", None, false),        // unscored
        ];
        recs.sort_by(|a, b| b.t_end.total_cmp(&a.t_end));
        let v = trial_views(4, &recs);
        assert_eq!(v.count, 4);
        let got: Vec<(&str, Option<f64>)> = v.rows.iter().map(|r| (r.label.as_str(), r.hw_sys)).collect();
        assert_eq!(got, [("8 cores · low power mode", Some(221.0)), ("5 cores · low power mode", Some(200.0))]);
    }

    fn curve_rec(threads: u32, hs: f64, sys: f64, lp: Option<bool>, t_end: f64, clamp: bool) -> TrialRecord {
        let mut r = TrialRecord::from_result(&result(threads, t_end - 179.0, Some(hs / sys)), "t", "cli", None, clamp);
        if let Some(s) = r.score.as_mut() {
            s.hs = hs;
            s.sys_w = Some(sys);
        }
        r.config.low_power_mode = lp;
        r
    }

    #[test]
    fn the_curve_is_the_newest_clean_trial_per_thread_count_in_one_power_mode() {
        // THESIS 14's curve (runs/20260923-131545-lpm-on-curve-pe), newest first as the Ledger answers.
        let pts = [(1, 278.75, 4.695), (2, 728.19, 7.057), (3, 1095.45, 8.233), (4, 1362.74, 8.896), (5, 1824.71, 9.104), (6, 1830.43, 9.011), (7, 1743.34, 8.922), (8, 1857.83, 8.964)];
        let mut recs: Vec<TrialRecord> = pts.iter().rev().enumerate().map(|(i, &(t, hs, sys))| curve_rec(t, hs, sys, Some(true), 100.0 - i as f64, false)).collect();
        recs.push(curve_rec(5, 9999.0, 9.0, Some(true), 1.0, false)); // an older 5-thread trial: the newer one wins
        recs.insert(0, curve_rec(4, 500.0, 4.7, Some(true), 200.0, true)); // clamped: not on this curve
        recs.insert(0, curve_rec(8, 3029.0, 21.1, Some(false), 300.0, false)); // the other power mode
        let c = curve_view(&recs, Some(true));
        assert_eq!(c.points.iter().map(|p| p.threads).collect::<Vec<_>>(), (1..=8).collect::<Vec<_>>());
        assert!((c.points[4].hs - 1824.71).abs() < 1e-9);
        let (n, dhs, dw) = c.standout.unwrap();
        assert_eq!(n, 5, "the 5th thread: +462 H/s for +0.21 W");
        assert!((dhs - 462.0).abs() < 1.0 && (dw - 0.21).abs() < 0.01);
        assert_eq!(c.flat_from, Some(5), "threads 6–8 add under 2% (THESIS 15)");
        assert_eq!(curve_view(&recs, None).low_power, Some(false), "state unknown: the newest trial's");
    }

    #[test]
    fn a_v1_ledger_gains_the_v2_columns_and_keeps_its_rows() {
        // A v1 Ledger, as a Plan 3 build left it.
        let p = crate::ledger::test_dir("v1-to-v2").join("l.sqlite");
        let k = crate::ledger::LedgerKey::generate();
        let l = Ledger::open_at(&p, &k, 1).unwrap();
        l.conn.execute(
            "INSERT INTO trials (t_start, t_end, label, source, config, settled, settle_s, partial, n, hs, cpu_w, ram_w, noisy, context_start, context_end)
             VALUES (1, 2, 'old', 'cli', '{\"threads\":8,\"e_core_clamp\":false,\"low_power_mode\":true}', 1, 60, 0, 180, 1858.0, 3.25, 0.87, 0, '{}', '{}')",
            [],
        ).unwrap();
        drop(l);
        let l = Ledger::open(&p, &k).unwrap();
        assert_eq!(l.schema_version().unwrap(), crate::ledger::SCHEMA_VERSION);
        let old = &l.trials(5).unwrap()[0];
        assert_eq!(old.label, "old");
        let s = old.score.as_ref().unwrap();
        assert_eq!((s.hs, s.hs_ci95, s.thermal_max), (1858.0, None, None));
    }
}
