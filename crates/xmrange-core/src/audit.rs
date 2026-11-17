//! The release audit's live scans (workflow steps 5–7), run from "save diagnostics" so the owner
//! needs no Terminal: the sockets XMRange's own processes hold, whether the wallet shows on any
//! command line, whether the xmrig API wants its token, and whether any file in the data folder
//! holds an IP. Reported as counts, ports and yes/no only: never an IP, never the wallet.
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

/// The processes XMRange runs (lsof's COMMAND column, truncated to 9 characters by lsof).
const OURS: [&str; 5] = ["xmrange-a", "xmrig", "p2pool", "arti", "monerod"];

/// One socket from `lsof -nP -i`'s NAME column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sock {
    /// Listening on this address.
    Listen(String),
    /// Connected to a loopback port (127.0.0.1 or ::1).
    Local(u16),
    /// Connected off the Mac, to this remote port.
    Remote(u16),
}

fn port(addr: &str) -> Option<u16> {
    addr.rsplit(':').next()?.parse().ok()
}

fn loopback(addr: &str) -> bool {
    addr.starts_with("127.") || addr.starts_with("[::1]") || addr.starts_with("localhost")
}

/// Parses `lsof -nP -i` output into (command, socket) pairs for XMRange's own processes.
pub fn parse_lsof(out: &str) -> Vec<(String, Sock)> {
    let mut v = Vec::new();
    for line in out.lines().skip(1) {
        let cols: Vec<&str> = line.split_whitespace().collect();
        let Some(cmd) = cols.first().map(|c| c.to_string()) else { continue };
        if !OURS.iter().any(|o| cmd.starts_with(o)) {
            continue;
        }
        let Some(name_at) = cols.iter().position(|c| *c == "TCP" || *c == "UDP").map(|i| i + 1) else { continue };
        let Some(name) = cols.get(name_at) else { continue };
        let listening = cols.get(name_at + 1) == Some(&"(LISTEN)");
        let sock = match name.split_once("->") {
            Some((_, remote)) if loopback(remote) => port(remote).map(Sock::Local),
            Some((_, remote)) => port(remote).map(Sock::Remote),
            None if listening => Some(Sock::Listen(name.to_string())),
            None => None,
        };
        if let Some(s) = sock {
            v.push((if cmd.starts_with("xmrange") { "xmrange-app".to_string() } else { cmd }, s));
        }
    }
    v
}

/// One line per process: "p2pool · listens local:3630 · local 3630 · remote 12 (37889 ×10, 37888 ×2)".
/// A listener off loopback is called out.
pub fn summarize(socks: &[(String, Sock)]) -> Vec<String> {
    let mut by: BTreeMap<&str, (Vec<String>, BTreeMap<u16, u32>, BTreeMap<u16, u32>)> = BTreeMap::new();
    for (cmd, s) in socks {
        let e = by.entry(cmd.as_str()).or_default();
        match s {
            // Words, not addresses: the report's scrubber hides every IP, loopback included.
            Sock::Listen(a) if loopback(a) => e.0.push(format!("local:{}", port(a).unwrap_or(0))),
            Sock::Listen(a) => e.0.push(format!("all:{} (OFF THE MAC)", port(a).unwrap_or(0))),
            Sock::Local(p) => *e.1.entry(*p).or_default() += 1,
            Sock::Remote(p) => *e.2.entry(*p).or_default() += 1,
        }
    }
    let ports = |m: &BTreeMap<u16, u32>| m.iter().map(|(p, n)| if *n > 1 { format!("{p} ×{n}") } else { p.to_string() }).collect::<Vec<_>>().join(", ");
    by.into_iter()
        .map(|(cmd, (mut listen, local, remote))| {
            listen.sort();
            listen.dedup();
            let mut parts = vec![cmd.to_string()];
            if !listen.is_empty() {
                parts.push(format!("listens {}", listen.join(", ")));
            }
            if !local.is_empty() {
                parts.push(format!("local {}", ports(&local)));
            }
            let n: u32 = remote.values().sum();
            parts.push(if n == 0 { "remote none".to_string() } else { format!("remote {n} ({})", ports(&remote)) });
            parts.join(" · ")
        })
        .collect()
}

/// A loopback port XMRange's own pieces use, in words.
fn local_name(port: u16) -> Option<&'static str> {
    match port {
        3620 => Some("xmrig's api"),
        3630 => Some("p2pool"),
        3631 => Some("p2pool"),
        3640 => Some("tor"),
        18081 | 18083 => Some("your monero node"),
        3632..=3635 => Some("xmrange's monero node"),
        _ => None,
    }
}

