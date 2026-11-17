//! The Intel Mac's SMC, read without root (Stage 3; `_Research/2026-10-05-intel-power-readings.md`):
//! IOKit's `AppleSMC` user client, struct method 2, command 9 (key info) then 5 (read bytes), the
//! path smcFanControl, stats and osx-cpu-temp read through. Reads only; nothing here writes a key.
//!
//! A key's data type differs between models (`PSTR` is `flt` on most, `sp96` on the 2019 iMacs),
//! so every value is decoded by the type the SMC reports for it, never an assumed one.
use anyhow::{bail, Result};

/// A four-character SMC code ("PSTR", "flt ") as the big-endian u32 the SMC uses.
pub fn fourcc(s: &str) -> u32 {
    let b = s.as_bytes();
    let at = |i: usize| u32::from(*b.get(i).unwrap_or(&b' '));
    (at(0) << 24) | (at(1) << 16) | (at(2) << 8) | at(3)
}

fn type_name(t: u32) -> String {
    t.to_be_bytes().iter().map(|&c| c as char).collect()
}

/// One value from its SMC type and bytes. `flt ` is a little-endian f32; `spXY` a signed 16-bit
/// big-endian fixed point with X integer and Y (hex) fraction bits; `fpXY` the unsigned one; `ui8 `/`ui16`/`ui32`
/// integers. None for any other type, or a value that isn't a finite number.
pub fn decode(data_type: &str, bytes: &[u8]) -> Option<f64> {
    let t = data_type.as_bytes();
    let v = match data_type {
        "flt " if bytes.len() >= 4 => f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f64,
        "ui8 " if !bytes.is_empty() => f64::from(bytes[0]),
        "ui16" if bytes.len() >= 2 => f64::from(u16::from_be_bytes([bytes[0], bytes[1]])),
        "ui32" if bytes.len() >= 4 => f64::from(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
        _ if t.len() == 4 && (t[0..2] == *b"sp" || t[0..2] == *b"fp") && bytes.len() >= 2 => {
            let frac = (t[3] as char).to_digit(16)?;
            let raw = u16::from_be_bytes([bytes[0], bytes[1]]);
            let n = if t[0] == b's' { f64::from(raw as i16) } else { f64::from(raw) };
            n / f64::from(1u32 << frac)
        }
        _ => return None,
    };
    v.is_finite().then_some(v)
}

#[cfg(target_os = "macos")]
mod iokit {
    use std::ffi::c_void;

    pub type KernReturn = i32;
    pub type MachPort = u32;
    pub type IoObject = MachPort;

    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        pub fn IOServiceMatching(name: *const std::ffi::c_char) -> *mut c_void;
        pub fn IOServiceGetMatchingService(main_port: MachPort, matching: *mut c_void) -> IoObject;
        pub fn IOServiceOpen(service: IoObject, owning_task: MachPort, kind: u32, connect: *mut IoObject) -> KernReturn;
        pub fn IOServiceClose(connect: IoObject) -> KernReturn;
        pub fn IOObjectRelease(object: IoObject) -> KernReturn;
        pub fn IOConnectCallStructMethod(
            connection: IoObject,
            selector: u32,
            input: *const c_void,
            input_size: usize,
            output: *mut c_void,
            output_size: *mut usize,
        ) -> KernReturn;
    }

    extern "C" {
        pub static mach_task_self_: MachPort;
    }
}

/// The SMC's request and reply, byte for byte as AppleSMC expects (80 bytes; smcFanControl `smc.h`).
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct KeyData {
    key: u32,
    vers: [u8; 6],
    // C aligns the nested pLimitData and keyInfo structs to 4 bytes: these pads keep the offsets.
    _pad0: [u8; 2],
    p_limit: [u8; 16],
    info_size: u32,
    info_type: u32,
    info_attributes: u8,
    _pad1: [u8; 3],
    result: u8,
    status: u8,
    data8: u8,
    data32: u32,
    bytes: [u8; 32],
}

const _: () = assert!(std::mem::size_of::<KeyData>() == 80);
const _: () = assert!(std::mem::offset_of!(KeyData, info_size) == 28 && std::mem::offset_of!(KeyData, result) == 40);
const _: () = assert!(std::mem::offset_of!(KeyData, data32) == 44 && std::mem::offset_of!(KeyData, bytes) == 48);

const KERNEL_INDEX_SMC: u32 = 2;
const CMD_READ_BYTES: u8 = 5;
const CMD_READ_KEYINFO: u8 = 9;

/// An open connection to AppleSMC. Closed on drop.
pub struct Smc {
    #[cfg(target_os = "macos")]
    conn: iokit::IoObject,
}

