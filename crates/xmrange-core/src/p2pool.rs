//! p2pool mode (doctrine §1a): p2pool runs beside xmrig, talks to the user's own Monero node on
//! 127.0.0.1, and pays the wallet on-chain. xmrig mines to p2pool's local stratum.
//! - The wallet reaches p2pool only through a 0600 params file, never the command line.
//! - Stratum and the p2p listener bind 127.0.0.1: peers are outbound only, nothing listens on the
//!   network, and UPnP is compiled out (tools/build-p2pool.sh).
//! - The chain is the user's (nano by default: at a Mac's few kH/s it finds shares often enough to
//!   be paid steadily). Each chain keeps its own data dir, so a switch never mixes share chains.
//! - `light-mode`: no second 2 GB RandomX dataset beside xmrig's.
//! - With Tor: every peer connection goes through Arti, with `no-dns`, so peers come from p2pool's
//!   built-in onion seeds and no DNS lookup leaves the Mac.
//! - No log file (peer IPs); p2pool's peer lists and its data-api folder (peer IPs in `local/p2p`,
//!   the wallet in `local/stratum`) are deleted at stop, so neither stays at rest.
use crate::settings::Chain;
use crate::sidecar::Sidecar;
use anyhow::{bail, Result};
use serde::Serialize;
use std::fs;
use std::net::{SocketAddr, TcpStream};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const STRATUM_PORT: u16 = 3630;
pub const P2P_PORT: u16 = 3631;
/// p2pool opens its stratum once it has the node's chain state: seconds on a synced node.
const STRATUM_LIMIT: Duration = Duration::from_secs(90);
/// p2pool v4.18.1's saved peer lists (vendor/p2pool/src/p2p_server.cpp `saved_*_peer_list_file_name`).
const PEER_FILES: [&str; 3] = ["p2pool_peers.txt", "p2pool_onion_peers.txt", "p2pool_i2p_peers.txt"];
/// p2pool's stderr lines that are expected on every Mac start, so never shown: macOS has no large
/// pages (it falls back to normal memory), and p2pool runs without a terminal for its console.
const EXPECTED_WARNINGS: [&str; 2] = ["using large pages", "tty or named pipe is not available"];

/// The latest p2pool session's output, in memory only, for "save diagnostics" (scrubbed there):
/// whether p2pool built a block template and whether xmrig logged in. Kept after a stop, replaced
/// at the next start, gone when the app quits.
static RECENT: Mutex<Option<Arc<Mutex<std::collections::VecDeque<String>>>>> = Mutex::new(None);

/// p2pool's last output lines (at most 50), oldest first; empty if p2pool hasn't run this launch.
pub fn recent_output() -> Vec<String> {
    let held = RECENT.lock().unwrap_or_else(|e| e.into_inner()).clone();
    held.map(|l| l.lock().unwrap_or_else(|e| e.into_inner()).iter().cloned().collect()).unwrap_or_default()
}

/// What p2pool reports (its own data-api files): enough for the expected-earnings figure and the
/// share timeline, the same inputs Gupaxx's calculator uses.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Stats {
    /// Connected p2pool peers.
    pub peers: Option<u64>,
    /// Shares this Mac found on the share chain, this session.
    pub shares_found: Option<u64>,
    /// Monero's difficulty and block reward (atomic units) at the chain tip, from the node.
    pub net_difficulty: Option<f64>,
    pub net_reward: Option<u64>,
    /// Hashes one share on p2pool's chain takes, on average.
    pub share_difficulty: Option<f64>,
}

/// One payout p2pool reported to this wallet: its amount (atomic units) and Monero block height.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Payout {
    pub atomic: u64,
    pub height: u64,
}

/// Reads p2pool's own payout line ("Your wallet … got a payout of 0.000412345678 XMR in block
/// 3775700", `side_chain.cpp`). The wallet in the line is never kept.
pub fn parse_payout(line: &str) -> Option<Payout> {
    let rest = &line[line.find("got a payout of ")? + "got a payout of ".len()..];
    let (xmr, rest) = rest.split_once(" XMR in block ")?;
    let (whole, frac) = xmr.trim().split_once('.').unwrap_or((xmr.trim(), ""));
    if frac.len() > 12 || !whole.chars().chain(frac.chars()).all(|c| c.is_ascii_digit()) || whole.is_empty() {
        return None;
    }
    let atomic = whole.parse::<u64>().ok()?.checked_mul(1_000_000_000_000)?.checked_add(format!("{frac:0<12}").parse::<u64>().ok()?)?;
    let height = rest.trim().split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()?;
    Some(Payout { atomic, height })
}

pub fn stratum_url() -> String {
    format!("127.0.0.1:{STRATUM_PORT}")
}