/// The Network Log's "now" (doctrine §2): one plain line per XMRange process, what it is connected
/// to, in counts and ports only. Loopback ports XMRange doesn't name (p2pool's own internal feed)
/// are left out; an address never appears.
pub fn describe(socks: &[(String, Sock)]) -> Vec<String> {
    let mut by: BTreeMap<&str, (BTreeMap<&'static str, u32>, BTreeMap<u16, u32>)> = BTreeMap::new();
    for (cmd, s) in socks {
        let e = by.entry(cmd.as_str()).or_default();
        match s {
            Sock::Local(p) => {
                if let Some(n) = local_name(*p) {
                    *e.0.entry(n).or_default() += 1;
                }
            }
            Sock::Remote(p) => *e.1.entry(*p).or_default() += 1,
            Sock::Listen(_) => {}
        }
    }
    let mut out = Vec::new();
    for (cmd, (local, remote)) in by {
        let who = match cmd {
            "xmrange-app" => "the app",
            "arti" => "tor",
            other => other,
        };
        let mut parts: Vec<String> = Vec::new();
        for (name, n) in &local {
            if cmd == "xmrange-app" && *name == "xmrig's api" {
                continue; // the app reading xmrig's figures, on this Mac
            }
            parts.push(match (cmd, *name) {
                ("p2pool", "tor") => format!("{n} peer{} through tor", if *n == 1 { "" } else { "s" }),
                ("xmrig", "tor") => "the pool through tor".to_string(),
                ("xmrange-app", "tor") => "the price feed through tor".to_string(),
                ("monerod", "tor") => format!("{n} monero peer{} through tor", if *n == 1 { "" } else { "s" }),
                (_, name) => format!("{name} on this mac"),
            });
        }
        let n: u32 = remote.values().sum();
        if n > 0 {
            let ports = remote.keys().map(|p| p.to_string()).collect::<Vec<_>>().join(", ");
            parts.push(match cmd {
                "arti" => format!("{n} tor relay{} (port {ports})", if n == 1 { "" } else { "s" }),
                "p2pool" => format!("{n} peer{} directly (port {ports})", if n == 1 { "" } else { "s" }),
                "monerod" => format!("{n} monero peer{} directly (port {ports})", if n == 1 { "" } else { "s" }),
                "xmrig" => format!("the pool directly (port {ports})"),
                _ => format!("{n} connection{} off this mac (port {ports})", if n == 1 { "" } else { "s" }),
            });
        }
        if parts.is_empty() {
            continue;
        }
        out.push(format!("{who} → {}", parts.join(" · ")));
    }
    out
}

/// The Network Log's "now", read live.
pub fn network_now() -> Vec<String> {
    match run("/usr/sbin/lsof", &["-nP", "-i"]) {
        Some(s) => {
            let lines = describe(&parse_lsof(&s));
            if lines.is_empty() { vec!["nothing connected (not mining)".into()] } else { lines }
        }
        None => vec!["cannot read connections right now".into()],
    }
}

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    String::from_utf8(out.stdout).ok()
}

/// Tor's directory cache: the public relay list every Tor client downloads, not this Mac's peers.
const TOR_DIRECTORY: &str = "tor/cache";
/// Arti's guard choice: public relays it keeps as entry points on purpose (fresh guards each start
/// would make the Mac easier to follow), so reported as Tor's own state, not as an IP at rest.
const TOR_GUARDS: &str = "guards.json";

/// Files under `dir` holding an IP address or the wallet, as paths relative to `dir`. Loopback
/// (127.0.0.1, in Arti's config) is not an IP that says anything, and Tor's public relay
/// directory is reported on its own line by `report`. The Ledger
/// (encrypted) and its key are skipped. Two files hold the wallet by design, so they are checked
/// for IPs only: `settings.json` (owner, 2026-09-28) and p2pool's 0600 `params.conf` (doctrine §1a).
pub fn at_rest(dir: &Path, wallet: &str) -> (Vec<String>, Vec<String>) {
    let (mut ips, mut wallets) = (Vec::new(), Vec::new());
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if p.strip_prefix(dir).ok() != Some(Path::new(TOR_DIRECTORY)) {
                    stack.push(p);
                }
                continue;
            }
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.ends_with(".db") || name.ends_with(".db-wal") || name.ends_with(".db-shm") || name.ends_with(".key") {
                continue;
            }
            if e.metadata().map_or(true, |m| m.len() > 64 << 20) {
                continue;
            }
            let Ok(bytes) = std::fs::read(&p) else { continue };
            let text = String::from_utf8_lossy(&bytes);
            let rel = p.strip_prefix(dir).unwrap_or(&p).display().to_string();
            if name == TOR_GUARDS {
                continue;
            }
            let text = text.replace("127.0.0.1", "");
            if crate::redact::redact_ips(&text) != text {
                ips.push(rel.clone());
            }
            if name != "settings.json" && name != "params.conf" && !wallet.is_empty() && text.contains(wallet) {
                wallets.push(rel);
            }
        }
    }
    ips.sort();
    wallets.sort();
    (ips, wallets)
}

