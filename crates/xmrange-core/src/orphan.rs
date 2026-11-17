//! Orphaned-miner recovery (final review I2).
//!
//! Every launched xmrig is recorded in `<data dir>/run/xmrig.pid`: its pid and process start time,
//! the resolved xmrig binary, and the pids and start times of its helpers (the `caffeinate -w` that
//! keeps the Mac awake and the parent guard, see `miner.rs`). A clean shutdown removes the file.
//!
//! If the owning process died without cleaning up (Force Quit, `kill -9`, a crash), the next launch
//! finds the file and `reap` decides, from live process facts only:
//! - the recorded pid still exists, with the recorded start time (so not a reused pid),
//! - its executable is the recorded xmrig binary, which is still this build's xmrig binary,
//! - its arguments name a handoff config in this data dir's `run/` (so it is this data dir's miner),
//! - and its parent is launchd (pid 1): nobody alive owns it any more.
//!
//! Only then is it stopped (SIGINT, then SIGKILL after 10 s). A record whose process is gone or is
//! someone else's is stale and just deleted. A miner that is ours but still has a live parent (a
//! running `xmrange` CLI trial, say) is left alone, and so is its record.
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

/// The run record's file name inside `<data dir>/run/`.
pub const RECORD_FILE: &str = "xmrig.pid";

/// A process as the kernel identifies it over time: pid plus start time (microseconds since the
/// epoch), so a pid the system reused later never matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcId {
    pub pid: u32,
    pub start_us: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRecord {
    pub xmrig: ProcId,
    /// The xmrig executable, resolved (symlinks and `..` gone).
    pub binary: PathBuf,
    #[serde(default)]
    pub caffeinate: Option<ProcId>,
    #[serde(default)]
    pub guard: Option<ProcId>,
    /// The session's dev xmrig log (dev builds), so the next launch can redact it if this session
    /// dies unredacted.
    #[serde(default)]
    pub dev_log: Option<PathBuf>,
}

/// What the kernel says about a live process right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcInfo {
    pub start_us: u64,
    pub ppid: u32,
    pub zombie: bool,
    pub path: Option<PathBuf>,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Ours, and nobody owns it: stop it.
    Reap,
    /// Gone, or not ours: delete the record, touch nothing.
    Stale,
    /// Ours, and a live process still owns it: leave it and its record alone.
    Owned,
}

/// The pure decision for the recorded xmrig. `expected_binary` is this build's xmrig, resolved.
pub fn judge_xmrig(rec: &RunRecord, info: Option<&ProcInfo>, expected_binary: &Path, run_dir: &Path) -> Verdict {
    let Some(info) = info else { return Verdict::Stale };
    if info.zombie || info.start_us != rec.xmrig.start_us {
        return Verdict::Stale;
    }
    if rec.binary != expected_binary || info.path.as_deref() != Some(rec.binary.as_path()) {
        return Verdict::Stale;
    }
    let handoff = run_dir.join("handoff-");
    let handoff = handoff.to_string_lossy();
    if !info.args.iter().any(|a| a.starts_with(handoff.as_ref())) {
        return Verdict::Stale;
    }
    if info.ppid != 1 {
        return Verdict::Owned;
    }
    Verdict::Reap
}

/// The pure decision for a recorded helper (caffeinate or the guard) of the xmrig `xmrig_pid`:
/// the same identity rules, with the helper's executable and an argument naming that xmrig.
pub fn judge_helper(id: ProcId, info: Option<&ProcInfo>, executable: &Path, xmrig_pid: u32) -> Verdict {
    let Some(info) = info else { return Verdict::Stale };
    if info.zombie || info.start_us != id.start_us || info.path.as_deref() != Some(executable) {
        return Verdict::Stale;
    }
    let pid = xmrig_pid.to_string();
    if !info.args.iter().any(|a| *a == pid) {
        return Verdict::Stale;
    }
    if info.ppid != 1 {
        return Verdict::Owned;
    }
    Verdict::Reap
}

pub const CAFFEINATE: &str = "/usr/bin/caffeinate";
/// Fixed path, never PATH lookup: the e-core clamp runs it against our own xmrig's pid.
pub const TASKPOLICY: &str = "/usr/sbin/taskpolicy";
/// The parent guard's shell. Not /bin/sh: on macOS that is a trampoline that re-executes another
/// shell, so the running guard's executable would not be the one recorded.
pub const SHELL: &str = "/bin/bash";

