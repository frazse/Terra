//! BLE broker: owns all Bluetooth connections, remembers devices, auto-reconnects,
//! and exposes everything to the webview shim.

#[allow(unused_imports)]
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

#[allow(unused_imports)]
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
use tauri::{AppHandle, Manager as _, State};
#[cfg(any(target_os = "linux", target_os = "windows"))]
use crate::linux_runtime::{AppHandle, State};
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

/// Platform-agnostic task spawn (tokio on Linux/Windows RPC mode, tauri elsewhere).
#[cfg(not(any(target_os = "android", target_os = "linux", target_os = "windows")))]
use tauri::async_runtime::spawn as spawn_task;
#[cfg(any(target_os = "linux", target_os = "windows"))]
use crate::linux_runtime::async_runtime::spawn as spawn_task;
#[cfg(not(any(target_os = "android", target_os = "linux", target_os = "windows")))]
use tauri::{async_runtime::JoinHandle, WebviewUrl, WebviewWindowBuilder, WindowEvent};
#[cfg(any(target_os = "linux", target_os = "windows"))]
use crate::linux_runtime::async_runtime::JoinHandle;
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
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn js(app: &AppHandle, func: &str, args: Vec<Value>) {
    let a: Vec<String> = args.iter().map(|v| v.to_string()).collect();
    let code = format!("window.__terra && window.__terra.{func}({})", a.join(","));
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.eval(code);
    }
}

/// On Linux/Windows (RPC mode), writes a js_eval event to stdout for main.js to handle.
#[cfg(any(target_os = "linux", target_os = "windows"))]
fn js(app_opt: Option<&AppHandle>, func: &str, args: Vec<Value>) {
    let a: Vec<String> = args.iter().map(|v| v.to_string()).collect();
    let code = format!("window.__terra && window.__terra.{func}({})", a.join(","));
    // On Linux, get_webview_window always returns None → falls through to stdout.
    if let Some(w) = app_opt.and_then(|a| a.get_webview_window("main")) {
        let _ = w.eval(code.clone());
    }
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
    // `wanted` is the LENIENT matcher used by the auto-pair (remembered-device) path only: a
    // device the user already paired stays offerable even if its advertised services don't
    // exactly match the current request (e.g. the COROS HR strap) via the "any named device" /
    // "no services advertised" fallbacks. The user-facing chooser uses the stricter
    // `chooser_matches` below instead.
    filters.iter().any(|f| f.matches(props)) || props.services.is_empty() || props.name.is_some()
}

