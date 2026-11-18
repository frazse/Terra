//! Addon system: scans addon folders, evaluates matching addons inside the biketerra webview
//! (via webview.eval, so the page's CSP cannot block them), and stores per-addon data.
//! Addons run with the page's privileges, like userscripts: only install ones you trust.

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
#[cfg(not(target_os = "linux"))]
use tauri::{AppHandle, Manager, State};
#[cfg(target_os = "linux")]
use crate::linux_runtime::{AppHandle, State};

#[derive(Deserialize)]
struct Manifest {
    id: Option<String>,
    name: Option<String>,
    version: Option<String>,
    description: Option<String>,
    #[serde(default)]
    category: Option<String>,
    matches: Vec<String>,
    #[serde(default)]
    js: Vec<String>,
    #[serde(default)]
    css: Vec<String>,
    /// Hosts the addon may reach through `terra.http` (exact name, or `*.example.com`).
    #[serde(default)]
    connect: Vec<String>,
    /// Settings shown in the cog menu. `"type": "password"` values are write-only for the page.
    #[serde(default)]
    settings: Vec<SettingDef>,
    #[serde(default = "yes")]
    enabled: bool,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct SettingDef {
    pub key: String,
    #[serde(default)]
    pub label: String,
    /// text | password | number | checkbox | select | color
    #[serde(default = "text_kind", rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub default: Option<Value>,
    /// Choices for `select` settings (the label and value are the same string).
    #[serde(default)]
    pub options: Vec<String>,
}

fn text_kind() -> String {
    "text".into()
}

impl SettingDef {
    fn secret(&self) -> bool {
        self.kind == "password"
    }
    fn name(&self) -> &str {
        if self.label.is_empty() { &self.key } else { &self.label }
    }
}

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// `example.com` matches only itself; `*.example.com` matches its subdomains.
fn host_matches(pattern: &str, host: &str) -> bool {
    let (p, h) = (pattern.to_lowercase(), host.to_lowercase());
    match p.strip_prefix("*.") {
        Some(suffix) => h.ends_with(&format!(".{suffix}")),
        None => p == h,
    }
}

fn yes() -> bool {
    true
}

pub struct Addon {
    pub id: String,
    pub name: String,
    pub version: String,
    description: String,
    category: String,
    matches: Vec<String>,
    js: String,
    css: Vec<String>,
    connect: Vec<String>,
    settings: Vec<SettingDef>,
    /// Manifest default (`"enabled": false` = off until the user turns it on).
    default_enabled: bool,
    /// Effective state: the user's choice in the Terra cog menu, else the manifest default.
    pub enabled: bool,
}

impl Addon {
    fn hits(&self, url: &str) -> bool {
        self.matches.iter().any(|m| glob(m, url))
    }
}

fn valid_id(s: &str) -> bool {
    !s.is_empty()
    && s.len() <= 64
    && !s.starts_with('.')
    && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

fn safe_rel(f: &str) -> bool {
    let p = Path::new(f);
    !p.is_absolute() && !p.components().any(|c| matches!(c, std::path::Component::ParentDir))
}

/// `*` matches any run of characters.
fn glob(pat: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pat.chars().collect(), text.chars().collect());
    let (mut pi, mut ti, mut star, mut mark) = (0, 0, None::<usize>, 0);
    while ti < t.len() {
        if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if pi < p.len() && p[pi] == t[ti] {
            pi += 1;
            ti += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// `TERRA_ADDONS_DIR` (path list) overrides; default is `<app data>/addons` and `./addons`.
#[cfg(target_os = "linux")]
pub fn addon_dirs(_app: &AppHandle) -> Vec<PathBuf> {
    if let Some(v) = std::env::var_os("TERRA_ADDONS_DIR") {
        return std::env::split_paths(&v).collect();
    }
    let mut d = vec![];
    let dir = crate::ble::get_terra_storage_dir();
    d.push(dir.join("addons"));
    if let Ok(c) = std::env::current_dir() {
        d.push(c.join("addons"));
    }
    d
}

#[cfg(not(target_os = "linux"))]
pub fn addon_dirs(app: &AppHandle) -> Vec<PathBuf> {
    if let Some(v) = std::env::var_os("TERRA_ADDONS_DIR") {
        return std::env::split_paths(&v).collect();
    }
    let mut d = vec![];
    let dir = crate::ble::get_terra_storage_dir(app);
    d.push(dir.join("addons"));
    if let Ok(c) = std::env::current_dir() {
        d.push(c.join("addons"));
    }
    d
}

pub fn load_all(app: &AppHandle) -> Vec<Addon> {
    load_all_inner(app, true)
}

/// Metadata-only scan (skips reading `.js`/`.css` files) for runtime checks and settings.
fn load_manifests(app: &AppHandle) -> Vec<Addon> {
    load_all_inner(app, false)
}

fn load_all_inner(app: &AppHandle, load_code: bool) -> Vec<Addon> {
    let mut out: Vec<Addon> = vec![];
    for dir in addon_dirs(app) {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        let mut entries: Vec<_> = rd.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let path = e.path();
            let folder = e.file_name().to_string_lossy().to_string();
            if !path.is_dir() || folder.starts_with('_') || folder.starts_with('.') {
                continue;
            }
            match load_one(&path, &folder, load_code) {
                Ok(Some(a)) => {
                    if out.iter().any(|x| x.id == a.id) {
                        eprintln!("[terra] addon '{}': duplicate id, skipped ({})", a.id, path.display());
                    } else {
                        out.push(a);
                    }
                }
                Ok(None) => {}
                Err(e) => eprintln!("[terra] addon '{folder}': {e}"),
            }
        }
    }
    // The user's on/off choices (cog menu) override the manifest default.
    if let Some(store) = app.try_state::<AddonStore>() {
        let ov = store.overrides();
        for a in &mut out {
            if let Some(v) = ov.get(&a.id) {
                a.enabled = *v;
            }
        }
    }
    out
}

fn read_files(dir: &Path, files: &[String]) -> Result<Vec<String>, String> {
    files
    .iter()
    .map(|f| {
        if !safe_rel(f) {
            return Err(format!("bad file path '{f}'"));
        }
        fs::read_to_string(dir.join(f)).map_err(|e| format!("{f}: {e}"))
    })
    .collect()
}

fn infer_category(id: &str, name: &str, explicit: Option<&str>) -> String {
    if let Some(c) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        return c.to_string();
    }
    let hay = format!("{id} {name}").to_lowercase();
    if ["zwift", "karoo", "trainer", "ble", "ant", "controller"]
        .iter()
        .any(|k| hay.contains(k))
    {
        return "Hardware".into();
    }
    if ["cam", "pip"].iter().any(|k| hay.contains(k)) {
        return "Camera".into();
    }
    if ["strava", "intervals", "export", "fit", "discord", "gpx", "tcx", "sync", "logger"]
        .iter()
        .any(|k| hay.contains(k))
    {
        return "Export".into();
    }
    if ["hud", "nameplate", "speed", "fps", "radar", "climb", "profile", "draft", "garmin"]
        .iter()
        .any(|k| hay.contains(k))
    {
        return "HUD".into();
    }
    "Other".into()
}

fn writable_addons_dir(app: &AppHandle) -> PathBuf {
    if let Some(v) = std::env::var_os("TERRA_ADDONS_DIR") {
        if let Some(first) = std::env::split_paths(&v).next() {
            return first;
        }
    }
    let _ = app;
    #[cfg(target_os = "linux")]
    let dir = crate::ble::get_terra_storage_dir();
    #[cfg(not(target_os = "linux"))]
    let dir = crate::ble::get_terra_storage_dir(app);
    dir.join("addons")
}

fn load_one(path: &Path, folder: &str, load_code: bool) -> Result<Option<Addon>, String> {
    let raw = fs::read_to_string(path.join("manifest.json")).map_err(|e| format!("manifest.json: {e}"))?;
    let m: Manifest = serde_json::from_str(&raw).map_err(|e| format!("manifest.json: {e}"))?;
    let id = m.id.unwrap_or_else(|| folder.to_string());
    if !valid_id(&id) {
        return Err(format!("invalid id '{id}'"));
    }
    let (js, css) = if load_code {
        (read_files(path, &m.js)?.join("\n;\n"), read_files(path, &m.css)?)
    } else {
        (String::new(), Vec::new())
    };
    let name = m.name.unwrap_or_else(|| id.clone());
    let category = infer_category(&id, &name, m.category.as_deref());
    let settings: Vec<SettingDef> = m.settings.into_iter().filter(|d| valid_id(&d.key)).collect();
    Ok(Some(Addon {
        name,
        version: m.version.unwrap_or_else(|| "0.0.0".into()),
        description: m.description.unwrap_or_default(),
        category,
        id,
        matches: m.matches,
        js,
        css,
        connect: m.connect,
        settings,
        default_enabled: m.enabled,
        enabled: m.enabled,
    }))
}

#[derive(Serialize)]
pub struct AddonScript {
    pub id: String,
    pub code: String,
}

#[derive(Serialize)]
pub struct Activation {
    pub started: Vec<String>,
    pub stop: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub scripts: Vec<AddonScript>,
}

/// Called by the page loader on load and on every URL change. Rescans the addon folders (so edits
/// apply on reload), evaluates addons that now match, and reports addons that should be stopped.
#[cfg_attr(not(target_os = "linux"), tauri::command)]
pub async fn addons_activate(app: AppHandle, url: String, active: Vec<String>) -> Activation {
    let addons = load_all(&app);
    let mut res = Activation { started: vec![], stop: vec![], scripts: vec![] };
    let w_opt = app.get_webview_window("main");

    for a in &addons {
        if a.enabled && a.hits(&url) && !active.contains(&a.id) {
            let meta = json!({ "id": a.id, "name": a.name, "version": a.version, "css": a.css });
            let code = format!(
                "window.__terraAddons && window.__terraAddons.start({meta}, async function(terra){{\n{}\n}});",
                               a.js
            );
            if let Some(ref w) = w_opt {
                if w.eval(code.clone()).is_ok() {
                    res.started.push(a.id.clone());
                }
            } else {
                res.started.push(a.id.clone());
                res.scripts.push(AddonScript { id: a.id.clone(), code });
            }
        }
    }
    for id in &active {
        if !addons.iter().any(|a| a.id == *id && a.enabled && a.hits(&url)) {
            res.stop.push(id.clone());
        }
    }

    // Reclaim stale TCP sockets left behind by a hard reload (active == []) or stopped addons.
    if let Some(reg) = app.try_state::<TcpRegistry>() {
        if active.is_empty() {
            reg.close_all().await;
        } else if !res.stop.is_empty() {
            reg.close_for_addons(&res.stop).await;
        }
    }

    res
}

// ---------- per-addon persistent storage ----------

#[derive(Clone)]
pub struct AddonStore {
    dir: PathBuf,
    cache: std::sync::Arc<Mutex<HashMap<String, Map<String, Value>>>>,
    overrides: std::sync::Arc<Mutex<HashMap<String, bool>>>,
    settings_dir: PathBuf,
    settings: std::sync::Arc<Mutex<HashMap<String, Map<String, Value>>>>,
    catalog_cache: std::sync::Arc<Mutex<Option<(std::time::Instant, Vec<u8>)>>>,
}

const STATE_FILE: &str = ".addons-state.json"; // leading '.' can never collide with an addon id

impl AddonStore {
    pub fn new(dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(&dir);
        let overrides = fs::read_to_string(dir.join(STATE_FILE))
        .ok()
        .and_then(|s| serde_json::from_str::<HashMap<String, bool>>(&s).ok())
        .unwrap_or_default();
        let settings_dir = dir.join("settings");
        let _ = fs::create_dir_all(&settings_dir);
        Self {
            dir,
            cache: std::sync::Arc::new(Mutex::new(HashMap::new())),
            overrides: std::sync::Arc::new(Mutex::new(overrides)),
            settings_dir,
            settings: std::sync::Arc::new(Mutex::new(HashMap::new())),
            catalog_cache: std::sync::Arc::new(Mutex::new(None)),
        }
    }

    pub fn overrides(&self) -> HashMap<String, bool> {
        self.overrides.lock().map(|g| g.clone()).unwrap_or_default()
    }

    // ---- settings (separate files from the addon's own storage; secrets never go back to the page) ----

    fn settings_access<R>(&self, id: &str, write: bool, f: impl FnOnce(&mut Map<String, Value>) -> R) -> Result<R, String> {
        if !valid_id(id) {
            return Err("invalid addon id".into());
        }
        let mut cache = self.settings.lock().map_err(|e| e.to_string())?;
        let path = self.settings_dir.join(format!("{id}.json"));
        let m = cache.entry(id.to_string()).or_insert_with(|| {
            fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
        });
        let r = f(m);
        if write {
            let _ = fs::write(&path, serde_json::to_string_pretty(&*m).unwrap_or_default());
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600)); // may hold API keys
            }
        }
        Ok(r)
    }

    /// The stored value, else the manifest default; empty strings count as unset.
    fn setting_value(&self, id: &str, def: &SettingDef) -> Option<Value> {
        let stored = self.settings_access(id, false, |m| m.get(&def.key).cloned()).ok().flatten();
        stored
        .or_else(|| def.default.clone())
        .filter(|v| !v.is_null() && !matches!(v, Value::String(s) if s.is_empty()))
    }

    /// Replaces `{{setting:KEY}}` with the stored value (secrets included). Done here, in Rust, so that
    /// secret values never have to travel to the page.
    fn expand(&self, id: &str, defs: &[SettingDef], text: &str) -> Result<String, String> {
        const OPEN: &str = "{{setting:";
        let mut out = String::new();
        let mut rest = text;
        while let Some(i) = rest.find(OPEN) {
            out.push_str(&rest[..i]);
            let after = &rest[i + OPEN.len()..];
            let j = after.find("}}").ok_or("unterminated {{setting:...}} placeholder")?;
            let key = &after[..j];
            let def = defs
            .iter()
            .find(|d| d.key == key)
            .ok_or_else(|| format!("unknown setting '{key}'"))?;
            let v = self
            .setting_value(id, def)
            .ok_or_else(|| format!("setting '{}' is not set", def.name()))?;
            match v {
                Value::String(s) => out.push_str(&s),
                other => out.push_str(&other.to_string()),
            }
            rest = &after[j + 2..];
        }
        out.push_str(rest);
        Ok(out)
    }

    fn set_enabled(&self, id: &str, enabled: bool) -> Result<(), String> {
        if !valid_id(id) {
            return Err("invalid addon id".into());
        }
        let mut g = self.overrides.lock().map_err(|e| e.to_string())?;
        g.insert(id.to_string(), enabled);
        fs::write(self.dir.join(STATE_FILE), serde_json::to_string_pretty(&*g).unwrap_or_default())
        .map_err(|e| e.to_string())
    }

    fn access<R>(&self, id: &str, write: bool, f: impl FnOnce(&mut Map<String, Value>) -> R) -> Result<R, String> {
        if !valid_id(id) {
            return Err("invalid addon id".into());
        }
        let mut cache = self.cache.lock().map_err(|e| e.to_string())?;
        let path = self.dir.join(format!("{id}.json"));
        let m = cache.entry(id.to_string()).or_insert_with(|| {
            fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
        });
        let r = f(m);
        if write {
            let _ = fs::write(&path, serde_json::to_string_pretty(&*m).unwrap_or_default());
        }
        Ok(r)
    }
}

