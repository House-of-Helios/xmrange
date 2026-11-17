fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "ping", "self_check_report", "app_info", "get_settings", "save_settings",
            "start_mining", "stop_mining", "pause_mining", "resume_mining",
            "set_threads", "set_worker", "set_wallet", "clear_wallet", "list_trials",
        "clear_trials",
        "start_trials",
        "node_status",
        "save_diagnostics",
        "copy_text",
        "payout_totals",
        "network_log",
        "apply_pick",
            "set_profile", "seek", "stop_seek", "skip_low_power",
            "open_battery_settings", "set_node_folder", "update_status", "install_update",
        ]),
    ))
    .expect("failed to run tauri-build");
}