/// Strict, Web-BT-style matcher for the manual chooser: a device only appears if it matches one
/// of the filters the game requested (or the request admits all). No "any named device"
/// fallback here — that fallback (in `wanted`) was leaking phones and other non-workout
/// clutter into the chooser.
fn chooser_matches(props: &DiscoveredProps, filters: &[Filter], excl: &[Filter], all: bool) -> bool {
    if excl.iter().any(|f| f.matches(props)) {
        return false;
    }
    if all || filters.is_empty() {
        return true;
    }
    filters.iter().any(|f| f.matches(props))
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

/// Returns true only when BlueZ has resolved at least one non-generic GATT service
/// (anything other than 0x1800/0x1801/0x180a) with characteristics, so we never hand
/// Biketerra a half-discovered peripheral that it immediately disconnects.
#[cfg(not(target_os = "android"))]
fn services_ready(p: &Peripheral) -> bool {
    p.services().iter().any(|s| {
        let short = su(&s.uuid);
        short != "1800" && short != "1801" && short != "180a" && !s.characteristics.is_empty()
    })
}

#[cfg(not(target_os = "android"))]
fn gatt_counts(p: &Peripheral) -> (usize, usize, usize) {
    let svcs = p.services();
    let chars: usize = svcs.iter().map(|s| s.characteristics.len()).sum();
    let descs: usize = svcs
        .iter()
        .flat_map(|s| s.characteristics.iter())
        .map(|c| c.descriptors.len())
        .sum();
    (svcs.len(), chars, descs)
}

/// Returns true once every NOTIFY/INDICATE characteristic on non-generic services has
/// at least one descriptor (its 0x2902 CCCD) populated by BlueZ. Calling StartNotify
/// before BlueZ finishes descriptor discovery causes `ATT error: 0x0e` and link drop.
#[cfg(not(target_os = "android"))]
fn cccd_ready(p: &Peripheral) -> bool {
    for s in p.services() {
        let short = su(&s.uuid);
        if short == "1800" || short == "1801" || short == "180a" {
            continue;
        }
        for c in s.characteristics {
            if (c.properties.contains(CharPropFlags::NOTIFY)
                || c.properties.contains(CharPropFlags::INDICATE))
                && c.descriptors.is_empty()
            {
                return false;
            }
        }
    }
    true
}

// ---------- wire types ----------

#[derive(Serialize, Deserialize, Debug)]
pub struct DeviceInfo {
    pub id: String,
    pub name: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct BleDeviceStatus {
    pub id: String,
    pub name: Option<String>,
    pub owner: Option<String>,
    pub connected: bool,
    #[serde(default)]
    pub connecting: bool,
    pub remembered: bool,
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
    /// `None` on Linux (RPC mode has no webview); `Some` on other platforms.
    app: Option<AppHandle>,
    adapter: Adapter,
    /// Direct BlueZ session on Linux so discovery uses `Transport::Le` instead of
    /// `btleplug`'s hardcoded `Transport::Auto` (which runs Classic BR/EDR Inquiry
    /// and starves active BLE trainer links for 2+ seconds until supervision timeout).
    #[cfg(target_os = "linux")]
    bluez: Option<(bluez_async::BluetoothSession, bluez_async::AdapterId)>,
    peripherals: Mutex<HashMap<String, Peripheral>>,
    notify_tasks: Mutex<HashMap<String, JoinHandle<()>>>,
    subs: Mutex<HashSet<(String, String)>>, // (device id, characteristic uuid)
    pub known: Mutex<Vec<Remembered>>,
    connecting: Mutex<HashSet<String>>,
    /// Serializes adapter-level `connect()` calls between supervisor and page/addons
    /// (Linux BlueZ rejects overlapping HCI LE Create Connection calls with `InProgress`).
    connect_gate: Mutex<()>,
    /// Tracks whether `adapter.start_scan` is currently active so we can pause scanning
    /// before calling `connect()`.
    scanning: AtomicBool,
    /// True while Biketerra's native BLE bridge (`window.native`) has an active scan running.
    native_scanning: AtomicBool,
    /// Per-device backoff timestamps so failed/timed-out BlueZ connects aren't hammered every 3 s.
    retry_after: Mutex<HashMap<String, std::time::Instant>>,
    /// Timestamp of the last OS-level disconnect per device, so post-dropout reconnects
    /// let BlueZ & the peripheral settle briefly before dialing again.
    disconnected_at: Mutex<HashMap<String, std::time::Instant>>,
    /// Hold off restarting background/native LE discovery briefly right after a connect
    /// completes so characteristic subscriptions + FTMS RequestControl aren't stalled.
    scan_holdoff_until: Mutex<Option<std::time::Instant>>,
    pending: Mutex<Option<Pending>>,
    /// Addon ids that currently let the page use their devices (see `ble_release`).
    pub released: Mutex<HashSet<String>>,
    shutting_down: AtomicBool,
    supervisor_task: Mutex<Option<JoinHandle<()>>>,
    store: PathBuf,
}

#[cfg(not(target_os = "android"))]
impl Broker {
    pub async fn new(app: Option<AppHandle>) -> Result<Arc<Self>, String> {
        let manager = Manager::new().await.map_err(err)?;
        let adapter = manager
            .adapters()
            .await
            .map_err(err)?
            .into_iter()
            .next()
            .ok_or("no bluetooth adapter found")?;

        #[cfg(any(target_os = "linux", target_os = "windows"))]
        let dir = crate::linux_runtime::terra_data_dir();
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        let dir = app.as_ref().unwrap().path().app_data_dir().unwrap_or_else(|_| PathBuf::from("."));
        let _ = std::fs::create_dir_all(&dir);
        let store = dir.join("devices.json");
        let known: Vec<Remembered> = std::fs::read_to_string(&store)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        println!("[terra] {} remembered device(s), store: {}", known.len(), store.display());

        #[cfg(target_os = "linux")]
        let bluez = match bluez_async::BluetoothSession::new().await {
            Ok((handle, session)) => {
                spawn_task(async move {
                    let _ = handle.await;
                });
                match session.get_adapters().await {
                    Ok(mut adapters) if !adapters.is_empty() => {
                        let aid = adapters.remove(0).id;
                        println!("[terra] BlueZ LE-only scan session ready");
                        Some((session, aid))
                    }
                    _ => None,
                }
            }
            Err(_) => None,
        };

        let b = Arc::new(Self {
            app: app,
            adapter,
            #[cfg(target_os = "linux")]
            bluez,
            peripherals: Mutex::new(HashMap::new()),
            notify_tasks: Mutex::new(HashMap::new()),
            subs: Mutex::new(HashSet::new()),
            known: Mutex::new(known),
            connecting: Mutex::new(HashSet::new()),
            connect_gate: Mutex::new(()),
            scanning: AtomicBool::new(false),
            native_scanning: AtomicBool::new(false),
            retry_after: Mutex::new(HashMap::new()),
            disconnected_at: Mutex::new(HashMap::new()),
            scan_holdoff_until: Mutex::new(None),
            pending: Mutex::new(None),
            released: Mutex::new(HashSet::new()),
            shutting_down: AtomicBool::new(false),
            supervisor_task: Mutex::new(None),
            store,
        });
        b.spawn_event_loop();
        b.spawn_supervisor().await;
        Ok(b)
    }

    /// Starts LE-only discovery on Linux (`Transport::Le`) to avoid `btleplug`'s
    /// `Transport::Auto` running Classic Bluetooth BR/EDR Inquiry, which stalls
    /// active BLE connections for 2+ seconds and triggers `OS disconnected`.
    async fn adapter_start_le_scan(&self) -> bool {
        #[cfg(target_os = "linux")]
        if let Some((ref session, ref aid)) = self.bluez {
            let filter = bluez_async::DiscoveryFilter {
                transport: Some(bluez_async::Transport::Le),
                duplicate_data: Some(true),
                ..Default::default()
            };
            return session
                .start_discovery_on_adapter_with_filter(aid, &filter)
                .await
                .is_ok();
        }
        self.adapter.start_scan(ScanFilter::default()).await.is_ok()
    }

    async fn adapter_stop_le_scan(&self) {
        #[cfg(target_os = "linux")]
        if let Some((ref session, ref aid)) = self.bluez {
            let _ = timeout(Duration::from_secs(2), session.stop_discovery_on_adapter(aid)).await;
            return;
        }
        let _ = timeout(Duration::from_secs(2), self.adapter.stop_scan()).await;
    }

    async fn pause_scan_for_connect(&self) {
        if self.scanning.swap(false, Ordering::SeqCst) {
            self.adapter_stop_le_scan().await;
            sleep(Duration::from_millis(150)).await;
        }
    }

    /// Starts LE discovery only when no connect/service-discovery holds `connect_gate`.
    /// Prevents concurrent `StartDiscovery` + `Device1.Connect` / `discover_services`
    /// collisions on Linux BlueZ that trigger `Service discovery timed out`.
    async fn start_scan_if_idle(&self) -> bool {
        if self.shutting_down.load(Ordering::SeqCst) {
            return false;
        }
        if self.scanning.load(Ordering::SeqCst) {
            return true;
        }
        let Ok(_gate) = self.connect_gate.try_lock() else {
            return false;
        };
        if self.shutting_down.load(Ordering::SeqCst) || self.scanning.load(Ordering::SeqCst) {
            return self.scanning.load(Ordering::SeqCst);
        }
        let ok = self.adapter_start_le_scan().await;
        self.scanning.store(ok, Ordering::SeqCst);
        ok
    }

    /// Connects to `p` (with LE discovery paused) and ensures its GATT table is resolved.
    ///
    /// Critical Linux BlueZ workaround: `btleplug`'s `p.connect()` delegates to
    /// `bluez_async::BluetoothSession::connect_with_timeout`, which waits on
    /// `ServicesResolved` with a hardcoded 5 s timeout (`SERVICE_DISCOVERY_TIMEOUT`).
    /// For peripherals with large GATT tables (e.g. Wahoo KICKR CORE's 12 services),
    /// `Device1.Connect()` frequently succeeds while GATT discovery takes 5.5–9 s,
    /// causing `p.connect()` to return `Err("Service discovery timed out")` while the
    /// BLE link is actually UP and still resolving! Instead of aborting and leaving
    /// BlueZ half-connected, we detect that case and continue polling `discover_services()`
    /// until the GATT table finishes populating and stabilizes.
    async fn connect_and_discover(&self, id: &str, p: &Peripheral) -> Result<(), String> {
        let mut slow_discovery = false;
        if !p.is_connected().await.unwrap_or(false) {
            // Fresh link: any previously tracked subscriptions are stale.
            self.subs.lock().await.retain(|(d, _)| d != id);
            self.pause_scan_for_connect().await;
            match timeout(Duration::from_secs(15), p.connect()).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    let msg = e.to_string();
                    let is_sd_timeout = msg.to_lowercase().contains("service discovery timed out");
                    if is_sd_timeout && p.is_connected().await.unwrap_or(false) {
                        println!(
                            "[ble] {id}: connect() hit 5s service-discovery timeout while connected; waiting for GATT table..."
                        );
                        slow_discovery = true;
                    } else {
                        if p.is_connected().await.unwrap_or(false) {
                            let _ = timeout(Duration::from_secs(3), p.disconnect()).await;
                            self.disconnected_at
                                .lock()
                                .await
                                .insert(id.to_string(), std::time::Instant::now());
                            sleep(Duration::from_millis(300)).await;
                        }
                        return Err(msg);
                    }
                }
                Err(_) => {
                    if p.is_connected().await.unwrap_or(false) {
                        let _ = timeout(Duration::from_secs(3), p.disconnect()).await;
                        self.disconnected_at
                            .lock()
                            .await
                            .insert(id.to_string(), std::time::Instant::now());
                        sleep(Duration::from_millis(300)).await;
                    }
                    return Err("connect timed out".to_string());
                }
            }
        } else if !services_ready(p) {
            slow_discovery = true;
        }
        if self.shutting_down.load(Ordering::SeqCst) {
            return Err("BLE broker is shutting down".into());
        }
        self.ensure_services_ready(id, p, slow_discovery).await
    }

    /// Resolves GATT services on a connected peripheral. When `slow_discovery` is true
    /// (because `bluez-async` hit its 5 s `ServicesResolved` timeout while connected),
    /// polls BlueZ's D-Bus GATT objects for up to 10 s and waits for the service +
    /// characteristic count to stabilize across two consecutive polls so we never hand
    /// Biketerra a partial GATT table. Disconnects `p` cleanly on persistent failure so
    /// BlueZ is never left wedged in a 0-service connected state.
    async fn ensure_services_ready(
        &self,
        id: &str,
        p: &Peripheral,
        slow_discovery: bool,
    ) -> Result<(), String> {
        if !slow_discovery && services_ready(p) {
            return Ok(());
        }
        let deadline = std::time::Instant::now()
            + Duration::from_secs(if slow_discovery { 10 } else { 6 });
        let mut last_err: Option<String> = None;
        let mut stable_counts: Option<(usize, usize, usize)> = None;
        let mut stable_ticks: u8 = 0;
        let mut first = true;

        while std::time::Instant::now() < deadline {
            if self.shutting_down.load(Ordering::SeqCst) {
                return Err("BLE broker is shutting down".into());
            }
            if !p.is_connected().await.unwrap_or(false) {
                self.disconnected_at
                    .lock()
                    .await
                    .insert(id.to_string(), std::time::Instant::now());
                return Err("peripheral disconnected during service discovery".into());
            }
            if first {
                first = false;
                if slow_discovery {
                    sleep(Duration::from_millis(200)).await;
                }
            } else {
                sleep(Duration::from_millis(if slow_discovery { 300 } else { 250 })).await;
            }
            match timeout(Duration::from_secs(5), p.discover_services()).await {
                Ok(Ok(())) => {
                    if services_ready(p) {
                        if !slow_discovery {
                            return Ok(());
                        }
                        if !cccd_ready(p) {
                            stable_counts = None;
                            stable_ticks = 0;
                            continue;
                        }
                        let counts = gatt_counts(p);
                        if stable_counts == Some(counts) {
                            stable_ticks += 1;
                            if stable_ticks >= 2 {
                                println!(
                                    "[ble] {id}: GATT table ready after slow discovery ({} service(s), {} char(s), {} desc(s))",
                                    counts.0, counts.1, counts.2
                                );
                                return Ok(());
                            }
                        } else {
                            stable_counts = Some(counts);
                            stable_ticks = 0;
                        }
                    } else {
                        last_err = Some("no GATT services discovered".into());
                    }
                }
                Ok(Err(e)) => {
                    let msg = e.to_string();
                    let lower = msg.to_lowercase();
                    let retryable = lower.contains("timed out")
                        || lower.contains("timeout")
                        || lower.contains("inprogress")
                        || lower.contains("in progress")
                        || lower.contains("not ready")
                        || lower.contains("busy");
                    last_err = Some(msg);
                    if !retryable {
                        break;
                    }
                }
                Err(_) => {
                    last_err = Some("service discovery timed out".into());
                }
            }
        }
        if services_ready(p) && p.is_connected().await.unwrap_or(false) {
            if slow_discovery {
                let counts = gatt_counts(p);
                println!(
                    "[ble] {id}: GATT table ready at deadline ({} service(s), {} char(s), {} desc(s))",
                    counts.0, counts.1, counts.2
                );
            }
            return Ok(());
        }
        if p.is_connected().await.unwrap_or(false) {
            let _ = timeout(Duration::from_secs(3), p.disconnect()).await;
        }
        self.disconnected_at
            .lock()
            .await
            .insert(id.to_string(), std::time::Instant::now());
        sleep(Duration::from_millis(350)).await;
        Err(last_err.unwrap_or_else(|| "no GATT services discovered".into()))
    }

    fn spawn_event_loop(self: &Arc<Self>) {
        let me = self.clone();
        spawn_task(async move {
            if let Ok(mut events) = me.adapter.events().await {
                while let Some(ev) = events.next().await {
                    if let CentralEvent::DeviceDisconnected(id) = ev {
                        let id = id.to_string();
                        if trace_level() > 0 {
                            println!("[ble] OS disconnected {id}");
                        }
                        me.subs.lock().await.retain(|(d, _)| *d != id);
                        me.peripherals.lock().await.remove(&id);
                        if let Some(t) = me.notify_tasks.lock().await.remove(&id) {
                            t.abort();
                        }
                        let now = std::time::Instant::now();
                        me.disconnected_at.lock().await.insert(id.clone(), now);
                        *me.scan_holdoff_until.lock().await = None;
                        // Brief settle window before the supervisor retries a freshly dropped link.
                        me.retry_after
                            .lock()
                            .await
                            .insert(id.clone(), now + Duration::from_secs(2));
                        // Use js() which on Linux (app is None) writes a js_eval event to stdout.
                        // main.js handles js_eval by executing JS directly in the webview.
                        js(me.app.as_ref(), "disconnected", vec![json!(id)]);
                    }
                }
            }
        });
    }

    async fn spawn_supervisor(self: &Arc<Self>) {
        let me = self.clone();
        let handle = spawn_task(async move {
            let started_at = std::time::Instant::now();
            let mut last_any_connected = false;
            // Kick off LE discovery immediately at app launch so remembered trainers are
            // seen and pre-connected in the background BEFORE the user clicks Start/Resume.
            if !me.known.lock().await.is_empty() && me.start_scan_if_idle().await {
                println!("[terra] scanning for remembered devices (started: true)");
            }
            loop {
                let fast_boot = !last_any_connected && started_at.elapsed() < Duration::from_secs(8);
                sleep(if fast_boot { Duration::from_millis(300) } else { Duration::from_secs(3) }).await;
                if me.shutting_down.load(Ordering::SeqCst) {
                    me.pause_scan_for_connect().await;
                    break;
                }
                if me.pending.lock().await.is_some() {
                    continue;
                }
                let known = me.known.lock().await.clone();
                let mut missing = false;
                let mut any_connected = false;
                let mut to_connect: Vec<(String, Peripheral)> = Vec::new();
                if !known.is_empty() {
                    // A remembered device an enabled addon keeps off the air (its
                    // `btExcludeNames` setting) is not pre-connected: e.g. a Kickr
                    // driven over Wi-Fi must not also be auto-connected over
                    // Bluetooth.
                    let excluded = me.app.as_ref().map(crate::addons::bt_exclude_names).unwrap_or_default();
                    // Prefer our cached Peripheral instances from `me.peripherals` because btleplug's
                    // `adapter.peripherals()` returns fresh structs whose local `services()` cache is
                    // empty until `discover_services()` is called on that specific instance.
                    let mut by_id: HashMap<String, Peripheral> = me.peripherals.lock().await.clone();
                    if let Ok(list) = me.adapter.peripherals().await {
                        for p in list {
                            by_id.entry(p.id().to_string()).or_insert(p);
                        }
                    }
                    let now = std::time::Instant::now();
                    for k in &known {
                        if me.shutting_down.load(Ordering::SeqCst) {
                            break;
                        }
                        if k.name.as_deref().map_or(false, |n| excluded.contains(n)) {
                            continue;
                        }
                        let Some(p) = by_id.get(&k.id) else {
                            missing = true;
                            continue;
                        };
                        if p.is_connected().await.unwrap_or(false) {
                            if services_ready(p) {
                                any_connected = true;
                                me.retry_after.lock().await.remove(&k.id);
                                if me.peripherals.lock().await.insert(k.id.clone(), p.clone()).is_none() {
                                    println!("[terra] {} already connected at BLE level", k.id);
                                    js(me.app.as_ref(), "autoConnected", vec![json!(k.id)]);
                                }
                            } else {
                                // OS-level connected without resolved GATT services yet (e.g.
                                // pre-existing BlueZ link at startup): resolve services via the
                                // serialized connect path instead of advertising a 0-service link.
                                let cooling_down = me
                                    .retry_after
                                    .lock()
                                    .await
                                    .get(&k.id)
                                    .map_or(false, |t| now < *t);
                                if !cooling_down && !me.connecting.lock().await.contains(&k.id) {
                                    to_connect.push((k.id.clone(), p.clone()));
                                }
                            }
                            continue;
                        }
                        // Recover from missed BlueZ disconnect events: if we still hold a
                        // notify task or active subscription for a peripheral BlueZ now
                        // reports as disconnected, clean up and tell the webview.
                        let stale_task = me.notify_tasks.lock().await.remove(&k.id);
                        let stale_subs = {
                            let mut subs = me.subs.lock().await;
                            let before = subs.len();
                            subs.retain(|(d, _)| d != &k.id);
                            subs.len() != before
                        };
                        let had_stale = stale_task.is_some() || stale_subs;
                        if let Some(t) = stale_task {
                            t.abort();
                        }
                        if had_stale {
                            println!("[terra] {} detected disconnected by supervisor", k.id);
                            me.peripherals.lock().await.remove(&k.id);
                            me.disconnected_at.lock().await.insert(k.id.clone(), now);
                            js(me.app.as_ref(), "disconnected", vec![json!(k.id)]);
                        }
                        missing = true;
                        let cooling_down = me
                            .retry_after
                            .lock()
                            .await
                            .get(&k.id)
                            .map_or(false, |t| now < *t);
                        if cooling_down {
                            continue;
                        }
                        let visible = matches!(p.properties().await, Ok(Some(ref pr)) if pr.rssi.is_some());
                        if visible {
                            to_connect.push((k.id.clone(), p.clone()));
                        }
                    }
                }

                // Connect visible remembered devices sequentially with LE discovery paused.
                // On Linux BlueZ, calling Device1.Connect() while StartDiscovery is active
                // frequently stalls until timeout and triggers org.bluez.Error.InProgress.
                if !to_connect.is_empty() && !me.shutting_down.load(Ordering::SeqCst) {
                    let _gate = me.connect_gate.lock().await;
                    if !me.shutting_down.load(Ordering::SeqCst) && me.pending.lock().await.is_none() {
                        me.pause_scan_for_connect().await;
                        for (id, p2) in to_connect {
                            if me.shutting_down.load(Ordering::SeqCst) || me.pending.lock().await.is_some() {
                                break;
                            }
                            let p2 = me.peripherals.lock().await.get(&id).cloned().unwrap_or(p2);
                            if p2.is_connected().await.unwrap_or(false) && services_ready(&p2) {
                                any_connected = true;
                                me.retry_after.lock().await.remove(&id);
                                if me.peripherals.lock().await.insert(id.clone(), p2).is_none() {
                                    js(me.app.as_ref(), "autoConnected", vec![json!(id)]);
                                }
                                continue;
                            }
                            me.connecting.lock().await.insert(id.clone());
                            let t0 = std::time::Instant::now();
                            let r = me.connect_and_discover(&id, &p2).await;
                            me.connecting.lock().await.remove(&id);
                            if me.shutting_down.load(Ordering::SeqCst) {
                                let _ = timeout(Duration::from_secs(3), p2.disconnect()).await;
                                break;
                            }
                            match r {
                                Ok(()) => {
                                    println!(
                                        "[terra] auto-connected {id} ({} service(s), {} ms)",
                                        p2.services().len(),
                                        t0.elapsed().as_millis()
                                    );
                                    any_connected = true;
                                    me.retry_after.lock().await.remove(&id);
                                    me.peripherals.lock().await.insert(id.clone(), p2);
                                    let now = std::time::Instant::now();
                                    *me.scan_holdoff_until.lock().await = Some(now + Duration::from_secs(4));
                                    js(me.app.as_ref(), "autoConnected", vec![json!(id)]);
                                }
                                Err(msg) => {
                                    let backoff = if msg.to_lowercase().contains("timed out") || msg.contains("InProgress") {
                                        Duration::from_secs(15)
                                    } else {
                                        Duration::from_secs(8)
                                    };
                                    eprintln!("[terra] auto-connect {id} failed (retry in {}s): {msg}", backoff.as_secs());
                                    me.retry_after.lock().await.insert(id, std::time::Instant::now() + backoff);
                                }
                            }
                        }
                    }
                }

                last_any_connected = any_connected;
                if me.shutting_down.load(Ordering::SeqCst) {
                    me.pause_scan_for_connect().await;
                    break;
                }
                if me.pending.lock().await.is_some() || !me.connecting.lock().await.is_empty() {
                    continue;
                }
                if me.scan_holdoff_until.lock().await.map_or(false, |t| std::time::Instant::now() < t) {
                    continue;
                }

                // Scan with LE-only discovery (`Transport::Le`) whenever:
                //   1) Biketerra explicitly requested a scan (`native_scanning`), OR
                //   2) Any remembered (non-excluded) device is still missing (e.g. Zwift Ride
                //      controllers woken up after the trainer already connected).
                // Discovery is always paused during `connect_gate` connects + the 4 s settle
                // window, and stops completely once all remembered devices are connected.
                let is_scanning = me.scanning.load(Ordering::SeqCst);
                let want_native_scan = me.native_scanning.load(Ordering::SeqCst);
                let want_scan = want_native_scan || missing;
                if !want_scan {
                    if is_scanning {
                        me.adapter_stop_le_scan().await;
                        me.scanning.store(false, Ordering::SeqCst);
                    }
                } else if !is_scanning {
                    if me.start_scan_if_idle().await {
                        println!("[terra] scanning for remembered devices (started: true)");
                    }
                }
            }
        });
        *self.supervisor_task.lock().await = Some(handle);
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

    /// Disconnect every paired peripheral and stop all notification tasks.
    /// Called once when the app is shutting down (see `ble_disconnect_all`).
    ///
    /// Each device is bounded by a timeout: one hanging disconnect must not
    /// eat the whole shutdown budget — electron kills the process after its
    /// 30 s RPC timeout, leaving every not-yet-reached device connected at
    /// the OS level. A second pass re-checks and disconnects anything still
    /// up: BlueZ happily keeps bonded devices (e.g. an HR strap) connected
    /// Disconnects every active BLE link on app shutdown.
    /// Runs disconnects concurrently across all devices so 4+ peripherals
    /// tear down in ~1 s instead of serially exhausting the shutdown timeout,
    /// then does one quick parallel re-check pass for bonded devices that BlueZ
    /// may briefly hold open.
    pub async fn disconnect_all(&self) -> Result<(), String> {
        // Stop auto-reconnect & scanning immediately so the supervisor cannot
        // fire a new `connect()` while we tear down existing links.
        self.shutting_down.store(true, Ordering::SeqCst);
        if let Some(t) = self.supervisor_task.lock().await.take() {
            t.abort();
        }
        if self.scanning.swap(false, Ordering::SeqCst) {
            self.adapter_stop_le_scan().await;
        }

        let mut nt = self.notify_tasks.lock().await;
        for (id, t) in nt.drain() {
            t.abort();
            println!("[ble] shutdown: aborted notify task {id}");
        }
        drop(nt);
        self.subs.lock().await.clear();

        // Collect every peripheral we tracked OR that is currently in `known` /
        // `connecting` at the adapter level so in-flight auto-connects are caught too.
        let mut targets = self.peripherals.lock().await.clone();
        let known_ids: HashSet<String> = self.known.lock().await.iter().map(|k| k.id.clone()).collect();
        let inflight_ids = self.connecting.lock().await.clone();
        if let Ok(list) = self.adapter.peripherals().await {
            for p in list {
                let id = p.id().to_string();
                if !targets.contains_key(&id) && (known_ids.contains(&id) || inflight_ids.contains(&id)) {
                    targets.insert(id, p);
                }
            }
        }

        // Pass 1: disconnect all connected peripherals in parallel.
        let pass1 = targets.iter().map(|(id, p)| {
            let id = id.clone();
            let p = p.clone();
            async move {
                if !p.is_connected().await.unwrap_or(false) {
                    return;
                }
                match timeout(Duration::from_secs(4), p.disconnect()).await {
                    Ok(Ok(())) => println!("[ble] shutdown: disconnected {id}"),
                    Ok(Err(e)) => eprintln!("[ble] shutdown: disconnect {id} failed: {e}"),
                    Err(_) => eprintln!("[ble] shutdown: disconnect {id} timed out"),
                }
            }
        });
        futures::future::join_all(pass1).await;

        // Wait briefly for any in-flight `p2.connect()` to settle (it checks
        // `shutting_down` and self-disconnects) before the second pass.
        for _ in 0..10 {
            if self.connecting.lock().await.is_empty() {
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }

        // Let BlueZ settle, then re-check in parallel: anything still up gets one more pass.
        sleep(Duration::from_millis(350)).await;
        let mut recheck = targets;
        if let Ok(list) = self.adapter.peripherals().await {
            for p in list {
                let id = p.id().to_string();
                if known_ids.contains(&id) {
                    recheck.entry(id).or_insert(p);
                }
            }
        }
        let pass2 = recheck.iter().map(|(id, p)| {
            let id = id.clone();
            let p = p.clone();
            async move {
                if !p.is_connected().await.unwrap_or(false) {
                    return;
                }
                match timeout(Duration::from_secs(3), p.disconnect()).await {
                    Ok(Ok(())) => println!("[ble] shutdown: disconnected (2nd pass) {id}"),
                    Ok(Err(e)) => eprintln!("[ble] shutdown: 2nd pass {id} failed: {e}"),
                    Err(_) => eprintln!("[ble] shutdown: 2nd pass {id} timed out"),
                }
            }
        });
        futures::future::join_all(pass2).await;
        self.peripherals.lock().await.clear();
        Ok(())
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

    pub async fn drop_device(&self, id: &str) -> Result<(), String> {
        if let Some(t) = self.notify_tasks.lock().await.remove(id) {
            t.abort();
        }
        self.subs.lock().await.retain(|(d, _)| d != id);
        if let Some(p) = self.peripherals.lock().await.remove(id) {
            let _ = timeout(Duration::from_secs(4), p.disconnect()).await;
        } else if let Ok(list) = self.adapter.peripherals().await {
            if let Some(p) = list.into_iter().find(|p| p.id().to_string() == id) {
                let _ = timeout(Duration::from_secs(4), p.disconnect()).await;
            }
        }
        js(self.app.as_ref(), "disconnected", vec![json!(id)]);
        Ok(())
    }

    pub async fn drop_all(&self) -> Result<(), String> {
        let mut nt = self.notify_tasks.lock().await;
        for (_, t) in nt.drain() {
            t.abort();
        }
        drop(nt);
        self.subs.lock().await.clear();
        let drained: Vec<(String, Peripheral)> = self.peripherals.lock().await.drain().collect();
        for (id, p) in drained {
            if p.is_connected().await.unwrap_or(false) {
                let _ = timeout(Duration::from_secs(4), p.disconnect()).await;
            }
            js(self.app.as_ref(), "disconnected", vec![json!(id)]);
        }
        Ok(())
    }

    pub async fn devices_status(&self) -> Vec<BleDeviceStatus> {
        let known = self.known.lock().await.clone();
        let connecting = self.connecting.lock().await.clone();
        let mut by_id: HashMap<String, Peripheral> = self.peripherals.lock().await.clone();
        if let Ok(list) = self.adapter.peripherals().await {
            for p in list {
                by_id.entry(p.id().to_string()).or_insert(p);
            }
        }
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for k in &known {
            seen.insert(k.id.clone());
            let is_connecting = connecting.contains(&k.id);
            let connected = if is_connecting {
                false
            } else {
                match by_id.get(&k.id) {
                    Some(p) => p.is_connected().await.unwrap_or(false) && services_ready(p),
                    None => false,
                }
            };
            out.push(BleDeviceStatus {
                id: k.id.clone(),
                name: k.name.clone(),
                owner: k.owner.clone(),
                connected,
                connecting: is_connecting,
                remembered: true,
            });
        }
        for (id, p) in by_id {
            if seen.contains(&id) || connecting.contains(&id) {
                continue;
            }
            if p.is_connected().await.unwrap_or(false) && services_ready(&p) {
                let name = p.properties().await.ok().flatten().and_then(|pr| pr.local_name);
                out.push(BleDeviceStatus {
                    id,
                    name,
                    owner: None,
                    connected: true,
                    connecting: false,
                    remembered: false,
                });
            }
        }
        out
    }

    pub async fn set_native_scan(&self, on: bool) -> Result<(), String> {
        self.native_scanning.store(on, Ordering::SeqCst);
        if on {
            if self.peripherals.lock().await.is_empty() {
                *self.scan_holdoff_until.lock().await = None;
            }
            // Do not call start_scan while BlueZ holds `connect_gate` (connecting/discovering) —
            // concurrent StartDiscovery triggers Service discovery timed out. The supervisor will
            // resume scanning automatically once `connect_gate` is free.
            let _ = self.start_scan_if_idle().await;
        } else if self.scanning.load(Ordering::SeqCst) && self.pending.lock().await.is_none() {
            let known = self.known.lock().await.clone();
            let excluded = self.app.as_ref().map(crate::addons::bt_exclude_names).unwrap_or_default();
            let map = self.peripherals.lock().await.clone();
            let mut any_missing = false;
            for k in &known {
                if k.name.as_deref().map_or(false, |n| excluded.contains(n)) {
                    continue;
                }
                let conn = match map.get(&k.id) {
                    Some(p) => p.is_connected().await.unwrap_or(false) && services_ready(p),
                    None => false,
                };
                if !conn {
                    any_missing = true;
                    break;
                }
            }
            if !any_missing {
                self.adapter_stop_le_scan().await;
                self.scanning.store(false, Ordering::SeqCst);
            }
        }
        Ok(())
    }

    pub async fn remember_device(&self, id: String, name: Option<String>) -> Result<(), String> {
        let resolved_name = match name {
            Some(n) if !n.is_empty() => Some(n),
            _ => match self.get(&id).await {
                Ok(p) => p.properties().await.ok().flatten().and_then(|pr| pr.local_name),
                Err(_) => None,
            },
        };
        self.remember(&DeviceInfo { id, name: resolved_name }, None).await;
        Ok(())
    }

    #[cfg_attr(any(target_os = "linux", target_os = "windows"), allow(dead_code))]
    pub async fn remembered_page_ids(&self) -> Vec<String> {
        self.known
            .lock()
            .await
            .iter()
            .filter(|k| k.owner.is_none())
            .map(|k| k.id.clone())
            .collect()
    }

    /// Returns nearby/connected non-addon peripherals matching Biketerra's native scan UUIDs
    /// (plus remembered non-addon devices in range even if they omit service UUIDs in ads).
    pub async fn scan_candidates(&self, filter_uuids: Vec<String>) -> Vec<BleDeviceStatus> {
        let known = self.known.lock().await.clone();
        let connecting = self.connecting.lock().await.clone();
        let released = self.released.lock().await.clone();
        let excluded_names = self.app.as_ref().map(crate::addons::bt_exclude_names).unwrap_or_default();
        let filters: Vec<Filter> = filter_uuids
            .into_iter()
            .map(|u| Filter { services: Some(vec![u]), name: None, name_prefix: None })
            .collect();
        let mut by_id: HashMap<String, Peripheral> = self.peripherals.lock().await.clone();
        if let Ok(list) = self.adapter.peripherals().await {
            for p in list {
                by_id.entry(p.id().to_string()).or_insert(p);
            }
        }
        let mut out: Vec<(u8, Option<i16>, BleDeviceStatus)> = Vec::new();
        for p in by_id.into_values() {
            let Ok(Some(props)) = p.properties().await else { continue };
            let id = p.id().to_string();
            let connected = p.is_connected().await.unwrap_or(false);
            let is_connecting = connecting.contains(&id);
            if props.rssi.is_none() && !connected && !is_connecting {
                continue;
            }
            let known_entry = known.iter().find(|k| k.id == id);
            if let Some(k) = known_entry {
                if k.owner.as_ref().map_or(false, |o| !released.contains(o)) {
                    continue;
                }
                if k.name.as_deref().map_or(false, |n| excluded_names.contains(n)) {
                    continue;
                }
            }
            let mut d_props = props_from_peripheral(&p, &props);
            if d_props.name.is_none() {
                d_props.name = known_entry.and_then(|k| k.name.clone());
            }
            if d_props.name.as_deref().map_or(false, |n| excluded_names.contains(n)) {
                continue;
            }
            for svc in p.services() {
                if !d_props.services.contains(&svc.uuid) {
                    d_props.services.push(svc.uuid);
                }
            }
            let remembered = known_entry.is_some();
            if !remembered && !filters.is_empty() && !chooser_matches(&d_props, &filters, &[], false) {
                continue;
            }
            let r = rank(&d_props);
            out.push((
                r,
                props.rssi,
                BleDeviceStatus {
                    id,
                    name: d_props.name,
                    owner: known_entry.and_then(|k| k.owner.clone()),
                    connected,
                    connecting: is_connecting,
                    remembered,
                },
            ));
        }
        out.sort_by(|a, b| {
            b.2.connected
                .cmp(&a.2.connected)
                .then(b.2.remembered.cmp(&a.2.remembered))
                .then(a.0.cmp(&b.0))
                .then(b.1.cmp(&a.1))
        });
        out.into_iter().map(|(_, _, s)| s).collect()
    }

    async fn remembered_match(
        &self,
        filters: &[Filter],
        excl: &[Filter],
        all: bool,
        exclude_ids: &[String],
    ) -> Option<DeviceInfo> {
        let known = self.known.lock().await.clone();
        if known.is_empty() {
            return None;
        }
        let released = self.released.lock().await.clone();
        let excluded_names = self.app.as_ref().map(crate::addons::bt_exclude_names).unwrap_or_default();
        let mut best: Option<(u8, Peripheral, Option<String>)> = None;
        for p in self.adapter.peripherals().await.ok()? {
            let id = p.id().to_string();
            if exclude_ids.iter().any(|x| x == &id) {
                continue;
            }
            let Some(k) = known
                .iter()
                .find(|k| k.id == id && k.owner.as_ref().map_or(true, |o| released.contains(o)))
            else {
                continue;
            };
            if k.name.as_deref().map_or(false, |n| excluded_names.contains(n)) {
                continue;
            }
            let Ok(Some(props)) = p.properties().await else { continue };
            if !in_range(&p, &props).await {
                continue;
            }
            let mut d_props = props_from_peripheral(&p, &props);
            if d_props.name.is_none() {
                d_props.name = k.name.clone();
            }
            if d_props.name.as_deref().map_or(false, |n| excluded_names.contains(n)) {
                continue;
            }
            // Merge discovered GATT service UUIDs for already-connected peripherals so
            // ranking and filter matching stay accurate even when BlueZ omits them in props.
            for svc in p.services() {
                if !d_props.services.contains(&svc.uuid) {
                    d_props.services.push(svc.uuid);
                }
            }
            if !wanted(&d_props, filters, excl, all) {
                continue;
            }
            let r = rank(&d_props);
            if best.as_ref().map_or(true, |b| r < b.0) {
                best = Some((r, p, d_props.name.clone()));
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
        exclude_ids: Vec<String>,
    ) -> Result<DeviceInfo, String> {
        if owner.is_none() && silent {
            if let Some(info) = self.remembered_match(&filters, &exclusions, accept_all, &exclude_ids).await {
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
        let scan_ok = self.adapter_start_le_scan().await;
        self.scanning.store(scan_ok, Ordering::SeqCst);
        if let Err(e) = self.open_chooser() {
            self.pending.lock().await.take();
            return Err(format!("NotFound: could not open chooser: {e}"));
        }

        let res = timeout(Duration::from_secs(120), rx).await;

        self.pending.lock().await.take();
        self.adapter_stop_le_scan().await;
        self.scanning.store(false, Ordering::SeqCst);
        if let Some(ref app) = self.app {
            if let Some(w) = app.get_webview_window("chooser") {
                let _ = w.close();
            }
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

    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    fn open_chooser(self: &Arc<Self>) -> Result<(), String> {
        let win = WebviewWindowBuilder::new(self.app.as_ref().unwrap(), "chooser", WebviewUrl::App("chooser.html".into()))
            .title("Select Bluetooth device")
            .inner_size(420.0, 520.0)
            .build()
            .map_err(err)?;
        let me = self.clone();
        win.on_window_event(move |ev| {
            if let WindowEvent::Destroyed = ev {
                let me = me.clone();
                spawn_task(async move { me.resolve(None).await });
            }
        });
        Ok(())
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn open_chooser(&self) -> Result<(), String> {
        // On Linux (RPC mode), use js() which writes a js_eval event to stdout.
        // main.js handles js_eval by executing JS directly in the webview.
        js(self.app.as_ref(), "showChooserModal", vec![]);
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
            if !chooser_matches(&d_props, &filters, &excl, all) {
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
        self.connect_with_wait(id, false).await
    }

    pub async fn connect_with_wait(&self, id: &str, wait_visible: bool) -> Result<Vec<ServiceInfo>, String> {
        let t_call = std::time::Instant::now();
        if self.shutting_down.load(Ordering::SeqCst) {
            return Err("BLE broker is shutting down".into());
        }
        let mut p = self.get(id).await?;
        let mut already_ready = p.is_connected().await.unwrap_or(false) && services_ready(&p);
        if !already_ready {
            let started = std::time::Instant::now();
            let max_attempts = if wait_visible { 2 } else { 1 };
            for attempt in 0..max_attempts {
                if self.shutting_down.load(Ordering::SeqCst) {
                    return Err("BLE broker is shutting down".into());
                }
                if attempt > 0 {
                    p = self.get(id).await?;
                }
                let is_conn = p.is_connected().await.unwrap_or(false);
                if is_conn && services_ready(&p) {
                    already_ready = true;
                    break;
                }
                if !is_conn {
                    // If this device just dropped out on BlueZ, let the link teardown settle briefly
                    // before dialing again (calling Device1.Connect() at t=5ms after DeviceDisconnected
                    // frequently triggers ServiceDiscoveryTimedOut on Linux BlueZ).
                    if wait_visible {
                        let settle = self
                            .disconnected_at
                            .lock()
                            .await
                            .get(id)
                            .and_then(|t| Duration::from_millis(1200).checked_sub(t.elapsed()));
                        if let Some(wait) = settle {
                            sleep(wait).await;
                        }
                    }
                    let mut visible = matches!(p.properties().await, Ok(Some(ref pr)) if pr.rssi.is_some());
                    if !visible && wait_visible {
                        // Explicit native connect / post-dropout reconnect: scan briefly (without
                        // holding `connect_gate`) for the peripheral to start advertising again.
                        let _ = self.start_scan_if_idle().await;
                        let wait_secs = if attempt == 0 { 10 } else { 5 };
                        let deadline = std::time::Instant::now() + Duration::from_secs(wait_secs);
                        while std::time::Instant::now() < deadline {
                            if self.shutting_down.load(Ordering::SeqCst) {
                                return Err("BLE broker is shutting down".into());
                            }
                            if p.is_connected().await.unwrap_or(false) {
                                visible = true;
                                break;
                            }
                            if matches!(p.properties().await, Ok(Some(ref pr)) if pr.rssi.is_some()) {
                                visible = true;
                                break;
                            }
                            let _ = self.start_scan_if_idle().await;
                            sleep(Duration::from_millis(250)).await;
                        }
                    }
                    if !visible {
                        // Do not pause LE discovery or block `connect_gate` for a powered-off/out-of-range
                        // peripheral (e.g. sleeping Zwift Ride controllers polled by addons).
                        return Err(format!("device {id} is not in range"));
                    }
                }
                let _gate = self.connect_gate.lock().await;
                if self.shutting_down.load(Ordering::SeqCst) {
                    return Err("BLE broker is shutting down".into());
                }
                // Pick up a Peripheral instance already resolved by the supervisor while we waited.
                if let Some(cached) = self.peripherals.lock().await.get(id).cloned() {
                    p = cached;
                }
                if p.is_connected().await.unwrap_or(false) && services_ready(&p) {
                    already_ready = true;
                    self.retry_after.lock().await.remove(id);
                    self.disconnected_at.lock().await.remove(id);
                    break;
                }
                self.connecting.lock().await.insert(id.to_string());
                let res = self.connect_and_discover(id, &p).await;
                self.connecting.lock().await.remove(id);
                match res {
                    Ok(()) => {
                        self.peripherals.lock().await.insert(id.to_string(), p.clone());
                        self.retry_after.lock().await.remove(id);
                        self.disconnected_at.lock().await.remove(id);
                        break;
                    }
                    Err(e) => {
                        if wait_visible {
                            eprintln!("[ble] connect {id} failed (attempt {}): {e}", attempt + 1);
                        }
                        if attempt + 1 < max_attempts
                            && started.elapsed() < Duration::from_secs(12)
                            && !self.shutting_down.load(Ordering::SeqCst)
                        {
                            continue;
                        }
                        return Err(e);
                    }
                }
            }
        }
        // Give JS + Biketerra a 4 s quiet window to finish subscribing and sending
        // FTMS RequestControl / SimParams before background LE discovery resumes.
        *self.scan_holdoff_until.lock().await = Some(std::time::Instant::now() + Duration::from_secs(4));

        let mut stream = p.notifications().await.map_err(err)?;
        let (app, did) = (self.app.clone(), id.to_string());
        let handle = spawn_task(async move {
            while let Some(n) = stream.next().await {
                js(
                    app.as_ref(),
                    "notify",
                    vec![json!(did), json!(n.uuid.to_string()), json!(B64.encode(&n.value))],
                );
            }
        });
        if let Some(old) = self.notify_tasks.lock().await.insert(id.to_string(), handle) {
            old.abort();
        }

        let svcs: Vec<ServiceInfo> = p
            .services()
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
            .collect();
        if svcs.is_empty() {
            return Err("no GATT services discovered".into());
        }
        if trace_level() > 0 {
            println!(
                "[ble] connect {id} -> {} service(s) (ready={already_ready}, {} ms)",
                svcs.len(),
                t_call.elapsed().as_millis()
            );
        }
        Ok(svcs)
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

        let is_known = self.known.lock().await.iter().any(|k| k.id == id);
        if trace_level() > 0 {
            println!("[ble] disconnect {id} (unsub={}, keep_os={is_known})", subscribed.len());
        }
        if is_known {
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
        if on && !p.is_connected().await.unwrap_or(false) {
            self.subs.lock().await.retain(|(d, _)| d != id);
            if let Some(t) = self.notify_tasks.lock().await.remove(id) {
                t.abort();
            }
            js(self.app.as_ref(), "disconnected", vec![json!(id)]);
            return Err(format!("device {id} is disconnected"));
        }
        let has_notify_task = self.notify_tasks.lock().await.contains_key(id);
        let mut subs = self.subs.lock().await;
        if on {
            if subs.contains(&key) && has_notify_task {
                if trace_level() > 0 {
                    println!("[ble] subscribe {id} {} (already subscribed, skipped)", su(&ch.uuid));
                }
                return Ok(());
            }
            let t0 = std::time::Instant::now();
            match p.subscribe(&ch).await {
                Ok(()) => {}
                Err(e) if e.to_string().to_lowercase().contains("already notifying") => {}
                Err(e) => return Err(err(e)),
            }
            if trace_level() > 0 {
                println!("[ble] subscribe {id} {} ok ({} ms)", su(&ch.uuid), t0.elapsed().as_millis());
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
pub(crate) fn alog(msg: &str) {
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
pub(crate) fn get_java_vm() -> Result<jni::JavaVM, String> {
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
pub(crate) fn find_app_class<'local>(
    env: &mut jni::JNIEnv<'local>,
    name: &str,
) -> Result<jni::objects::JClass<'local>, String> {
    let read_guard = BLE_MANAGER_CLASS.read().map_err(|e| e.to_string())?;
    let global_ref = read_guard
        .as_ref()
        .ok_or_else(|| "BleManager class not registered".to_string())?;
    let ble_cls = unsafe { jni::objects::JObject::from_raw(global_ref.as_raw()) };
    let loader = env
        .call_method(&ble_cls, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])
        .map_err(err)?
        .l()
        .map_err(err)?;
    let jname = env.new_string(name).map_err(err)?;
    let cls_obj = env
        .call_method(
            &loader,
            "loadClass",
            "(Ljava/lang/String;)Ljava/lang/Class;",
            &[jni::objects::JValue::Object(&jname)],
        )
        .map_err(err)?
        .l()
        .map_err(err)?;
    Ok(jni::objects::JClass::from(cls_obj))
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

/// Resolve the Terra storage directory.
/// On Android: uses the app's external files dir.
/// On Linux: uses XDG data home (`~/.local/share/terra`).
/// On Windows: uses `%LOCALAPPDATA%\terra`.
/// On other platforms: uses Tauri's app_data_dir.
#[cfg(any(target_os = "linux", target_os = "windows"))]
pub fn get_terra_storage_dir() -> PathBuf {
    let dir = crate::linux_runtime::terra_data_dir();
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::create_dir_all(dir.join("addons"));
    let _ = std::fs::create_dir_all(dir.join("addon-data"));
    dir
}

#[cfg(target_os = "android")]
pub(crate) fn android_internal_files_dir(app: &AppHandle) -> Option<PathBuf> {
    static FILES_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    if let Some(p) = FILES_DIR.get() {
        return Some(p.clone());
    }
    if let Ok(vm) = get_java_vm() {
        if let Ok(mut env) = vm.attach_current_thread() {
            if let Ok(cls) = find_app_class(&mut env, "dev.frazse.terra.MainActivity") {
                if let Ok(val) =
                    env.call_static_method(&cls, "getInternalFilesDir", "()Ljava/lang/String;", &[])
                {
                    if let Ok(obj) = val.l() {
                        if !obj.is_null() {
                            let jstr = jni::objects::JString::from(obj);
                            let s_opt = env.get_string(&jstr).ok().map(String::from);
                            if let Some(s) = s_opt {
                                let p = PathBuf::from(s);
                                if !p.as_os_str().is_empty() {
                                    let _ = FILES_DIR.set(p.clone());
                                    return Some(p);
                                }
                            }
                        }
                    }
                }
            }
            if env.exception_check().unwrap_or(false) {
                let _ = env.exception_clear();
            }
        }
    }
    app.path()
        .data_dir()
        .ok()
        .map(|d| d.join("files"))
        .or_else(|| app.path().app_data_dir().ok())
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub fn get_terra_storage_dir(app: &AppHandle) -> PathBuf {
    #[cfg(target_os = "android")]
    {
        // Check for a user-picked data directory (SAF pointer file in internal filesDir)
        if let Some(files_dir) = android_internal_files_dir(app) {
            let pointer = files_dir.join("terra-data-dir.txt");
            if let Ok(content) = std::fs::read_to_string(&pointer) {
                let path = content.trim();
                if !path.is_empty() {
                    let dir = PathBuf::from(path);
                    if dir.exists() || std::fs::create_dir_all(&dir).is_ok() {
                        let _ = std::fs::create_dir_all(dir.join("addons"));
                        let _ = std::fs::create_dir_all(dir.join("addon-data"));
                        return dir;
                    }
                }
            }
        }

        // Fall back to the default Android external files dir
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
    native_scanning: AtomicBool,
    connect_gate: Mutex<()>,
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
            native_scanning: AtomicBool::new(false),
            connect_gate: Mutex::new(()),
            store,
        });
        b.spawn_supervisor();
        Ok(b)
    }

    pub async fn remembered_page_ids(&self) -> Vec<String> {
        self.known
            .lock()
            .await
            .iter()
            .filter(|k| k.owner.is_none())
            .map(|k| k.id.clone())
            .collect()
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
                    // A remembered device an enabled addon keeps off the air (its
                    // `btExcludeNames` setting) is not pre-connected.
                    let excluded = crate::addons::bt_exclude_names(&me.app);
                    let discovered = me.discovered_devices();
                    for k in &known {
                        if k.name.as_deref().map_or(false, |n| excluded.contains(n)) {
                            continue;
                        }
                        if me.is_connected(&k.id) {
                            continue;
                        }
                        missing = true;
                        let visible = discovered.iter().any(|d| d.id == k.id);
                        if visible && me.connecting.lock().await.insert(k.id.clone()) {
                            let (me2, id) = (me.clone(), k.id.clone());
                            tauri::async_runtime::spawn(async move {
                                let r = timeout(Duration::from_secs(30), async {
                                    me2.connect_gated(&id).await
                                })
                                .await;
                                match r {
                                    Ok(Ok(_)) => {
                                        println!("[terra] auto-connected {id}");
                                        js(&me2.app, "autoConnected", vec![json!(id)]);
                                    }
                                    other => eprintln!("[terra] auto-connect {id} failed: {other:?}"),
                                }
                                me2.connecting.lock().await.remove(&id);
                            });
                        }
                    }
                }
                let want_scan = missing || me.native_scanning.load(Ordering::SeqCst);
                if want_scan {
                    if me.connect_gate.try_lock().is_ok() {
                        let res = call_ble_manager("startScan", "()Z", &[]);
                        let started = res.ok().as_deref() == Some("true");
                        if started && !scanning {
                            println!("[terra] scanning for BLE devices (missing={missing}, native={})", me.native_scanning.load(Ordering::SeqCst));
                        }
                        scanning = started;
                    }
                } else if scanning {
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

    async fn remembered_match(
        &self,
        filters: &[Filter],
        excl: &[Filter],
        all: bool,
        exclude_ids: &[String],
    ) -> Option<DeviceInfo> {
        let known = self.known.lock().await.clone();
        if known.is_empty() {
            return None;
        }
        let released = self.released.lock().await.clone();
        let excluded_names = crate::addons::bt_exclude_names(&self.app);
        let mut best: Option<(u8, DiscoveredProps)> = None;
        for mut props in self.discovered_devices() {
            if exclude_ids.iter().any(|x| x == &props.id) {
                continue;
            }
            let Some(k) = known
                .iter()
                .find(|k| k.id == props.id && k.owner.as_ref().map_or(true, |o| released.contains(o)))
            else {
                continue;
            };
            if k.name.as_deref().map_or(false, |n| excluded_names.contains(n)) {
                continue;
            }
            if props.name.is_none() {
                props.name = k.name.clone();
            }
            if props.name.as_deref().map_or(false, |n| excluded_names.contains(n)) {
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
        exclude_ids: Vec<String>,
    ) -> Result<DeviceInfo, String> {
        alog(&format!("request_device called: silent={} accept_all={} owner={:?}", silent, accept_all, owner));
        if owner.is_none() && silent {
            if let Some(info) = self.remembered_match(&filters, &exclusions, accept_all, &exclude_ids).await {
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
            let w = chooser_matches(&props, &filters, &excl, all);
            alog(&format!("device id={} name={:?} chooser_match={}", props.id, props.name, w));
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

    pub async fn set_native_scan(&self, on: bool) -> Result<(), String> {
        self.native_scanning.store(on, Ordering::SeqCst);
        if on {
            if self.connect_gate.try_lock().is_ok() {
                let _ = call_ble_manager("startScan", "()Z", &[]);
            }
        } else if self.pending.lock().await.is_none() {
            let known = self.known.lock().await.clone();
            let excluded = crate::addons::bt_exclude_names(&self.app);
            let any_missing = known.iter().any(|k| {
                !k.name.as_deref().map_or(false, |n| excluded.contains(n))
                    && !self.is_connected(&k.id)
            });
            if !any_missing {
                let _ = call_ble_manager("stopScan", "()V", &[]);
            }
        }
        Ok(())
    }

    pub async fn remember_device(&self, id: String, name: Option<String>) -> Result<(), String> {
        let resolved_name = match name {
            Some(n) if !n.is_empty() => Some(n),
            _ => self
                .discovered_devices()
                .into_iter()
                .find(|d| d.id == id)
                .and_then(|d| d.name),
        };
        self.remember(&DeviceInfo { id, name: resolved_name }, None).await;
        Ok(())
    }

    pub async fn scan_candidates(&self, filter_uuids: Vec<String>) -> Vec<BleDeviceStatus> {
        let known = self.known.lock().await.clone();
        let connecting = self.connecting.lock().await.clone();
        let released = self.released.lock().await.clone();
        let excluded_names = crate::addons::bt_exclude_names(&self.app);
        let filters: Vec<Filter> = filter_uuids
            .into_iter()
            .map(|u| Filter { services: Some(vec![u]), name: None, name_prefix: None })
            .collect();

        let mut seen = HashSet::new();
        let mut items = self.discovered_devices();
        for d in &items {
            seen.insert(d.id.clone());
        }
        // Include remembered non-addon devices that are currently connected or connecting
        // even if they haven't emitted an advertisement packet in the current window.
        for k in &known {
            if seen.contains(&k.id) {
                continue;
            }
            if self.is_connected(&k.id) || connecting.contains(&k.id) {
                items.push(DiscoveredProps {
                    id: k.id.clone(),
                    name: k.name.clone(),
                    rssi: Some(-50),
                    services: vec![],
                });
            }
        }

        let mut out: Vec<(u8, Option<i16>, BleDeviceStatus)> = Vec::new();
        for mut props in items {
            let id = props.id.clone();
            let connected = self.is_connected(&id);
            let is_connecting = connecting.contains(&id);
            let known_entry = known.iter().find(|k| k.id == id);
            if let Some(k) = known_entry {
                if k.owner.as_ref().map_or(false, |o| !released.contains(o)) {
                    continue;
                }
                if k.name.as_deref().map_or(false, |n| excluded_names.contains(n)) {
                    continue;
                }
            }
            if props.name.is_none() {
                props.name = known_entry.and_then(|k| k.name.clone());
            }
            if props.name.as_deref().map_or(false, |n| excluded_names.contains(n)) {
                continue;
            }
            let remembered = known_entry.is_some();
            if !remembered && !filters.is_empty() && !chooser_matches(&props, &filters, &[], false) {
                continue;
            }
            let r = rank(&props);
            out.push((
                r,
                props.rssi,
                BleDeviceStatus {
                    id,
                    name: props.name,
                    owner: known_entry.and_then(|k| k.owner.clone()),
                    connected,
                    connecting: is_connecting,
                    remembered,
                },
            ));
        }
        out.sort_by(|a, b| {
            b.2.connected
                .cmp(&a.2.connected)
                .then(b.2.remembered.cmp(&a.2.remembered))
                .then(a.0.cmp(&b.0))
                .then(b.1.cmp(&a.1))
        });
        out.into_iter().map(|(_, _, s)| s).collect()
    }

    async fn connect_gated(&self, id: &str) -> Result<Vec<ServiceInfo>, String> {
        // Don't grab `connect_gate` or stop BLE scanning for a device that isn't even
        // connected or advertising right now (e.g. a switched-off remembered Zwift Ride
        // controller being retried by an addon). Mirrors desktop `self.get(id)?`.
        if !self.is_connected(id) && !self.discovered_devices().iter().any(|d| d.id == id) {
            if self.connect_gate.try_lock().is_ok() {
                let _ = call_ble_manager("startScan", "()Z", &[]);
            }
            return Err(format!("Device {id} not currently advertising"));
        }
        let _gate = self.connect_gate.lock().await;
        let id_owned = id.to_string();
        let res = tokio::task::spawn_blocking(move || {
            let vm = get_java_vm()?;
            let env = vm.attach_current_thread().map_err(err)?;
            let j_id = env.new_string(&id_owned).map_err(err)?;
            let res = call_ble_manager(
                "connect",
                "(Ljava/lang/String;)Ljava/lang/String;",
                &[jni::objects::JValue::Object(&j_id)],
            )?;
            if res.starts_with("Error:") {
                return Err(res);
            }
            serde_json::from_str(&res).map_err(err)
        })
        .await
        .map_err(err)?;
        drop(_gate);
        // Resume scanning immediately if native scan is still active and no other connect is queued.
        if self.native_scanning.load(Ordering::SeqCst) && self.connect_gate.try_lock().is_ok() {
            let _ = call_ble_manager("startScan", "()Z", &[]);
        }
        res
    }

    pub async fn connect(&self, id: &str) -> Result<Vec<ServiceInfo>, String> {
        let inserted = self.connecting.lock().await.insert(id.to_string());
        let res = self.connect_gated(id).await;
        if inserted {
            self.connecting.lock().await.remove(id);
        }
        res
    }

    pub async fn connect_with_wait(&self, id: &str, _wait_visible: bool) -> Result<Vec<ServiceInfo>, String> {
        self.connect(id).await
    }

    pub async fn disconnect(&self, id: &str) -> Result<(), String> {
        let is_known = self.known.lock().await.iter().any(|k| k.id == id);
        let id_owned = id.to_string();
        let _ = tokio::task::spawn_blocking(move || {
            let Ok(vm) = get_java_vm() else { return };
            let Ok(env) = vm.attach_current_thread() else { return };
            let Ok(j_id) = env.new_string(&id_owned) else { return };
            if is_known {
                let _ = call_ble_manager("unsubscribeAll", "(Ljava/lang/String;)Ljava/lang/String;", &[jni::objects::JValue::Object(&j_id)]);
            } else {
                let _ = call_ble_manager("disconnect", "(Ljava/lang/String;)V", &[jni::objects::JValue::Object(&j_id)]);
            }
        })
        .await;
        Ok(())
    }

    pub async fn read(&self, id: &str, s: &str, c: &str) -> Result<String, String> {
        let (id, s, c) = (id.to_string(), s.to_string(), c.to_string());
        tokio::task::spawn_blocking(move || {
            let vm = get_java_vm()?;
            let env = vm.attach_current_thread().map_err(err)?;
            let j_id = env.new_string(&id).map_err(err)?;
            let j_s = env.new_string(&s).map_err(err)?;
            let j_c = env.new_string(&c).map_err(err)?;
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
        })
        .await
        .map_err(err)?
    }

    pub async fn write(&self, id: &str, s: &str, c: &str, value: &str, with_response: bool) -> Result<(), String> {
        let (id, s, c, value) = (id.to_string(), s.to_string(), c.to_string(), value.to_string());
        tokio::task::spawn_blocking(move || {
            let vm = get_java_vm()?;
            let env = vm.attach_current_thread().map_err(err)?;
            let j_id = env.new_string(&id).map_err(err)?;
            let j_s = env.new_string(&s).map_err(err)?;
            let j_c = env.new_string(&c).map_err(err)?;
            let j_v = env.new_string(&value).map_err(err)?;
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
        })
        .await
        .map_err(err)?
    }

    pub async fn subscribe(&self, id: &str, s: &str, c: &str, on: bool) -> Result<(), String> {
        let (id, s, c) = (id.to_string(), s.to_string(), c.to_string());
        tokio::task::spawn_blocking(move || {
            let vm = get_java_vm()?;
            let env = vm.attach_current_thread().map_err(err)?;
            let j_id = env.new_string(&id).map_err(err)?;
            let j_s = env.new_string(&s).map_err(err)?;
            let j_c = env.new_string(&c).map_err(err)?;
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
        })
        .await
        .map_err(err)?
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
                if info.name.is_some() {
                    e.name = info.name.clone();
                }
                if owner.is_some() {
                    e.owner = owner;
                }
            }
            None => k.push(Remembered { id: info.id.clone(), name: info.name.clone(), owner }),
        }
        self.save_known(&k);
    }

    /// No-op counterpart of the desktop `disconnect_all`: Android reclaims GATT
    /// connections when the app is destroyed, and we have no list of handles here.
    pub async fn disconnect_all(&self) -> Result<(), String> {
        let ids = self.known.lock().await.iter().map(|k| k.id.clone()).collect::<Vec<_>>();
        for id in ids {
            let _ = self.disconnect(&id).await;
        }
        Ok(())
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

    pub async fn drop_device(&self, id: &str) -> Result<(), String> {
        if let Ok(vm) = get_java_vm() {
            if let Ok(env) = vm.attach_current_thread() {
                if let Ok(j_id) = env.new_string(id) {
                    let _ = call_ble_manager("disconnect", "(Ljava/lang/String;)V", &[jni::objects::JValue::Object(&j_id)]);
                }
            }
        }
        js(&self.app, "disconnected", vec![json!(id)]);
        Ok(())
    }

    pub async fn drop_all(&self) -> Result<(), String> {
        let ids = self.known.lock().await.iter().map(|k| k.id.clone()).collect::<Vec<_>>();
        for id in ids {
            let _ = self.drop_device(&id).await;
        }
        Ok(())
    }

    pub async fn devices_status(&self) -> Vec<BleDeviceStatus> {
        let known = self.known.lock().await.clone();
        let connecting = self.connecting.lock().await.clone();
        known
            .into_iter()
            .map(|k| {
                let connected = self.is_connected(&k.id);
                let is_connecting = connecting.contains(&k.id);
                BleDeviceStatus {
                    id: k.id,
                    name: k.name,
                    owner: k.owner,
                    connected,
                    connecting: is_connecting,
                    remembered: true,
                }
            })
            .collect()
    }
}

/// Failed GATT commands are always printed (they used to vanish: the page just saw a rejected promise).
#[cfg_attr(any(target_os = "linux", target_os = "windows"), allow(dead_code))]
fn logged<T>(what: &str, id: &str, r: Result<T, String>) -> Result<T, String> {
    if let Err(e) = &r {
        eprintln!("[ble] {what} {id} FAILED: {e}");
    }
    r
}

// ---------- tauri commands ----------

#[derive(Clone)]
pub struct Ble(pub Option<Arc<Broker>>);

pub fn broker<'a>(s: &'a State<'_, Ble>) -> Result<&'a Arc<Broker>, String> {
    s.0.as_ref().ok_or_else(|| "no bluetooth adapter available".to_string())
}

#[cfg_attr(not(any(target_os = "linux", target_os = "windows")), tauri::command)]
#[cfg_attr(any(target_os = "linux", target_os = "windows"), allow(dead_code))]
pub async fn ble_request_device(
    state: State<'_, Ble>,
    filters: Vec<Filter>,
    exclusion_filters: Option<Vec<Filter>>,
    accept_all: bool,
    silent: Option<bool>,
    owner: Option<String>,
    exclude_ids: Option<Vec<String>>,
) -> Result<DeviceInfo, String> {
    broker(&state)?
        .request_device(
            filters,
            exclusion_filters.unwrap_or_default(),
            accept_all,
            silent.unwrap_or(false),
            owner,
            exclude_ids.unwrap_or_default(),
        )
        .await
}

/// Devices an addon paired earlier (so it can restore them without any UI).
#[cfg_attr(not(any(target_os = "linux", target_os = "windows")), tauri::command)]
#[cfg_attr(any(target_os = "linux", target_os = "windows"), allow(dead_code))]
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
#[cfg_attr(not(any(target_os = "linux", target_os = "windows")), tauri::command)]
#[cfg_attr(any(target_os = "linux", target_os = "windows"), allow(dead_code))]
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

#[cfg_attr(not(any(target_os = "linux", target_os = "windows")), tauri::command)]
#[cfg_attr(any(target_os = "linux", target_os = "windows"), allow(dead_code))]
pub async fn ble_connect(
    state: State<'_, Ble>,
    id: String,
    wait_visible: Option<bool>,
) -> Result<Vec<ServiceInfo>, String> {
    let r = broker(&state)?.connect_with_wait(&id, wait_visible.unwrap_or(false)).await;
    logged("connect", &id, r)
}

#[cfg_attr(not(any(target_os = "linux", target_os = "windows")), tauri::command)]
#[cfg_attr(any(target_os = "linux", target_os = "windows"), allow(dead_code))]
pub async fn ble_native_scan_start(state: State<'_, Ble>) -> Result<(), String> {
    broker(&state)?.set_native_scan(true).await
}

#[cfg_attr(not(any(target_os = "linux", target_os = "windows")), tauri::command)]
#[cfg_attr(any(target_os = "linux", target_os = "windows"), allow(dead_code))]
pub async fn ble_native_scan_stop(state: State<'_, Ble>) -> Result<(), String> {
    broker(&state)?.set_native_scan(false).await
}

#[cfg_attr(not(any(target_os = "linux", target_os = "windows")), tauri::command)]
#[cfg_attr(any(target_os = "linux", target_os = "windows"), allow(dead_code))]
pub async fn ble_scan_candidates(
    state: State<'_, Ble>,
    filter_uuids: Option<Vec<String>>,
) -> Result<Vec<BleDeviceStatus>, String> {
    Ok(broker(&state)?.scan_candidates(filter_uuids.unwrap_or_default()).await)
}

#[cfg_attr(not(any(target_os = "linux", target_os = "windows")), tauri::command)]
#[cfg_attr(any(target_os = "linux", target_os = "windows"), allow(dead_code))]
pub async fn ble_remember(
    state: State<'_, Ble>,
    id: String,
    name: Option<String>,
) -> Result<(), String> {
    broker(&state)?.remember_device(id, name).await
}

#[cfg_attr(not(any(target_os = "linux", target_os = "windows")), tauri::command)]
#[cfg_attr(any(target_os = "linux", target_os = "windows"), allow(dead_code))]
pub async fn ble_disconnect(state: State<'_, Ble>, id: String) -> Result<(), String> {
    broker(&state)?.disconnect(&id).await
}

#[cfg_attr(not(any(target_os = "linux", target_os = "windows")), tauri::command)]
#[cfg_attr(any(target_os = "linux", target_os = "windows"), allow(dead_code))]
pub async fn ble_disconnect_all(state: State<'_, Ble>) -> Result<(), String> {
    broker(&state)?.disconnect_all().await
}

#[cfg_attr(not(any(target_os = "linux", target_os = "windows")), tauri::command)]
#[cfg_attr(any(target_os = "linux", target_os = "windows"), allow(dead_code))]
pub async fn ble_forget(state: State<'_, Ble>, id: String) -> Result<(), String> {
    broker(&state)?.forget(&id).await
}

#[cfg_attr(not(any(target_os = "linux", target_os = "windows")), tauri::command)]
#[cfg_attr(any(target_os = "linux", target_os = "windows"), allow(dead_code))]
pub async fn ble_devices_status(state: State<'_, Ble>) -> Result<Vec<BleDeviceStatus>, String> {
    Ok(broker(&state)?.devices_status().await)
}

#[cfg_attr(not(any(target_os = "linux", target_os = "windows")), tauri::command)]
#[cfg_attr(any(target_os = "linux", target_os = "windows"), allow(dead_code))]
pub async fn ble_drop_device(state: State<'_, Ble>, id: String) -> Result<(), String> {
    broker(&state)?.drop_device(&id).await
}

#[cfg_attr(not(any(target_os = "linux", target_os = "windows")), tauri::command)]
#[cfg_attr(any(target_os = "linux", target_os = "windows"), allow(dead_code))]
pub async fn ble_drop_all(state: State<'_, Ble>) -> Result<(), String> {
    broker(&state)?.drop_all().await
}

#[cfg_attr(not(any(target_os = "linux", target_os = "windows")), tauri::command)]
#[cfg_attr(any(target_os = "linux", target_os = "windows"), allow(dead_code))]
pub async fn ble_read(state: State<'_, Ble>, id: String, service: String, characteristic: String) -> Result<String, String> {
    let r = broker(&state)?.read(&id, &service, &characteristic).await;
    logged(&format!("read {characteristic}"), &id, r)
}

#[cfg_attr(not(any(target_os = "linux", target_os = "windows")), tauri::command)]
#[cfg_attr(any(target_os = "linux", target_os = "windows"), allow(dead_code))]
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

#[cfg_attr(not(any(target_os = "linux", target_os = "windows")), tauri::command)]
#[cfg_attr(any(target_os = "linux", target_os = "windows"), allow(dead_code))]
pub async fn ble_subscribe(state: State<'_, Ble>, id: String, service: String, characteristic: String) -> Result<(), String> {
    let r = broker(&state)?.subscribe(&id, &service, &characteristic, true).await;
    logged(&format!("subscribe {characteristic}"), &id, r)
}

#[cfg_attr(not(any(target_os = "linux", target_os = "windows")), tauri::command)]
#[cfg_attr(any(target_os = "linux", target_os = "windows"), allow(dead_code))]
pub async fn ble_unsubscribe(state: State<'_, Ble>, id: String, service: String, characteristic: String) -> Result<(), String> {
    let r = broker(&state)?.subscribe(&id, &service, &characteristic, false).await;
    logged(&format!("unsubscribe {characteristic}"), &id, r)
}

#[cfg_attr(not(any(target_os = "linux", target_os = "windows")), tauri::command)]
#[cfg_attr(any(target_os = "linux", target_os = "windows"), allow(dead_code))]
pub async fn chooser_candidates(state: State<'_, Ble>) -> Result<Vec<Candidate>, String> {
    Ok(broker(&state)?.candidates().await)
}

#[cfg_attr(not(any(target_os = "linux", target_os = "windows")), tauri::command)]
#[cfg_attr(any(target_os = "linux", target_os = "windows"), allow(dead_code))]
pub async fn chooser_pick(state: State<'_, Ble>, id: String) -> Result<(), String> {
    broker(&state)?.resolve(Some(id)).await;
    Ok(())
}

#[cfg_attr(not(any(target_os = "linux", target_os = "windows")), tauri::command)]
#[cfg_attr(any(target_os = "linux", target_os = "windows"), allow(dead_code))]
pub async fn chooser_cancel(state: State<'_, Ble>) -> Result<(), String> {
    broker(&state)?.resolve(None).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props(name: Option<&str>, services: &[&str]) -> DiscoveredProps {
        DiscoveredProps {
            id: "AA:BB:CC:DD:EE:FF".into(),
            name: name.map(String::from),
            rssi: Some(-60),
            services: services.iter().map(|s| Uuid::parse_str(s).unwrap()).collect(),
        }
    }

    #[test]
    fn filter_matches_short_and_full_uuid_and_name() {
        let p = props(
            Some("KICKR CORE 1234"),
            &["00001826-0000-1000-8000-00805f9b34fb"],
        );
        let f_short = Filter { services: Some(vec!["1826".into()]), ..Default::default() };
        let f_full = Filter {
            services: Some(vec!["00001826-0000-1000-8000-00805f9b34fb".into()]),
            ..Default::default()
        };
        let f_prefix = Filter { name_prefix: Some("KICKR".into()), ..Default::default() };
        let f_other = Filter { services: Some(vec!["180d".into()]), ..Default::default() };

        assert!(f_short.matches(&p));
        assert!(f_full.matches(&p));
        assert!(f_prefix.matches(&p));
        assert!(!f_other.matches(&p));
    }

    #[test]
    fn chooser_is_strict_while_wanted_is_lenient() {
        let phone = props(Some("My Phone"), &[]);
        let ftms_filter = vec![Filter { services: Some(vec!["1826".into()]), ..Default::default() }];

        assert!(!chooser_matches(&phone, &ftms_filter, &[], false));
        assert!(wanted(&phone, &ftms_filter, &[], false));

        let excl = vec![Filter { name_prefix: Some("My ".into()), ..Default::default() }];
        assert!(!wanted(&phone, &ftms_filter, &excl, false));
        assert!(!chooser_matches(&phone, &[], &excl, true));
    }

    #[test]
    fn rank_orders_trainer_before_power_csc_hr() {
        let trainer = props(None, &["00001826-0000-1000-8000-00805f9b34fb"]);
        let power = props(None, &["00001818-0000-1000-8000-00805f9b34fb"]);
        let csc = props(None, &["00001816-0000-1000-8000-00805f9b34fb"]);
        let hr = props(None, &["0000180d-0000-1000-8000-00805f9b34fb"]);
        let other = props(None, &[]);

        assert_eq!(rank(&trainer), 0);
        assert_eq!(rank(&power), 1);
        assert_eq!(rank(&csc), 2);
        assert_eq!(rank(&hr), 3);
        assert_eq!(rank(&other), 4);
    }

    #[test]
    fn ftms_cp_decoding() {
        assert_eq!(ftms_cp(&[0x00]).as_deref(), Some("RequestControl"));
        assert_eq!(ftms_cp(&[0x05, 0xFA, 0x00]).as_deref(), Some("SetTargetPower 250 W"));
        assert_eq!(ftms_cp(&[0x80, 0x05, 0x01]).as_deref(), Some("Response to 0x05: success"));
        assert_eq!(ftms_cp(&[]), None);
    }
}
