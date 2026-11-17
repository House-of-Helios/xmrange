use serde::Serialize;
use std::borrow::Cow;
use std::fs::{File, OpenOptions, TryLockError};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tauri::utils::assets::{AssetKey, AssetsIter, CspHash};
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, State, WebviewWindow, WebviewWindowBuilder};
use xmrange_core::config::{mask_wallet, validate_worker, DEFAULT_WALLET};
use xmrange_core::context::read_low_power;
use xmrange_core::context::{read_context, Context};
use xmrange_core::engine::OptimizerInit;
use xmrange_core::engine::{self, Command, EngineConfig, EngineCtl};
use xmrange_core::ledger::{curve_view, open_default, trial_views, LedgerWriter, TrialsView};
use xmrange_core::levers::Setup;
use xmrange_core::lowpower;
use xmrange_core::miner::{max_threads, validate_threads};
use xmrange_core::optimizer::driver::{OptCommand, Optimizer};
use xmrange_core::optimizer::{hw, picks, usable, Mode, Measured, OptimizerPrefs, Picks, Profile};
use xmrange_core::orphan;
use xmrange_core::paths::{data_dir, xmrig_binary};
use xmrange_core::settings::{Measure, Settings, Wallet};
use xmrange_core::telemetry::{now_secs, MachineInfo, PowerSample};

/// Self-check settings backup: dev builds only. The declaration's cfg is the gate that keeps it out
/// of release builds; `self_check_surface_is_dev_only` fails if it is ever lost.
#[cfg(feature = "dev-diagnostics")]
mod selfcheck_backup;

/// Product tagline, surfaced verbatim in `app_info()`.
const TAGLINE: &str = "hashes per watt · every watt, weighed";

/// The release bundle id (`tauri.conf.json`'s `identifier`), pinned in the updater's codesign
/// requirement check. The dev build's is `house.helios.xmrange.dev` (`make-dev-app.sh`'s
/// Info.plist); the updater only ever installs the release id.
#[cfg(not(feature = "dev-diagnostics"))]
const RELEASE_BUNDLE_ID: &str = "house.helios.xmrange";

/// The running engine, tagged with its generation so a finished engine's forwarding thread can
/// release the slot without ever clearing a newer engine started after it.
type EngineSlot = Mutex<Option<(u64, EngineCtl)>>;

/// Stops in flight: engines already taken out of the slot whose `stop()` (the join) hasn't
/// finished. The quit path waits for these (final review I3): a stop clicked just before ⌘Q still
/// reaches xmrig before the process exits.
#[derive(Default)]
struct StopTracker {
    in_flight: Mutex<u32>,
    done: Condvar,
}

impl StopTracker {
    fn begin(&self) {
        *self.in_flight.lock().unwrap_or_else(|e| e.into_inner()) += 1;
    }

    fn end(&self) {
        let mut n = self.in_flight.lock().unwrap_or_else(|e| e.into_inner());
        *n = n.saturating_sub(1);
        self.done.notify_all();
    }

    fn idle(&self) -> bool {
        *self.in_flight.lock().unwrap_or_else(|e| e.into_inner()) == 0
    }

    /// Waits until no stop is in flight, or `limit` passes. The condvar wait releases the lock,
    /// so nothing is held while the stops' joins run. True if they all finished.
    fn wait_idle(&self, limit: Duration) -> bool {
        let n = self.in_flight.lock().unwrap_or_else(|e| e.into_inner());
        let (n, _) = self.done.wait_timeout_while(n, limit, |n| *n > 0).unwrap_or_else(|e| e.into_inner());
        *n == 0
    }
}

/// One engine being stopped. Dropping it (or `run`) performs the stop, then marks it finished,
/// even if the stop panics.
struct InFlightStop {
    ctl: Option<EngineCtl>,
    tracker: Arc<StopTracker>,
}

impl InFlightStop {
    fn run(self) {
        drop(self);
    }
}

impl Drop for InFlightStop {
    fn drop(&mut self) {
        if let Some(ctl) = self.ctl.take() {
            ctl.stop();
        }
        self.tracker.end();
    }
}

/// How long quitting waits for a stop already in flight: one engine tick (about 1 s of sampling
/// plus up to 3 s of API timeout) and `Miner::shutdown`'s 10 s, with room to spare.
const EXIT_STOP_WAIT: Duration = Duration::from_secs(20);

struct AppState {
    engine: EngineSlot,
    stops: Arc<StopTracker>,
    /// Last generation handed out by `start_mining`.
    engine_gen: AtomicU64,
    /// The wallet mining pays lives in `settings.wallet`: `None` is the disclosed GhostDrip
    /// default, `Some` the user's own (owner decision 2026-09-28: no separate secret store).
    settings: Mutex<Settings>,
    /// Read once at startup via `telemetry::open_source()` (macmon, or the SMC on Intel).
    machine: Option<MachineInfo>,
    /// The Ledger's writer thread; `None` = no history this session (the startup note says why).
    /// Closed on the way out, once every engine is down (`stop_engine_for_exit`).
    ledger: Mutex<Option<LedgerWriter>>,
    /// What startup did that the user should hear about (a reaped orphan, the Ledger), for the tape.
    startup_note: Option<String>,
    /// The last run left its marker: it did not exit cleanly. The UI offers a report, once.
    crashed_last_time: bool,
    /// M1: held for the app's lifetime; the OS releases it when the process ends, however it ends.
    _instance_lock: Option<File>,
}

/// Take the engine out of its slot and mark its stop as in flight, in one step under the slot's
/// lock, so there is no moment when a live engine is in neither place.
fn begin_stop(state: &AppState) -> Option<InFlightStop> {
    let mut slot = state.engine.lock().unwrap_or_else(|e| e.into_inner());
    let (_, ctl) = slot.take()?;
    state.stops.begin();
    drop(slot);
    Some(InFlightStop { ctl: Some(ctl), tracker: state.stops.clone() })
}

/// No engine running and none stopping: the idle sampler may use macmon.
fn engine_idle(state: &AppState) -> bool {
    state.engine.lock().unwrap_or_else(|e| e.into_inner()).is_none() && state.stops.idle()
}

/// Final review M1: one app per data dir. An exclusive `flock` on `<data dir>/app.lock`, held for
/// the process's life. `Ok(None)` means another copy holds it.
///
/// Final review M8: the data dir is made 0700 here (the first thing `main()` touches in it), not
/// left to `create_dir_all`'s umask-shaped mode. `mode` only applies to a directory this call
/// creates, so a dir left loose by an older build is tightened too, matching `Settings::save`.
fn instance_lock(dir: &Path) -> std::io::Result<Option<File>> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    let file = OpenOptions::new().create(true).truncate(false).write(true).open(dir.join("app.lock"))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(e)) => Err(e),
    }
}

#[derive(Serialize)]
struct WalletView {
    masked: String,
    is_default: bool,
    label: String,
}

/// `None` resolves to the GhostDrip dev default wallet; `Some(addr)` is the user's own.
fn resolve_wallet(user: Option<&str>) -> (String, bool) {
    match user {
        Some(w) => (w.to_string(), false),
        None => (DEFAULT_WALLET.to_string(), true),
    }
}

fn wallet_view(user: Option<&str>) -> WalletView {
    let (wallet, is_default) = resolve_wallet(user);
    let label = if is_default { "default · ghostdrip dev wallet".to_string() } else { "your wallet".to_string() };
    WalletView { masked: mask_wallet(&wallet), is_default, label }
}

/// Startup notes for the tape, in order (a reaped orphan, then the Ledger), joined by " · ".
fn join_notes(a: Option<&str>, b: Option<String>) -> Option<String> {
    match (a, b) {
        (Some(a), Some(b)) => Some(format!("{a} · {b}")),
        (Some(a), None) => Some(a.to_string()),
        (None, b) => b,
    }
}

/// Take the slot's value only if it still belongs to generation `gen`. A slot already emptied by
/// `stop_mining`, or holding a newer engine, is left as it is.
fn take_if_gen<T>(slot: &Mutex<Option<(u64, T)>>, gen: u64) -> Option<T> {
    let mut guard = slot.lock().unwrap_or_else(|e| e.into_inner());
    if matches!(guard.as_ref(), Some((id, _)) if *id == gen) {
        guard.take().map(|(_, value)| value)
    } else {
        None
    }
}

/// Hand a live-settings command to the running engine, if there is one. No engine, or one that
/// has just died, counts as "not running": the caller has already saved the setting, and the next
/// `start_mining` picks it up, so this never turns into an error.
fn send_if_running(state: &AppState, cmd: Command) {
    if let Some((_, ctl)) = state.engine.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        let _ = ctl.send(cmd);
    }
}

/// Changes the settings on disk first, then in memory: a failed write leaves both as they were,
/// so the next start never runs a value the window rolled back.
fn change_settings(state: &AppState, dir: &Path, f: impl FnOnce(&mut Settings)) -> Result<(), String> {
    let mut stored = state.settings.lock().unwrap_or_else(|e| e.into_inner());
    let mut next = stored.clone();
    f(&mut next);
    next.save(dir).map_err(|e| e.to_string())?;
    *stored = next;
    Ok(())
}

/// Hand input drops the optimizer to manual (spec §2 "Mode: hold ideal vs manual").
fn to_manual(p: &mut OptimizerPrefs) {
    p.mode = Mode::Manual;
    p.sweep = None;
}

/// The engine's latest prefs into settings.json. Runs on the snapshot pump's thread, never the main one.
fn save_prefs(state: &AppState, p: &OptimizerPrefs) {
    let mut s = state.settings.lock().unwrap_or_else(|e| e.into_inner());
    if s.optimizer != *p {
        s.optimizer = p.clone();
        if let Err(e) = s.save(&data_dir()) {
            eprintln!("settings: {e}");
        }
    }
}

/// Without power readings (macmon didn't answer at launch) `start_mining` runs the engine with no
/// optimizer, so every optimizer command says so instead of doing nothing (P4-C19).
/// It shows inline in the tuning tab's captions, so it stays one short line.
const NO_POWER_READINGS: &str = "no power readings on this mac";
const NOT_MINING: &str = "start mining to run trials";

fn optimizer_available(state: &AppState) -> Result<(), String> {
    state.machine.as_ref().map(|_| ()).ok_or_else(|| NO_POWER_READINGS.to_string())
}

/// Trials commands need a running engine: there is nothing to measure while stopped. The UI's
/// "run trials" starts mining first.
fn optimizer_command(state: &AppState, c: OptCommand) -> Result<(), String> {
    optimizer_available(state)?;
    let slot = state.engine.lock().unwrap_or_else(|e| e.into_inner());
    let Some((_, ctl)) = slot.as_ref() else { return Err(NOT_MINING.into()) };
    ctl.send(Command::Optimizer(c)).map_err(|e| e.to_string())
}

#[tauri::command]
fn ping() -> String {
    "pong".to_string()
}

/// Self-check mode: dev builds with `XMRANGE_SELF_CHECK=1` only. Always false in a release build,
/// whatever the environment says, so a release app never runs the settings-writing checks,
/// never emits heartbeats, and never exits on a report.
fn self_check_mode() -> bool {
    #[cfg(feature = "dev-diagnostics")]
    {
        std::env::var("XMRANGE_SELF_CHECK").as_deref() == Ok("1")
    }
    #[cfg(not(feature = "dev-diagnostics"))]
    {
        false
    }
}

/// The self-check's report. A no-op outside self-check mode (and so always in release builds).
/// It ends the process with `exit`, which skips `RunEvent::Exit`, so it takes the quit path's
/// stop first: any engine stops and the Ledger is closed whole, as on a normal quit.
#[tauri::command]
fn self_check_report(app: AppHandle, report: serde_json::Value) {
    if self_check_mode() {
        stop_engine_for_exit(&app);
        #[cfg(feature = "dev-diagnostics")]
        match selfcheck_backup::restore(&data_dir()) {
            Ok(true) => println!("SELF-CHECK-SETTINGS restored from backup"),
            Ok(false) => {}
            Err(e) => println!("SELF-CHECK-SETTINGS restore failed: {e}"),
        }
        println!("SELF-CHECK {report}");
        let ok = report.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
        std::process::exit(if ok { 0 } else { 1 });
    }
}

/// Dev builds only: `XMRANGE_AUTOSTART=1` starts mining during setup, for
/// `tools/check-app-lifecycle.sh`. Always false in a release build, whatever the environment says.
fn autostart_mode() -> bool {
    #[cfg(feature = "dev-diagnostics")]
    {
        std::env::var("XMRANGE_AUTOSTART").as_deref() == Ok("1")
    }
    #[cfg(not(feature = "dev-diagnostics"))]
    {
        false
    }
}

/// Ruling P2-R10: an agent's test run (self-check or autostart) never takes the owner's screen.
/// Its window is created hidden and unfocused, and the app can't be activated or take a Dock slot.
/// False for every normal launch, and always false in a release build (both gates are hard false).
fn background_mode() -> bool {
    self_check_mode() || autostart_mode()
}

/// The dev self-check harness. It lives in `ui-dev/`, outside `frontendDist` (`ui/`), so a release
/// build never embeds it: the bytes are compiled in here, in dev builds only.
#[cfg(feature = "dev-diagnostics")]
const DEV_HARNESS: (&str, &[u8]) = ("selfcheck.js", include_bytes!("../ui-dev/selfcheck.js"));

/// The dev-only frontend files this build adds to the embedded `ui/`: the harness in dev builds,
/// nothing in release.
fn dev_assets() -> &'static [(&'static str, &'static [u8])] {
    #[cfg(feature = "dev-diagnostics")]
    {
        &[DEV_HARNESS]
    }
    #[cfg(not(feature = "dev-diagnostics"))]
    {
        &[]
    }
}

/// The embedded frontend (`ui/`) plus `dev_assets()`. `None` only for the moment it takes to swap
/// the original assets in (`Context::set_assets` hands back the one it replaces).
struct FrontendAssets<R: tauri::Runtime>(Option<Box<dyn tauri::Assets<R>>>);

impl<R: tauri::Runtime> tauri::Assets<R> for FrontendAssets<R> {
    fn setup(&self, app: &tauri::App<R>) {
        if let Some(inner) = &self.0 {
            inner.setup(app);
        }
    }

    fn get(&self, key: &AssetKey) -> Option<Cow<'_, [u8]>> {
        let name = key.as_ref().trim_start_matches('/');
        if let Some((_, bytes)) = dev_assets().iter().find(|(n, _)| *n == name) {
            return Some(Cow::Borrowed(bytes));
        }
        self.0.as_ref()?.get(key)
    }

    fn iter(&self) -> Box<AssetsIter<'_>> {
        let dev = dev_assets().iter().map(|(n, b)| (Cow::Owned(format!("/{n}")), Cow::Borrowed(*b)));
        match &self.0 {
            Some(inner) => Box::new(inner.iter().chain(dev)),
            None => Box::new(dev),
        }
    }

    fn csp_hashes(&self, html_path: &AssetKey) -> Box<dyn Iterator<Item = CspHash<'_>> + '_> {
        match &self.0 {
            Some(inner) => inner.csp_hashes(html_path),
            None => Box::new(std::iter::empty()),
        }
    }
}

/// A background-mode (test) app's longest life: the dead-man's switch stops it after this.
const BACKGROUND_MAX_LIFETIME: Duration = Duration::from_secs(600);

/// Why a background-mode app must stop now, if it must. Its parent changed (the runner that
/// started it died and it was reparented, usually to launchd, pid 1), or it outlived
/// `BACKGROUND_MAX_LIFETIME`.
fn deadman_reason(start_ppid: u32, now_ppid: u32, age: Duration) -> Option<&'static str> {
    if now_ppid != start_ppid || now_ppid == 1 {
        Some("the process that started it is gone")
    } else if age >= BACKGROUND_MAX_LIFETIME {
        Some("its maximum lifetime passed")
    } else {
        None
    }
}

