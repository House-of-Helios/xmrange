#![cfg(feature = "dev-diagnostics")]
use std::path::Path;
use std::sync::Mutex;
use xmrange_core::config::MinerConfig;
use xmrange_core::miner::{build_argv, max_threads, port_is_free, validate_threads, LaunchOptions, Miner};
use xmrange_core::paths::scratch_dir;

const W: &str = "4QAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

/// These tests mutate the shared `XMRANGE_DATA_DIR` env var (process-global), so they must not
/// run concurrently with each other. Hold this for the whole body of any test that sets it.
/// Recover from a poisoned lock (an earlier test panicked while holding it) rather than
/// cascading that panic into every later test.
static LOCK: Mutex<()> = Mutex::new(());

fn opts(port: u16) -> LaunchOptions {
    LaunchOptions { config: MinerConfig::new(W, "rig", 2).unwrap(), api_port: port, e_core_clamp: false, qos_utility: false, dev_log: None }
}

fn handoff_dirs(base: &Path) -> usize {
    std::fs::read_dir(base).map(|d| d.count()).unwrap_or(0)
}

#[test]
fn argv_never_carries_the_wallet() {
    let argv = build_argv(Path::new("/p/config.json"));
    assert_eq!(argv, vec!["-c", "/p/config.json", "--no-color"]);
    assert!(argv.iter().all(|a| !a.contains(W)));
}

#[test]
fn thread_bounds_are_enforced() {
    assert!(validate_threads(0, 8).is_err());
    assert!(validate_threads(9, 8).is_err());
    assert!(validate_threads(1, 8).is_ok() && validate_threads(8, 8).is_ok());
    assert!(max_threads() >= 1);
}

#[test]
fn launch_fails_when_port_busy_and_leaves_no_file() {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let base = scratch_dir("miner-busy").unwrap();
    std::env::set_var("XMRANGE_DATA_DIR", &base);
    let hold = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = hold.local_addr().unwrap().port();
    assert!(!port_is_free(port));
    let e = Miner::launch(Path::new("/usr/bin/true"), opts(port)).unwrap_err().to_string();
    assert!(e.contains(&port.to_string()) && e.contains("busy"), "{e}");
    assert_eq!(handoff_dirs(&base.join("run")), 0);
}

#[test]
fn launch_cleans_up_when_xmrig_exits_immediately() {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let base = scratch_dir("miner-exit").unwrap();
    std::env::set_var("XMRANGE_DATA_DIR", &base);
    let port = { std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port() };
    let e = Miner::launch(Path::new("/usr/bin/false"), opts(port)).unwrap_err().to_string();
    assert!(e.contains("exited"), "{e}");
    assert_eq!(handoff_dirs(&base.join("run")), 0, "wallet handoff must not survive a failed launch");
}

#[test]
fn launch_sweeps_stale_handoff_dirs_before_creating_a_new_one() {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let base = scratch_dir("miner-stale").unwrap();
    std::env::set_var("XMRANGE_DATA_DIR", &base);
    let run_dir = base.join("run");
    let stale = run_dir.join("handoff-deadbeef");
    std::fs::create_dir_all(&stale).unwrap();
    std::fs::write(stale.join("config.json"), "{}").unwrap();
    assert!(stale.exists());
    let port = { std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port() };
    let _ = Miner::launch(Path::new("/usr/bin/false"), opts(port));
    assert!(!stale.exists(), "stale handoff dir must be swept even though this launch failed");
}

