//! The levers the optimizer pulls. THESIS 5: huge pages, MSR, affinity, asm, max-threads-hint and
//! yield do nothing on Apple Silicon, so they are not levers. Low Power Mode is a lever the user
//! pulls (spec §6), so it is context, never part of a `Setup`.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Setup {
    pub threads: u32,
    /// `taskpolicy -b`: the background band, which keeps xmrig on the E-cores at low clocks
    /// (THESIS 10, 18).
    pub e_core_clamp: bool,
    /// Mining threads at utility QoS (our xmrig patch, `cpu.qos`), switched live by a reload:
    /// lower clocks, politer to the user (runs/2026-10-03-yield-utility). Absent in older settings.
    #[serde(default)]
    pub utility: bool,
}

impl Setup {
    pub fn threads(n: u32) -> Setup {
        Setup { threads: n, e_core_clamp: false, utility: false }
    }

    /// "5 cores", "4 e-cores", "8 cores · utility": the words on screen, in the logs and in the
    /// diagnostics (ASD-STE100: one word for one thing; xmrig runs one thread on each core).
    pub fn label(&self) -> String {
        let word = if self.e_core_clamp { "e-core" } else { "core" };
        let mut s = format!("{} {word}{}", self.threads, if self.threads == 1 { "" } else { "s" });
        if self.utility {
            s.push_str(" · utility");
        }
        s
    }
}

/// `taskpolicy` arguments that put a running process into the background band (`-b`) or take it
/// out (`-B`). No privileges are needed for a process we started.
pub fn clamp_args(on: bool, pid: u32) -> [String; 3] {
    [if on { "-b" } else { "-B" }.to_string(), "-p".to_string(), pid.to_string()]
}


/// The two side effects `Miner::apply` orders, split out so the order is testable without xmrig.
pub(crate) trait LeverOps {
    fn setup(&self) -> Setup;
    /// `taskpolicy`; on Err nothing changed.
    fn set_clamp(&mut self, on: bool) -> anyhow::Result<()>;
    /// One live config reload, threads and QoS (the dataset stays, THESIS 17); on Err nothing changed.
    fn reload(&mut self, threads: u32, utility: bool) -> anyhow::Result<()>;
}

/// Clamp first (cheap, reversible), then the reload. A failed reload undoes the clamp, best
/// effort, so an Err leaves the old setup, or at worst a failed undo that `setup()` reports.
pub(crate) fn apply_setup<L: LeverOps>(l: &mut L, s: &Setup) -> anyhow::Result<()> {
    let old_clamp = l.setup().e_core_clamp;
    if s.e_core_clamp != old_clamp {
        l.set_clamp(s.e_core_clamp)?;
    }
    if l.setup().threads != s.threads || l.setup().utility != s.utility {
        if let Err(e) = l.reload(s.threads, s.utility) {
            if s.e_core_clamp != old_clamp {
                let _ = l.set_clamp(old_clamp);
            }
            return Err(e);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_read_like_the_trials_card() {
        assert_eq!(Setup::threads(1).label(), "1 core");
        assert_eq!(Setup { threads: 4, e_core_clamp: true, utility: false }.label(), "4 e-cores");
    }

    #[test]
    fn clamp_args_use_taskpolicy_background_on_and_off() {
        assert_eq!(clamp_args(true, 42), ["-b", "-p", "42"]);
        assert_eq!(clamp_args(false, 42), ["-B", "-p", "42"]);
    }

    /// A fake rig that logs each side effect and fails on request.
    struct Fake {
        cur: Setup,
        fail_clamp: bool,
        fail_reload: bool,
        log: Vec<String>,
    }

    impl Fake {
        fn new(cur: Setup) -> Fake {
            Fake { cur, fail_clamp: false, fail_reload: false, log: vec![] }
        }
    }

    impl LeverOps for Fake {
        fn setup(&self) -> Setup {
            self.cur
        }
        fn set_clamp(&mut self, on: bool) -> anyhow::Result<()> {
            self.log.push(format!("clamp {on}"));
            if self.fail_clamp {
                anyhow::bail!("taskpolicy failed");
            }
            self.cur.e_core_clamp = on;
            Ok(())
        }
        fn reload(&mut self, threads: u32, utility: bool) -> anyhow::Result<()> {
            self.log.push(format!("reload {threads}"));
            self.cur.utility = utility;
            if self.fail_reload {
                anyhow::bail!("api down");
            }
            self.cur.threads = threads;
            Ok(())
        }
    }

    #[test]
    fn apply_clamps_first_then_reloads() {
        let mut f = Fake::new(Setup::threads(5));
        let want = Setup { threads: 4, e_core_clamp: true, utility: false };
        apply_setup(&mut f, &want).unwrap();
        assert_eq!(f.log, ["clamp true", "reload 4"]);
        assert_eq!(f.setup(), want);
    }

    #[test]
    fn apply_pulls_only_the_levers_that_differ() {
        let mut f = Fake::new(Setup::threads(5));
        apply_setup(&mut f, &Setup::threads(5)).unwrap();
        apply_setup(&mut f, &Setup { e_core_clamp: true, ..Setup::threads(5) }).unwrap();
        assert_eq!(f.log, ["clamp true"], "no reload for a clamp-only change");
    }

    #[test]
    fn a_failed_clamp_changes_nothing() {
        let mut f = Fake::new(Setup::threads(5));
        f.fail_clamp = true;
        assert!(apply_setup(&mut f, &Setup { threads: 4, e_core_clamp: true, utility: false }).is_err());
        assert_eq!(f.log, ["clamp true"], "no reload after a failed clamp");
        assert_eq!(f.setup(), Setup::threads(5));
    }

    #[test]
    fn a_failed_reload_undoes_the_clamp() {
        let mut f = Fake::new(Setup::threads(5));
        f.fail_reload = true;
        assert!(apply_setup(&mut f, &Setup { threads: 4, e_core_clamp: true, utility: false }).is_err());
        assert_eq!(f.log, ["clamp true", "reload 4", "clamp false"]);
        assert_eq!(f.setup(), Setup::threads(5));
    }
}
