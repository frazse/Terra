//! Android: let the user choose a visible folder for Terra's data
//! (addons/, addon-data/, devices.json) via Storage Access Framework.
//!
//! Flow:
//!   JS → `terra_pick_data_dir` (Rust) → JNI → MainActivity.startDataDirPicker()
//!   User picks folder → Kotlin processes (SAF grant, URI→path, probe, migrate, write pointer)
//!   Kotlin sets result → Rust polls `getDataDirPickResult()` → returns to JS.
//!
//! The pointer file lives in internal storage: `<app_data_dir>/terra-data-dir.txt`
//! (one line: the absolute path the user chose).

use std::path::PathBuf;

#[cfg(not(target_os = "linux"))]
use tauri::AppHandle;
#[cfg(target_os = "linux")]
use crate::linux_runtime::AppHandle;

// ─── JNI helpers (Android only) ──────────────────────────────────────────────

#[cfg(target_os = "android")]
mod jni {
    use crate::ble::{alog, find_app_class, get_java_vm};

    /// Call a no-arg @JvmStatic method on MainActivity that returns a String.
    pub fn call_main_activity_str(method: &str) -> Result<String, String> {
        let vm = get_java_vm()?;
        let mut env = vm
            .attach_current_thread()
            .map_err(|e| format!("attach: {e}"))?;
        let cls = find_app_class(&mut env, "dev.frazse.terra.MainActivity")
            .map_err(|e| format!("find_class: {e}"))?;
        let result = env
            .call_static_method(&cls, method, "()Ljava/lang/String;", &[])
            .map_err(|e| {
                if env.exception_check().unwrap_or(false) {
                    let _ = env.exception_clear();
                }
                let msg = format!("call {method}: {e}");
                alog(&msg);
                msg
            })?;
        let obj = result
            .l()
            .map_err(|e| format!("result {method}: {e}"))?;
        if obj.is_null() {
            return Err(format!("{method} returned null"));
        }
        let jstr = jni::objects::JString::from(obj);
        let rust_str: String = env.get_string(&jstr).map_err(|e| e.to_string())?.into();
        Ok(rust_str)
    }

    /// Call a no-arg @JvmStatic void method on MainActivity.
    pub fn call_main_activity_void(method: &str) -> Result<(), String> {
        let vm = get_java_vm()?;
        let mut env = vm
            .attach_current_thread()
            .map_err(|e| format!("attach: {e}"))?;
        let cls = find_app_class(&mut env, "dev.frazse.terra.MainActivity")
            .map_err(|e| format!("find_class: {e}"))?;
        env.call_static_method(&cls, method, "()V", &[])
            .map_err(|e| {
                if env.exception_check().unwrap_or(false) {
                    let _ = env.exception_clear();
                }
                format!("call {method}: {e}")
            })?;
        Ok(())
    }

    /// Call a no-arg @JvmStatic boolean method on MainActivity.
    pub fn call_main_activity_bool(method: &str) -> Result<bool, String> {
        let vm = get_java_vm()?;
        let mut env = vm
            .attach_current_thread()
            .map_err(|e| format!("attach: {e}"))?;
        let cls = find_app_class(&mut env, "dev.frazse.terra.MainActivity")
            .map_err(|e| format!("find_class: {e}"))?;
        let result = env
            .call_static_method(&cls, method, "()Z", &[])
            .map_err(|e| {
                if env.exception_check().unwrap_or(false) {
                    let _ = env.exception_clear();
                }
                format!("call {method}: {e}")
            })?;
        result.z().map_err(|e| format!("result {method}: {e}"))
    }
}

#[cfg(target_os = "android")]
pub(crate) fn call_main_activity_str_pub(method: &str) -> Result<String, String> {
    jni::call_main_activity_str(method)
}

// ─── Tauri / RPC commands ────────────────────────────────────────────────────

/// Open the Android folder picker. Blocks (async) until the user picks, cancels,
/// or a 10-minute timeout elapses.
#[cfg_attr(not(target_os = "linux"), tauri::command)]
pub async fn terra_pick_data_dir() -> Result<serde_json::Value, String> {
    #[cfg(target_os = "android")]
    {
        use std::time::{Duration, Instant};

        jni::call_main_activity_void("resetDataDirPickResult")
            .map_err(|e| format!("failed to start picker: {e}"))?;
        jni::call_main_activity_void("startDataDirPicker")
            .map_err(|e| format!("failed to start picker: {e}"))?;

        let deadline = Instant::now() + Duration::from_secs(600); // 10 min
        loop {
            tokio::time::sleep(Duration::from_millis(250)).await;
            if Instant::now() > deadline {
                return Ok(serde_json::json!({
                    "status": "timeout",
                    "message": "Picker timed out (10 min). No folder was selected."
                }));
            }
            let result = match jni::call_main_activity_str("getDataDirPickResult") {
                Ok(r) => r,
                Err(_) => continue, // JNI hiccup; keep polling
            };
            if result == "pending" {
                continue;
            }
            if let Some(path) = result.strip_prefix("success:") {
                return Ok(serde_json::json!({
                    "status": "success",
                    "path": path
                }));
            }
            if result == "cancelled" {
                return Ok(serde_json::json!({
                    "status": "cancelled",
                    "message": "Picker cancelled. No changes made."
                }));
            }
            if let Some(msg) = result.strip_prefix("error:") {
                return Ok(serde_json::json!({
                    "status": "error",
                    "message": msg
                }));
            }
            // Unknown state — keep polling
        }
    }

    #[cfg(not(target_os = "android"))]
    {
        Ok(serde_json::json!({
            "status": "error",
            "message": "Data folder selection is only available on Android."
        }))
    }
}