/// The dead-man's switch for background mode (dev test runs only, see `background_mode`). A hidden
/// app with no Dock slot must never mine unseen after its runner dies, so once a second it checks
/// `deadman_reason` and, when that fires, stops through the normal stop path and exits.
fn spawn_deadman(handle: AppHandle, start_ppid: u32, started: Instant) {
    thread::spawn(move || loop {
        thread::sleep(Duration::from_secs(1));
        if let Some(reason) = deadman_reason(start_ppid, std::os::unix::process::parent_id(), started.elapsed()) {
            use std::io::Write as _;
            // Never println!: with the runner gone, a closed pipe must not panic this thread.
            let _ = writeln!(std::io::stderr(), "DEADMAN: {reason}; stopping");
            stop_engine_for_exit(&handle);
            handle.exit(0);
            // Mining is already stopped; if the event loop hasn't exited by now, leave anyway.
            thread::sleep(Duration::from_secs(10));
            std::process::exit(0);
        }
    });
}

/// `send_report` refuses with this prefix and the scrubbed text when scrubbing changed what the user sent.
const RESCRUBBED: &str = "rescrubbed:\n";

/// Present from launch to a clean quit; found at launch, the last run ended without one.
const RUNNING_MARKER: &str = "running.marker";

/// The one stop path for quitting, shared by `RunEvent::Exit`, the SIGTERM/SIGINT handler and the
/// dead-man's switch. It stops a live engine, then waits for any stop already in flight (a stop
/// clicked just before quitting, I3), so the process never exits with xmrig still running. No
/// lock is held across a join: `begin_stop` releases the slot first, and the wait is a condvar's.
fn stop_engine_for_exit(app: &AppHandle) {
    let state = app.state::<AppState>();
    if let Some(stop) = begin_stop(&state) {
        stop.run();
    }
    if !state.stops.wait_idle(EXIT_STOP_WAIT) {
        let _ = std::io::Write::write_all(&mut std::io::stderr(), b"a stop was still in flight when quitting\n");
    }
    // Then the own Monero node (its database closes cleanly before the app goes).
    stop_node_for_exit();
    // Every engine is down, so no more trials: close the Ledger.
    let msg = close_ledger(&state.ledger);
    let _ = std::fs::remove_file(data_dir().join(RUNNING_MARKER));
    let _ = std::fs::remove_file(data_dir().join(xmrange_core::report::CRASH_FILE));
    if let Some(msg) = msg {
        // Never println!: from the signal thread a closed pipe must not panic.
        let _ = std::io::Write::write_all(&mut std::io::stderr(), msg.as_bytes());
    }
}

/// Closes the session's Ledger writer, if it is still open, and says how it went. The close is
/// bounded (T7 review m4): quitting never hangs on it, and a write it cuts short is one whole
/// transaction or none.
///
/// The slot's lock is held across the close (T9 review m2), so a second quit path arriving
/// meanwhile (SIGTERM during `RunEvent::Exit`) waits for it instead of finding `None` and ending
/// the process mid-close. That wait is the close's own bound. Only `start_mining` also takes this
/// lock, after the engine lock, and nothing here takes the engine lock: no deadlock.
fn close_ledger(slot: &Mutex<Option<LedgerWriter>>) -> Option<String> {
    let mut guard = slot.lock().unwrap_or_else(|e| e.into_inner());
    let writer = guard.take()?;
    let closed = writer.close();
    drop(guard);
    Some(if closed {
        "ledger closed\n".to_string()
    } else {
        "ledger still writing when quitting; it was left to finish\n".to_string()
    })
}

/// Dev builds only: `XMRANGE_SELF_CHECK_LIVE=1` (with `XMRANGE_SELF_CHECK=1`) lets the
/// self-check start real mining for a short live-wiring run. Always false in release builds.
fn self_check_live() -> bool {
    #[cfg(feature = "dev-diagnostics")]
    {
        self_check_mode() && std::env::var("XMRANGE_SELF_CHECK_LIVE").as_deref() == Ok("1")
    }
    #[cfg(not(feature = "dev-diagnostics"))]
    {
        false
    }
}

#[tauri::command]
fn app_info(state: State<AppState>) -> Result<serde_json::Value, String> {
    let wallet = wallet_view(state.settings.lock().unwrap_or_else(|e| e.into_inner()).wallet.0.as_deref());
    let dev_build = cfg!(feature = "dev-diagnostics");
    let info = serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "tagline": TAGLINE,
        "machine": state.machine.clone(),
        "max_threads": max_threads(),
        "low_power": lowpower::latest(),
        "low_power_supported": xmrange_core::context::low_power_supported(),
        // Laptop or desktop, from the spec table (None for a model it doesn't know).
        "form": xmrange_core::context::hw_model().and_then(|m| xmrange_core::macs::lookup(&m)).map(|m| if m.form == xmrange_core::macs::Form::Laptop { "laptop" } else { "desktop" }),
        "price": LAST_PRICE.lock().unwrap_or_else(|e| e.into_inner()).clone(),
        "wallet": wallet,
        "dev_build": dev_build,
        "self_check": self_check_mode(),
        "self_check_live": self_check_live(),
        "startup_note": state.startup_note,
        "crashed_last_time": state.crashed_last_time,
    });
    Ok(info)
}

#[tauri::command]
fn get_settings(state: State<AppState>) -> Result<Settings, String> {
    Ok(state.settings.lock().unwrap_or_else(|e| e.into_inner()).clone())
}

/// What `save_settings` persists: the incoming settings, validated, with six fields pinned to
/// the stored values:
/// - `update_folder`: retired in 0.0.1 (updates come from GitHub); old settings files keep it, unused;
/// - `worker`, `threads`, `e_cores_only` and `wallet`, which have their own commands (`set_worker`,
///   `set_threads`/`apply_pick`, `set_wallet`/`clear_wallet`) that also reach the running engine. A save carrying an older
///   copy of them (a worker edit still in flight when the period pill is clicked, final review
///   M3) must never write them back;
/// - and `optimizer`, which only the optimizer commands and the engine's snapshots move.
fn accept_settings(incoming: Settings, stored: &Settings, max_threads: u32) -> Result<Settings, String> {
    let accepted = Settings {
        update_folder: stored.update_folder.clone(),
        node_dir: stored.node_dir.clone(),
        worker: stored.worker.clone(),
        threads: stored.threads,
        e_cores_only: stored.e_cores_only,
        utility_manual: stored.utility_manual,
        profile_log: stored.profile_log.clone(),
        wallet: stored.wallet.clone(),
        optimizer: stored.optimizer.clone(),
        ..incoming
    };
    accepted.validate(max_threads).map_err(|e| e.to_string())?;
    Ok(accepted)
}

const MINING_LOCKED: &str = "stop mining to change this · it applies at start";

/// Validates before anything is written; a refusal comes back verbatim for the UI to show.
#[tauri::command]
fn save_settings(state: State<AppState>, settings: Settings) -> Result<(), String> {
    let mut stored = state.settings.lock().unwrap_or_else(|e| e.into_inner());
    let accepted = accept_settings(settings, &stored, max_threads())?;
    // The pool, chain, node and Tor are read once, at start: changed while mining they would show
    // one thing and run another. Refused instead (the window locks these controls while mining).
    let at_start = |s: &Settings| (s.pool, s.p2pool_chain, s.node, s.tor);
    if at_start(&accepted) != at_start(&stored) && !engine_idle(&state) {
        return Err(MINING_LOCKED.into());
    }
    accepted.save(&data_dir()).map_err(|e| e.to_string())?;
    let measure = (accepted.measure != stored.measure).then_some(accepted.measure);
    let guard = (accepted.guard != stored.guard).then_some(accepted.guard);
    if accepted.price_feed && !stored.price_feed {
        PRICE_NUDGE.store(true, Ordering::SeqCst);
    }
    *stored = accepted;
    drop(stored);
    if let Some(m) = measure {
        send_if_running(&state, Command::Optimizer(OptCommand::SetMeasure(m)));
    }
    if let Some(g) = guard {
        send_if_running(&state, Command::SetGuard(g));
    }
    Ok(())
}

/// The chain folder for XMRange's own node: picked with the native folder dialog (an external drive
/// is fine), checked (`monerod::validate_dir`), then saved; disk first, then memory. Empty is off.
/// Locked while mining: the node is read at start.
#[tauri::command]
fn set_node_folder(state: State<AppState>, folder: Option<String>) -> Result<(), String> {
    let next = xmrange_core::monerod::validate_dir(folder.as_deref()).map_err(|e| e.to_string())?;
    if next != state.settings.lock().unwrap_or_else(|e| e.into_inner()).node_dir && !engine_idle(&state) {
        return Err(MINING_LOCKED.into());
    }
    change_settings(&state, &data_dir(), |s| s.node_dir = next)
}

/// What the update pill shows: a verified GitHub offer's version and notes (`update::Offer`).
#[derive(Debug, Clone, Serialize)]
struct UpdateOffer {
    version: String,
    notes: String,
}

/// The latest verified offer from GitHub (`spawn_update_watch`), or `None`. Memory only.
static GITHUB_OFFER: Mutex<Option<xmrange_core::update::Offer>> = Mutex::new(None);

#[tauri::command]
fn update_status() -> Result<Option<UpdateOffer>, String> {
    Ok(update_offer())
}

fn update_offer() -> Option<UpdateOffer> {
    GITHUB_OFFER.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|o| UpdateOffer { version: o.version.clone(), notes: o.notes.clone() })
}

/// The XMR price feed (doctrine §1b): only while the user turned it on (after its confirmation)
/// and the currency is "$", since the feed is in USD. Hourly through Tor; a failure (a cold Tor
/// start can pass 90 s) retries in 5 minutes. Never in a self-check run.
const PRICE_EVERY: Duration = Duration::from_secs(3600);
const PRICE_RETRY: Duration = Duration::from_secs(300);
/// Set when the feed is turned on, so the first price comes at once.
static PRICE_NUDGE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// The latest fetch, for app_info (a reload shows it without waiting an hour). Memory only.
static LAST_PRICE: Mutex<Option<PriceFeed>> = Mutex::new(None);

/// One fetch's outcome: the price (USD) and its source, or why there is none.
#[derive(Debug, Clone, Serialize)]
struct PriceFeed {
    usd: Option<f64>,
    source: Option<&'static str>,
    error: Option<String>,
    /// Unix seconds of the fetch.
    t: f64,
}

fn price_feed_on(state: &AppState) -> bool {
    let s = state.settings.lock().unwrap_or_else(|e| e.into_inner());
    s.price_feed && s.currency == "$"
}

fn spawn_price_watch(handle: AppHandle) {
    if self_check_mode() {
        return;
    }
    thread::spawn(move || {
        let mut next = Instant::now();
        loop {
            let on = price_feed_on(&handle.state::<AppState>());
            let nudged = PRICE_NUDGE.swap(false, Ordering::SeqCst);
            if !on {
                next = Instant::now(); // turned back on later: fetch at once
            } else if nudged || Instant::now() >= next {
                let cancel = || !price_feed_on(&handle.state::<AppState>());
                let feed = match xmrange_core::price::fetch_through_tor(&cancel) {
                    Ok(q) => PriceFeed { usd: Some(q.usd), source: Some(q.source), error: None, t: xmrange_core::telemetry::now_secs() },
                    Err(e) => PriceFeed { usd: None, source: None, error: Some(xmrange_core::redact::redact_ips(&e.to_string())), t: xmrange_core::telemetry::now_secs() },
                };
                next = Instant::now() + if feed.usd.is_some() { PRICE_EVERY } else { PRICE_RETRY };
                *LAST_PRICE.lock().unwrap_or_else(|e| e.into_inner()) = Some(feed.clone());
                let _ = handle.emit("xmrange://price", &feed);
            }
            thread::sleep(Duration::from_secs(2));
        }
    });
}

/// GitHub is asked at launch, then every 6 hours; a failure (Tor or GitHub unreachable) retries
/// in an hour.
const UPDATE_EVERY: Duration = Duration::from_secs(6 * 3600);
const UPDATE_RETRY: Duration = Duration::from_secs(3600);

/// Release builds ask GitHub's latest release (through Tor, doctrine §1c) and keep a verified,
/// newer offer for the pill (xmrange://update, sent when it changes). A dev app and a self-check
/// run never ask GitHub.
fn spawn_update_watch(handle: AppHandle) {
    if cfg!(feature = "dev-diagnostics") || self_check_mode() {
        return;
    }
    thread::spawn(move || loop {
        let wait = match xmrange_core::update::check_github(env!("CARGO_PKG_VERSION"), &|| false) {
            Ok(offer) => {
                let changed = {
                    let mut g = GITHUB_OFFER.lock().unwrap_or_else(|e| e.into_inner());
                    let changed = g.as_ref().map(|o| &o.version) != offer.as_ref().map(|o| &o.version);
                    *g = offer;
                    changed
                };
                if changed {
                    let _ = handle.emit("xmrange://update", &update_offer());
                }
                UPDATE_EVERY
            }
            Err(_) => UPDATE_RETRY,
        };
        thread::sleep(wait);
    });
}

/// The flag `run_install` leaves for a fresh launch to consume once (P5-R3 ruling): mining was
/// running right before the swap, so the relaunched app resumes it, then deletes the flag.
#[cfg(not(feature = "dev-diagnostics"))]
const RESUME_MINING_FLAG: &str = "resume-mining.flag";

/// The installed app's bundle root (`…/XMRange.app`) from the running binary's own path:
/// `Contents/MacOS/<exe>` is always two levels below it.
#[cfg(not(feature = "dev-diagnostics"))]
fn app_bundle_root() -> Result<std::path::PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    exe.parent()
        .and_then(std::path::Path::parent)
        .and_then(std::path::Path::parent)
        .map(std::path::Path::to_path_buf)
        .ok_or_else(|| "could not locate the app bundle".to_string())
}

/// Installs the GitHub offer: download it through Tor, check its SHA-256 against the signed feed,
/// unpack it, copy the new bundle in beside this one and verify it, then stop mining, swap it in
/// with a rename, relaunch, exit. Release builds only (P5-R4); a dev app never replaces itself.
#[cfg(not(feature = "dev-diagnostics"))]
fn run_install(app: AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let offer = GITHUB_OFFER.lock().unwrap_or_else(|e| e.into_inner()).clone().ok_or_else(|| "no update offer".to_string())?;
    let work = data_dir().join("update");
    let source = xmrange_core::update::fetch(&offer, &work, &|| false).map_err(|e| e.to_string())?;

    let bundle_root = app_bundle_root()?;
    let name = bundle_root.file_name().and_then(|n| n.to_str()).unwrap_or("XMRange.app").to_string();

    // 1. Copy the new bundle beside the installed app, under a temp name, then drop the download.
    //    Mining goes on: a copy or verify that fails leaves the app as it was.
    let staged = bundle_root.with_file_name(format!(".{name}.update"));
    let _ = std::fs::remove_dir_all(&staged);
    let copied = std::process::Command::new("/usr/bin/ditto").arg(&source).arg(&staged).status().map_err(|e| e.to_string());
    let _ = std::fs::remove_dir_all(&work);
    if !copied?.success() {
        let _ = std::fs::remove_dir_all(&staged);
        return Err("could not copy the update".into());
    }

    // 2. Verify before anything is swapped (the signed feed already vouched for the zip). The app is ad-hoc signed (no Team ID until a Developer
    // ID arrives in Stage 4), so a Team-ID or notarization requirement isn't possible yet; pinning
    // the bundle identifier instead still catches a corrupted or mismatched bundle.
    // The leading "=" tells codesign this is literal requirement source text, not a path to a
    // file containing one (csreq(1), "specifying requirements") — without it, codesign reads the
    // whole string as a filename and fails with a confusing "No such file or directory".
    let requirement = format!("=identifier \"{RELEASE_BUNDLE_ID}\"");
    let verified = std::process::Command::new("/usr/bin/codesign")
        .args(["--verify", "--strict", "--deep", "-R"])
        .arg(&requirement)
        .arg(&staged)
        .status()
        .map_err(|e| e.to_string())?;
    if !verified.success() {
        let _ = std::fs::remove_dir_all(&staged);
        return Err("the update did not verify (codesign --verify)".into());
    }

    // 3. Only now stop mining cleanly (and close the Ledger), remembering whether it was mining.
    let was_mining = !engine_idle(&state);
    stop_engine_for_exit(&app);

    // 4. Swap with rename: the old bundle aside, the staged one into place; roll back on failure.
    //    Mining is stopped and the Ledger closed by now, so a failure here asks for a reopen.
    let backup = bundle_root.with_file_name(format!(".{name}.old"));
    let _ = std::fs::remove_dir_all(&backup);
    if let Err(e) = std::fs::rename(&bundle_root, &backup) {
        let _ = std::fs::remove_dir_all(&staged);
        return Err(format!("could not swap in the update: {e} · reopen XMRange"));
    }
    if let Err(e) = std::fs::rename(&staged, &bundle_root) {
        let _ = std::fs::rename(&backup, &bundle_root);
        return Err(format!("could not swap in the update: {e} · reopen XMRange"));
    }
    let _ = std::fs::remove_dir_all(&backup);

    // It was mining: the relaunched app resumes once, through this one-shot flag.
    if was_mining {
        let _ = std::fs::write(data_dir().join(RESUME_MINING_FLAG), b"");
    }

    // 5. Relaunch a fresh instance. If that fails, stay up so the error can show: the new bundle is
    //    already in place, so reopening XMRange by hand finishes the update.
    let relaunched = std::process::Command::new("/usr/bin/open")
        .args(["-n", bundle_root.to_str().unwrap_or_default()])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !relaunched {
        return Err("update installed · reopen XMRange to finish".into());
    }

    // 6. Exit. The engine is stopped and the Ledger closed already (stop_engine_for_exit, above).
    std::process::exit(0);
}

