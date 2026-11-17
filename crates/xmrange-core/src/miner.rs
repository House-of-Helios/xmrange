use crate::api::{Summary, XmrigApi};
use crate::config::MinerConfig;
use crate::handoff::Handoff;
use crate::orphan::{self, RunRecord};
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub struct LaunchOptions {
    pub config: MinerConfig,
    pub api_port: u16,
    pub e_core_clamp: bool,
    /// Launch under `taskpolicy -c utility` (THESIS 18's QoS clamp): an experiment lever only.
    pub qos_utility: bool,
    pub dev_log: Option<PathBuf>,
}

/// A running, privately configured xmrig. Dropping it stops xmrig and cleans up.
pub struct Miner {
    child: Child,
    caffeinate: Option<Child>,
    /// Stops xmrig if the process that launched it dies without doing so (see `spawn_guard`).
    guard: Option<Child>,
    /// `<data dir>/run`: the handoff dirs and the run record (`orphan::RECORD_FILE`) live here.
    run_dir: PathBuf,
    api: XmrigApi,
    config: MinerConfig,
    token: String,
    api_port: u16,
    dev_log: Option<PathBuf>,
    handoff: Option<Handoff>,
    handoff_file: PathBuf,
    stopped: bool,
    /// Whether the background band (`taskpolicy -b`) is actually applied to xmrig right now.
    e_core_clamp: bool,
}

pub fn build_argv(config_path: &Path) -> Vec<String> {
    vec!["-c".into(), config_path.display().to_string(), "--no-color".into()]
}

pub fn port_is_free(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
}

pub fn max_threads() -> u32 {
    thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(1)
}

pub fn validate_threads(n: u32, max: u32) -> Result<()> {
    if n == 0 || n > max {
        bail!("threads must be between 1 and {max}");
    }
    Ok(())
}

