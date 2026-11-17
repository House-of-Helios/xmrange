//! Every Mac from 2018 on, by model identifier (`sysctl hw.model`), and whether it has Low Power
//! Mode (owner, 2026-10-03: a spec table, so the app knows instead of guessing). Sources, read
//! 2026-10-03:
//! - Model identifiers: Apple's "Identify your … model" pages (MacBook Air 102869, MacBook Pro
//!   108052, iMac 108054, Mac mini 102852, Mac Studio 102231, Mac Pro 102887; all published
//!   September 2026).
//! - Desktops and the newest laptops: "About Power Modes on your Mac" (support.apple.com/101613).
//!   Low Power Mode on Mac Studio 2025 or later, Mac mini 2024 or later, iMac 2024, Mac Studio
//!   (2023) with macOS 15.3 or later, Mac mini (2023) and iMac (24-inch, 2023) with 15.1 or later.
//! - Older laptops: Low Power Mode arrived in macOS 12 Monterey for the MacBook and MacBook Pro
//!   from early 2016 on. The Intel MacBook Air (2018–2020) isn't named, so it is `Ask`.
//!
//! A model this table doesn't know (newer than it, or an `Ask` row) falls back to asking the Mac
//! (`context::low_power_supported`, `pmset -g`).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Form {
    Laptop,
    Desktop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lpm {
    /// No Low Power Mode on any macOS.
    No,
    /// Low Power Mode from this macOS version on (major, minor).
    Since(u32, u32),
    /// Not settled by Apple's pages: ask the Mac.
    Ask,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MacModel {
    pub id: &'static str,
    pub name: &'static str,
    pub year: u16,
    pub form: Form,
    pub apple_silicon: bool,
    pub lpm: Lpm,
}

use Form::{Desktop, Laptop};
use Lpm::{Ask, No, Since};

const fn m(id: &'static str, name: &'static str, year: u16, form: Form, apple_silicon: bool, lpm: Lpm) -> MacModel {
    MacModel { id, name, year, form, apple_silicon, lpm }
}

/// Laptops got Low Power Mode with macOS 12.
const MONTEREY: Lpm = Since(12, 0);

pub const MACS: &[MacModel] = &[
    // MacBook Air
    m("MacBookAir8,1", "MacBook Air (Retina, 13-inch, 2018)", 2018, Laptop, false, Ask),
    m("MacBookAir8,2", "MacBook Air (Retina, 13-inch, 2019)", 2019, Laptop, false, Ask),
    m("MacBookAir9,1", "MacBook Air (Retina, 13-inch, 2020)", 2020, Laptop, false, Ask),
    m("MacBookAir10,1", "MacBook Air (M1, 2020)", 2020, Laptop, true, MONTEREY),
    m("Mac14,2", "MacBook Air (M2, 2022)", 2022, Laptop, true, MONTEREY),
    m("Mac14,15", "MacBook Air (15-inch, M2, 2023)", 2023, Laptop, true, MONTEREY),
    m("Mac15,12", "MacBook Air (13-inch, M3, 2024)", 2024, Laptop, true, MONTEREY),
    m("Mac15,13", "MacBook Air (15-inch, M3, 2024)", 2024, Laptop, true, MONTEREY),
    m("Mac16,12", "MacBook Air (13-inch, M4, 2025)", 2025, Laptop, true, MONTEREY),
    m("Mac16,13", "MacBook Air (15-inch, M4, 2025)", 2025, Laptop, true, MONTEREY),
    m("Mac17,3", "MacBook Air (13-inch, M5)", 2026, Laptop, true, MONTEREY),
    m("Mac17,4", "MacBook Air (15-inch, M5)", 2026, Laptop, true, MONTEREY),
    // MacBook Pro
    m("MacBookPro15,1", "MacBook Pro (15-inch, 2018/2019)", 2018, Laptop, false, MONTEREY),
    m("MacBookPro15,2", "MacBook Pro (13-inch, 2018/2019, Four Thunderbolt 3 ports)", 2018, Laptop, false, MONTEREY),
    m("MacBookPro15,3", "MacBook Pro (15-inch, 2019)", 2019, Laptop, false, MONTEREY),
    m("MacBookPro15,4", "MacBook Pro (13-inch, 2019, Two Thunderbolt 3 ports)", 2019, Laptop, false, MONTEREY),
    m("MacBookPro16,1", "MacBook Pro (16-inch, 2019)", 2019, Laptop, false, MONTEREY),
    m("MacBookPro16,4", "MacBook Pro (16-inch, 2019)", 2019, Laptop, false, MONTEREY),
    m("MacBookPro16,2", "MacBook Pro (13-inch, 2020, Four Thunderbolt 3 ports)", 2020, Laptop, false, MONTEREY),
    m("MacBookPro16,3", "MacBook Pro (13-inch, 2020, Two Thunderbolt 3 ports)", 2020, Laptop, false, MONTEREY),
    m("MacBookPro17,1", "MacBook Pro (13-inch, M1, 2020)", 2020, Laptop, true, MONTEREY),
    m("MacBookPro18,1", "MacBook Pro (16-inch, 2021)", 2021, Laptop, true, MONTEREY),
    m("MacBookPro18,2", "MacBook Pro (16-inch, 2021)", 2021, Laptop, true, MONTEREY),
    m("MacBookPro18,3", "MacBook Pro (14-inch, 2021)", 2021, Laptop, true, MONTEREY),
    m("MacBookPro18,4", "MacBook Pro (14-inch, 2021)", 2021, Laptop, true, MONTEREY),
    m("Mac14,7", "MacBook Pro (13-inch, M2, 2022)", 2022, Laptop, true, MONTEREY),
    m("Mac14,5", "MacBook Pro (14-inch, 2023)", 2023, Laptop, true, MONTEREY),
    m("Mac14,9", "MacBook Pro (14-inch, 2023)", 2023, Laptop, true, MONTEREY),
    m("Mac14,6", "MacBook Pro (16-inch, 2023)", 2023, Laptop, true, MONTEREY),
    m("Mac14,10", "MacBook Pro (16-inch, 2023)", 2023, Laptop, true, MONTEREY),
    m("Mac15,3", "MacBook Pro (14-inch, M3, Nov 2023)", 2023, Laptop, true, MONTEREY),
    m("Mac15,6", "MacBook Pro (14-inch, M3 Pro or Max, Nov 2023)", 2023, Laptop, true, MONTEREY),
    m("Mac15,8", "MacBook Pro (14-inch, M3 Pro or Max, Nov 2023)", 2023, Laptop, true, MONTEREY),
    m("Mac15,10", "MacBook Pro (14-inch, M3 Pro or Max, Nov 2023)", 2023, Laptop, true, MONTEREY),
    m("Mac15,7", "MacBook Pro (16-inch, M3 Pro or Max, Nov 2023)", 2023, Laptop, true, MONTEREY),
    m("Mac15,9", "MacBook Pro (16-inch, M3 Pro or Max, Nov 2023)", 2023, Laptop, true, MONTEREY),
    m("Mac15,11", "MacBook Pro (16-inch, M3 Pro or Max, Nov 2023)", 2023, Laptop, true, MONTEREY),
    m("Mac16,1", "MacBook Pro (14-inch, M4, 2024)", 2024, Laptop, true, MONTEREY),
    m("Mac16,6", "MacBook Pro (14-inch, M4 Pro or Max, 2024)", 2024, Laptop, true, MONTEREY),
    m("Mac16,8", "MacBook Pro (14-inch, M4 Pro or Max, 2024)", 2024, Laptop, true, MONTEREY),
    m("Mac16,5", "MacBook Pro (16-inch, M4 Pro or Max, 2024)", 2024, Laptop, true, MONTEREY),
    m("Mac16,7", "MacBook Pro (16-inch, M4 Pro or Max, 2024)", 2024, Laptop, true, MONTEREY),
    m("Mac17,2", "MacBook Pro (14-inch, M5)", 2025, Laptop, true, MONTEREY),
    m("Mac17,7", "MacBook Pro (14-inch, M5 Pro or M5 Max)", 2026, Laptop, true, MONTEREY),
    m("Mac17,9", "MacBook Pro (14-inch, M5 Pro or M5 Max)", 2026, Laptop, true, MONTEREY),
    m("Mac17,6", "MacBook Pro (16-inch, M5 Pro or M5 Max)", 2026, Laptop, true, MONTEREY),
    m("Mac17,8", "MacBook Pro (16-inch, M5 Pro or M5 Max)", 2026, Laptop, true, MONTEREY),
    // iMac
    m("iMac19,1", "iMac (Retina 5K, 27-inch, 2019)", 2019, Desktop, false, No),
    m("iMac19,2", "iMac (Retina 4K, 21.5-inch, 2019)", 2019, Desktop, false, No),
    m("iMac20,1", "iMac (Retina 5K, 27-inch, 2020)", 2020, Desktop, false, No),
    m("iMac20,2", "iMac (Retina 5K, 27-inch, 2020)", 2020, Desktop, false, No),
    m("iMac21,1", "iMac (24-inch, M1, 2021)", 2021, Desktop, true, No),
    m("iMac21,2", "iMac (24-inch, M1, 2021)", 2021, Desktop, true, No),
    m("Mac15,4", "iMac (24-inch, 2023, Two ports)", 2023, Desktop, true, Since(15, 1)),
    m("Mac15,5", "iMac (24-inch, 2023, Four ports)", 2023, Desktop, true, Since(15, 1)),
    m("Mac16,2", "iMac (24-inch, 2024, Two ports)", 2024, Desktop, true, Since(15, 0)),
    m("Mac16,3", "iMac (24-inch, 2024, Four ports)", 2024, Desktop, true, Since(15, 0)),
    // Mac mini
    m("Macmini8,1", "Mac mini (2018)", 2018, Desktop, false, No),
    m("Macmini9,1", "Mac mini (M1, 2020)", 2020, Desktop, true, No),
    m("Mac14,3", "Mac mini (M2, 2023)", 2023, Desktop, true, Since(15, 1)),
    m("Mac14,12", "Mac mini (M2 Pro, 2023)", 2023, Desktop, true, Since(15, 1)),
    m("Mac16,10", "Mac mini (2024)", 2024, Desktop, true, Since(15, 0)),
    m("Mac16,11", "Mac mini (2024)", 2024, Desktop, true, Since(15, 0)),
    m("Mac17,16", "Mac mini (M5 Pro)", 2026, Desktop, true, Since(15, 0)),
    m("Mac18,5", "Mac mini (M6)", 2026, Desktop, true, Since(15, 0)),
    // Mac Studio
    m("Mac13,1", "Mac Studio (M1 Max, 2022)", 2022, Desktop, true, No),
    m("Mac13,2", "Mac Studio (M1 Ultra, 2022)", 2022, Desktop, true, No),
    m("Mac14,13", "Mac Studio (M2 Max, 2023)", 2023, Desktop, true, Since(15, 3)),
    m("Mac14,14", "Mac Studio (M2 Ultra, 2023)", 2023, Desktop, true, Since(15, 3)),
    m("Mac16,9", "Mac Studio (M4 Max, 2025)", 2025, Desktop, true, Since(15, 0)),
    m("Mac15,14", "Mac Studio (M3 Ultra, 2025)", 2025, Desktop, true, Since(15, 0)),
    m("Mac17,14", "Mac Studio (M5 Max)", 2026, Desktop, true, Since(15, 0)),
    m("Mac17,15", "Mac Studio (M5 Ultra)", 2026, Desktop, true, Since(15, 0)),
    // Mac Pro
    m("MacPro7,1", "Mac Pro (2019)", 2019, Desktop, false, No),
    m("Mac14,8", "Mac Pro (2023)", 2023, Desktop, true, No),
];

pub fn lookup(id: &str) -> Option<&'static MacModel> {
    MACS.iter().find(|m| m.id == id.trim())
}

