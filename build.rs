fn main() {
    // Skip Tauri build steps on Linux (RPC mode only, no webview)
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "linux" {
        return;
    }

    tauri_build::try_build(
        tauri_build::Attributes::new().app_manifest(
            tauri_build::AppManifest::new().commands(&[
                "shim_log",
                "ble_request_device",
                "ble_connect",
                "ble_disconnect",
                "ble_disconnect_all",
                "ble_forget",
                "ble_devices_status",
                "ble_drop_device",
                "ble_drop_all",
                "ble_remembered",
                "ble_release",
                "ble_read",
                "ble_write",
                "ble_subscribe",
                "ble_unsubscribe",
                "chooser_candidates",
                "chooser_pick",
                "chooser_cancel",
                "addons_activate",
                "addon_storage_get",
                "addon_storage_set",
                "addon_storage_remove",
                "addons_list",
                "addons_set_enabled",
                "addons_catalog",
                "addons_install_github",
                "addons_install_zip",
                "addons_uninstall",
                "addon_settings_get",
                "addon_settings_set",
                "addon_settings_reset",
                "addon_settings_export",
                "addon_settings_import",
                "addon_http",
                "addon_save_file",
                "addon_udp_send",
                "addon_tcp_connect",
                "addon_tcp_send",
                "addon_tcp_list",
                "addon_tcp_close",
                "ride_push",
                "ride_latest",
                "ride_history",
                "discord_set_activity",
                "discord_clear_activity",
                "discord_status",
                "perf_stats",
                "terra_pick_data_dir",
                "terra_data_dir_status",
                "terra_dismiss_data_dir_prompt",
                "terra_reset_all_data",
            ]),
        ),
    )
    .expect("failed to run tauri-build");
}
