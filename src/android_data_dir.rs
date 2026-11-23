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

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
use tauri::AppHandle;
#[cfg(any(target_os = "linux", target_os = "windows"))]
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
#[cfg_attr(not(any(target_os = "linux", target_os = "windows")), tauri::command)]
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
#[cfg_attr(not(any(target_os = "linux", target_os = "windows")), tauri::command)]
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
        let ride_backups = count_json_files(&data_dir.join("ride-backups"));
        Ok(serde_json::json!({
            "path": data_dir.to_string_lossy().to_string(),
            "isDefault": data_dir == default,
            "prompted": prompted,
            "hasStoragePermission": has_storage_permission,
            "addonsCount": std::fs::read_dir(data_dir.join("addons")).map(|d| d.count()).unwrap_or(0),
            "rideBackupsCount": ride_backups
        }))
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    {
        let _ = app;
        let data_dir = crate::ble::get_terra_storage_dir();
        let ride_backups = count_json_files(&data_dir.join("ride-backups"));
        Ok(serde_json::json!({
            "path": data_dir.to_string_lossy().to_string(),
            "isDefault": true,
            "prompted": true,
            "hasStoragePermission": true,
            "rideBackupsCount": ride_backups
        }))
    }

    #[cfg(not(any(target_os = "android", target_os = "linux", target_os = "windows")))]
    {
        let dir = crate::ble::get_terra_storage_dir(&app);
        let ride_backups = count_json_files(&dir.join("ride-backups"));
        Ok(serde_json::json!({
            "path": dir.to_string_lossy().to_string(),
            "isDefault": true,
            "prompted": true,
            "hasStoragePermission": true,
            "rideBackupsCount": ride_backups
        }))
    }
}

/// Mark the Android first-launch data-folder prompt as dismissed so it won't reappear on launch.
#[cfg_attr(not(any(target_os = "linux", target_os = "windows")), tauri::command)]
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
#[cfg_attr(not(any(target_os = "linux", target_os = "windows")), tauri::command)]
pub fn terra_reset_all_data(app: AppHandle) -> Result<serde_json::Value, String> {
    do_reset(resolve_data_dir(&app))
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

// ─── WebView storage export & proactive ride auto-backup ────────────────────

fn resolve_data_dir(app: &AppHandle) -> PathBuf {
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    {
        let _ = app;
        crate::ble::get_terra_storage_dir()
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        crate::ble::get_terra_storage_dir(app)
    }
}

fn resolve_webview_default_dir(app: &AppHandle) -> Option<PathBuf> {
    #[cfg(target_os = "android")]
    {
        crate::ble::android_internal_files_dir(app)
            .and_then(|d| d.parent().map(|p| p.join("app_webview").join("Default")))
            .filter(|p| p.exists())
    }
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    {
        let _ = app;
        let base = dirs::config_dir()?;
        for name in ["Terra", "terra-desktop", "terra"] {
            let cand = base.join(name);
            if cand.join("IndexedDB").exists() || cand.join("Local Storage").exists() {
                return Some(cand);
            }
        }
        None
    }
    #[cfg(not(any(target_os = "android", target_os = "linux", target_os = "windows")))]
    {
        use tauri::Manager;
        app.path().app_data_dir().ok().filter(|p| p.exists())
    }
}

/// Format current UTC time as `YYYYMMDD-HHMMSS` without external date crates.
fn utc_timestamp_tag() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let hh = rem / 3600;
    let mm = (rem % 3600) / 60;
    let ss = rem % 60;
    // Howard Hinnant's civil_from_days algorithm (days since 1970-01-01)
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}{m:02}{d:02}-{hh:02}{mm:02}{ss:02}")
}

fn copy_dir_skip_locks(
    src: &std::path::Path,
    dst: &std::path::Path,
    files: &mut usize,
    bytes: &mut u64,
    skipped: &mut usize,
) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| format!("mkdir {}: {e}", dst.display()))?;
    let entries = std::fs::read_dir(src).map_err(|e| format!("readdir {}: {e}", src.display()))?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name == "LOCK" {
            continue;
        }
        let sp = entry.path();
        let dp = dst.join(&name);
        let ft = match entry.file_type() {
            Ok(t) => t,
            Err(_) => {
                *skipped += 1;
                continue;
            }
        };
        if ft.is_dir() {
            if copy_dir_skip_locks(&sp, &dp, files, bytes, skipped).is_err() {
                *skipped += 1;
            }
        } else if ft.is_file() {
            match std::fs::copy(&sp, &dp) {
                Ok(n) => {
                    *files += 1;
                    *bytes += n;
                }
                Err(_) => {
                    *skipped += 1;
                }
            }
        }
    }
    Ok(())
}

fn count_json_files(dir: &std::path::Path) -> usize {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("json"))
                .count()
        })
        .unwrap_or(0)
}

fn prune_oldest_json(dir: &std::path::Path, keep: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<(std::time::SystemTime, PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if p.extension().and_then(|s| s.to_str()) != Some("json") {
                return None;
            }
            let mtime = e
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            Some((mtime, p))
        })
        .collect();
    if entries.len() <= keep {
        return;
    }
    entries.sort_by_key(|(t, _)| *t);
    let to_remove = entries.len() - keep;
    for (_, p) in entries.into_iter().take(to_remove) {
        let _ = std::fs::remove_file(p);
    }
}