impl Smc {
    #[cfg(target_os = "macos")]
    pub fn open() -> Result<Smc> {
        use iokit::*;
        unsafe {
            let matching = IOServiceMatching(c"AppleSMC".as_ptr());
            if matching.is_null() {
                bail!("no AppleSMC on this mac");
            }
            let service = IOServiceGetMatchingService(0, matching);
            if service == 0 {
                bail!("no AppleSMC on this mac");
            }
            let mut conn = 0;
            let kr = IOServiceOpen(service, mach_task_self_, 0, &mut conn);
            IOObjectRelease(service);
            if kr != 0 {
                bail!("AppleSMC would not open ({kr:#x})");
            }
            Ok(Smc { conn })
        }
    }

    #[cfg(not(target_os = "macos"))]
    pub fn open() -> Result<Smc> {
        bail!("no SMC off macOS")
    }

    #[cfg(target_os = "macos")]
    fn call(&self, input: &KeyData) -> Result<KeyData> {
        let mut out = KeyData::default();
        let mut size = std::mem::size_of::<KeyData>();
        let kr = unsafe {
            iokit::IOConnectCallStructMethod(
                self.conn,
                KERNEL_INDEX_SMC,
                (input as *const KeyData).cast(),
                std::mem::size_of::<KeyData>(),
                (&mut out as *mut KeyData).cast(),
                &mut size,
            )
        };
        if kr != 0 || out.result != 0 {
            bail!("smc read failed");
        }
        Ok(out)
    }

    #[cfg(not(target_os = "macos"))]
    fn call(&self, _input: &KeyData) -> Result<KeyData> {
        bail!("no SMC off macOS")
    }

    /// One key: its type (as the SMC names it) and value. Err when the key doesn't exist here.
    pub fn read(&self, key: &str) -> Result<(String, Option<f64>)> {
        let info = self.call(&KeyData { key: fourcc(key), data8: CMD_READ_KEYINFO, ..KeyData::default() })?;
        let size = info.info_size.min(32);
        let out = self.call(&KeyData { key: fourcc(key), info_size: size, data8: CMD_READ_BYTES, ..KeyData::default() })?;
        let t = type_name(info.info_type);
        let v = decode(&t, &out.bytes[..size as usize]);
        Ok((t, v))
    }

    /// The value only; None when the key is missing or unreadable.
    pub fn value(&self, key: &str) -> Option<f64> {
        self.read(key).ok().and_then(|(_, v)| v)
    }
}

#[cfg(target_os = "macos")]
impl Drop for Smc {
    fn drop(&mut self) {
        unsafe {
            iokit::IOServiceClose(self.conn);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Values decoded from real Intel SMC dumps (VirtualSMC `Docs/SMCDumps`, the research doc's
    /// table): each type the 2018+ Macs use for their power keys.
    #[test]
    fn every_power_key_type_the_intel_dumps_show_decodes() {
        assert_eq!(fourcc("PSTR"), 0x5053_5452);
        assert_eq!(type_name(fourcc("sp96")), "sp96");
        // flt: little-endian f32.
        assert_eq!(decode("flt ", &8.22f32.to_le_bytes()).map(|v| (v * 100.0).round()), Some(822.0));
        // spXY: X integer bits, Y fraction bits (the 4th character, hex).
        assert_eq!(decode("sp87", &[0x01, 0x5d]), Some(349.0 / 128.0)); // laptops' PCPT
        let w = ((61.59f64 * 64.0).round() as i16).to_be_bytes();
        assert!((decode("sp96", &w).unwrap() - 61.59).abs() < 1.0 / 64.0); // iMac 2019's PSTR
        assert_eq!(decode("spa5", &[0x00, 0x20]), Some(1.0)); // iMac Pro's PCPT: 5 fraction bits
        assert_eq!(decode("sp78", &[0x2d, 0x80]), Some(45.5)); // TC0P, °C
        assert_eq!(decode("sp78", &[0xff, 0x00]), Some(-1.0), "signed");
        assert_eq!(decode("fpe2", &[0x00, 0x08]), Some(2.0));
        assert_eq!(decode("ui16", &[0x01, 0x00]), Some(256.0));
        assert_eq!(decode("ch8*", &[0x41]), None, "unknown types are not numbers");
        assert_eq!(decode("flt ", &f32::NAN.to_le_bytes()), None);
    }

    /// A real read through IOKit, no root: the SMC answers this Mac's own whole-system watts key.
    /// Apple Silicon and Intel both name it PSTR; run by hand (`--ignored`) on each kind of Mac.
    #[test]
    #[ignore]
    fn reads_pstr_without_root_on_this_mac() {
        let smc = Smc::open().expect("AppleSMC opens without root");
        let (t, v) = smc.read("PSTR").expect("PSTR exists");
        eprintln!("PSTR {t} {v:?}");
        assert!(v.is_some_and(|w| w > 0.5 && w < 400.0), "{t} {v:?}");
        assert!(smc.read("ZZZZ").is_err(), "a missing key is an error, not a zero");
    }
}
