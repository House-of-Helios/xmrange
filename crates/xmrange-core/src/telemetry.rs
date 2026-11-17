use crate::context::ThermalPressure;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct PowerSample {
    pub t: f64,
    pub cpu_w: f64,
    pub ram_w: f64,
    /// Whole-Mac watts (SMC PSTR). None when the Mac does not report it.
    pub sys_w: Option<f64>,
    /// CPU average °C. None when sensors report nothing.
    pub temp_c: Option<f64>,
    pub p_mhz: u32,
    pub e_mhz: u32,
    pub p_active: f64,
    pub e_active: f64,
    /// RAM used in GB. None when 0 or unknown.
    #[serde(default)]
    pub mem_used_gb: Option<f64>,
    /// Total RAM in GB. None when 0 or unknown.
    #[serde(default)]
    pub mem_total_gb: Option<f64>,
    /// macOS thermal pressure at this sample (the Darwin notify key, read without privileges).
    /// None when unread: fixtures, and `from_metrics`, which only maps macmon.
    #[serde(default)]
    pub thermal: Option<ThermalPressure>,
}

pub trait PowerSource {
    /// Blocks for about `interval_ms` and returns the average over that interval.
    fn sample(&mut self, interval_ms: u32) -> Result<PowerSample>;
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct MachineInfo {
    pub model: String,
    pub chip: String,
    pub p_cores: u8,
    pub e_cores: u8,
    pub memory_gb: u16,
}

pub fn now_secs() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// Monotonic seconds that stand still while the Mac sleeps: set against `now_secs`, they show a
/// sleep (ruling P4-C23). Re-review N3: `CLOCK_UPTIME_RAW` is read directly, not through
/// `Instant`, whose clock std may change.
#[cfg(target_os = "macos")]
pub fn mono_secs() -> f64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: `ts` is a valid, writable timespec for the call's duration.
    let rc = unsafe { libc::clock_gettime(libc::CLOCK_UPTIME_RAW, &mut ts) };
    if rc != 0 {
        return 0.0;
    }
    ts.tv_sec as f64 + ts.tv_nsec as f64 / 1e9
}

#[cfg(not(target_os = "macos"))]
pub fn mono_secs() -> f64 {
    static T0: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    T0.get_or_init(std::time::Instant::now).elapsed().as_secs_f64()
}

/// Ruling P4-C23: wall time and monotonic time drifting apart by more than this between two
/// ticks is a sleep and wake (or a wall clock stepped).
pub const SLEEP_GAP_S: f64 = 10.0;

/// Spots a sleep and wake from one tick to the next (ruling P4-C23): wall time moved more than
/// `SLEEP_GAP_S` away from monotonic time. A lid close, Apple menu → Sleep, or a clock step all
/// show; `caffeinate -i` stops only idle sleep.
#[derive(Debug, Clone, Copy, Default)]
pub struct ClockGap {
    last: Option<(f64, f64)>,
}

impl ClockGap {
    /// One tick's wall (unix) and monotonic seconds. After a gap, `Some(wall seconds the
    /// monotonic clock didn't see)`: the time asleep (negative for a wall clock stepped back).
    /// Ticks may be skipped in between (a paused miner, manual): the drift adds up all the same.
    pub fn step(&mut self, wall: f64, mono: f64) -> Option<f64> {
        let drift = self.last.map(|(w, m)| (wall - w) - (mono - m));
        self.last = Some((wall, mono));
        drift.filter(|d| d.abs() > SLEEP_GAP_S)
    }
}

pub fn from_metrics(m: &macmon::Metrics, t: f64) -> PowerSample {
    let positive = |v: f32| (v.is_finite() && v > 0.0).then_some(v as f64);
    let bytes_to_gb = |b: u64| (b > 0).then(|| b as f64 / (1024.0 * 1024.0 * 1024.0));
    PowerSample {
        t,
        cpu_w: m.cpu_power as f64,
        ram_w: m.ram_power as f64,
        sys_w: positive(m.sys_power),
        temp_c: positive(m.temp.cpu_temp_avg),
        p_mhz: m.pcpu_freq_mhz,
        e_mhz: m.ecpu_freq_mhz,
        p_active: m.pcpu_active_ratio as f64,
        e_active: m.ecpu_active_ratio as f64,
        mem_used_gb: bytes_to_gb(m.memory.ram_usage),
        mem_total_gb: bytes_to_gb(m.memory.ram_total),
        thermal: None,
    }
}

/// Sudoless Apple Silicon power via macmon (IOReport + SMC). Not Send: create it on the thread that samples.
pub struct AppleSiliconSource {
    sampler: macmon::Sampler,
}

impl AppleSiliconSource {
    pub fn new() -> Result<Self> {
        Ok(Self { sampler: macmon::Sampler::new().map_err(|e| anyhow!("macmon could not start: {e}"))? })
    }

    pub fn machine(&self) -> MachineInfo {
        let s = self.sampler.get_soc_info();
        MachineInfo {
            model: s.mac_model.clone(),
            chip: s.chip_name.clone(),
            p_cores: s.pcpu_cores,
            e_cores: s.ecpu_cores,
            memory_gb: s.memory_gb,
        }
    }
}

/// This Mac's power source and facts: macmon on Apple Silicon, the SMC on an Intel Mac. The one
/// place that decides, so the engine, the idle sampler and the machine facts never disagree.
/// Not Send: open it on the thread that samples. Dev builds can force the SMC path
/// (`XMRANGE_FORCE_SMC=1`) to run it on Apple Silicon, whose SMC answers PSTR too.
pub fn open_source() -> Result<(Box<dyn PowerSource>, MachineInfo)> {
    #[cfg(feature = "dev-diagnostics")]
    let force_smc = std::env::var_os("XMRANGE_FORCE_SMC").is_some();
    #[cfg(not(feature = "dev-diagnostics"))]
    let force_smc = false;
    if !force_smc {
        if let Ok(s) = AppleSiliconSource::new() {
            let m = s.machine();
            return Ok((Box::new(s), m));
        }
    }
    let s = crate::intel::IntelSource::new()?;
    Ok((Box::new(s), crate::intel::machine()))
}