#[cfg_attr(not(target_os = "linux"), tauri::command)]
pub fn addon_storage_get(store: State<'_, AddonStore>, id: String, key: String) -> Result<Option<Value>, String> {
    store.access(&id, false, |m| m.get(&key).cloned())
}

#[cfg_attr(not(target_os = "linux"), tauri::command)]
pub fn addon_storage_set(store: State<'_, AddonStore>, id: String, key: String, value: Value) -> Result<(), String> {
    store.access(&id, true, |m| {
        m.insert(key, value);
    })
}

#[cfg_attr(not(target_os = "linux"), tauri::command)]
pub fn addon_storage_remove(store: State<'_, AddonStore>, id: String, key: String) -> Result<(), String> {
    store.access(&id, true, |m| {
        m.remove(&key);
    })
}

// ---------- cog menu: list addons / turn them on or off ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddonInfo {
    id: String,
    name: String,
    version: String,
    description: String,
    category: String,
    matches: Vec<String>,
    enabled: bool,
    default_enabled: bool,
    settings: Vec<SettingDef>,
    connect: Vec<String>,
    user_installed: bool,
    bundled: bool,
}

#[derive(Serialize)]
pub struct AddonList {
    version: String,
    dirs: Vec<String>,
    addons: Vec<AddonInfo>,
}

#[cfg_attr(not(target_os = "linux"), tauri::command)]
pub async fn addons_list(app: AppHandle) -> AddonList {
    let dirs = addon_dirs(&app);
    let user_dir = writable_addons_dir(&app);
    let other_dirs: Vec<&PathBuf> = dirs.iter().filter(|d| *d != &user_dir).collect();
    AddonList {
        version: app.package_info().version.to_string(),
        dirs: dirs.iter().map(|d| d.display().to_string()).collect(),
        addons: load_manifests(&app)
        .into_iter()
        .map(|a| {
            let user_installed = user_dir.join(&a.id).join("manifest.json").is_file();
            let bundled = other_dirs.iter().any(|d| d.join(&a.id).join("manifest.json").is_file());
            AddonInfo {
                id: a.id,
                name: a.name,
                version: a.version,
                description: a.description,
                category: a.category,
                matches: a.matches,
                enabled: a.enabled,
                default_enabled: a.default_enabled,
                settings: a.settings,
                connect: a.connect,
                user_installed,
                bundled,
            }
        })
        .collect(),
    }
}