/// "15.6" → (15, 6); "26" → (26, 0).
pub fn parse_macos(v: &str) -> Option<(u32, u32)> {
    let mut it = v.trim().split('.');
    let major = it.next()?.parse().ok()?;
    let minor = it.next().map_or(Some(0), |m| m.parse().ok())?;
    Some((major, minor))
}

/// Whether this Mac has Low Power Mode, from the table: Some(answer) for a known model on a known
/// macOS, None when the table can't say (an unknown model, an `Ask` row, an unreadable version).
pub fn low_power_mode(model: &str, macos: &str) -> Option<bool> {
    match lookup(model)?.lpm {
        No => Some(false),
        Ask => None,
        Since(maj, min) => parse_macos(macos).map(|v| v >= (maj, min)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_answers_for_the_macs_we_have() {
        assert_eq!(low_power_mode("iMac21,1", "15.6"), Some(false), "the owner's M1 iMac: pmset lists no key either");
        assert_eq!(low_power_mode("Mac15,12", "15.6"), Some(true), "this M3 Air");
        assert_eq!(lookup("iMac21,1").map(|m| m.form), Some(Desktop));
    }

    #[test]
    fn macos_minimums_and_unknowns() {
        assert_eq!(low_power_mode("Mac14,3", "15.0.1"), Some(false), "Mac mini (2023) needs 15.1");
        assert_eq!(low_power_mode("Mac14,3", "15.1"), Some(true));
        assert_eq!(low_power_mode("Mac14,13", "15.2"), Some(false), "Mac Studio (2023) needs 15.3");
        assert_eq!(low_power_mode("Mac14,13", "26"), Some(true));
        assert_eq!(low_power_mode("MacBookPro15,1", "11.7"), Some(false), "before Monterey");
        assert_eq!(low_power_mode("MacBookAir9,1", "14.0"), None, "not settled by Apple's pages: ask the Mac");
        assert_eq!(low_power_mode("Mac19,1", "27.0"), None, "newer than the table: ask the Mac");
    }

    #[test]
    fn every_identifier_appears_once() {
        let mut ids: Vec<&str> = MACS.iter().map(|m| m.id).collect();
        ids.sort_unstable();
        let n = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), n, "a model identifier is listed twice");
        assert!(MACS.iter().all(|m| m.year >= 2018));
    }
}
