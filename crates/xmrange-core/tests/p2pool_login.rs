//! The login our xmrig sends p2pool must fit p2pool's stratum buffer: p2pool v4.18.1 reads at
//! most 1024 bytes per message and bans the sender for a longer one (`stratum_server.cpp`,
//! "sent too long Stratum message"). xmrig-mo's full algo list plus algo-perf is about 1.4 KB, so
//! p2pool mode names rx/0 and our xmrig patch then sends that one alone. Runs the real xmrig
//! against a local listener that only reads the login: nothing is mined, nothing leaves the Mac.
#![cfg(feature = "dev-diagnostics")]
use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::time::Duration;
use xmrange_core::config::MinerConfig;

const W: &str = "4QAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const P2POOL_STRATUM_BUF: usize = 1024;

fn login_line(config: MinerConfig) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    // Plain TCP so the listener reads the JSON login (moneroocean mode uses TLS).
    let mut json = MinerConfig { pool_url: format!("127.0.0.1:{port}"), tls: false, ..config }.to_xmrig_json(0, "t", None);
    json["http"]["enabled"] = false.into();
    let dir = xmrange_core::paths::scratch_dir("p2pool-login").unwrap();
    let cfg = dir.join("config.json");
    std::fs::write(&cfg, json.to_string()).unwrap();
    let mut xmrig = Command::new(xmrange_core::paths::xmrig_binary())
        .args(["-c", cfg.to_str().unwrap(), "--no-color"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let (stream, _) = listener.accept().unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut line = String::new();
    let read = BufReader::new(stream).read_line(&mut line);
    let _ = xmrig.kill();
    let _ = xmrig.wait();
    read.unwrap();
    line
}

#[test]
fn the_p2pool_login_fits_p2pools_buffer_with_the_longest_worker_name() {
    let worker = "w".repeat(64); // validate_worker's limit
    let line = login_line(MinerConfig::new(W, &worker, 1).unwrap().to_p2pool());
    assert!(line.len() <= P2POOL_STRATUM_BUF, "p2pool bans a {}-byte login: {line}", line.len());
    assert!(line.contains(r#""algo":["rx/0"]"#), "{line}");
    assert!(!line.contains(W), "p2pool takes the wallet from its own params, never from xmrig");
}

#[test]
fn the_moneroocean_login_still_advertises_every_algo() {
    let line = login_line(MinerConfig::new(W, "rig", 1).unwrap());
    assert!(line.contains("\"algo-perf\"") && line.contains("\"cn/r\"") && line.contains("\"rx/0\""), "{line}");
}