#[cfg_attr(not(target_os = "linux"), tauri::command)]
pub fn addons_set_enabled(store: State<'_, AddonStore>, id: String, enabled: bool) -> Result<(), String> {
    store.set_enabled(&id, enabled)
}

// ---------- settings commands ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    /// Non-secret values (stored value, else the manifest default).
    values: Map<String, Value>,
    /// Keys of password settings that have a value. The values themselves are never returned.
    secrets_set: Vec<String>,
}

#[cfg_attr(not(target_os = "linux"), tauri::command)]
pub async fn addon_settings_get(app: AppHandle, store: State<'_, AddonStore>, id: String) -> Result<SettingsView, String> {
    let addons = load_manifests(&app);
    let a = addons.iter().find(|a| a.id == id).ok_or("unknown addon")?;
    let mut values = Map::new();
    let mut secrets_set = vec![];
    for d in &a.settings {
        let v = store.setting_value(&id, d);
        if d.secret() {
            if v.is_some() {
                secrets_set.push(d.key.clone());
            }
        } else if let Some(v) = v {
            values.insert(d.key.clone(), v);
        }
    }
    Ok(SettingsView { values, secrets_set })
}

/// Platform convention for the BLE supervisor: an enabled addon may declare a text
/// setting named `btExcludeNames` (device names, comma-separated). While the addon
/// is enabled, remembered devices with those names must not be pre-connected over
/// Bluetooth — e.g. the Wi-Fi Trainer Bridge keeps the physical Kickr's Bluetooth
/// link out of auto-connect while it drives the unit over Wi-Fi, while HR straps
/// and the like keep their fast pre-connect.
pub fn bt_exclude_names(app: &AppHandle) -> HashSet<String> {
    let store: State<'_, AddonStore> = app.state();
    let mut names = HashSet::new();
    for a in load_manifests(app) {
        if !a.enabled {
            continue;
        }
        let Some(def) = a.settings.iter().find(|d| d.key == "btExcludeNames") else {
            continue;
        };
        if let Some(Value::String(s)) = store.setting_value(&a.id, def) {
            for n in s.split(',') {
                let n = n.trim();
                if !n.is_empty() {
                    names.insert(n.to_string());
                }
            }
        }
    }
    names
}

fn validate_setting_value(d: &SettingDef, value: &Value) -> Result<Option<Value>, String> {
    match (d.kind.as_str(), value) {
        (_, Value::Null) => Ok(None),
        (_, Value::String(s)) if s.is_empty() => Ok(None),
        ("number", Value::String(s)) => {
            let n = s.trim().parse::<f64>().map_err(|_| "not a number")?;
            if !n.is_finite() {
                return Err("not a number".into());
            }
            Ok(Some(json!(n)))
        }
        ("number", Value::Number(_)) => Ok(Some(value.clone())),
        ("number", _) => Err("not a number".into()),
        ("checkbox", Value::Bool(_)) => Ok(Some(value.clone())),
        ("checkbox", _) => Err("not a boolean".into()),
        (_, Value::String(s)) => Ok(Some(Value::String(s.clone()))),
        _ => Err("unsupported value".into()),
    }
}

/// An empty string or null removes the value (the manifest default applies again).
#[cfg_attr(not(target_os = "linux"), tauri::command)]
pub async fn addon_settings_set(
    app: AppHandle,
    store: State<'_, AddonStore>,
    id: String,
    key: String,
    value: Value,
) -> Result<(), String> {
    let addons = load_manifests(&app);
    let a = addons.iter().find(|a| a.id == id).ok_or("unknown addon")?;
    let d = a
        .settings
        .iter()
        .find(|d| d.key == key)
        .ok_or_else(|| format!("unknown setting '{key}'"))?;
    let v = validate_setting_value(d, &value)?;
    store.settings_access(&id, true, |m| match v {
        Some(v) => {
            m.insert(key, v);
        }
        None => {
            m.remove(&key);
        }
    })
}