pub fn record_path(run_dir: &Path) -> PathBuf {
    run_dir.join(RECORD_FILE)
}

pub fn write_record(run_dir: &Path, rec: &RunRecord) -> std::io::Result<()> {
    fs::create_dir_all(run_dir)?;
    let tmp = run_dir.join(".xmrig.pid.tmp");
    let json = serde_json::to_string(rec).map_err(std::io::Error::other)?;
    let written = fs::write(&tmp, json).and_then(|_| fs::rename(&tmp, record_path(run_dir)));
    if written.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    written
}

pub fn remove_record(run_dir: &Path) {
    let _ = fs::remove_file(record_path(run_dir));
}

/// What `reap` did, for the caller to report (the app puts it on the tape).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReapOutcome {
    /// An orphaned xmrig was found and stopped.
    pub xmrig: bool,
    /// Orphaned helpers (caffeinate, guard) stopped.
    pub helpers: u32,
    /// A stale record was deleted.
    pub stale: bool,
}

impl ReapOutcome {
    /// The tape line for a reap, if anything was stopped.
    pub fn note(&self) -> Option<&'static str> {
        (self.xmrig || self.helpers > 0).then_some("stopped a miner left running by a previous session")
    }
}

/// Stop what a dead session left running in `run_dir`, per the rules at the top of this file.
/// `expected_binary` is this build's xmrig (`paths::xmrig_binary()`). Blocks while it stops
/// (usually well under a second; at most about 12 s per process).
pub fn reap(run_dir: &Path, expected_binary: &Path) -> ReapOutcome {
    let mut out = ReapOutcome::default();
    let path = record_path(run_dir);
    let Ok(text) = fs::read_to_string(&path) else { return out };
    let Ok(rec) = serde_json::from_str::<RunRecord>(&text) else {
        let _ = fs::remove_file(&path);
        out.stale = true;
        return out;
    };
    let expected = fs::canonicalize(expected_binary).unwrap_or_else(|_| expected_binary.to_path_buf());
    let verdict = judge_xmrig(&rec, proc_info(rec.xmrig.pid).as_ref(), &expected, run_dir);
    if verdict == Verdict::Owned {
        return out;
    }
    if verdict == Verdict::Reap && stop_process(rec.xmrig) {
        out.xmrig = true;
    }
    // The recorded xmrig is stopped or gone, so its log is finished. A session that died never
    // ran the redaction its `shutdown` would have (roadmap: Plan 3), so do it now.
    #[cfg(feature = "dev-diagnostics")]
    if let Some(log) = &rec.dev_log {
        let _ = crate::redact::redact_file_in_place(log);
    }
    // Helpers are judged on their own facts: caffeinate normally exits by itself once xmrig does.
    let helpers = [(rec.caffeinate, CAFFEINATE), (rec.guard, SHELL)];
    for (id, exe) in helpers {
        let Some(id) = id else { continue };
        if judge_helper(id, proc_info(id.pid).as_ref(), Path::new(exe), rec.xmrig.pid) == Verdict::Reap && stop_process(id) {
            out.helpers += 1;
        }
    }
    let _ = fs::remove_file(&path);
    out.stale = !out.xmrig && out.helpers == 0;
    out
}

/// Whether `id` still names a live (non-zombie) process.
pub fn still_running(id: ProcId) -> bool {
    matches!(proc_info(id.pid), Some(i) if !i.zombie && i.start_us == id.start_us)
}

/// SIGINT, up to 10 s to exit, then SIGKILL and up to 2 s more. Every signal is sent only after
/// re-checking the start time, so a pid reused meanwhile is never signalled. True once it's gone.
pub(crate) fn stop_process(id: ProcId) -> bool {
    let wait_gone = |limit: Duration| {
        let deadline = Instant::now() + limit;
        while Instant::now() < deadline {
            if !still_running(id) {
                return true;
            }
            thread::sleep(Duration::from_millis(100));
        }
        !still_running(id)
    };
    if !still_running(id) {
        return true;
    }
    unsafe { libc::kill(id.pid as i32, libc::SIGINT) };
    if wait_gone(Duration::from_secs(10)) {
        return true;
    }
    if still_running(id) {
        unsafe { libc::kill(id.pid as i32, libc::SIGKILL) };
    }
    wait_gone(Duration::from_secs(2))
}

