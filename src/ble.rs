//! BLE broker: owns all Bluetooth connections, remembers devices, auto-reconnects,
//! and exposes everything to the webview shim.

#[allow(unused_imports)]
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

#[allow(unused_imports)]
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager as _, State};
use tokio::{
    sync::{oneshot, Mutex},
    time::timeout,
};
use uuid::Uuid;

#[cfg(not(target_os = "android"))]
use btleplug::api::{
    Central, CentralEvent, CharPropFlags, Characteristic, Manager as _, Peripheral as _,
    PeripheralProperties, ScanFilter, WriteType,
};
#[cfg(not(target_os = "android"))]
use btleplug::platform::{Adapter, Manager, Peripheral};
#[cfg(not(target_os = "android"))]
use futures::StreamExt;
#[cfg(not(target_os = "android"))]
use tauri::{async_runtime::JoinHandle, WebviewUrl, WebviewWindowBuilder, WindowEvent};
#[cfg(not(target_os = "android"))]
use tokio::time::sleep;

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

// ---------- tracing: `TERRA_TRACE=1` (writes + control point), `TERRA_TRACE=all` (also every notification) ----------

#[allow(dead_code)]
fn trace_level() -> u8 {
    static L: std::sync::OnceLock<u8> = std::sync::OnceLock::new();
    *L.get_or_init(|| match std::env::var("TERRA_TRACE").as_deref() {
        Ok("all") => 2,
        Ok("0") | Ok("off") => 0,
        _ => 1, // default: log GATT writes, subscribes and control-point responses
    })
}

fn su(u: &Uuid) -> String {
    let s = u.to_string();
    if s.starts_with("0000") && s.ends_with("-0000-1000-8000-00805f9b34fb") {
        s[4..8].to_string()
    } else {
        s
    }
}

#[allow(dead_code)]
fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(" ")
}

/// Decodes an FTMS Fitness Machine Control Point (0x2AD9) command or response.
#[allow(dead_code)]
fn ftms_cp(b: &[u8]) -> Option<String> {
    let op = *b.first()?;
    let i16le = |i: usize| b.get(i..i + 2).map(|x| i16::from_le_bytes([x[0], x[1]]));
    Some(match op {
        0x00 => "RequestControl".to_string(),
        0x01 => "Reset".to_string(),
        0x02 => "SetTargetSpeed".to_string(),
        0x03 => "SetTargetInclination".to_string(),
        0x04 => "SetTargetResistance".to_string(),
        0x05 => format!("SetTargetPower {} W", i16le(1)?),
        0x07 => "Start/Resume".to_string(),
        0x08 => format!("Stop/Pause {}", b.get(1).copied().unwrap_or(0)),
        0x11 => {
            let wind = i16le(1)? as f32 * 0.001;
            let grade = i16le(3)? as f32 * 0.01;
            let crr = *b.get(5)? as f32 * 0.0001;
            let cw = *b.get(6)? as f32 * 0.01;
            format!("SimParams wind={wind:.3}m/s grade={grade:.2}% crr={crr:.4} cw={cw:.2}")
        }
        0x80 => {
            let (req, res) = (*b.get(1)?, *b.get(2)?);
            let name = match res {
                1 => "success",
                2 => "not supported",
                3 => "invalid parameter",
                4 => "operation failed",
                5 => "control not permitted",
                _ => "?",
            };
            format!("Response to 0x{req:02x}: {name}")
        }
        _ => return None,
    })
}

#[allow(dead_code)]
fn describe(chr: &Uuid, b: &[u8]) -> String {
    let h = hex(b);
    match (su(chr).as_str(), ftms_cp(b)) {
        ("2ad9", Some(d)) => format!("{h}  [{d}]"),
        _ => h,
    }
}

/// Call `window.__terra.<func>(...args)` in the biketerra webview.
fn js(app: &AppHandle, func: &str, args: Vec<Value>) {
    let a: Vec<String> = args.iter().map(|v| v.to_string()).collect();
    let code = format!("window.__terra && window.__terra.{func}({})", a.join(","));
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.eval(code);
    } else {
        let evt = serde_json::json!({
            "event": "js_eval",
            "code": code
        });
        if let Ok(line) = serde_json::to_string(&evt) {
            use std::io::Write;
            let mut out = std::io::stdout().lock();
            let _ = writeln!(out, "{line}");
            let _ = out.flush();
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DiscoveredProps {
    pub id: String,
    pub name: Option<String>,
    pub rssi: Option<i16>,
    pub services: Vec<Uuid>,
}

// ---------- filters ----------

#[derive(Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Filter {
    services: Option<Vec<String>>,
    name: Option<String>,
    name_prefix: Option<String>,
}

impl Filter {
    fn matches(&self, props: &DiscoveredProps) -> bool {
        if let Some(svcs) = &self.services {
            if !svcs.is_empty() {
                let mut matched = false;
                for s in svcs {
                    let s_clean = s.trim().to_lowercase();
                    let s_short = if s_clean.len() == 4 {
                        s_clean.clone()
                    } else if s_clean.len() == 36 && s_clean.starts_with("0000") && s_clean.ends_with("-0000-1000-8000-00805f9b34fb") {
                        s_clean[4..8].to_string()
                    } else {
                        "".to_string()
                    };

                    for ps in &props.services {
                        let ps_str = ps.to_string().to_lowercase();
                        if ps_str == s_clean {
                            matched = true;
                            break;
                        }
                        if !s_short.is_empty() && su(ps).to_lowercase() == s_short {
                            matched = true;
                            break;
                        }
                    }
                    if matched {
                        break;
                    }
                }
                if !matched {
                    return false;
                }
            }
        }
        if let Some(n) = &self.name {
            if props.name.as_deref() != Some(n.as_str()) {
                return false;
            }
        }
        if let Some(p) = &self.name_prefix {
            if !props.name.as_deref().map_or(false, |x| x.starts_with(p.as_str())) {
                return false;
            }
        }
        true
    }
}

fn wanted(props: &DiscoveredProps, filters: &[Filter], excl: &[Filter], all: bool) -> bool {
    if excl.iter().any(|f| f.matches(props)) {
        return false;
    }
    if all || filters.is_empty() {
        return true;
    }
    filters.iter().any(|f| f.matches(props)) || props.services.is_empty() || props.name.is_some()
}

/// Lower = more important (trainers first, HR last), used to order auto-picks.
fn rank(props: &DiscoveredProps) -> u8 {
    let has = |s: &str| props.services.contains(&Uuid::parse_str(s).unwrap());
    if has("00001826-0000-1000-8000-00805f9b34fb") || has("6e40fec1-b5a3-f393-e0a9-e50e24dcca9e") {
        0
    } else if has("00001818-0000-1000-8000-00805f9b34fb") {
        1
    } else if has("00001816-0000-1000-8000-00805f9b34fb") {
        2
    } else if has("0000180d-0000-1000-8000-00805f9b34fb") {
        3
    } else {
        4
    }
}

fn kind(rank: u8) -> &'static str {
    ["Trainer", "Power meter", "Speed / cadence", "Heart rate", "Device"][rank.min(4) as usize]
}