/// Read-only export of the WebView's IndexedDB, Local Storage, and Session Storage
/// into `<terra_data_dir>/webview-backups/webview-<YYYYMMDD-HHMMSS>/`.
#[cfg_attr(not(any(target_os = "linux", target_os = "windows")), tauri::command)]
pub async fn terra_export_webview_storage(app: AppHandle) -> Result<serde_json::Value, String> {
    let src_root = resolve_webview_default_dir(&app)
        .ok_or_else(|| "WebView storage directory not found on this device.".to_string())?;
    let data_dir = resolve_data_dir(&app);
    let dest_root = data_dir
        .join("webview-backups")
        .join(format!("webview-{}", utc_timestamp_tag()));

    tokio::task::spawn_blocking(move || {
        let mut files = 0usize;
        let mut bytes = 0u64;
        let mut skipped = 0usize;
        let mut sections: Vec<&str> = Vec::new();

        for sub in ["IndexedDB", "Local Storage", "Session Storage"] {
            let sp = src_root.join(sub);
            if sp.is_dir() {
                let dp = dest_root.join(sub);
                copy_dir_skip_locks(&sp, &dp, &mut files, &mut bytes, &mut skipped)?;
                sections.push(sub);
            }
        }

        if files == 0 {
            return Err(format!(
                "No WebView storage files found under {}",
                src_root.display()
            ));
        }

        Ok(serde_json::json!({
            "status": "ok",
            "path": dest_root.to_string_lossy().to_string(),
            "files": files,
            "bytes": bytes,
            "skipped": skipped,
            "sections": sections
        }))
    })
    .await
    .map_err(|e| format!("export task failed: {e}"))?
}

fn sanitize_token(s: &str, max_len: usize) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .take(max_len)
        .collect();
    cleaned.trim_matches('_').to_string()
}

/// Atomically persist an in-progress Biketerra ride snapshot into
/// `<terra_data_dir>/ride-backups/<key>-<session_tag>.json`.
/// Never overwrites a backup for the same session with a shorter/reset snapshot.
#[cfg_attr(not(any(target_os = "linux", target_os = "windows")), tauri::command)]
pub async fn terra_backup_ride(
    app: AppHandle,
    key: String,
    session_tag: String,
    max_dist: f64,
    row_count: usize,
    payload: String,
) -> Result<serde_json::Value, String> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    static LAST_SAVED: OnceLock<Mutex<HashMap<String, (f64, usize)>>> = OnceLock::new();

    if payload.is_empty() {
        return Err("empty ride payload".into());
    }
    if payload.len() > 50 * 1024 * 1024 {
        return Err("ride payload exceeds 50 MB limit".into());
    }
    let clean_key = sanitize_token(&key, 64);
    let clean_tag = sanitize_token(&session_tag, 32);
    if clean_key.is_empty() || clean_tag.is_empty() {
        return Err("invalid ride key or session tag".into());
    }

    let data_dir = resolve_data_dir(&app);
    let dir = data_dir.join("ride-backups");
    let filename = format!("{clean_key}-{clean_tag}.json");
    let dest = dir.join(&filename);
    let map_key = filename.clone();

    // Monotonic guard in memory: never regress distance or row count for the same session file.
    let cache = LAST_SAVED.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(guard) = cache.lock() {
        if let Some(&(prev_dist, prev_rows)) = guard.get(&map_key) {
            if max_dist < prev_dist - 0.01 || (max_dist <= prev_dist + 0.01 && row_count <= prev_rows) {
                return Ok(serde_json::json!({
                    "status": "skipped",
                    "reason": "up_to_date",
                    "path": dest.to_string_lossy().to_string(),
                    "backupsCount": count_json_files(&dir)
                }));
            }
        }
    }

    let payload_len = payload.len() as u64;
    let res = tokio::task::spawn_blocking(move || -> Result<(bool, PathBuf, usize), String> {
        std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir ride-backups: {e}"))?;
        // Disk guard across restarts: don't clobber a larger existing file for the same session.
        if let Ok(meta) = std::fs::metadata(&dest) {
            if meta.len() > payload_len {
                return Ok((false, dest, count_json_files(&dir)));
            }
        }
        let tmp = dir.join(format!(".{filename}.tmp"));
        std::fs::write(&tmp, payload.as_bytes())
            .map_err(|e| format!("write ride backup tmp: {e}"))?;
        std::fs::rename(&tmp, &dest)
            .map_err(|e| format!("rename ride backup: {e}"))?;
        prune_oldest_json(&dir, 50);
        let count = count_json_files(&dir);
        Ok((true, dest, count))
    })
    .await
    .map_err(|e| format!("backup task failed: {e}"))??;

    let (wrote, dest_path, count) = res;
    if wrote {
        if let Ok(mut guard) = cache.lock() {
            guard.insert(map_key, (max_dist, row_count));
        }
        Ok(serde_json::json!({
            "status": "saved",
            "path": dest_path.to_string_lossy().to_string(),
            "bytes": payload_len,
            "backupsCount": count
        }))
    } else {
        Ok(serde_json::json!({
            "status": "skipped",
            "reason": "existing_larger",
            "path": dest_path.to_string_lossy().to_string(),
            "backupsCount": count
        }))
    }
}
