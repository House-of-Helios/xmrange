//! Intel Macs' power, temperature and load, without root (Stage 3;
//! `_Research/2026-10-05-intel-power-readings.md`). macmon reads Apple Silicon only, so on an Intel
//! Mac the SMC's own power keys stand in: whole-Mac watts from `PSTR` (else `PDTR`), CPU package
//! watts from the first of `PCPT`, `PCPC`, `PCTR`, `PC0C`, `PC0R` that reads, memory from `PMTR`.
//! The keys are probed once, at open, and kept: one source per figure for the whole session.
use crate::smc::Smc;
use crate::telemetry::{now_secs, MachineInfo, PowerSample, PowerSource};
use anyhow::{bail, Result};
use std::time::Duration;

pub const SYS_KEYS: [&str; 2] = ["PSTR", "PDTR"];
pub const CPU_KEYS: [&str; 5] = ["PCPT", "PCPC", "PCTR", "PC0C", "PC0R"];
pub const RAM_KEYS: [&str; 1] = ["PMTR"];
/// CPU temperature: the die (laptops, the mini: `TCXC`; iMacs: `TCXc`), else the proximity sensor.
/// `TC0E`/`TC0F` read junk on desktops and are never used.
pub const TEMP_KEYS: [&str; 3] = ["TCXC", "TCXc", "TC0P"];

/// A reading a sensor can really give: watts in (0, 1000), °C in (0, 125). The SMC's "no sensor"
/// value (0x8100, -127 °C as sp78) and empty zeros fall out here.
fn plausible_w(v: f64) -> bool {
    v > 0.0 && v < 1000.0
}
fn plausible_c(v: f64) -> bool {
    v > 0.0 && v < 125.0
}

/// The keys this Mac answers, picked once: the first that reads a plausible value.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Keys {
    pub sys: Option<&'static str>,
    pub cpu: Option<&'static str>,
    pub ram: Option<&'static str>,
    pub temp: Option<&'static str>,
}

