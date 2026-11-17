//! One thread owns the Ledger's connection, and every write goes through it: trials, one at a
//! time, each answered to its caller. The engine's 1 Hz samples no longer come here (ruling P5-O1:
//! the Ledger keeps trials, not power history), so there is no sample queue, no batching and no
//! rollup.
//!
//! A refused write (a busy Ledger, a full disk, any other error) is reported to the one caller it
//! belongs to; the next request tries again. Nothing is retried here, so nothing is written twice.
use super::{Ledger, TrialRecord};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Queue depth, in requests. Callers wait for their answer, so a few is plenty.
pub const QUEUE_DEPTH: usize = 16;
/// How long `trial` and `trials` wait, in total, to be queued and answered.
const REPLY_WAIT: Duration = Duration::from_secs(10);
/// How often a request retries a full queue within `REPLY_WAIT`.
const RETRY_EVERY: Duration = Duration::from_millis(2);
/// How long `close` waits for the thread to finish the request in hand (T7 review m4). Quitting
/// must never hang on a slow disk: past this the thread is left to finish on its own, or to be
/// ended with the process, which SQLite's transactions make safe.
pub const CLOSE_WAIT: Duration = Duration::from_secs(5);

type Reply<T> = Sender<Result<T, String>>;

/// Why a request to the Ledger thread has no confirmed answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AskError {
    /// The Ledger is closed or closing. The request was not carried out.
    Closed,
    /// The queue stayed full for the whole wait. The request was never queued.
    Busy,
    /// Queued, but not answered in time (T7 review m3). It may still be carried out later, so
    /// it is unconfirmed, not lost: callers report it as such and never retry, which could
    /// write a trial twice.
    Unanswered,
    /// The Ledger thread answered with an error.
    Failed(String),
}

impl AskError {
    /// True when the request may still land (`Unanswered`).
    pub fn unconfirmed(&self) -> bool {
        matches!(self, AskError::Unanswered)
    }
}

impl std::fmt::Display for AskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AskError::Closed => write!(f, "the Ledger is closed"),
            AskError::Busy => write!(f, "the Ledger is busy"),
            AskError::Unanswered => write!(f, "the Ledger did not answer in time (not confirmed; it may still be written)"),
            AskError::Failed(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for AskError {}

enum Msg {
    Trial(Box<TrialRecord>, Option<String>, Reply<i64>),
    Trials(u32, Reply<(u64, Vec<TrialRecord>)>),
    ClearTrials(Reply<()>),
    Payout(f64, crate::p2pool::Payout, String, Reply<bool>),
    Payouts(String, Reply<super::PayoutTotals>),
    Close,
}

/// A cheap, cloneable way in to the Ledger thread. Send + Sync.
#[derive(Clone)]
pub struct LedgerHandle {
    tx: SyncSender<Msg>,
}

impl LedgerHandle {
    /// Queues a request and waits for its answer, `REPLY_WAIT` at most in all. A full queue is
    /// retried rather than waited on, so the deadline holds even while the queue is full.
    fn ask<T>(&self, make: impl FnOnce(Reply<T>) -> Msg) -> Result<T, AskError> {
        let deadline = Instant::now() + REPLY_WAIT;
        let (tx, rx) = mpsc::channel();
        let mut msg = make(tx);
        loop {
            match self.tx.try_send(msg) {
                Ok(()) => break,
                Err(TrySendError::Disconnected(_)) => return Err(AskError::Closed),
                Err(TrySendError::Full(m)) if Instant::now() < deadline => {
                    msg = m;
                    thread::sleep(RETRY_EVERY);
                }
                Err(TrySendError::Full(_)) => return Err(AskError::Busy),
            }
        }
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(answer) => answer.map_err(AskError::Failed),
            Err(RecvTimeoutError::Timeout) => Err(AskError::Unanswered),
            // The reply was dropped unanswered: the thread ended, or the request was discarded
            // with the queue at close. Either way it was not carried out.
            Err(RecvTimeoutError::Disconnected) => Err(AskError::Closed),
        }
    }

    /// Writes one trial; `wallet` is stored only as this Ledger's salted tag. Returns its row id.
    /// `Err(e)` with `e.unconfirmed()` means the trial may still be written: never retry it.
    pub fn trial(&self, rec: TrialRecord, wallet: Option<String>) -> Result<i64, AskError> {
        self.ask(|r| Msg::Trial(Box::new(rec), wallet, r))
    }
    /// (total trial count, the newest `limit` trials, newest first).
    pub fn trials(&self, limit: u32) -> Result<(u64, Vec<TrialRecord>), String> {
        self.ask(|r| Msg::Trials(limit, r)).map_err(|e| e.to_string())
    }

    /// Records one p2pool payout to `wallet` (stored as its salted tag). Ok(false): already there.
    pub fn payout(&self, t: f64, payout: crate::p2pool::Payout, wallet: String) -> Result<bool, AskError> {
        self.ask(|r| Msg::Payout(t, payout, wallet, r))
    }

    /// All-time payouts to `wallet`.
    pub fn payouts(&self, wallet: String) -> Result<super::PayoutTotals, String> {
        self.ask(|r| Msg::Payouts(wallet, r)).map_err(|e| e.to_string())
    }

    /// Deletes every trial (the user's "clear trials").
    pub fn clear_trials(&self) -> Result<(), String> {
        self.ask(Msg::ClearTrials).map_err(|e| e.to_string())
    }
}

