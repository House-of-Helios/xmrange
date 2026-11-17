use serde::{Deserialize, Serialize};
use std::io::Read as _;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThermalPressure {
    Nominal,
    Moderate,
    Heavy,
    Trapping,
    Sleeping,
}

impl ThermalPressure {
    pub fn from_level(level: u64) -> Option<Self> {
        match level {
            0 => Some(ThermalPressure::Nominal),
            1 => Some(ThermalPressure::Moderate),
            2 => Some(ThermalPressure::Heavy),
            3 => Some(ThermalPressure::Trapping),
            4 => Some(ThermalPressure::Sleeping),
            _ => None,
        }
    }

    pub fn level(self) -> u8 {
        match self {
            ThermalPressure::Nominal => 0,
            ThermalPressure::Moderate => 1,
            ThermalPressure::Heavy => 2,
            ThermalPressure::Trapping => 3,
            ThermalPressure::Sleeping => 4,
        }
    }
}

mod notify_ffi {
    use std::os::raw::{c_char, c_int};
    extern "C" {
        pub fn notify_register_check(name: *const c_char, out_token: *mut c_int) -> u32;
        pub fn notify_get_state(token: c_int, state64: *mut u64) -> u32;
        pub fn notify_cancel(token: c_int) -> u32;
    }
}

/// macOS thermal pressure (0 nominal … 4 sleeping), sudoless, from the Darwin notify key.
pub fn thermal_pressure() -> Option<ThermalPressure> {
    let name = std::ffi::CString::new("com.apple.system.thermalpressurelevel").ok()?;
    let (mut token, mut state) = (0i32, 0u64);
    unsafe {
        if notify_ffi::notify_register_check(name.as_ptr(), &mut token) != 0 {
            return None;
        }
        let ok = notify_ffi::notify_get_state(token, &mut state) == 0;
        notify_ffi::notify_cancel(token);
        if !ok {
            return None;
        }
    }
    ThermalPressure::from_level(state)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Context {
    pub low_power_mode: Option<bool>,
    pub on_ac: Option<bool>,
    pub battery_pct: Option<u8>,
    pub brightness_pct: Option<f64>,
    #[serde(default)]
    pub thermal: Option<ThermalPressure>,
}

/// Low Power Mode from `pmset -g`. Most Macs print `lowpowermode 0|1`. Macs that also offer High
/// Power Mode print `powermode 0|1|2` instead (automatic, low, high). `lowpowermode` wins if both
/// appear. Any other value under either key (a future third state, truncated output, garbage) is
/// `None`, never guessed as off.
pub fn parse_lowpower(pmset_g: &str) -> Option<bool> {
    let value = |key: &str| {
        pmset_g.lines().find_map(|l| {
            let mut it = l.split_whitespace();
            (it.next()? == key).then(|| it.next()).flatten().map(str::to_string)
        })
    };
    if let Some(v) = value("lowpowermode") {
        return match v.as_str() {
            "1" => Some(true),
            "0" => Some(false),
            _ => None,
        };
    }
    match value("powermode").as_deref() {
        Some("1") => Some(true),
        Some("0") | Some("2") => Some(false),
        _ => None,
    }
}

pub fn parse_batt(pmset_batt: &str) -> (Option<bool>, Option<u8>) {
    let on_ac = pmset_batt.lines().next().and_then(|l| {
        if l.contains("'AC Power'") { Some(true) } else if l.contains("'Battery Power'") { Some(false) } else { None }
    });
    let pct = pmset_batt.lines().skip(1).find_map(|l| {
        let i = l.find('%')?;
        let digits: String = l[..i].chars().rev().take_while(|c| c.is_ascii_digit()).collect::<Vec<_>>().into_iter().rev().collect();
        digits.parse().ok()
    });
    (on_ac, pct)
}

pub fn parse_brightness(ioreg: &str) -> Option<f64> {
    let s = &ioreg[ioreg.find("\"brightness\"={")?..];
    let field = |k: &str| -> Option<f64> {
        let i = s.find(&format!("\"{k}\"="))? + k.len() + 3;
        s[i..].chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().ok()
    };
    let (max, val) = (field("max")?, field("value")?);
    (max > 0.0).then(|| (100.0 * val / max * 10.0).round() / 10.0)
}

pub fn parse_other_cpu(ps: &str, exclude: &[u32]) -> f64 {
    ps.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let pid: u32 = it.next()?.parse().ok()?;
            let cpu: f64 = it.next()?.parse().ok()?;
            (!exclude.contains(&pid)).then_some(cpu)
        })
        .sum()
}