#[cfg_attr(not(target_os = "linux"), tauri::command)]
pub async fn addon_settings_reset(
    app: AppHandle,
    store: State<'_, AddonStore>,
    id: String,
) -> Result<(), String> {
    let addons = load_manifests(&app);
    if !addons.iter().any(|a| a.id == id) {
        return Err("unknown addon".into());
    }
    store.settings_access(&id, true, |m| m.clear())
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddonBackup {
    pub id: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub settings: Map<String, Value>,
    #[serde(default)]
    pub storage: Map<String, Value>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub settings_count: usize,
    pub storage_count: usize,
}

#[cfg_attr(not(target_os = "linux"), tauri::command)]
pub async fn addon_settings_export(
    app: AppHandle,
    store: State<'_, AddonStore>,
    id: String,
) -> Result<AddonBackup, String> {
    let addons = load_manifests(&app);
    let a = addons.iter().find(|a| a.id == id).ok_or("unknown addon")?;
    let secret_keys: HashSet<&str> = a.settings.iter().filter(|d| d.secret()).map(|d| d.key.as_str()).collect();
    let settings = store.settings_access(&id, false, |m| {
        m.iter()
            .filter(|(k, _)| !secret_keys.contains(k.as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect::<Map<String, Value>>()
    })?;
    let storage = store.access(&id, false, |m| m.clone())?;
    Ok(AddonBackup {
        id: a.id.clone(),
        version: a.version.clone(),
        settings,
        storage,
    })
}

#[cfg_attr(not(target_os = "linux"), tauri::command)]
pub async fn addon_settings_import(
    app: AppHandle,
    store: State<'_, AddonStore>,
    id: String,
    payload: Value,
) -> Result<ImportResult, String> {
    let addons = load_manifests(&app);
    let a = addons.iter().find(|a| a.id == id).ok_or("unknown addon")?;
    let backup: AddonBackup = serde_json::from_value(payload).map_err(|e| format!("invalid backup JSON: {e}"))?;
    if !backup.id.is_empty() && backup.id != id {
        return Err(format!("backup is for addon '{}', not '{}'", backup.id, id));
    }

    let mut validated = Map::new();
    for (k, raw_v) in &backup.settings {
        if let Some(d) = a.settings.iter().find(|d| d.key == *k && !d.secret()) {
            if let Ok(Some(v)) = validate_setting_value(d, raw_v) {
                validated.insert(k.clone(), v);
            }
        }
    }
    let settings_count = validated.len();
    if settings_count > 0 {
        store.settings_access(&id, true, |m| {
            for (k, v) in validated {
                m.insert(k, v);
            }
        })?;
    }

    let storage_count = backup.storage.len();
    if storage_count > 0 {
        store.access(&id, true, |m| {
            for (k, v) in backup.storage {
                m.insert(k, v);
            }
        })?;
    }

    Ok(ImportResult { settings_count, storage_count })
}

// ---------- addon install / catalog / uninstall ----------

const GITHUB_ADDONS_ZIP_URL: &str = "https://codeload.github.com/frazse/Terra-Addons/zip/refs/heads/main";
const CATALOG_CACHE_TTL: Duration = Duration::from_secs(90);
const MAX_ZIP_FILES: usize = 500;
const MAX_ZIP_FILE_BYTES: usize = 10 * 1024 * 1024;
const MAX_ZIP_TOTAL_BYTES: usize = 25 * 1024 * 1024;

#[derive(Debug)]
struct ZipEntry {
    path: String,
    data: Vec<u8>,
}

fn le_u16(b: &[u8], off: usize) -> Result<u16, String> {
    b.get(off..off + 2)
        .map(|s| u16::from_le_bytes([s[0], s[1]]))
        .ok_or_else(|| "truncated zip header".into())
}

fn le_u32(b: &[u8], off: usize) -> Result<u32, String> {
    b.get(off..off + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(|| "truncated zip header".into())
}

fn read_zip_entries(zip: &[u8]) -> Result<Vec<ZipEntry>, String> {
    use std::io::Read;
    if zip.len() < 22 {
        return Err("file is too small to be a zip archive".into());
    }
    let min_off = zip.len().saturating_sub(65535 + 22);
    let mut eocd = None;
    let mut i = zip.len() - 22;
    loop {
        if &zip[i..i + 4] == b"PK\x05\x06" {
            let cmt_len = u16::from_le_bytes([zip[i + 20], zip[i + 21]]) as usize;
            if i + 22 + cmt_len == zip.len() {
                eocd = Some(i);
                break;
            }
        }
        if i == min_off {
            break;
        }
        i -= 1;
    }
    let eocd_off = eocd.ok_or("invalid zip archive (missing end-of-central-directory)")?;
    let entry_count = le_u16(zip, eocd_off + 10)? as usize;
    let cd_size = le_u32(zip, eocd_off + 12)? as usize;
    let cd_offset = le_u32(zip, eocd_off + 16)? as usize;
    if entry_count == 0xFFFF || cd_size == 0xFFFFFFFF || cd_offset == 0xFFFFFFFF {
        return Err("zip64 archives are not supported".into());
    }
    if entry_count > MAX_ZIP_FILES {
        return Err("zip archive contains too many files".into());
    }
    let cd_end = cd_offset.checked_add(cd_size).ok_or("corrupt central directory")?;
    if cd_end > eocd_off {
        return Err("corrupt central directory bounds".into());
    }

    let mut pos = cd_offset;
    let mut total_bytes: usize = 0;
    let mut out = Vec::new();

    for _ in 0..entry_count {
        if pos + 46 > cd_end || &zip[pos..pos + 4] != b"PK\x01\x02" {
            return Err("corrupt zip central directory entry".into());
        }
        let method = le_u16(zip, pos + 10)?;
        let expected_crc = le_u32(zip, pos + 16)?;
        let comp_size = le_u32(zip, pos + 20)? as usize;
        let uncomp_size = le_u32(zip, pos + 24)? as usize;
        let name_len = le_u16(zip, pos + 28)? as usize;
        let extra_len = le_u16(zip, pos + 30)? as usize;
        let comment_len = le_u16(zip, pos + 32)? as usize;
        let local_off = le_u32(zip, pos + 42)? as usize;

        let name_start = pos + 46;
        let name_end = name_start + name_len;
        let next_pos = name_end + extra_len + comment_len;
        if next_pos > cd_end {
            return Err("truncated zip central directory name".into());
        }
        let raw_name = std::str::from_utf8(&zip[name_start..name_end])
            .map_err(|_| "non-UTF8 filename in zip archive")?;
        let norm_name = raw_name.replace('\\', "/");
        pos = next_pos;

        if norm_name.ends_with('/') {
            continue;
        }
        if norm_name.contains('\0') || norm_name.starts_with('/') || !safe_rel(&norm_name) {
            return Err(format!("unsafe path in zip: '{norm_name}'"));
        }
        if uncomp_size > MAX_ZIP_FILE_BYTES {
            return Err(format!("file '{norm_name}' exceeds size limit"));
        }
        total_bytes = total_bytes
            .checked_add(uncomp_size)
            .filter(|t| *t <= MAX_ZIP_TOTAL_BYTES)
            .ok_or("zip archive uncompressed size exceeds limit")?;

        if local_off + 30 > zip.len() || &zip[local_off..local_off + 4] != b"PK\x03\x04" {
            return Err(format!("corrupt local header for '{norm_name}'"));
        }
        let l_name_len = le_u16(zip, local_off + 26)? as usize;
        let l_extra_len = le_u16(zip, local_off + 28)? as usize;
        let data_start = local_off + 30 + l_name_len + l_extra_len;
        let data_end = data_start.checked_add(comp_size).ok_or("truncated zip data")?;
        if data_end > zip.len() {
            return Err(format!("truncated data for '{norm_name}'"));
        }
        let comp = &zip[data_start..data_end];
        let data = match method {
            0 => {
                if comp.len() != uncomp_size {
                    return Err(format!("size mismatch for '{norm_name}'"));
                }
                comp.to_vec()
            }
            8 => {
                let mut dec = flate2::read::DeflateDecoder::new(comp).take((MAX_ZIP_FILE_BYTES + 1) as u64);
                let mut buf = Vec::with_capacity(uncomp_size.min(1024 * 1024));
                dec.read_to_end(&mut buf).map_err(|e| format!("deflate error in '{norm_name}': {e}"))?;
                if buf.len() != uncomp_size {
                    return Err(format!("uncompressed size mismatch in '{norm_name}'"));
                }
                buf
            }
            other => return Err(format!("unsupported zip compression method {other} for '{norm_name}'")),
        };
        if crc32fast::hash(&data) != expected_crc {
            return Err(format!("CRC mismatch in '{norm_name}'"));
        }
        out.push(ZipEntry { path: norm_name, data });
    }
    Ok(out)
}

fn catalog_http_client() -> Result<&'static reqwest::Client, String> {
    static CLIENT: std::sync::OnceLock<Result<reqwest::Client, String>> = std::sync::OnceLock::new();
    match CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::limited(5))
            .user_agent("Terra-Addons-Installer")
            .build()
            .map_err(err)
    }) {
        Ok(c) => Ok(c),
        Err(e) => Err(e.clone()),
    }
}

async fn fetch_github_zip(store: &AddonStore, refresh: bool) -> Result<Vec<u8>, String> {
    if !refresh {
        if let Ok(guard) = store.catalog_cache.lock() {
            if let Some((at, bytes)) = &*guard {
                if at.elapsed() < CATALOG_CACHE_TTL {
                    return Ok(bytes.clone());
                }
            }
        }
    }
    let resp = catalog_http_client()?
        .get(GITHUB_ADDONS_ZIP_URL)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(err)?;
    if !resp.status().is_success() {
        return Err(format!("GitHub returned HTTP {}", resp.status()));
    }
    let bytes = resp.bytes().await.map_err(err)?.to_vec();
    if bytes.len() > MAX_ZIP_TOTAL_BYTES {
        return Err("downloaded archive exceeds size limit".into());
    }
    if let Ok(mut guard) = store.catalog_cache.lock() {
        *guard = Some((std::time::Instant::now(), bytes.clone()));
    }
    Ok(bytes)
}

fn is_newer_version(remote: &str, local: &str) -> bool {
    let parse = |s: &str| -> Option<Vec<u64>> {
        let s = s.trim().trim_start_matches('v');
        let parts: Vec<&str> = s.split('.').collect();
        if parts.is_empty() {
            return None;
        }
        let mut nums = Vec::with_capacity(parts.len());
        for p in parts {
            let head = p.split(['-', '+']).next().unwrap_or(p);
            nums.push(head.parse::<u64>().ok()?);
        }
        Some(nums)
    };
    match (parse(remote), parse(local)) {
        (Some(mut r), Some(mut l)) => {
            let len = r.len().max(l.len());
            r.resize(len, 0);
            l.resize(len, 0);
            r > l
        }
        _ => remote.trim() != local.trim(),
    }
}

