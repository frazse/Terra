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

// ─── Ride backup manager (list, read/restore, GPX export, delete) ───────────

fn validate_backup_filename(filename: &str) -> Result<&str, String> {
    if filename.is_empty()
        || filename.len() > 140
        || !filename.ends_with(".json")
        || filename.contains('/')
        || filename.contains('\\')
        || filename.contains("..")
        || !filename
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err("invalid backup filename".into());
    }
    Ok(filename)
}

/// Extract the Biketerra IndexedDB key from `<key>-YYYYMMDD-HHMMSS.json`.
fn infer_key_from_filename(filename: &str) -> String {
    let stem = filename.strip_suffix(".json").unwrap_or(filename);
    // Strip trailing `-YYYYMMDD-HHMMSS` (16 chars) when present
    if stem.len() > 16 {
        let suffix = &stem[stem.len() - 16..];
        let b = suffix.as_bytes();
        if b[0] == b'-'
            && b[9] == b'-'
            && b[1..9].iter().all(|c| c.is_ascii_digit())
            && b[10..16].iter().all(|c| c.is_ascii_digit())
        {
            return stem[..stem.len() - 16].to_string();
        }
    }
    stem.to_string()
}

fn format_utc_iso_ms(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400) as u64;
    let hh = rem / 3600;
    let mm = (rem % 3600) / 60;
    let ss = rem % 60;
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
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

fn to_degrees(v: f64) -> Option<f64> {
    if !v.is_finite() || v == 0.0 || (v as i64) == 0x7FFF_FFFF {
        return None;
    }
    let deg = if v.abs() > 180.0 {
        v * (180.0 / 2_147_483_648.0)
    } else {
        v
    };
    if deg.abs() <= 180.0 {
        Some(deg)
    } else {
        None
    }
}

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

fn resolve_export_dir(app: &AppHandle) -> PathBuf {
    #[cfg(target_os = "android")]
    {
        let storage = crate::ble::get_terra_storage_dir(app);
        let hidden_default = PathBuf::from("/storage/emulated/0/Android/data/dev.frazse.terra/files");
        if storage != hidden_default {
            return storage.join("exports");
        }
        let public_dl = PathBuf::from("/storage/emulated/0/Download/Terra");
        if std::fs::create_dir_all(&public_dl).is_ok() {
            return public_dl;
        }
        storage.join("exports")
    }
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    {
        let _ = app;
        dirs::download_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(crate::ble::get_terra_storage_dir)
            .join("Terra")
    }
    #[cfg(not(any(target_os = "android", target_os = "linux", target_os = "windows")))]
    {
        resolve_data_dir(app).join("exports")
    }
}

/// List saved JSON snapshots in `<terra_data_dir>/ride-backups/` with summary metrics.
#[cfg_attr(not(any(target_os = "linux", target_os = "windows")), tauri::command)]
pub async fn terra_list_ride_backups(app: AppHandle) -> Result<serde_json::Value, String> {
    let dir = resolve_data_dir(&app).join("ride-backups");
    tokio::task::spawn_blocking(move || {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            return Ok(serde_json::json!({ "dir": dir.to_string_lossy(), "backups": [] }));
        };
        let mut items: Vec<(u64, serde_json::Value)> = Vec::new();
        for entry in rd.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let Some(fname) = path.file_name().and_then(|s| s.to_str()).map(|s| s.to_string()) else {
                continue;
            };
            let meta = entry.metadata().ok();
            let bytes = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let mtime_ms = meta
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);

            let key = infer_key_from_filename(&fname);
            let mut row_count = 0usize;
            let mut max_dist = 0.0f64;
            let mut start_ts: Option<i64> = None;
            let mut end_ts: Option<i64> = None;
            let mut pwr_sum = 0.0f64;
            let mut pwr_cnt = 0usize;
            let mut max_hr = 0u64;

            if let Ok(raw) = std::fs::read_to_string(&path) {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) {
                    let keys: Vec<&str> = val
                        .get("keys")
                        .and_then(|k| k.as_array())
                        .map(|arr| arr.iter().filter_map(|x| x.as_str()).collect())
                        .unwrap_or_default();
                    let col = |name: &str| keys.iter().position(|&k| k == name);
                    let t_idx = col("timestamp");
                    let d_idx = col("distance");
                    let p_idx = col("power");
                    let h_idx = col("heart_rate");

                    if let Some(rows) = val.get("rows").and_then(|r| r.as_array()) {
                        row_count = rows.len();
                        if let Some(ti) = t_idx {
                            start_ts = rows.first().and_then(|r| r.get(ti)).and_then(|v| v.as_i64());
                            end_ts = rows.last().and_then(|r| r.get(ti)).and_then(|v| v.as_i64());
                        }
                        for r in rows {
                            if let Some(di) = d_idx {
                                if let Some(d) = r.get(di).and_then(|v| v.as_f64()) {
                                    if d > max_dist {
                                        max_dist = d;
                                    }
                                }
                            }
                            if let Some(pi) = p_idx {
                                if let Some(p) = r.get(pi).and_then(|v| v.as_f64()) {
                                    if p > 0.0 {
                                        pwr_sum += p;
                                        pwr_cnt += 1;
                                    }
                                }
                            }
                            if let Some(hi) = h_idx {
                                if let Some(h) = r.get(hi).and_then(|v| v.as_u64()) {
                                    if h > max_hr {
                                        max_hr = h;
                                    }
                                }
                            }
                        }
                    }
                }
            }

            let avg_power = if pwr_cnt > 0 {
                Some((pwr_sum / pwr_cnt as f64).round() as u64)
            } else {
                None
            };
            items.push((
                mtime_ms,
                serde_json::json!({
                    "filename": fname,
                    "key": key,
                    "bytes": bytes,
                    "rows": row_count,
                    "maxDist": (max_dist * 100.0).round() / 100.0,
                    "startTs": start_ts,
                    "endTs": end_ts,
                    "avgPower": avg_power,
                    "maxHr": if max_hr > 0 { Some(max_hr) } else { None },
                    "modifiedMs": mtime_ms
                }),
            ));
        }
        items.sort_by(|a, b| b.0.cmp(&a.0));
        let backups: Vec<serde_json::Value> = items.into_iter().map(|(_, v)| v).collect();
        Ok(serde_json::json!({
            "dir": dir.to_string_lossy(),
            "backups": backups
        }))
    })
    .await
    .map_err(|e| format!("list backups task failed: {e}"))?
}

