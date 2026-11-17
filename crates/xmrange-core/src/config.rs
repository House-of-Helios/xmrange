use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;

pub const DEFAULT_POOL: &str = "gulf.moneroocean.stream:20004";
pub const API_PORT: u16 = 3620;

/// Default wallet when the user has not set their own: the GhostDrip dev wallet (not an XMRange wallet).
/// Always disclosed to the user — never mine to it silently.
pub const DEFAULT_WALLET: &str = "47C4vQftcmvKEmz6NQPsjc3LYnP3Rk5kZW1YD7PT3crbKrXaEUvg42QZ9vXM2Czk5bhoWLkF9wjJiMyMYUBCZYeRGjDhzPB";
const M3_ALGO_PERF: &str = include_str!("algo_perf_m3.json");

#[derive(Clone, PartialEq)]
pub struct MinerConfig {
    pub pool_url: String,
    pub tls: bool,
    /// p2pool mode: xmrig mines to the local p2pool (which pays `wallet`); the pool login is then
    /// the worker name, and the wallet never goes over the stratum socket.
    pub p2pool: bool,
    /// Tor: Arti's SOCKS address for the pool socket (MoneroOcean mode only; p2pool is local).
    pub socks5: Option<String>,
    pub wallet: String,
    pub worker: String,
    pub threads: u32,
    pub algo_perf: BTreeMap<String, f64>,
    /// xmrig `cpu.yield`: workers step aside between hash batches so other apps get the CPU at
    /// once. On by default; off only in an experiment (`xmrange trial --no-yield`).
    pub cpu_yield: bool,
    /// Mining threads at utility QoS (our xmrig patch, `cpu.qos`): lower clocks, politer to other
    /// apps, switchable live. Measured on the M3 Air: -18% h/s, -31% W, +19% h/w, 35°F cooler, and
    /// a user's work slowed 22% instead of 65–74% (runs/2026-10-03-yield-utility).
    pub utility: bool,
}

/// Char-safe mask: first 4 + '…' + last 4 (never splits a multi-byte codepoint).
pub fn mask_wallet(a: &str) -> String {
    let head: String = a.chars().take(4).collect();
    let n = a.chars().count();
    let tail: String = a.chars().skip(n.saturating_sub(4)).collect();
    format!("{head}…{tail}")
}

impl std::fmt::Debug for MinerConfig {
    /// Manual impl: `{:?}` must never leak the full wallet address into logs.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MinerConfig")
            .field("pool_url", &self.pool_url)
            .field("tls", &self.tls)
            .field("p2pool", &self.p2pool)
            .field("socks5", &self.socks5)
            .field("wallet", &mask_wallet(&self.wallet))
            .field("worker", &self.worker)
            .field("threads", &self.threads)
            .field("algo_perf_entries", &self.algo_perf.len())
            .finish()
    }
}

impl MinerConfig {
    pub fn new(wallet: &str, worker: &str, threads: u32) -> Result<Self> {
        validate_wallet(wallet)?;
        validate_worker(worker)?;
        if threads == 0 {
            bail!("threads must be at least 1");
        }
        Ok(Self {
            pool_url: DEFAULT_POOL.to_string(),
            tls: true,
            p2pool: false,
            socks5: None,
            wallet: wallet.to_string(),
            worker: worker.to_string(),
            threads,
            algo_perf: default_algo_perf(),
            cpu_yield: true,
            utility: false,
        })
    }

    /// The same miner pointed at the local p2pool's stratum instead of MoneroOcean.
    pub fn to_p2pool(mut self) -> Self {
        self.pool_url = crate::p2pool::stratum_url();
        self.tls = false;
        self.p2pool = true;
        self.socks5 = None;
        self
    }

    pub fn pool_pass(&self) -> String {
        format!("{}~rx/0", self.worker)
    }

    pub fn to_xmrig_json(&self, api_port: u16, token: &str, log_file: Option<&Path>) -> Value {
        let mut perf = self.algo_perf.clone();
        if let Some(rx0) = perf.get("rx/0").copied() {
            perf.entry("rx/2".to_string()).or_insert(rx0);
        }
        json!({
            "autosave": false,
            "watch": false,
            "background": false,
            "colors": false,
            "donate-level": 0,
            "print-time": 60,
            "log-file": log_file.map(|p| p.display().to_string()),
            "http": {
                "enabled": true,
                "host": "127.0.0.1",
                "port": api_port,
                "access-token": token,
                "restricted": false
            },
            "randomx": { "mode": "fast", "1gb-pages": false },
            "cpu": {
                "enabled": true,
                // Intel macOS gives 2 MB superpages without root (`_Research/2026-10-05-intel-power-
                // readings.md` §6); xmrig falls back to normal pages when the request fails. Apple
                // Silicon has none to give.
                "huge-pages": cfg!(target_arch = "x86_64"),
                "yield": self.cpu_yield,
                "qos": if self.utility { Some("utility") } else { None },
                "priority": null,
                "rx": vec![-1; self.threads as usize]
            },
            "algo-perf": perf,
            "pools": [{
                "url": self.pool_url,
                "user": if self.p2pool { &self.worker } else { &self.wallet },
                "pass": if self.p2pool { "x".to_string() } else { self.pool_pass() },
                // p2pool: naming the algorithm keeps xmrig's login to rx/0 alone (our xmrig patch).
                // The full list with algo-perf is about 1.4 KB; p2pool bans a login over 1024 bytes.
                "algo": if self.p2pool { Some(P2POOL_ALGO) } else { None },
                "keepalive": true,
                "tls": self.tls,
                "socks5": self.socks5
            }]
        })
    }
}

