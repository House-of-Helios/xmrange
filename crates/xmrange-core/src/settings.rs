//! User preferences and configuration.
use crate::config::validate_worker;
use crate::miner::validate_threads;
use crate::optimizer::OptimizerPrefs;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::path::Path;

/// The wallet address, if the user set one — otherwise mining pays the disclosed default
/// (GhostDrip). Kept in plain `settings.json`, like the worker name (owner decision 2026-09-28:
/// no separate secret store, and no OS prompt, ever). `{:?}` never shows more than a mask, so a
/// stray `Settings` in a log line or the tape still never leaks the full address.
#[derive(Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Wallet(pub Option<String>);

impl fmt::Debug for Wallet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Some(w) => write!(f, "Some({})", crate::config::mask_wallet(w)),
            None => write!(f, "None"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Measure {
    #[default]
    Sys,
    Cpu,
}

/// Where the hashes go (doctrine §1a: one outbound set per mode, never both).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Pool {
    /// MoneroOcean: xmrig connects to the MO pool.
    #[default]
    Mo,
    /// p2pool: xmrig mines to a local p2pool, which reaches only its peers.
    P2pool,
}

/// p2pool's share chain (owner, 2026-10-05: the user picks). Nano by default: at a Mac's few
/// kH/s it finds shares often enough to be paid steadily; mini and main pay larger, rarer shares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Chain {
    #[default]
    Nano,
    Mini,
    Main,
}

impl Chain {
    pub fn name(self) -> &'static str {
        match self {
            Chain::Nano => "nano",
            Chain::Mini => "mini",
            Chain::Main => "main",
        }
    }
}

/// Which Monero node p2pool uses (owner, 2026-10-05: the user decides): the Monero GUI's own
/// local node, or a node XMRange runs itself, its chain in a folder the user picked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeChoice {
    #[default]
    Gui,
    Own,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Period {
    Day,
    Week,
    Month,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub rate_per_kwh: f64,
    pub currency: String,
    pub theme: Theme,
    pub measure: Measure,
    pub period: Period,
    pub worker: String,
    pub threads: u32,
    pub update_folder: Option<String>,
    /// The wallet mining pays, if the user set one. Absent means the disclosed GhostDrip default.
    pub wallet: Wallet,
    /// The first-run notice (default wallet, what the pool sees) was dismissed. Kept here, in the
    /// app's own data dir, not in the webview's storage (final review M12).
    pub notice_dismissed: bool,
    /// The optimizer's throttle, mode, held setup and any sweep in progress.
    pub optimizer: OptimizerPrefs,
    /// MoneroOcean or p2pool; applied at the next start.
    pub pool: Pool,
    /// Route the mode's connections through the bundled Tor client (doctrine §1a). On by default
    /// (owner, 2026-10-03); a saved choice is kept.
    pub tor: bool,
    /// The XMR price the user typed, in `currency`, for showing earnings in money.
    pub xmr_price: Option<f64>,
    /// Fetch the XMR price (USD) through Tor once an hour instead (doctrine §1b; opt-in, confirmed).
    pub price_feed: bool,
    /// The user's limits: pause above a temperature (°F), or while on battery. Off by default.
    pub guard: crate::guard::GuardPrefs,
    /// Every profile change, newest last (at most 20): "2026-10-03 21:14:05 · medium · pill". Only
    /// `set_profile` writes it, so a profile that differs from its last entry changed some other
    /// way, and diagnostics says so.
    pub profile_log: Vec<String>,
    /// Manual mining on the E-cores only (`taskpolicy -b`), as the light profile runs; `threads`
    /// then counts E-cores.
    pub e_cores_only: bool,
    /// Manual mining at utility QoS (a trials row picked by hand that ran it).
    pub utility_manual: bool,
    /// p2pool's share chain.
    pub p2pool_chain: Chain,
    /// The node p2pool uses.
    pub node: NodeChoice,
    /// The folder XMRange's own node keeps the Monero chain in (a native folder picker; an
    /// external drive is fine). Set only through `set_node_folder`, which validates it.
    pub node_dir: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            rate_per_kwh: 0.16,
            currency: "$".to_string(),
            theme: Theme::System,
            measure: Measure::Sys,
            period: Period::Day,
            worker: "xmrange".to_string(),
            threads: 5,
            update_folder: None,
            wallet: Wallet(None),
            notice_dismissed: false,
            optimizer: OptimizerPrefs::default(),
            pool: Pool::Mo,
            tor: true,
            xmr_price: None,
            price_feed: false,
            e_cores_only: false,
            utility_manual: false,
            profile_log: Vec::new(),
            guard: crate::guard::GuardPrefs::default(),
            p2pool_chain: Chain::Nano,
            node: NodeChoice::Gui,
            node_dir: None,
        }
    }
}

