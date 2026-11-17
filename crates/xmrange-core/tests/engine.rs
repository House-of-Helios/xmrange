#![cfg(feature = "dev-diagnostics")]
use std::sync::Mutex;
use std::time::{Duration, Instant};
use xmrange_core::engine::{spawn, Command, EngineConfig, RunState, Snapshot};
use xmrange_core::paths::scratch_dir;

/// Engine tests that mutate the shared `XMRANGE_XMRIG` / `XMRANGE_DATA_DIR` env vars
/// (process-global) must not run concurrently. Hold this for the whole body. Recover from a
/// poisoned lock (an earlier test panicked while holding it) rather than cascading the panic.
static LOCK: Mutex<()> = Mutex::new(());

#[test]
fn engine_reports_error_when_launch_fails() {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let base = scratch_dir("engine-launch-fail").unwrap();
    std::env::set_var("XMRANGE_DATA_DIR", &base);
    std::env::set_var("XMRANGE_XMRIG", "/usr/bin/false");

    let wallet = format!("4Q{}", "A".repeat(93));
    let cfg = EngineConfig { wallet, worker: "xmrange-test".into(), threads: 2, e_core_clamp: false, utility: false, dev_log: None, ledger: None, optimizer: None, pool: Default::default(), chain: Default::default(), node: Default::default(), tor: false, guard: Default::default() };
    let (ctl, rx) = spawn(cfg);

    let snap = rx.recv_timeout(Duration::from_secs(10)).expect("a snapshot within 10s");
    assert_eq!(snap.state, RunState::Error, "{:?}", snap.error);
    assert!(snap.error.as_deref().unwrap_or("").contains("exited"), "{:?}", snap.error);

    assert!(rx.recv_timeout(Duration::from_secs(2)).is_err(), "receiver should close after the error snapshot");
    ctl.stop(); // idempotent: the thread has already ended
    std::env::remove_var("XMRANGE_XMRIG");
    std::env::remove_var("XMRANGE_DATA_DIR");
}