/// The only algorithm p2pool mines.
pub const P2POOL_ALGO: &str = "rx/0";

pub fn default_algo_perf() -> BTreeMap<String, f64> {
    serde_json::from_str(M3_ALGO_PERF).expect("embedded algo-perf is valid JSON")
}

pub fn validate_worker(w: &str) -> Result<()> {
    let ok = !w.is_empty()
        && w.len() <= 64
        && w.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.');
    if !ok {
        bail!("worker name must be 1–64 characters of letters, digits, '-', '_' or '.'");
    }
    Ok(())
}

pub fn validate_wallet(a: &str) -> Result<()> {
    let base58 = |c: char| c.is_ascii() && crate::wallet::ALPHABET.contains(&(c as u8));
    let ok = a.len() == 95 && (a.starts_with('4') || a.starts_with('8')) && a.chars().all(base58);
    if !ok {
        bail!("not a Monero primary address or subaddress (95 base58 characters starting with 4 or 8)");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const W: &str = "4QAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

    #[test]
    fn builds_private_xmrig_json() {
        let c = MinerConfig::new(W, "xmrange-m3air", 5).unwrap();
        let j = c.to_xmrig_json(3620, "tok123", None);
        assert_eq!(j["autosave"], false);
        assert_eq!(j["watch"], false);
        assert_eq!(j["donate-level"], 0);
        assert_eq!(j["log-file"], Value::Null);
        assert_eq!(j["http"]["host"], "127.0.0.1");
        assert_eq!(j["http"]["port"], 3620);
        assert_eq!(j["http"]["access-token"], "tok123");
        assert_eq!(j["http"]["restricted"], false);
        assert_eq!(j["cpu"]["rx"].as_array().unwrap().len(), 5);
        assert_eq!(j["randomx"]["mode"], "fast");
        assert_eq!(j["pools"][0]["url"], DEFAULT_POOL);
        assert_eq!(j["pools"][0]["tls"], true);
        assert_eq!(j["pools"][0]["user"], W);
        assert_eq!(j["pools"][0]["pass"], "xmrange-m3air~rx/0");
        assert_eq!(j["algo-perf"]["rx/2"], j["algo-perf"]["rx/0"]);
        assert!(j["algo-perf"].as_object().unwrap().len() >= 27, "every algo, or MO re-benchmarks");
        // the wallet appears exactly once: the pool user field
        assert_eq!(j.to_string().matches(W).count(), 1);
    }

    #[test]
    fn only_p2pool_names_the_algorithm() {
        let mo = MinerConfig::new(W, "rig", 1).unwrap();
        assert_eq!(mo.to_xmrig_json(3620, "t", None)["pools"][0]["algo"], Value::Null, "moneroocean picks the algo from algo-perf");
        let p2 = mo.to_p2pool().to_xmrig_json(3620, "t", None);
        assert_eq!(p2["pools"][0]["algo"], "rx/0");
        assert_eq!(p2["pools"][0]["url"], "127.0.0.1:3630");
        assert_eq!(p2["pools"][0]["tls"], false);
    }

    #[test]
    fn rx2_is_filled_from_rx0_when_missing() {
        let mut c = MinerConfig::new(W, "rig", 1).unwrap();
        c.algo_perf.remove("rx/2");
        let j = c.to_xmrig_json(3620, "t", None);
        assert_eq!(j["algo-perf"]["rx/2"], j["algo-perf"]["rx/0"]);
    }

    #[test]
    fn log_file_is_passed_through() {
        let c = MinerConfig::new(W, "rig", 1).unwrap();
        let j = c.to_xmrig_json(3620, "t", Some(Path::new("/x/y.log")));
        assert_eq!(j["log-file"], "/x/y.log");
    }

    #[test]
    fn rejects_bad_worker_names() {
        for bad in ["", "my rig", "a~b", "🚀rig", "rig/1", &"x".repeat(65)] {
            let e = MinerConfig::new(W, bad, 4).unwrap_err().to_string();
            assert!(e.contains("worker name"), "{bad:?} → {e}");
        }
        for good in ["xmrange-m3air", "rig_1", "M3.air", &"x".repeat(64)] {
            assert!(validate_worker(good).is_ok(), "{good:?}");
        }
    }

    #[test]
    fn default_wallet_is_valid() {
        assert!(validate_wallet(DEFAULT_WALLET).is_ok());
    }

    #[test]
    fn debug_masks_the_wallet() {
        let c = MinerConfig::new(W, "rig", 4).unwrap();
        let d = format!("{:?}", c);
        assert!(!d.contains(W), "{d}");
        assert!(d.contains("4QAA") && d.contains("AAAA"), "{d}");
    }

    #[test]
    fn rejects_bad_wallets_and_zero_threads() {
        assert!(MinerConfig::new("4short", "rig", 4).is_err());
        assert!(MinerConfig::new(&W.replacen('4', "1", 1), "rig", 4).is_err());
        assert!(MinerConfig::new(&W.replacen('Q', "0", 1), "rig", 4).is_err(), "0 is not base58");
        assert!(MinerConfig::new(W, "rig", 0).is_err());
    }
}