/// p2pool pays a primary address only (starts with 4); a subaddress starts with 8.
pub fn check_wallet(wallet: &str) -> Result<()> {
    if !wallet.starts_with('4') {
        bail!("p2pool pays a primary address (starting with 4), not a subaddress");
    }
    Ok(())
}

/// Dropping it stops p2pool, then deletes its peer lists and data-api folder (no IPs at rest).
pub struct P2pool {
    proc: Option<Sidecar>,
    dir: PathBuf,
}

impl P2pool {
    /// Starts p2pool and returns once its stratum accepts xmrig, or early (an error) if `cancel()`
    /// says stop. `socks`: Arti's address.
    pub fn start(wallet: &str, socks: Option<&str>, chain: Chain, node: (u16, u16), cancel: &dyn Fn() -> bool) -> Result<P2pool> {
        check_wallet(wallet)?;
        let dir = chain_dir(chain);
        fs::DirBuilder::new().recursive(true).mode(0o700).create(dir.join("api"))?;
        let params = dir.join("params.conf");
        let mut f = fs::OpenOptions::new().create(true).write(true).truncate(true).mode(0o600).open(&params)?;
        std::io::Write::write_all(&mut f, params_text(wallet, &dir, socks, chain, node).as_bytes())?;
        drop(f);
        let args = vec!["--params-file".to_string(), params.display().to_string()];
        let mut proc = Sidecar::spawn("p2pool", &crate::paths::p2pool_binary(), &args, &params, &["got a payout of "])?;
        *RECENT.lock().unwrap_or_else(|e| e.into_inner()) = Some(proc.lines_handle());
        proc.wait_until(STRATUM_LIMIT, cancel, |_| TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], STRATUM_PORT)), Duration::from_millis(300)).is_ok())?;
        Ok(P2pool { proc: Some(proc), dir })
    }

    pub fn stats(&self) -> Stats {
        stats_from(&self.dir.join("api"))
    }

    /// Payouts p2pool reported since the last call.
    pub fn take_payouts(&self) -> Vec<Payout> {
        self.proc.as_ref().map_or_else(Vec::new, |p| p.take_marked()).iter().filter_map(|l| parse_payout(l)).collect()
    }

    /// p2pool's warnings and errors (its stderr) since the last call, as tape text.
    pub fn take_errors(&self) -> Vec<String> {
        let lines = self.proc.as_ref().map_or_else(Vec::new, |p| p.take_errors());
        lines.iter().filter(|l| !expected_warning(l)).map(|l| tape_text(l)).collect()
    }

    /// Whether p2pool has said its share chain is synced (its own "SYNCHRONIZED" line).
    pub fn synced(&self) -> bool {
        self.proc.as_ref().is_some_and(|p| p.said("SYNCHRONIZED"))
    }

    /// Some(why) if p2pool has stopped.
    pub fn exited(&mut self) -> Option<String> {
        self.proc.as_mut().and_then(|p| p.exited())
    }
}

impl Drop for P2pool {
    fn drop(&mut self) {
        if let Some(p) = self.proc.take() {
            p.stop();
        }
        forget_peers(&self.dir);
    }
}

fn forget_peers(dir: &Path) {
    for f in PEER_FILES {
        let _ = fs::remove_file(dir.join(f));
    }
    // p2pool rewrites these while it runs; read only then (`stats`).
    let _ = fs::remove_dir_all(dir.join("api"));
}

/// Each chain's own data dir (nano keeps the folder it always had).
pub fn chain_dir(chain: Chain) -> PathBuf {
    crate::paths::data_dir().join(match chain {
        Chain::Nano => "p2pool",
        Chain::Mini => "p2pool-mini",
        Chain::Main => "p2pool-main",
    })
}

/// The params file (p2pool's `--params-file` format: `name value`, one per line). The main chain
/// takes no option; mini and nano take theirs (vendor/p2pool/src/params.cpp). `node`: the RPC and
/// ZMQ ports of the node the user chose.
pub fn params_text(wallet: &str, dir: &Path, socks: Option<&str>, chain: Chain, node: (u16, u16)) -> String {
    let q = |p: &Path| crate::sidecar::quoted(&p.display().to_string());
    let chain_line = match chain {
        Chain::Main => String::new(),
        c => format!("{}\n", c.name()),
    };
    let mut t = format!(
        "wallet {wallet}\nhost 127.0.0.1\nrpc-port {}\nzmq-port {}\nstratum 127.0.0.1:{STRATUM_PORT}\np2p 127.0.0.1:{P2P_PORT}\n{chain_line}light-mode\nno-log-file\nno-color\nloglevel 2\ndata-dir {}\ndata-api {}\nlocal-api\n",
        node.0,
        node.1,
        q(dir),
        q(&dir.join("api")),
    );
    if let Some(s) = socks {
        t.push_str(&format!("socks5 {s}\nsocks5-proxy-type tor\nno-dns\n"));
    }
    t
}

