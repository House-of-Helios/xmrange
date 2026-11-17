//! The user's limits (design §3, optional constraints): pause mining above a temperature, or while
//! on battery. Pure: the engine feeds it one tick a second and carries out what it says.
//!
//! Its rules, each from a failure thought through first:
//! - No flapping: heat pauses only after `HOT_TICKS` over the limit, and resumes only after
//!   `COOL_TICKS` at `COOL_MARGIN_F` under it.
//! - The user's own pause is never resumed by a limit.
//! - A user who resumes while paused by a limit overrides it until its condition clears once.
//! - Trials run unguarded (a pause would spoil the measurement); a limit pause in force when trials
//!   start is lifted, and says so.
//! - A missing reading (no temperature, no battery state) never triggers anything.
//! - A limit turned off while it holds mining paused resumes it at once.
use serde::{Deserialize, Serialize};

/// The limits as saved in settings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct GuardPrefs {
    /// Pause above this CPU temperature, °F. None: no heat limit.
    pub pause_above_f: Option<f64>,
    /// Pause while the Mac runs on battery.
    pub only_on_ac: bool,
}

pub const HOT_TICKS: u32 = 30;
pub const COOL_TICKS: u32 = 60;
pub const COOL_MARGIN_F: f64 = 9.0;
/// Ticks on battery before pausing: a cable wiggle is not a reason.
pub const BATTERY_TICKS: u32 = 5;
/// The range a heat limit may take, °F.
pub const PAUSE_ABOVE_F: std::ops::RangeInclusive<f64> = 100.0..=230.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reason {
    Heat,
    Battery,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GuardAct {
    Pause(String),
    Resume(String),
}

#[derive(Debug, Default)]
pub struct Guard {
    prefs: GuardPrefs,
    paused_for: Option<Reason>,
    hot: u32,
    cool: u32,
    on_battery: u32,
    /// The user resumed past a limit; it re-arms once its condition clears.
    overridden: bool,
}

impl Guard {
    pub fn new(prefs: GuardPrefs) -> Guard {
        Guard { prefs, ..Guard::default() }
    }

    /// New limits. Turning off the one that holds mining paused resumes it.
    pub fn set_prefs(&mut self, prefs: GuardPrefs) -> Option<GuardAct> {
        self.prefs = prefs;
        let lifted = match self.paused_for {
            Some(Reason::Heat) => prefs.pause_above_f.is_none(),
            Some(Reason::Battery) => !prefs.only_on_ac,
            None => false,
        };
        if lifted {
            self.paused_for = None;
            return Some(GuardAct::Resume("resumed · limit turned off".into()));
        }
        None
    }

    /// The user pressed resume. Past a limit pause, that overrides it until its condition clears.
    pub fn user_resumed(&mut self) {
        if self.paused_for.take().is_some() {
            self.overridden = true;
        }
    }

    /// Why mining is paused by a limit, for the screen.
    pub fn note(&self) -> Option<&'static str> {
        match self.paused_for {
            Some(Reason::Heat) => Some("paused · over your temperature limit"),
            Some(Reason::Battery) => Some("paused · on battery"),
            None => None,
        }
    }

    /// One second. `user_paused`: the user's own pause (never resumed here). `sweeping`: trials run.
    pub fn tick(&mut self, temp_f: Option<f64>, on_ac: Option<bool>, user_paused: bool, sweeping: bool) -> Option<GuardAct> {
        let limit = self.prefs.pause_above_f;
        let hot = matches!((temp_f, limit), (Some(t), Some(l)) if t >= l);
        let cooled = match (temp_f, limit) {
            (Some(t), Some(l)) => t <= l - COOL_MARGIN_F,
            _ => true,
        };
        self.hot = if hot { self.hot + 1 } else { 0 };
        self.cool = if cooled { self.cool + 1 } else { 0 };
        let battery = self.prefs.only_on_ac && on_ac == Some(false);
        self.on_battery = if battery { self.on_battery + 1 } else { 0 };
        // An override lasts until the condition behind it clears once.
        if self.overridden && cooled && !battery {
            self.overridden = false;
        }
        if sweeping {
            if self.paused_for.take().is_some() {
                return Some(GuardAct::Resume("resumed · trials run without your limits".into()));
            }
            return None;
        }
        match self.paused_for {
            Some(Reason::Heat) if self.cool >= COOL_TICKS => {
                self.paused_for = None;
                Some(GuardAct::Resume(format!("resumed · cooled to {:.0}°f", temp_f.unwrap_or(0.0))))
            }
            Some(Reason::Battery) if on_ac == Some(true) => {
                self.paused_for = None;
                Some(GuardAct::Resume("resumed · back on the power adapter".into()))
            }
            Some(_) => None,
            None if user_paused || self.overridden => None,
            None if self.hot >= HOT_TICKS => {
                self.paused_for = Some(Reason::Heat);
                Some(GuardAct::Pause(format!("paused · {:.0}°f, over your {:.0}°f limit", temp_f.unwrap_or(0.0), limit.unwrap_or(0.0))))
            }
            None if self.on_battery >= BATTERY_TICKS => {
                self.paused_for = Some(Reason::Battery);
                Some(GuardAct::Pause("paused · on battery (only on power adapter)".into()))
            }
            None => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(g: &mut Guard, n: u32, temp: Option<f64>, ac: Option<bool>, user_paused: bool, sweeping: bool) -> Vec<GuardAct> {
        (0..n).filter_map(|_| g.tick(temp, ac, user_paused, sweeping)).collect()
    }

    #[test]
    fn heat_pauses_after_30_seconds_and_resumes_only_well_cooled() {
        let mut g = Guard::new(GuardPrefs { pause_above_f: Some(200.0), only_on_ac: false });
        assert!(run(&mut g, HOT_TICKS - 1, Some(201.0), Some(true), false, false).is_empty(), "a short spike never pauses");
        assert_eq!(g.tick(Some(201.0), Some(true), false, false), Some(GuardAct::Pause("paused · 201°f, over your 200°f limit".into())));
        assert!(run(&mut g, 200, Some(195.0), Some(true), false, false).is_empty(), "4°f under is not cool enough: no flapping");
        assert!(run(&mut g, COOL_TICKS - 1, Some(190.0), Some(true), false, false).is_empty());
        assert_eq!(g.tick(Some(190.0), Some(true), false, false), Some(GuardAct::Resume("resumed · cooled to 190°f".into())));
    }

    #[test]
    fn the_users_pause_is_never_resumed_and_their_resume_overrides_until_it_clears() {
        let mut g = Guard::new(GuardPrefs { pause_above_f: Some(200.0), only_on_ac: false });
        assert!(run(&mut g, 100, Some(205.0), Some(true), true, false).is_empty(), "paused by the user: the limit stays out of it");
        run(&mut g, HOT_TICKS, Some(205.0), Some(true), false, false);
        assert!(g.note().is_some());
        g.user_resumed();
        assert!(run(&mut g, 300, Some(205.0), Some(true), false, false).is_empty(), "the user's resume overrides while still hot");
        g.tick(Some(180.0), Some(true), false, false); // cleared once: re-armed
        assert_eq!(run(&mut g, HOT_TICKS, Some(205.0), Some(true), false, false).len(), 1, "re-armed after it cooled");
    }

    #[test]
    fn battery_pauses_after_5_seconds_and_resumes_on_the_adapter() {
        let mut g = Guard::new(GuardPrefs { pause_above_f: None, only_on_ac: true });
        assert!(run(&mut g, BATTERY_TICKS - 1, None, Some(false), false, false).is_empty(), "a cable wiggle");
        assert!(matches!(g.tick(None, Some(false), false, false), Some(GuardAct::Pause(_))));
        assert_eq!(g.tick(None, Some(true), false, false), Some(GuardAct::Resume("resumed · back on the power adapter".into())));
    }

    #[test]
    fn trials_run_unguarded_and_missing_readings_never_trigger() {
        let mut g = Guard::new(GuardPrefs { pause_above_f: Some(150.0), only_on_ac: true });
        assert!(run(&mut g, 500, Some(210.0), Some(false), false, true).is_empty(), "trials: no pause");
        assert!(run(&mut g, 500, None, None, false, false).is_empty(), "no temperature, no battery state: nothing");
        run(&mut g, HOT_TICKS, Some(160.0), Some(true), false, false);
        assert_eq!(g.tick(Some(160.0), Some(true), false, true), Some(GuardAct::Resume("resumed · trials run without your limits".into())));
    }

    #[test]
    fn turning_a_limit_off_while_it_holds_mining_paused_resumes() {
        let mut g = Guard::new(GuardPrefs { pause_above_f: Some(150.0), only_on_ac: false });
        run(&mut g, HOT_TICKS, Some(160.0), Some(true), false, false);
        assert_eq!(g.set_prefs(GuardPrefs::default()), Some(GuardAct::Resume("resumed · limit turned off".into())));
        assert_eq!(g.set_prefs(GuardPrefs::default()), None);
        assert_eq!(GuardPrefs::default(), GuardPrefs { pause_above_f: None, only_on_ac: false }, "off unless the user sets one");
    }
}