/// LIVE: needs the patched xmrig (tools/build-xmrig.sh), network, and XMRANGE_TEST_WALLET.
/// Run: XMRANGE_TEST_WALLET=<addr> cargo test -p xmrange-core --features dev-diagnostics \
///   --test engine -- --ignored --nocapture
#[test]
#[ignore]
fn engine_live_session() {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let wallet = std::env::var("XMRANGE_TEST_WALLET").expect("set XMRANGE_TEST_WALLET");
    let base = scratch_dir("engine-live").unwrap();
    std::env::set_var("XMRANGE_DATA_DIR", &base);
    std::env::remove_var("XMRANGE_XMRIG"); // use the real, default (patched) xmrig binary

    // A scratch Ledger: the live session proves the engine runs with one and, with no optimizer,
    // writes nothing to it (the 1 Hz samples stay in memory, ruling P5-O1).
    let lp = scratch_dir("engine-live-ledger").unwrap().join("ledger.sqlite");
    let lk = xmrange_core::ledger::LedgerKey::generate();
    let writer = xmrange_core::ledger::LedgerWriter::spawn(xmrange_core::ledger::Ledger::open(&lp, &lk).unwrap());

    let cfg = EngineConfig {
        wallet: wallet.clone(),
        worker: "xmrange-test".into(),
        threads: 4,
        e_core_clamp: false,
        utility: false,
        dev_log: None,
        ledger: Some(writer.handle()),
        optimizer: None,
        pool: Default::default(), chain: Default::default(), node: Default::default(),
        tor: false,
        guard: Default::default(),
    };
    let (ctl, rx) = spawn(cfg);

    let recv = |secs: u64| -> Snapshot { rx.recv_timeout(Duration::from_secs(secs)).expect("engine snapshot") };
    let wait_until = |mut pred: Box<dyn FnMut(&Snapshot) -> bool>, budget_s: u64| -> Snapshot {
        let deadline = Instant::now() + Duration::from_secs(budget_s);
        loop {
            let snap = recv(5);
            assert_ne!(snap.state, RunState::Error, "{:?}", snap.error);
            if pred(&snap) {
                return snap;
            }
            assert!(Instant::now() < deadline, "condition not met within {budget_s}s");
        }
    };

    let mining = wait_until(Box::new(|s| s.state == RunState::Mining), 120);
    assert_eq!(mining.threads, 4);

    ctl.send(Command::SetThreads(2)).unwrap();
    let two = wait_until(Box::new(|s| s.threads == 2), 10);
    assert_eq!(two.state, RunState::Mining);

    ctl.send(Command::Pause).unwrap();
    wait_until(Box::new(|s| s.state == RunState::Paused), 10);

    ctl.send(Command::Resume).unwrap();
    wait_until(Box::new(|s| s.state == RunState::Mining), 10);

    // Identity change 1: new worker name, same (valid) wallet.
    ctl.send(Command::SetIdentity { wallet: wallet.clone(), worker: "xmrange-test2".into() }).unwrap();
    let renamed = wait_until(Box::new(|s| s.worker == "xmrange-test2"), 15);
    assert_eq!(renamed.state, RunState::Mining);

    // Identity change 2: a typo'd wallet (char 40 changed) must fail checksum validation,
    // leave the engine's config untouched (worker stays xmrange-test2), and surface an
    // `error`-tagged tape line, without crashing the engine (still Mining).
    let mut chars: Vec<char> = wallet.chars().collect();
    chars[40] = if chars[40] == 'a' { 'b' } else { 'a' };
    let typo_wallet: String = chars.into_iter().collect();
    ctl.send(Command::SetIdentity { wallet: typo_wallet, worker: "xmrange-test2".into() }).unwrap();
    let errored = wait_until(Box::new(|s| s.tape.iter().any(|l| l.tag == "error")), 15);
    assert_eq!(errored.worker, "xmrange-test2", "config must be unchanged after a rejected identity change");
    assert_eq!(errored.state, RunState::Mining, "a bad SetIdentity must not crash the engine");

    ctl.stop();
    assert!(writer.close(), "the Ledger closed within its limit");
    let written = xmrange_core::ledger::Ledger::open(&lp, &lk).unwrap().trial_count().unwrap();
    assert_eq!(written, 0, "a session without the optimizer writes no trials, and nothing else goes to the Ledger");

    let out = std::process::Command::new("pgrep").args(["-f", "vendor/xmrig-mo/build/xmrig"]).output().unwrap();
    assert!(out.stdout.is_empty(), "xmrig should not be running after stop: {}", String::from_utf8_lossy(&out.stdout));
}