fn expected_warning(line: &str) -> bool {
    EXPECTED_WARNINGS.iter().any(|w| line.contains(w))
}

/// One p2pool console line for the tape: p2pool's own timestamp dropped (the tape has its own),
/// IPs redacted.
pub fn tape_text(line: &str) -> String {
    let mut parts = line.splitn(3, ' ');
    let msg = match (parts.next(), parts.next(), parts.next()) {
        (Some(date), Some(_time), Some(rest)) if date.starts_with(|c: char| c.is_ascii_digit()) && date.contains('-') => rest,
        _ => line,
    };
    format!("p2pool: {}", crate::redact::redact_ips(msg.trim()))
}

fn read_json(path: &Path) -> Option<serde_json::Value> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

/// Reads p2pool's data-api files. Missing files or fields are None: p2pool writes them as it goes.
pub fn stats_from(api: &Path) -> Stats {
    let stratum = read_json(&api.join("local/stratum"));
    let p2p = read_json(&api.join("local/p2p"));
    let pool = read_json(&api.join("pool/stats"));
    let net = read_json(&api.join("network/stats"));
    let ps = pool.as_ref().and_then(|v| v.get("pool_statistics").cloned());
    let positive = |v: Option<f64>| v.filter(|d| *d > 0.0);
    Stats {
        peers: p2p.as_ref().and_then(|v| v.get("connections")?.as_u64()),
        shares_found: stratum.as_ref().and_then(|v| v.get("shares_found")?.as_u64()),
        net_difficulty: positive(net.as_ref().and_then(|v| v.get("difficulty")?.as_f64())),
        net_reward: net.as_ref().and_then(|v| v.get("reward")?.as_u64()).filter(|r| *r > 0),
        share_difficulty: positive(ps.as_ref().and_then(|v| v.get("sidechainDifficulty")?.as_f64())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: &str = "47C4vQftcmvKEmz6NQPsjc3LYnP3Rk5kZW1YD7PT3crbKrXaEUvg42QZ9vXM2Czk5bhoWLkF9wjJiMyMYUBCZYeRGjDhzPB";

    #[test]
    fn params_bind_local_only_and_tor_adds_no_dns() {
        let t = params_text(W, Path::new("/data/p2pool"), None, Chain::Nano, (18081, 18083));
        assert!(t.contains("stratum 127.0.0.1:3630\n") && t.contains("p2p 127.0.0.1:3631\n"));
        assert!(t.contains("\nnano\n") && t.contains("\nlight-mode\n") && t.contains("\nno-log-file\n"));
        assert!(t.contains("rpc-port 18081\nzmq-port 18083\n"));
        assert!(!t.contains("upnp"), "UPnP is compiled out (tools/build-p2pool.sh); the option would be refused");
        assert!(!t.contains("socks5"), "no proxy unless Tor is on");
        let tor = params_text(W, Path::new("/data/p2pool"), Some("127.0.0.1:3640"), Chain::Nano, (18081, 18083));
        assert!(tor.contains("socks5 127.0.0.1:3640\nsocks5-proxy-type tor\nno-dns\n"));
    }

    /// Each chain gets p2pool's own option (main: none), its own data dir, and the chosen node's ports.
    #[test]
    fn each_chain_gets_its_option_and_dir_and_the_chosen_nodes_ports() {
        let src = include_str!("../../../vendor/p2pool/src/params.cpp");
        for (chain, line) in [(Chain::Nano, Some("nano")), (Chain::Mini, Some("mini")), (Chain::Main, None)] {
            let t = params_text(W, Path::new("/d"), None, chain, (3632, 3633));
            assert!(t.contains("rpc-port 3632\nzmq-port 3633\n"));
            for opt in ["nano", "mini"] {
                assert_eq!(t.contains(&format!("\n{opt}\n")), line == Some(opt), "{chain:?}: {t}");
                assert!(src.contains(&format!("arg[0] == \"{opt}\"")), "p2pool reads {opt}");
            }
        }
        let dirs: std::collections::HashSet<_> = [Chain::Nano, Chain::Mini, Chain::Main].into_iter().map(chain_dir).collect();
        assert_eq!(dirs.len(), 3, "a chain switch never shares a data dir");
        assert!(chain_dir(Chain::Nano).ends_with("p2pool"), "nano keeps its folder");
    }

    #[test]
    fn peer_lists_carry_p2pools_own_names_and_are_deleted() {
        let src = include_str!("../../../vendor/p2pool/src/p2p_server.cpp");
        for f in PEER_FILES {
            assert!(src.contains(&format!("\"{f}\"")), "{f} is a name p2pool writes");
        }
        let d = crate::paths::scratch_dir("p2pool-peers").unwrap();
        for f in PEER_FILES {
            fs::write(d.join(f), "203.0.113.9:37889").unwrap();
        }
        fs::create_dir_all(d.join("api/local")).unwrap();
        fs::write(d.join("api/local/p2p"), r#"{"peers":["203.0.113.9:37889"]}"#).unwrap();
        fs::write(d.join("api/local/stratum"), r#"{"wallet":"4x"}"#).unwrap();
        forget_peers(&d);
        assert!(PEER_FILES.iter().all(|f| !d.join(f).exists()));
        assert!(!d.join("api").exists(), "the data-api folder (peer IPs, the wallet) goes too");
    }

    #[test]
    fn paths_with_spaces_quotes_and_accents_quote_cleanly() {
        let t = params_text(W, Path::new("/Users/José \"J\"/Library/Application Support/XMRange/p2pool"), None, Chain::Nano, (18081, 18083));
        assert!(t.contains("data-dir \"/Users/José \\\"J\\\"/Library/Application Support/XMRange/p2pool\"\n"), "{t}");
        assert!(!t.contains("\\u{"), "no Rust Debug escapes");
    }

    #[test]
    fn tape_text_drops_p2pools_timestamp_and_redacts_ips() {
        let t = tape_text("2026-10-2 14:3:7.0412 P2PServer peer 203.0.113.9:37889 banned");
        assert!(t.starts_with("p2pool: P2PServer peer ") && t.ends_with(" banned"), "{t}");
        assert!(!t.contains("203.0.113.9") && !t.contains("2026-10-2"), "{t}");
        assert_eq!(tape_text("no timestamp here"), "p2pool: no timestamp here");
    }

    #[test]
    fn a_macs_expected_start_warnings_stay_off_the_tape() {
        assert!(expected_warning("2026-10-2 20:21:32.1 RandomX_Hasher couldn't allocate RandomX cache using large pages"));
        assert!(expected_warning("2026-10-2 20:21:32.1 RandomX_Hasher couldn't allocate RandomX light VM using large pages"));
        assert!(expected_warning("2026-10-2 20:21:32.1 ConsoleCommands tty or named pipe is not available"));
        assert!(!expected_warning("2026-10-2 20:21:32.1 P2Pool get_miner_data RPC request failed"));
    }

    #[test]
    fn payouts_read_from_p2pools_own_line_without_the_wallet() {
        let line = format!("2026-10-3 4:12:9.5512 SideChain Your wallet {W} got a payout of 0.000412345678 XMR in block 3775700");
        assert_eq!(parse_payout(&line), Some(Payout { atomic: 412_345_678, height: 3_775_700 }));
        assert_eq!(parse_payout("x got a payout of 1.5 XMR in block 9"), Some(Payout { atomic: 1_500_000_000_000, height: 9 }));
        assert_eq!(parse_payout(&format!("Your wallet {W} didn't get a payout in block 3775701 because you had no shares in PPLNS window")), None);
        assert_eq!(parse_payout("got a payout of 0.1234567890123 XMR in block 9"), None, "more than 12 decimals is not XMR");
    }

    #[test]
    fn a_subaddress_is_refused() {
        assert!(check_wallet(W).is_ok());
        assert!(check_wallet(&W.replacen('4', "8", 1)).is_err());
    }

    #[test]
    fn stats_read_p2pools_own_files() {
        let d = crate::paths::scratch_dir("p2pool-api").unwrap();
        fs::create_dir_all(d.join("local")).unwrap();
        fs::create_dir_all(d.join("pool")).unwrap();
        fs::create_dir_all(d.join("network")).unwrap();
        fs::write(d.join("local/stratum"), r#"{"hashrate_15m":2400,"shares_found":3,"current_effort":41.5,"block_reward_share_percent":0.012,"wallet":"4x"}"#).unwrap();
        fs::write(d.join("local/p2p"), r#"{"connections":12,"incoming_connections":0}"#).unwrap();
        fs::write(d.join("pool/stats"), r#"{"pool_list":["pplns"],"pool_statistics":{"hashRate":5100000,"miners":480,"pplnsWindowSize":2160,"sidechainDifficulty":120673173,"sidechainHeight":1453123}}"#).unwrap();
        fs::write(d.join("network/stats"), r#"{"difficulty":753538665462,"hash":"69dd","height":3775602,"reward":613199000000,"timestamp":1790989758}"#).unwrap();
        let s = stats_from(&d);
        assert_eq!(s, Stats { peers: Some(12), shares_found: Some(3), net_difficulty: Some(753538665462.0), net_reward: Some(613199000000), share_difficulty: Some(120673173.0) });
        assert_eq!(stats_from(&d.join("nope")), Stats::default());
    }
}