/// The kernel's facts about `pid`: start time, parent, zombie state, executable and arguments.
/// None if there is no such process (or it isn't ours to inspect).
pub fn proc_info(pid: u32) -> Option<ProcInfo> {
    if pid == 0 {
        return None;
    }
    let mut bsd: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let got = unsafe { libc::proc_pidinfo(pid as libc::c_int, libc::PROC_PIDTBSDINFO, 0, &mut bsd as *mut _ as *mut libc::c_void, size) };
    if got != size {
        return None;
    }
    const SZOMB: u32 = 5; // <sys/proc.h>
    let start_us = bsd.pbi_start_tvsec.saturating_mul(1_000_000).saturating_add(bsd.pbi_start_tvusec);
    let (exec_path, args) = proc_args(pid).unwrap_or_default();
    let path = proc_path(pid).or(exec_path);
    Some(ProcInfo { start_us, ppid: bsd.pbi_ppid, zombie: bsd.pbi_status == SZOMB, path, args })
}

/// A process id with its start time, for the run record.
pub fn proc_id(pid: u32) -> Option<ProcId> {
    proc_info(pid).map(|i| ProcId { pid, start_us: i.start_us })
}

fn proc_path(pid: u32) -> Option<PathBuf> {
    let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    let n = unsafe { libc::proc_pidpath(pid as libc::c_int, buf.as_mut_ptr() as *mut libc::c_void, buf.len() as u32) };
    if n <= 0 {
        return None;
    }
    buf.truncate(n as usize);
    Some(PathBuf::from(String::from_utf8_lossy(&buf).into_owned()))
}

/// KERN_PROCARGS2: argc, the exec path, NUL padding, then argc arguments (then the environment,
/// which is never read).
fn proc_args(pid: u32) -> Option<(Option<PathBuf>, Vec<String>)> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as libc::c_int];
    let mut size: libc::size_t = 256 * 1024;
    let mut buf = vec![0u8; size];
    let rc = unsafe { libc::sysctl(mib.as_mut_ptr(), 3, buf.as_mut_ptr() as *mut libc::c_void, &mut size, std::ptr::null_mut(), 0) };
    if rc != 0 || size < 4 {
        return None;
    }
    buf.truncate(size);
    Some(parse_procargs(&buf))
}

/// Parses a KERN_PROCARGS2 buffer into (exec path, argv). Pure, for tests.
pub fn parse_procargs(buf: &[u8]) -> (Option<PathBuf>, Vec<String>) {
    if buf.len() < 4 {
        return (None, Vec::new());
    }
    let argc = i32::from_ne_bytes([buf[0], buf[1], buf[2], buf[3]]).max(0) as usize;
    let rest = &buf[4..];
    let end = rest.iter().position(|b| *b == 0).unwrap_or(rest.len());
    let exec = (end > 0).then(|| PathBuf::from(String::from_utf8_lossy(&rest[..end]).into_owned()));
    let mut i = end;
    while i < rest.len() && rest[i] == 0 {
        i += 1;
    }
    let args = rest[i..].split(|b| *b == 0).take(argc).map(|s| String::from_utf8_lossy(s).into_owned()).collect();
    (exec, args)
}

#[cfg(test)]
mod tests {
    use super::*;

    const XMRIG: &str = "/repo/vendor/xmrig-mo/build/xmrig";
    const RUN: &str = "/repo/_devdata/run";

    fn rec() -> RunRecord {
        RunRecord {
            xmrig: ProcId { pid: 4242, start_us: 1_000_000 },
            binary: PathBuf::from(XMRIG),
            caffeinate: Some(ProcId { pid: 4250, start_us: 1_500_000 }),
            guard: None,
            dev_log: None,
        }
    }

    /// The live facts of the orphan the record describes: same start, our binary, our handoff, parent launchd.
    fn orphan() -> ProcInfo {
        ProcInfo {
            start_us: 1_000_000,
            ppid: 1,
            zombie: false,
            path: Some(PathBuf::from(XMRIG)),
            args: vec![XMRIG.into(), "-c".into(), format!("{RUN}/handoff-0123abcd/config.json"), "--no-color".into()],
        }
    }

    fn judge(info: Option<&ProcInfo>) -> Verdict {
        judge_xmrig(&rec(), info, Path::new(XMRIG), Path::new(RUN))
    }

    #[test]
    fn reaps_our_orphan() {
        assert_eq!(judge(Some(&orphan())), Verdict::Reap);
    }

    #[test]
    fn gone_or_zombie_is_stale() {
        assert_eq!(judge(None), Verdict::Stale);
        assert_eq!(judge(Some(&ProcInfo { zombie: true, ..orphan() })), Verdict::Stale);
    }

