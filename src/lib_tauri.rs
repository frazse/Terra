//! Tauri entry point: used on Android and other non-Linux platforms.

use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

const SHIM: &str = include_str!("../shim/bluetooth-shim.js");
const LOADER: &str = include_str!("../shim/addons-loader.js");
const UI: &str = include_str!("../shim/terra-ui.js");

#[cfg_attr(target_os = "android", tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            crate::ble::ble_request_device,
            crate::ble::ble_connect,
            crate::ble::ble_disconnect,
            crate::ble::ble_forget,
            crate::ble::ble_remembered,
            crate::ble::ble_release,
            crate::ble::ble_read,
            crate::ble::ble_write,
            crate::ble::ble_subscribe,
            crate::ble::ble_unsubscribe,
            crate::ble::chooser_candidates,
            crate::ble::chooser_pick,
            crate::ble::chooser_cancel,
            crate::addons::addons_activate,
            crate::addons::addon_storage_get,
            crate::addons::addon_storage_set,
            crate::addons::addon_storage_remove,
            crate::addons::addons_list,
            crate::addons::addons_set_enabled,
            crate::addons::addon_settings_get,
            crate::addons::addon_settings_set,
            crate::addons::addon_http,
            crate::addons::addon_save_file,
        ])
        .setup(|app| {
            let handle = app.handle();

            // Initialize BLE broker (best-effort; non-fatal if unavailable)
            let ble_opt = match tauri::async_runtime::block_on(crate::ble::Broker::new(handle.clone())) {
                Ok(b) => Some(crate::ble::Ble(Some(b))),
                Err(e) => {
                    eprintln!("[terra] BLE unavailable: {e}");
                    None
                }
            };
            handle.manage(ble_opt);

            // Initialize addon store
            let data = crate::ble::get_terra_storage_dir(&handle);
            handle.manage(crate::addons::AddonStore::new(data.join("addon-data")));

            // Log addon directories and loaded addons
            for dir in crate::addons::addon_dirs(&handle) {
                println!("[terra] addon dir: {}", dir.display());
            }
            for a in crate::addons::load_all(&handle) {
                println!("[terra] addon: {} v{} ({}){}", a.name, a.version, a.id, if a.enabled { "" } else { " [disabled]" });
            }

            // RPC mode (Linux desktop via Tauri fallback): spawn the stdio JSON-RPC server
            let is_rpc = std::env::args().any(|a| a == "--rpc");

            if is_rpc {
                let app_handle = handle.clone();
                tauri::async_runtime::spawn(async move {
                    #[cfg(target_os = "linux")]
                    crate::rpc::run_rpc_server(app_handle).await;
                });
            } else {
                // Normal mode: create the webview window
                let init = format!("{SHIM}\n;\n{LOADER}\n;\n{UI}");
                WebviewWindowBuilder::new(
                    app,
                    "main",
                    WebviewUrl::External("https://biketerra.com".parse().unwrap()),
                )
                .title("Terra")
                .inner_size(1400.0, 900.0)
                .initialization_script(&init)
                .build()
                .expect("failed to create webview window");
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Terra");
}
