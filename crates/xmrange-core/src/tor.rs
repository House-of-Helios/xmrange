//! Tor for the mining connections (doctrine §1a): Arti, the Tor Project's Rust client, bundled.
//! One SOCKS port on 127.0.0.1, no DNS port, its cache and state inside the data dir. xmrig
//! (MoneroOcean mode) or p2pool (its peers) connect through it; nothing else does.
use crate::sidecar::Sidecar;
use anyhow::Result;
use std::fs;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::time::Duration;

pub const SOCKS_PORT: u16 = 3640;
/// A first start downloads the directory: about 10 s measured, but once over 90 s on the M3 Air
/// (2026-10-03); later starts reuse the cache. With Tor on by default, a new user's first start
/// must not fail on a slow network.
pub const BOOTSTRAP_LIMIT: Duration = Duration::from_secs(180);

pub fn socks_addr() -> String {
    format!("127.0.0.1:{SOCKS_PORT}")
}

pub struct Tor {
    proc: Sidecar,
}

impl Tor {
    /// Starts Arti and returns once it can carry traffic, or early (an error) if `cancel()` says stop.
    pub fn start(cancel: &dyn Fn() -> bool) -> Result<Tor> {
        let dir = crate::paths::data_dir().join("tor");
        fs::DirBuilder::new().recursive(true).mode(0o700).create(&dir)?;
        let q = |p: &std::path::Path| crate::sidecar::quoted(&p.display().to_string());
        let config = dir.join("arti.toml");
        let text = format!(
            "[proxy]\nsocks_listen = \"{}\"\ndns_listen = 0\n\n[storage]\ncache_dir = {}\nstate_dir = {}\n\n[logging]\nconsole = \"info\"\n\n[address_filter]\nallow_onion_addrs = true\n",
            socks_addr(),
            q(&dir.join("cache")),
            q(&dir.join("state")),
        );
        let mut f = fs::OpenOptions::new().create(true).write(true).truncate(true).mode(0o600).open(&config)?;
        std::io::Write::write_all(&mut f, text.as_bytes())?;
        let args = vec!["proxy".to_string(), "-c".to_string(), config.display().to_string()];
        let mut proc = Sidecar::spawn("arti", &crate::paths::arti_binary(), &args, &config, &[])?;
        proc.wait_until(BOOTSTRAP_LIMIT, cancel, |p| p.said("Sufficiently bootstrapped"))?;
        Ok(Tor { proc })
    }

    /// Some(why) if Arti has stopped.
    pub fn exited(&mut self) -> Option<String> {
        self.proc.exited()
    }

    pub fn stop(self) {
        self.proc.stop();
    }
}

/// The one Arti this app runs, shared: the engine (xmrig or p2pool) and XMRange's own Monero node
/// both route through it, on the one SOCKS port. The first lease starts Arti, the last one to drop
/// stops it.
static SHARED: std::sync::Mutex<(Option<Tor>, usize)> = std::sync::Mutex::new((None, 0));

/// A hold on the shared Arti. Dropping it releases the hold.
pub struct Lease(());

impl Lease {
    /// Some(why) if the shared Arti has stopped.
    pub fn exited(&mut self) -> Option<String> {
        SHARED.lock().unwrap_or_else(|e| e.into_inner()).0.as_mut().and_then(Tor::exited)
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        let mut s = SHARED.lock().unwrap_or_else(|e| e.into_inner());
        s.1 = s.1.saturating_sub(1);
        if s.1 == 0 {
            if let Some(t) = s.0.take() {
                drop(s);
                t.stop();
            }
        }
    }
}

/// A lease on the shared Arti: it starts Arti (waiting up to `BOOTSTRAP_LIMIT`) unless one runs.
/// The lock is held through a start, so two callers never start two Artis on one port.
pub fn acquire(cancel: &dyn Fn() -> bool) -> Result<Lease> {
    let mut s = SHARED.lock().unwrap_or_else(|e| e.into_inner());
    let alive = s.0.as_mut().is_some_and(|t| t.exited().is_none());
    if !alive {
        if let Some(dead) = s.0.take() {
            dead.stop();
        }
        s.0 = Some(Tor::start(cancel)?);
    }
    s.1 += 1;
    Ok(Lease(()))
}

/// Whether the shared Arti runs now (for the network log and tests).
pub fn running() -> bool {
    SHARED.lock().unwrap_or_else(|e| e.into_inner()).0.is_some()
}

