mod addons;
mod ble;
#[cfg(all(not(target_os = "android"), target_os = "linux"))]
mod rpc;

use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

const SHIM: &str = include_str!("../shim/bluetooth-shim.js");
const LOADER: &str = include_str!("../shim/addons-loader.js");
const UI: &str = include_str!("../shim/terra-ui.js");

#[tauri::command]
fn shim_log(kind: String, detail: serde_json::Value) {
    println!("[shim] {kind}: {detail}");
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            shim_log,
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
            let broker = match tauri::async_runtime::block_on(ble::Broker::new(app.handle().clone())) {
                Ok(b) => Some(b),
                Err(e) => {
                    eprintln!("[terra] BLE unavailable: {e}");
                    None
                }
            };
            app.manage(ble::Ble(broker));

            let data = ble::get_terra_storage_dir(app.handle());
            app.manage(addons::AddonStore::new(data.join("addon-data")));

            for dir in addons::addon_dirs(app.handle()) {
                println!("[terra] addon dir: {}", dir.display());
            }
            for a in addons::load_all(app.handle()) {
                println!("[terra] addon: {} v{} ({}){}", a.name, a.version, a.id, if a.enabled { "" } else { " [disabled]" });
            }

            let is_rpc = std::env::args().any(|a| a == "--rpc");

            if is_rpc {
                #[cfg(all(not(target_os = "android"), target_os = "linux"))]
                {
                    let app_handle = app.handle().clone();
                    tauri::async_runtime::spawn(async move {
                        rpc::run_rpc_server(app_handle).await;
                    });
                }
            } else {
                let init = format!("{SHIM}\n;\n{LOADER}\n;\n{UI}");
                WebviewWindowBuilder::new(
                    app,
                    "main",
                    WebviewUrl::External("https://biketerra.com".parse().unwrap()),
                )
                .title("Terra")
                .inner_size(1400.0, 900.0)
                .initialization_script(&init)
                .build()?;
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Terra");
}
