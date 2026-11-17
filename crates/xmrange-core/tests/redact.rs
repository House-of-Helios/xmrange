#![cfg(feature = "dev-diagnostics")]
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use xmrange_core::orphan::{reap, write_record, ProcId, RunRecord};
use xmrange_core::paths::scratch_dir;
use xmrange_core::redact::{redact_file_in_place, sweep_dev_logs};

const RAW: &str = "net use pool gulf.moneroocean.stream:20004  205.172.58.170\nnet [2001:db8::1]:20004 connected\n";

#[test]
fn a_dead_sessions_log_is_redacted_at_the_next_launch() {
    let dir = scratch_dir("reap-redact").unwrap();
    let run = dir.join("run");
    fs::create_dir_all(&run).unwrap();
    let log = dir.join("1790000000.log");
    fs::write(&log, RAW).unwrap();
    // xmrig was killed with the app (or the guard stopped it): the record names a pid that is gone.
    let rec = RunRecord { xmrig: ProcId { pid: 999_999, start_us: 1 }, binary: "/nonexistent/xmrig".into(), caffeinate: None, guard: None, dev_log: Some(log.clone()) };
    write_record(&run, &rec).unwrap();
    assert!(reap(&run, Path::new("/nonexistent/xmrig")).stale);
    let text = fs::read_to_string(&log).unwrap();
    assert!(!text.contains("205.172.58.170") && !text.contains("2001:db8"), "{text}");
    assert!(text.contains("gulf.moneroocean.stream:20004"), "hosts stay, IPs go");
}

#[test]
fn sweep_finishes_every_raw_log_and_skips_clean_ones() {
    let dir = scratch_dir("sweep").unwrap();
    fs::write(dir.join("1.log"), RAW).unwrap();
    fs::write(dir.join("2-curve.log"), "speed 10s/60s/15m 1826.6 n/a n/a H/s\n").unwrap();
    fs::write(dir.join("notes.txt"), RAW).unwrap();
    assert_eq!(sweep_dev_logs(&dir), 1, "only the raw .log is rewritten");
    assert!(!fs::read_to_string(dir.join("1.log")).unwrap().contains("205.172"));
    assert_eq!(fs::read_to_string(dir.join("notes.txt")).unwrap(), RAW, "not a log: untouched");
}

#[test]
fn redaction_is_atomic_and_keeps_the_mode() {
    let dir = scratch_dir("redact-atomic").unwrap();
    let log = dir.join("9.log");
    fs::write(&log, RAW).unwrap();
    fs::set_permissions(&log, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(redact_file_in_place(&log).unwrap());
    assert_eq!(fs::metadata(&log).unwrap().permissions().mode() & 0o777, 0o600);
    assert!(!redact_file_in_place(&log).unwrap(), "already clean: not rewritten");
    assert!(fs::read_dir(&dir).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().contains("redact-tmp")));
}