/// Read a saved ride backup JSON so the WebView can restore it into Biketerra's IndexedDB.
#[cfg_attr(not(any(target_os = "linux", target_os = "windows")), tauri::command)]
pub async fn terra_read_ride_backup(
    app: AppHandle,
    filename: String,
) -> Result<serde_json::Value, String> {
    let clean = validate_backup_filename(&filename)?.to_string();
    let path = resolve_data_dir(&app).join("ride-backups").join(&clean);
    tokio::task::spawn_blocking(move || {
        let payload =
            std::fs::read_to_string(&path).map_err(|e| format!("read backup {clean}: {e}"))?;
        let key = infer_key_from_filename(&clean);
        Ok(serde_json::json!({
            "filename": clean,
            "key": key,
            "payload": payload
        }))
    })
    .await
    .map_err(|e| format!("read backup task failed: {e}"))?
}

/// Convert a saved ride backup JSON into GPX 1.1 (with power, HR, cadence extensions)
/// and write it to the visible exports / Downloads folder.
#[cfg_attr(not(any(target_os = "linux", target_os = "windows")), tauri::command)]
pub async fn terra_export_ride_gpx(
    app: AppHandle,
    filename: String,
) -> Result<serde_json::Value, String> {
    let clean = validate_backup_filename(&filename)?.to_string();
    let src_path = resolve_data_dir(&app).join("ride-backups").join(&clean);
    let export_dir = resolve_export_dir(&app);

    tokio::task::spawn_blocking(move || {
        let raw =
            std::fs::read_to_string(&src_path).map_err(|e| format!("read backup {clean}: {e}"))?;
        let val: serde_json::Value =
            serde_json::from_str(&raw).map_err(|e| format!("parse backup JSON: {e}"))?;

        let keys: Vec<&str> = val
            .get("keys")
            .and_then(|k| k.as_array())
            .map(|arr| arr.iter().filter_map(|x| x.as_str()).collect())
            .ok_or_else(|| "backup missing keys array".to_string())?;
        let rows = val
            .get("rows")
            .and_then(|r| r.as_array())
            .ok_or_else(|| "backup missing rows array".to_string())?;

        let col = |name: &str| keys.iter().position(|&k| k == name);
        let t_idx = col("timestamp");
        let lat_idx = col("lat").ok_or_else(|| "backup missing lat column".to_string())?;
        let lng_idx = col("lng").ok_or_else(|| "backup missing lng column".to_string())?;
        let alt_idx = col("altitude");
        let hr_idx = col("heart_rate");
        let cad_idx = col("cadence");
        let pwr_idx = col("power");

        let stem = clean.strip_suffix(".json").unwrap_or(&clean);
        let ride_key = infer_key_from_filename(&clean);
        let start_iso = t_idx
            .and_then(|ti| rows.first().and_then(|r| r.get(ti)).and_then(|v| v.as_i64()))
            .map(format_utc_iso_ms);

        let mut gpx = String::with_capacity(rows.len() * 160 + 512);
        gpx.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        gpx.push_str("<gpx version=\"1.1\" creator=\"Terra\" xmlns=\"http://www.topografix.com/GPX/1/1\" xmlns:gpxtpx=\"http://www.garmin.com/xmlschemas/TrackPointExtension/v1\">\n");
        gpx.push_str("  <metadata>\n");
        gpx.push_str(&format!("    <name>Biketerra {}</name>\n", xml_escape(&ride_key)));
        if let Some(ref iso) = start_iso {
            gpx.push_str(&format!("    <time>{iso}</time>\n"));
        }
        gpx.push_str("  </metadata>\n  <trk>\n");
        gpx.push_str(&format!("    <name>Biketerra {}</name>\n    <type>cycling</type>\n    <trkseg>\n", xml_escape(&ride_key)));

        let mut points = 0usize;
        for r in rows {
            let Some(lat) = r.get(lat_idx).and_then(|v| v.as_f64()).and_then(to_degrees) else {
                continue;
            };
            let Some(lon) = r.get(lng_idx).and_then(|v| v.as_f64()).and_then(to_degrees) else {
                continue;
            };
            gpx.push_str(&format!("      <trkpt lat=\"{lat:.7}\" lon=\"{lon:.7}\">\n"));
            if let Some(alt) = alt_idx.and_then(|i| r.get(i)).and_then(|v| v.as_f64()) {
                if alt.is_finite() {
                    gpx.push_str(&format!("        <ele>{alt:.1}</ele>\n"));
                }
            }
            if let Some(ts) = t_idx.and_then(|i| r.get(i)).and_then(|v| v.as_i64()) {
                gpx.push_str(&format!("        <time>{}</time>\n", format_utc_iso_ms(ts)));
            }
            let pwr = pwr_idx
                .and_then(|i| r.get(i))
                .and_then(|v| v.as_f64())
                .filter(|&v| v.is_finite() && v > 0.0)
                .map(|v| v.round() as u64);
            let hr = hr_idx
                .and_then(|i| r.get(i))
                .and_then(|v| v.as_u64())
                .filter(|&v| v > 0);
            let cad = cad_idx
                .and_then(|i| r.get(i))
                .and_then(|v| v.as_u64())
                .filter(|&v| v > 0);
            if pwr.is_some() || hr.is_some() || cad.is_some() {
                gpx.push_str("        <extensions>\n");
                if let Some(p) = pwr {
                    gpx.push_str(&format!("          <power>{p}</power>\n"));
                }
                if hr.is_some() || cad.is_some() {
                    gpx.push_str("          <gpxtpx:TrackPointExtension>\n");
                    if let Some(h) = hr {
                        gpx.push_str(&format!("            <gpxtpx:hr>{h}</gpxtpx:hr>\n"));
                    }
                    if let Some(c) = cad {
                        gpx.push_str(&format!("            <gpxtpx:cad>{c}</gpxtpx:cad>\n"));
                    }
                    gpx.push_str("          </gpxtpx:TrackPointExtension>\n");
                }
                gpx.push_str("        </extensions>\n");
            }
            gpx.push_str("      </trkpt>\n");
            points += 1;
        }

        if points == 0 {
            return Err("Backup has no valid GPS coordinates to export.".into());
        }
        gpx.push_str("    </trkseg>\n  </trk>\n</gpx>\n");

        std::fs::create_dir_all(&export_dir)
            .map_err(|e| format!("mkdir {}: {e}", export_dir.display()))?;
        let mut out_path = export_dir.join(format!("{stem}.gpx"));
        let mut n = 1usize;
        while out_path.exists() {
            out_path = export_dir.join(format!("{stem}-{n}.gpx"));
            n += 1;
        }
        let bytes = gpx.len() as u64;
        std::fs::write(&out_path, gpx.as_bytes())
            .map_err(|e| format!("write GPX {}: {e}", out_path.display()))?;

        Ok(serde_json::json!({
            "status": "ok",
            "path": out_path.to_string_lossy().to_string(),
            "points": points,
            "bytes": bytes
        }))
    })
    .await
    .map_err(|e| format!("export GPX task failed: {e}"))?
}

/// Delete a single saved ride backup file from `<terra_data_dir>/ride-backups/`.
#[cfg_attr(not(any(target_os = "linux", target_os = "windows")), tauri::command)]
pub async fn terra_delete_ride_backup(
    app: AppHandle,
    filename: String,
) -> Result<serde_json::Value, String> {
    let clean = validate_backup_filename(&filename)?.to_string();
    let dir = resolve_data_dir(&app).join("ride-backups");
    let path = dir.join(&clean);
    tokio::task::spawn_blocking(move || {
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| format!("delete {clean}: {e}"))?;
        }
        Ok(serde_json::json!({
            "status": "ok",
            "backupsCount": count_json_files(&dir)
        }))
    })
    .await
    .map_err(|e| format!("delete backup task failed: {e}"))?
}
