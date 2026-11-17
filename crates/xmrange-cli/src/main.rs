use anyhow::Result;
use clap::{Parser, Subcommand};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use xmrange_core::config::{MinerConfig, API_PORT};
use xmrange_core::context::read_context;
use xmrange_core::ledger::{LedgerWriter, TrialRecord};
use xmrange_core::miner::{max_threads, validate_threads, LaunchOptions, Miner};
use xmrange_core::telemetry::{AppleSiliconSource, PowerSource};
use xmrange_core::trial::{marginal_hw_sys, run_trial, Score, Tick, TrialResult, TrialSpec};
use xmrange_core::units::{c_to_f, hw};

#[derive(Parser)]
#[command(name = "xmrange", version, about = "Hashes per watt, measured on this Mac.")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Machine, power context and one power sample.
    Info,
    /// Store the wallet and worker name (dev builds: private file in _devdata).
    #[cfg(feature = "dev-diagnostics")]
    Wallet {
        #[command(subcommand)]
        cmd: WalletCmd,
    },
    /// Ledger counts: schema, trials, size, cipher, integrity. Never prints row contents.
    #[cfg(feature = "dev-diagnostics")]
    Ledger {
        /// Delete every trial with this exact label first (e.g. a stray `curve` run).
        #[arg(long)]
        drop_label: Option<String>,
        /// Also print the three profile picks the app would offer (setup + h/w only).
        #[arg(long)]
        picks: bool,
    },
    /// Mine and print a live line every 5 s. Ctrl-C stops.
    Mine {
        #[arg(long, default_value_t = 5)]
        threads: u32,
        #[arg(long)]
        minutes: Option<u64>,
        #[arg(long)]
        e_core: bool,
    },
    /// One measured trial: settle, measure, score.
    Trial {
        #[arg(long)]
        threads: u32,
        #[arg(long, default_value_t = 180)]
        measure: u64,
        #[arg(long, default_value = "trial")]
        label: String,
        #[arg(long)]
        e_core: bool,
        /// Experiment: xmrig `cpu.yield` off (workers never step aside for other apps).
        #[arg(long)]
        no_yield: bool,
        /// Mining threads at utility QoS (xmrig `cpu.qos`, our patch): the shipping lever.
        #[arg(long)]
        utility: bool,
        /// Experiment: launch xmrig under `taskpolicy -c utility` instead (the process-wide clamp).
        #[arg(long)]
        utility_launch: bool,
    },
    /// Per-thread cost curve in one xmrig process (dataset kept between steps).
    Curve {
        #[arg(long, value_delimiter = ',', default_value = "1,2,3,4,5,6,7,8")]
        steps: Vec<u32>,
        #[arg(long, default_value_t = 180)]
        measure: u64,
        #[arg(long, default_value = "curve")]
        label: String,
        #[arg(long)]
        e_core: bool,
    },
}