fn install_addon_files(target_root: &Path, id: &str, files: &[(String, Vec<u8>)]) -> Result<(String, String), String> {
    if !valid_id(id) {
        return Err(format!("invalid addon id '{id}'"));
    }
    let manifest_bytes = files
        .iter()
        .find(|(p, _)| p == "manifest.json")
        .map(|(_, d)| d)
        .ok_or_else(|| format!("addon '{id}' is missing manifest.json"))?;
    let m: Manifest = serde_json::from_slice(manifest_bytes).map_err(|e| format!("manifest.json: {e}"))?;
    let manifest_id = m.id.unwrap_or_else(|| id.to_string());
    if manifest_id != id && !valid_id(&manifest_id) {
        return Err(format!("invalid manifest id '{manifest_id}'"));
    }
    let install_id = manifest_id;
    let name = m.name.unwrap_or_else(|| install_id.clone());
    let version = m.version.unwrap_or_else(|| "0.0.0".into());

    fs::create_dir_all(target_root).map_err(err)?;
    let stamp = format!("{}-{}", std::process::id(), std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0));
    let stage = target_root.join(format!(".tmp-{install_id}-{stamp}"));
    let _ = fs::remove_dir_all(&stage);
    fs::create_dir_all(&stage).map_err(err)?;

    let write_res: Result<(), String> = (|| {
        for (rel, bytes) in files {
            if !safe_rel(rel) || rel.is_empty() {
                return Err(format!("unsafe file path '{rel}'"));
            }
            let dest = stage.join(rel);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent).map_err(err)?;
            }
            fs::write(&dest, bytes).map_err(err)?;
        }
        load_one(&stage, &install_id, true).map(|_| ())
    })();

    if let Err(e) = write_res {
        let _ = fs::remove_dir_all(&stage);
        return Err(e);
    }

    let dest = target_root.join(&install_id);
    let backup = target_root.join(format!(".bak-{install_id}-{stamp}"));
    let had_old = dest.exists();
    if had_old {
        fs::rename(&dest, &backup).map_err(|e| {
            let _ = fs::remove_dir_all(&stage);
            err(e)
        })?;
    }
    if let Err(e) = fs::rename(&stage, &dest) {
        let _ = fs::remove_dir_all(&stage);
        if had_old {
            let _ = fs::rename(&backup, &dest);
        }
        return Err(err(e));
    }
    if had_old {
        let _ = fs::remove_dir_all(&backup);
    }
    Ok((name, version))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogItem {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub category: String,
    pub matches: Vec<String>,
    pub default_enabled: bool,
    pub installed_version: Option<String>,
    pub update_available: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledAddon {
    pub id: String,
    pub name: String,
    pub version: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UninstallResult {
    pub removed: bool,
    pub still_present: bool,
}

#[cfg_attr(not(target_os = "linux"), tauri::command)]
pub async fn addons_catalog(
    app: AppHandle,
    store: State<'_, AddonStore>,
    refresh: Option<bool>,
) -> Result<Vec<CatalogItem>, String> {
    let zip_bytes = fetch_github_zip(&store, refresh.unwrap_or(false)).await?;
    let entries = read_zip_entries(&zip_bytes)?;
    let local: HashMap<String, String> = load_manifests(&app)
        .into_iter()
        .map(|a| (a.id, a.version))
        .collect();

    let mut items = Vec::new();
    for e in &entries {
        let parts: Vec<&str> = e.path.split('/').collect();
        if parts.len() != 3 || parts[2] != "manifest.json" {
            continue;
        }
        let folder = parts[1];
        if folder.starts_with('.') || folder.starts_with('_') {
            continue;
        }
        let Ok(m) = serde_json::from_slice::<Manifest>(&e.data) else { continue };
        let id = m.id.unwrap_or_else(|| folder.to_string());
        if !valid_id(&id) {
            continue;
        }
        let name = m.name.unwrap_or_else(|| id.clone());
        let version = m.version.unwrap_or_else(|| "0.0.0".into());
        let category = infer_category(&id, &name, m.category.as_deref());
        let installed_version = local.get(&id).cloned();
        let update_available = installed_version
            .as_deref()
            .map_or(false, |lv| is_newer_version(&version, lv));
        items.push(CatalogItem {
            id,
            name,
            version,
            description: m.description.unwrap_or_default(),
            category,
            matches: m.matches,
            default_enabled: m.enabled,
            installed_version,
            update_available,
        });
    }
    items.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(items)
}

#[cfg_attr(not(target_os = "linux"), tauri::command)]
pub async fn addons_install_github(
    app: AppHandle,
    store: State<'_, AddonStore>,
    id: String,
) -> Result<InstalledAddon, String> {
    if !valid_id(&id) {
        return Err("invalid addon id".into());
    }
    let zip_bytes = fetch_github_zip(&store, false).await?;
    let entries = read_zip_entries(&zip_bytes)?;

    let mut target_prefix: Option<String> = None;
    for e in &entries {
        let parts: Vec<&str> = e.path.split('/').collect();
        if parts.len() == 3 && parts[2] == "manifest.json" {
            let folder = parts[1];
            if let Ok(m) = serde_json::from_slice::<Manifest>(&e.data) {
                let mid = m.id.unwrap_or_else(|| folder.to_string());
                if mid == id || folder == id {
                    target_prefix = Some(format!("{}/{}/", parts[0], folder));
                    break;
                }
            }
        }
    }
    let prefix = target_prefix.ok_or_else(|| format!("addon '{id}' not found in GitHub catalog"))?;
    let files: Vec<(String, Vec<u8>)> = entries
        .into_iter()
        .filter_map(|e| e.path.strip_prefix(&prefix).map(|rel| (rel.to_string(), e.data)))
        .collect();

    let target_root = writable_addons_dir(&app);
    let (name, version) = install_addon_files(&target_root, &id, &files)?;
    Ok(InstalledAddon { id, name, version })
}

#[cfg_attr(not(target_os = "linux"), tauri::command)]
pub async fn addons_install_zip(
    app: AppHandle,
    base64: String,
    filename: Option<String>,
) -> Result<Vec<InstalledAddon>, String> {
    if base64.len() > 20 * 1024 * 1024 {
        return Err("zip file is too large (max ~15 MB)".into());
    }
    let zip_bytes = B64.decode(base64.trim()).map_err(|e| format!("invalid base64: {e}"))?;
    let entries = read_zip_entries(&zip_bytes)?;
    let clean_entries: Vec<ZipEntry> = entries
        .into_iter()
        .filter(|e| {
            let top = e.path.split('/').next().unwrap_or("");
            top != "__MACOSX" && top != ".git" && !e.path.ends_with("/.DS_Store") && e.path != ".DS_Store"
        })
        .collect();

    let target_root = writable_addons_dir(&app);
    let mut installed = Vec::new();

    // Case 1: root manifest.json -> entire zip is one addon
    if let Some(m_entry) = clean_entries.iter().find(|e| e.path == "manifest.json") {
        let m: Manifest = serde_json::from_slice(&m_entry.data).map_err(|e| format!("manifest.json: {e}"))?;
        let fallback_id = filename
            .as_deref()
            .and_then(|f| Path::new(f).file_stem().and_then(|s| s.to_str()))
            .unwrap_or("custom-addon")
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') { c } else { '-' })
            .collect::<String>();
        let id = m.id.unwrap_or(fallback_id);
        let files: Vec<(String, Vec<u8>)> = clean_entries.into_iter().map(|e| (e.path, e.data)).collect();
        let (name, version) = install_addon_files(&target_root, &id, &files)?;
        installed.push(InstalledAddon { id, name, version });
        return Ok(installed);
    }

    // Case 2: <folder>/manifest.json (single or multi-addon zip)
    let mut folders: HashMap<String, Vec<(String, Vec<u8>)>> = HashMap::new();
    for e in &clean_entries {
        if let Some((top, rest)) = e.path.split_once('/') {
            if !top.starts_with('.') && !top.starts_with('_') {
                folders.entry(top.to_string()).or_default().push((rest.to_string(), e.data.clone()));
            }
        }
    }
    let mut depth1_addons: Vec<(String, Vec<(String, Vec<u8>)>)> = folders
        .iter()
        .filter(|(_, files)| files.iter().any(|(p, _)| p == "manifest.json"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    depth1_addons.sort_by(|a, b| a.0.cmp(&b.0));

    if !depth1_addons.is_empty() {
        for (folder, files) in depth1_addons {
            let m_bytes = &files.iter().find(|(p, _)| p == "manifest.json").unwrap().1;
            let m: Manifest = serde_json::from_slice(m_bytes).map_err(|e| format!("{folder}/manifest.json: {e}"))?;
            let id = m.id.unwrap_or(folder);
            let (name, version) = install_addon_files(&target_root, &id, &files)?;
            installed.push(InstalledAddon { id, name, version });
        }
        return Ok(installed);
    }

    // Case 3: single wrapper directory <repo>/<addon>/manifest.json (e.g. GitHub repo zip)
    if folders.len() == 1 {
        let (_, sub_entries) = folders.into_iter().next().unwrap();
        let mut sub_folders: HashMap<String, Vec<(String, Vec<u8>)>> = HashMap::new();
        for (p, data) in sub_entries {
            if let Some((sub, rest)) = p.split_once('/') {
                if !sub.starts_with('.') && !sub.starts_with('_') {
                    sub_folders.entry(sub.to_string()).or_default().push((rest.to_string(), data));
                }
            }
        }
        let mut depth2_addons: Vec<(String, Vec<(String, Vec<u8>)>)> = sub_folders
            .into_iter()
            .filter(|(_, files)| files.iter().any(|(p, _)| p == "manifest.json"))
            .collect();
        depth2_addons.sort_by(|a, b| a.0.cmp(&b.0));
        for (folder, files) in depth2_addons {
            let m_bytes = &files.iter().find(|(p, _)| p == "manifest.json").unwrap().1;
            let m: Manifest = serde_json::from_slice(m_bytes).map_err(|e| format!("{folder}/manifest.json: {e}"))?;
            let id = m.id.unwrap_or(folder);
            let (name, version) = install_addon_files(&target_root, &id, &files)?;
            installed.push(InstalledAddon { id, name, version });
        }
    }

    if installed.is_empty() {
        return Err("No valid addon (manifest.json) found in zip archive".into());
    }
    Ok(installed)
}

#[cfg_attr(not(target_os = "linux"), tauri::command)]
pub async fn addons_uninstall(app: AppHandle, id: String) -> Result<UninstallResult, String> {
    if !valid_id(&id) {
        return Err("invalid addon id".into());
    }
    let user_dir = writable_addons_dir(&app).join(&id);
    let removed = if user_dir.exists() {
        fs::remove_dir_all(&user_dir).map_err(err)?;
        true
    } else {
        false
    };
    let still_present = load_manifests(&app).iter().any(|a| a.id == id);
    Ok(UninstallResult { removed, still_present })
}

// ---------- terra.http: requests that bypass the page's CORS rules (what GM_xmlhttpRequest did) ----------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpReq {
    method: Option<String>,
    url: String,
    #[serde(default)]
    headers: HashMap<String, String>,
    body: Option<String>,
    basic: Option<BasicAuth>,
    form: Option<FormSpec>,
        timeout_ms: Option<u64>,
}

#[derive(Deserialize)]
pub struct BasicAuth {
    user: String,
    password: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormSpec {
    #[serde(default)]
    fields: HashMap<String, String>,
    file: FilePart,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilePart {
    field: String,
    filename: String,
    content_type: Option<String>,
    base64: String,
}

#[derive(Serialize)]
pub struct HttpResp {
    status: u16,
    headers: HashMap<String, Vec<String>>,
    body: String,
}

fn http_client() -> Result<&'static reqwest::Client, String> {
    static CLIENT: std::sync::OnceLock<Result<reqwest::Client, String>> = std::sync::OnceLock::new();
    match CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none()) // a redirect could leave the allowed hosts
            .build()
            .map_err(err)
    }) {
        Ok(c) => Ok(c),
        Err(e) => Err(e.clone()),
    }
}

/// https only, and only to hosts in the addon's manifest `connect` list. `{{setting:KEY}}` placeholders in the
/// url, header values, body, basic auth and form fields are filled in here from the addon's settings.
#[cfg_attr(not(target_os = "linux"), tauri::command)]
pub async fn addon_http(app: AppHandle, store: State<'_, AddonStore>, addon: String, req: HttpReq) -> Result<HttpResp, String> {
    let addons = load_manifests(&app);
    let a = addons
    .iter()
    .find(|a| a.id == addon && a.enabled)
    .ok_or("unknown or disabled addon")?;
    let ex = |s: &str| store.expand(&addon, &a.settings, s);

    let url = reqwest::Url::parse(&ex(&req.url)?).map_err(err)?;
    if url.scheme() != "https" {
        return Err("only https URLs are allowed".into());
    }
    let host = url.host_str().unwrap_or("").to_string();
    if !a.connect.iter().any(|c| host_matches(c, &host)) {
        return Err(format!("host '{host}' is not in this addon's \"connect\" list"));
    }

    let method = reqwest::Method::from_bytes(req.method.as_deref().unwrap_or("GET").to_uppercase().as_bytes()).map_err(err)?;
    let client = http_client()?;
    let mut rb = client
        .request(method, url)
        .timeout(Duration::from_millis(req.timeout_ms.unwrap_or(60_000)));
    for (k, v) in &req.headers {
        rb = rb.header(k.as_str(), ex(v)?);
    }
    if let Some(b) = &req.basic {
        rb = rb.basic_auth(ex(&b.user)?, Some(ex(&b.password)?));
    }
    if let Some(f) = &req.form {
        let mut form = reqwest::multipart::Form::new();
        for (k, v) in &f.fields {
            form = form.text(k.clone(), ex(v)?);
        }
        let bytes = B64.decode(&f.file.base64).map_err(err)?;
        let mut part = reqwest::multipart::Part::bytes(bytes).file_name(f.file.filename.clone());
        if let Some(ct) = &f.file.content_type {
            part = part.mime_str(ct).map_err(err)?;
        }
        rb = rb.multipart(form.part(f.file.field.clone(), part));
    } else if let Some(body) = &req.body {
        rb = rb.body(ex(body)?);
    }

    let resp = rb.send().await.map_err(err)?;
    let status = resp.status().as_u16();
    let mut headers: HashMap<String, Vec<String>> = HashMap::new();
    for (k, v) in resp.headers().iter() {
        if let Ok(val_str) = v.to_str() {
            headers
            .entry(k.as_str().to_lowercase())
            .or_default()
            .push(val_str.to_string());
        }
    }
    let body = resp.text().await.map_err(err)?;
    Ok(HttpResp { status, headers, body })
}

// ---------- terra.files: save a file to Downloads/Terra (the webview can't do <a download>) ----------

#[cfg_attr(not(target_os = "linux"), tauri::command)]
pub async fn addon_save_file(app: AppHandle, addon: String, filename: String, base64: String) -> Result<String, String> {
    if !load_manifests(&app).iter().any(|a| a.id == addon && a.enabled) {
        return Err("unknown or disabled addon".into());
    }
    let clean: String = filename
    .chars()
    .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' })
    .collect();
    let clean = clean.trim_start_matches('.').to_string(); // no hidden files
    let ext = Path::new(&clean)
    .extension()
    .and_then(|e| e.to_str())
    .unwrap_or("")
    .to_lowercase();
    if clean.is_empty() || clean.len() > 120 || !["fit", "gpx", "tcx", "json", "csv", "txt"].contains(&ext.as_str()) {
        return Err("file name or type not allowed".into());
    }
    let bytes = B64.decode(base64).map_err(err)?;
    let dir = app
    .path()
    .download_dir()
    .or_else(|_| app.path().home_dir())
    .map_err(err)?
    .join("Terra");
    fs::create_dir_all(&dir).map_err(err)?;
    let stem = Path::new(&clean).file_stem().and_then(|s| s.to_str()).unwrap_or("file").to_string();
    let mut path = dir.join(&clean);
    let mut n = 1;
    while path.exists() {
        path = dir.join(format!("{stem}-{n}.{ext}"));
        n += 1;
    }
    fs::write(&path, bytes).map_err(err)?;
    Ok(path.display().to_string())
}

// ---------- terra.udp: fire-and-forget UDP datagram (e.g. a bike computer on the local Wi-Fi) ----------

async fn udp_socket(ipv4: bool) -> Result<&'static tokio::net::UdpSocket, String> {
    static V4: tokio::sync::OnceCell<tokio::net::UdpSocket> = tokio::sync::OnceCell::const_new();
    static V6: tokio::sync::OnceCell<tokio::net::UdpSocket> = tokio::sync::OnceCell::const_new();
    if ipv4 {
        V4.get_or_try_init(|| async { tokio::net::UdpSocket::bind("0.0.0.0:0").await.map_err(err) })
            .await
    } else {
        V6.get_or_try_init(|| async { tokio::net::UdpSocket::bind("[::]:0").await.map_err(err) })
            .await
    }
}

