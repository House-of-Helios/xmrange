//! A helper process that runs beside xmrig: p2pool, and Arti (Tor). One lifecycle for both.
//! - Started with xmrig's parent guard (`miner::spawn_guard`), so a crashed or force-quit app
//!   never leaves one running.
//! - Recorded in `<data dir>/run/<name>.pid`, so a leftover from a dead session is stopped on the
//!   next start, but only if it is still this build's binary.
//! - Its output goes through a pipe into memory: the last lines are kept for error messages
//!   and for the few events read from it. Lines on stderr (p2pool's warnings and errors) also
//!   queue for the tape, until `take_errors` reads them. Both p2pool and Arti can print peer or relay IPs, so
//!   only dev builds also write it to a log file (doctrine: no IPs at rest in release).
//! - Stopped with SIGINT, then SIGKILL after 10 s (`STOP_GRACE`; monerod sets 2 minutes).
use crate::orphan::{self, ProcId};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const KEEP_LINES: usize = 50;
/// A stop's wait between SIGINT and SIGKILL, unless the helper sets its own (`with_grace`).
pub const STOP_GRACE: Duration = Duration::from_secs(10);
/// The error a cancelled start-up returns: the user stopped, so the engine ends quietly.
pub const CANCELLED: &str = "start-up cancelled";