#[cfg(feature = "dev-diagnostics")]
#[tauri::command]
async fn install_update(_app: AppHandle) -> Result<(), String> {
    Err("release builds only".into())
}

#[cfg(not(feature = "dev-diagnostics"))]
#[tauri::command]
async fn install_update(app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || run_install(app)).await.map_err(|e| e.to_string())?
}

/// Starts an engine and returns its generation (the running one's, if an engine is already live).
/// `xmrange://stopped` carries the same generation, so the UI can ignore a stale one.
#[tauri::command]
fn start_mining(app: AppHandle, state: State<AppState>) -> Result<u64, String> {
    start_engine(app, &state, None)
}

/// Start / continue trials while nothing mines: mark the sweep (the newest unfinished one in the
/// Ledger, else a new one) in settings, then launch xmrig straight onto its next setup, so it never
/// mines at the user's own thread count first. The engine's optimizer resumes that sweep.
#[tauri::command]
async fn start_trials(app: AppHandle, state: State<'_, AppState>) -> Result<u64, String> {
    let handle = state.ledger.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(LedgerWriter::handle);
    let recs = match handle {
        Some(h) => tauri::async_runtime::spawn_blocking(move || h.trials(engine::HISTORY_TRIALS)).await.map_err(|e| e.to_string())??.1,
        None => Vec::new(),
    };
    let e_cores = state.machine.as_ref().map_or(0, |m| u32::from(m.e_cores));
    let lp = lowpower::latest();
    let plan = xmrange_core::optimizer::Limits { threads: max_threads(), e_cores }.plan();
    let newest = recs.iter().filter(|t| t.label.starts_with("sweep-")).max_by(|a, b| a.t_end.total_cmp(&b.t_end)).map(|t| t.label.clone());
    let measured = |id: &str, s: &Setup| {
        recs.iter().any(|t| t.label == id && t.config.setup() == *s && (t.config.low_power_mode.is_none() || lp.is_none() || t.config.low_power_mode == lp))
    };
    let (id, next) = match newest.as_deref().and_then(|id| plan.iter().find(|s| !measured(id, s)).map(|s| (id.to_string(), *s))) {
        Some(found) => found,
        None => (format!("sweep-{}", now_secs() as u64), plan.first().copied().unwrap_or(Setup::threads(1))),
    };
    change_settings(&state, &data_dir(), |settings| {
        settings.optimizer.mode = xmrange_core::optimizer::Mode::Hold;
        settings.optimizer.sweep = Some(xmrange_core::optimizer::SweepRef { id, skipped: false });
    })?;
    start_engine(app, &state, Some(next))
}

fn start_engine(app: AppHandle, state: &AppState, setup: Option<Setup>) -> Result<u64, String> {
    let mut engine_guard = state.engine.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((gen, _)) = engine_guard.as_ref() {
        return Ok(*gen);
    }

    let settings = state.settings.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let wallet = settings.wallet.0.clone().unwrap_or_else(|| DEFAULT_WALLET.to_string());

    #[cfg(feature = "dev-diagnostics")]
    let dev_log = {
        let dir = data_dir().join("logs/xmrig");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        tidy_dev_logs(&dir, DEV_LOGS_KEPT - 1);
        Some(dir.join(format!("{}.log", now_secs() as u64)))
    };
    #[cfg(not(feature = "dev-diagnostics"))]
    let dev_log = None;

    let ledger = state.ledger.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(LedgerWriter::handle);
    // Holding: launch straight onto the held throttle.
    let manual = Setup { threads: settings.threads, e_core_clamp: settings.e_cores_only, utility: settings.utility_manual };
    let launch = setup.unwrap_or_else(|| Optimizer::launch_setup(&settings.optimizer, manual));
    let optimizer = state.machine.as_ref().map(|m| OptimizerInit {
        prefs: settings.optimizer.clone(),
        measure: settings.measure,
        chip: Some(m.chip.clone()),
        e_cores: u32::from(m.e_cores),
        own_threads: Some(settings.threads),
    });
    let cfg = EngineConfig {
        wallet, worker: settings.worker.clone(), threads: launch.threads, e_core_clamp: launch.e_core_clamp, utility: launch.utility,
        dev_log, ledger, optimizer, pool: settings.pool, chain: settings.p2pool_chain, node: settings.node, tor: settings.tor, guard: settings.guard,
    };
    let gen = state.engine_gen.fetch_add(1, Ordering::Relaxed) + 1;
    let (ring_private, ring_home) = report_private(&settings);
    xmrange_core::report::ring_scrubber(ring_private, ring_home);
    let (ctl, rx) = engine::spawn(cfg);
    *engine_guard = Some((gen, ctl));
    drop(engine_guard);

    thread::spawn(move || {
        for snap in rx {
            for l in &snap.tape {
                xmrange_core::report::ring_push(&format!("{} {} {}", l.ts, l.tag, l.msg));
            }
            if let Some(p) = &snap.prefs {
                save_prefs(&app.state::<AppState>(), p);
            }
            if app.emit("xmrange://snapshot", &snap).is_err() {
                break;
            }
        }
        // The engine ended, on its own (xmrig exited or failed to launch) or via stop_mining.
        // Release the slot if it is still ours, so the next start_mining really starts.
        let dead = take_if_gen(&app.state::<AppState>().engine, gen);
        drop(dead); // joins the already-finished engine thread, outside the lock
        let _ = app.emit("xmrange://stopped", gen);
    });

    Ok(gen)
}