#[cfg(feature = "dev-diagnostics")]
#[derive(Subcommand)]
enum WalletCmd {
    Set {
        address: String,
        /// Only written to settings when given; otherwise the worker already chosen in the app wins.
        #[arg(long)]
        worker: Option<String>,
    },
    Show,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let stop = Arc::new(AtomicBool::new(false));
    let s2 = stop.clone();
    ctrlc::set_handler(move || s2.store(true, Ordering::SeqCst))?;
    match cli.cmd {
        Cmd::Info => info(),
        #[cfg(feature = "dev-diagnostics")]
        Cmd::Wallet { cmd } => wallet(cmd),
        #[cfg(feature = "dev-diagnostics")]
        Cmd::Ledger { drop_label, picks } => ledger_stats(drop_label.as_deref(), picks),
        Cmd::Mine { threads, minutes, e_core } => mine(threads, minutes, e_core, &stop),
        Cmd::Trial { threads, measure, label, e_core, no_yield, utility, utility_launch } => {
            if measure == 0 {
                anyhow::bail!("--measure must be at least 1");
            }
            let mut src = AppleSiliconSource::new()?;
            let chip = src.machine().chip; // F17: the chip from the source already open
            let (mut miner, wallet) = launch(threads, e_core, &label, !no_yield, utility, utility_launch)?;
            let r = run_trial(&mut miner, &mut src, &TrialSpec::new(threads, measure), &stop, &mut progress)?;
            miner.stop()?;
            println!();
            let results = [r];
            report(&results, &label)?;
            record(&results, &label, e_core, utility, &wallet, Some(&chip));
            Ok(())
        }
        Cmd::Curve { steps, measure, label, e_core } => {
            if measure == 0 {
                anyhow::bail!("--measure must be at least 1");
            }
            let first = *steps.first().ok_or_else(|| anyhow::anyhow!("--steps is empty"))?;
            let max = max_threads();
            for &n in &steps {
                validate_threads(n, max)?;
            }
            let mut src = AppleSiliconSource::new()?;
            let chip = src.machine().chip; // F17: the chip from the source already open
            let (mut miner, wallet) = launch(first, e_core, &label, true, false, false)?;
            let mut results = Vec::new();
            for n in steps {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                eprintln!("▶ {n} threads");
                match run_trial(&mut miner, &mut src, &TrialSpec::new(n, measure), &stop, &mut progress) {
                    Ok(r) => results.push(r),
                    Err(e) => {
                        eprintln!();
                        let _ = miner.stop();
                        report(&results, &label)?;
                        record(&results, &label, e_core, false, &wallet, Some(&chip));
                        return Err(e);
                    }
                }
                eprintln!();
            }
            miner.stop()?;
            report(&results, &label)?;
            record(&results, &label, e_core, false, &wallet, Some(&chip));
            Ok(())
        }
    }
}

fn info() -> Result<()> {
    let mut src = AppleSiliconSource::new()?;
    let m = src.machine();
    let s = src.sample(1000)?;
    println!("{} · {} · {}P + {}E · {} GB", m.chip, m.model, m.p_cores, m.e_cores, m.memory_gb);
    println!("{}", serde_json::to_string_pretty(&read_context())?);
    println!(
        "cpu {:.2} W · ram {:.2} W · whole mac {} · cpu temp {}",
        s.cpu_w,
        s.ram_w,
        s.sys_w.map_or("unknown".into(), |w| format!("{w:.2} W")),
        s.temp_c.map_or("unknown".into(), |c| format!("{:.0}°F", c_to_f(c)))
    );
    Ok(())
}

#[cfg(feature = "dev-diagnostics")]
fn wallet(cmd: WalletCmd) -> Result<()> {
    let data = xmrange_core::paths::data_dir();
    xmrange_core::settings::migrate_old_secrets(&data)?;
    match cmd {
        WalletCmd::Set { address, worker } => {
            xmrange_core::wallet::check_address(&address)?;
            if let Some(w) = &worker {
                xmrange_core::config::validate_worker(w)?;
            }
            let mut settings = xmrange_core::settings::Settings::load(&data);
            settings.wallet = xmrange_core::settings::Wallet(Some(address.clone()));
            if let Some(w) = &worker {
                settings.worker = w.clone();
            }
            settings.save(&data)?;
            match worker {
                Some(w) => println!("saved · {} · worker {w}", mask(&address)),
                None => println!("saved · {}", mask(&address)),
            }
        }
        WalletCmd::Show => {
            let settings = xmrange_core::settings::Settings::load(&data);
            match settings.wallet.0 {
                Some(a) => println!("{} · worker {}", mask(&a), settings.worker),
                None => println!("no wallet set · mining uses the default (GhostDrip dev wallet)"),
            }
        }
    }
    Ok(())
}

#[cfg(feature = "dev-diagnostics")]
fn mask(a: &str) -> String {
    let head: String = a.chars().take(4).collect();
    let n = a.chars().count();
    let tail: String = a.chars().skip(n.saturating_sub(4)).collect();
    format!("{head}…{tail}")
}

