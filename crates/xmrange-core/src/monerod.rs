//! XMRange's own Monero node (owner, 2026-10-05): the user's alternative to the Monero GUI's node
//! for p2pool. Its chain lives in a folder the user picked with the native folder dialog, often an
//! external drive, so the folder is checked before anything is written there.
use anyhow::{bail, Result};
use std::ffi::CStr;
use std::path::{Path, PathBuf};

/// Free space a pruned chain needs, with room to grow, in GB (1e9 bytes): about 100 GiB pruned in
/// January 2026, growing (`_Research/2026-10-05-managed-node-and-chains.md`).
pub const MIN_FREE_GB: u64 = 140;
/// The folder XMRange makes inside the picked one, unless the picked folder already holds a chain.
pub const CHAIN_SUBDIR: &str = "monero-chain";

/// The only filesystems the chain database (LMDB: one memory-mapped file of many GB, with file
/// locks) goes on. FAT32 caps a file at 4 GB; exFAT corrupts silently on macOS; NTFS is read-only
/// here; network shares break the mapping and locks.
const ALLOWED_FS: [&str; 2] = ["apfs", "hfs"];

/// The filesystem type and the bytes free to this user, from `statfs` (no privileges).
fn fs_facts(dir: &Path) -> Result<(String, u64)> {
    let c = std::ffi::CString::new(dir.as_os_str().as_encoded_bytes())?;
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(c.as_ptr(), &mut st) } != 0 {
        bail!("could not read the drive: {}", std::io::Error::last_os_error());
    }
    let name = unsafe { CStr::from_ptr(st.f_fstypename.as_ptr()) }.to_string_lossy().into_owned();
    Ok((name, st.f_bavail.saturating_mul(u64::from(st.f_bsize))))
}

/// Checks a folder the user picked for the chain: it exists, this app can write there, its drive
/// holds an LMDB database, and it has room. Returns the folder to save (trimmed); None for "off".
pub fn validate_dir(folder: Option<&str>) -> Result<Option<String>> {
    let Some(f) = folder.map(str::trim).filter(|f| !f.is_empty()) else { return Ok(None) };
    let mut dir = Path::new(f);
    if !dir.is_dir() {
        bail!("the chain folder must be an existing folder");
    }
    // A picked `lmdb` folder means the chain folder around it.
    if dir.file_name().is_some_and(|n| n == "lmdb") && dir.join("data.mdb").is_file() {
        dir = dir.parent().unwrap_or(dir);
    }
    if dir.components().any(|c| c.as_os_str() == "Mobile Documents") {
        bail!("this folder is in icloud drive: pick a folder on a local or external drive");
    }
    let probe = dir.join(".xmrange-write-check");
    std::fs::write(&probe, b"").map_err(|e| anyhow::anyhow!("xmrange cannot write in this folder: {e}"))?;
    let _ = std::fs::remove_file(&probe);
    let (fs, free) = fs_facts(dir)?;
    if !ALLOWED_FS.contains(&fs.as_str()) {
        bail!("this drive is formatted {fs}: the monero chain needs apfs or mac os extended");
    }
    if !holds_chain(dir) && free < MIN_FREE_GB * 1_000_000_000 {
        bail!("this drive has {} gb free: the chain needs about {MIN_FREE_GB} gb", free / 1_000_000_000);
    }
    Ok(Some(dir.to_string_lossy().into_owned()))
}

/// A folder that already holds a Monero chain (its LMDB database).
fn holds_chain(dir: &Path) -> bool {
    dir.join("lmdb").join("data.mdb").is_file()
}

/// Where monerod keeps its data: the picked folder itself when it already holds a chain (one the
/// Monero GUI or an earlier run synced), else XMRange's own folder inside it.
pub fn data_dir_in(picked: &Path) -> PathBuf {
    if holds_chain(picked) {
        picked.to_path_buf()
    } else {
        picked.join(CHAIN_SUBDIR)
    }
}

/// monerod closes its database on SIGINT; a long sync can take this long to stop cleanly.
pub const STOP_GRACE: std::time::Duration = std::time::Duration::from_secs(120);
/// monerod's saved peer list (peer IPs): deleted at every stop, like p2pool's (no IPs at rest).
pub const PEER_FILE: &str = "p2pstate.bin";