/// Return the current data directory info.
#[cfg_attr(not(target_os = "linux"), tauri::command)]
pub fn terra_data_dir_status(app: AppHandle) -> Result<serde_json::Value, String> {
    #[cfg(target_os = "android")]
    {
        let data_dir = crate::ble::get_terra_storage_dir(&app);
        let default = PathBuf::from("/storage/emulated/0/Android/data/dev.frazse.terra/files");
        let files_dir = crate::ble::android_internal_files_dir(&app);
        let prompted = files_dir.as_ref().map_or(false, |d| {
            d.join("terra-data-dir.txt").exists() || d.join("terra-datadir-prompted.txt").exists()
        });
        let has_storage_permission =
            jni::call_main_activity_bool("hasAllFilesAccess").unwrap_or(true);
        Ok(serde_json::json!({
            "path": data_dir.to_string_lossy().to_string(),
            "isDefault": data_dir == default,
            "prompted": prompted,
            "hasStoragePermission": has_storage_permission,
            "addonsCount": std::fs::read_dir(data_dir.join("addons")).map(|d| d.count()).unwrap_or(0)
        }))
    }

    #[cfg(target_os = "linux")]
    {
        let _ = app;
        Ok(serde_json::json!({
            "path": crate::ble::get_terra_storage_dir().to_string_lossy().to_string(),
            "isDefault": true,
            "prompted": true,
            "hasStoragePermission": true
        }))
    }

    #[cfg(all(not(target_os = "android"), not(target_os = "linux")))]
    {
        let dir = crate::ble::get_terra_storage_dir(&app);
        Ok(serde_json::json!({
            "path": dir.to_string_lossy().to_string(),
            "isDefault": true,
            "prompted": true,
            "hasStoragePermission": true
        }))
    }
}

/// Mark the Android first-launch data-folder prompt as dismissed so it won't reappear on launch.
#[cfg_attr(not(target_os = "linux"), tauri::command)]
pub fn terra_dismiss_data_dir_prompt(app: AppHandle) -> Result<serde_json::Value, String> {
    #[cfg(target_os = "android")]
    {
        if let Some(files_dir) = crate::ble::android_internal_files_dir(&app) {
            let _ = std::fs::create_dir_all(&files_dir);
            std::fs::write(files_dir.join("terra-datadir-prompted.txt"), "1")
                .map_err(|e| format!("failed to persist prompt dismissal: {e}"))?;
        }
        Ok(serde_json::json!({ "status": "ok" }))
    }

    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        Ok(serde_json::json!({ "status": "ok" }))
    }
}

/// Wipe all Terra data (addons/, addon-data/, devices.json) from the current data dir.
/// The caller (UI) MUST confirm with the user before invoking this.
#[cfg_attr(not(target_os = "linux"), tauri::command)]
pub fn terra_reset_all_data(app: AppHandle) -> Result<serde_json::Value, String> {
    #[cfg(target_os = "android")]
    {
        do_reset(crate::ble::get_terra_storage_dir(&app))
    }

    #[cfg(target_os = "linux")]
    {
        let _ = app;
        do_reset(crate::ble::get_terra_storage_dir())
    }

    #[cfg(all(not(target_os = "android"), not(target_os = "linux")))]
    {
        do_reset(crate::ble::get_terra_storage_dir(&app))
    }
}

fn do_reset(data_dir: PathBuf) -> Result<serde_json::Value, String> {
    let mut removed: Vec<&str> = Vec::new();

    for item in ["addons", "addon-data"] {
        let p = data_dir.join(item);
        if p.exists() {
            std::fs::remove_dir_all(&p).map_err(|e| format!("failed to remove {item}: {e}"))?;
            removed.push(item);
        }
    }

    let devices = data_dir.join("devices.json");
    if devices.exists() {
        std::fs::remove_file(&devices).map_err(|e| format!("failed to remove devices.json: {e}"))?;
        removed.push("devices.json");
    }

    Ok(serde_json::json!({
        "status": "ok",
        "removed": removed,
        "path": data_dir.to_string_lossy().to_string()
    }))
}
