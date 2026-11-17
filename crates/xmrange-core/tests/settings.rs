#![cfg(feature = "dev-diagnostics")]
use xmrange_core::settings::{Measure, Settings, Theme};
use xmrange_core::paths::scratch_dir;
use std::fs;

#[test]
fn defaults_when_missing() {
    let dir = scratch_dir("settings-missing").unwrap();
    let loaded = Settings::load(&dir);
    assert_eq!(loaded, Settings::default());
}

#[test]
fn roundtrips() {
    let dir = scratch_dir("settings-roundtrip").unwrap();
    let mut settings = Settings::default();
    settings.rate_per_kwh = 0.21;
    settings.theme = Theme::Dark;
    settings.measure = Measure::Cpu;
    settings.threads = 6;

    settings.save(&dir).unwrap();
    let loaded = Settings::load(&dir);

    assert_eq!(loaded, settings);
}

#[test]
fn corrupt_file_falls_back_to_default() {
    let dir = scratch_dir("settings-corrupt").unwrap();
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("settings.json"), "{not json").unwrap();

    let loaded = Settings::load(&dir);
    assert_eq!(loaded, Settings::default());
}

#[test]
fn partial_file_fills_defaults() {
    let dir = scratch_dir("settings-partial").unwrap();
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("settings.json"), r#"{"rate_per_kwh":0.3}"#).unwrap();

    let loaded = Settings::load(&dir);
    assert_eq!(loaded.rate_per_kwh, 0.3);
    assert_eq!(loaded.currency, Settings::default().currency);
    assert_eq!(loaded.theme, Settings::default().theme);
    assert_eq!(loaded.measure, Settings::default().measure);
    assert_eq!(loaded.period, Settings::default().period);
    assert_eq!(loaded.worker, Settings::default().worker);
    assert_eq!(loaded.threads, Settings::default().threads);
    assert_eq!(loaded.update_folder, Settings::default().update_folder);
}

#[test]
fn save_is_atomic_rename() {
    let dir = scratch_dir("settings-atomic").unwrap();
    let settings = Settings::default();
    settings.save(&dir).unwrap();

    let tmp_path = dir.join(".settings.json.tmp");
    assert!(!tmp_path.exists(), "temporary file should not remain after save");

    let final_path = dir.join("settings.json");
    assert!(final_path.exists(), "settings.json should exist");
}
