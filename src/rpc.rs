use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use crate::linux_runtime::{AppHandle, State};

#[derive(Deserialize, Debug)]
pub struct RpcRequest {
    pub id: Value,
    pub cmd: String,
    pub args: Option<Value>,
}

#[derive(Serialize, Debug)]
pub struct RpcResponse {
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub async fn run_rpc_server(app_handle: AppHandle) {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(100);

    std::thread::spawn(move || {
        use std::io::BufRead;
        let stdin = std::io::stdin();
        let mut reader = stdin.lock();
        let mut buffer = String::new();

        while let Ok(bytes) = reader.read_line(&mut buffer) {
            if bytes == 0 {
                std::thread::sleep(std::time::Duration::from_millis(100));
                buffer.clear();
                continue;
            }
            let line = buffer.trim().to_string();
            if !line.is_empty() {
                let _ = tx.blocking_send(line);
            }
            buffer.clear();
        }
    });

    println!("[rpc] Terra Rust RPC server started on stdio");

    while let Some(line) = rx.recv().await {
        if let Ok(req) = serde_json::from_str::<RpcRequest>(&line) {
            let req_id = req.id.clone();
            let cmd = req.cmd.clone();
            let app = app_handle.clone();

            tokio::spawn(async move {
                let result = dispatch_rpc_command(&app, &cmd, req.args).await;
                let resp = match result {
                    Ok(val) => RpcResponse { id: req_id, result: Some(val), error: None },
                    Err(err_msg) => RpcResponse { id: req_id, result: None, error: Some(err_msg) },
                };
                if let Ok(json_str) = serde_json::to_string(&resp) {
                    use std::io::Write;
                    let mut out = std::io::stdout().lock();
                    let _ = writeln!(out, "{json_str}");
                    let _ = out.flush();
                }
            });
        }
    }
}

async fn dispatch_rpc_command(
    app: &AppHandle,
    cmd: &str,
    args: Option<Value>,
) -> Result<Value, String> {
    let args_val = args.unwrap_or_else(|| json!({}));

    match cmd {
        "shim_log" => Ok(json!(null)),

        "addons_list" => {
            let list = crate::addons::addons_list(app.clone()).await;
            serde_json::to_value(list).map_err(|e| e.to_string())
        }

        "addons_activate" => {
            let active: Vec<String> = serde_json::from_value(args_val.get("active").cloned().unwrap_or(json!([]))).unwrap_or_default();
            let url = args_val.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let res = crate::addons::addons_activate(app.clone(), url, active).await;
            serde_json::to_value(res).map_err(|e| e.to_string())
        }

        "addons_set_enabled" => {
            let store: State<'_, crate::addons::AddonStore> = app.state();
            let id = args_val.get("id").and_then(|v| v.as_str()).ok_or("missing id")?.to_string();
            let enabled = args_val.get("enabled").and_then(|v| v.as_bool()).ok_or("missing enabled")?;
            crate::addons::addons_set_enabled(store, id, enabled)?;
            Ok(json!(null))
        }

        "addon_storage_get" => {
            let store: State<'_, crate::addons::AddonStore> = app.state();
            let id = args_val.get("id").and_then(|v| v.as_str()).ok_or("missing id")?.to_string();
            let key = args_val.get("key").and_then(|v| v.as_str()).ok_or("missing key")?.to_string();
            let res = crate::addons::addon_storage_get(store, id, key)?;
            serde_json::to_value(res).map_err(|e| e.to_string())
        }

        "addon_storage_set" => {
            let store: State<'_, crate::addons::AddonStore> = app.state();
            let id = args_val.get("id").and_then(|v| v.as_str()).ok_or("missing id")?.to_string();
            let key = args_val.get("key").and_then(|v| v.as_str()).ok_or("missing key")?.to_string();
            let val = args_val.get("value").cloned().unwrap_or(json!(null));
            crate::addons::addon_storage_set(store, id, key, val)?;
            Ok(json!(null))
        }

        "addon_storage_remove" => {
            let store: State<'_, crate::addons::AddonStore> = app.state();
            let id = args_val.get("id").and_then(|v| v.as_str()).ok_or("missing id")?.to_string();
            let key = args_val.get("key").and_then(|v| v.as_str()).ok_or("missing key")?.to_string();
            crate::addons::addon_storage_remove(store, id, key)?;
            Ok(json!(null))
        }

        "addon_settings_get" => {
            let store: State<'_, crate::addons::AddonStore> = app.state();
            let id = args_val.get("id").and_then(|v| v.as_str()).ok_or("missing id")?.to_string();
            let res = crate::addons::addon_settings_get(app.clone(), store, id).await?;
            serde_json::to_value(res).map_err(|e| e.to_string())
        }

        "addon_settings_set" => {
            let store: State<'_, crate::addons::AddonStore> = app.state();
            let id = args_val.get("id").and_then(|v| v.as_str()).ok_or("missing id")?.to_string();
            let key = args_val.get("key").and_then(|v| v.as_str()).ok_or("missing key")?.to_string();
            let val = args_val.get("value").cloned().unwrap_or(json!(null));
            crate::addons::addon_settings_set(app.clone(), store, id, key, val).await?;
            Ok(json!(null))
        }

        "addon_http" => {
            let store: State<'_, crate::addons::AddonStore> = app.state();
            let addon = args_val.get("addon").and_then(|v| v.as_str()).ok_or("missing addon")?.to_string();
            let req_val = args_val.get("req").cloned().ok_or("missing req")?;
            let req = serde_json::from_value(req_val).map_err(|e| e.to_string())?;
            let res = crate::addons::addon_http(app.clone(), store, addon, req).await?;
            serde_json::to_value(res).map_err(|e| e.to_string())
        }

        "addon_save_file" => {
            let addon = args_val.get("addon").and_then(|v| v.as_str()).ok_or("missing addon")?.to_string();
            let filename = args_val.get("filename").and_then(|v| v.as_str()).ok_or("missing filename")?.to_string();
            let base64 = args_val.get("base64").and_then(|v| v.as_str()).ok_or("missing base64")?.to_string();
            let path = crate::addons::addon_save_file(app.clone(), addon, filename, base64).await?;
            Ok(json!(path))
        }

        "addon_udp_send" => {
            let store: State<'_, crate::addons::AddonStore> = app.state();
            let addon = args_val.get("addon").and_then(|v| v.as_str()).ok_or("missing addon")?.to_string();
            let target = args_val.get("target").and_then(|v| v.as_str()).ok_or("missing target")?.to_string();
            let payload = args_val.get("payload").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let sent = crate::addons::addon_udp_send(app.clone(), store, addon, target, payload).await?;
            Ok(json!(sent))
        }

        "ble_request_device" => {
            let ble_state: State<'_, crate::ble::Ble> = app.state();
            let broker = crate::ble::broker(&ble_state)?;
            let filters = serde_json::from_value(args_val.get("filters").cloned().unwrap_or(json!([]))).unwrap_or_default();
            let exclusions = serde_json::from_value(args_val.get("exclusionFilters").cloned().unwrap_or(json!([]))).unwrap_or_default();
            let accept_all = args_val.get("acceptAll").and_then(|v| v.as_bool()).unwrap_or(false);
            let silent = args_val.get("silent").and_then(|v| v.as_bool()).unwrap_or(false);
            let owner = args_val.get("owner").and_then(|v| v.as_str()).map(|s| s.to_string());
            let dev = broker.request_device(filters, exclusions, accept_all, silent, owner).await?;
            serde_json::to_value(dev).map_err(|e| e.to_string())
        }

        "ble_connect" => {
            let ble_state: State<'_, crate::ble::Ble> = app.state();
            let broker = crate::ble::broker(&ble_state)?;
            let id = args_val.get("id").and_then(|v| v.as_str()).ok_or("missing id")?;
            let svcs = broker.connect(id).await?;
            serde_json::to_value(svcs).map_err(|e| e.to_string())
        }

        "ble_disconnect" => {
            let ble_state: State<'_, crate::ble::Ble> = app.state();
            let broker = crate::ble::broker(&ble_state)?;
            let id = args_val.get("id").and_then(|v| v.as_str()).ok_or("missing id")?;
            broker.disconnect(id).await?;
            Ok(json!(null))
        }

        "ble_forget" => {
            let ble_state: State<'_, crate::ble::Ble> = app.state();
            let broker = crate::ble::broker(&ble_state)?;
            let id = args_val.get("id").and_then(|v| v.as_str()).ok_or("missing id")?;
            broker.forget(id).await?;
            Ok(json!(null))
        }

        "ble_remembered" => {
            let ble_state: State<'_, crate::ble::Ble> = app.state();
            let broker = crate::ble::broker(&ble_state)?;
            let owner = args_val.get("owner").and_then(|v| v.as_str()).map(|s| s.to_string());
            let known = broker.known.lock().await;
            let list: Vec<_> = known
                .iter()
                .filter(|k| owner.as_ref().map_or(false, |o| k.owner.as_deref() == Some(o.as_str())))
                .map(|k| crate::ble::DeviceInfo { id: k.id.clone(), name: k.name.clone() })
                .collect();
            serde_json::to_value(list).map_err(|e| e.to_string())
        }

        "ble_release" => {
            let ble_state: State<'_, crate::ble::Ble> = app.state();
            let broker = crate::ble::broker(&ble_state)?;
            let owner = args_val.get("owner").and_then(|v| v.as_str()).ok_or("missing owner")?.to_string();
            let released = args_val.get("released").and_then(|v| v.as_bool()).unwrap_or(false);
            let mut r = broker.released.lock().await;
            if released {
                r.insert(owner);
            } else {
                r.remove(&owner);
            }
            Ok(json!(null))
        }

        "ble_read" => {
            let ble_state: State<'_, crate::ble::Ble> = app.state();
            let broker = crate::ble::broker(&ble_state)?;
            let id = args_val.get("id").and_then(|v| v.as_str()).ok_or("missing id")?;
            let s = args_val.get("service").or_else(|| args_val.get("s")).and_then(|v| v.as_str()).ok_or("missing service")?;
            let c = args_val.get("characteristic").or_else(|| args_val.get("c")).and_then(|v| v.as_str()).ok_or("missing characteristic")?;
            let val = broker.read(id, s, c).await?;
            Ok(json!(val))
        }

        "ble_write" => {
            let ble_state: State<'_, crate::ble::Ble> = app.state();
            let broker = crate::ble::broker(&ble_state)?;
            let id = args_val.get("id").and_then(|v| v.as_str()).ok_or("missing id")?;
            let s = args_val.get("service").or_else(|| args_val.get("s")).and_then(|v| v.as_str()).ok_or("missing service")?;
            let c = args_val.get("characteristic").or_else(|| args_val.get("c")).and_then(|v| v.as_str()).ok_or("missing characteristic")?;
            let val = args_val.get("value").and_then(|v| v.as_str()).ok_or("missing value")?;
            let with_resp = args_val.get("withResponse").and_then(|v| v.as_bool()).unwrap_or(true);
            broker.write(id, s, c, val, with_resp).await?;
            Ok(json!(null))
        }

        "ble_subscribe" => {
            let ble_state: State<'_, crate::ble::Ble> = app.state();
            let broker = crate::ble::broker(&ble_state)?;
            let id = args_val.get("id").and_then(|v| v.as_str()).ok_or("missing id")?;
            let s = args_val.get("service").or_else(|| args_val.get("s")).and_then(|v| v.as_str()).ok_or("missing service")?;
            let c = args_val.get("characteristic").or_else(|| args_val.get("c")).and_then(|v| v.as_str()).ok_or("missing characteristic")?;
            broker.subscribe(id, s, c, true).await?;
            Ok(json!(null))
        }

        "ble_unsubscribe" => {
            let ble_state: State<'_, crate::ble::Ble> = app.state();
            let broker = crate::ble::broker(&ble_state)?;
            let id = args_val.get("id").and_then(|v| v.as_str()).ok_or("missing id")?;
            let s = args_val.get("service").or_else(|| args_val.get("s")).and_then(|v| v.as_str()).ok_or("missing service")?;
            let c = args_val.get("characteristic").or_else(|| args_val.get("c")).and_then(|v| v.as_str()).ok_or("missing characteristic")?;
            broker.subscribe(id, s, c, false).await?;
            Ok(json!(null))
        }

        "chooser_candidates" => {
            let ble_state: State<'_, crate::ble::Ble> = app.state();
            let broker = crate::ble::broker(&ble_state)?;
            let candidates = broker.candidates().await;
            serde_json::to_value(candidates).map_err(|e| e.to_string())
        }

        "chooser_pick" => {
            let ble_state: State<'_, crate::ble::Ble> = app.state();
            let broker = crate::ble::broker(&ble_state)?;
            let id = args_val.get("id").and_then(|v| v.as_str()).ok_or("missing id")?.to_string();
            broker.resolve(Some(id)).await;
            Ok(json!(null))
        }

        "chooser_cancel" => {
            let ble_state: State<'_, crate::ble::Ble> = app.state();
            let broker = crate::ble::broker(&ble_state)?;
            broker.resolve(None).await;
            Ok(json!(null))
        }

        "ride_push" => {
            let store: State<'_, crate::ride::Ride> = app.state();
            let data = args_val.get("data").cloned().unwrap_or(json!(null));
            crate::ride::ride_push(store, data);
            Ok(json!(null))
        }

        "ride_latest" => {
            let store: State<'_, crate::ride::Ride> = app.state();
            let res = crate::ride::ride_latest(store);
            serde_json::to_value(res).map_err(|e| e.to_string())
        }

        "ride_history" => {
            let store: State<'_, crate::ride::Ride> = app.state();
            let limit = args_val.get("limit").and_then(|v| v.as_u64()).map(|n| n as usize);
            let res = crate::ride::ride_history(store, limit);
            serde_json::to_value(res).map_err(|e| e.to_string())
        }

        _ => Err(format!("Unknown RPC command: {cmd}")),
    }
}
