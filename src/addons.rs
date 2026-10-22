//! Addon system: scans addon folders, evaluates matching addons inside the biketerra webview
//! (via webview.eval, so the page's CSP cannot block them), and stores per-addon data.
//! Addons run with the page's privileges, like userscripts: only install ones you trust.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

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
    /// text | password | number | checkbox
    #[serde(default = "text_kind", rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub default: Option<Value>,
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
            match load_one(&path, &folder) {
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

fn load_one(path: &Path, folder: &str) -> Result<Option<Addon>, String> {
    let raw = fs::read_to_string(path.join("manifest.json")).map_err(|e| format!("manifest.json: {e}"))?;
    let m: Manifest = serde_json::from_str(&raw).map_err(|e| format!("manifest.json: {e}"))?;
    let id = m.id.unwrap_or_else(|| folder.to_string());
    if !valid_id(&id) {
        return Err(format!("invalid id '{id}'"));
    }
    let js = read_files(path, &m.js)?.join("\n;\n");
    let css = read_files(path, &m.css)?;
    let settings: Vec<SettingDef> = m.settings.into_iter().filter(|d| valid_id(&d.key)).collect();
    Ok(Some(Addon {
        name: m.name.unwrap_or_else(|| id.clone()),
            version: m.version.unwrap_or_else(|| "0.0.0".into()),
            description: m.description.unwrap_or_default(),
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
    for id in active {
        if !addons.iter().any(|a| a.id == id && a.enabled && a.hits(&url)) {
            res.stop.push(id);
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
    matches: Vec<String>,
    enabled: bool,
    default_enabled: bool,
        settings: Vec<SettingDef>,
        connect: Vec<String>,
}

#[derive(Serialize)]
pub struct AddonList {
    version: String,
    dirs: Vec<String>,
    addons: Vec<AddonInfo>,
}

#[cfg_attr(not(target_os = "linux"), tauri::command)]
pub async fn addons_list(app: AppHandle) -> AddonList {
    AddonList {
        version: app.package_info().version.to_string(),
        dirs: addon_dirs(&app).iter().map(|d| d.display().to_string()).collect(),
        addons: load_all(&app)
        .into_iter()
        .map(|a| AddonInfo {
            id: a.id,
            name: a.name,
            version: a.version,
            description: a.description,
            matches: a.matches,
            enabled: a.enabled,
            default_enabled: a.default_enabled,
                settings: a.settings,
                connect: a.connect,
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
    let addons = load_all(&app);
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

/// An empty string or null removes the value (the manifest default applies again).
#[cfg_attr(not(target_os = "linux"), tauri::command)]
pub async fn addon_settings_set(
    app: AppHandle,
    store: State<'_, AddonStore>,
    id: String,
    key: String,
    value: Value,
) -> Result<(), String> {
    let addons = load_all(&app);
    let a = addons.iter().find(|a| a.id == id).ok_or("unknown addon")?;
    let d = a
    .settings
    .iter()
    .find(|d| d.key == key)
    .ok_or_else(|| format!("unknown setting '{key}'"))?;
    let v: Option<Value> = match (d.kind.as_str(), &value) {
        (_, Value::Null) => None,
        (_, Value::String(s)) if s.is_empty() => None,
        ("number", Value::String(s)) => {
            let n = s.trim().parse::<f64>().map_err(|_| "not a number")?;
            if !n.is_finite() {
                return Err("not a number".into());
            }
            Some(json!(n))
        }
        ("number", Value::Number(_)) => Some(value.clone()),
        ("number", _) => return Err("not a number".into()),
        ("checkbox", Value::Bool(_)) => Some(value.clone()),
        ("checkbox", _) => return Err("not a boolean".into()),
        (_, Value::String(s)) => Some(Value::String(s.clone())),
        _ => return Err("unsupported value".into()),
    };
    store.settings_access(&id, true, |m| match v {
        Some(v) => {
            m.insert(key, v);
        }
        None => {
            m.remove(&key);
        }
    })
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

/// https only, and only to hosts in the addon's manifest `connect` list. `{{setting:KEY}}` placeholders in the
/// url, header values, body, basic auth and form fields are filled in here from the addon's settings.
#[cfg_attr(not(target_os = "linux"), tauri::command)]
pub async fn addon_http(app: AppHandle, store: State<'_, AddonStore>, addon: String, req: HttpReq) -> Result<HttpResp, String> {
    let addons = load_all(&app);
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
    let client = reqwest::Client::builder()
    .timeout(Duration::from_millis(req.timeout_ms.unwrap_or(60_000)))
    .redirect(reqwest::redirect::Policy::none()) // a redirect could leave the allowed hosts
    .build()
    .map_err(err)?;
    let mut rb = client.request(method, url);
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
    if !load_all(&app).iter().any(|a| a.id == addon && a.enabled) {
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
