//! Low Power Mode, the user-assisted lever (design §6). The state is read without privileges
//! (`context::read_low_power`, `pmset -g`) and watched every `POLL`. Plan 5 adds the Settings-pane
//! choice for its walkthrough to this module. Nothing here needs sudo, signing or entitlements.
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Design §6 asks for 5 s or faster. One `pmset -g` costs a few milliseconds.
pub const POLL: Duration = Duration::from_secs(2);

/// 0 unknown, 1 off, 2 on. Written by the watch, read by the engine each tick.
static LATEST: AtomicU8 = AtomicU8::new(0);

pub fn store(v: Option<bool>) {
    LATEST.store(
        match v {
            None => 0,
            Some(false) => 1,
            Some(true) => 2,
        },
        Ordering::Relaxed,
    );
}

pub fn latest() -> Option<bool> {
    match LATEST.load(Ordering::Relaxed) {
        1 => Some(false),
        2 => Some(true),
        _ => None,
    }
}

/// How long `stop()` waits for the watch thread to notice the flag and finish its current
/// iteration. `context::read_low_power`'s own child is bounded well inside this, so the normal
/// case returns almost at once; if a read is still in flight past the deadline, `stop()` detaches
/// instead of blocking shutdown on it (the thread exits on its own once that read returns).
const STOP_DEADLINE: Duration = Duration::from_secs(4);

pub struct WatchHandle {
    stop: Arc<AtomicBool>,
    done: mpsc::Receiver<()>,
    join: JoinHandle<()>,
}

impl WatchHandle {
    pub fn stop(self) {
        self.stop.store(true, Ordering::SeqCst);
        match self.done.recv_timeout(STOP_DEADLINE) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => {
                let _ = self.join.join();
            }
            Err(RecvTimeoutError::Timeout) => {
                // A read is still in flight past the deadline: don't block shutdown on it.
                // Dropping the JoinHandle (unlike `.join()`) never blocks.
            }
        }
    }
}

/// Reads every `period`. Calls `on_change` with the first value, then only when it changes.
/// Dropping the handle leaves the watch running for the process's life (the app's case).
pub fn spawn_watch<R, F>(period: Duration, mut read: R, mut on_change: F) -> WatchHandle
where
    R: FnMut() -> Option<bool> + Send + 'static,
    F: FnMut(Option<bool>) + Send + 'static,
{
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let (done_tx, done_rx) = mpsc::channel();
    let join = thread::spawn(move || {
        let mut last: Option<Option<bool>> = None;
        while !flag.load(Ordering::SeqCst) {
            let now = read();
            if last != Some(now) {
                last = Some(now);
                on_change(now);
            }
            thread::sleep(period);
        }
        let _ = done_tx.send(());
    });
    WatchHandle { stop, done: done_rx, join }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn store_and_latest_round_trip() {
        for v in [Some(true), Some(false), None] {
            store(v);
            assert_eq!(latest(), v);
        }
    }

    #[test]
    fn watch_reports_the_first_value_then_changes_only() {
        let values = [Some(false), Some(false), Some(true), Some(true), None];
        let mut i = 0usize;
        let read = move || {
            let v = values[i.min(values.len() - 1)];
            i += 1;
            v
        };
        let (tx, rx) = mpsc::channel();
        let h = spawn_watch(Duration::from_millis(5), read, move |v| {
            let _ = tx.send(v);
        });
        let got: Vec<_> = (0..3).map(|_| rx.recv_timeout(Duration::from_secs(2)).unwrap()).collect();
        h.stop();
        assert_eq!(got, vec![Some(false), Some(true), None]);
        assert!(rx.try_recv().is_err(), "an unchanged value is never re-sent");
    }

    #[test]
    fn stop_returns_promptly_even_while_a_read_is_in_flight() {
        let read = || {
            thread::sleep(Duration::from_secs(10));
            Some(true)
        };
        let h = spawn_watch(Duration::from_millis(5), read, |_| {});
        thread::sleep(Duration::from_millis(50)); // let the slow read start
        let start = Instant::now();
        h.stop();
        // Bounded by STOP_DEADLINE (4 s), not by the read's own 10 s sleep.
        assert!(start.elapsed() < Duration::from_secs(6), "stop() must not wait out an in-flight read");
    }
}
