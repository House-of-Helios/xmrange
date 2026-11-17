#![cfg(feature = "dev-diagnostics")]
use std::os::unix::fs::PermissionsExt;
use xmrange_core::{handoff::Handoff, paths::scratch_dir, token::new_token};

fn mode(p: &std::path::Path) -> u32 {
    std::fs::metadata(p).unwrap().permissions().mode() & 0o777
}

#[test]
fn token_is_32_hex_and_unique() {
    let a = new_token();
    assert_eq!(a.len(), 32);
    assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    assert_ne!(a, new_token());
}

#[test]
fn handoff_is_private_and_self_cleaning() {
    let base = scratch_dir("handoff").unwrap();
    let dir;
    {
        let h = Handoff::create(&base).unwrap();
        h.write(&serde_json::json!({"pools": [{"user": "4secret"}]})).unwrap();
        assert_eq!(mode(h.dir()), 0o700);
        assert_eq!(mode(h.config_path()), 0o600);
        assert!(h.write(&serde_json::json!({})).is_err(), "must never overwrite an existing handoff");
        h.remove_file().unwrap();
        h.remove_file().unwrap(); // idempotent
        assert!(!h.config_path().exists());
        dir = h.dir().to_path_buf();
    }
    assert!(!dir.exists(), "drop removes the private dir");
}

#[test]
fn handoff_drop_removes_file_on_error_paths() {
    let base = scratch_dir("handoff-drop").unwrap();
    let path;
    {
        let h = Handoff::create(&base).unwrap();
        h.write(&serde_json::json!({"wallet": "x"})).unwrap();
        path = h.config_path().to_path_buf();
    }
    assert!(!path.exists());
}

#[test]
fn a_saved_wallet_reaches_only_the_private_handoff() {
    use xmrange_core::config::{MinerConfig, DEFAULT_WALLET};
    use xmrange_core::settings::{Settings, Wallet};
    let base = scratch_dir("handoff-wallet").unwrap();
    let mut settings = Settings::default();
    settings.wallet = Wallet(Some(DEFAULT_WALLET.to_string()));
    settings.save(&base).unwrap();
    let wallet = Settings::load(&base).wallet.0.unwrap();
    let cfg = MinerConfig::new(&wallet, "w", 2).unwrap();
    let json = cfg.to_xmrig_json(3620, "tok", None);
    assert_eq!(json["pools"][0]["user"], DEFAULT_WALLET);
    assert!(!format!("{cfg:?}").contains(DEFAULT_WALLET), "Debug stays masked");
    let h = Handoff::create(&base.join("run")).unwrap();
    h.write(&json).unwrap();
    assert_eq!(mode(h.config_path()), 0o600);
    h.remove_file().unwrap();
    assert!(!h.config_path().exists());
}
