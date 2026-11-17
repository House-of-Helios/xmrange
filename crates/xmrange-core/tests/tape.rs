//! The tape's pure rules (TapeState, clock_hms). Not gated on dev-diagnostics, so they run in
//! plain `cargo test --workspace` too (final review M9). The engine tests that launch processes
//! stay in engine.rs, behind the gate.
use xmrange_core::api::Summary;
use xmrange_core::engine::{clock_hms, TapeState};

fn summary(json: &str) -> Summary {
    serde_json::from_str(json).unwrap()
}

/// The tape must never leak an IP: reusing `redact::redact_ips` (already tested elsewhere) is
/// stronger than a hand-rolled check here, and a no-op redaction means the message had no IP.
fn assert_no_ip(msg: &str) {
    assert_eq!(xmrange_core::redact::redact_ips(msg), msg, "tape line leaked an IP: {msg}");
}

#[test]
fn tape_reports_connected_and_new_job_on_first_summary_with_algo() {
    let mut tape = TapeState::default();
    let s = summary(
        r#"{"algo":"rx/0","hashrate":{"total":[null,null],"highest":null},
            "results":{"shares_good":0,"shares_total":0,"diff_current":0},
            "connection":{"ip":"205.172.58.170"}}"#,
    );
    let lines = tape.update(&s, "12:00:00");
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert_eq!(lines[0].tag, "net");
    assert_eq!(lines[0].msg, "connected to pool");
    assert_eq!(lines[1].tag, "net");
    assert_eq!(lines[1].msg, "new job algo rx/0");
    for l in &lines {
        assert_no_ip(&l.msg);
    }
}

#[test]
fn tape_does_not_reconnect_on_repeated_summaries_with_same_algo() {
    let mut tape = TapeState::default();
    let s = summary(r#"{"algo":"rx/0","hashrate":{"total":[1234.5]},"results":{"shares_good":0,"shares_total":0,"diff_current":0}}"#);
    tape.update(&s, "12:00:00");
    let lines = tape.update(&s, "12:00:01");
    assert!(lines.is_empty(), "{lines:?}");
}

#[test]
fn tape_reports_new_job_on_algo_change_without_reconnecting() {
    let mut tape = TapeState::default();
    let first = summary(r#"{"algo":"rx/0","hashrate":{"total":[1234.5]},"results":{"shares_good":0,"shares_total":0,"diff_current":0}}"#);
    tape.update(&first, "12:00:00");
    let second = summary(r#"{"algo":"rx/1","hashrate":{"total":[1234.5]},"results":{"shares_good":0,"shares_total":0,"diff_current":0}}"#);
    let lines = tape.update(&second, "12:00:01");
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0].tag, "net");
    assert_eq!(lines[0].msg, "new job algo rx/1");
}

#[test]
fn tape_reports_difficulty_change_but_not_the_first_nonzero_baseline() {
    let mut tape = TapeState::default();
    let baseline = summary(r#"{"algo":"rx/0","hashrate":{"total":[1234.5]},"results":{"shares_good":0,"shares_total":0,"diff_current":50000}}"#);
    let first_lines = tape.update(&baseline, "12:00:00");
    assert!(!first_lines.iter().any(|l| l.msg.starts_with("difficulty")), "{first_lines:?}");

    let changed = summary(r#"{"algo":"rx/0","hashrate":{"total":[1234.5]},"results":{"shares_good":0,"shares_total":0,"diff_current":60000}}"#);
    let lines = tape.update(&changed, "12:00:01");
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0].tag, "net");
    assert_eq!(lines[0].msg, "difficulty 60000");
}

#[test]
fn tape_reports_accepted_and_rejected_shares() {
    let mut tape = TapeState::default();
    let start = summary(r#"{"algo":"rx/0","hashrate":{"total":[1234.5]},"results":{"shares_good":0,"shares_total":0,"diff_current":50000}}"#);
    tape.update(&start, "12:00:00");

    let accepted = summary(r#"{"algo":"rx/0","hashrate":{"total":[1234.5]},"results":{"shares_good":1,"shares_total":1,"diff_current":50000}}"#);
    let lines = tape.update(&accepted, "12:00:01");
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0].tag, "cpu");
    assert_eq!(lines[0].msg, "accepted (1/0) diff 50000");

    let rejected = summary(r#"{"algo":"rx/0","hashrate":{"total":[1234.5]},"results":{"shares_good":1,"shares_total":2,"diff_current":50000}}"#);
    let lines = tape.update(&rejected, "12:00:02");
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0].tag, "cpu");
    assert_eq!(lines[0].msg, "rejected (1/1)");
}

#[test]
fn tape_reports_speed_exactly_on_the_60th_update() {
    let mut tape = TapeState::default();
    let s = summary(r#"{"hashrate":{"total":[1826.3,1810.5],"highest":1900.0},"results":{"shares_good":0,"shares_total":0,"diff_current":0}}"#);
    for i in 1..60 {
        let lines = tape.update(&s, "12:00:00");
        assert!(lines.is_empty(), "update {i} should be silent, got {lines:?}");
    }
    let lines = tape.update(&s, "12:00:59");
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0].tag, "miner");
    assert_eq!(lines[0].msg, "speed 10s/60s 1826.3 1810.5 h/s max 1900.0");
    for l in &lines {
        assert_no_ip(&l.msg);
    }
}

#[test]
fn tape_speed_line_uses_na_when_hs60_and_highest_are_unknown() {
    let mut tape = TapeState::default();
    let s = summary(r#"{"hashrate":{"total":[1826.3]},"results":{"shares_good":0,"shares_total":0,"diff_current":0}}"#);
    for _ in 1..60 {
        tape.update(&s, "12:00:00");
    }
    let lines = tape.update(&s, "12:00:59");
    assert_eq!(lines[0].msg, "speed 10s/60s 1826.3 n/a h/s max n/a");
}

#[test]
fn clock_hms_matches_hh_mm_ss() {
    let t = clock_hms();
    let bytes = t.as_bytes();
    assert_eq!(bytes.len(), 8, "{t}");
    assert_eq!(bytes[2], b':', "{t}");
    assert_eq!(bytes[5], b':', "{t}");
    for (i, b) in bytes.iter().enumerate() {
        if i != 2 && i != 5 {
            assert!(b.is_ascii_digit(), "{t}");
        }
    }
}

#[test]
fn tape_says_when_xmrig_loses_the_pool_once_per_new_loss() {
    let mut tape = TapeState::default();
    let no_job = |n: u64| summary(&format!(r#"{{"algo":null,"hashrate":{{"total":[0.0,0.0,0.0],"highest":null}},"connection":{{"ip":"127.0.0.1","failures":{n}}}}}"#));
    assert!(tape.update(&no_job(0), "12:00:00").is_empty(), "no job and no failure: nothing to say");
    let lines = tape.update(&no_job(2), "12:00:05");
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!((lines[0].tag.as_str(), lines[0].msg.as_str()), ("error", "lost the pool connection (2 times)"));
    assert_no_ip(&lines[0].msg);
    assert!(tape.update(&no_job(2), "12:00:06").is_empty(), "the same count again is quiet");
}
