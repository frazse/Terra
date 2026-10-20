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
                "ble_forget",
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
            ]),
        ),
    )
    .expect("failed to run tauri-build");
}