impl Keys {
    /// From any reader (the SMC, or a table in tests).
    pub fn pick(read: impl Fn(&str) -> Option<f64>) -> Keys {
        let first = |keys: &[&'static str], ok: fn(f64) -> bool| keys.iter().copied().find(|k| read(k).is_some_and(ok));
        Keys { sys: first(&SYS_KEYS, plausible_w), cpu: first(&CPU_KEYS, plausible_w), ram: first(&RAM_KEYS, plausible_w), temp: first(&TEMP_KEYS, plausible_c) }
    }

    /// "PSTR · PCPT · TCXC", for diagnostics: which sensors every figure came from.
    pub fn describe(&self) -> String {
        [self.sys, self.cpu, self.ram, self.temp].iter().flatten().copied().collect::<Vec<_>>().join(" · ")
    }
}

/// The whole CPU's busy and total ticks (Mach `host_statistics`, `HOST_CPU_LOAD_INFO`), no root.
#[cfg(target_os = "macos")]
fn cpu_ticks() -> Option<(u64, u64)> {
    #[repr(C)]
    struct Load {
        ticks: [u32; 4], // user, system, idle, nice
    }
    extern "C" {
        fn mach_host_self() -> u32;
        fn host_statistics(host: u32, flavor: i32, info: *mut i32, count: *mut u32) -> i32;
    }
    const HOST_CPU_LOAD_INFO: i32 = 3;
    let mut l = Load { ticks: [0; 4] };
    let mut count = 4u32;
    let kr = unsafe { host_statistics(mach_host_self(), HOST_CPU_LOAD_INFO, (&mut l as *mut Load).cast(), &mut count) };
    if kr != 0 {
        return None;
    }
    let [user, sys, idle, nice] = l.ticks.map(u64::from);
    Some((user + sys + nice, user + sys + nice + idle))
}

#[cfg(not(target_os = "macos"))]
fn cpu_ticks() -> Option<(u64, u64)> {
    None
}

fn sysctl_string(name: &str) -> Option<String> {
    let out = std::process::Command::new("/usr/sbin/sysctl").args(["-n", name]).output().ok()?;
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (!s.is_empty()).then_some(s)
}

fn sysctl_u64(name: &str) -> Option<u64> {
    sysctl_string(name)?.parse().ok()
}

/// "Intel(R) Core(TM) i7-8700B CPU @ 3.20GHz" → "intel core i7-8700b".
pub fn chip_name(brand: &str) -> String {
    let b = brand.replace("(R)", "").replace("(TM)", "").replace("(tm)", "");
    let b = b.split(" CPU").next().unwrap_or(&b).split('@').next().unwrap_or(&b);
    b.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// The Intel Mac's facts: physical cores (the cores card counts threads, one per logical CPU), no
/// E-cores, memory.
pub fn machine() -> MachineInfo {
    MachineInfo {
        model: sysctl_string("hw.model").unwrap_or_default(),
        chip: sysctl_string("machdep.cpu.brand_string").map(|b| chip_name(&b)).unwrap_or_else(|| "intel".into()),
        p_cores: sysctl_u64("hw.physicalcpu").unwrap_or(1).min(255) as u8,
        e_cores: 0,
        memory_gb: (sysctl_u64("hw.memsize").unwrap_or(0) / (1 << 30)).min(u64::from(u16::MAX)) as u16,
    }
}

pub struct IntelSource {
    smc: Smc,
    keys: Keys,
    ticks: Option<(u64, u64)>,
    mem_total_gb: Option<f64>,
}

impl IntelSource {
    /// Opens the SMC and picks the keys. Err when no whole-Mac or CPU watts read: without them
    /// there is no h/w to measure (the app then mines without trials, as on any Mac without readings).
    pub fn new() -> Result<IntelSource> {
        let smc = Smc::open()?;
        let keys = Keys::pick(|k| smc.value(k));
        if keys.sys.is_none() && keys.cpu.is_none() {
            bail!("no power readings in this mac's smc");
        }
        let mem_total_gb = sysctl_u64("hw.memsize").map(|b| b as f64 / f64::from(1u32 << 30));
        Ok(IntelSource { smc, keys, ticks: cpu_ticks(), mem_total_gb })
    }

    pub fn keys(&self) -> &Keys {
        &self.keys
    }

    fn read(&self, key: Option<&str>, ok: fn(f64) -> bool) -> Option<f64> {
        key.and_then(|k| self.smc.value(k)).filter(|v| ok(*v))
    }
}

/// The SMC is read four times across the interval and averaged, like macmon's interval sample.
const READS: u32 = 4;

impl PowerSource for IntelSource {
    fn sample(&mut self, interval_ms: u32) -> Result<PowerSample> {
        let (mut sys, mut cpu, mut ram, mut temp) = (vec![], vec![], vec![], vec![]);
        for _ in 0..READS {
            std::thread::sleep(Duration::from_millis(u64::from(interval_ms / READS)));
            sys.extend(self.read(self.keys.sys, plausible_w));
            cpu.extend(self.read(self.keys.cpu, plausible_w));
            ram.extend(self.read(self.keys.ram, plausible_w));
            temp.extend(self.read(self.keys.temp, plausible_c));
        }
        let mean = |v: &[f64]| (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64);
        let now = cpu_ticks();
        let active = match (self.ticks, now) {
            (Some((b0, t0)), Some((b1, t1))) if t1 > t0 => (b1.saturating_sub(b0)) as f64 / (t1 - t0) as f64,
            _ => 0.0,
        };
        self.ticks = now;
        Ok(PowerSample {
            t: now_secs(),
            cpu_w: mean(&cpu).unwrap_or(0.0),
            ram_w: mean(&ram).unwrap_or(0.0),
            sys_w: mean(&sys),
            temp_c: mean(&temp),
            p_mhz: 0,
            e_mhz: 0,
            p_active: active,
            e_active: 0.0,
            mem_used_gb: None,
            mem_total_gb: self.mem_total_gb,
            thermal: crate::context::thermal_pressure(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The key choice on the real dumps' key sets (VirtualSMC `Docs/SMCDumps`): the Mac mini 2018,
    /// the 2019 iMac (whose `TCXC` is spelled `TCXc`) and the 2019 Mac Pro (no `PCPT`/`PCPC`).
    #[test]
    fn keys_are_picked_from_what_each_intel_mac_answers() {
        let table = |pairs: &'static [(&'static str, f64)]| move |k: &str| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| *v);
        let mini = Keys::pick(table(&[("PSTR", 8.22), ("PCPT", 2.73), ("PCPC", 0.30), ("PMTR", 0.9), ("TCXC", 41.0), ("TC0P", 38.0)]));
        assert_eq!(mini, Keys { sys: Some("PSTR"), cpu: Some("PCPT"), ram: Some("PMTR"), temp: Some("TCXC") });
        assert_eq!(mini.describe(), "PSTR · PCPT · PMTR · TCXC");
        let imac = Keys::pick(table(&[("PSTR", 61.59), ("PCPT", 0.48), ("TCXc", 45.0), ("TC0P", 40.0)]));
        assert_eq!(imac.temp, Some("TCXc"));
        let mac_pro = Keys::pick(table(&[("PSTR", 128.55), ("PC0R", 34.66), ("TCXC", -127.0), ("TC0P", 39.0)]));
        assert_eq!((mac_pro.cpu, mac_pro.temp), (Some("PC0R"), Some("TC0P")), "no-sensor -127 °C is skipped");
        assert_eq!(Keys::pick(|_| None), Keys::default());
    }

    #[test]
    fn the_cpu_brand_reads_as_a_short_chip_name() {
        assert_eq!(chip_name("Intel(R) Core(TM) i7-8700B CPU @ 3.20GHz"), "intel core i7-8700b");
        assert_eq!(chip_name("Intel(R) Core(TM) i3-8100B CPU @ 3.60GHz"), "intel core i3-8100b");
    }

    /// The real SMC, through the whole source (`--ignored`, on any Mac: Apple Silicon answers PSTR
    /// too, so the path runs here before an Intel Mac proves its own keys).
    #[test]
    #[ignore]
    fn samples_this_macs_smc() {
        let mut s = IntelSource::new().expect("the smc has power keys");
        let p = s.sample(1000).unwrap();
        eprintln!("keys {} · sys {:?} cpu {} temp {:?} active {:.2}", s.keys().describe(), p.sys_w, p.cpu_w, p.temp_c, p.p_active);
        assert!(p.sys_w.is_some_and(|w| w > 0.5));
        assert!((0.0..=1.0).contains(&p.p_active));
    }
}
