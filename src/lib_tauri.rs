//! Tauri entry point: used on Android and other non-Linux platforms.

use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

const SHIM: &str = include_str!("../shim/bluetooth-shim.js");
const LOADER: &str = include_str!("../shim/addons-loader.js");
const UI: &str = include_str!("../shim/terra-ui.js");

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            ble::ble_request_device,
            ble::ble_connect,
            ble::ble_disconnect,
            ble::ble_forget,
            ble::ble_remembered,
            ble::ble_release,
            ble::ble_read,
            ble::ble_write,
            ble::ble_subscribe,
            ble::ble_unsubscribe,
            ble::chooser_candidates,
            ble::chooser_pick,
            ble::chooser_cancel,
            addons::addons_activate,
            addons::addon_storage_get,
            addons::addon_storage_set,
            addons::addon_storage_remove,
            addons::addons_list,
            addons::addons_set_enabled,
            addons::addon_settings_get,
            addons::addon_settings_set,
            addons::addon_http,
            addons::addon_save_file,
        ])
        .setup(|app| {
            let handle = app.handle();

            // Initialize BLE broker (best-effort; non-fatal if unavailable)
            let ble_opt = match tauri::async_runtime::block_on(ble::Broker::new(handle.clone())) {
                Ok(b) => Some(ble::Ble(b)),
                Err(e) => {
                    eprintln!("[terra] BLE unavailable: {e}");
                    None
                }
            };
            handle.manage(ble_opt);

            // Initialize addon store
            let data = ble::get_terra_storage_dir(&handle);
            handle.manage(addons::AddonStore::new(data.join("addon-data")));

            // Log addon directories and loaded addons
            for dir in addons::addon_dirs(&handle) {
                println!("[terra] addon dir: {}", dir.display());
            }
            for a in addons::load_all(&handle) {
                println!("[terra] addon: {} v{} ({}){}", a.name, a.version, a.id, if a.enabled { "" } else { " [disabled]" });
            }

            // RPC mode (Linux desktop via Tauri fallback): spawn the stdio JSON-RPC server
            let is_rpc = std::env::args().any(|a| a == "--rpc");

            if is_rpc {
                let app_handle = handle.clone();
                tauri::async_runtime::spawn(async move {
                    #[cfg(target_os = "linux")]
                    rpc::run_rpc_server(app_handle).await;
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