/// The privacy check's lines for the report.
pub fn report(wallet: &str, api_port: u16) -> Vec<String> {
    let mut out = Vec::new();
    match run("/usr/sbin/lsof", &["-nP", "-i"]) {
        Some(s) => {
            let lines = summarize(&parse_lsof(&s));
            if lines.is_empty() {
                out.push("sockets: none open (not mining)".into());
            }
            out.extend(lines.into_iter().map(|l| format!("sockets: {l}")));
        }
        None => out.push("sockets: lsof unavailable".into()),
    }
    // XMRange's own processes only (any other program's command line is not ours to report).
    let on_cmdline = run("/bin/ps", &["-axo", "comm=,args="]).map(|s| {
        s.lines().any(|l| {
            let comm = l.split_whitespace().next().unwrap_or("");
            let name = comm.rsplit('/').next().unwrap_or(comm);
            ["xmrig", "p2pool", "arti", "xmrange-app", "xmrange"].contains(&name) && !wallet.is_empty() && l.contains(wallet)
        })
    });
    out.push(format!("wallet on any command line: {}", on_cmdline.map_or("unknown", |b| if b { "YES" } else { "no" })));
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(2)).build();
    out.push(match agent.get(&format!("http://127.0.0.1:{api_port}/2/summary")).call() {
        Err(ureq::Error::Status(401 | 403, _)) => "xmrig api: this mac only, token required".into(),
        Ok(_) => "xmrig api: ANSWERS WITHOUT A TOKEN".into(),
        Err(_) => "xmrig api: not running".into(),
    });
    let (ips, wallets) = at_rest(&crate::paths::data_dir(), wallet);
    out.push(if ips.is_empty() { "data folder: no ip in any file".into() } else { format!("data folder: IP IN {}", ips.join(", ")) });
    if crate::paths::data_dir().join(TOR_DIRECTORY).is_dir() {
        out.push("data folder: tor's public relay directory and its kept entry relays (tor's own state)".into());
    }
    out.push(if wallets.is_empty() { "data folder: wallet only where it belongs (settings.json, p2pool params)".into() } else { format!("data folder: WALLET ALSO IN {}", wallets.join(", ")) });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const LSOF: &str = "COMMAND     PID   USER   FD   TYPE  DEVICE SIZE/OFF NODE NAME