#[cfg(not(target_os = "android"))]
async fn in_range(p: &Peripheral, props: &PeripheralProperties) -> bool {
    props.rssi.is_some() || p.is_connected().await.unwrap_or(false)
}

#[cfg(not(target_os = "android"))]
fn props_from_peripheral(p: &Peripheral, props: &PeripheralProperties) -> DiscoveredProps {
    DiscoveredProps {
        id: p.id().to_string(),
        name: props.local_name.clone(),
        rssi: props.rssi,
        services: props.services.clone(),
    }
}

// ---------- wire types ----------

#[derive(Serialize, Deserialize, Debug)]
pub struct DeviceInfo {
    pub id: String,
    pub name: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CharInfo {
    uuid: String,
    read: bool,
    write: bool,
    write_without_response: bool,
    notify: bool,
    indicate: bool,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ServiceInfo {
    uuid: String,
    characteristics: Vec<CharInfo>,
}

#[derive(Serialize, Deserialize)]
pub struct Candidate {
    id: String,
    name: Option<String>,
    rssi: Option<i16>,
    kind: String,
    remembered: bool,
    #[serde(default)]
    rank: u8,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Remembered {
    pub id: String,
    pub name: Option<String>,
    /// Set when an addon paired the device: the page's auto-pair/chooser then leaves it alone.
    #[serde(default)]
    pub owner: Option<String>,
}

struct Pending {
    filters: Vec<Filter>,
    exclusions: Vec<Filter>,
    accept_all: bool,
    owner: Option<String>,
    tx: Option<oneshot::Sender<Option<String>>>,
}

// ---------- DESKTOP BROKER ----------

#[cfg(not(target_os = "android"))]
pub struct Broker {
    app: AppHandle,
    adapter: Adapter,
    peripherals: Mutex<HashMap<String, Peripheral>>,
    notify_tasks: Mutex<HashMap<String, JoinHandle<()>>>,
    subs: Mutex<HashSet<(String, String)>>, // (device id, characteristic uuid)
    pub known: Mutex<Vec<Remembered>>,
    connecting: Mutex<HashSet<String>>,
    /// One lock per device so only one connect runs at a time (see `connect`).
    #[allow(dead_code)]
    conn_locks: Mutex<std::collections::HashMap<String, Arc<Mutex<()>>>>,
    pending: Mutex<Option<Pending>>,
    /// Addon ids that currently let the page use their devices (see `ble_release`).
    pub released: Mutex<HashSet<String>>,
    store: PathBuf,
}

#[cfg(not(target_os = "android"))]
impl Broker {
    pub async fn new(app: AppHandle) -> Result<Arc<Self>, String> {
        let manager = Manager::new().await.map_err(err)?;
        let adapter = manager
            .adapters()
            .await
            .map_err(err)?
            .into_iter()
            .next()
            .ok_or("no bluetooth adapter found")?;

        let dir = app.path().app_data_dir().unwrap_or_else(|_| PathBuf::from("."));
        let _ = std::fs::create_dir_all(&dir);
        let store = dir.join("devices.json");
        let known: Vec<Remembered> = std::fs::read_to_string(&store)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        println!("[terra] {} remembered device(s), store: {}", known.len(), store.display());

        let b = Arc::new(Self {
            app,
            adapter,
            peripherals: Mutex::new(HashMap::new()),
            notify_tasks: Mutex::new(HashMap::new()),
            subs: Mutex::new(HashSet::new()),
            known: Mutex::new(known),
            connecting: Mutex::new(HashSet::new()),
            conn_locks: Mutex::new(HashMap::new()),
            pending: Mutex::new(None),
            released: Mutex::new(HashSet::new()),
            store,
        });
        b.spawn_event_loop();
        b.spawn_supervisor();
        Ok(b)
    }

    fn spawn_event_loop(self: &Arc<Self>) {
        let me = self.clone();
        tauri::async_runtime::spawn(async move {
            if let Ok(mut events) = me.adapter.events().await {
                while let Some(ev) = events.next().await {
                    if let CentralEvent::DeviceDisconnected(id) = ev {
                        let id = id.to_string();
                        me.subs.lock().await.retain(|(d, _)| *d != id);
                        if let Some(t) = me.notify_tasks.lock().await.remove(&id) {
                            t.abort();
                        }
                        js(&me.app, "disconnected", vec![json!(id)]);
                    }
                }
            }
        });
    }

    fn spawn_supervisor(self: &Arc<Self>) {
        let me = self.clone();
        tauri::async_runtime::spawn(async move {
            let mut scanning = false;
            loop {
                sleep(Duration::from_secs(3)).await;
                if me.pending.lock().await.is_some() {
                    scanning = false;
                    continue;
                }
                let known = me.known.lock().await.clone();
                let mut missing = false;
                if !known.is_empty() {
                    let list = me.adapter.peripherals().await.unwrap_or_default();
                    for k in &known {
                        let Some(p) = list.iter().find(|p| p.id().to_string() == k.id) else {
                            missing = true;
                            continue;
                        };
                        if p.is_connected().await.unwrap_or(false) {
                            if me.peripherals.lock().await.insert(k.id.clone(), p.clone()).is_none() {
                                println!("[terra] {} already connected at BLE level", k.id);
                            }
                            continue;
                        }
                        missing = true;
                        let visible = matches!(p.properties().await, Ok(Some(ref pr)) if pr.rssi.is_some());
                        if visible && me.connecting.lock().await.insert(k.id.clone()) {
                            let (me2, p2, id) = (me.clone(), p.clone(), k.id.clone());
                            tauri::async_runtime::spawn(async move {
                                let r = timeout(Duration::from_secs(20), async {
                                    p2.connect().await?;
                                    p2.discover_services().await
                                })
                                .await;
                                match r {
                                    Ok(Ok(())) => {
                                        println!("[terra] auto-connected {id}");
                                        me2.peripherals.lock().await.insert(id.clone(), p2);
                                    }
                                    other => eprintln!("[terra] auto-connect {id} failed: {other:?}"),
                                }
                                me2.connecting.lock().await.remove(&id);
                            });
                        }
                    }
                }
                if missing && !scanning {
                    scanning = me.adapter.start_scan(ScanFilter::default()).await.is_ok();
                    println!("[terra] scanning for remembered devices (started: {scanning})");
                } else if !missing && scanning {
                    let _ = me.adapter.stop_scan().await;
                    scanning = false;
                }
            }
        });
    }

    async fn get(&self, id: &str) -> Result<Peripheral, String> {
        if let Some(p) = self.peripherals.lock().await.get(id).cloned() {
            return Ok(p);
        }
        let p = self
            .adapter
            .peripherals()
            .await
            .map_err(err)?
            .into_iter()
            .find(|p| p.id().to_string() == id)
            .ok_or_else(|| format!("unknown device {id}"))?;
        self.peripherals.lock().await.insert(id.to_string(), p.clone());
        Ok(p)
    }

    fn save_known(&self, k: &[Remembered]) {
        if let Ok(s) = serde_json::to_string_pretty(k) {
            let _ = std::fs::write(&self.store, s);
        }
    }

    async fn remember(&self, info: &DeviceInfo, owner: Option<String>) {
        let mut k = self.known.lock().await;
        match k.iter_mut().find(|x| x.id == info.id) {
            Some(e) => {
                e.name = info.name.clone();
                if owner.is_some() {
                    e.owner = owner;
                }
            }
            None => k.push(Remembered { id: info.id.clone(), name: info.name.clone(), owner }),
        }
        self.save_known(&k);
    }

    pub async fn forget(&self, id: &str) -> Result<(), String> {
        {
            let mut k = self.known.lock().await;
            k.retain(|x| x.id != id);
            self.save_known(&k);
        }
        if let Some(t) = self.notify_tasks.lock().await.remove(id) {
            t.abort();
        }
        self.subs.lock().await.retain(|(d, _)| d != id);
        if let Some(p) = self.peripherals.lock().await.remove(id) {
            let _ = p.disconnect().await;
        }
        Ok(())
    }

    async fn remembered_match(&self, filters: &[Filter], excl: &[Filter], all: bool) -> Option<DeviceInfo> {
        let known = self.known.lock().await.clone();
        if known.is_empty() {
            return None;
        }
        let released = self.released.lock().await.clone();
        let mut best: Option<(u8, Peripheral, Option<String>)> = None;
        for p in self.adapter.peripherals().await.ok()? {
            let id = p.id().to_string();
            if !known.iter().any(|k| k.id == id && k.owner.as_ref().map_or(true, |o| released.contains(o))) {
                continue;
            }
            let Ok(Some(props)) = p.properties().await else { continue };
            if !in_range(&p, &props).await {
                continue;
            }
            let d_props = props_from_peripheral(&p, &props);
            if !wanted(&d_props, filters, excl, all) {
                continue;
            }
            let r = rank(&d_props);
            if best.as_ref().map_or(true, |b| r < b.0) {
                best = Some((r, p, props.local_name.clone()));
            }
        }
        let (_, p, name) = best?;
        let id = p.id().to_string();
        self.peripherals.lock().await.insert(id.clone(), p);
        Some(DeviceInfo { id, name })
    }

    pub async fn request_device(
        self: &Arc<Self>,
        filters: Vec<Filter>,
        exclusions: Vec<Filter>,
        accept_all: bool,
        silent: bool,
        owner: Option<String>,
    ) -> Result<DeviceInfo, String> {
        if owner.is_none() && silent {
            if let Some(info) = self.remembered_match(&filters, &exclusions, accept_all).await {
                println!("[terra] remembered match: {:?}", info.name);
                return Ok(info);
            }
        }

        if silent {
            return Err("NotFound: no remembered device available".into());
        }

        let (tx, rx) = oneshot::channel();
        {
            let mut pend = self.pending.lock().await;
            if pend.is_some() {
                return Err("NotFound: a device chooser is already open".into());
            }
            *pend = Some(Pending { filters, exclusions, accept_all, owner: owner.clone(), tx: Some(tx) });
        }
        let _ = self.adapter.start_scan(ScanFilter::default()).await;
        if let Err(e) = self.open_chooser() {
            self.pending.lock().await.take();
            return Err(format!("NotFound: could not open chooser: {e}"));
        }

        let res = timeout(Duration::from_secs(120), rx).await;

        self.pending.lock().await.take();
        let _ = self.adapter.stop_scan().await;
        if let Some(w) = self.app.get_webview_window("chooser") {
            let _ = w.close();
        }

        let Ok(Ok(Some(id))) = res else {
            return Err("NotFound: no device selected".into());
        };
        let p = self
            .adapter
            .peripherals()
            .await
            .map_err(err)?
            .into_iter()
            .find(|p| p.id().to_string() == id)
            .ok_or("NotFound: device vanished")?;
        let name = p.properties().await.ok().flatten().and_then(|x| x.local_name);
        let info = DeviceInfo { id: id.clone(), name };
        self.peripherals.lock().await.insert(id, p);
        self.remember(&info, owner).await;
        Ok(info)
    }

    fn open_chooser(self: &Arc<Self>) -> Result<(), String> {
        if self.app.get_webview_window("main").is_none() {
            return Ok(());
        }
        let win = WebviewWindowBuilder::new(&self.app, "chooser", WebviewUrl::App("chooser.html".into()))
            .title("Select Bluetooth device")
            .inner_size(420.0, 520.0)
            .build()
            .map_err(err)?;
        let me = self.clone();
        win.on_window_event(move |ev| {
            if let WindowEvent::Destroyed = ev {
                let me = me.clone();
                tauri::async_runtime::spawn(async move { me.resolve(None).await });
            }
        });
        Ok(())
    }

    pub async fn resolve(&self, id: Option<String>) {
        if let Some(p) = self.pending.lock().await.as_mut() {
            if let Some(tx) = p.tx.take() {
                let _ = tx.send(id);
            }
        }
    }

    pub async fn candidates(&self) -> Vec<Candidate> {
        let (filters, excl, all, owner) = {
            let g = self.pending.lock().await;
            match g.as_ref() {
                Some(p) => (p.filters.clone(), p.exclusions.clone(), p.accept_all, p.owner.clone()),
                None => return vec![],
            }
        };
        let known = self.known.lock().await.clone();
        let released = self.released.lock().await.clone();
        let mut out = vec![];
        for p in self.adapter.peripherals().await.unwrap_or_default() {
            let Ok(Some(props)) = p.properties().await else { continue };
            if !in_range(&p, &props).await {
                continue;
            }
            let d_props = props_from_peripheral(&p, &props);
            if !wanted(&d_props, &filters, &excl, all) {
                continue;
            }
            let id = p.id().to_string();
            if owner.is_none() && known.iter().any(|k| k.id == id && k.owner.as_ref().map_or(false, |o| !released.contains(o))) {
                continue;
            }
            let r = rank(&d_props);
            out.push(Candidate {
                remembered: known.iter().any(|k| k.id == id),
                id,
                name: props.local_name.clone(),
                rssi: props.rssi,
                kind: kind(r).to_string(),
                rank: r,
            });
        }
        out.sort_by(|a, b| a.rank.cmp(&b.rank).then(b.rssi.cmp(&a.rssi)));
        out
    }

    pub async fn connect(&self, id: &str) -> Result<Vec<ServiceInfo>, String> {
        let p = self.get(id).await?;
        if !p.is_connected().await.unwrap_or(false) {
            p.connect().await.map_err(err)?;
        }
        p.discover_services().await.map_err(err)?;

        let mut stream = p.notifications().await.map_err(err)?;
        let (app, did) = (self.app.clone(), id.to_string());
        let handle = tauri::async_runtime::spawn(async move {
            while let Some(n) = stream.next().await {
                js(
                    &app,
                    "notify",
                    vec![json!(did), json!(n.uuid.to_string()), json!(B64.encode(&n.value))],
                );
            }
        });
        if let Some(old) = self.notify_tasks.lock().await.insert(id.to_string(), handle) {
            old.abort();
        }

        Ok(p.services()
            .into_iter()
            .map(|s| ServiceInfo {
                uuid: s.uuid.to_string(),
                characteristics: s
                    .characteristics
                    .into_iter()
                    .map(|c| CharInfo {
                        uuid: c.uuid.to_string(),
                        read: c.properties.contains(CharPropFlags::READ),
                        write: c.properties.contains(CharPropFlags::WRITE),
                        write_without_response: c.properties.contains(CharPropFlags::WRITE_WITHOUT_RESPONSE),
                        notify: c.properties.contains(CharPropFlags::NOTIFY),
                        indicate: c.properties.contains(CharPropFlags::INDICATE),
                    })
                    .collect(),
            })
            .collect())
    }

    pub async fn disconnect(&self, id: &str) -> Result<(), String> {
        let p = self.get(id).await?;
        if let Some(t) = self.notify_tasks.lock().await.remove(id) {
            t.abort();
        }
        let subscribed: Vec<String> = self
            .subs
            .lock()
            .await
            .iter()
            .filter(|(d, _)| d.as_str() == id)
            .map(|(_, c)| c.clone())
            .collect();
        self.subs.lock().await.retain(|(d, _)| d != id);

        if self.known.lock().await.iter().any(|k| k.id == id) {
            for c in subscribed {
                if let Some(ch) = p.characteristics().into_iter().find(|x| x.uuid.to_string() == c) {
                    let _ = p.unsubscribe(&ch).await;
                }
            }
            Ok(())
        } else {
            p.disconnect().await.map_err(err)
        }
    }

    async fn characteristic(&self, id: &str, service: &str, chr: &str) -> Result<(Peripheral, Characteristic), String> {
        let p = self.get(id).await?;
        let (s, c) = (Uuid::parse_str(service).map_err(err)?, Uuid::parse_str(chr).map_err(err)?);
        let find = |p: &Peripheral| p.characteristics().into_iter().find(|x| x.uuid == c && x.service_uuid == s);
        if let Some(ch) = find(&p) {
            return Ok((p, ch));
        }
        p.discover_services().await.map_err(err)?;
        let ch = find(&p).ok_or_else(|| format!("characteristic {chr} not found"))?;
        Ok((p, ch))
    }

    pub async fn read(&self, id: &str, s: &str, c: &str) -> Result<String, String> {
        let (p, ch) = self.characteristic(id, s, c).await?;
        let v = p.read(&ch).await.map_err(err)?;
        if trace_level() > 0 {
            println!("[ble] read {id} {} -> {}", su(&ch.uuid), describe(&ch.uuid, &v));
        }
        Ok(B64.encode(v))
    }

    pub async fn write(&self, id: &str, s: &str, c: &str, value: &str, with_response: bool) -> Result<(), String> {
        let (p, ch) = self.characteristic(id, s, c).await?;
        let bytes = B64.decode(value).map_err(err)?;
        let wt = if with_response { WriteType::WithResponse } else { WriteType::WithoutResponse };
        let t0 = std::time::Instant::now();
        let r = p.write(&ch, &bytes, wt).await.map_err(err);
        if trace_level() > 0 {
            let res = match &r {
                Ok(_) => "ok".to_string(),
                Err(e) => format!("ERR {e}"),
            };
            println!(
                "[ble] write {id} {} {} {} -> {res} ({} ms)",
                su(&ch.uuid),
                if with_response { "req" } else { "cmd" },
                describe(&ch.uuid, &bytes),
                t0.elapsed().as_millis()
            );
        }
        r
    }

    pub async fn subscribe(&self, id: &str, s: &str, c: &str, on: bool) -> Result<(), String> {
        let (p, ch) = self.characteristic(id, s, c).await?;
        let key = (id.to_string(), ch.uuid.to_string());
        let mut subs = self.subs.lock().await;
        if on {
            if subs.contains(&key) {
                if trace_level() > 0 {
                    println!("[ble] subscribe {id} {} (already subscribed, skipped)", su(&ch.uuid));
                }
                return Ok(());
            }
            p.subscribe(&ch).await.map_err(err)?;
            if trace_level() > 0 {
                println!("[ble] subscribe {id} {} ok", su(&ch.uuid));
            }
            subs.insert(key);
        } else {
            p.unsubscribe(&ch).await.map_err(err)?;
            subs.remove(&key);
        }
        Ok(())
    }
}

// ---------- ANDROID BROKER & JNI ----------

#[cfg(target_os = "android")]
extern "C" {
    fn __android_log_print(prio: std::os::raw::c_int, tag: *const std::os::raw::c_char, fmt: *const std::os::raw::c_char, ...) -> std::os::raw::c_int;
}

#[cfg(target_os = "android")]
fn alog(msg: &str) {
    let tag = match std::ffi::CString::new("TerraRustBle") {
        Ok(s) => s,
        Err(_) => return,
    };
    let fmt = match std::ffi::CString::new("%s") {
        Ok(s) => s,
        Err(_) => return,
    };
    let c_msg = match std::ffi::CString::new(msg) {
        Ok(s) => s,
        Err(_) => return,
    };
    unsafe {
        __android_log_print(4, tag.as_ptr(), fmt.as_ptr(), c_msg.as_ptr());
    }
}

#[cfg(target_os = "android")]
static ANDROID_APP: std::sync::Mutex<Option<AppHandle>> = std::sync::Mutex::new(None);

#[cfg(target_os = "android")]
static JAVA_VM: std::sync::atomic::AtomicPtr<jni::sys::JavaVM> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

#[cfg(target_os = "android")]
#[no_mangle]
pub unsafe extern "C" fn JNI_OnLoad(
    vm: *mut jni::sys::JavaVM,
    _reserved: *mut std::ffi::c_void,
) -> jni::sys::jint {
    JAVA_VM.store(vm, std::sync::atomic::Ordering::SeqCst);
    jni::sys::JNI_VERSION_1_6
}

#[cfg(target_os = "android")]
fn get_java_vm() -> Result<jni::JavaVM, String> {
    let ptr = JAVA_VM.load(std::sync::atomic::Ordering::SeqCst);
    if ptr.is_null() {
        return Err("JavaVM pointer is null".to_string());
    }
    unsafe { jni::JavaVM::from_raw(ptr) }.map_err(err)
}

#[cfg(target_os = "android")]
static BLE_MANAGER_CLASS: std::sync::RwLock<Option<jni::objects::GlobalRef>> =
    std::sync::RwLock::new(None);

#[cfg(target_os = "android")]
#[derive(Clone, Copy)]
struct SendJMethodID(jni::sys::jmethodID);

#[cfg(target_os = "android")]
unsafe impl Send for SendJMethodID {}

#[cfg(target_os = "android")]
unsafe impl Sync for SendJMethodID {}

#[cfg(target_os = "android")]
struct BleManagerMethods {
    start_scan: SendJMethodID,
    stop_scan: SendJMethodID,
    get_discovered_devices_json: SendJMethodID,
    is_connected: SendJMethodID,
    connect: SendJMethodID,
    disconnect: SendJMethodID,
    read_characteristic: SendJMethodID,
    write_characteristic: SendJMethodID,
    subscribe_characteristic: SendJMethodID,
    unsubscribe_all: SendJMethodID,
}

#[cfg(target_os = "android")]
static BLE_METHODS: std::sync::RwLock<Option<BleManagerMethods>> = std::sync::RwLock::new(None);

#[cfg(target_os = "android")]
#[no_mangle]
pub unsafe extern "C" fn Java_dev_frazse_terra_BleManager_registerClassJni(
    env: *mut jni::sys::JNIEnv,
    class: jni::sys::jclass,
) {
    if env.is_null() || class.is_null() {
        return;
    }
    let mut env = match unsafe { jni::JNIEnv::from_raw(env) } {
        Ok(e) => e,
        Err(_) => return,
    };
    let cls = unsafe { jni::objects::JClass::from_raw(class) };
    let global_ref = match env.new_global_ref(&cls) {
        Ok(g) => g,
        Err(e) => {
            alog(&format!("Failed to create global ref for BleManager: {:?}", e));
            return;
        }
    };

    let mut get_mid = |name: &str, sig: &str| -> Option<SendJMethodID> {
        match env.get_static_method_id(&cls, name, sig) {
            Ok(m) => Some(SendJMethodID(m.into_raw())),
            Err(err) => {
                alog(&format!("Failed to get static method id for {} {}: {:?}", name, sig, err));
                if env.exception_check().unwrap_or(false) {
                    let _ = env.exception_describe();
                    let _ = env.exception_clear();
                }
                None
            }
        }
    };

    let start_scan = get_mid("startScan", "()Z");
    let stop_scan = get_mid("stopScan", "()V");
    let get_discovered_devices_json = get_mid("getDiscoveredDevicesJson", "()Ljava/lang/String;");
    let is_connected = get_mid("isConnected", "(Ljava/lang/String;)Z");
    let connect = get_mid("connect", "(Ljava/lang/String;)Ljava/lang/String;");
    let disconnect = get_mid("disconnect", "(Ljava/lang/String;)V");
    let read_characteristic = get_mid("readCharacteristic", "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;");
    let write_characteristic = get_mid("writeCharacteristic", "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Z)Ljava/lang/String;");
    let subscribe_characteristic = get_mid("subscribeCharacteristic", "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Z)Ljava/lang/String;");
    let unsubscribe_all = get_mid("unsubscribeAll", "(Ljava/lang/String;)Ljava/lang/String;");

    if let (
        Some(start_scan),
        Some(stop_scan),
        Some(get_discovered_devices_json),
        Some(is_connected),
        Some(connect),
        Some(disconnect),
        Some(read_characteristic),
        Some(write_characteristic),
        Some(subscribe_characteristic),
        Some(unsubscribe_all),
    ) = (
        start_scan,
        stop_scan,
        get_discovered_devices_json,
        is_connected,
        connect,
        disconnect,
        read_characteristic,
        write_characteristic,
        subscribe_characteristic,
        unsubscribe_all,
    ) {
        if let Ok(mut w) = BLE_METHODS.write() {
            *w = Some(BleManagerMethods {
                start_scan,
                stop_scan,
                get_discovered_devices_json,
                is_connected,
                connect,
                disconnect,
                read_characteristic,
                write_characteristic,
                subscribe_characteristic,
                unsubscribe_all,
            });
        }
        if let Ok(mut w) = BLE_MANAGER_CLASS.write() {
            *w = Some(global_ref);
        }
        alog("BleManager methods successfully registered and cached!");
    } else {
        alog("ERROR: Some BleManager methods could not be found!");
    }
}

#[cfg(target_os = "android")]
fn call_ble_manager(method: &str, sig: &str, args: &[jni::objects::JValue]) -> Result<String, String> {
    let vm = get_java_vm()?;
    let mut env = vm.attach_current_thread().map_err(err)?;
    let read_guard = BLE_MANAGER_CLASS.read().map_err(|e| e.to_string())?;
    let global_ref = read_guard.as_ref().ok_or_else(|| "BleManager class not registered".to_string())?;
    let cls = unsafe { jni::objects::JClass::from_raw(global_ref.as_raw()) };

    let method_id = {
        let guard = BLE_METHODS.read().map_err(|e| e.to_string())?;
        let methods = guard.as_ref().ok_or_else(|| "BleManager methods not registered".to_string())?;
        let raw = match method {
            "startScan" => methods.start_scan.0,
            "stopScan" => methods.stop_scan.0,
            "getDiscoveredDevicesJson" => methods.get_discovered_devices_json.0,
            "isConnected" => methods.is_connected.0,
            "connect" => methods.connect.0,
            "disconnect" => methods.disconnect.0,
            "readCharacteristic" => methods.read_characteristic.0,
            "writeCharacteristic" => methods.write_characteristic.0,
            "subscribeCharacteristic" => methods.subscribe_characteristic.0,
            "unsubscribeAll" => methods.unsubscribe_all.0,
            _ => return Err(format!("Unknown method name: {method}")),
        };
        unsafe { jni::objects::JStaticMethodID::from_raw(raw) }
    };

    let ret_type = if sig.ends_with("Z") {
        jni::signature::ReturnType::Primitive(jni::signature::Primitive::Boolean)
    } else if sig.ends_with("V") {
        jni::signature::ReturnType::Primitive(jni::signature::Primitive::Void)
    } else {
        jni::signature::ReturnType::Object
    };

    let jvalues: Vec<jni::sys::jvalue> = args.iter().map(|v| v.as_jni()).collect();

    let res = match unsafe { env.call_static_method_unchecked(&cls, method_id, ret_type, &jvalues) } {
        Ok(v) => v,
        Err(e) => {
            alog(&format!("call_static_method_unchecked {method} {sig} failed: {e}"));
            if env.exception_check().unwrap_or(false) {
                let _ = env.exception_describe();
                let _ = env.exception_clear();
            }
            return Err(format!("JNI error calling {method}: {e}"));
        }
    };

    if sig.ends_with("Z") {
        let b = res.z().map_err(err)?;
        return Ok(if b { "true".to_string() } else { "false".to_string() });
    }
    if sig.ends_with("V") {
        return Ok("".to_string());
    }
    if let Ok(jstr) = res.l() {
        if jstr.is_null() {
            return Ok("".to_string());
        }
        let rust_str: String = env.get_string(&jni::objects::JString::from(jstr)).map_err(err)?.into();
        return Ok(rust_str);
    }
    Ok("".to_string())
}

#[cfg(target_os = "android")]
#[no_mangle]
pub unsafe extern "C" fn Java_dev_frazse_terra_BleManager_notifyValueJni(
    env: *mut jni::sys::JNIEnv,
    _class: jni::sys::jclass,
    address: jni::sys::jstring,
    char_uuid: jni::sys::jstring,
    b64_val: jni::sys::jstring,
) {
    if env.is_null() {
        return;
    }
    let mut env = match unsafe { jni::JNIEnv::from_raw(env) } {
        Ok(e) => e,
        Err(_) => return,
    };
    let addr_jstr = unsafe { jni::objects::JString::from_raw(address) };
    let char_jstr = unsafe { jni::objects::JString::from_raw(char_uuid) };
    let val_jstr = unsafe { jni::objects::JString::from_raw(b64_val) };

    let addr_str: String = env.get_string(&addr_jstr).map(|s| s.into()).unwrap_or_default();
    let chr_str: String = env.get_string(&char_jstr).map(|s| s.into()).unwrap_or_default();
    let val_str: String = env.get_string(&val_jstr).map(|s| s.into()).unwrap_or_default();

    if let Ok(guard) = ANDROID_APP.lock() {
        if let Some(app) = guard.as_ref() {
            js(
                app,
                "notify",
                vec![
                    json!(addr_str),
                    json!(chr_str),
                    json!(val_str),
                ],
            );
        }
    }
}

#[cfg(target_os = "android")]
#[no_mangle]
pub unsafe extern "C" fn Java_dev_frazse_terra_BleManager_notifyDisconnectedJni(
    env: *mut jni::sys::JNIEnv,
    _class: jni::sys::jclass,
    address: jni::sys::jstring,
) {
    if env.is_null() {
        return;
    }
    let mut env = match unsafe { jni::JNIEnv::from_raw(env) } {
        Ok(e) => e,
        Err(_) => return,
    };
    let addr_jstr = unsafe { jni::objects::JString::from_raw(address) };
    let addr_str: String = env.get_string(&addr_jstr).map(|s| s.into()).unwrap_or_default();
    if let Ok(guard) = ANDROID_APP.lock() {
        if let Some(app) = guard.as_ref() {
            js(app, "disconnected", vec![json!(addr_str)]);
        }
    }
}

#[cfg(target_os = "android")]
#[derive(Deserialize)]
struct RawDiscoveredDevice {
    id: String,
    name: Option<String>,
    rssi: Option<i16>,
    services: Option<Vec<String>>,
}

pub fn get_terra_storage_dir(app: &AppHandle) -> PathBuf {
    #[cfg(target_os = "android")]
    {
        let ext = PathBuf::from("/storage/emulated/0/Android/data/dev.frazse.terra/files");
        if ext.exists() || std::fs::create_dir_all(&ext).is_ok() {
            let _ = std::fs::create_dir_all(ext.join("addons"));
            let _ = std::fs::create_dir_all(ext.join("addon-data"));
            return ext;
        }
    }

    let dir = app.path().app_data_dir().unwrap_or_else(|_| PathBuf::from("."));
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::create_dir_all(dir.join("addons"));
    let _ = std::fs::create_dir_all(dir.join("addon-data"));
    dir
}

#[cfg(target_os = "android")]
pub struct Broker {
    app: AppHandle,
    known: Mutex<Vec<Remembered>>,
    connecting: Mutex<HashSet<String>>,
    pending: Mutex<Option<Pending>>,
    released: Mutex<HashSet<String>>,
    store: PathBuf,
}

#[cfg(target_os = "android")]
impl Broker {
    pub async fn new(app: AppHandle) -> Result<Arc<Self>, String> {
        if let Ok(mut guard) = ANDROID_APP.lock() {
            *guard = Some(app.clone());
        }

        let dir = get_terra_storage_dir(&app);
        let store = dir.join("devices.json");
        let known: Vec<Remembered> = std::fs::read_to_string(&store)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();

        let b = Arc::new(Self {
            app,
            known: Mutex::new(known),
            connecting: Mutex::new(HashSet::new()),
            pending: Mutex::new(None),
            released: Mutex::new(HashSet::new()),
            store,
        });
        b.spawn_supervisor();
        Ok(b)
    }

    fn spawn_supervisor(self: &Arc<Self>) {
        let me = self.clone();
        tauri::async_runtime::spawn(async move {
            let mut scanning = false;
            loop {
                tokio::time::sleep(Duration::from_secs(3)).await;
                if me.pending.lock().await.is_some() {
                    scanning = false;
                    continue;
                }
                let known = me.known.lock().await.clone();
                let mut missing = false;
                if !known.is_empty() {
                    let discovered = me.discovered_devices();
                    for k in &known {
                        if me.is_connected(&k.id) {
                            continue;
                        }
                        missing = true;
                        let visible = discovered.iter().any(|d| d.id == k.id);
                        if visible && me.connecting.lock().await.insert(k.id.clone()) {
                            let (me2, id) = (me.clone(), k.id.clone());
                            tauri::async_runtime::spawn(async move {
                                let r = timeout(Duration::from_secs(20), async {
                                    me2.connect(&id).await
                                })
                                .await;
                                match r {
                                    Ok(Ok(_)) => println!("[terra] auto-connected {id}"),
                                    other => eprintln!("[terra] auto-connect {id} failed: {other:?}"),
                                }
                                me2.connecting.lock().await.remove(&id);
                            });
                        }
                    }
                }
                if missing && !scanning {
                    let res = call_ble_manager("startScan", "()Z", &[]);
                    scanning = res.ok().as_deref() == Some("true");
                    println!("[terra] scanning for remembered devices (started: {scanning})");
                } else if !missing && scanning {
                    let _ = call_ble_manager("stopScan", "()V", &[]);
                    scanning = false;
                }
            }
        });
    }

    fn discovered_devices(&self) -> Vec<DiscoveredProps> {
        let json_str = match call_ble_manager("getDiscoveredDevicesJson", "()Ljava/lang/String;", &[]) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        let raw: Vec<RawDiscoveredDevice> = serde_json::from_str(&json_str).unwrap_or_default();
        raw.into_iter()
            .map(|d| DiscoveredProps {
                id: d.id,
                name: d.name,
                rssi: d.rssi,
                services: d
                    .services
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|s| Uuid::parse_str(&s).ok())
                    .collect(),
            })
            .collect()
    }

    fn is_connected(&self, id: &str) -> bool {
        let Ok(vm) = get_java_vm() else { return false };
        let env = match vm.attach_current_thread() {
            Ok(e) => e,
            Err(_) => return false,
        };
        let Ok(j_id) = env.new_string(id) else { return false };
        let Ok(res) = call_ble_manager("isConnected", "(Ljava/lang/String;)Z", &[jni::objects::JValue::Object(&j_id)]) else { return false };
        res == "true"
    }

    fn open_chooser(self: &Arc<Self>) -> Result<(), String> {
        js(&self.app, "showChooserModal", vec![]);
        Ok(())
    }

    async fn resolve(&self, id: Option<String>) {
        js(&self.app, "hideChooserModal", vec![]);
        let tx = {
            let mut pend = self.pending.lock().await;
            pend.take().and_then(|mut p| p.tx.take())
        };
        if let Some(tx) = tx {
            let _ = tx.send(id);
        }
    }

    async fn remembered_match(&self, filters: &[Filter], excl: &[Filter], all: bool) -> Option<DeviceInfo> {
        let known = self.known.lock().await.clone();
        if known.is_empty() {
            return None;
        }
        let released = self.released.lock().await.clone();
        let mut best: Option<(u8, DiscoveredProps)> = None;
        for props in self.discovered_devices() {
            if !known.iter().any(|k| k.id == props.id && k.owner.as_ref().map_or(true, |o| released.contains(o))) {
                continue;
            }
            if !wanted(&props, filters, excl, all) {
                continue;
            }
            let r = rank(&props);
            if best.as_ref().map_or(true, |b| r < b.0) {
                best = Some((r, props));
            }
        }
        let (_, props) = best?;
        Some(DeviceInfo { id: props.id, name: props.name })
    }

    pub async fn request_device(
        self: &Arc<Self>,
        filters: Vec<Filter>,
        exclusions: Vec<Filter>,
        accept_all: bool,
        silent: bool,
        owner: Option<String>,
    ) -> Result<DeviceInfo, String> {
        alog(&format!("request_device called: silent={} accept_all={} owner={:?}", silent, accept_all, owner));
        if owner.is_none() && silent {
            if let Some(info) = self.remembered_match(&filters, &exclusions, accept_all).await {
                alog(&format!("remembered match: {:?}", info.name));
                return Ok(info);
            }
        }

        if silent {
            return Err("NotFound: no remembered device available".into());
        }

        let (tx, rx) = oneshot::channel();
        {
            let mut pend = self.pending.lock().await;
            if pend.is_some() {
                alog("request_device error: a device chooser is already open");
                return Err("NotFound: a device chooser is already open".into());
            }
            *pend = Some(Pending { filters, exclusions, accept_all, owner: owner.clone(), tx: Some(tx) });
        }

        alog("Calling BleManager.startScan()...");
        let scan_res = call_ble_manager("startScan", "()Z", &[]);
        alog(&format!("BleManager.startScan() result: {:?}", scan_res));

        if let Err(e) = self.open_chooser() {
            self.pending.lock().await.take();
            return Err(format!("NotFound: could not open chooser: {e}"));
        }

        let res = timeout(Duration::from_secs(120), rx).await;

        self.pending.lock().await.take();
        let _ = call_ble_manager("stopScan", "()V", &[]);

        let Ok(Ok(Some(id))) = res else {
            return Err("NotFound: no device selected".into());
        };

        let name = self.discovered_devices().into_iter().find(|d| d.id == id).and_then(|d| d.name);
        let info = DeviceInfo { id: id.clone(), name };
        self.remember(&info, owner).await;
        Ok(info)
    }

    pub async fn candidates(&self) -> Vec<Candidate> {
        let (filters, excl, all, owner) = {
            let g = self.pending.lock().await;
            match g.as_ref() {
                Some(p) => (p.filters.clone(), p.exclusions.clone(), p.accept_all, p.owner.clone()),
                None => return vec![],
            }
        };
        let known = self.known.lock().await.clone();
        let released = self.released.lock().await.clone();
        let mut out = vec![];
        let disc = self.discovered_devices();
        alog(&format!("candidates: discovered_count={}", disc.len()));
        for props in disc {
            let w = wanted(&props, &filters, &excl, all);
            alog(&format!("device id={} name={:?} wanted={}", props.id, props.name, w));
            if !w {
                continue;
            }
            let id = props.id.clone();
            if owner.is_none() && known.iter().any(|k| k.id == id && k.owner.as_ref().map_or(false, |o| !released.contains(o))) {
                continue;
            }
            let r = rank(&props);
            out.push(Candidate {
                remembered: known.iter().any(|k| k.id == id),
                id,
                name: props.name.clone(),
                rssi: props.rssi,
                kind: kind(r).to_string(),
                rank: r,
            });
        }
        out.sort_by(|a, b| a.rank.cmp(&b.rank).then(b.rssi.cmp(&a.rssi)));
        out
    }

    pub async fn connect(&self, id: &str) -> Result<Vec<ServiceInfo>, String> {
        let vm = get_java_vm()?;
        let env = vm.attach_current_thread().map_err(err)?;
        let j_id = env.new_string(id).map_err(err)?;
        let res = call_ble_manager("connect", "(Ljava/lang/String;)Ljava/lang/String;", &[jni::objects::JValue::Object(&j_id)])?;
        if res.starts_with("Error:") {
            return Err(res);
        }
        serde_json::from_str(&res).map_err(err)
    }

    pub async fn disconnect(&self, id: &str) -> Result<(), String> {
        let is_known = self.known.lock().await.iter().any(|k| k.id == id);
        let vm = get_java_vm()?;
        let env = vm.attach_current_thread().map_err(err)?;
        let j_id = env.new_string(id).map_err(err)?;

        if is_known {
            let _ = call_ble_manager("unsubscribeAll", "(Ljava/lang/String;)Ljava/lang/String;", &[jni::objects::JValue::Object(&j_id)]);
        } else {
            let _ = call_ble_manager("disconnect", "(Ljava/lang/String;)V", &[jni::objects::JValue::Object(&j_id)]);
        }
        Ok(())
    }

    pub async fn read(&self, id: &str, s: &str, c: &str) -> Result<String, String> {
        let vm = get_java_vm()?;
        let env = vm.attach_current_thread().map_err(err)?;
        let j_id = env.new_string(id).map_err(err)?;
        let j_s = env.new_string(s).map_err(err)?;
        let j_c = env.new_string(c).map_err(err)?;
        let res = call_ble_manager("readCharacteristic", "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;", &[
            jni::objects::JValue::Object(&j_id),
            jni::objects::JValue::Object(&j_s),
            jni::objects::JValue::Object(&j_c),
        ])?;
        if res.starts_with("Error:") {
            Err(res)
        } else {
            Ok(res)
        }
    }

    pub async fn write(&self, id: &str, s: &str, c: &str, value: &str, with_response: bool) -> Result<(), String> {
        let vm = get_java_vm()?;
        let env = vm.attach_current_thread().map_err(err)?;
        let j_id = env.new_string(id).map_err(err)?;
        let j_s = env.new_string(s).map_err(err)?;
        let j_c = env.new_string(c).map_err(err)?;
        let j_v = env.new_string(value).map_err(err)?;
        let res = call_ble_manager("writeCharacteristic", "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Z)Ljava/lang/String;", &[
            jni::objects::JValue::Object(&j_id),
            jni::objects::JValue::Object(&j_s),
            jni::objects::JValue::Object(&j_c),
            jni::objects::JValue::Object(&j_v),
            jni::objects::JValue::Bool(if with_response { 1 } else { 0 }),
        ])?;
        if res.starts_with("Error:") {
            Err(res)
        } else {
            Ok(())
        }
    }

    pub async fn subscribe(&self, id: &str, s: &str, c: &str, on: bool) -> Result<(), String> {
        let vm = get_java_vm()?;
        let env = vm.attach_current_thread().map_err(err)?;
        let j_id = env.new_string(id).map_err(err)?;
        let j_s = env.new_string(s).map_err(err)?;
        let j_c = env.new_string(c).map_err(err)?;
        let res = call_ble_manager("subscribeCharacteristic", "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Z)Ljava/lang/String;", &[
            jni::objects::JValue::Object(&j_id),
            jni::objects::JValue::Object(&j_s),
            jni::objects::JValue::Object(&j_c),
            jni::objects::JValue::Bool(if on { 1 } else { 0 }),
        ])?;
        if res.starts_with("Error:") {
            Err(res)
        } else {
            Ok(())
        }
    }

    fn save_known(&self, k: &[Remembered]) {
        if let Ok(s) = serde_json::to_string_pretty(k) {
            let _ = std::fs::write(&self.store, s);
        }
    }

    async fn remember(&self, info: &DeviceInfo, owner: Option<String>) {
        let mut k = self.known.lock().await;
        match k.iter_mut().find(|x| x.id == info.id) {
            Some(e) => {
                e.name = info.name.clone();
                if owner.is_some() {
                    e.owner = owner;
                }
            }
            None => k.push(Remembered { id: info.id.clone(), name: info.name.clone(), owner }),
        }
        self.save_known(&k);
    }

    pub async fn forget(&self, id: &str) -> Result<(), String> {
        {
            let mut k = self.known.lock().await;
            k.retain(|x| x.id != id);
            self.save_known(&k);
        }
        let _ = self.disconnect(id).await;
        Ok(())
    }
}

/// Failed GATT commands are always printed (they used to vanish: the page just saw a rejected promise).
fn logged<T>(what: &str, id: &str, r: Result<T, String>) -> Result<T, String> {
    if let Err(e) = &r {
        eprintln!("[ble] {what} {id} FAILED: {e}");
    }
    r
}

// ---------- tauri commands ----------

pub struct Ble(pub Option<Arc<Broker>>);

pub fn broker<'a>(s: &'a State<'_, Ble>) -> Result<&'a Arc<Broker>, String> {
    s.0.as_ref().ok_or_else(|| "no bluetooth adapter available".to_string())
}

#[tauri::command]
pub async fn ble_request_device(
    state: State<'_, Ble>,
    filters: Vec<Filter>,
    exclusion_filters: Option<Vec<Filter>>,
    accept_all: bool,
    silent: Option<bool>,
    owner: Option<String>,
) -> Result<DeviceInfo, String> {
    broker(&state)?
        .request_device(filters, exclusion_filters.unwrap_or_default(), accept_all, silent.unwrap_or(false), owner)
        .await
}

/// Devices an addon paired earlier (so it can restore them without any UI).
#[tauri::command]
pub async fn ble_remembered(state: State<'_, Ble>, owner: String) -> Result<Vec<DeviceInfo>, String> {
    let b = broker(&state)?;
    let known = b.known.lock().await;
    Ok(known
        .iter()
        .filter(|k| k.owner.as_deref() == Some(owner.as_str()))
        .map(|k| DeviceInfo { id: k.id.clone(), name: k.name.clone() })
        .collect())
}

/// Let the page (not just the owning addon) use the owner's devices, e.g. so Biketerra's own
/// Zwift Ride support can take the controller during rides. Not persisted: addons set it per page load.
#[tauri::command]
pub async fn ble_release(state: State<'_, Ble>, owner: String, released: bool) -> Result<(), String> {
    let b = broker(&state)?;
    let mut r = b.released.lock().await;
    if released {
        r.insert(owner);
    } else {
        r.remove(&owner);
    }
    Ok(())
}

#[tauri::command]
pub async fn ble_connect(state: State<'_, Ble>, id: String) -> Result<Vec<ServiceInfo>, String> {
    let r = broker(&state)?.connect(&id).await;
    logged("connect", &id, r)
}

#[tauri::command]
pub async fn ble_disconnect(state: State<'_, Ble>, id: String) -> Result<(), String> {
    broker(&state)?.disconnect(&id).await
}

#[tauri::command]
pub async fn ble_forget(state: State<'_, Ble>, id: String) -> Result<(), String> {
    broker(&state)?.forget(&id).await
}

#[tauri::command]
pub async fn ble_read(state: State<'_, Ble>, id: String, service: String, characteristic: String) -> Result<String, String> {
    let r = broker(&state)?.read(&id, &service, &characteristic).await;
    logged(&format!("read {characteristic}"), &id, r)
}

#[tauri::command]
pub async fn ble_write(
    state: State<'_, Ble>,
    id: String,
    service: String,
    characteristic: String,
    value: String,
    with_response: bool,
) -> Result<(), String> {
    let r = broker(&state)?.write(&id, &service, &characteristic, &value, with_response).await;
    logged(&format!("write {characteristic}"), &id, r)
}

#[tauri::command]
pub async fn ble_subscribe(state: State<'_, Ble>, id: String, service: String, characteristic: String) -> Result<(), String> {
    let r = broker(&state)?.subscribe(&id, &service, &characteristic, true).await;
    logged(&format!("subscribe {characteristic}"), &id, r)
}

#[tauri::command]
pub async fn ble_unsubscribe(state: State<'_, Ble>, id: String, service: String, characteristic: String) -> Result<(), String> {
    let r = broker(&state)?.subscribe(&id, &service, &characteristic, false).await;
    logged(&format!("unsubscribe {characteristic}"), &id, r)
}

#[tauri::command]
pub async fn chooser_candidates(state: State<'_, Ble>) -> Result<Vec<Candidate>, String> {
    Ok(broker(&state)?.candidates().await)
}

#[tauri::command]
pub async fn chooser_pick(state: State<'_, Ble>, id: String) -> Result<(), String> {
    broker(&state)?.resolve(Some(id)).await;
    Ok(())
}

#[tauri::command]
pub async fn chooser_cancel(state: State<'_, Ble>) -> Result<(), String> {
    broker(&state)?.resolve(None).await;
    Ok(())
}