    #[test]
    fn reused_pid_is_stale() {
        assert_eq!(judge(Some(&ProcInfo { start_us: 9_999_999, ..orphan() })), Verdict::Stale);
    }

    #[test]
    fn another_binary_is_stale() {
        let other = ProcInfo { path: Some(PathBuf::from("/usr/local/bin/xmrig")), ..orphan() };
        assert_eq!(judge(Some(&other)), Verdict::Stale, "someone else's xmrig");
        assert_eq!(judge(Some(&ProcInfo { path: None, ..orphan() })), Verdict::Stale, "unknown path");
        // A record naming a binary this build doesn't use is never acted on, even if it matches.
        let moved = judge_xmrig(&rec(), Some(&orphan()), Path::new("/Applications/XMRange.app/Contents/MacOS/xmrig"), Path::new(RUN));
        assert_eq!(moved, Verdict::Stale);
    }

    #[test]
    fn another_data_dirs_miner_is_stale() {
        let args = vec![XMRIG.into(), "-c".into(), "/repo/_devdata/lifecycle/data/run/handoff-9/config.json".into()];
        assert_eq!(judge(Some(&ProcInfo { args, ..orphan() })), Verdict::Stale);
        assert_eq!(judge(Some(&ProcInfo { args: vec![], ..orphan() })), Verdict::Stale, "args unreadable");
    }

    #[test]
    fn a_miner_with_a_live_parent_is_left_alone() {
        assert_eq!(judge(Some(&ProcInfo { ppid: 777, ..orphan() })), Verdict::Owned);
    }

    #[test]
    fn helpers_follow_the_same_rules() {
        let id = ProcId { pid: 4250, start_us: 1_500_000 };
        let caff = ProcInfo {
            start_us: 1_500_000,
            ppid: 1,
            zombie: false,
            path: Some(PathBuf::from(CAFFEINATE)),
            args: vec!["caffeinate".into(), "-i".into(), "-w".into(), "4242".into()],
        };
        let exe = Path::new(CAFFEINATE);
        assert_eq!(judge_helper(id, Some(&caff), exe, 4242), Verdict::Reap);
        assert_eq!(judge_helper(id, None, exe, 4242), Verdict::Stale);
        assert_eq!(judge_helper(id, Some(&ProcInfo { start_us: 7, ..caff.clone() }), exe, 4242), Verdict::Stale);
        assert_eq!(judge_helper(id, Some(&caff), exe, 5555), Verdict::Stale, "waits on another pid");
        assert_eq!(judge_helper(id, Some(&caff), Path::new(SHELL), 4242), Verdict::Stale, "another executable");
        assert_eq!(judge_helper(id, Some(&ProcInfo { ppid: 99, ..caff }), exe, 4242), Verdict::Owned);
    }

    #[test]
    fn parses_procargs2() {
        let mut buf = 3i32.to_ne_bytes().to_vec();
        buf.extend_from_slice(b"/bin/xmrig\0\0\0\0xmrig\0-c\0/r/run/handoff-1/config.json\0HOME=/x\0");
        let (exec, args) = parse_procargs(&buf);
        assert_eq!(exec, Some(PathBuf::from("/bin/xmrig")));
        assert_eq!(args, vec!["xmrig", "-c", "/r/run/handoff-1/config.json"], "the environment is never read");
        assert_eq!(parse_procargs(&[1, 0]), (None, vec![]));
    }

    #[test]
    fn reads_this_process() {
        let me = proc_info(std::process::id()).expect("own process info");
        assert!(!me.zombie);
        assert_eq!(me.ppid, std::os::unix::process::parent_id());
        assert_eq!(me.path.as_deref().map(|p| p.canonicalize().unwrap()), Some(std::env::current_exe().unwrap().canonicalize().unwrap()));
        assert!(!me.args.is_empty());
        assert_eq!(proc_id(std::process::id()).map(|i| i.start_us), Some(me.start_us));
        assert!(proc_info(0).is_none());
    }

    #[test]
    fn record_round_trips_and_notes_only_a_real_reap() {
        let json = serde_json::to_string(&rec()).unwrap();
        assert_eq!(serde_json::from_str::<RunRecord>(&json).unwrap(), rec());
        let note = ReapOutcome { xmrig: true, ..Default::default() }.note();
        assert_eq!(note, Some("stopped a miner left running by a previous session"));
        assert_eq!(ReapOutcome { stale: true, ..Default::default() }.note(), None);
    }
}