/// Send a single UDP datagram to `host:port` (or `hostname:port`). `{{setting:KEY}}` placeholders in
/// `target` are expanded from the addon's settings. Resolves to the number of bytes sent.
#[cfg_attr(not(target_os = "linux"), tauri::command)]
pub async fn addon_udp_send(
    app: AppHandle,
    store: State<'_, AddonStore>,
    addon: String,
    target: String,
    payload: String,
) -> Result<u32, String> {
    if payload.len() > 65_507 {
        return Err("payload exceeds the UDP datagram limit (65507 bytes)".into());
    }
    let addons = load_manifests(&app);
    let a = addons
        .iter()
        .find(|a| a.id == addon && a.enabled)
        .ok_or("unknown or disabled addon")?;
    let expanded = store.expand(&addon, &a.settings, &target)?;
    let addr: std::net::SocketAddr = match expanded.parse() {
        Ok(a) => a,
        Err(_) => {
            // Not an IP literal: resolve the hostname (tokio's lookup_host accepts "host:port" too).
            let mut it = tokio::net::lookup_host(&expanded).await.map_err(err)?;
            it.next().ok_or_else(|| format!("could not resolve target '{expanded}'"))?
        }
    };
    let socket = udp_socket(addr.is_ipv4()).await?;
    let sent = socket.send_to(payload.as_bytes(), addr).await.map_err(err)?;
    Ok(sent as u32)
}

// ---------- terra.tcp: persistent TCP stream (e.g. a Wi-Fi trainer speaking a binary GATT protocol) ----------
//
// Unlike `udp.send` (fire-and-forget datagrams), a connection stays open: the addon connects once,
// pushes raw frames with `send`, and receives inbound bytes as `tcp:rx:<addon>` bus events.
// Framing is left to the addon — the core only moves bytes.

/// One open connection: its write queue plus ownership so an addon can't use another addon's socket.
#[derive(Clone)]
struct TcpConn {
    id: String,
    addon: String,
    peer: String, // resolved "a.b.c.d:port", for diagnostics
    tx: tokio::sync::mpsc::Sender<Vec<u8>>,
    /// Handle to the reader task. An explicit close must end it too — the read
    /// half otherwise sits parked in `read()` forever, the socket never fully
    /// closes, and single-session peers (e.g. the Wi-Fi Kickr) keep their
    /// session slot latched, refusing every later connect.
    reader: tokio::task::AbortHandle,
}

#[derive(Clone)]
pub struct TcpRegistry(Arc<Inner>);

