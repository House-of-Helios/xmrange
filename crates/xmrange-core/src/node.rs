//! The user's own Monero node, as the Monero GUI runs it on this Mac (doctrine §1a). p2pool needs
//! its RPC (18081) and its ZMQ feed (18083, which the GUI turns on only with the startup flag
//! `--zmq-pub tcp://127.0.0.1:18083`). Read without privileges, on 127.0.0.1 only.
use serde::{Deserialize, Serialize};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

pub const RPC_PORT: u16 = 18081;
pub const ZMQ_PORT: u16 = 18083;
/// XMRange's own node (`NodeChoice::Own`): RPC, ZMQ and p2p in the app's 36xx band, all on
/// 127.0.0.1 (HouseOfHelios/PORTS.md), so it never collides with a Monero GUI node.
pub const OWN_RPC_PORT: u16 = 3632;
pub const OWN_ZMQ_PORT: u16 = 3633;
pub const OWN_P2P_PORT: u16 = 3634;

/// The RPC and ZMQ ports of the node the user chose: one probe and one p2pool config for both.
pub fn ports(choice: crate::settings::NodeChoice) -> (u16, u16) {
    match choice {
        crate::settings::NodeChoice::Gui => (RPC_PORT, ZMQ_PORT),
        crate::settings::NodeChoice::Own => (OWN_RPC_PORT, OWN_ZMQ_PORT),
    }
}
pub const ZMQ_FLAG: &str = "--zmq-pub tcp://127.0.0.1:18083";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Node {
    /// No node answers on 127.0.0.1:18081 (the Monero GUI isn't running its local node).
    Missing,
    /// The node answers but its ZMQ feed is off (the startup flag is missing).
    NoZmq,
    /// Still syncing: `height` of `target`.
    Syncing { height: u64, target: u64 },
    Ready { height: u64 },
}

#[derive(Deserialize)]
struct Info {
    #[serde(default)]
    height: u64,
    #[serde(default)]
    target_height: u64,
    #[serde(default)]
    synchronized: bool,
}

fn port_open(port: u16) -> bool {
    TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], port)), Duration::from_millis(500)).is_ok()
}

/// One look at the chosen node: RPC `get_info`, then whether the ZMQ port listens.
pub fn probe(choice: crate::settings::NodeChoice) -> Node {
    let (rpc, zmq) = ports(choice);
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(2)).build();
    let Ok(resp) = agent.get(&format!("http://127.0.0.1:{rpc}/get_info")).call() else { return Node::Missing };
    let Ok(info) = resp.into_json::<Info>() else { return Node::Missing };
    if !port_open(zmq) {
        return Node::NoZmq;
    }
    if info.synchronized && info.height > 0 {
        Node::Ready { height: info.height }
    } else {
        Node::Syncing { height: info.height, target: info.target_height.max(info.height) }
    }
}