/// Owns the Ledger thread. `close` (or drop) finishes the request in hand, then joins the
/// thread, waiting `CLOSE_WAIT` at most.
pub struct LedgerWriter {
    handle: LedgerHandle,
    join: Option<JoinHandle<()>>,
    /// Disconnects when the thread ends (its Sender is dropped last), so a close can wait on it
    /// with a limit, which `JoinHandle::join` can't.
    ended: Receiver<()>,
}

impl LedgerWriter {
    pub fn spawn(ledger: Ledger) -> LedgerWriter {
        let (tx, rx) = mpsc::sync_channel(QUEUE_DEPTH);
        let (ended_tx, ended) = mpsc::channel::<()>();
        let join = thread::Builder::new()
            .name("xmrange-ledger".into())
            .spawn(move || {
                let _ended = ended_tx; // dropped after `run` returns, the Ledger already closed
                run(ledger, rx)
            })
            .expect("spawn the Ledger thread");
        LedgerWriter { handle: LedgerHandle { tx }, join: Some(join), ended }
    }
    pub fn handle(&self) -> LedgerHandle {
        self.handle.clone()
    }
    /// Ends the thread once the request in hand is done; handles then refuse. Waits `CLOSE_WAIT`
    /// at most. `false`: the thread was still writing when the wait ran out.
    pub fn close(self) -> bool {
        self.close_within(CLOSE_WAIT)
    }
    /// `close`, waiting `limit` at most.
    pub fn close_within(mut self, limit: Duration) -> bool {
        self.shutdown(limit)
    }
    fn shutdown(&mut self, limit: Duration) -> bool {
        let Some(j) = self.join.take() else { return true };
        let deadline = Instant::now() + limit;
        // Never a blocking send: the queue could be full behind a slow write.
        let mut msg = Msg::Close;
        loop {
            match self.handle.tx.try_send(msg) {
                Ok(()) | Err(TrySendError::Disconnected(_)) => break,
                Err(TrySendError::Full(m)) if Instant::now() < deadline => {
                    msg = m;
                    thread::sleep(RETRY_EVERY);
                }
                Err(TrySendError::Full(_)) => return false, // detached; ends with the process
            }
        }
        match self.ended.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => {
                let _ = j.join(); // already finished: returns at once
                true
            }
            Err(RecvTimeoutError::Timeout) => false, // detached: dropping `j` doesn't stop it
        }
    }
}

impl Drop for LedgerWriter {
    fn drop(&mut self) {
        self.shutdown(CLOSE_WAIT);
    }
}