/// LIVE: the first sweep trial through the real engine lands in the Ledger, and "stop" holds the
/// efficient throttle from what was measured. About 10–15 minutes of mining. Mines to XMRANGE_TEST_WALLET if set, else to
/// the default dev wallet (GhostDrip's, not XMRange's), as tests/miner.rs does.
/// Run: cargo devtest --test engine -- --ignored --nocapture sweep
#[test]
#[ignore]
fn engine_sweep_live_smoke() {
    use xmrange_core::engine::OptimizerInit;
    use xmrange_core::ledger::{Ledger, LedgerKey, LedgerWriter};
    use xmrange_core::optimizer::driver::OptCommand;
    use xmrange_core::optimizer::{Mode, OptimizerPrefs, Profile};
    use xmrange_core::settings::Measure;
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let wallet = std::env::var("XMRANGE_TEST_WALLET").unwrap_or_else(|_| xmrange_core::config::DEFAULT_WALLET.to_string());
    let base = scratch_dir("engine-sweep").unwrap();
    std::env::set_var("XMRANGE_DATA_DIR", &base);
    std::env::remove_var("XMRANGE_XMRIG");
    let writer = LedgerWriter::spawn(Ledger::open(&base.join("ledger.sqlite"), &LedgerKey::generate()).unwrap());
    let cfg = EngineConfig {
        wallet,
        worker: "xmrange-test".into(),
        threads: 4,
        e_core_clamp: true,
        utility: false,
        dev_log: None,
        ledger: Some(writer.handle()),
        optimizer: Some(OptimizerInit {
            prefs: OptimizerPrefs { profile: Profile::Efficient, ..Default::default() },
            measure: Measure::Sys,
            chip: Some("test".into()),
            e_cores: 4,
            own_threads: None,
        }),
        pool: Default::default(), chain: Default::default(), node: Default::default(),
        tor: false,
        guard: Default::default(),
    };
    let t0 = Instant::now();
    let (ctl, rx) = spawn(cfg);
    ctl.send(Command::Optimizer(OptCommand::Seek)).unwrap();
    let last_note = std::cell::RefCell::new(None::<String>);
    let wait_for = |pred: &dyn Fn(&Snapshot) -> bool, secs: u64| -> Snapshot {
        let deadline = Instant::now() + Duration::from_secs(secs);
        loop {
            let s = rx.recv_timeout(Duration::from_secs(10)).expect("engine snapshot");
            assert_ne!(s.state, RunState::Error, "{:?}", s.error);
            // The optimizer's note and trial stage as they change: why it waits, what it measures.
            let key = s.optimizer.as_ref().map(|o| {
                let k = o.sweep.as_ref();
                format!("{:?} · testing {:?} {:?}", o.note, k.and_then(|k| k.testing).map(|t| t.label()), k.and_then(|k| k.stage))
            });
            if key != *last_note.borrow() {
                println!("[{:>4} s] {}", t0.elapsed().as_secs(), key.as_deref().unwrap_or("-"));
                *last_note.borrow_mut() = key;
            }
            // The optimizer's own tape lines (lever pulls, errors), never the pool's.
            for l in s.tape.iter().filter(|l| l.tag == "error" || l.msg.starts_with("optimizer") || l.msg.starts_with("you set")) {
                println!("[{:>4} s] {} {}", t0.elapsed().as_secs(), l.tag, l.msg);
            }
            if pred(&s) {
                return s;
            }
            assert!(Instant::now() < deadline, "not within {secs} s");
        }
    };
    wait_for(&|s| s.optimizer.as_ref().is_some_and(|o| o.trials_recorded >= 1), 16 * 60);
    ctl.send(Command::Optimizer(OptCommand::StopSeek)).unwrap();
    let s = wait_for(&|s| s.optimizer.as_ref().is_some_and(|o| o.held.is_some()), 60);
    let v = s.optimizer.unwrap();
    let held = v.held.as_ref().map(|h| h.setup.label());
    println!("[{:>4} s] stopped the sweep; holding {held:?} (mode {:?})", t0.elapsed().as_secs(), v.mode);
    assert_eq!(v.mode, Mode::Hold);
    ctl.stop();
    let (n, recs) = writer.handle().trials(10).unwrap();
    for r in &recs {
        let sc = r.score.as_ref();
        println!(
            "trial {} · {} · lpm {:?} · settle {} s · {:.0} H/s · {:.1} H/W sys · noisy {:?}",
            r.source,
            r.config.setup().label(),
            r.config.low_power_mode,
            r.settle_s,
            sc.map_or(f64::NAN, |s| s.hs),
            sc.and_then(|s| s.hw_sys).unwrap_or(f64::NAN),
            sc.map(|s| s.noisy)
        );
    }
    assert!(n >= 1 && recs[0].source == "sweep" && recs[0].config.threads == 1 && !recs[0].config.e_core_clamp, "the plan's first setup");
    assert!(writer.close(), "the Ledger closed within its limit");
    std::env::remove_var("XMRANGE_DATA_DIR");
}