struct Inner {
    next: AtomicUsize,
    conns: tokio::sync::Mutex<HashMap<String, TcpConn>>,
}

impl TcpRegistry {
    pub fn new() -> Self {
        Self(Arc::new(Inner {
            next: AtomicUsize::new(1),
            conns: tokio::sync::Mutex::new(HashMap::new()),
        }))
    }
    async fn get(&self, id: &str) -> Option<TcpConn> {
        self.0.conns.lock().await.get(id).cloned()
    }
    async fn insert(&self, conn: TcpConn) {
        self.0.conns.lock().await.insert(conn.id.clone(), conn);
    }
    /// Remove a connection; the first caller (read EOF, write failure, or explicit close)
    /// is the one that emits the `closed` event.
    async fn close(&self, id: &str) -> Option<TcpConn> {
        self.0.conns.lock().await.remove(id)
    }
    async fn close_all(&self) {
        let drained: Vec<TcpConn> = self.0.conns.lock().await.drain().map(|(_, c)| c).collect();
        for c in drained {
            Self::teardown_conn(c);
        }
    }
    async fn close_for_addons(&self, addons: &[String]) {
        let mut g = self.0.conns.lock().await;
        let ids: Vec<String> = g
            .iter()
            .filter(|(_, c)| addons.contains(&c.addon))
            .map(|(k, _)| k.clone())
            .collect();
        for id in ids {
            if let Some(c) = g.remove(&id) {
                Self::teardown_conn(c);
            }
        }
    }
    fn teardown_conn(c: TcpConn) {
        tokio::spawn(async move {
            drop(c.tx);
            tokio::time::sleep(Duration::from_millis(250)).await;
            c.reader.abort();
        });
    }
}

/// Run a snippet in the page: the webview on non-Linux platforms, a stdout `js_eval`
/// event in RPC mode (Linux), where electron forwards it via executeJavaScript.
fn page_js(app: &AppHandle, code: String) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.eval(code);
        return;
    }
    if let Ok(line) = serde_json::to_string(&json!({ "event": "js_eval", "code": code })) {
        use std::io::Write;
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }
}

/// Emit a bus event the addon can subscribe to with `bus.on(topic, cb)`.
fn page_bus(app: &AppHandle, topic: &str, payload: &Value) {
    let code = format!(
        "window.__terraCore && window.__terraCore.bus.emit({:?}, {})",
        topic, payload
    );
    page_js(app, code);
}

/// The addon must exist and be enabled (same gate as the other terra.* network RPCs).
fn require_addon(app: &AppHandle, addon: &str, enabled_only: bool) -> Result<(), String> {
    let ok = load_manifests(app).iter().any(|a| a.id == addon && (!enabled_only || a.enabled));
    if ok { Ok(()) } else { Err("unknown or disabled addon".into()) }
}

/// Open a TCP connection to `host:port`; resolves to a connection id.
/// `{{setting:KEY}}` placeholders in `host` expand from the addon's settings;
/// `timeout_ms` 0 = 3 s. Inbound bytes arrive as `tcp:rx:<addon>` events `{conn, b64}`;
/// lifecycle as `tcp:state:<addon>` events `{conn, state, reason?}`.
#[cfg_attr(not(target_os = "linux"), tauri::command)]
pub async fn addon_tcp_connect(
    app: AppHandle,
    store: State<'_, AddonStore>,
    addon: String,
    host: String,
    port: u16,
    timeout_ms: u32,
) -> Result<String, String> {
    let addons = load_manifests(&app);
    let a = addons.iter().find(|a| a.id == addon && a.enabled).ok_or("unknown or disabled addon")?;
    let host = store.expand(&addon, &a.settings, &host)?;

    let addr: std::net::SocketAddr = match format!("{host}:{port}").parse() {
        Ok(a) => a,
        Err(_) => {
            let spec = format!("{host}:{port}");
            let mut it = tokio::net::lookup_host(&spec).await.map_err(err)?;
            it.next().ok_or_else(|| format!("could not resolve host '{host}'"))?
        }
    };
    let timeout = Duration::from_millis(if timeout_ms == 0 { 3000 } else { (timeout_ms as u64).min(15_000) });
    let stream = tokio::time::timeout(timeout, tokio::net::TcpStream::connect(addr))
        .await
        .map_err(|_| format!("connect to {host}:{port} timed out after {} ms", timeout.as_millis()))?
        .map_err(err)?;
    let _ = stream.set_nodelay(true);

    let reg: State<'_, TcpRegistry> = app.state();
    // Snapshot an owned registry (clone of the shared Arc) up front: on tauri,
    // `State` borrows `app`, and the spawned tasks below must not capture that
    // borrow — they only need the registry itself.
    let reg0 = (*reg).clone();
    let id = format!("tcp{}", reg0.0.next.fetch_add(1, Ordering::Relaxed));
    let peer = addr.to_string();
    let reader_addon = a.id.clone();
    let (reader, writer) = stream.into_split();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(64);

    // Reader: bytes in -> bus events; EOF/error -> `closed` event exactly once.
    // Spawned before the insert so the conn can carry the abort handle an
    // explicit close (see `addon_tcp_close`) needs.
    let reader_task = {
        let app_r = app.clone();
        let reg_r = reg0.clone();
        let id_r = id.clone();
        let addon_r = reader_addon.clone();
        tokio::spawn(async move {
            let mut r = reader;
            let mut buf = vec![0u8; 4096];
            loop {
                match r.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => page_bus(&app_r, &format!("tcp:rx:{addon_r}"), &json!({ "conn": id_r, "b64": B64.encode(&buf[..n]) })),
                    Err(e) => {
                        if let Some(c) = reg_r.close(&id_r).await {
                            page_bus(&app_r, &format!("tcp:state:{}", c.addon), &json!({ "conn": id_r, "state": "error", "reason": e.to_string() }));
                        }
                        break;
                    }
                }
            }
            if let Some(c) = reg_r.close(&id_r).await {
                page_bus(&app_r, &format!("tcp:state:{}", c.addon), &json!({ "conn": id_r, "state": "closed" }));
            }
        })
    };
    reg.insert(TcpConn { id: id.clone(), addon: a.id.clone(), peer: peer.clone(), tx, reader: reader_task.abort_handle() }).await;

    // Writer: drains the send queue onto the socket.
    {
        let app_w = app.clone();
        let reg_w = reg0.clone();
        let id_w = id.clone();
        tokio::spawn(async move {
            let mut w = writer;
            while let Some(bytes) = rx.recv().await {
                if w.write_all(&bytes).await.is_err() || w.flush().await.is_err() {
                    if let Some(c) = reg_w.close(&id_w).await {
                        // The dead socket's read half may still be parked in read() — end it.
                        c.reader.abort();
                        page_bus(&app_w, &format!("tcp:state:{}", c.addon), &json!({ "conn": id_w, "state": "error", "reason": "write failed" }));
                    }
                    break;
                }
            }
        });
    }

    page_bus(&app, &format!("tcp:state:{addon}"), &json!({ "conn": id, "state": "connected", "peer": peer }));
    Ok(id)
}

/// Write raw bytes (base64) onto connection `conn`; resolves to the byte count.
#[cfg_attr(not(target_os = "linux"), tauri::command)]
pub async fn addon_tcp_send(
    app: AppHandle,
    addon: String,
    conn: String,
    base64: String,
) -> Result<u32, String> {
    require_addon(&app, &addon, true)?;
    let reg: State<'_, TcpRegistry> = app.state();
    let c = reg.get(&conn).await.ok_or("unknown connection")?;
    if c.addon != addon {
        return Err("connection belongs to another addon".into());
    }
    let bytes = B64.decode(&base64).map_err(err)?;
    c.tx.send(bytes.clone()).await.map_err(err)?;
    Ok(bytes.len() as u32)
}

/// List this addon's open connections (diagnostics).
#[cfg_attr(not(target_os = "linux"), tauri::command)]
pub async fn addon_tcp_list(
    app: AppHandle,
    addon: String,
) -> Result<Vec<Value>, String> {
    require_addon(&app, &addon, false)?;
    let reg: State<'_, TcpRegistry> = app.state();
    let conns = reg.0.conns.lock().await;
    Ok(conns.values().filter(|c| c.addon == addon).map(|c| json!({ "conn": c.id, "peer": c.peer })).collect())
}