/// R8: never bail when no wallet is set — mine to the disclosed GhostDrip dev wallet instead.
#[cfg(feature = "dev-diagnostics")]
fn credentials() -> Result<(String, String)> {
    let data = xmrange_core::paths::data_dir();
    xmrange_core::settings::migrate_old_secrets(&data)?;
    let settings = xmrange_core::settings::Settings::load(&data);
    match settings.wallet.0 {
        Some(w) => Ok((w, settings.worker)),
        None => {
            eprintln!(
                "mining to the default wallet (GhostDrip dev wallet) — set yours with `cargo dev wallet set <ADDRESS>`"
            );
            Ok((xmrange_core::config::DEFAULT_WALLET.to_string(), settings.worker))
        }
    }
}

/// R8: never bail when no wallet is set — mine to the disclosed GhostDrip dev wallet instead.
#[cfg(not(feature = "dev-diagnostics"))]
fn credentials() -> Result<(String, String)> {
    eprintln!(
        "mining to the default wallet (GhostDrip dev wallet) — the command line does not use the wallet saved in the XMRange app"
    );
    Ok((xmrange_core::config::DEFAULT_WALLET.to_string(), "xmrange".to_string()))
}

/// Starts xmrig; also returns the wallet it mines to, for the trial's salted wallet tag.
fn launch(threads: u32, e_core: bool, label: &str, cpu_yield: bool, utility: bool, qos_utility: bool) -> Result<(Miner, String)> {
    let (wallet, worker) = credentials()?;
    #[cfg(feature = "dev-diagnostics")]
    let dev_log = {
        let dir = xmrange_core::paths::data_dir().join("logs/xmrig");
        std::fs::create_dir_all(&dir)?;
        Some(dir.join(format!("{}-{label}.log", xmrange_core::telemetry::now_secs() as u64)))
    };
    #[cfg(not(feature = "dev-diagnostics"))]
    let dev_log = {
        let _ = label;
        None
    };
    let config = MinerConfig { cpu_yield, utility, ..MinerConfig::new(&wallet, &worker, threads)? };
    let opts = LaunchOptions { config, api_port: API_PORT, e_core_clamp: e_core, qos_utility, dev_log };
    eprintln!(
        "starting xmrig ({threads} threads{}{}{})…",
        if e_core { ", efficiency cores only" } else { "" },
        if cpu_yield { "" } else { ", yield off" },
        if utility { ", utility qos (threads)" } else if qos_utility { ", utility qos (taskpolicy)" } else { "" }
    );
    Ok((Miner::launch(&xmrange_core::paths::xmrig_binary(), opts)?, wallet))
}

fn progress(t: &Tick) {
    eprint!(
        "\r  {:>6.0} h/s · {} · {}      ",
        t.hs10.unwrap_or(0.0),
        t.power.sys_w.map_or("? W".into(), |w| format!("{w:.1} W")),
        t.power.temp_c.map_or("?°F".into(), |c| format!("{:.0}°F", c_to_f(c)))
    );
}

fn mine(threads: u32, minutes: Option<u64>, e_core: bool, stop: &AtomicBool) -> Result<()> {
    let mut src = AppleSiliconSource::new()?;
    let (miner, _) = launch(threads, e_core, "mine", true, false, false)?;
    let limit = minutes.map(|m| m * 60);
    let mut n = 0u64;
    while !stop.load(Ordering::SeqCst) && limit.map_or(true, |l| n < l) {
        let p = src.sample(1000)?;
        n += 1;
        if n % 5 == 0 {
            let s = miner.summary().ok();
            let hs = s.as_ref().and_then(|s| s.hs10());
            println!(
                "{:>6.0} h/s · {} · {} H/W sys · {} · shares {}",
                hs.unwrap_or(0.0),
                p.sys_w.map_or("? W".into(), |w| format!("{w:.1} W")),
                hw(hs, p.sys_w).map_or("?".into(), |v| format!("{v:.0}")),
                p.temp_c.map_or("?°F".into(), |c| format!("{:.0}°F", c_to_f(c))),
                s.map_or(0, |s| s.results.shares_good)
            );
        }
    }
    miner.stop()
}