/// The Ledger thread: answers each request in order until Close, then refuses what is left.
fn run(mut ledger: Ledger, rx: Receiver<Msg>) {
    for msg in rx.iter() {
        match msg {
            Msg::Trial(rec, wallet, reply) => {
                let _ = reply.send(write_trial(&mut ledger, *rec, wallet.as_deref()));
            }
            Msg::Trials(limit, reply) => {
                // M7: the count matches what `trial_views` actually lists (scored, complete
                // trials), not every row `trial_count` sees (partial ones included).
                let got = ledger.scored_trial_count().and_then(|n| Ok((n, ledger.trials(limit)?)));
                let _ = reply.send(got.map_err(|e| e.to_string()));
            }
            Msg::ClearTrials(reply) => {
                let _ = reply.send(ledger.clear_trials().map_err(|e| e.to_string()));
            }
            Msg::Payout(t, payout, wallet, reply) => {
                let _ = reply.send(ledger.insert_payout(t, payout, &wallet).map_err(|e| e.to_string()));
            }
            Msg::Payouts(wallet, reply) => {
                let _ = reply.send(ledger.payout_totals(&wallet).map_err(|e| e.to_string()));
            }
            Msg::Close => break,
        }
    }
    // Requests that raced in behind Close are refused, and the Receiver dropped at once (T7 review
    // m1), so a request sent from here on fails at `try_send` instead of waiting on a thread that
    // is ending.
    let late: Vec<Msg> = rx.try_iter().collect();
    drop(rx);
    for msg in late {
        match msg {
            Msg::Trial(_, _, reply) => drop(reply.send(Err("the Ledger is closing".into()))),
            Msg::Trials(_, reply) => drop(reply.send(Err("the Ledger is closing".into()))),
            Msg::ClearTrials(reply) => drop(reply.send(Err("the Ledger is closing".into()))),
            Msg::Payout(_, _, _, reply) => drop(reply.send(Err("the Ledger is closing".into()))),
            Msg::Payouts(_, reply) => drop(reply.send(Err("the Ledger is closing".into()))),
            Msg::Close => {}
        }
    }
}