/// Close this addon's connection. `reg.close` drops the write side (after a
/// short grace the writer drains and its dropped half sends FIN), then the
/// reader is aborted so the read half drops too — without that the socket
/// hangs half-open forever: the reader stays parked in `read()`, the peer's
/// close never completes, and single-session peers such as the Wi-Fi Kickr
/// keep their session slot latched, refusing every later connect (os error
/// 111). Aborting is FIN-safe: the reader continuously drains, so no unread
/// data means no reset.
#[cfg_attr(not(target_os = "linux"), tauri::command)]
pub async fn addon_tcp_close(
    app: AppHandle,
    addon: String,
    conn: String,
) -> Result<(), String> {
    require_addon(&app, &addon, false)?;
    let reg: State<'_, TcpRegistry> = app.state();
    if let Some(c) = reg.close(&conn).await {
        if c.addon != addon {
            // Not ours: hand it back rather than closing a foreign socket.
            reg.insert(c).await;
            return Err("connection belongs to another addon".into());
        }
        // Grace for the writer to flush its (near-empty) queue and drop the
        // write half (FIN); LAN drain of a few small frames is <5 ms, so
        // 250 ms is a 50x margin. Then end the read side so the 4-way close
        // can complete instead of hanging on our side.
        tokio::time::sleep(Duration::from_millis(250)).await;
        c.reader.abort();
        page_bus(&app, &format!("tcp:state:{addon}"), &json!({ "conn": conn, "state": "closed" }));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(name: &str) -> (PathBuf, AddonStore) {
        let dir = std::env::temp_dir().join(format!("terra-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let store = AddonStore::new(dir.clone());
        (dir, store)
    }

    #[test]
    fn glob_matching() {
        assert!(glob("https://biketerra.com/*", "https://biketerra.com/ride/123"));
        assert!(glob("*://biketerra.com/ride*", "https://biketerra.com/ride"));
        assert!(!glob("https://biketerra.com/ride*", "https://biketerra.com/routes"));
        assert!(glob("*", "anything"));
        assert!(glob("", ""));
        assert!(!glob("", "a"));
    }

    #[test]
    fn host_matching() {
        assert!(host_matches("api.example.com", "API.EXAMPLE.COM"));
        assert!(!host_matches("example.com", "sub.example.com"));
        assert!(host_matches("*.example.com", "sub.example.com"));
        assert!(host_matches("*.example.com", "a.b.example.com"));
        assert!(!host_matches("*.example.com", "example.com"));
        assert!(!host_matches("*.example.com", "badexample.com"));
    }

    #[test]
    fn id_and_path_validation() {
        assert!(valid_id("garmin-hud"));
        assert!(valid_id("addon_1.0"));
        assert!(!valid_id(""));
        assert!(!valid_id(".hidden"));
        assert!(!valid_id("../escape"));
        assert!(!valid_id("a/b"));
        assert!(!valid_id(&"a".repeat(65)));

        assert!(safe_rel("main.js"));
        assert!(safe_rel("sub/style.css"));
        assert!(!safe_rel("../secret.js"));
        assert!(!safe_rel("/etc/passwd"));
    }

    #[test]
    fn expand_settings_with_defaults_and_secrets() {
        let (dir, store) = temp_store("expand");
        let defs = vec![
            SettingDef {
                key: "host".into(),
                label: "Host".into(),
                kind: "text".into(),
                description: String::new(),
                default: Some(json!("api.example.com")),
                options: vec![],
            },
            SettingDef {
                key: "apiKey".into(),
                label: "API Key".into(),
                kind: "password".into(),
                description: String::new(),
                default: None,
                options: vec![],
            },
        ];

        store
            .settings_access("test-addon", true, |m| {
                m.insert("apiKey".into(), json!("secret-123"));
            })
            .unwrap();

        let expanded = store
            .expand("test-addon", &defs, "https://{{setting:host}}/v1?k={{setting:apiKey}}")
            .unwrap();
        assert_eq!(expanded, "https://api.example.com/v1?k=secret-123");

        assert!(store.expand("test-addon", &defs, "{{setting:unknown}}").is_err());
        assert!(store.expand("test-addon", &defs, "{{setting:host").is_err());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn version_comparison_and_categories() {
        assert!(is_newer_version("1.4.0", "1.3.0"));
        assert!(is_newer_version("v0.10.0", "0.9.5"));
        assert!(!is_newer_version("1.3.0", "1.3.0"));
        assert!(!is_newer_version("1.2.9", "1.3.0"));

        assert_eq!(infer_category("garmin-hud", "Garmin HUD", None), "HUD");
        assert_eq!(infer_category("zwift-ride", "Zwift Ride", None), "Hardware");
        assert_eq!(infer_category("pip-cam", "PiP Cam", None), "Camera");
        assert_eq!(infer_category("fit-export", "FIT Export", None), "Export");
        assert_eq!(infer_category("custom", "Custom", Some("Hardware")), "Hardware");
    }

    #[test]
    fn zip_parse_and_install_roundtrip() {
        use flate2::{write::DeflateEncoder, Compression};
        use std::io::Write;

        // Build a valid minimal zip in memory containing "my-addon/manifest.json" and "my-addon/main.js"
        let manifest = br#"{"id":"my-addon","name":"My Addon","version":"1.2.3","matches":["https://biketerra.com/*"],"js":["main.js"]}"#;
        let main_js = b"terra.log('hello');";

        let files: &[(&str, &[u8], bool)] = &[
            ("my-addon/manifest.json", manifest, true),
            ("my-addon/main.js", main_js, false),
        ];

        let mut zip = Vec::new();
        let mut cd = Vec::new();
        for (name, data, deflate) in files {
            let local_off = zip.len() as u32;
            let crc = crc32fast::hash(data);
            let (method, comp): (u16, Vec<u8>) = if *deflate {
                let mut enc = DeflateEncoder::new(Vec::new(), Compression::default());
                enc.write_all(data).unwrap();
                (8, enc.finish().unwrap())
            } else {
                (0, data.to_vec())
            };
            let nb = name.as_bytes();
            // Local file header
            zip.extend_from_slice(b"PK\x03\x04");
            zip.extend_from_slice(&20u16.to_le_bytes());
            zip.extend_from_slice(&0u16.to_le_bytes());
            zip.extend_from_slice(&method.to_le_bytes());
            zip.extend_from_slice(&0u16.to_le_bytes());
            zip.extend_from_slice(&0u16.to_le_bytes());
            zip.extend_from_slice(&crc.to_le_bytes());
            zip.extend_from_slice(&(comp.len() as u32).to_le_bytes());
            zip.extend_from_slice(&(data.len() as u32).to_le_bytes());
            zip.extend_from_slice(&(nb.len() as u16).to_le_bytes());
            zip.extend_from_slice(&0u16.to_le_bytes());
            zip.extend_from_slice(nb);
            zip.extend_from_slice(&comp);

            // Central directory header
            cd.extend_from_slice(b"PK\x01\x02");
            cd.extend_from_slice(&20u16.to_le_bytes());
            cd.extend_from_slice(&20u16.to_le_bytes());
            cd.extend_from_slice(&0u16.to_le_bytes());
            cd.extend_from_slice(&method.to_le_bytes());
            cd.extend_from_slice(&0u16.to_le_bytes());
            cd.extend_from_slice(&0u16.to_le_bytes());
            cd.extend_from_slice(&crc.to_le_bytes());
            cd.extend_from_slice(&(comp.len() as u32).to_le_bytes());
            cd.extend_from_slice(&(data.len() as u32).to_le_bytes());
            cd.extend_from_slice(&(nb.len() as u16).to_le_bytes());
            cd.extend_from_slice(&0u16.to_le_bytes());
            cd.extend_from_slice(&0u16.to_le_bytes());
            cd.extend_from_slice(&0u16.to_le_bytes());
            cd.extend_from_slice(&0u16.to_le_bytes());
            cd.extend_from_slice(&0u32.to_le_bytes());
            cd.extend_from_slice(&local_off.to_le_bytes());
            cd.extend_from_slice(nb);
        }
        let cd_off = zip.len() as u32;
        let cd_len = cd.len() as u32;
        zip.extend_from_slice(&cd);
        // EOCD
        zip.extend_from_slice(b"PK\x05\x06");
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&(files.len() as u16).to_le_bytes());
        zip.extend_from_slice(&(files.len() as u16).to_le_bytes());
        zip.extend_from_slice(&cd_len.to_le_bytes());
        zip.extend_from_slice(&cd_off.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());

        let parsed = read_zip_entries(&zip).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].data, manifest);
        assert_eq!(parsed[1].data, main_js);

        let dir = std::env::temp_dir().join(format!("terra-test-zip-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let install_files: Vec<(String, Vec<u8>)> = parsed
            .into_iter()
            .map(|e| (e.path.strip_prefix("my-addon/").unwrap().to_string(), e.data))
            .collect();
        let (name, version) = install_addon_files(&dir, "my-addon", &install_files).unwrap();
        assert_eq!(name, "My Addon");
        assert_eq!(version, "1.2.3");
        assert!(dir.join("my-addon/main.js").is_file());
        let _ = fs::remove_dir_all(&dir);
    }
}
