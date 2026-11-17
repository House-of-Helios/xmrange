use std::path::PathBuf;

/// Where XMRange keeps its data. `XMRANGE_DATA_DIR` always wins.
/// Dev builds default inside the repo (`_devdata/`); release builds use Application Support.
pub fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("XMRANGE_DATA_DIR") {
        return PathBuf::from(d);
    }
    default_data_dir()
}

/// The Ledger file (spec §5): `ledger-dev.sqlite` in dev builds, `ledger.sqlite` in release.
#[cfg(feature = "dev-diagnostics")]
pub fn ledger_path() -> PathBuf {
    data_dir().join("ledger-dev.sqlite")
}
#[cfg(not(feature = "dev-diagnostics"))]
pub fn ledger_path() -> PathBuf {
    data_dir().join("ledger.sqlite")
}

/// The Ledger's key, a 0600 file (owner decision 2026-09-28: no OS credential store, ever, so
/// opening the Ledger never prompts). `dev.key` in dev builds, `app.key` in release; same code
/// path either way, only the data dir differs.
pub fn key_path() -> PathBuf {
    data_dir().join(key_file_name())
}

pub(crate) fn key_file_name() -> &'static str {
    if cfg!(feature = "dev-diagnostics") {
        "dev.key"
    } else {
        "app.key"
    }
}

/// The patched xmrig binary. `XMRANGE_XMRIG` always wins.
pub fn xmrig_binary() -> PathBuf {
    if let Some(p) = std::env::var_os("XMRANGE_XMRIG") {
        return PathBuf::from(p);
    }
    default_xmrig()
}

/// p2pool (tools/build-p2pool.sh). `XMRANGE_P2POOL` always wins.
pub fn p2pool_binary() -> PathBuf {
    std::env::var_os("XMRANGE_P2POOL").map(PathBuf::from).unwrap_or_else(|| bundled("p2pool"))
}

/// Arti, the Tor client (tools/build-arti.sh). `XMRANGE_ARTI` always wins.
pub fn arti_binary() -> PathBuf {
    std::env::var_os("XMRANGE_ARTI").map(PathBuf::from).unwrap_or_else(|| bundled("arti"))
}

/// house-report, the problem-report sender (tools/build-house-report.sh). `XMRANGE_HOUSE_REPORT` always wins.
pub fn house_report_binary() -> PathBuf {
    std::env::var_os("XMRANGE_HOUSE_REPORT").map(PathBuf::from).unwrap_or_else(|| bundled("house-report"))
}

/// monerod, XMRange's own Monero node (tools/build-monerod.sh). `XMRANGE_MONEROD` always wins.
pub fn monerod_binary() -> PathBuf {
    std::env::var_os("XMRANGE_MONEROD").map(PathBuf::from).unwrap_or_else(|| bundled("monerod"))
}

/// A helper binary: the vendor build in dev (these paths exist in dev builds only), beside the
/// app's own executable in release.
#[cfg(feature = "dev-diagnostics")]
fn bundled(name: &str) -> PathBuf {
    workspace_root().join(match name {
        "p2pool" => "vendor/p2pool/build/p2pool",
        "monerod" => "vendor/monero/build-out/arm64/monerod",
        "house-report" => "vendor/universal/house-report",
        _ => "vendor/arti/target/release/arti",
    })
}

#[cfg(not(feature = "dev-diagnostics"))]
fn bundled(name: &str) -> PathBuf {
    std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join(name))).unwrap_or_else(|| PathBuf::from(name))
}

#[cfg(feature = "dev-diagnostics")]
pub fn workspace_root() -> PathBuf {
    let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    here.parent()
        .and_then(|p| p.parent())
        .expect("crate lives at <root>/crates/<name>")
        .to_path_buf()
}

/// A fresh test directory inside the repo (Rule #1: never system temp).
#[cfg(feature = "dev-diagnostics")]
pub fn scratch_dir(tag: &str) -> std::io::Result<PathBuf> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let n = N.fetch_add(1, Ordering::SeqCst);
    let d = workspace_root()
        .join("_devdata/test-tmp")
        .join(format!("{tag}-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&d)?;
    Ok(d)
}

#[cfg(feature = "dev-diagnostics")]
fn default_data_dir() -> PathBuf {
    workspace_root().join("_devdata")
}

#[cfg(not(feature = "dev-diagnostics"))]
fn default_data_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"));
    home.join("Library/Application Support/XMRange")
}

#[cfg(feature = "dev-diagnostics")]
fn default_xmrig() -> PathBuf {
    workspace_root().join("vendor/xmrig-mo/build/xmrig")
}

#[cfg(not(feature = "dev-diagnostics"))]
fn default_xmrig() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join("xmrig")))
        .unwrap_or_else(|| PathBuf::from("xmrig"))
}