fn write_trial(ledger: &mut Ledger, mut rec: TrialRecord, wallet: Option<&str>) -> Result<i64, String> {
    if let Some(w) = wallet {
        rec.wallet_tag = Some(ledger.wallet_tag(w));
    }
    ledger.insert_trial(&rec).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::{test_dir, LedgerKey};
    use std::path::Path;
    use std::process::{Child, Stdio};

    fn fresh(tag: &str) -> (std::path::PathBuf, LedgerKey) {
        let p = test_dir(tag).join("l.sqlite");
        let k = LedgerKey::generate();
        drop(Ledger::open(&p, &k).unwrap());
        (p, k)
    }

    fn a_trial(label: &str) -> TrialRecord {
        TrialRecord::from_result(&crate::trial::TrialResult {
            threads: 5, settled: true, settle_s: 60, partial: true, score: None, ticks: vec![],
            context_start: Default::default(), context_end: Default::default(),
        }, label, "cli", None, false)
    }

    #[test]
    fn the_handle_crosses_threads() {
        fn send_sync<T: Send + Sync>() {}
        send_sync::<LedgerHandle>();
    }

    #[test]
    fn trials_round_trip_through_the_writer_with_a_wallet_tag() {
        let (p, k) = fresh("wtrials");
        let w = LedgerWriter::spawn(Ledger::open(&p, &k).unwrap());
        let h = w.handle();
        // Scored and complete (final review M7): `trials()`'s count is `scored_trial_count`, which
        // a partial, unscored trial wouldn't move.
        let score = crate::trial::Score {
            n: 180, hs: 1826.0, cpu_w: 3.8, ram_w: 0.8, sys_w: Some(9.1), temp_c: Some(55.0),
            hw_sys: Some(220.0), hw_cpu: Some(480.0), hw_sys_ci95: Some(3.2), other_cpu_pct: Some(2.0), noisy: false,
            hs_ci95: None, hw_cpu_ci95: None, thermal_max: None,
        };
        let rec = TrialRecord::from_result(&crate::trial::TrialResult {
            threads: 5, settled: true, settle_s: 60, partial: false, score: Some(score), ticks: vec![],
            context_start: Default::default(), context_end: Default::default(),
        }, "t", "cli", None, false);
        // Synthetic address only (project hard rule): never the real default wallet, even in tests.
        let wallet = format!("4{}", "B".repeat(94));
        let id = h.trial(rec, Some(wallet)).unwrap();
        let (n, got) = h.trials(5).unwrap();
        assert_eq!((n, got[0].id), (1, Some(id)));
        assert_eq!(got[0].wallet_tag.as_ref().map(String::len), Some(16));
        w.close();
    }

    #[test]
    fn a_closed_ledger_refuses_politely() {
        let (p, k) = fresh("closed");
        let w = LedgerWriter::spawn(Ledger::open(&p, &k).unwrap());
        let h = w.handle();
        assert!(w.close());
        assert_eq!(h.trial(a_trial("late"), None), Err(AskError::Closed));
        assert!(h.trials(1).is_err());
        assert_eq!(Ledger::open(&p, &k).unwrap().trial_count().unwrap(), 0);
    }

    /// A write error refuses that one trial, to its caller; it never ends the session's writing.
    #[test]
    fn a_write_error_refuses_the_trial_and_the_next_one_lands() {
        let (p, k) = fresh("refused");
        let w = LedgerWriter::spawn(Ledger::open(&p, &k).unwrap());
        let h = w.handle();
        let other = Ledger::open(&p, &k).unwrap();
        other.conn.execute_batch("CREATE TRIGGER refuse BEFORE INSERT ON trials BEGIN SELECT RAISE(ABORT, 'refused by the test'); END;").unwrap();
        let e = h.trial(a_trial("a"), None).unwrap_err();
        assert!(!e.unconfirmed() && e.to_string().contains("refused by the test"), "{e}");
        other.conn.execute_batch("DROP TRIGGER refuse").unwrap();
        assert!(h.trial(a_trial("b"), None).is_ok(), "writing goes on after a refusal");
        assert!(w.close());
        assert_eq!(other.trial_count().unwrap(), 1);
    }

    /// T7 review m5: another process holding the write lock (the CLI recording a trial while
    /// the app mines) refuses a trial for now; it never ends the session's writing.
    #[test]
    fn a_busy_ledger_refuses_a_trial_and_writing_goes_on() {
        let (p, k) = fresh("busy");
        let l = Ledger::open(&p, &k).unwrap();
        l.conn.busy_timeout(Duration::from_millis(50)).unwrap();
        let w = LedgerWriter::spawn(l);
        let h = w.handle();
        let other = Ledger::open(&p, &k).unwrap();
        other.conn.execute_batch("BEGIN IMMEDIATE").unwrap();
        let refused = h.trial(a_trial("a"), None).unwrap_err();
        assert!(!refused.unconfirmed() && refused.to_string().contains("busy"), "{refused}");
        other.conn.execute_batch("ROLLBACK").unwrap();
        assert!(h.trial(a_trial("b"), None).is_ok(), "trials are written again once the lock is let go");
        assert!(w.close());
        assert_eq!(other.trial_count().unwrap(), 1);
    }

    /// T7 review m4: quitting never waits on the Ledger past the limit. A writer still busy
    /// when the wait runs out finishes its work on its own.
    #[test]
    fn close_is_bounded_and_a_detached_writer_still_finishes() {
        let (p, k) = fresh("slowclose");
        let l = Ledger::open(&p, &k).unwrap();
        l.conn.busy_timeout(Duration::from_millis(1500)).unwrap();
        let w = LedgerWriter::spawn(l);
        let h = w.handle();
        let other = Ledger::open(&p, &k).unwrap();
        other.conn.execute_batch("BEGIN IMMEDIATE").unwrap();
        let writer = thread::spawn(move || h.trial(a_trial("slow"), None));
        thread::sleep(Duration::from_millis(100)); // the trial is queued ahead of the close
        let started = Instant::now();
        assert!(!w.close_within(Duration::from_millis(200)), "the write is waiting on the lock");
        assert!(started.elapsed() < Duration::from_millis(1000), "close waited {:?}", started.elapsed());
        other.conn.execute_batch("ROLLBACK").unwrap();
        assert!(writer.join().unwrap().is_ok(), "the detached writer finished the trial");
        assert_eq!(other.trial_count().unwrap(), 1);
    }

    const CHILD_PATH: &str = "XMRANGE_LEDGER_CRASH_CHILD";
    const CHILD_KEY: &str = "XMRANGE_LEDGER_CRASH_KEY";
    /// The child's own stop, in case its parent died without killing it (F9).
    const CHILD_MAX: Duration = Duration::from_secs(60);

    /// Not a test on its own (hence ignored, F9): `sigkill_mid_write_leaves_a_whole_ledger` re-runs
    /// this test binary with only this test selected and the env set, and it writes trials labelled
    /// 0, 1, 2, … until it is killed. It stops by itself after `CHILD_MAX`, or as soon as its
    /// parent is gone.
    #[test]
    #[ignore = "the child process of sigkill_mid_write_leaves_a_whole_ledger, which runs it"]
    fn crash_child_writer() {
        let (Ok(path), Ok(key)) = (std::env::var(CHILD_PATH), std::env::var(CHILD_KEY)) else {
            eprintln!("crash_child_writer runs only as sigkill_mid_write_leaves_a_whole_ledger's child");
            return;
        };
        let parent = std::os::unix::process::parent_id();
        let w = LedgerWriter::spawn(Ledger::open(Path::new(&path), &LedgerKey::from_hex(&key).unwrap()).unwrap());
        let h = w.handle();
        let started = Instant::now();
        let mut i = 0u64;
        while started.elapsed() < CHILD_MAX && std::os::unix::process::parent_id() == parent {
            h.trial(a_trial(&i.to_string()), None).unwrap();
            i += 1;
        }
    }

    /// Kills and reaps the child on every way out of the parent test, a panic included (F9).
    struct KillOnDrop(Child);

    impl Drop for KillOnDrop {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    fn sigkill_mid_write_leaves_a_whole_ledger() {
        use std::os::unix::process::ExitStatusExt;
        let (p, k) = fresh("crash");
        let mut child = KillOnDrop(
            std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "ledger::writer::tests::crash_child_writer", "--ignored", "--nocapture", "--test-threads=1"])
                .env(CHILD_PATH, &p)
                .env(CHILD_KEY, k.to_hex())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        // Wait for committed trials, read through our own connection (WAL allows it).
        let reader = Ledger::open(&p, &k).unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut seen = 0;
        while seen < 200 {
            assert!(Instant::now() < deadline, "the child never wrote");
            assert!(child.0.try_wait().unwrap().is_none(), "the child stopped before it was killed");
            thread::sleep(Duration::from_millis(20));
            seen = reader.trial_count().unwrap();
        }
        drop(reader);
        child.0.kill().unwrap(); // SIGKILL, mid-stream
        let status = child.0.wait().unwrap();
        assert_eq!(status.signal(), Some(9), "the child died of SIGKILL, still writing: {status:?}");

        let l = Ledger::open(&p, &k).expect("a killed writer leaves a Ledger that opens");
        let ok: String = l.conn.query_row("PRAGMA integrity_check", [], |r| r.get(0)).unwrap();
        assert_eq!(ok, "ok");
        let bad_pages = l.conn.prepare("PRAGMA cipher_integrity_check").unwrap().query_map([], |r| r.get::<_, String>(0)).unwrap().count();
        assert_eq!(bad_pages, 0, "every page's HMAC checks out");
        let (n, distinct, max): (i64, i64, i64) = l
            .conn
            .query_row("SELECT count(*), count(DISTINCT label), MAX(CAST(label AS INTEGER)) FROM trials", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .unwrap();
        assert!(n >= seen as i64, "committed trials survive: {n} after the kill, {seen} seen before it");
        assert_eq!(distinct, n, "no trial written twice");
        assert_eq!(max, n - 1, "no gap: {n} trials, the last labelled {max}");
        eprintln!("sigkill: {seen} trials seen before the kill, {n} whole trials after; integrity ok; 0 bad HMAC pages");
    }
}