/// The electricity rate's upper bound, per kWh, in any currency. Far above any real tariff; it
/// exists so a typo can't turn the cost card into "$Infinity".
pub const RATE_MAX: f64 = 100.0;
/// The currency symbol's length bound, in characters (Unicode scalar values): "$", "kr", "CHF".
pub const CURRENCY_MAX_CHARS: usize = 3;

/// The currency rule: 1–3 characters, with no whitespace (so none leading or trailing either)
/// and no control characters. Anything printable passes, so "$", "€", "kr" and "CHF" all work.
pub fn validate_currency(c: &str) -> Result<()> {
    let n = c.chars().count();
    if n == 0 || n > CURRENCY_MAX_CHARS || c.chars().any(|ch| ch.is_whitespace() || ch.is_control()) {
        bail!("currency must be 1–3 characters, with no spaces or control characters");
    }
    Ok(())
}

/// A typed XMR price: positive and below a bound no market will reach, so a typo can't show
/// "$Infinity".
pub fn validate_price(p: Option<f64>) -> Result<()> {
    if let Some(p) = p {
        if !p.is_finite() || !(0.0..=1_000_000.0).contains(&p) {
            bail!("xmr price must be a number from 0 to 1,000,000");
        }
    }
    Ok(())
}

pub fn validate_rate(rate: f64) -> Result<()> {
    if !rate.is_finite() || !(0.0..=RATE_MAX).contains(&rate) {
        bail!("electricity rate must be a number from 0 to 100 per kWh");
    }
    Ok(())
}

impl Settings {
    /// Every rule a saved setting must meet. The worker and thread rules are the same functions
    /// `set_worker` and `set_threads` use. Theme, measure and period are enums, so serde already
    /// refuses any value outside their known set before a `Settings` exists.
    pub fn validate(&self, max_threads: u32) -> Result<()> {
        validate_rate(self.rate_per_kwh)?;
        validate_currency(&self.currency)?;
        validate_price(self.xmr_price)?;
        if let Some(t) = self.guard.pause_above_f {
            if !t.is_finite() || !crate::guard::PAUSE_ABOVE_F.contains(&t) {
                bail!("the temperature limit must be from 100 to 230 °F");
            }
        }
        validate_worker(&self.worker)?;
        validate_threads(self.threads, max_threads)?;
        if let Some(w) = &self.wallet.0 {
            crate::wallet::check_address(w)?;
        }
        Ok(())
    }

    /// Load settings from a directory, for this Mac (`max_threads()`). See `load_for`.
    pub fn load(dir: &Path) -> Settings {
        Self::load_for(dir, crate::miner::max_threads())
    }

    /// Load settings from a directory. A missing or unreadable file gives the defaults. Otherwise
    /// each field is taken on its own: a field that doesn't parse, or breaks its rule (a hand-edited
    /// `threads: 12` on an 8-core Mac, say), falls back to its default, and threads are clamped into
    /// range, so a loaded `Settings` always passes `validate` (final review M11).
    pub fn load_for(dir: &Path, max_threads: u32) -> Settings {
        let Ok(content) = fs::read_to_string(dir.join("settings.json")) else { return Settings::default() };
        let Ok(serde_json::Value::Object(fields)) = serde_json::from_str::<serde_json::Value>(&content) else {
            return Settings::default();
        };
        let mut s = Settings::default();
        for (key, value) in fields {
            let Ok(mut trial) = serde_json::to_value(&s) else { break };
            trial[key.as_str()] = value;
            if let Ok(next) = serde_json::from_value::<Settings>(trial) {
                s = next;
            }
        }
        let d = Settings::default();
        if validate_rate(s.rate_per_kwh).is_err() {
            s.rate_per_kwh = d.rate_per_kwh;
        }
        if validate_currency(&s.currency).is_err() {
            s.currency = d.currency;
        }
        if validate_worker(&s.worker).is_err() {
            s.worker = d.worker;
        }
        if let Some(w) = &s.wallet.0 {
            if crate::wallet::check_address(w).is_err() {
                s.wallet = Wallet(None); // a hand-edited or damaged address: mine to the default, not to a typo
            }
        }
        s.threads = s.threads.clamp(1, max_threads.max(1));
        s.optimizer.sanitize(max_threads.max(1));
        s
    }

