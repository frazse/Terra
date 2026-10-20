//! Linux entry point: lightweight runtime without Tauri (no webkit2gtk/GTK).

use crate::linux_runtime::AppHandle;

pub fn run() {
    let app = AppHandle::new();

    // Initialize addon store
    let data_dir = crate::ble::get_terra_storage_dir();
    app.manage(crate::addons::AddonStore::new(data_dir.join("addon-data")));

    // Log addon directories and loaded addons
    for dir in crate::addons::addon_dirs(&app) {
        println!("[terra] addon dir: {}", dir.display());
    }
    for a in crate::addons::load_all(&app) {
        println!("[terra] addon: {} v{} ({}){}", a.name, a.version, a.id, if a.enabled { "" } else { " [disabled]" });
    }

    // RPC mode: spawn the stdio JSON-RPC server and block forever.
    // IMPORTANT: Broker::new() must run on the SAME tokio runtime as the RPC server,
    // because spawn_supervisor() spawns tasks on the current runtime.
    let is_rpc = std::env::args().any(|a| a == "--rpc");

    if is_rpc {
        let app_handle = app.clone();
        tokio::runtime::Runtime::new()
            .expect("failed to build tokio runtime")
            .block_on(async {
                // Initialize BLE broker on this runtime (best-effort; non-fatal if unavailable)
                let ble = match crate::ble::Broker::new(None).await {
                    Ok(b) => crate::ble::Ble(Some(b)),
                    Err(e) => {
                        eprintln!("[terra] BLE unavailable: {e}");
                        crate::ble::Ble(None)
                    }
                };
                app_handle.manage(ble);

                // Run RPC server (blocks forever)
                crate::rpc::run_rpc_server(app_handle).await;
            });
    }
    // In non-RPC mode on Linux, the process just exits (Electron handles the UI).
}