/// Stops the engine on a blocking thread. While it runs, the stop is "in flight": a quit that
/// arrives meanwhile waits for it (`stop_engine_for_exit`).
#[tauri::command]
async fn stop_mining(state: State<'_, AppState>) -> Result<(), String> {
    // Stopping mining during trials is stopping trials: same state, whichever button was pressed.
    end_trials_run(&state);
    if let Some(stop) = begin_stop(&state) {
        tauri::async_runtime::spawn_blocking(move || stop.run()).await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn pause_mining(state: State<AppState>) -> Result<(), String> {
    if let Some((_, ctl)) = state.engine.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        ctl.send(Command::Pause).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn resume_mining(state: State<AppState>) -> Result<(), String> {
    if let Some((_, ctl)) = state.engine.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        ctl.send(Command::Resume).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn set_threads(state: State<AppState>, threads: u32) -> Result<(), String> {
    set_threads_in(&state, &data_dir(), threads)
}

/// `set_threads` with its settings folder passed in, so a test drives the whole command (F6).
fn set_threads_in(state: &AppState, dir: &Path, threads: u32) -> Result<(), String> {
    validate_threads(threads, max_threads()).map_err(|e| e.to_string())?;
    change_settings(&state, dir, |settings| {
        settings.threads = threads;
        // The engine's hand change lets the clamp go (OptCommand::ManualThreads); the cores card
        // steps through `apply_pick` while it stays on the E-cores.
        settings.e_cores_only = false;
        settings.utility_manual = false;
        // Hand input: even a stopped app must not launch next time pulling the levers against it.
        to_manual(&mut settings.optimizer);
    })?;
    send_if_running(state, Command::SetThreads(threads));
    Ok(())
}

/// A profile pill (controller ruling Q4): the intent to optimize. The engine holds the
/// profile's winner, or seeks one if it has no valid trials. Stopped, the next start does.
/// Async, with the settings lock and write on a blocking thread, never the main one (P4-C19).
#[tauri::command]
async fn set_profile(app: AppHandle, profile: Profile) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || set_profile_in(&app.state::<AppState>(), &data_dir(), profile)).await.map_err(|e| e.to_string())?
}

fn set_profile_in(state: &AppState, dir: &Path, profile: Profile) -> Result<(), String> {
    optimizer_available(state)?;
    change_settings(&state, dir, |s| {
        s.optimizer.profile = profile;
        s.optimizer.mode = Mode::Hold;
        let stamp = chrono_local_stamp();
        s.profile_log.push(format!("{stamp} · {} · pill", xmrange_core::optimizer::profile_name(profile)));
        let extra = s.profile_log.len().saturating_sub(20);
        s.profile_log.drain(..extra);
    })?;
    send_if_running(state, Command::Optimizer(OptCommand::SetProfile(profile)));
    Ok(())
}

#[tauri::command]
fn seek(state: State<AppState>) -> Result<(), String> {
    optimizer_command(&state, OptCommand::Seek)
}

#[tauri::command]
async fn stop_seek(state: State<'_, AppState>) -> Result<(), String> {
    // Stop trials = stop mining; the trials measured so far stay, and "run trials" continues.
    end_trials_run(&state);
    if let Some(stop) = begin_stop(&state) {
        tauri::async_runtime::spawn_blocking(move || stop.run()).await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Ends a trials run (if any) before mining stops: the sweep ref leaves settings, so the next
/// plain start mines normally and "run trials" continues after the last measured trial. A failed
/// write never blocks the stop that follows: the run still ends, and the error goes to stderr.
fn end_trials_run(state: &AppState) {
    let _ = optimizer_command(state, OptCommand::StopSeek);
    let mut settings = state.settings.lock().unwrap_or_else(|e| e.into_inner());
    if settings.optimizer.sweep.take().is_some() {
        if let Err(e) = settings.save(&data_dir()) {
            eprintln!("settings: {e}");
        }
    }
}

#[tauri::command]
fn skip_low_power(state: State<AppState>) -> Result<(), String> {
    optimizer_command(&state, OptCommand::SkipLowPower)
}

/// A trials-card row, clicked: run that exact measured setup and switch to manual (P5 "pick a
/// result"). If it was measured in the other Low Power Mode state, this still applies the threads
/// and the clamp (both independent of it) and shows the usual ask instead of doing nothing.
/// Stopped, the setup is saved and the next start runs it (owner, 2026-10-04: a click that only
/// said "start mining to run trials" looked broken).
#[tauri::command]
fn apply_pick(state: State<AppState>, threads: u32, e_core_clamp: bool, utility: Option<bool>, low_power: Option<bool>) -> Result<(), String> {
    let utility = utility.unwrap_or(false);
    validate_threads(threads, max_threads()).map_err(|e| e.to_string())?;
    change_settings(&state, &data_dir(), |settings| {
        settings.threads = threads;
        settings.e_cores_only = e_core_clamp;
        settings.utility_manual = utility;
        to_manual(&mut settings.optimizer);
    })?;
    if state.engine.lock().unwrap_or_else(|e| e.into_inner()).is_none() {
        return Ok(()); // saved: the next start runs it
    }
    optimizer_command(&state, OptCommand::ApplySetup { setup: Setup { threads, e_core_clamp, utility }, low_power })
}

/// The Battery pane's URL, then the legacy Energy Saver pane as a fallback (older macOS, or a
/// desktop Mac with no Battery pane).
fn battery_settings_urls() -> [&'static str; 2] {
    ["x-apple.systempreferences:com.apple.Battery-Settings.extension", "x-apple.systempreferences:com.apple.preference.energysaver"]
}

/// Opens System Settings at a fixed URL: `/usr/bin/open <url>` only, never AppleScript or Apple
/// Events (Rule #1). Tries each pane in order, stopping at the first `open` exits zero on.
/// Self-check mode (dev builds, `XMRANGE_SELF_CHECK=1`) never really shells out, same as every
/// other test-only gate in this file (`self_check_mode`, `autostart_mode`): the self-check proves
/// the button is wired to this command without touching Settings.
#[tauri::command]
fn open_battery_settings() -> Result<(), String> {
    if self_check_mode() {
        return Ok(());
    }
    let mut last_err = String::new();
    for url in battery_settings_urls() {
        match std::process::Command::new("/usr/bin/open").arg(url).status() {
            Ok(status) if status.success() => return Ok(()),
            Ok(status) => last_err = format!("open exited with {status}"),
            Err(e) => last_err = e.to_string(),
        }
    }
    Err(format!("could not open Settings: {last_err}"))
}

#[tauri::command]
fn set_worker(state: State<AppState>, worker: String) -> Result<(), String> {
    validate_worker(&worker).map_err(|e| e.to_string())?;
    change_settings(&state, &data_dir(), |settings| {
        settings.worker = worker.clone();
    })?;
    // The identity is read and sent under the engine lock, so the last identity sent always
    // matches the latest worker/wallet pair, whichever command set it last.
    let engine = state.engine.lock().unwrap_or_else(|e| e.into_inner());
    let wallet = state.settings.lock().unwrap_or_else(|e| e.into_inner()).wallet.0.clone().unwrap_or_else(|| DEFAULT_WALLET.to_string());
    if let Some((_, ctl)) = engine.as_ref() {
        let _ = ctl.send(Command::SetIdentity { wallet, worker });
    }
    Ok(())
}

/// p2pool reads the wallet once, when it starts (it pays the wallet in its shares), so a wallet
/// change while p2pool mines would show the new wallet and keep paying the old one. Refused
/// instead, before anything is saved. MoneroOcean takes a live change (xmrig's login).
const P2POOL_WALLET_LOCKED: &str = "stop mining to change the wallet · p2pool reads it only at start";

fn p2pool_wallet_unlocked(state: &AppState) -> Result<(), String> {
    let p2pool = state.settings.lock().unwrap_or_else(|e| e.into_inner()).pool == xmrange_core::settings::Pool::P2pool;
    if p2pool && !engine_idle(state) {
        return Err(P2POOL_WALLET_LOCKED.into());
    }
    Ok(())
}

/// Saves the user's wallet in `settings.json`, like `set_worker` saves the worker: checksum
/// first, then a plain, synchronous write — there is no dialog to wait on (owner decision
/// 2026-09-28: no separate secret store, no OS prompt, ever).
#[tauri::command]
fn set_wallet(state: State<AppState>, address: String) -> Result<WalletView, String> {
    xmrange_core::wallet::check_address(&address).map_err(|e| e.to_string())?;
    p2pool_wallet_unlocked(&state)?;
    change_settings(&state, &data_dir(), |settings| {
        settings.wallet = Wallet(Some(address.clone()));
    })?;
    let engine = state.engine.lock().unwrap_or_else(|e| e.into_inner());
    let worker = state.settings.lock().unwrap_or_else(|e| e.into_inner()).worker.clone();
    if let Some((_, ctl)) = engine.as_ref() {
        let _ = ctl.send(Command::SetIdentity { wallet: address.clone(), worker });
    }
    Ok(wallet_view(Some(&address)))
}

/// Back to the disclosed default.
#[tauri::command]
fn clear_wallet(state: State<AppState>) -> Result<WalletView, String> {
    p2pool_wallet_unlocked(&state)?;
    change_settings(&state, &data_dir(), |settings| {
        settings.wallet = Wallet(None);
    })?;
    let engine = state.engine.lock().unwrap_or_else(|e| e.into_inner());
    let worker = state.settings.lock().unwrap_or_else(|e| e.into_inner()).worker.clone();
    if let Some((_, ctl)) = engine.as_ref() {
        let _ = ctl.send(Command::SetIdentity { wallet: DEFAULT_WALLET.to_string(), worker });
    }
    Ok(wallet_view(None))
}

/// The throttles selected from the same trials, on each denominator ("h/w counts").
#[derive(Debug, Default, Serialize)]
struct PicksBy {
    sys: Picks,
    cpu: Picks,
}

/// The efficient throttle's measured H/W with Low Power Mode on vs off, for the machine tab's low
/// power card: never a hard-coded figure, only this Mac's own trials (Helios, "we don't want to
/// hard code my own computer into the app").
#[derive(Debug, Clone, Copy, Serialize)]
struct LpGain {
    off: f64,
    on: f64,
    pct: f64,
}

#[derive(Debug, Default, Serialize)]
struct LpGainBy {
    sys: Option<LpGain>,
    cpu: Option<LpGain>,
}

/// The efficient pick's exact setup, measured in both Low Power Mode states: `None` until both
/// exist.
fn lp_gain(measured: &[Measured], picks: &Picks, m: Measure) -> Option<LpGain> {
    let setup = picks.efficient.as_ref()?.setup;
    let hw_at = |lp: bool| measured.iter().find(|x| x.setup == setup && x.low_power == Some(lp)).and_then(|x| hw(&x.score, m));
    let off = hw_at(false)?;
    let on = hw_at(true)?;
    (off > 0.0).then(|| LpGain { off, on, pct: (on - off) / off * 100.0 })
}

#[derive(Debug, Default, Serialize)]
struct TrialsReply {
    #[serde(flatten)]
    view: TrialsView,
    picks: PicksBy,
    low_power_gain: LpGainBy,
    /// The newest sweep's progress in the current power mode: `[done, planned]`, for
    /// "continue trials · 4 of 12" while nothing mines. `None` with no sweep yet.
    progress: Option<[u32; 2]>,
}

/// `[done, planned]` for the newest sweep in `recs` (the same rule "run trials" continues by).
fn sweep_progress(recs: &[xmrange_core::ledger::TrialRecord], e_cores: u32, lp: Option<bool>) -> Option<[u32; 2]> {
    let newest = recs.iter().filter(|t| t.label.starts_with("sweep-")).max_by(|a, b| a.t_end.total_cmp(&b.t_end))?;
    let plan = xmrange_core::optimizer::Limits { threads: max_threads(), e_cores }.plan();
    let done = plan
        .iter()
        .filter(|s| {
            recs.iter().any(|t| {
                t.label == newest.label
                    && t.config.setup() == **s
                    && (t.config.low_power_mode.is_none() || lp.is_none() || t.config.low_power_mode == lp)
            })
        })
        .count() as u32;
    Some([done, plan.len() as u32])
}

/// Every name that could identify the person or the Mac, from the Mac itself, and the home folder.
fn report_private(settings: &Settings) -> (Vec<String>, Option<String>) {
    let home = std::env::var("HOME").ok();
    let mut private: Vec<String> = ["ComputerName", "LocalHostName", "HostName"]
        .iter()
        .filter_map(|k| std::process::Command::new("/usr/sbin/scutil").args(["--get", k]).output().ok())
        .filter_map(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .collect();
    private.extend(std::env::var("USER").ok());
    private.extend(home.as_deref().and_then(|h| Path::new(h).file_name()).map(|n| n.to_string_lossy().into_owned()));
    if settings.worker != Settings::default().worker {
        private.push(settings.worker.clone());
    }
    (private, home)
}

/// The scrubbed diagnostics text, with the words and home folder that scrubbing used (so a report
/// can scrub the user's own note the same way). Blocking: it reads the Mac and the Ledger.
fn diagnostics_text(settings: &Settings, chip: Option<String>, ledger: Option<xmrange_core::ledger::LedgerHandle>, tape: &[String], hours: &[String]) -> (String, Vec<String>, Option<String>) {
    let macos = std::process::Command::new("/usr/bin/sw_vers").arg("-productVersion").output().ok()
        .and_then(|o| String::from_utf8(o.stdout).ok()).map(|s| s.trim().to_string());
    let p2pool = settings.pool == xmrange_core::settings::Pool::P2pool;
    let (private, home) = report_private(settings);
    let facts = xmrange_core::diagnostics::Facts {
        version: env!("CARGO_PKG_VERSION"),
        macos,
        chip: chip.as_deref(),
        model: std::process::Command::new("/usr/sbin/sysctl").args(["-n", "hw.model"]).output().ok()
            .and_then(|o| String::from_utf8(o.stdout).ok()).map(|s| s.trim().to_string()),
        pool: if p2pool { "p2pool" } else { "moneroocean" },
        tor: settings.tor,
        node: p2pool.then(|| xmrange_core::node::probe(settings.node)),
        cost_inputs: format!("{} {}/kwh · period {:?}", settings.rate_per_kwh, settings.currency, settings.period).to_lowercase(),
        running: match (settings.optimizer.mode, &settings.optimizer.holding) {
            (Mode::Hold, Some(p)) => p.setup.label(),
            _ => Setup { threads: settings.threads, e_core_clamp: settings.e_cores_only, utility: settings.utility_manual }.label(),
        },
        optimizer_mode: &format!("{:?}", settings.optimizer.mode).to_lowercase(),
        trials: ledger.as_ref().and_then(|h| h.trials(1).ok().map(|(n, _)| n)),
        low_power: lowpower::latest(),
        tape,
        p2pool_output: &if p2pool { xmrange_core::p2pool::recent_output() } else { Vec::new() },
        // The wallet is only compared against, never written into the report.
        trials_table: &ledger
            .as_ref()
            .and_then(|h| h.trials(engine::HISTORY_TRIALS).ok())
            .map(|(_, recs)| xmrange_core::diagnostics::trial_lines(&settings.optimizer, if settings.measure == xmrange_core::settings::Measure::Cpu { "cpu" } else { "whole mac" }, &recs, &settings.profile_log))
            .unwrap_or_default(),
        hours,
        privacy: &xmrange_core::audit::report(&resolve_wallet(settings.wallet.0.as_deref()).0, xmrange_core::config::API_PORT),
        private: &private,
        home: home.as_deref(),
    };
    let text = xmrange_core::diagnostics::report(&facts);
    (text, private, home)
}

/// ⚙️ save diagnostics: a scrubbed plain-text report (xmrange-core `diagnostics`: no wallet, no
/// IPs, no user, computer or worker name), written only when the user presses the button, to the
/// `.txt` file they picked in the save dialog. Nothing is sent anywhere. Returns the file's name.
#[tauri::command]
async fn save_diagnostics(state: State<'_, AppState>, tape: Vec<String>, hours: Vec<String>, path: String) -> Result<String, String> {
    let target = txt_target(&path)?;
    let (settings, chip, ledger) = report_inputs(&state);
    tauri::async_runtime::spawn_blocking(move || {
        let (text, _, _) = diagnostics_text(&settings, chip, ledger, &tape, &hours);
        std::fs::write(&target, text).map_err(|e| e.to_string())?;
        Ok(target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
    })
    .await
    .map_err(|e| e.to_string())?
}

fn txt_target(path: &str) -> Result<std::path::PathBuf, String> {
    let target = std::path::PathBuf::from(path);
    if target.extension().and_then(|e| e.to_str()) != Some("txt") || !target.parent().is_some_and(Path::is_dir) {
        return Err("reports save as a .txt file in an existing folder".into());
    }
    Ok(target)
}

fn report_inputs(state: &AppState) -> (Settings, Option<String>, Option<xmrange_core::ledger::LedgerHandle>) {
    (
        state.settings.lock().unwrap_or_else(|e| e.into_inner()).clone(),
        state.machine.as_ref().map(|m| m.chip.clone()),
        state.ledger.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(LedgerWriter::handle),
    )
}

/// The report the user reads before anything is sent: header, their note, the scrubbed diagnostics.
#[tauri::command]
async fn report_text(state: State<'_, AppState>, kind: String, note: String, tape: Vec<String>, hours: Vec<String>) -> Result<String, String> {
    if !xmrange_core::report::KINDS.contains(&kind.as_str()) {
        return Err("a report is a problem, an idea or a crash".into());
    }
    let (settings, chip, ledger) = report_inputs(&state);
    tauri::async_runtime::spawn_blocking(move || {
        let (mut diag, private, home) = diagnostics_text(&settings, chip, ledger, &tape, &hours);
        if kind == "crash" {
            diag.push_str(&format!("\n{}", xmrange_core::report::before_the_crash(&data_dir())));
        }
        let macos = std::process::Command::new("/usr/bin/sw_vers").arg("-productVersion").output().ok()
            .and_then(|o| String::from_utf8(o.stdout).ok()).map(|s| s.trim().to_string()).unwrap_or_else(|| "unknown".into());
        let arch = if cfg!(target_arch = "aarch64") { "apple silicon" } else { "intel" };
        Ok(xmrange_core::report::build(&kind, env!("CARGO_PKG_VERSION"), &macos, arch, &note, &diag, &private, home.as_deref()))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The crash file has done its job (sent, closed or dismissed): delete it.
#[tauri::command]
fn discard_crash_file() {
    let _ = std::fs::remove_file(data_dir().join(xmrange_core::report::CRASH_FILE));
}

/// Saves the (re-scrubbed) report text to the `.txt` file the user picked. Returns the file's name.
#[tauri::command]
async fn save_report(state: State<'_, AppState>, text: String, path: String) -> Result<String, String> {
    let target = txt_target(&path)?;
    let settings = state.settings.lock().unwrap_or_else(|e| e.into_inner()).clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (private, home) = report_private(&settings);
        std::fs::write(&target, xmrange_core::report::clean(&text, &private, home.as_deref())).map_err(|e| e.to_string())?;
        Ok(target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Sends one report, only when the user presses send: the text is scrubbed again (they may have
/// edited it), then `house-report` seals it for XMRange's key and posts it to the Nostr relays
/// through the shared Arti, whatever the mining mode. Returns house-report's `{sent, relays}`.
#[tauri::command]
async fn send_report(state: State<'_, AppState>, kind: String, text: String) -> Result<serde_json::Value, String> {
    if self_check_mode() {
        return Err("self-check never sends".into());
    }
    if !xmrange_core::report::KINDS.contains(&kind.as_str()) {
        return Err("a report is a problem, an idea or a crash".into());
    }
    let settings = state.settings.lock().unwrap_or_else(|e| e.into_inner()).clone();
    tauri::async_runtime::spawn_blocking(move || {
        use std::io::Write;
        let (private, home) = report_private(&settings);
        let cleaned = xmrange_core::report::clean(&text, &private, home.as_deref());
        // Never silent: if scrubbing changes the box, the UI shows the scrubbed text and asks again.
        if cleaned != text {
            return Err(format!("{RESCRUBBED}{cleaned}"));
        }
        let _tor = xmrange_core::tor::acquire(&|| false).map_err(|e| format!("tor did not start: {e}"))?;
        let mut child = std::process::Command::new(xmrange_core::paths::house_report_binary())
            .args(["--to", xmrange_core::report::REPORT_PUBKEY, "--app", "xmrange", "--version", env!("CARGO_PKG_VERSION"), "--kind", &kind, "--socks", &xmrange_core::tor::socks_addr()])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("house-report did not start: {e}"))?;
        child.stdin.take().ok_or("house-report has no input")?.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err("no relay took the report".to_string());
        }
        serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Puts `text` on the clipboard (the node guide's copy button): macOS's own pbcopy, which needs no
/// permission and shows no prompt. The webview can't select or copy text itself. A self-check
/// run never touches the owner's clipboard.
#[tauri::command]
async fn copy_text(text: String) -> Result<(), String> {
    if self_check_mode() {
        return Ok(());
    }
    tauri::async_runtime::spawn_blocking(move || {
        use std::io::Write;
        let mut child = std::process::Command::new("/usr/bin/pbcopy").stdin(std::process::Stdio::piped()).spawn().map_err(|e| e.to_string())?;
        child.stdin.take().ok_or("pbcopy has no input")?.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
        match child.wait().map_err(|e| e.to_string())?.success() {
            true => Ok(()),
            false => Err("copy failed".into()),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The Network Log (doctrine §2): what XMRange's processes are connected to right now (counts and
/// ports, never an address) and what this setup is allowed to reach.
#[derive(Debug, Clone, Serialize)]
struct NetworkLog {
    now: Vec<String>,
    allowed: Vec<String>,
}

fn allowed_now(s: &Settings) -> Vec<String> {
    let via = if s.tor { "through tor" } else { "directly" };
    let mut out = vec![match s.pool {
        xmrange_core::settings::Pool::Mo => {
            let host = xmrange_core::config::DEFAULT_POOL.split(':').next().unwrap_or("moneroocean");
            format!("moneroocean pool ({host}), {via}")
        }
        xmrange_core::settings::Pool::P2pool if s.node == xmrange_core::settings::NodeChoice::Own => {
            format!("p2pool peers and monero peers (xmrange's own node), {via}")
        }
        xmrange_core::settings::Pool::P2pool => format!("p2pool peers, {via} · your monero node on this mac"),
    }];
    if s.tor {
        out.push("the tor network (tor relays only)".into());
    }
    if s.price_feed && s.currency == "$" {
        out.push("kraken, then yahoo finance: the xmr price, hourly, through tor".into());
    }
    out.push("reports: only when you press send, through tor (4 nostr relays)".into());
    out.push(if cfg!(feature = "dev-diagnostics") {
        "updates: none (dev build)".into()
    } else {
        "updates: github (house-of-helios/xmrange), at launch and every 6 h, through tor".into()
    });
    out
}

#[tauri::command]
async fn network_log(state: State<'_, AppState>) -> Result<NetworkLog, String> {
    let allowed = allowed_now(&state.settings.lock().unwrap_or_else(|e| e.into_inner()));
    let now = tauri::async_runtime::spawn_blocking(xmrange_core::audit::network_now).await.map_err(|e| e.to_string())?;
    Ok(NetworkLog { now, allowed })
}

/// All-time p2pool payouts to the saved wallet, from the Ledger (none without a Ledger).
#[tauri::command]
async fn payout_totals(state: State<'_, AppState>) -> Result<xmrange_core::ledger::PayoutTotals, String> {
    let handle = state.ledger.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(LedgerWriter::handle);
    let Some(h) = handle else { return Ok(Default::default()) };
    let (wallet, _) = resolve_wallet(state.settings.lock().unwrap_or_else(|e| e.into_inner()).wallet.0.as_deref());
    tauri::async_runtime::spawn_blocking(move || h.payouts(wallet)).await.map_err(|e| e.to_string())?
}

/// Local wall time, "2026-10-03 21:14:05", from the system's own `date` (no time-zone crate).
fn chrono_local_stamp() -> String {
    std::process::Command::new("/bin/date")
        .arg("+%Y-%m-%d %H:%M:%S")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| format!("{:.0}", xmrange_core::telemetry::now_secs()))
}

/// The user's own Monero node (p2pool mode): one look at 127.0.0.1, off the main thread.
#[tauri::command]
async fn node_status(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let choice = state.settings.lock().unwrap_or_else(|e| e.into_inner()).node;
    let own = OWN_NODE.lock().unwrap_or_else(|e| e.into_inner()).clone();
    // The own node, before monerod answers (or after it stopped): the keeper's words.
    if choice == xmrange_core::settings::NodeChoice::Own && own != OwnNode::Running {
        return serde_json::to_value(own).map_err(|e| e.to_string());
    }
    let probed = tauri::async_runtime::spawn_blocking(move || xmrange_core::node::probe(choice)).await.map_err(|e| e.to_string())?;
    let mut v = serde_json::to_value(probed).map_err(|e| e.to_string())?;
    // Running but not answering yet: monerod opens its database first.
    if choice == xmrange_core::settings::NodeChoice::Own && v["state"] == "missing" {
        v = serde_json::json!({ "state": "starting" });
    }
    Ok(v)
}

/// The tuning tab's trials card, curve and throttle pills: real measured setups from the Ledger.
/// Async + spawn_blocking: a Ledger read never runs on the main thread (Tauri runs sync commands there).
#[tauri::command]
async fn list_trials(state: State<'_, AppState>) -> Result<TrialsReply, String> {
    let handle = state.ledger.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(LedgerWriter::handle);
    let Some(h) = handle else { return Ok(TrialsReply::default()) };
    let chip = state.machine.as_ref().map(|m| m.chip.clone());
    let e_cores = state.machine.as_ref().map_or(0, |m| u32::from(m.e_cores));
    tauri::async_runtime::spawn_blocking(move || {
        h.trials(engine::HISTORY_TRIALS).map(|(n, recs)| {
            let progress = sweep_progress(&recs, e_cores, lowpower::latest());
            let measured = usable(&recs, chip.as_deref());
            let picks = PicksBy { sys: picks(&measured, Measure::Sys, max_threads()), cpu: picks(&measured, Measure::Cpu, max_threads()) };
            let low_power_gain = LpGainBy { sys: lp_gain(&measured, &picks.sys, Measure::Sys), cpu: lp_gain(&measured, &picks.cpu, Measure::Cpu) };
            TrialsReply { view: TrialsView { curve: curve_view(&recs, lowpower::latest()), ..trial_views(n, &recs) }, picks, low_power_gain, progress }
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// ⚙️ "clear trials": deletes every trial so a sweep starts from trial 1. Refused while mining
/// (a running sweep would write into it); the UI confirms first.
#[tauri::command]
async fn clear_trials(state: State<'_, AppState>) -> Result<(), String> {
    if state.engine.lock().unwrap_or_else(|e| e.into_inner()).is_some() {
        return Err("stop mining first".into());
    }
    let handle = state.ledger.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(LedgerWriter::handle);
    let Some(h) = handle else { return Err("no trials store on this mac".into()) };
    change_settings(&state, &data_dir(), |settings| {
        settings.optimizer.sweep = None;
        settings.optimizer.holding = None;
    })?;
    tauri::async_runtime::spawn_blocking(move || h.clear_trials()).await.map_err(|e| e.to_string())?
}

/// Dev xmrig logs kept in `<data dir>/logs/xmrig/`, counting the one about to be written (M13).
#[cfg(feature = "dev-diagnostics")]
const DEV_LOGS_KEPT: usize = 5;

/// Dev builds only: keep the newest `keep` of the app's own xmrig logs (`<unix secs>.log`) and
/// delete the rest. Other files in the folder (CLI runs, smoke logs) are left alone. It only
/// deletes: redacting the kept logs a killed app left raw (M13) is `miner::launch`'s
/// `sweep_dev_logs`, which runs on this same folder once the API port is free (pre-flight F7).
#[cfg(feature = "dev-diagnostics")]
fn tidy_dev_logs(dir: &Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut logs: Vec<(u64, std::path::PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let stamp = name.strip_suffix(".log")?.parse::<u64>().ok()?;
            Some((stamp, e.path()))
        })
        .collect();
    logs.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, path) in logs.iter().skip(keep) {
        let _ = std::fs::remove_file(path);
    }
}

/// Dev builds only: `XMRANGE_DEV_STOP_QUIT_AFTER=<secs>` (with autostart) clicks stop and quits in
/// the same instant, `secs` after launch, for `tools/check-app-lifecycle.sh`'s I3 round.
#[cfg(feature = "dev-diagnostics")]
fn dev_stop_quit_after() -> Option<Duration> {
    let secs = std::env::var("XMRANGE_DEV_STOP_QUIT_AFTER").ok()?.parse::<u64>().ok()?;
    autostart_mode().then(|| Duration::from_secs(secs))
}

/// The I3 test hook: a stop begins on its own thread (as `stop_mining` does) and the app quits
/// without waiting for it. Only `stop_engine_for_exit`'s wait stands between that and an orphan.
#[cfg(feature = "dev-diagnostics")]
fn spawn_stop_then_quit(handle: AppHandle, after: Duration) {
    thread::spawn(move || {
        thread::sleep(after);
        if let Some(stop) = begin_stop(&handle.state::<AppState>()) {
            thread::spawn(move || stop.run());
            println!("DEV stop-then-quit: stop in flight, quitting now");
        }
        handle.exit(0);
    });
}

/// What the idle sampler emits on `xmrange://idle`: whole-Mac telemetry while nothing mines.
#[derive(Clone, Serialize)]
struct IdleSample {
    t: f64,
    power: Option<PowerSample>,
    context: Option<Context>,
}

/// Idle cadence (I4): one 500 ms macmon sample every 2 s, slower than the engine's 1 s.
const IDLE_GAP: Duration = Duration::from_millis(1500);
const IDLE_SAMPLE_MS: u32 = 500;
/// Context (pmset/ioreg shell-outs) rides on the first idle sample and then every 15th (~30 s).
const IDLE_CONTEXT_EVERY: u64 = 15;

/// Whether anyone can see the window: it exists, is visible (not hidden with ⌘H or closed) and is
/// not minimized. Background mode (dev test runs, P2-R10) counts as shown, so the self-check can
/// prove the idle telemetry with its hidden window. Always a real check in release.
fn window_shown(handle: &AppHandle) -> bool {
    if background_mode() {
        return true;
    }
    match handle.get_webview_window("main") {
        Some(w) => w.is_visible().unwrap_or(true) && !w.is_minimized().unwrap_or(false),
        None => false,
    }
}

/// Final review I4: the machine tab is live while nothing mines. macmon is sampled here, in the
/// app's own process, only while no engine runs and none is stopping, so it never overlaps the
/// engine's sampler: a start fills the slot before its engine exists, and the engine's first
/// sample waits for xmrig's API (seconds), far longer than one 500 ms idle sample. It also rests
/// while the window can't be seen.
/// XMRange's own Monero node, as the keeper last left it (the node card's words come from here
/// until monerod answers its RPC; then from the probe).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum OwnNode {
    /// Not wanted now: the GUI node is chosen, or the pool is MoneroOcean (doctrine §1a).
    Off,
    /// Chosen, but no chain folder yet.
    NoFolder,
    Starting,
    Running,
    Stopping,
    Failed { why: String },
}

static OWN_NODE: Mutex<OwnNode> = Mutex::new(OwnNode::Off);
/// Set at quit: the keeper stops the node and ends.
static NODE_QUIT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// A node that failed waits this long before the keeper tries again (a loop of failures would
/// thrash an external drive).
const NODE_RETRY: Duration = Duration::from_secs(60);

fn set_own(s: OwnNode) {
    *OWN_NODE.lock().unwrap_or_else(|e| e.into_inner()) = s;
}

/// What the keeper wants: the own node runs while p2pool is the selected pool (outbound follows
/// the mode, doctrine §1a), the own node is chosen, a folder is set and the app isn't quitting.
/// The folder and Tor are part of it: a change restarts the node.
fn node_wanted(s: &Settings) -> Option<(String, bool)> {
    let on = s.pool == xmrange_core::settings::Pool::P2pool && s.node == xmrange_core::settings::NodeChoice::Own;
    (on && !NODE_QUIT.load(Ordering::SeqCst)).then(|| s.node_dir.clone().map(|d| (d, s.tor))).flatten()
}

/// The own node's keeper: one thread, every 2 s. It starts monerod (through the shared Tor when
/// Tor is on), stops it when it is no longer wanted, restarts it on a folder or Tor change, and
/// notices monerod ending or its drive going away. Quit sets `NODE_QUIT`; the keeper stops the
/// node (up to monerod's 2-minute grace) and ends.
fn spawn_node_keeper(handle: AppHandle) {
    thread::spawn(move || {
        // Node first, then the lease: monerod stops before its Tor does.
        let mut running: Option<((String, bool), xmrange_core::monerod::Node, Option<xmrange_core::tor::Lease>)> = None;
        let mut failed_at: Option<Instant> = None;
        loop {
            let settings = handle.state::<AppState>().settings.lock().unwrap_or_else(|e| e.into_inner()).clone();
            let want = node_wanted(&settings);
            if let Some((key, node, _)) = running.as_mut() {
                let gone = if !node.folder_present() {
                    Some("the chain folder is gone · reconnect its drive, then choose the folder again".to_string())
                } else {
                    node.exited().map(|why| format!("monerod stopped: {why}"))
                };
                if gone.is_some() || want.as_ref() != Some(key) {
                    set_own(OwnNode::Stopping);
                    drop(running.take());
                    match gone {
                        Some(why) => {
                            failed_at = Some(Instant::now());
                            set_own(OwnNode::Failed { why });
                        }
                        None => set_own(OwnNode::Off),
                    }
                }
            }
            if running.is_none() {
                match want {
                    Some((dir, tor)) if failed_at.is_none_or(|t| t.elapsed() >= NODE_RETRY) => {
                        set_own(OwnNode::Starting);
                        let wanted = (dir.clone(), tor);
                        let still = || {
                            NODE_QUIT.load(Ordering::SeqCst)
                                || node_wanted(&handle.state::<AppState>().settings.lock().unwrap_or_else(|e| e.into_inner())) != Some(wanted.clone())
                        };
                        let lease = if tor { xmrange_core::tor::acquire(&still).map(Some) } else { Ok(None) };
                        let started = lease.and_then(|l| {
                            let socks = l.as_ref().map(|_| xmrange_core::tor::socks_addr());
                            xmrange_core::monerod::Node::start(Path::new(&dir), socks.as_deref()).map(|n| (n, l))
                        });
                        match started {
                            Ok((n, l)) => {
                                failed_at = None;
                                set_own(OwnNode::Running);
                                running = Some(((dir, tor), n, l));
                            }
                            Err(e) => {
                                failed_at = Some(Instant::now());
                                set_own(OwnNode::Failed { why: e.to_string() });
                            }
                        }
                    }
                    Some(_) => {} // waiting out NODE_RETRY after a failure; the card says why
                    None => {
                        let chosen = settings.pool == xmrange_core::settings::Pool::P2pool && settings.node == xmrange_core::settings::NodeChoice::Own;
                        failed_at = None;
                        set_own(if chosen && settings.node_dir.is_none() && !NODE_QUIT.load(Ordering::SeqCst) { OwnNode::NoFolder } else { OwnNode::Off });
                    }
                }
            }
            if NODE_QUIT.load(Ordering::SeqCst) && running.is_none() {
                set_own(OwnNode::Off);
                return;
            }
            thread::sleep(Duration::from_secs(2));
        }
    });
}

/// At quit, after the engine: the keeper stops the own node (monerod closes its database) and
/// this waits for it, at most monerod's grace plus a little.
fn stop_node_for_exit() {
    NODE_QUIT.store(true, Ordering::SeqCst);
    let deadline = Instant::now() + xmrange_core::monerod::STOP_GRACE + Duration::from_secs(10);
    while Instant::now() < deadline {
        if *OWN_NODE.lock().unwrap_or_else(|e| e.into_inner()) == OwnNode::Off {
            return;
        }
        thread::sleep(Duration::from_millis(200));
    }
}

fn spawn_idle_sampler(handle: AppHandle) {
    thread::spawn(move || {
        // Not Send: created and used on this thread only.
        let Ok((mut source, _)) = xmrange_core::telemetry::open_source() else { return };
        let mut tick: u64 = 0;
        loop {
            thread::sleep(IDLE_GAP);
            let state = handle.state::<AppState>();
            if !engine_idle(&state) {
                tick = 0; // fresh context as soon as the engine is gone
                continue;
            }
            if !window_shown(&handle) {
                continue;
            }
            let power = source.sample(IDLE_SAMPLE_MS).ok();
            let context = (tick % IDLE_CONTEXT_EVERY == 0).then(read_context);
            if !engine_idle(&state) {
                continue; // a start raced this sample: the engine's snapshots own the tab now
            }
            tick += 1;
            if handle.emit("xmrange://idle", IdleSample { t: now_secs(), power, context }).is_err() {
                break;
            }
        }
    });
}

/// The window's designed size, from tauri.conf.json (the 4-column board at full height).
const WINDOW_WANT: (f64, f64) = (1280.0, 970.0);

/// Final review M10: the launch size is the designed size clamped to the monitor's work area (the
/// screen less the menu bar and Dock), but never below the window's minimum; the position centres
/// it in that area. All values are logical points: `(x, y, w, h)`.
fn fit_window(want: (f64, f64), min: (f64, f64), area: (f64, f64, f64, f64)) -> (f64, f64, f64, f64) {
    let (ax, ay, aw, ah) = area;
    let w = want.0.min(aw).max(min.0);
    let h = want.1.min(ah).max(min.1);
    (ax + ((aw - w) / 2.0).max(0.0), ay + ((ah - h) / 2.0).max(0.0), w, h)
}

/// Size and centre the (still hidden) window on the monitor it opens on.
fn fit_to_work_area(window: &WebviewWindow, min: (f64, f64)) {
    let monitor = match window.current_monitor() {
        Ok(Some(m)) => Some(m),
        _ => window.primary_monitor().ok().flatten(),
    };
    let Some(monitor) = monitor else { return };
    let scale = monitor.scale_factor();
    let area = monitor.work_area();
    let logical = (
        area.position.x as f64 / scale,
        area.position.y as f64 / scale,
        area.size.width as f64 / scale,
        area.size.height as f64 / scale,
    );
    let (x, y, w, h) = fit_window(WINDOW_WANT, min, logical);
    let _ = window.set_size(LogicalSize::new(w, h));
    let _ = window.set_position(LogicalPosition::new(x, y));
}

/// macOS: the window's traffic lights (close, minimize, zoom) show only while the window is key,
/// in both themes. Light mode already made them vanish on an inactive window; dark mode left them
/// greyed out (Helios, 2026-09-27). Every launch, dev and release alike: nothing here is gated.
#[cfg(target_os = "macos")]
mod traffic_lights {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSWindow, NSWindowButton};

    const BUTTONS: [NSWindowButton; 3] = [NSWindowButton::CloseButton, NSWindowButton::MiniaturizeButton, NSWindowButton::ZoomButton];

    /// Shows the three buttons when the window is key, hides them when it isn't. AppKit is
    /// main-thread only: Tauri delivers window events on the main thread, so this runs at once;
    /// from anywhere else it is queued onto the main thread.
    pub fn follow_focus(window: &tauri::Window, key: bool) {
        if MainThreadMarker::new().is_some() {
            apply(window, key);
        } else {
            let w = window.clone();
            let _ = window.run_on_main_thread(move || apply(&w, key));
        }
    }

    /// Only `hidden` changes: the buttons keep their place, their hover and their clicks while
    /// shown, and a hidden button takes no click (a click there only brings the window forward).
    fn apply(window: &tauri::Window, key: bool) {
        let Ok(ptr) = window.ns_window() else { return };
        if ptr.is_null() {
            return;
        }
        // SAFETY: `ns_window()` is tao's NSWindow for this Tauri window, alive as long as the
        // window is, and we are on the main thread (see follow_focus).
        let ns: &NSWindow = unsafe { &*ptr.cast::<NSWindow>() };
        for which in BUTTONS {
            if let Some(button) = ns.standardWindowButton(which) {
                button.setHidden(!key);
            }
        }
    }
}

/// Window events for every window, on every launch: on macOS the traffic lights follow focus.
fn on_window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    #[cfg(target_os = "macos")]
    if let tauri::WindowEvent::Focused(key) = event {
        traffic_lights::follow_focus(window, *key);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (window, event);
}

/// The generated context, its embedded frontend extended by `dev_assets()`.
fn context() -> tauri::Context<tauri::Wry> {
    let mut context = tauri::generate_context!();
    let embedded = context.set_assets(Box::new(FrontendAssets(None)));
    context.set_assets(Box::new(FrontendAssets(Some(embedded))));
    context
}

fn main() -> tauri::Result<()> {
    // M1: one app per data dir. A second copy (another build, `open -n`) leaves at once, before it
    // touches settings, secrets or the run dir. Test runs use their own data dirs, so they never
    // collide with the owner's app.
    let instance_lock = match instance_lock(&data_dir()) {
        Ok(Some(lock)) => Some(lock),
        Ok(None) => {
            eprintln!("XMRange is already running with this data folder; this copy exits");
            return Ok(());
        }
        Err(e) => {
            eprintln!("single-instance lock unavailable ({e}); continuing");
            None
        }
    };
    // I2: a miner that a crashed or force-quit session left running is stopped now (only if it is
    // provably ours and orphaned; see xmrange_core::orphan), before anything else can start one.
    let reap_note = orphan::reap(&data_dir().join("run"), &xmrig_binary()).note();
    if let Some(note) = reap_note {
        eprintln!("{note}");
    }
    // Dev builds only: a self-check that died mid-run left its backup, so put the settings back
    // before loading them; a new self-check run takes a fresh backup.
    #[cfg(feature = "dev-diagnostics")]
    {
        if let Ok(true) = selfcheck_backup::restore(&data_dir()) {
            eprintln!("settings restored from an interrupted self-check");
        }
        if self_check_mode() {
            if let Err(e) = selfcheck_backup::write(&data_dir()) {
                eprintln!("self-check settings backup failed: {e}");
            }
        }
    }
    // The dead-man's switch compares against the parent as it was at launch.
    let start_ppid = std::os::unix::process::parent_id();
    let started = Instant::now();
    #[cfg(feature = "dev-diagnostics")]
    if let Err(e) = xmrange_core::settings::migrate_old_secrets(&data_dir()) {
        eprintln!("dev secrets migration: {e}");
    }
    // The marker lives while the app runs; `stop_engine_for_exit` (every quit path) removes it.
    let crashed_last_time = !self_check_mode() && data_dir().join(RUNNING_MARKER).exists();
    if !self_check_mode() {
        let _ = std::fs::create_dir_all(data_dir());
        let _ = std::fs::write(data_dir().join(RUNNING_MARKER), b"");
        xmrange_core::report::tidy_crash(&data_dir(), crashed_last_time);
        xmrange_core::report::install_crash_hook(data_dir(), env!("CARGO_PKG_VERSION"));
    }
    let settings = Settings::load(&data_dir());
    let opened = open_default();
    if let Some(note) = &opened.note {
        eprintln!("{note}");
    }
    let ledger = opened.ledger.map(LedgerWriter::spawn);
    let startup_note = join_notes(reap_note, opened.note);
    let machine = xmrange_core::telemetry::open_source().ok().map(|(_, m)| m);

    let state = AppState {
        engine: Mutex::new(None),
        stops: Arc::new(StopTracker::default()),
        engine_gen: AtomicU64::new(0),
        settings: Mutex::new(settings),
        machine,
        ledger: Mutex::new(ledger),
        startup_note,
        crashed_last_time,
        _instance_lock: instance_lock,
    };

    #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
    let mut app = tauri::Builder::default()
        .manage(state)
        .plugin(tauri_plugin_dialog::init())
        .on_window_event(on_window_event)
        .invoke_handler(tauri::generate_handler![
            ping,
            self_check_report,
            app_info,
            get_settings,
            save_settings,
            start_mining,
            stop_mining,
            pause_mining,
            resume_mining,
            set_threads,
            set_worker,
            set_wallet,
            clear_wallet,
            list_trials,
            clear_trials,
            start_trials,
            node_status,
            save_diagnostics,
            report_text,
            discard_crash_file,
            save_report,
            send_report,
            copy_text,
            payout_totals,
            network_log,
            set_profile,
            seek,
            stop_seek,
            skip_low_power,
            apply_pick,
            open_battery_settings,
            set_node_folder,
            update_status,
            install_update,
        ])
        .setup(move |app| {
            // The main window is `"create": false` in tauri.conf.json, so it is built here, where
            // a test run (P2-R10) can keep it hidden and unfocused. Normal launches are unchanged.
            // Every launch builds it hidden, fits it to the screen (M10), then shows it.
            let config = app.config().app.windows.iter().find(|w| w.label == "main").cloned().ok_or("no main window in tauri.conf.json")?;
            let min = (config.min_width.unwrap_or(WINDOW_WANT.0), config.min_height.unwrap_or(WINDOW_WANT.1));
            let mut builder = WebviewWindowBuilder::from_config(app.handle(), &config)?.visible(false);
            if background_mode() {
                builder = builder.focused(false);
            }
            let window = builder.build()?;
            fit_to_work_area(&window, min);
            if background_mode() {
                spawn_deadman(app.handle().clone(), start_ppid, started);
            } else {
                window.show()?;
                window.set_focus()?;
            }
            spawn_idle_sampler(app.handle().clone());
            spawn_node_keeper(app.handle().clone());
            spawn_update_watch(app.handle().clone());
            spawn_price_watch(app.handle().clone());
            // Low Power Mode, watched every 2 s whether or not anything mines: the app's one source.
            // Each change is stored for the engine and emitted to the UI (xmrange://lowpower).
            let lp_handle = app.handle().clone();
            let _ = lowpower::spawn_watch(lowpower::POLL, read_low_power, move |v| {
                lowpower::store(v);
                let _ = lp_handle.emit("xmrange://lowpower", v);
            });

            // Quitting by signal (SIGTERM from `kill`, SIGINT from ^C) takes the same stop path as
            // RunEvent::Exit, so xmrig and its caffeinate never outlive the app.
            let handle = app.handle().clone();
            if let Err(e) = ctrlc::set_handler(move || {
                stop_engine_for_exit(&handle);
                handle.exit(0);
            }) {
                eprintln!("quit-signal handler not installed: {e}");
            }

            if autostart_mode() {
                match start_mining(app.handle().clone(), app.state::<AppState>()) {
                    Ok(gen) => println!("AUTOSTART mining, engine {gen}"),
                    Err(e) => println!("AUTOSTART failed: {e}"),
                }
            }
            // A relaunch right after an update install: the one-shot flag `run_install` left says
            // mining was running before the swap, so this fresh copy resumes it once.
            #[cfg(not(feature = "dev-diagnostics"))]
            {
                let flag = data_dir().join(RESUME_MINING_FLAG);
                if flag.exists() {
                    let _ = std::fs::remove_file(&flag);
                    match start_mining(app.handle().clone(), app.state::<AppState>()) {
                        Ok(gen) => println!("resuming mining after update, engine {gen}"),
                        Err(e) => println!("resume-after-update failed: {e}"),
                    }
                }
            }
            #[cfg(feature = "dev-diagnostics")]
            if let Some(after) = dev_stop_quit_after() {
                spawn_stop_then_quit(app.handle().clone(), after);
            }

            if self_check_mode() {
                let handle: AppHandle = app.handle().clone();
                thread::spawn(move || loop {
                    thread::sleep(Duration::from_millis(500));
                    if handle.emit("xmrange://heartbeat", ()).is_err() {
                        break;
                    }
                });
            }
            Ok(())
        })
        .build(context())?;

    // P2-R10: a test run's app is never activated and never takes a Dock slot. Prohibited, not
    // Accessory: tao activates the app at launch, and only a Prohibited app ignores that.
    #[cfg(target_os = "macos")]
    if background_mode() {
        app.set_activation_policy(tauri::ActivationPolicy::Prohibited);
    }

    // Quitting the app always stops xmrig: RunEvent::Exit stops any live engine.
    app.run(|app_handle, event| {
        if let tauri::RunEvent::Exit = event {
            stop_engine_for_exit(app_handle);
        }
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_wallet_defaults_to_ghostdrip() {
        let (wallet, is_default) = resolve_wallet(None);
        assert_eq!(wallet, DEFAULT_WALLET);
        assert!(is_default);
    }

    #[test]
    fn resolve_wallet_uses_user() {
        let (wallet, is_default) = resolve_wallet(Some("some-user-address"));
        assert_eq!(wallet, "some-user-address");
        assert!(!is_default);
    }

    #[test]
    fn finished_engine_releases_its_own_slot() {
        let slot: Mutex<Option<(u64, &str)>> = Mutex::new(Some((3, "engine 3")));
        assert_eq!(take_if_gen(&slot, 3), Some("engine 3"));
        assert!(slot.lock().unwrap().is_none(), "the dead engine's slot is free for the next start");
    }

    #[test]
    fn finished_engine_never_clears_a_newer_one() {
        // Engine 3 was stopped and engine 4 started before 3's forwarding thread wound down.
        let slot: Mutex<Option<(u64, &str)>> = Mutex::new(Some((4, "engine 4")));
        assert_eq!(take_if_gen(&slot, 3), None);
        assert_eq!(*slot.lock().unwrap(), Some((4, "engine 4")));
    }

    #[test]
    fn finished_engine_after_stop_mining_is_a_no_op() {
        // stop_mining already took the slot.
        let slot: Mutex<Option<(u64, &str)>> = Mutex::new(None);
        assert_eq!(take_if_gen(&slot, 3), None);
        assert!(slot.lock().unwrap().is_none());
    }

    #[test]
    fn ui_never_carries_the_default_wallet() {
        // Fixtures and copy in ui/ and ui-dev/ must be synthetic: not even a short prefix of GhostDrip's wallet.
        let prefix = &DEFAULT_WALLET[..6];
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut checked = 0;
        let entries = ["ui", "ui-dev"].into_iter().flat_map(|d| std::fs::read_dir(root.join(d)).expect("ui dir exists"));
        for entry in entries {
            let path = entry.expect("readable entry").path();
            let is_text = matches!(path.extension().and_then(|e| e.to_str()), Some("js" | "html" | "css"));
            if !is_text {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("readable ui file");
            assert!(!text.contains(prefix), "{} contains a prefix of the default wallet", path.display());
            checked += 1;
        }
        assert!(checked >= 5, "expected the ui and ui-dev sources, found {checked}");
    }

    #[test]
    fn save_settings_pins_update_folder_to_the_stored_value() {
        let stored = Settings { update_folder: Some("kept".to_string()), ..Settings::default() };
        let incoming = Settings { update_folder: Some("/elsewhere".to_string()), rate_per_kwh: 0.2, ..Settings::default() };
        let accepted = accept_settings(incoming, &stored, 8).expect("valid settings");
        assert_eq!(accepted.update_folder.as_deref(), Some("kept"), "an incoming update_folder is ignored");
        assert_eq!(accepted.rate_per_kwh, 0.2, "everything else is taken");

        let stored_none = Settings::default();
        let incoming = Settings { update_folder: Some("/elsewhere".to_string()), ..Settings::default() };
        assert_eq!(accept_settings(incoming, &stored_none, 8).unwrap().update_folder, None);
    }

    #[test]
    fn save_settings_refuses_invalid_settings_verbatim() {
        let stored = Settings::default();
        let cases = [
            (Settings { rate_per_kwh: 1e308, ..Settings::default() }, "electricity rate must be a number from 0 to 100 per kWh"),
            (Settings { currency: "abcd".to_string(), ..Settings::default() }, "currency must be 1–3 characters, with no spaces or control characters"),
        ];
        for (incoming, want) in cases {
            assert_eq!(accept_settings(incoming, &stored, 8).unwrap_err(), want);
        }
    }

    /// M3: worker and threads have their own commands; a save never writes them back.
    #[test]
    fn save_settings_pins_worker_and_threads_to_the_stored_values() {
        let stored = Settings { worker: "new-name".to_string(), threads: 6, ..Settings::default() };
        let stale = Settings { worker: "old-name".to_string(), threads: 2, period: xmrange_core::settings::Period::Week, ..Settings::default() };
        let accepted = accept_settings(stale, &stored, 8).expect("valid");
        assert_eq!((accepted.worker.as_str(), accepted.threads), ("new-name", 6), "a stale copy never reverts them");
        assert_eq!(accepted.period, xmrange_core::settings::Period::Week, "the rest is taken");
        // Invalid incoming values for pinned fields are ignored, not refused.
        let odd = Settings { worker: "bad name!".to_string(), threads: 0, ..Settings::default() };
        assert!(accept_settings(odd, &stored, 8).is_ok());
    }

    /// I3: a stop in flight holds the quit path's wait; the wait ends when the stop does.
    #[test]
    fn quitting_waits_for_a_stop_in_flight() {
        let tracker = Arc::new(StopTracker::default());
        assert!(tracker.wait_idle(Duration::from_millis(1)), "nothing in flight: no wait");
        tracker.begin();
        assert!(!tracker.idle());
        assert!(!tracker.wait_idle(Duration::from_millis(20)), "still in flight: the wait times out");
        let t = tracker.clone();
        let stopper = thread::spawn(move || {
            thread::sleep(Duration::from_millis(100));
            InFlightStop { ctl: None, tracker: t }.run();
        });
        let t0 = Instant::now();
        assert!(tracker.wait_idle(Duration::from_secs(5)), "the stop finished within the wait");
        assert!(t0.elapsed() >= Duration::from_millis(90), "the wait really waited for it");
        stopper.join().unwrap();
        assert!(tracker.idle());
    }

    /// T9 review m2: two quit paths at once (SIGTERM during `RunEvent::Exit`). Exactly one closes
    /// the Ledger, and neither returns before the writer has ended, so neither can end the
    /// process mid-close: a request sent right after either returns is refused.
    #[test]
    fn a_second_quit_path_waits_for_the_ledger_close() {
        use xmrange_core::ledger::{Ledger, LedgerKey};
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../target/test-scratch/app-ledger-close-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ledger = Ledger::open(&dir.join("ledger.sqlite"), &LedgerKey::generate()).expect("open a scratch Ledger");
        let writer = LedgerWriter::spawn(ledger);
        let handle = writer.handle();
        assert!(handle.trials(1).is_ok(), "the writer answers while open");
        let slot = Arc::new(Mutex::new(Some(writer)));
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let quits: Vec<_> = (0..2)
            .map(|_| {
                let (slot, barrier, handle) = (slot.clone(), barrier.clone(), handle.clone());
                thread::spawn(move || {
                    barrier.wait();
                    let msg = close_ledger(&slot);
                    (msg, handle.trials(1).is_ok())
                })
            })
            .collect();
        let results: Vec<(Option<String>, bool)> = quits.into_iter().map(|q| q.join().unwrap()).collect();
        let closers: Vec<&String> = results.iter().filter_map(|(m, _)| m.as_ref()).collect();
        assert_eq!(closers.len(), 1, "exactly one quit path closes the Ledger: {results:?}");
        assert_eq!(closers[0], "ledger closed\n");
        for (msg, answered) in &results {
            assert!(!answered, "a quit path returned while the writer was still open ({msg:?})");
        }
        assert!(slot.lock().unwrap().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// M10: the window fits a 13" Air's work area, centred; a large screen keeps the designed size.
    #[test]
    fn window_fits_the_work_area_and_centres() {
        let min = (960.0, 600.0);
        // 13" M3 Air at default scaling: 1470 × 956 points, less a 37 pt menu bar and a 70 pt Dock.
        let air = (0.0, 37.0, 1470.0, 849.0);
        assert_eq!(fit_window(WINDOW_WANT, min, air), (95.0, 37.0, 1280.0, 849.0));
        let big = (0.0, 25.0, 1728.0, 1080.0);
        assert_eq!(fit_window(WINDOW_WANT, min, big), (224.0, 80.0, 1280.0, 970.0));
        // Never below the minimum, even on a tiny area.
        let tiny = (0.0, 0.0, 800.0, 500.0);
        assert_eq!(fit_window(WINDOW_WANT, min, tiny), (0.0, 0.0, 960.0, 600.0));
    }

    #[test]
    fn window_config_is_resizable_with_a_lower_minimum() {
        let conf: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let w = &conf["app"]["windows"][0];
        assert_eq!((w["width"].as_f64(), w["height"].as_f64()), (Some(WINDOW_WANT.0), Some(WINDOW_WANT.1)));
        assert_eq!(w["resizable"], true);
        assert!(w["minHeight"].as_f64().unwrap() < 956.0 - 37.0, "a 13-inch Air's work area is taller than the minimum");
    }

    /// F7: `tidy_dev_logs` only deletes. The oldest stamped logs go; kept logs stay byte for
    /// byte (redaction is `sweep_dev_logs`'s job), and unstamped files and folders are left alone.
    #[cfg(feature = "dev-diagnostics")]
    #[test]
    fn tidy_dev_logs_only_deletes() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../target/test-scratch/app-tidy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        // A documentation-range address (RFC 5737), built at runtime: a line redaction would change.
        let addr = [192, 0, 2, 7].map(|n: u8| n.to_string()).join(".");
        let body = format!("[net] use pool {addr}:443\n");
        assert_ne!(xmrange_core::redact::redact_ips(&body), body, "the planted line is one redaction would rewrite");
        for stamp in 1..=7 {
            std::fs::write(dir.join(format!("{stamp}.log")), &body).unwrap();
        }
        std::fs::write(dir.join("smoke.log"), &body).unwrap();
        std::fs::write(dir.join("sub/9.log"), &body).unwrap();

        tidy_dev_logs(&dir, 3);

        let mut left: Vec<String> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, ["5.log", "6.log", "7.log", "smoke.log", "sub"], "only the oldest stamped logs are deleted");
        for kept in ["5.log", "6.log", "7.log", "smoke.log", "sub/9.log"] {
            assert_eq!(std::fs::read_to_string(dir.join(kept)).unwrap(), body, "{kept} is left exactly as it was");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// M1: the second lock on the same data dir is refused while the first is held.
    #[test]
    fn a_second_instance_is_refused_the_lock() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../target/test-scratch/app-lock-{}", std::process::id()));
        let first = instance_lock(&dir).expect("lock").expect("first copy gets it");
        assert!(instance_lock(&dir).expect("lock").is_none(), "a second copy is refused");
        drop(first);
        // Other tests spawn processes in parallel: between fork and exec a child briefly holds a
        // copy of the lock's descriptor, so the release can lag a moment. Wait for it, bounded.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let again = loop {
            match instance_lock(&dir).expect("lock") {
                Some(l) => break Some(l),
                None if std::time::Instant::now() < deadline => std::thread::sleep(std::time::Duration::from_millis(20)),
                None => break None,
            }
        };
        assert!(again.is_some(), "free again once the first copy is gone");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Final review M8: `instance_lock` makes the data dir itself, so it must leave it 0700, not
    /// whatever `create_dir_all`'s umask-shaped mode would have given it.
    #[test]
    fn instance_lock_makes_the_data_dir_0700() {
        use std::os::unix::fs::PermissionsExt;
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../target/test-scratch/app-lock-mode-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let _lock = instance_lock(&dir).expect("lock").expect("first copy gets it");
        assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
        drop(_lock);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Guard for the dev-only self-check surface (fix round 2, N1). Source-level, so it runs in
    /// every build: the backup module is declared only behind the dev-diagnostics cfg, nothing
    /// outside that module names the backup file, and every env read goes through
    /// `self_check_mode()`, whose release branch is a hard `false`. (A call into
    /// `selfcheck_backup::` without its own cfg would already fail the release build.)
    #[test]
    fn self_check_surface_is_dev_only() {
        let src = include_str!("main.rs");
        let lines: Vec<&str> = src.lines().map(str::trim).collect();
        let decl = lines.iter().position(|l| *l == "mod selfcheck_backup;").expect("module declared");
        assert_eq!(lines[decl - 1], "#[cfg(feature = \"dev-diagnostics\")]", "the backup module must stay dev-only");
        let backup_name = ["settings", ".selfcheck-backup"].concat();
        assert!(!src.contains(&backup_name), "only selfcheck_backup.rs names the backup file");
        let env_read = ["env::var(\"XMRANGE", "_SELF_CHECK\")"].concat();
        assert_eq!(src.matches(&env_read).count(), 1, "XMRANGE_SELF_CHECK is read in self_check_mode() only");
        for gate in ["fn self_check_mode() -> bool {", "fn autostart_mode() -> bool {"] {
            let f = src.find(gate).expect(gate);
            let body = &src[f..f + src[f..].find("\n}\n").expect("end of fn")];
            assert!(body.contains("#[cfg(not(feature = \"dev-diagnostics\"))]\n    {\n        false\n    }"), "{gate} release branch must be a hard false");
        }
        let autostart_read = ["env::var(\"XMRANGE", "_AUTOSTART\")"].concat();
        assert_eq!(src.matches(&autostart_read).count(), 1, "XMRANGE_AUTOSTART is read in autostart_mode() only");
        let bg = src.find("fn background_mode() -> bool {").expect("background_mode");
        let bg_body = &src[bg..bg + src[bg..].find("\n}\n").expect("end of fn")];
        assert!(bg_body.contains("self_check_mode() || autostart_mode()"), "P2-R10 gate is built from the two dev-only gates");
    }

    /// Task 11: selfcheck.js never loads in a release build. app.js has no static import of it,
    /// only one dynamic import behind `info?.self_check` (which `self_check_mode()` makes a hard
    /// false in release, see `release_ignores_self_check_env`), and a release build doesn't even
    /// contain it (`harness_lives_outside_the_release_frontend`, `release_context_has_no_harness`).
    #[test]
    fn selfcheck_js_loads_only_in_self_check_mode() {
        let app_js = include_str!("../ui/app.js");
        let module = ["\"./selfcheck", ".js\""].concat();
        let dynamic = ["import(", &module, ")"].concat();
        assert!(!app_js.contains(&["from ", &module].concat()), "no static import of selfcheck.js");
        assert_eq!(app_js.matches(&module).count(), 1, "selfcheck.js is named once, in the dynamic import");
        let at = app_js.find(&dynamic).expect("dynamic import of selfcheck.js");
        let guard = app_js[..at].rfind("if (!info?.self_check) return;").expect("self_check guard");
        let block = app_js[..guard].rfind("ready.then(async () => {").expect("guarded block");
        // No block closes between the block's start, its guard and the import (JS, 2-space indent).
        let closes = |text: &str| text.contains("\n}") || text.contains("\n  }");
        assert!(!closes(&app_js[block..guard]), "the guard is the block's own early return");
        assert!(!closes(&app_js[guard..at]), "the import sits after the guard, in the same block");
        let html = include_str!("../ui/index.html");
        assert!(!html.contains("selfcheck"), "index.html never loads the self-check");
    }

    /// The harness sits in ui-dev/, never in ui/ (the frontendDist a release build embeds).
    #[test]
    fn harness_lives_outside_the_release_frontend() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let conf = std::fs::read_to_string(root.join("tauri.conf.json")).expect("tauri.conf.json");
        assert!(conf.contains("\"frontendDist\": \"ui\""), "frontendDist is ui/");
        assert!(root.join("ui-dev/selfcheck.js").is_file(), "the harness is in ui-dev/");
        assert!(!root.join("ui/selfcheck.js").exists(), "the harness is not in ui/");
        let marker = ["xmrange-dev", "-harness"].concat();
        assert!(include_str!("../ui-dev/selfcheck.js").contains(&marker), "check-release.sh's marker is in the harness");
    }

    /// Dev builds serve the harness from `dev_assets()`, next to the embedded ui/.
    #[cfg(feature = "dev-diagnostics")]
    #[test]
    fn dev_context_serves_the_harness() {
        let context = context();
        let got = context.assets().get(&"selfcheck.js".into()).expect("dev builds serve the harness");
        assert_eq!(&*got, include_bytes!("../ui-dev/selfcheck.js"));
        assert!(context.assets().get(&"/app.js".into()).is_some(), "the embedded ui/ is still served");
        assert!(context.assets().iter().any(|(k, _)| k.trim_start_matches('/') == "selfcheck.js"));
    }

    /// Release builds: the real context has no harness to serve or list. (A request for it gets
    /// Tauri's index.html fallback, which the module loader refuses, never harness bytes.)
    #[cfg(not(feature = "dev-diagnostics"))]
    #[test]
    fn release_context_has_no_harness() {
        let context = context();
        assert!(dev_assets().is_empty());
        assert!(context.assets().get(&"selfcheck.js".into()).is_none());
        assert!(context.assets().get(&"/selfcheck.js".into()).is_none());
        assert!(!context.assets().iter().any(|(k, _)| k.contains("selfcheck")));
        assert!(context.assets().get(&"/app.js".into()).is_some(), "the embedded ui/ is still served");
    }

    #[test]
    fn deadman_stops_an_orphan_or_an_old_test_run() {
        let young = Duration::from_secs(5);
        assert_eq!(deadman_reason(4242, 4242, young), None, "parent alive, young: keep running");
        assert!(deadman_reason(4242, 1, young).is_some(), "reparented to launchd: the runner died");
        assert!(deadman_reason(4242, 777, young).is_some(), "any parent change stops it");
        assert!(deadman_reason(1, 1, young).is_some(), "started as an orphan: stop at once");
        assert!(deadman_reason(4242, 4242, BACKGROUND_MAX_LIFETIME).is_some(), "max lifetime reached");
        assert_eq!(deadman_reason(4242, 4242, BACKGROUND_MAX_LIFETIME - Duration::from_secs(1)), None);
    }

    /// Parked from Plan 2's final review (N1): the stop-then-quit test hook must not exist in a
    /// release build, not merely be unreachable.
    #[test]
    fn stop_then_quit_hook_is_dev_only() {
        let src = include_str!("main.rs");
        let lines: Vec<&str> = src.lines().collect();
        for decl in ["fn dev_stop_quit_after() -> Option<Duration> {", "fn spawn_stop_then_quit(handle: AppHandle, after: Duration) {"] {
            let at = lines.iter().position(|l| *l == decl).unwrap_or_else(|| panic!("{decl} not found"));
            let gate = lines[..at].iter().rev().find(|l| !l.trim_start().starts_with("///")).copied();
            assert_eq!(gate, Some("#[cfg(feature = \"dev-diagnostics\")]"), "{decl} must be dev-only");
        }
    }

    /// Release builds only: the self-check stays off even with its env vars set.
    #[cfg(not(feature = "dev-diagnostics"))]
    #[test]
    fn release_ignores_self_check_env() {
        std::env::set_var("XMRANGE_SELF_CHECK", "1");
        std::env::set_var("XMRANGE_SELF_CHECK_LIVE", "1");
        std::env::set_var("XMRANGE_AUTOSTART", "1");
        let (mode, live, auto, bg) = (self_check_mode(), self_check_live(), autostart_mode(), background_mode());
        std::env::remove_var("XMRANGE_SELF_CHECK");
        std::env::remove_var("XMRANGE_SELF_CHECK_LIVE");
        std::env::remove_var("XMRANGE_AUTOSTART");
        assert!(!mode && !live, "a release build never enters self-check mode");
        assert!(!auto && !bg, "a release build never autostarts or hides its window");
    }

    /// M14: the dark palette is written twice in styles.css (explicit dark, and system while macOS is
    /// dark), because CSS can't share one block between a selector and a media query without JS or
    /// `light-dark()`, which older macOS WebKit lacks. This keeps the two copies identical.
    #[test]
    fn dark_tokens_match_in_both_blocks() {
        let css = include_str!("../ui/styles.css");
        let block = |start: &str| {
            let at = css.find(start).unwrap_or_else(|| panic!("{start} not found"));
            let body = &css[at + start.len()..];
            let body = &body[..body.find('}').expect("block end")];
            body.split(';').map(str::trim).filter(|d| !d.is_empty()).collect::<Vec<_>>()
        };
        let dark = block(":root[data-theme=\"dark\"] {");
        let system = block("@media (prefers-color-scheme: dark) {\n  :root[data-theme=\"system\"] {");
        assert!(dark.len() >= 15, "{dark:?}");
        assert_eq!(dark, system, "the explicit-dark and system-dark palettes drifted apart");
    }

    /// The traffic lights follow focus on every launch: the handler is registered on the builder,
    /// reacts to `Focused`, and is gated by nothing (no dev feature, no test-run mode).
    #[test]
    fn traffic_lights_follow_focus_on_every_launch() {
        let src = include_str!("main.rs");
        let code = &src[..src.find("#[cfg(test)]\nmod tests").expect("tests module")];
        assert_eq!(code.matches(".on_window_event(on_window_event)").count(), 1, "registered once, on the builder");
        let f = code.find("fn on_window_event(").expect("handler");
        let body = &code[f..f + code[f..].find("\n}\n").expect("end of fn")];
        assert!(body.contains("WindowEvent::Focused(key)") && body.contains("traffic_lights::follow_focus(window, *key)"), "{body}");
        for gate in ["dev-diagnostics", "background_mode", "self_check", "autostart"] {
            assert!(!body.contains(gate), "the handler must not be gated by {gate}");
        }
        let m = code.find("mod traffic_lights {").expect("traffic_lights module");
        assert!(code[..m].trim_end().ends_with("#[cfg(target_os = \"macos\")]"), "macOS only, and nothing else");
    }

    #[test]
    fn startup_notes_join_in_order() {
        assert_eq!(join_notes(None, None), None);
        assert_eq!(join_notes(Some("reaped"), None).as_deref(), Some("reaped"));
        assert_eq!(join_notes(None, Some("ledger".into())).as_deref(), Some("ledger"));
        assert_eq!(join_notes(Some("reaped"), Some("ledger".into())).as_deref(), Some("reaped · ledger"));
    }

    /// Review Focus 2: threads −/+ are hand input (a profile pick is intent, Q4).
    #[test]
    fn hand_changes_drop_to_manual() {
        use xmrange_core::optimizer::{Mode, OptimizerPrefs, Profile};
        let mut p = OptimizerPrefs { profile: Profile::Max, mode: Mode::Hold, ..Default::default() };
        super::to_manual(&mut p);
        assert_eq!((p.mode, p.sweep.is_none(), p.profile), (Mode::Manual, true, Profile::Max));
    }

    #[test]
    fn save_settings_pins_the_optimizer_prefs() {
        use xmrange_core::optimizer::{Mode, OptimizerPrefs, Profile};
        let stored = Settings { optimizer: OptimizerPrefs { profile: Profile::Max, mode: Mode::Hold, ..Default::default() }, ..Settings::default() };
        let accepted = accept_settings(Settings::default(), &stored, 8).unwrap();
        assert_eq!(accepted.optimizer, stored.optimizer, "only the optimizer's commands and the engine move it");
    }

    #[test]
    fn battery_settings_pane_falls_back_to_energy_saver() {
        let urls = battery_settings_urls();
        assert_eq!(urls[0], "x-apple.systempreferences:com.apple.Battery-Settings.extension");
        assert_eq!(urls[1], "x-apple.systempreferences:com.apple.preference.energysaver");
    }

    /// Every command the app registers is in build.rs's manifest and allowed in the capabilities,
    /// read from generate_handler! itself, so a new command can't be left out (apply_pick was,
    /// from Plan 5 to 0.2.21: clicking a trials row failed with "not allowed by ACL").
    #[test]
    fn every_registered_command_is_in_the_manifest_and_allowed() {
        let build = include_str!("../build.rs");
        let caps = include_str!("../capabilities/main.json");
        let main = include_str!("main.rs");
        let start = main.find("tauri::generate_handler![").expect("generate_handler!") + "tauri::generate_handler![".len();
        let end = start + main[start..].find(']').unwrap();
        let commands: Vec<&str> = main[start..end].split(',').map(str::trim).filter(|c| !c.is_empty()).collect();
        assert!(commands.len() > 20, "{commands:?}");
        for c in commands {
            assert!(build.contains(&format!("\"{c}\"")), "{c} missing from build.rs");
            assert!(caps.contains(&format!("\"allow-{}\"", c.replace('_', "-"))), "{c} missing from capabilities/main.json");
        }
    }

    /// The window's side of every command, read from the UI source itself (owner, 2026-10-05: "are
    /// you not simulating using all the buttons"): each `invoke("cmd", { … })` names a registered
    /// command, passes exactly its arguments in camelCase (a missing one fails at click time with
    /// "missing required key"), every command run by name (`trialsCmd("…")`, the run pills) is
    /// registered, every dialog the window opens is granted, every button's `data-act` has a
    /// handler and every handler has a button, and every `$("id")` the window wires exists.
    #[test]
    fn every_ui_control_reaches_a_registered_granted_command_with_its_exact_arguments() {
        let main = include_str!("main.rs");
        let caps = include_str!("../capabilities/main.json");
        let app = include_str!("../ui/app.js");
        let modules = include_str!("../ui/modules.js");
        let html = include_str!("../ui/index.html");
        let ui = format!("{app}\n{modules}");

        // Rust: command name → (every argument, the required ones), camelCase as the window sends them.
        let camel = |s: &str| {
            let mut out = String::new();
            let mut up = false;
            for ch in s.chars() {
                if ch == '_' {
                    up = true;
                } else if up {
                    out.extend(ch.to_uppercase());
                    up = false;
                } else {
                    out.push(ch);
                }
            }
            out
        };
        let mut sigs: std::collections::HashMap<String, (Vec<String>, Vec<String>)> = Default::default();
        let source = &main[..main.find("#[cfg(test)]\nmod tests").expect("the test module")];
        for chunk in source.split("#[tauri::command]").skip(1) {
            let at = chunk.find("fn ").expect("fn after #[tauri::command]") + 3;
            let open = at + chunk[at..].find('(').unwrap();
            let name = chunk[at..open].trim().to_string();
            // The parameter list ends at the ')' that balances the opening '(' (types like State<'_, AppState>).
            let (mut depth, mut close) = (0i32, open);
            for (i, ch) in chunk[open..].char_indices() {
                match ch {
                    '(' | '<' => depth += 1,
                    ')' | '>' => depth -= 1,
                    _ => {}
                }
                if depth == 0 {
                    close = open + i;
                    break;
                }
            }
            let (mut all, mut required) = (vec![], vec![]);
            let params = &chunk[open + 1..close];
            let (mut depth, mut cur) = (0i32, String::new());
            let mut parts = vec![];
            for ch in params.chars() {
                match ch {
                    '<' | '(' => depth += 1,
                    '>' | ')' => depth -= 1,
                    ',' if depth == 0 => {
                        parts.push(std::mem::take(&mut cur));
                        continue;
                    }
                    _ => {}
                }
                cur.push(ch);
            }
            parts.push(cur);
            for p in parts.iter().map(|p| p.trim()).filter(|p| !p.is_empty()) {
                let (arg, ty) = p.split_once(':').unwrap();
                let ty = ty.trim();
                if ty.starts_with("State<") || ty.starts_with("AppHandle") || ty.starts_with("Window") || ty.starts_with("WebviewWindow") {
                    continue;
                }
                let arg = camel(arg.trim().trim_start_matches('_'));
                if !ty.starts_with("Option<") {
                    required.push(arg.clone());
                }
                all.push(arg);
            }
            sigs.entry(name).or_insert((all, required)); // install_update has a dev and a release twin
        }
        let start = main.find("tauri::generate_handler![").unwrap() + "tauri::generate_handler![".len();
        let end = start + main[start..].find(']').unwrap();
        let registered: Vec<&str> = main[start..end].split(',').map(str::trim).filter(|c| !c.is_empty()).collect();
        for c in &registered {
            assert!(sigs.contains_key(*c), "{c} is registered but no #[tauri::command] fn {c} was read");
        }

        // The window: every literal invoke and its argument keys.
        let mut used: std::collections::HashSet<String> = Default::default();
        let mut failures = vec![];
        for (i, _) in ui.match_indices("invoke(\"") {
            let rest = &ui[i + "invoke(\"".len()..];
            let name = &rest[..rest.find('"').unwrap()];
            used.insert(name.to_string());
            let Some((all, required)) = sigs.get(name) else {
                failures.push(format!("the window calls {name}, which no command defines"));
                continue;
            };
            if !registered.contains(&name) {
                failures.push(format!("the window calls {name}, which is not registered"));
            }
            let after = rest[name.len() + 1..].trim_start();
            let mut keys = vec![];
            if let Some(obj) = after.strip_prefix(',').map(str::trim_start).and_then(|s| s.strip_prefix('{')) {
                let (mut depth, mut cur) = (0i32, String::new());
                for ch in obj.chars() {
                    match ch {
                        '{' | '(' | '[' => depth += 1,
                        ')' | ']' => depth -= 1,
                        '}' if depth == 0 => break,
                        '}' => depth -= 1,
                        ',' if depth == 0 => {
                            keys.push(std::mem::take(&mut cur));
                            continue;
                        }
                        _ => {}
                    }
                    cur.push(ch);
                }
                keys.push(cur);
            }
            let keys: Vec<String> = keys.iter().map(|k| k.split(':').next().unwrap().trim().to_string()).filter(|k| !k.is_empty()).collect();
            for r in required {
                if !keys.contains(r) {
                    failures.push(format!("invoke(\"{name}\") misses its argument {r} (sends {keys:?})"));
                }
            }
            for k in &keys {
                if !all.contains(k) {
                    failures.push(format!("invoke(\"{name}\") sends {k}, which {name} does not take ({all:?})"));
                }
            }
        }
        // Commands run by name through a helper, with no arguments.
        for helper in ["trialsCmd(\"", "startMining(\"", "sendMining(\""] {
            for (i, _) in ui.match_indices(helper) {
                let rest = &ui[i + helper.len()..];
                let name = &rest[..rest.find('"').unwrap()];
                used.insert(name.to_string());
                match sigs.get(name) {
                    Some((_, req)) if registered.contains(&name) && req.is_empty() => {}
                    _ => failures.push(format!("{helper}{name}\") is not a registered command without arguments")),
                }
            }
        }
        // The run pills: the commands liveCommand returns.
        let live = &modules[modules.find("export function liveCommand").expect("liveCommand")..];
        let live = &live[..live.find("\n}").unwrap()];
        for (i, _) in live.match_indices("return \"") {
            let rest = &live[i + "return \"".len()..];
            let name = &rest[..rest.find('"').unwrap()];
            used.insert(name.to_string());
            if !registered.contains(&name) {
                failures.push(format!("the run pill sends {name}, which is not registered"));
            }
        }
        // Commands only the dev harness calls, by design.
        for dev in ["ping", "self_check_report"] {
            used.insert(dev.to_string());
        }
        for c in &registered {
            if !used.contains(*c) {
                failures.push(format!("{c} is registered but nothing in the window calls it"));
            }
        }
        // Dialogs the window opens.
        for d in ["open", "save", "ask", "message", "confirm"] {
            if ui.contains(&format!("dialog.{d}(")) && !caps.contains(&format!("\"dialog:allow-{d}\"")) {
                failures.push(format!("the window opens dialog.{d} without dialog:allow-{d}"));
            }
        }

        // Buttons: every data-act (literal, or `act: "…"` fed into one) has a handler, and back.
        let block = &app[app.find("Object.assign(actions, {").expect("actions map")..];
        let block = &block[..block.find("\n});").unwrap()];
        let handlers: Vec<String> = block
            .lines()
            .filter_map(|l| {
                let l = l.trim();
                let key = l.split(':').next()?.trim().trim_matches('"');
                (!l.starts_with("//") && l.contains(':') && !key.is_empty() && key.chars().all(|c| c.is_ascii_lowercase() || c == '-')).then(|| key.to_string())
            })
            .collect();
        let mut acts: std::collections::HashSet<String> = Default::default();
        for pat in ["data-act=\"", "act: \"", "pill(\""] {
            for (i, _) in ui.match_indices(pat) {
                let rest = &ui[i + pat.len()..];
                let a = &rest[..rest.find('"').unwrap()];
                if !a.starts_with('$') {
                    acts.insert(a.to_string());
                }
            }
        }
        for a in &acts {
            if !handlers.contains(a) {
                failures.push(format!("a button has data-act \"{a}\" but no handler"));
            }
        }
        for h in &handlers {
            if !acts.contains(h) {
                failures.push(format!("handler \"{h}\" has no button that sends it"));
            }
        }

        // Every element the window wires by id exists.
        for (i, _) in app.match_indices("$(\"") {
            let rest = &app[i + 3..];
            let id = &rest[..rest.find('"').unwrap()];
            if !html.contains(&format!("id=\"{id}\"")) && !modules.contains(&format!("id=\"{id}\"")) && !app.contains(&format!("id=\"{id}\"")) {
                failures.push(format!("$(\"{id}\") names no element"));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// A settings write that fails leaves memory as it was (disk first, then memory): the window
    /// rolls its change back, and the next start must not run the value it took back.
    #[test]
    fn a_failed_settings_write_changes_nothing_in_memory() {
        let base = scratch("failed-write");
        std::fs::create_dir_all(&base).unwrap();
        let not_a_dir = base.join("file");
        std::fs::write(&not_a_dir, b"x").unwrap();
        let state = stopped_state(Settings { threads: 3, ..Settings::default() });
        assert!(set_threads_in(&state, &not_a_dir, 6).is_err());
        assert!(set_profile_in(&state, &not_a_dir, Profile::Max).is_err());
        let s = state.settings.lock().unwrap();
        assert_eq!((s.threads, s.optimizer.mode), (3, Mode::Manual), "memory kept the old values");
        drop(s);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// p2pool reads the wallet only at start: stopped, a wallet change is fine; the window shows
    /// the backend's refusal in the same words before anyone types.
    #[test]
    fn the_p2pool_wallet_lock_is_open_while_stopped_and_worded_alike_in_the_window() {
        let state = stopped_state(Settings { pool: xmrange_core::settings::Pool::P2pool, ..Settings::default() });
        assert_eq!(p2pool_wallet_unlocked(&state), Ok(()));
        assert!(include_str!("../ui/app.js").contains(&format!("\"{P2POOL_WALLET_LOCKED}\"")), "app.js says the lock in other words");
    }

    /// A stopped app's state for the command tests: no engine, no Ledger, and a made-up machine
    /// (so the optimizer is available). Nothing here reaches the owner's data folder.
    fn stopped_state(settings: Settings) -> AppState {
        let machine = MachineInfo { model: "test".into(), chip: "Apple M3".into(), p_cores: 4, e_cores: 4, memory_gb: 16 };
        AppState { machine: Some(machine), ..no_readings_state(settings) }
    }

    /// As `stopped_state`, on a Mac whose power readings didn't answer at launch.
    fn no_readings_state(settings: Settings) -> AppState {
        AppState {
            engine: Mutex::new(None),
            stops: Arc::new(StopTracker::default()),
            engine_gen: AtomicU64::new(0),
            settings: Mutex::new(settings),
            machine: None,
            ledger: Mutex::new(None),
            startup_note: None,
            crashed_last_time: false,
            _instance_lock: None,
        }
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../target/test-scratch/app-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// F6: the command itself, not just the helper. A threads −/+ on a stopped app saves manual,
    /// so the next launch never pulls the levers against the user (Review Focus 2).
    #[test]
    fn set_threads_drops_a_stopped_app_to_manual() {
        use xmrange_core::optimizer::SweepRef;
        let dir = scratch("set-threads");
        let held = OptimizerPrefs { profile: Profile::Max, mode: Mode::Hold, sweep: Some(SweepRef::default()), ..Default::default() };
        let state = stopped_state(Settings { optimizer: held, ..Settings::default() });
        set_threads_in(&state, &dir, 1).expect("one thread is always valid");
        let mem = state.settings.lock().unwrap().optimizer.clone();
        assert_eq!((mem.mode, mem.sweep.is_none(), mem.profile), (Mode::Manual, true, Profile::Max));
        let saved = Settings::load(&dir);
        assert_eq!((saved.threads, saved.optimizer.mode, saved.optimizer.profile), (1, Mode::Manual, Profile::Max), "saved to disk as manual");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A stopped app's pick is saved as intent: the next start holds that throttle.
    #[test]
    fn a_stopped_pick_is_saved_for_the_next_start() {
        let dir = scratch("set-profile");
        let state = stopped_state(Settings::default());
        assert_eq!(state.settings.lock().unwrap().optimizer.mode, Mode::Manual, "first launch is manual");
        set_profile_in(&state, &dir, Profile::Light).unwrap();
        let saved = Settings::load(&dir).optimizer;
        assert_eq!((saved.profile, saved.mode), (Profile::Light, Mode::Hold));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Run trials, stop and skip need a running engine; stopped, they say so and save nothing.
    #[test]
    fn search_commands_refuse_while_stopped() {
        let state = stopped_state(Settings::default());
        for c in [OptCommand::Seek, OptCommand::StopSeek, OptCommand::SkipLowPower] {
            assert_eq!(optimizer_command(&state, c).unwrap_err(), NOT_MINING);
        }
        assert_eq!(state.settings.lock().unwrap().optimizer, OptimizerPrefs::default());
    }

    /// P4-C19: no power readings means no optimizer in the engine, so every optimizer
    /// commands refuse with words the UI can show. None is a silent Ok, and nothing is saved.
    #[test]
    fn optimizer_commands_refuse_without_power_readings() {
        let dir = scratch("no-readings");
        let state = no_readings_state(Settings::default());
        for c in [OptCommand::Seek, OptCommand::StopSeek, OptCommand::SkipLowPower] {
            assert_eq!(optimizer_command(&state, c).unwrap_err(), NO_POWER_READINGS);
        }
        assert_eq!(set_profile_in(&state, &dir, Profile::Max).unwrap_err(), NO_POWER_READINGS);
        assert_eq!(state.settings.lock().unwrap().optimizer, OptimizerPrefs::default());
        assert!(!dir.exists(), "nothing was written");
    }

    /// P4-C19: the saving optimizer command is async and does its work on a blocking
    /// thread; the three that only send are refusals or a channel send, never a disk write.
    #[test]
    fn saving_optimizer_commands_run_off_the_main_thread() {
        let src = include_str!("main.rs");
        for (decl, call) in [
            ("async fn set_profile(app: AppHandle, profile: Profile)", "spawn_blocking(move || set_profile_in("),
        ] {
            let at = src.find(decl).unwrap_or_else(|| panic!("{decl} not found"));
            let body = &src[at..at + src[at..].find("\n}\n").expect("end of fn")];
            assert!(body.contains(call), "{decl} must run its save through spawn_blocking");
        }
    }

    #[test]
    fn default_label_is_ghostdrip_not_xmrange() {
        let view = wallet_view(None);
        assert!(view.is_default);
        assert!(view.label.contains("ghostdrip"), "{}", view.label);
        assert!(!view.label.to_lowercase().contains("xmrange"), "{}", view.label);
        assert!(!view.label.to_lowercase().contains("project"), "{}", view.label);
    }
}