fn report(results: &[TrialResult], label: &str) -> Result<()> {
    let mut prev: Option<&Score> = None;
    println!("threads    h/s   whole W  H/W sys ±ci   H/W cpu   °F   marginal H/W sys  flags");
    for r in results {
        let Some(s) = &r.score else {
            println!("{:>7}  (stopped before scoring){}", r.threads, if r.partial { " PARTIAL" } else { "" });
            continue;
        };
        let marginal = prev.and_then(|p| marginal_hw_sys(p, s)).map_or("—".into(), |m| format!("{m:.0}"));
        let mut flags = String::new();
        if s.noisy {
            flags.push_str("NOISY ");
        }
        if !r.settled {
            flags.push_str("UNSETTLED ");
        }
        if r.partial {
            flags.push_str("PARTIAL ");
        }
        println!(
            "{:>7} {:>6.0} {:>9} {:>8} ±{:<4} {:>8} {:>5} {:>9}  {}",
            r.threads,
            s.hs,
            s.sys_w.map_or("?".into(), |w| format!("{w:.2}")),
            s.hw_sys.map_or("?".into(), |v| format!("{v:.0}")),
            s.hw_sys_ci95.map_or("?".into(), |v| format!("{v:.0}")),
            s.hw_cpu.map_or("?".into(), |v| format!("{v:.0}")),
            s.temp_c.map(c_to_f).map_or("?".into(), |v| format!("{v:.0}")),
            marginal,
            flags.trim_end()
        );
        prev = Some(s);
    }
    #[cfg(feature = "dev-diagnostics")]
    {
        use std::io::Write;
        let dir = xmrange_core::paths::workspace_root()
            .join("runs")
            .join(format!("{}-{label}", xmrange_core::telemetry::now_secs() as u64));
        std::fs::create_dir_all(&dir)?;
        let mut f = std::fs::File::create(dir.join("ticks.jsonl"))?;
        for r in results {
            for t in &r.ticks {
                writeln!(f, "{}", serde_json::json!({"threads": r.threads, "tick": t}))?;
            }
        }
        let summary: Vec<_> = results
            .iter()
            .map(|r| {
                serde_json::json!({
                    "threads": r.threads,
                    "settled": r.settled,
                    "settle_s": r.settle_s,
                    "partial": r.partial,
                    "score": r.score,
                    "context_start": r.context_start,
                    "context_end": r.context_end,
                })
            })
            .collect();
        std::fs::write(dir.join("summary.json"), serde_json::to_string_pretty(&summary)?)?;
        eprintln!("saved {}", dir.display());
    }
    #[cfg(not(feature = "dev-diagnostics"))]
    let _ = label;
    Ok(())
}

fn trial_records(results: &[TrialResult], label: &str, chip: Option<&str>, e_core: bool, utility: bool) -> Vec<TrialRecord> {
    results.iter().map(|r| TrialRecord::from_result(r, label, "cli", chip, e_core).with_utility(utility)).collect()
}

/// Every trial goes into the Ledger (spec §3: "every trial is stored, so a second run on the
/// same Mac starts warm"). A Ledger problem is reported, never fatal: the table above (and the
/// dev runs/ dump) already hold the numbers. `chip` comes from the trial's own power source (F17).
/// A trial whose write wasn't confirmed in time may still land, so it is reported as unconfirmed
/// and never retried, which could store it twice (T7 review m3).
fn record(results: &[TrialResult], label: &str, e_core: bool, utility: bool, wallet: &str, chip: Option<&str>) {
    if results.is_empty() {
        return;
    }
    let opened = xmrange_core::ledger::open_default();
    if let Some(n) = &opened.note {
        eprintln!("{n}");
    }
    let Some(ledger) = opened.ledger else { return };
    let writer = LedgerWriter::spawn(ledger);
    let handle = writer.handle();
    let (mut saved, mut unconfirmed) = (0, 0);
    for rec in trial_records(results, label, chip, e_core, utility) {
        match handle.trial(rec, Some(wallet.to_string())) {
            Ok(_) => saved += 1,
            Err(e) if e.unconfirmed() => unconfirmed += 1,
            Err(e) => eprintln!("ledger: {e}"),
        }
    }
    if !writer.close() {
        eprintln!("ledger: still writing when the close wait ran out");
    }
    if unconfirmed > 0 {
        eprintln!("ledger: {saved} trial(s) recorded, {unconfirmed} not confirmed (they may still have been written)");
    } else {
        eprintln!("ledger: {saved} trial(s) recorded");
    }
}