/// LIVE: needs the patched xmrig (tools/build-xmrig.sh), network, and XMRANGE_TEST_WALLET.
/// Run: XMRANGE_TEST_WALLET=<addr> cargo devtest --test miner -- --ignored --nocapture
#[test]
#[ignore]
fn live_thread_changes_keep_dataset_and_never_rewrite_config() {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let wallet = std::env::var("XMRANGE_TEST_WALLET").expect("set XMRANGE_TEST_WALLET");
    let base = scratch_dir("miner-live").unwrap();
    std::env::set_var("XMRANGE_DATA_DIR", &base);
    let log = base.join("xmrig.log");
    let o = LaunchOptions {
        config: MinerConfig::new(&wallet, "xmrange-test", 8).unwrap(),
        api_port: 3620,
        e_core_clamp: false,
        qos_utility: false,
        dev_log: Some(log.clone()),
    };
    let mut m = Miner::launch(&xmrange_core::paths::xmrig_binary(), o).unwrap();
    let handoff = m.handoff_file().unwrap();
    assert!(!handoff.exists(), "handoff deleted once the API answered");
    std::thread::sleep(std::time::Duration::from_secs(20)); // dataset init + first job
    for n in [4, 8, 2] {
        m.set_threads(n).unwrap();
        std::thread::sleep(std::time::Duration::from_secs(8));
        assert!(!handoff.exists(), "config rewritten to disk after PUT threads={n}");
        assert_eq!(m.threads(), n);
    }
    m.stop().unwrap();
    let text = std::fs::read_to_string(&log).unwrap();
    assert_eq!(text.matches("init dataset").count(), 1, "thread changes must reuse the dataset");
    assert!(!text.contains("configuration saved"));
    let readies: Vec<&str> = text.lines().filter(|l| l.contains("READY threads")).collect();
    assert!(readies.iter().any(|l| l.contains("READY threads 2/2")), "{readies:?}");
}

/// LIVE: the clamp goes on and off on a running xmrig and survives a thread change (new worker
/// threads inherit the background band), with the dataset kept. Needs the patched xmrig and network. Mines to
/// XMRANGE_TEST_WALLET if set, else to the default dev wallet (GhostDrip's), as the app does.
/// Run: cargo devtest --test miner -- --ignored --nocapture clamp
#[test]
#[ignore]
fn live_clamp_toggles_and_survives_a_thread_change() {
    use xmrange_core::levers::Setup;
    use xmrange_core::telemetry::{AppleSiliconSource, PowerSource};
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let wallet = std::env::var("XMRANGE_TEST_WALLET").unwrap_or_else(|_| xmrange_core::config::DEFAULT_WALLET.to_string());
    let base = scratch_dir("miner-clamp").unwrap();
    std::env::set_var("XMRANGE_DATA_DIR", &base);
    let log = base.join("xmrig.log");
    let o = LaunchOptions {
        config: MinerConfig::new(&wallet, "xmrange-test", 4).unwrap(),
        api_port: 3620,
        e_core_clamp: false,
        qos_utility: false,
        dev_log: Some(log.clone()),
    };
    // Only counted, never printed: the raw log is redacted when the miner stops.
    let count = |needle: &str| std::fs::read_to_string(&log).unwrap_or_default().matches(needle).count();
    let mut m = Miner::launch(&xmrange_core::paths::xmrig_binary(), o).unwrap();
    let mut src = AppleSiliconSource::new().unwrap();
    // Mean P-cluster active ratio over 20 s, after 40 s to let the scheduler move threads.
    let p_active = |src: &mut AppleSiliconSource| {
        std::thread::sleep(std::time::Duration::from_secs(40));
        let p = (0..20).map(|_| src.sample(1000).unwrap().p_active).sum::<f64>() / 20.0;
        eprintln!("p_active {p:.2}");
        p
    };
    assert!(p_active(&mut src) > 0.5, "4 unclamped threads land on the P-cores");
    m.apply(&Setup { threads: 4, e_core_clamp: true, utility: false }).unwrap();
    assert!(p_active(&mut src) < 0.3, "clamped: the E-cores only");
    m.apply(&Setup { threads: 3, e_core_clamp: true, utility: false }).unwrap();
    assert!(p_active(&mut src) < 0.3, "a thread change keeps the clamp");
    m.apply(&Setup { threads: 3, e_core_clamp: false, utility: false }).unwrap();
    assert!(p_active(&mut src) > 0.4, "unclamped again");
    assert_eq!(m.setup(), Setup { threads: 3, e_core_clamp: false, utility: false });
    m.stop().unwrap();
    assert_eq!(count("init dataset"), 1, "lever changes must reuse the dataset");
    std::env::remove_var("XMRANGE_DATA_DIR");
}