/// A value for p2pool's params file or Arti's TOML: double-quoted, with `\` and `"` escaped. Both
/// formats read the rest (spaces, non-ASCII) as-is; Rust's `{:?}` would add `\u{…}` escapes neither
/// understands.
pub fn quoted(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

#[derive(Serialize, Deserialize)]
struct Record {
    proc: ProcId,
    binary: PathBuf,
}

pub struct Sidecar {
    name: &'static str,
    child: Child,
    guard: Option<Child>,
    record: PathBuf,
    lines: Arc<Mutex<VecDeque<String>>>,
    errs: Arc<Mutex<VecDeque<String>>>,
    marked: Arc<Mutex<VecDeque<String>>>,
    stopped: bool,
    /// How long a stop waits after SIGINT before SIGKILL.
    grace: Duration,
}

/// Appends `line`, dropping the oldest once `KEEP_LINES` are held.
fn keep_line(q: &Mutex<VecDeque<String>>, line: &str) {
    let mut k = q.lock().unwrap_or_else(|e| e.into_inner());
    if k.len() == KEEP_LINES {
        k.pop_front();
    }
    k.push_back(line.to_string());
}

impl Sidecar {
    /// `marker`: a path that appears in the child's own arguments (its config file), so the
    /// parent guard can tell this launch's process from a reused pid. `marks`: output lines that
    /// contain any of these also queue for `take_marked` (p2pool's payout line).
    pub fn spawn(name: &'static str, binary: &Path, args: &[String], marker: &Path, marks: &'static [&'static str]) -> Result<Sidecar> {
        let run_dir = crate::paths::data_dir().join("run");
        fs::create_dir_all(&run_dir)?;
        let record = run_dir.join(format!("{name}.pid"));
        reap(&record, binary);
        if !binary.exists() {
            bail!("{name} is missing ({})", binary.display());
        }
        let mut child = Command::new(binary)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("starting {name}"))?;
        let lines = Arc::new(Mutex::new(VecDeque::new()));
        let errs = Arc::new(Mutex::new(VecDeque::new()));
        let marked = Arc::new(Mutex::new(VecDeque::new()));
        #[cfg(feature = "dev-diagnostics")]
        let log = crate::paths::data_dir().join("logs").join(format!("{name}.log"));
        #[cfg(feature = "dev-diagnostics")]
        let _ = fs::create_dir_all(log.parent().unwrap_or(Path::new(".")));
        let pipes = [(child.stdout.take().map(|p| Box::new(p) as Box<dyn std::io::Read + Send>), false), (child.stderr.take().map(|p| Box::new(p) as Box<dyn std::io::Read + Send>), true)];
        for (pipe, is_err) in pipes.into_iter().filter_map(|(p, e)| p.map(|p| (p, e))) {
            let keep = lines.clone();
            let errs = errs.clone();
            let marked = marked.clone();
            #[cfg(feature = "dev-diagnostics")]
            let log = log.clone();
            thread::spawn(move || {
                #[cfg(feature = "dev-diagnostics")]
                let mut file = fs::OpenOptions::new().create(true).append(true).open(&log).ok();
                for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                    #[cfg(feature = "dev-diagnostics")]
                    if let Some(f) = file.as_mut() {
                        use std::io::Write;
                        let _ = writeln!(f, "{line}");
                    }
                    keep_line(&keep, &line);
                    if is_err {
                        keep_line(&errs, &line);
                    }
                    if marks.iter().any(|m| line.contains(m)) {
                        keep_line(&marked, &line);
                    }
                }
            });
        }
        let guard = if crate::miner::parent_guard_enabled() { crate::miner::spawn_guard(child.id(), marker) } else { None };
        if let Some(proc) = orphan::proc_id(child.id()) {
            let binary = fs::canonicalize(binary).unwrap_or_else(|_| binary.to_path_buf());
            let _ = fs::write(&record, serde_json::to_vec(&Record { proc, binary })?);
        }
        Ok(Sidecar { name, child, guard, record, lines, errs, marked, stopped: false, grace: STOP_GRACE })
    }

    /// Some(why) once the process has exited: its status and last output line.
    pub fn exited(&mut self) -> Option<String> {
        match self.child.try_wait() {
            Ok(Some(status)) => Some(format!("{} exited ({status}){}", self.name, self.last_line().map(|l| format!(": {l}")).unwrap_or_default())),
            _ => None,
        }
    }

    pub fn last_line(&self) -> Option<String> {
        self.lines.lock().unwrap_or_else(|e| e.into_inner()).back().cloned()
    }

    /// The output lines matching a `marks` needle since the last call, oldest first.
    pub fn take_marked(&self) -> Vec<String> {
        self.marked.lock().unwrap_or_else(|e| e.into_inner()).drain(..).collect()
    }

    /// The kept output lines (both pipes), shared: they stay readable after the process stops.
    pub fn lines_handle(&self) -> Arc<Mutex<VecDeque<String>>> {
        self.lines.clone()
    }

    /// The stderr lines printed since the last call, oldest first (at most `KEEP_LINES`).
    pub fn take_errors(&self) -> Vec<String> {
        self.errs.lock().unwrap_or_else(|e| e.into_inner()).drain(..).collect()
    }

    /// Whether any kept output line contains `needle`.
    pub fn said(&self, needle: &str) -> bool {
        self.lines.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|l| l.contains(needle))
    }

    /// Waits until `ready()` holds, the process exits, `limit` passes, or `cancel()` says stop
    /// (checked every 250 ms, so a Stop or quit during start-up is never held up).
    pub fn wait_until(&mut self, limit: Duration, cancel: &dyn Fn() -> bool, mut ready: impl FnMut(&Sidecar) -> bool) -> Result<()> {
        let deadline = Instant::now() + limit;
        loop {
            if ready(self) {
                return Ok(());
            }
            if cancel() {
                bail!(CANCELLED);
            }
            if let Some(why) = self.exited() {
                bail!(why);
            }
            if Instant::now() >= deadline {
                bail!("{} did not come up within {} s{}", self.name, limit.as_secs(), self.last_line().map(|l| format!(" (last: {l})")).unwrap_or_default());
            }
            thread::sleep(Duration::from_millis(250));
        }
    }

    /// A longer stop for a helper that must close a database first (monerod: up to 2 minutes).
    pub fn with_grace(mut self, grace: Duration) -> Sidecar {
        self.grace = grace;
        self
    }

    /// The helper's process id.
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        if self.stopped {
            return;
        }
        self.stopped = true;
        if let Some(g) = self.guard.as_mut() {
            let _ = g.kill();
            let _ = g.wait();
        }
        if let Ok(None) = self.child.try_wait() {
            unsafe { libc::kill(self.child.id() as i32, libc::SIGINT) };
            let deadline = Instant::now() + self.grace;
            while Instant::now() < deadline {
                if let Ok(Some(_)) = self.child.try_wait() {
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        let _ = fs::remove_file(&self.record);
    }
}

impl Drop for Sidecar {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Stops what a dead session left behind under `record`, if it is still this build's binary.
fn reap(record: &Path, binary: &Path) {
    let Ok(bytes) = fs::read(record) else { return };
    let _ = fs::remove_file(record);
    let Ok(rec) = serde_json::from_slice::<Record>(&bytes) else { return };
    let expected = fs::canonicalize(binary).unwrap_or_else(|_| binary.to_path_buf());
    let ours = orphan::proc_info(rec.proc.pid).is_some_and(|i| !i.zombie && i.start_us == rec.proc.start_us && i.path.as_deref() == Some(expected.as_path()));
    if ours && rec.binary == expected {
        orphan::stop_process(rec.proc);
    }
}
