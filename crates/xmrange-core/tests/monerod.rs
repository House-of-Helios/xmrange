//! Live: XMRange's own monerod (Plan 6, M3), on a scratch chain folder inside the repo, for about
//! a minute. It starts, answers its RPC and opens its ZMQ feed on the app's own ports, finds peers,
//! then stops cleanly within its grace, leaving no peer list (no IPs at rest) and no listener.
//! Downloads a little of the chain, so it is run by hand:
//! `cargo devtest -p xmrange-core --test monerod -- --ignored --nocapture`
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};
use xmrange_core::monerod::{Node, CHAIN_SUBDIR, PEER_FILE, STOP_GRACE};
use xmrange_core::node::{probe, Node as State, OWN_RPC_PORT};
use xmrange_core::settings::NodeChoice;

fn listening(port: u16) -> bool {
    TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], port)), Duration::from_millis(300)).is_ok()
}

#[test]
#[ignore]
fn the_own_node_starts_answers_finds_peers_and_stops_clean() {
    assert!(!listening(OWN_RPC_PORT), "another node holds 3632: stop it first");
    let picked = xmrange_core::paths::scratch_dir("monerod-live").unwrap();
    let node = Node::start(&picked, None).expect("monerod starts");

    // Up: the RPC answers and the ZMQ feed listens (probe says syncing or ready, never missing).
    let t0 = Instant::now();
    let mut seen = State::Missing;
    while t0.elapsed() < Duration::from_secs(90) {
        seen = probe(NodeChoice::Own);
        if matches!(seen, State::Syncing { .. } | State::Ready { .. }) {
            break;
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    eprintln!("up after {:?}: {seen:?}", t0.elapsed());
    assert!(matches!(seen, State::Syncing { .. } | State::Ready { .. }), "never answered: {seen:?}");

    // Peers: a target height above zero means peers told it where the chain is.
    let t1 = Instant::now();
    let mut target = 0;
    while t1.elapsed() < Duration::from_secs(60) {
        if let State::Syncing { target: t, .. } | State::Ready { height: t } = probe(NodeChoice::Own) {
            target = t;
            if t > 1 {
                break;
            }
        }
        std::thread::sleep(Duration::from_secs(3));
    }
    eprintln!("chain target {target} after {:?}", t1.elapsed());
    assert!(target > 3_000_000, "no peers reported the chain's height");

    // Stop: within the grace, the peer list gone, nothing listening.
    let t2 = Instant::now();
    drop(node);
    eprintln!("stopped in {:?}", t2.elapsed());
    assert!(t2.elapsed() < STOP_GRACE, "monerod needed SIGKILL");
    assert!(!picked.join(CHAIN_SUBDIR).join(PEER_FILE).exists(), "peer IPs left at rest");
    assert!(!listening(OWN_RPC_PORT));
    let _ = std::fs::remove_dir_all(&picked);
}

/// The same node through the bundled Tor (the default): every peer through Arti's SOCKS port, so
/// monerod itself holds no connection off this Mac; still finds the chain's height.
#[test]
#[ignore]
fn the_own_node_finds_peers_through_tor() {
    assert!(!listening(OWN_RPC_PORT), "another node holds 3632: stop it first");
    let picked = xmrange_core::paths::scratch_dir("monerod-tor").unwrap();
    let lease = xmrange_core::tor::acquire(&|| false).expect("arti starts");
    let node = Node::start(&picked, Some(&xmrange_core::tor::socks_addr())).expect("monerod starts");
    let t0 = Instant::now();
    let mut target = 0;
    while t0.elapsed() < Duration::from_secs(240) {
        if let State::Syncing { target: t, .. } | State::Ready { height: t } = probe(NodeChoice::Own) {
            target = t;
            if t > 3_000_000 {
                break;
            }
        }
        std::thread::sleep(Duration::from_secs(3));
    }
    eprintln!("through tor: chain target {target} after {:?}", t0.elapsed());
    // monerod's own sockets: loopback only (Arti and its listeners), never a remote address.
    let out = std::process::Command::new("/usr/sbin/lsof").args(["-nP", "-a", "-c", "monerod", "-i"]).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let remote = text.lines().filter(|l| l.contains("->") && !l.contains("->127.0.0.1:")).count();
    eprintln!("monerod sockets: {} lines, {remote} off this mac", text.lines().count());
    drop(node);
    drop(lease);
    let _ = std::fs::remove_dir_all(&picked);
    assert!(target > 3_000_000, "no peers through tor");
    assert_eq!(remote, 0, "monerod connected off this mac directly");
}