/// Counts only, never row contents, plus SQLite's and SQLCipher's integrity checks. Opens (and
/// on first run makes) the dev Ledger and `dev.key`, but never moves a Ledger aside for a stats
/// command: a Ledger with no `dev.key` is left alone (T8 review m7).
#[cfg(feature = "dev-diagnostics")]
fn ledger_stats(drop_label: Option<&str>, show_picks: bool) -> Result<()> {
    use xmrange_core::paths::{key_path, ledger_path};
    if ledger_path().exists() && !key_path().exists() {
        anyhow::bail!(
            "{} has no dev.key beside it; left untouched (the app moves it aside on its next launch)",
            ledger_path().file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
        );
    }
    let opened = xmrange_core::ledger::open_default();
    if let Some(n) = &opened.note {
        eprintln!("{n}");
    }
    let Some(l) = opened.ledger else { anyhow::bail!("no Ledger this session") };
    if let Some(label) = drop_label {
        println!("dropped {} trial(s) labelled {label:?}", l.delete_label(label)?);
    }
    println!(
        "ledger v{} · trials {} · {:.2} MB · {} on {} · integrity {}",
        l.schema_version()?,
        l.trial_count()?,
        l.file_bytes()? as f64 / 1e6,
        l.cipher_version()?,
        l.cipher_provider()?,
        l.integrity()?
    );
    if show_picks {
        use xmrange_core::{optimizer::{picks, usable}, settings::Measure};
        let recs = l.trials(xmrange_core::engine::HISTORY_TRIALS)?;
        let chip = recs.first().and_then(|t| t.chip.clone());
        let measured = usable(&recs, chip.as_deref());
        let p = picks(&measured, Measure::Sys, xmrange_core::miner::max_threads());
        println!("usable {} of {} · chip {:?}", measured.len(), recs.len(), chip);
        for (name, x) in [("light", p.light), ("efficient", p.efficient), ("max", p.max)] {
            match x {
                Some(x) => println!("{name}: {} threads{} · lpm {:?} · {:.1} h/w", x.setup.threads, if x.setup.e_core_clamp { " (e-cores)" } else { "" }, x.low_power, x.hw.unwrap_or(0.0)),
                None => println!("{name}: none"),
            }
        }
    }
    Ok(())
}

#[cfg(all(test, feature = "dev-diagnostics"))]
mod tests {
    use super::mask;

    #[test]
    fn every_trial_becomes_a_cli_record() {
        use xmrange_core::trial::TrialResult;
        let r = |threads| TrialResult { threads, settled: true, settle_s: 60, partial: false, score: None, ticks: vec![], context_start: Default::default(), context_end: Default::default() };
        let recs = super::trial_records(&[r(4), r(8)], "curve", Some("Apple M3"), true, false);
        assert_eq!(recs.iter().map(|x| x.config.threads).collect::<Vec<_>>(), [4, 8]);
        assert!(recs.iter().all(|x| x.source == "cli" && x.label == "curve" && x.config.e_core_clamp && x.chip.as_deref() == Some("Apple M3")));
    }

    #[test]
    fn masks_a_synthetic_wallet_address() {
        let w = format!("4Q{}", "A".repeat(93));
        assert_eq!(mask(&w), "4QAA…AAAA");
    }

    #[test]
    fn masks_empty_string_to_bare_ellipsis() {
        assert_eq!(mask(""), "…");
    }

    #[test]
    fn masks_non_ascii_without_panicking() {
        // char-based slicing must never split a multi-byte codepoint
        let _ = mask("ü漢字ab");
    }
}