/// Best-effort cleanup of `handoff-*` dirs left behind by a crash or force-kill. Only called
/// after the port check passes, so no XMRange-launched xmrig can still be reading one of them.
fn sweep_stale_handoffs(run_dir: &Path) {
    let Ok(entries) = fs::read_dir(run_dir) else { return };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with("handoff-") {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

/// The parent guard: a tiny shell loop, started beside xmrig, that watches the process which
/// launched it (the app or the CLI). If that process dies without stopping xmrig (Force Quit,
/// `kill -9`, a crash), the guard stops xmrig within about a second: SIGINT, then SIGKILL after
/// 10 s. It acts only on the process whose arguments still name this launch's handoff config, so a
/// reused pid is never signalled; and it ends by itself once xmrig does. `caffeinate -w <xmrig>`
/// then ends too. A thread can't do this: it dies with the process it would have to outlive.
const GUARD_SCRIPT: &str = r#"a=$1; x=$2; h=$3
ours() { case "$(ps -ww -o args= -p "$x" 2>/dev/null)" in *"$h"*) return 0;; esac; return 1; }
while kill -0 "$a" 2>/dev/null && kill -0 "$x" 2>/dev/null; do sleep 1; done
kill -0 "$a" 2>/dev/null && exit 0
ours || exit 0
kill -INT "$x" 2>/dev/null
i=0; while [ $i -lt 20 ] && ours; do sleep 0.5; i=$((i+1)); done
ours && kill -KILL "$x" 2>/dev/null
exit 0"#;

pub(crate) fn spawn_guard(xmrig_pid: u32, handoff_config: &Path) -> Option<Child> {
    Command::new(orphan::SHELL)
        .args(["-c", GUARD_SCRIPT, "xmrange-guard"])
        .arg(std::process::id().to_string())
        .arg(xmrig_pid.to_string())
        .arg(handoff_config)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()
}

/// The guard is always on in release builds. Dev builds only: `XMRANGE_DEV_NO_PARENT_GUARD=1`
/// turns it off, so `tools/check-app-lifecycle.sh` can leave a real orphan for the next launch to
/// reap (the I2 recovery test).
pub(crate) fn parent_guard_enabled() -> bool {
    #[cfg(feature = "dev-diagnostics")]
    {
        std::env::var("XMRANGE_DEV_NO_PARENT_GUARD").as_deref() != Ok("1")
    }
    #[cfg(not(feature = "dev-diagnostics"))]
    {
        true
    }
}

impl Miner {
    pub fn launch(xmrig: &Path, opts: LaunchOptions) -> Result<Miner> {
        validate_threads(opts.config.threads, max_threads())?;
        let run_dir = crate::paths::data_dir().join("run");
        // A miner a dead session left running holds the port: stop it first (orphan.rs has the rules).
        let reaped = orphan::reap(&run_dir, xmrig);
        if let Some(note) = reaped.note() {
            eprintln!("{note}");
        }
        if !port_is_free(opts.api_port) {
            bail!("port {} is busy (is another xmrig still running?)", opts.api_port);
        }
        sweep_stale_handoffs(&run_dir);
        // The port is free, so no xmrig of ours is writing a log: finish any log a crash left raw
        // (a CLI run's `<secs>-<label>.log` included).
        #[cfg(feature = "dev-diagnostics")]
        if let Some(dir) = opts.dev_log.as_deref().and_then(Path::parent) {
            crate::redact::sweep_dev_logs(dir);
        }
        let token = crate::token::new_token();
        let handoff = Handoff::create(&run_dir)?;
        handoff.write(&opts.config.to_xmrig_json(opts.api_port, &token, opts.dev_log.as_deref()))?;
        let handoff_file = handoff.config_path().to_path_buf();

        // taskpolicy execs xmrig in place, so the pid recorded below is xmrig's own.
        let mut cmd = if opts.qos_utility {
            let mut c = Command::new("/usr/sbin/taskpolicy");
            c.args(["-c", "utility"]).arg(xmrig);
            c
        } else {
            Command::new(xmrig)
        };
        let mut child = cmd
            .args(build_argv(handoff.config_path()))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("could not start xmrig at {}", xmrig.display()))?;
        let pid = child.id();
        // Recorded at once, so even a crash during startup leaves the next launch a trail.
        let binary = fs::canonicalize(xmrig).unwrap_or_else(|_| xmrig.to_path_buf());
        let mut record =
            orphan::proc_id(pid).map(|id| RunRecord { xmrig: id, binary, caffeinate: None, guard: None, dev_log: opts.dev_log.clone() });
        if let Some(r) = &record {
            let _ = orphan::write_record(&run_dir, r);
        }

        let clamped = opts.e_core_clamp && {
            let r = clamp_child(&mut child, true);
            if let Err(e) = &r {
                eprintln!("warning: efficiency-core clamp failed ({e:#}); mining is NOT restricted to E-cores");
            }
            r.is_ok()
        };

        // Every error path from here on must kill and reap `child`: it is a live, mining xmrig
        // process, and `Child`'s Drop does not signal or reap — a raw early return would orphan
        // it with nothing left able to stop it. Route all of it through one cleanup path.
        let api = match Self::wait_until_ready(&mut child, opts.api_port, &token, &handoff) {
            Ok(api) => api,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                orphan::remove_record(&run_dir);
                return Err(e);
            }
        };
        // keep the Mac awake only once mining is real (spawned after readiness → nothing orphaned on failed launches)
        let caffeinate = Command::new(orphan::CAFFEINATE).args(["-i", "-w", &pid.to_string()]).spawn().ok();
        let guard = if parent_guard_enabled() { spawn_guard(pid, handoff.config_path()) } else { None };
        if let Some(r) = record.as_mut() {
            r.caffeinate = caffeinate.as_ref().and_then(|c| orphan::proc_id(c.id()));
            r.guard = guard.as_ref().and_then(|g| orphan::proc_id(g.id()));
            let _ = orphan::write_record(&run_dir, r);
        }

        Ok(Miner {
            child,
            caffeinate,
            api,
            config: opts.config,
            token,
            api_port: opts.api_port,
            dev_log: opts.dev_log,
            handoff: Some(handoff),
            handoff_file,
            stopped: false,
            guard,
            run_dir,
            e_core_clamp: clamped,
        })
    }

    /// Polls the loopback API until xmrig answers (or it exits/times out), then deletes the
    /// wallet-bearing handoff file. Never kills `child` itself — on `Err`, the caller does that
    /// as the single cleanup path for every failure mode below (exited, timed out, or the
    /// handoff delete failing after xmrig is already up and mining).
    fn wait_until_ready(child: &mut Child, api_port: u16, token: &str, handoff: &Handoff) -> Result<XmrigApi> {
        let api = XmrigApi::new(api_port, token);
        let deadline = Instant::now() + Duration::from_secs(90);
        loop {
            if let Some(status) = child.try_wait()? {
                bail!("xmrig exited during startup ({status})");
            }
            if api.summary().is_ok() {
                break;
            }
            if Instant::now() > deadline {
                bail!("xmrig api did not answer within 90 s");
            }
            thread::sleep(Duration::from_millis(500));
        }
        handoff.remove_file()?; // xmrig has read it; the API is up
        Ok(api)
    }

    /// Live thread change: PUT the full in-memory config. Workers restart (~4 ms); the dataset stays.
    pub fn set_threads(&mut self, n: u32) -> Result<()> {
        validate_threads(n, max_threads())?;
        self.reload_with(n, self.config.utility)
    }

    /// The one live reload path. `self.config` changes only once xmrig took it.
    fn reload_with(&mut self, threads: u32, utility: bool) -> Result<()> {
        let mut next = self.config.clone();
        next.threads = threads;
        next.utility = utility;
        self.api.put_config(&next.to_xmrig_json(self.api_port, &self.token, self.dev_log.as_deref()))?;
        self.config = next;
        Ok(())
    }

    pub fn threads(&self) -> u32 {
        self.config.threads
    }

    /// The background band on or off, live (spec §3 lever table: `taskpolicy -b/-B -p <pid>`).
    /// Err (xmrig gone, or taskpolicy refused) leaves the clamp as it was.
    pub fn set_e_core_clamp(&mut self, on: bool) -> Result<()> {
        clamp_child(&mut self.child, on)?;
        self.e_core_clamp = on;
        Ok(())
    }

    pub fn setup(&self) -> crate::levers::Setup {
        crate::levers::Setup { threads: self.config.threads, e_core_clamp: self.e_core_clamp, utility: self.config.utility }
    }

    /// Pull every lever that differs: the clamp first, then threads in one live reload.
    /// A failed reload undoes the clamp, best effort (`levers::apply_setup`).
    ///
    /// Callers (the T11 engine) must re-read `setup()` / `threads()` on **both** arms: on Err the
    /// rig is normally unchanged, but a failed undo can leave the new clamp on, and `setup()` is
    /// always what is actually running.
    pub fn apply(&mut self, s: &crate::levers::Setup) -> Result<()> {
        validate_threads(s.threads, max_threads())?;
        crate::levers::apply_setup(self, s)
    }

    pub fn summary(&self) -> Result<Summary> {
        self.api.summary()
    }

    /// Pause mining via xmrig's JSON-RPC API.
    pub fn pause(&self) -> Result<()> {
        self.api.pause()
    }

    /// Resume mining via xmrig's JSON-RPC API.
    pub fn resume(&self) -> Result<()> {
        self.api.resume()
    }

    /// Change wallet and worker, and apply via config update.
    pub fn set_identity(&mut self, wallet: &str, worker: &str) -> Result<()> {
        crate::wallet::check_address(wallet)?;
        crate::config::validate_worker(worker)?;
        let mut next = self.config.clone();
        next.wallet = wallet.to_string();
        next.worker = worker.to_string();
        self.api.put_config(&next.to_xmrig_json(self.api_port, &self.token, self.dev_log.as_deref()))?;
        self.config = next;
        Ok(())
    }

    /// Err once xmrig has exited on its own, with a short reason the app shows verbatim.
    pub fn ensure_running(&mut self) -> Result<()> {
        if let Some(status) = self.child.try_wait()? {
            bail!("xmrig stopped unexpectedly ({status})");
        }
        Ok(())
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    pub fn handoff_file(&self) -> Option<PathBuf> {
        Some(self.handoff_file.clone())
    }

    pub fn stop(mut self) -> Result<()> {
        self.shutdown();
        Ok(())
    }

    fn shutdown(&mut self) {
        if self.stopped {
            return;
        }
        self.stopped = true;
        // The guard goes first: xmrig is about to exit on purpose.
        if let Some(g) = self.guard.as_mut() {
            let _ = g.kill();
            let _ = g.wait();
        }
        if let Ok(None) = self.child.try_wait() {
            unsafe {
                libc::kill(self.child.id() as i32, libc::SIGINT);
            }
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline {
                if let Ok(Some(_)) = self.child.try_wait() {
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        if let Some(c) = self.caffeinate.as_mut() {
            let _ = c.kill();
            let _ = c.wait();
        }
        self.handoff.take();
        orphan::remove_record(&self.run_dir);
        #[cfg(feature = "dev-diagnostics")]
        if let Some(log) = &self.dev_log {
            let _ = crate::redact::redact_file_in_place(log);
        }
    }
}

impl crate::levers::LeverOps for Miner {
    fn setup(&self) -> crate::levers::Setup {
        Miner::setup(self)
    }
    fn set_clamp(&mut self, on: bool) -> Result<()> {
        self.set_e_core_clamp(on)
    }
    fn reload(&mut self, threads: u32, utility: bool) -> Result<()> {
        self.reload_with(threads, utility)
    }
}

/// `taskpolicy -b/-B` on our own child. `try_wait` first: while the child is unreaped its pid
/// stays ours (even as a zombie), and only `&mut Child` can reap it, so a recycled pid can never
/// be clamped. The fixed binary path (`orphan::TASKPOLICY`) keeps PATH out of it.
fn clamp_child(child: &mut Child, on: bool) -> Result<()> {
    let word = if on { "on" } else { "off" };
    if let Some(st) = child.try_wait()? {
        bail!("xmrig is not running ({st}); the e-core clamp was not turned {word}");
    }
    let mut cmd = Command::new(orphan::TASKPOLICY);
    cmd.args(crate::levers::clamp_args(on, child.id()));
    let (status, stderr) = run_bounded(&mut cmd, TASKPOLICY_TIMEOUT)
        .with_context(|| format!("could not run {} to turn the e-core clamp {word}", orphan::TASKPOLICY))?;
    if !status.success() {
        let why = stderr.trim().to_string();
        bail!("taskpolicy could not turn the e-core clamp {word} ({status}{})", if why.is_empty() { String::new() } else { format!(": {why}") });
    }
    Ok(())
}

/// How long `taskpolicy` may take before it is killed (Task 11 review M6): the engine loop runs
/// the clamp inline, so it must never wait on it without a limit.
const TASKPOLICY_TIMEOUT: Duration = Duration::from_secs(3);

/// Run a short local helper with a deadline, the same shape as `context::run_timeout`: poll,
/// and past the deadline kill and reap it (never a zombie). Returns its exit status and stderr.
fn run_bounded(cmd: &mut Command, timeout: Duration) -> Result<(std::process::ExitStatus, String)> {
    use std::io::Read;
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn()?;
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(st) = child.try_wait()? {
            break st;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("it took longer than {} s and was stopped", timeout.as_secs_f64());
        }
        thread::sleep(Duration::from_millis(10));
    };
    let mut err = String::new();
    if let Some(mut e) = child.stderr.take() {
        let _ = e.read_to_string(&mut err);
    }
    Ok((status, err))
}

impl Drop for Miner {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl std::fmt::Debug for Miner {
    /// Manual impl: never print wallet/token-bearing fields. Exists only so `Result<Miner, _>`
    /// satisfies `unwrap_err`'s `T: Debug` bound in tests.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Miner").field("pid", &self.child.id()).finish()
    }
}

impl crate::trial::Rig for Miner {
    fn threads(&self) -> u32 {
        Miner::threads(self)
    }
    fn set_threads(&mut self, n: u32) -> anyhow::Result<()> {
        Miner::set_threads(self, n)
    }
    fn hs10(&self) -> Option<f64> {
        self.summary().ok().and_then(|s| s.hs10())
    }
    fn other_cpu_pct(&self) -> Option<f64> {
        crate::context::other_cpu_pct(&[std::process::id(), self.pid()])
    }
    fn ensure_alive(&mut self) -> anyhow::Result<()> {
        if let Some(status) = self.child.try_wait()? {
            bail!("xmrig exited mid-trial ({status})");
        }
        Ok(())
    }
    fn context(&self) -> crate::context::Context {
        crate::context::read_context()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clamp_never_touches_a_child_that_has_exited() {
        let mut c = Command::new("/usr/bin/true").spawn().unwrap();
        c.wait().unwrap(); // reaped: its pid may belong to anyone now
        let e = clamp_child(&mut c, true).unwrap_err().to_string();
        assert!(e.contains("not running") && e.contains("not turned on"), "{e}");
    }

    /// I1: taskpolicy runs from its fixed path, never via a PATH lookup.
    #[test]
    fn taskpolicy_is_never_resolved_through_path() {
        let src = include_str!("miner.rs");
        let bare = ["Command::new(", "\"taskpolicy\")"].concat();
        assert!(!src.contains(&bare), "use orphan::TASKPOLICY");
        assert_eq!(orphan::TASKPOLICY, "/usr/sbin/taskpolicy");
    }

    /// Task 11 review M6: a helper that hangs is killed at its deadline and reaped.
    #[test]
    fn run_bounded_kills_a_slow_helper() {
        let t0 = Instant::now();
        let e = run_bounded(Command::new("/bin/sleep").arg("5"), Duration::from_millis(200)).unwrap_err();
        assert!(t0.elapsed() < Duration::from_secs(2), "{:?}", t0.elapsed());
        assert!(e.to_string().contains("longer than"), "{e}");
        let (st, err) = run_bounded(Command::new("/bin/sh").args(["-c", "echo nope >&2; exit 3"]), Duration::from_secs(3)).unwrap();
        assert_eq!((st.code(), err.trim()), (Some(3), "nope"));
    }
}

#[cfg(all(test, not(feature = "dev-diagnostics")))]
mod release_tests {
    /// Release builds: the parent guard can't be switched off, whatever the environment says.
    #[test]
    fn release_always_guards_xmrig() {
        std::env::set_var("XMRANGE_DEV_NO_PARENT_GUARD", "1");
        let on = super::parent_guard_enabled();
        std::env::remove_var("XMRANGE_DEV_NO_PARENT_GUARD");
        assert!(on);
    }
}