xmrange-a 100 helios   20u  IPv4 0x1      0t0  TCP 127.0.0.1:50100->127.0.0.1:3620 (ESTABLISHED)
xmrig     200 helios   21u  IPv4 0x2      0t0  TCP 127.0.0.1:3620 (LISTEN)
xmrig     200 helios   22u  IPv4 0x3      0t0  TCP 127.0.0.1:50200->127.0.0.1:3630 (ESTABLISHED)
p2pool    300 helios   23u  IPv4 0x4      0t0  TCP 127.0.0.1:3630 (LISTEN)
p2pool    300 helios   24u  IPv4 0x5      0t0  TCP 192.168.1.20:50300->203.0.113.9:37889 (ESTABLISHED)
p2pool    300 helios   25u  IPv4 0x6      0t0  TCP 192.168.1.20:50301->198.51.100.7:37889 (ESTABLISHED)
p2pool    300 helios   26u  IPv4 0x7      0t0  TCP 0.0.0.0:3631 (LISTEN)
Safari    400 helios   27u  IPv4 0x8      0t0  TCP 192.168.1.20:50400->198.51.100.8:443 (ESTABLISHED)";

    #[test]
    fn sockets_report_ports_and_counts_never_ips() {
        let lines = summarize(&parse_lsof(LSOF));
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(lines.iter().any(|l| l == "xmrig · listens local:3620 · local 3630 · remote none"), "{lines:?}");
        assert!(lines.iter().all(|l| crate::redact::redact_ips(l) == *l), "survives the report's scrubber unchanged: {lines:?}");
        assert!(lines.iter().any(|l| l.starts_with("p2pool · listens") && l.contains("(OFF THE MAC)") && l.ends_with("remote 2 (37889 ×2)")), "{lines:?}");
        assert!(lines.iter().all(|l| !l.contains("Safari") && !l.contains("203.0.113.9") && !l.contains("192.168.1.20")), "{lines:?}");
    }

    /// Live, on this Mac: `cargo devtest -p xmrange-core --lib audit -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn the_live_report_prints() {
        for l in report("4-not-a-real-wallet", crate::config::API_PORT) {
            println!("{l}");
        }
    }

    #[test]
    fn the_network_log_speaks_in_words_and_never_shows_an_address() {
        let lsof = "COMMAND PID USER FD TYPE DEVICE SIZE NODE NAME
xmrange-a 1 h 1u IPv4 0 0t0 TCP 127.0.0.1:50000->127.0.0.1:3620 (ESTABLISHED)
xmrig 2 h 2u IPv4 0 0t0 TCP 127.0.0.1:50001->127.0.0.1:3630 (ESTABLISHED)
p2pool 3 h 3u IPv4 0 0t0 TCP 127.0.0.1:50002->127.0.0.1:3640 (ESTABLISHED)
p2pool 3 h 4u IPv4 0 0t0 TCP 127.0.0.1:50003->127.0.0.1:3640 (ESTABLISHED)
p2pool 3 h 5u IPv4 0 0t0 TCP 127.0.0.1:50004->127.0.0.1:18083 (ESTABLISHED)
p2pool 3 h 6u IPv4 0 0t0 TCP 127.0.0.1:50005->127.0.0.1:61957 (ESTABLISHED)
arti 4 h 7u IPv4 0 0t0 TCP 192.168.1.20:50006->203.0.113.9:443 (ESTABLISHED)
arti 4 h 8u IPv4 0 0t0 TCP 192.168.1.20:50007->198.51.100.7:9001 (ESTABLISHED)";
        let lines = describe(&parse_lsof(lsof));
        assert_eq!(lines, [
            "tor → 2 tor relays (port 443, 9001)",
            "p2pool → 2 peers through tor · your monero node on this mac",
            "xmrig → p2pool on this mac",
        ], "{lines:?}");
        assert!(lines.iter().all(|l| crate::redact::redact_ips(l) == *l));
        let direct = describe(&parse_lsof("H\nxmrig 2 h 2u IPv4 0 0t0 TCP 192.168.1.20:5->203.0.113.9:10128 (ESTABLISHED)"));
        assert_eq!(direct, ["xmrig → the pool directly (port 10128)"]);
    }

    /// Live, while XMRange mines: `cargo devtest -p xmrange-core --lib live_network_log -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn the_live_network_log_prints() {
        for l in network_now() {
            println!("now: {l}");
        }
    }

    #[test]
    fn at_rest_finds_ips_and_stray_wallets_but_not_the_settings_wallet() {
        let d = crate::paths::scratch_dir("audit-at-rest").unwrap();
        std::fs::write(d.join("settings.json"), r#"{"wallet":"4WALLETX"}"#).unwrap();
        std::fs::create_dir_all(d.join("p2pool")).unwrap();
        std::fs::write(d.join("p2pool/p2pool_peers.txt"), "203.0.113.9:37889").unwrap();
        std::fs::write(d.join("p2pool/params.conf"), "wallet 4WALLETX").unwrap();
        std::fs::write(d.join("ledger.db"), "203.0.113.9 4WALLETX").unwrap();
        std::fs::write(d.join("stray.txt"), "pays 4WALLETX").unwrap();
        std::fs::create_dir_all(d.join("tor/cache")).unwrap();
        std::fs::write(d.join("tor/arti.toml"), "socks_listen = \"127.0.0.1:3640\"").unwrap();
        std::fs::write(d.join("tor/cache/microdesc"), "r relay 198.51.100.7 9001").unwrap();
        let (ips, wallets) = at_rest(&d, "4WALLETX");
        assert_eq!(ips, vec!["p2pool/p2pool_peers.txt".to_string()]);
        assert_eq!(wallets, vec!["stray.txt".to_string()], "params.conf and settings.json hold it by design");
    }
}