/// Bound on any `run()` child: the watchdog pattern this codebase already uses for xmrig
/// (`miner.rs::wait_until_ready`, and the `--bench` gotcha in CLAUDE.md). A hung `pmset` must
/// never stall the 2 s watch, or its shutdown.
const RUN_TIMEOUT: Duration = Duration::from_secs(3);

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    run_timeout(cmd, args, RUN_TIMEOUT)
}

/// `run`'s bounded implementation, factored out so tests can hand it a short deadline against a
/// genuinely slow child (the seam `run` alone, with its fixed timeout, doesn't offer).
fn run_timeout(cmd: &str, args: &[&str], timeout: Duration) -> Option<String> {
    let mut child = Command::new(cmd).args(args).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait(); // reap: never leave a zombie behind
                    return None;
                }
                thread::sleep(Duration::from_millis(20));
            }
            Err(_) => return None,
        }
    }
    let mut buf = String::new();
    child.stdout.take()?.read_to_string(&mut buf).ok()?;
    Some(buf)
}

/// Low Power Mode alone, for the 2 s watch: one `pmset -g`, none of `read_context`'s other shell-outs.
/// Whether this Mac has Low Power Mode at all: the spec table (`macs`, model and macOS version)
/// first; for a model it doesn't know, the Mac itself (`low_power_listed`).
pub fn low_power_supported() -> bool {
    match (hw_model(), macos_version()) {
        (Some(model), Some(v)) => crate::macs::low_power_mode(&model, &v).unwrap_or_else(low_power_listed),
        _ => low_power_listed(),
    }
}

/// macOS lists `lowpowermode` (laptops) or `powermode` (desktops with an Energy Mode setting) in
/// `pmset -g` only where it offers one. `pmset` failing says nothing, so it reads as supported.
pub fn low_power_listed() -> bool {
    run("pmset", &["-g"]).map_or(true, |s| parse_lowpower(&s).is_some())
}