/// monerod's arguments (`_Research/2026-10-05-managed-node-and-chains.md` §2–3): pruned, every
/// port on 127.0.0.1 in the app's band, no inbound peers, no UPnP, no update check. With Tor
/// (`socks`), every peer connection goes through Arti, and monerod itself then turns off its DNS
/// lookups (seeds, checkpoints, blocklist); without Tor it enforces DNS checkpoints, as p2pool's
/// README asks since 2025. Release builds log nothing (peer IPs).
pub fn args(data: &Path, socks: Option<&str>) -> Vec<String> {
    use crate::node::{OWN_P2P_PORT, OWN_RPC_PORT, OWN_ZMQ_PORT};
    let mut a: Vec<String> = [
        "--non-interactive",
        "--prune-blockchain",
        "--sync-pruned-blocks",
        "--rpc-bind-ip",
        "127.0.0.1",
        "--p2p-bind-ip",
        "127.0.0.1",
        "--hide-my-port",
        "--no-igd",
        "--in-peers",
        "0",
        "--check-updates",
        "disabled",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    a.extend(["--data-dir".into(), data.display().to_string()]);
    a.extend(["--rpc-bind-port".into(), OWN_RPC_PORT.to_string()]);
    a.extend(["--p2p-bind-port".into(), OWN_P2P_PORT.to_string()]);
    a.extend(["--zmq-rpc-bind-port".into(), ZMQ_RPC_PORT.to_string()]);
    a.extend(["--zmq-pub".into(), format!("tcp://127.0.0.1:{OWN_ZMQ_PORT}")]);
    match socks {
        Some(s) => a.extend(["--proxy".into(), s.to_string(), "--out-peers".into(), "12".into()]),
        None => a.extend(["--out-peers".into(), "16".into(), "--enforce-dns-checkpointing".into(), "--enable-dns-blocklist".into()]),
    }
    #[cfg(not(feature = "dev-diagnostics"))]
    a.extend(["--log-file".into(), "/dev/null".into(), "--max-log-file-size".into(), "0".into()]);
    #[cfg(feature = "dev-diagnostics")]
    a.extend(["--log-file".into(), crate::paths::data_dir().join("logs/monerod.log").display().to_string()]);
    a
}

/// monerod's ZMQ RPC server (it must run for the ZMQ feed p2pool reads); its default 18082 would
/// clash with a Monero GUI node.
pub const ZMQ_RPC_PORT: u16 = 3635;

/// XMRange's own monerod, running. Dropping it stops monerod (SIGINT, up to `STOP_GRACE`), then
/// deletes its peer list.
pub struct Node {
    proc: Option<crate::sidecar::Sidecar>,
    data: PathBuf,
}

impl Node {
    /// Starts monerod on the chain in `picked` (the folder the user chose). `socks`: Arti's address.
    pub fn start(picked: &Path, socks: Option<&str>) -> Result<Node> {
        if !picked.is_dir() {
            bail!("the chain folder is gone · reconnect its drive or choose another folder");
        }
        let data = data_dir_in(picked);
        std::fs::create_dir_all(&data)?;
        let proc = crate::sidecar::Sidecar::spawn("monerod", &crate::paths::monerod_binary(), &args(&data, socks), &data, &[])?.with_grace(STOP_GRACE);
        Ok(Node { proc: Some(proc), data })
    }

    /// Some(why) if monerod has stopped on its own.
    pub fn exited(&mut self) -> Option<String> {
        self.proc.as_mut().and_then(|p| p.exited())
    }

    /// The chain's folder is still there (an external drive can be unplugged).
    pub fn folder_present(&self) -> bool {
        self.data.is_dir()
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        if let Some(p) = self.proc.take() {
            p.stop();
        }
        let _ = std::fs::remove_file(self.data.join(PEER_FILE));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The node's arguments: local-only ports in the app's band, no inbound peers, no update check,
    /// and with Tor every peer through Arti (monerod then does no DNS of its own).
    #[test]
    fn the_node_binds_local_only_and_tor_carries_every_peer() {
        let a = args(Path::new("/chain"), None).join(" ");
        for want in ["--rpc-bind-ip 127.0.0.1", "--p2p-bind-ip 127.0.0.1", "--rpc-bind-port 3632", "--p2p-bind-port 3634", "--zmq-rpc-bind-port 3635", "--zmq-pub tcp://127.0.0.1:3633", "--in-peers 0", "--check-updates disabled", "--hide-my-port", "--data-dir /chain", "--prune-blockchain", "--enforce-dns-checkpointing"] {
            assert!(a.contains(want), "missing {want}: {a}");
        }
        assert!(!a.contains("--proxy"));
        let t = args(Path::new("/chain"), Some("127.0.0.1:3640")).join(" ");
        assert!(t.contains("--proxy 127.0.0.1:3640") && !t.contains("dns"), "{t}");
        assert_eq!(crate::node::ports(crate::settings::NodeChoice::Own), (3632, 3633), "p2pool reads the node's own ports");
    }

    #[test]
    fn a_folder_is_checked_before_the_chain_goes_there() {
        assert_eq!(validate_dir(None).unwrap(), None);
        assert_eq!(validate_dir(Some("  ")).unwrap(), None, "empty is off");
        assert!(validate_dir(Some("/definitely/not/here")).unwrap_err().to_string().contains("existing folder"));
        let d = crate::paths::scratch_dir("node-dir").unwrap();
        let (fs, _) = fs_facts(&d).unwrap();
        assert!(ALLOWED_FS.contains(&fs.as_str()), "this Mac's own drive: {fs}");
        // The repo's drive may be short of 100 GB; either way the answer is a pass or the free-space words.
        match validate_dir(Some(d.to_str().unwrap())) {
            Ok(Some(saved)) => assert_eq!(saved, d.to_str().unwrap()),
            Err(e) => assert!(e.to_string().contains("gb free"), "{e}"),
            Ok(None) => panic!("a real folder is not off"),
        }
        assert!(!d.join(".xmrange-write-check").exists(), "the write check leaves nothing");
    }

    #[test]
    fn an_existing_chain_is_used_where_it_is() {
        let d = crate::paths::scratch_dir("node-existing").unwrap();
        assert_eq!(data_dir_in(&d), d.join(CHAIN_SUBDIR));
        std::fs::create_dir_all(d.join("lmdb")).unwrap();
        std::fs::write(d.join("lmdb/data.mdb"), b"").unwrap();
        assert_eq!(data_dir_in(&d), d);
        assert!(validate_dir(Some(d.to_str().unwrap())).is_ok(), "a chain already there needs no free space");
        let picked_lmdb = d.join("lmdb");
        assert_eq!(validate_dir(Some(picked_lmdb.to_str().unwrap())).unwrap().as_deref(), d.to_str(), "a picked lmdb folder means its parent");
    }
}