    /// Save settings to a directory using atomic rename (writes to .tmp first). A failed write or
    /// rename removes the .tmp, so nothing half-written is left behind.
    pub fn save(&self, dir: &Path) -> anyhow::Result<()> {
        fs::create_dir_all(dir)?;
        let tmp_path = dir.join(".settings.json.tmp");
        let final_path = dir.join("settings.json");
        let content = serde_json::to_string_pretty(self)?;
        let written = fs::write(&tmp_path, content).and_then(|_| fs::rename(&tmp_path, &final_path));
        if written.is_err() {
            let _ = fs::remove_file(&tmp_path);
        }
        Ok(written?)
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Migration {
    /// Plan 2's app saved "no wallet" as `"wallet": ""`; that is now an absent field.
    pub cleared_empty_wallet: bool,
    /// The old dev secrets file's `worker`, moved here — only when the app's worker is still the
    /// default, so a name already chosen in the app always wins. `Some` only when it moved; a
    /// discarded worker leaves this `None`.
    pub worker_moved: Option<String>,
    /// A wallet carried over from the old plaintext dev secrets file.
    pub wallet_migrated: bool,
}

/// Brings the old dev secrets file (`<data dir>/secrets/secrets.json`, plaintext, dev builds only
/// — release never had one) forward into `settings.json`: Plan 1/2's `wallet: ""` quirk, the
/// wallet itself, and the worker (settings.json's own rule already wins). The old file and its
/// now-empty folder are then removed. Idempotent. No old file → no-op, in either build (owner
/// decision 2026-09-28: no OS credential store, no separate secret store — the wallet lives here, like the
/// worker name). A file that fails to parse is left untouched for a human to look at.
pub fn migrate_old_secrets(data_dir: &Path) -> Result<Migration> {
    let mut out = Migration::default();
    let old_dir = data_dir.join("secrets");
    let old_path = old_dir.join("secrets.json");
    if !old_path.exists() {
        return Ok(out);
    }
    let content = fs::read_to_string(&old_path)?;
    let serde_json::Value::Object(map) = serde_json::from_str::<serde_json::Value>(&content)? else {
        bail!("the old secrets file isn't a JSON object; it was left untouched");
    };
    let mut settings = Settings::load(data_dir);
    match map.get("wallet").and_then(|v| v.as_str()) {
        Some("") => out.cleared_empty_wallet = true,
        Some(w) => {
            settings.wallet = Wallet(Some(w.to_string()));
            out.wallet_migrated = true;
        }
        None => {}
    }
    if let Some(worker) = map.get("worker").and_then(|v| v.as_str()) {
        if settings.worker == Settings::default().worker && validate_worker(worker).is_ok() {
            settings.worker = worker.to_string();
            out.worker_moved = Some(worker.to_string());
        }
    }
    settings.save(data_dir)?;
    fs::remove_file(&old_path)?;
    let _ = fs::remove_dir(&old_dir); // only removed if now empty
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(s: &Settings) -> String {
        s.validate(8).expect_err("should be refused").to_string()
    }

    #[test]
    fn defaults_are_valid() {
        Settings::default().validate(8).expect("defaults pass");
    }

    #[test]
    fn rate_bounds() {
        for ok in [0.0, 0.16, 100.0] {
            Settings { rate_per_kwh: ok, ..Settings::default() }.validate(8).expect("in range");
        }
        for bad in [-0.01, 100.01, 1e308, f64::INFINITY, f64::NEG_INFINITY, f64::NAN] {
            let msg = err(&Settings { rate_per_kwh: bad, ..Settings::default() });
            assert_eq!(msg, "electricity rate must be a number from 0 to 100 per kWh", "{bad}");
        }
    }

    #[test]
    fn currency_rule() {
        for ok in ["$", "€", "kr", "CHF", "R$", "zł"] {
            Settings { currency: ok.to_string(), ..Settings::default() }.validate(8).expect(ok);
        }
        for bad in ["", "abcd", "€€€€", " $", "$ ", "k r", "\t", "$\u{7}", "\n"] {
            let msg = err(&Settings { currency: bad.to_string(), ..Settings::default() });
            assert_eq!(msg, "currency must be 1–3 characters, with no spaces or control characters", "{bad:?}");
        }
    }

    #[test]
    fn worker_uses_the_set_worker_rule() {
        for bad in ["", "bad name!", "a/b", &"w".repeat(65)] {
            let s = Settings { worker: bad.to_string(), ..Settings::default() };
            assert_eq!(err(&s), validate_worker(bad).unwrap_err().to_string(), "{bad:?}");
        }
    }

    #[test]
    fn threads_use_the_set_threads_rule() {
        for bad in [0, 9, 64] {
            let s = Settings { threads: bad, ..Settings::default() };
            assert_eq!(err(&s), "threads must be between 1 and 8", "{bad}");
        }
        Settings { threads: 8, ..Settings::default() }.validate(8).expect("max is allowed");
    }

    fn load_json(json: &str, max: u32) -> Settings {
        // A scratch dir inside the crate's own target tree (Rule #1: never system temp).
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/test-scratch")
            .join(format!("settings-load-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("settings.json"), json).unwrap();
        let s = Settings::load_for(&dir, max);
        let _ = fs::remove_dir_all(&dir);
        s
    }

    #[test]
    fn load_clamps_and_falls_back_per_field() {
        let s = load_json(r#"{"threads":12,"rate_per_kwh":0.21,"theme":"neon","currency":"abcd","worker":"ok-name","period":"week"}"#, 8);
        assert_eq!(s.threads, 8, "clamped to this Mac's cores");
        assert_eq!(s.rate_per_kwh, 0.21, "a good field survives a bad neighbour");
        assert_eq!(s.theme, Theme::System, "an unknown theme falls back alone");
        assert_eq!(s.currency, "$", "a rule-breaking currency falls back");
        assert_eq!(s.worker, "ok-name");
        assert_eq!(s.period, Period::Week);
        s.validate(8).expect("a loaded Settings always validates");

        let s = load_json(r#"{"threads":0,"rate_per_kwh":1e308,"worker":"bad name!","notice_dismissed":true}"#, 8);
        assert_eq!((s.threads, s.rate_per_kwh, s.worker.as_str(), s.notice_dismissed), (1, 0.16, "xmrange", true));
        assert_eq!(load_json("not json", 8), Settings::default());
        assert_eq!(load_json("[1,2]", 8), Settings::default());
    }

    #[test]
    fn save_leaves_no_tmp_after_a_failed_rename() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../target/test-scratch/settings-save-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        // settings.json as a non-empty directory: the rename onto it fails.
        fs::create_dir_all(dir.join("settings.json/blocker")).unwrap();
        assert!(Settings::default().save(&dir).is_err());
        assert!(!dir.join(".settings.json.tmp").exists(), "no half-written tmp left behind");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_enum_values_never_become_settings() {
        for (field, bad) in [("theme", "neon"), ("measure", "gpu"), ("period", "year")] {
            let mut v = serde_json::to_value(Settings::default()).unwrap();
            v[field] = serde_json::Value::String(bad.to_string());
            assert!(serde_json::from_value::<Settings>(v).is_err(), "{field}: {bad}");
        }
    }

    #[test]
    fn optimizer_prefs_round_trip_and_old_files_load() {
        use crate::optimizer::{Mode, OptimizerPrefs, Profile};
        let s = load_json(r#"{"threads":5}"#, 8);
        assert_eq!(s.optimizer, OptimizerPrefs::default(), "a Plan 2 settings.json has no optimizer block");
        let s = load_json(r#"{"optimizer":{"profile":"max","mode":"hold"}}"#, 8);
        assert_eq!((s.optimizer.profile, s.optimizer.mode), (Profile::Max, Mode::Hold));
        // Settings from before the sweep: the old names map onto the throttles, the old winners go.
        let s = load_json(r#"{"optimizer":{"profile":"quiet","mode":"hold","held":[],"search":null}}"#, 8);
        assert_eq!((s.optimizer.profile, s.optimizer.mode, s.optimizer.holding), (Profile::Light, Mode::Hold, None));
        assert_eq!(load_json(r#"{"optimizer":{"profile":"balanced"}}"#, 8).optimizer.profile, Profile::Efficient);
        let s = load_json(r#"{"optimizer":{"profile":"turbo"}}"#, 8);
        assert_eq!(s.optimizer, OptimizerPrefs::default(), "an unknown profile falls back alone");
    }

    /// Format-valid, checksum-invalid: never a real address (Plan 2 ruling P2-R5).
    fn synthetic_bad() -> String {
        format!("4Q{}", "A".repeat(93))
    }

    #[test]
    fn wallet_uses_the_checksum_rule() {
        use crate::config::DEFAULT_WALLET;
        Settings { wallet: Wallet(Some(DEFAULT_WALLET.to_string())), ..Settings::default() }.validate(8).expect("a real address passes");
        let e = err(&Settings { wallet: Wallet(Some(synthetic_bad())), ..Settings::default() });
        assert!(e.contains("checksum"), "{e}");
    }

    #[test]
    fn load_falls_back_a_bad_wallet_checksum_and_keeps_a_good_one() {
        use crate::config::DEFAULT_WALLET;
        let s = load_json(&format!(r#"{{"wallet":"{}"}}"#, synthetic_bad()), 8);
        assert_eq!(s.wallet, Wallet(None), "a hand-edited or damaged address falls back alone");
        let s = load_json(&format!(r#"{{"wallet":"{DEFAULT_WALLET}"}}"#), 8);
        assert_eq!(s.wallet.0.as_deref(), Some(DEFAULT_WALLET));
    }

    #[test]
    fn wallet_debug_never_leaks_the_full_address() {
        use crate::config::DEFAULT_WALLET;
        let d = format!("{:?}", Wallet(Some(DEFAULT_WALLET.to_string())));
        assert!(!d.contains(DEFAULT_WALLET), "{d}");
        assert_eq!(format!("{:?}", Wallet(None)), "None");
    }

    fn migrate_scratch(tag: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/test-scratch")
            .join(format!("settings-migrate-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("secrets")).unwrap();
        dir
    }

    #[test]
    fn migration_moves_the_old_wallet_and_worker_into_settings() {
        use crate::config::DEFAULT_WALLET;
        let dir = migrate_scratch("basic");
        fs::write(dir.join("secrets/secrets.json"), format!(r#"{{"wallet":"{DEFAULT_WALLET}","worker":"m3air"}}"#)).unwrap();
        let m = migrate_old_secrets(&dir).unwrap();
        assert!(m.wallet_migrated && m.worker_moved.as_deref() == Some("m3air"));
        assert!(!dir.join("secrets/secrets.json").exists(), "the old file is gone");
        let s = Settings::load(&dir);
        assert_eq!(s.wallet.0.as_deref(), Some(DEFAULT_WALLET));
        assert_eq!(s.worker, "m3air");
        // Idempotent: a second run is a no-op.
        assert_eq!(migrate_old_secrets(&dir).unwrap(), Migration::default());
    }

    #[test]
    fn migration_clears_the_old_empty_wallet_quirk_and_keeps_a_chosen_worker() {
        let dir = migrate_scratch("blank");
        fs::write(dir.join("secrets/secrets.json"), r#"{"wallet":"","worker":"old"}"#).unwrap();
        Settings { worker: "chosen".into(), ..Settings::default() }.save(&dir).unwrap();
        let m = migrate_old_secrets(&dir).unwrap();
        assert!(m.cleared_empty_wallet && !m.wallet_migrated);
        assert_eq!(m.worker_moved, None, "a discarded worker did not move");
        let s = Settings::load(&dir);
        assert_eq!(s.wallet, Wallet(None), "blank means none");
        assert_eq!(s.worker, "chosen", "a worker already chosen in the app wins");
    }

    #[test]
    fn migration_without_a_file_is_a_no_op() {
        let dir = migrate_scratch("none");
        fs::remove_dir_all(dir.join("secrets")).unwrap();
        assert_eq!(migrate_old_secrets(&dir).unwrap(), Migration::default());
        assert!(!dir.join("secrets/secrets.json").exists());
    }
}