/// The model identifier ("iMac21,1"): a type, not a name.
pub fn hw_model() -> Option<String> {
    run("/usr/sbin/sysctl", &["-n", "hw.model"]).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// "15.6".
pub fn macos_version() -> Option<String> {
    run("/usr/bin/sw_vers", &["-productVersion"]).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

pub fn read_low_power() -> Option<bool> {
    run("pmset", &["-g"]).and_then(|s| parse_lowpower(&s))
}

/// Denominator context, all sudoless: Low Power Mode, power source, display brightness, thermal pressure.
pub fn read_context() -> Context {
    let (on_ac, battery_pct) = run("pmset", &["-g", "batt"]).map(|s| parse_batt(&s)).unwrap_or((None, None));
    Context {
        // The app's 2 s watch is the one reader; the CLI (no watch) reads it here.
        low_power_mode: crate::lowpower::latest().or_else(read_low_power),
        on_ac,
        battery_pct,
        brightness_pct: run("ioreg", &["-r", "-c", "AppleARMBacklight", "-d", "1"]).and_then(|s| parse_brightness(&s)),
        thermal: thermal_pressure(),
    }
}

/// CPU % used by every process except `exclude` (our own pid and xmrig's). Noise signal for trials.
pub fn other_cpu_pct(exclude: &[u32]) -> Option<f64> {
    // Minor M11: a fixed path, not PATH (it runs every second while the optimizer holds).
    run("/bin/ps", &["-A", "-o", "pid=,%cpu="]).map(|s| parse_other_cpu(&s, exclude))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_low_power_mode() {
        assert_eq!(parse_lowpower(" standby  1\n lowpowermode         1\n"), Some(true));
        assert_eq!(parse_lowpower(" lowpowermode         0\n"), Some(false));
        assert_eq!(parse_lowpower("no such key"), None);
    }

    #[test]
    fn parses_power_mode_on_high_power_capable_macs() {
        assert_eq!(parse_lowpower(" powermode            1\n"), Some(true));
        assert_eq!(parse_lowpower(" powermode            0\n"), Some(false));
        assert_eq!(parse_lowpower(" powermode            2\n"), Some(false), "2 is High Power Mode");
        assert_eq!(parse_lowpower(" lowpowermode 1\n powermode 0\n"), Some(true), "lowpowermode wins when both appear");
    }

    #[test]
    fn a_garbage_value_is_none_not_false() {
        assert_eq!(parse_lowpower(" lowpowermode maybe\n"), None);
        assert_eq!(parse_lowpower(" powermode 3\n"), None, "an unknown state, e.g. a future third value");
        assert_eq!(parse_lowpower(" powermode \n"), None, "the key present with no value");
    }

    #[test]
    fn run_times_out_on_a_hung_child_and_reaps_it() {
        let start = Instant::now();
        let out = run_timeout("sleep", &["5"], Duration::from_millis(100));
        assert_eq!(out, None, "a child still running past the deadline reads as unknown, not a crash");
        assert!(start.elapsed() < Duration::from_secs(2), "the timeout must bound the wait, not the child's own runtime");
    }

    #[test]
    fn run_still_returns_output_from_a_fast_child() {
        assert_eq!(run_timeout("echo", &["hi"], Duration::from_secs(3)), Some("hi\n".to_string()));
    }

    #[test]
    fn parses_power_source_and_battery() {
        let laptop = "Now drawing from 'AC Power'\n -InternalBattery-0 (id=7077987)\t100%; charged; 0:00 remaining present: true\n";
        assert_eq!(parse_batt(laptop), (Some(true), Some(100)));
        let on_batt = "Now drawing from 'Battery Power'\n -InternalBattery-0 (id=1)\t57%; discharging; 3:10 remaining\n";
        assert_eq!(parse_batt(on_batt), (Some(false), Some(57)));
        assert_eq!(parse_batt("Now drawing from 'AC Power'\n"), (Some(true), None), "desktop Mac");
    }

    #[test]
    fn parses_brightness() {
        let io = r#"  | "IODisplayParameters" = {"brightness"={"min"=0,"max"=65536,"value"=32768}}"#;
        assert_eq!(parse_brightness(io), Some(50.0));
        assert_eq!(parse_brightness("nothing"), None);
    }

    #[test]
    fn sums_other_processes_cpu() {
        let ps = "  123   5.0\n  456  30.5\n  789 100.0\n  bad line\n";
        assert!((parse_other_cpu(ps, &[789]) - 35.5).abs() < 1e-9);
    }

    #[test]
    fn thermal_levels_map() {
        assert_eq!(ThermalPressure::from_level(0), Some(ThermalPressure::Nominal));
        assert_eq!(ThermalPressure::from_level(3), Some(ThermalPressure::Trapping));
        assert_eq!(ThermalPressure::from_level(9), None);
        assert_eq!(ThermalPressure::Heavy.level(), 2);
    }

    #[test]
    fn thermal_pressure_reads_on_this_mac() {
        assert!(thermal_pressure().is_some(), "Darwin notify key com.apple.system.thermalpressurelevel");
    }

    #[test]
    fn old_context_json_without_thermal_still_loads() {
        let old_json = r#"{"battery_pct":100,"brightness_pct":50.0,"low_power_mode":false,"on_ac":true}"#;
        let ctx: Context = serde_json::from_str(old_json).expect("old Context JSON should deserialize");
        assert_eq!(ctx.low_power_mode, Some(false));
        assert_eq!(ctx.on_ac, Some(true));
        assert_eq!(ctx.battery_pct, Some(100));
        assert_eq!(ctx.brightness_pct, Some(50.0));
        assert_eq!(ctx.thermal, None, "missing thermal field defaults to None");
    }
}