/// Which sensors the power figures come from, for diagnostics: "macmon" on Apple Silicon, the SMC
/// keys on an Intel Mac ("smc PSTR · PCPT · TCXC"), or None without readings.
pub fn source_note() -> Option<String> {
    #[cfg(feature = "dev-diagnostics")]
    let force_smc = std::env::var_os("XMRANGE_FORCE_SMC").is_some();
    #[cfg(not(feature = "dev-diagnostics"))]
    let force_smc = false;
    if !force_smc && AppleSiliconSource::new().is_ok() {
        return Some("macmon".into());
    }
    crate::intel::IntelSource::new().ok().map(|s| format!("smc {}", s.keys().describe()))
}

impl PowerSource for AppleSiliconSource {
    fn sample(&mut self, interval_ms: u32) -> Result<PowerSample> {
        let m = self.sampler.get_metrics(interval_ms).map_err(|e| anyhow!("macmon sample failed: {e}"))?;
        let mut s = from_metrics(&m, now_secs());
        // Roadmap carried item: thermal pressure on every sample, so a trial knows the worst it saw.
        s.thermal = crate::context::thermal_pressure();
        Ok(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// P4-C23: a synthetic tick stream. Steady 1 s ticks, a skipped stretch (both clocks move),
    /// jitter under the threshold: no gap. An hour of wall time with a second of monotonic time
    /// (a closed lid): a gap of about an hour, once. A wall clock stepped back: a gap too.
    #[test]
    fn a_clock_gap_is_wall_time_the_monotonic_clock_did_not_see() {
        let mut g = ClockGap::default();
        assert_eq!(g.step(1000.0, 5.0), None, "the first tick has nothing to compare");
        for i in 1..100 {
            // A tick every second, give or take a third of one.
            assert_eq!(g.step(1000.0 + i as f64 + 0.3 * (i % 2) as f64, 5.0 + i as f64), None, "tick {i}");
        }
        assert_eq!(g.step(1400.0, 404.7), None, "a paused stretch: both clocks moved 300.7 s");
        assert_eq!(g.step(1409.5, 405.2), None, "9 s apart: under the threshold");
        let slept = g.step(1409.5 + 3601.0, 406.2).expect("a sleep");
        assert!((slept - 3600.0).abs() < 1e-6, "{slept}");
        assert_eq!(g.step(5011.5, 407.2), None, "awake again: one gap, not two");
        assert!(g.step(4000.0, 408.2).is_some_and(|d| d < -SLEEP_GAP_S), "a wall clock stepped back");
    }

    #[test]
    fn the_monotonic_clock_moves_forward() {
        let a = mono_secs();
        let b = mono_secs();
        assert!(b >= a && a >= 0.0);
    }

    #[test]
    fn maps_macmon_metrics() {
        let m = macmon::Metrics {
            cpu_power: 3.81,
            ram_power: 0.8,
            sys_power: 9.1,
            temp: macmon::TempMetrics { cpu_temp_avg: 55.0, gpu_temp_avg: 0.0 },
            pcpu_freq_mhz: 1923,
            ecpu_freq_mhz: 2554,
            pcpu_active_ratio: 1.0,
            ecpu_active_ratio: 0.31,
            ..Default::default()
        };
        let s = from_metrics(&m, 42.0);
        assert_eq!(s.t, 42.0);
        assert!((s.cpu_w - 3.81).abs() < 1e-6);
        assert!((s.sys_w.unwrap() - 9.1).abs() < 1e-6);
        assert!((s.temp_c.unwrap() - 55.0).abs() < 1e-6);
        assert_eq!((s.p_mhz, s.e_mhz), (1923, 2554));
    }

    #[test]
    fn zero_sys_power_is_unknown() {
        let m = macmon::Metrics { cpu_power: 3.0, ..Default::default() };
        let s = from_metrics(&m, 0.0);
        assert_eq!(s.sys_w, None);
        assert_eq!(s.temp_c, None);
        assert_eq!(crate::units::hw(Some(1800.0), s.sys_w), None, "no inf H/W");
    }

    #[test]
    fn maps_memory_in_gb() {
        let mut m = macmon::Metrics::default();
        m.memory.ram_total = 16 * 1024 * 1024 * 1024;
        m.memory.ram_usage = 8 * 1024 * 1024 * 1024;
        let s = from_metrics(&m, 0.0);
        assert_eq!(s.mem_total_gb, Some(16.0));
        assert_eq!(s.mem_used_gb, Some(8.0));
        assert_eq!(from_metrics(&macmon::Metrics::default(), 0.0).mem_total_gb, None);
    }

    #[test]
    fn a_sample_without_thermal_still_loads_and_mapping_leaves_it_to_the_source() {
        let s = from_metrics(&macmon::Metrics::default(), 1.0);
        assert_eq!(s.thermal, None, "from_metrics is pure; AppleSiliconSource::sample fills it");
        let old = r#"{"t":1.0,"cpu_w":3.8,"ram_w":0.8,"sys_w":9.1,"temp_c":55.0,"p_mhz":1900,"e_mhz":2500,"p_active":1.0,"e_active":0.3}"#;
        assert_eq!(serde_json::from_str::<PowerSample>(old).unwrap().thermal, None);
    }
}
