//! Local socket that feeds the terminal UI (`terra --tui`). Linux only.
//!
//! The `--rpc` backend listens on a user-private Unix socket (see [`socket_path`]) and streams
//! newline-delimited JSON to every connected TUI client:
//!
//! ```text
//! -> {"event":"hello","version":"..","url":"..","latest":<frame|null>,"history":[..],"route":<..>,"world":<..>,"host":<..>}
//! -> {"event":"ride","data":<frame>}          every biketerra-ride broadcast frame (~2Hz)
//! -> {"event":"route","data":<route>}         full route polyline + elevation from btApp
//! -> {"event":"world","data":<world>}         all riders' world/route positions from btApp (~4Hz)
//! -> {"event":"host","data":<host>}           window visibility + 3D render mode
//! -> {"event":"page","url":".."}              page navigations (reported by Electron)
//! ```
//!
//! Clients may send back a small, fixed set of commands. Never raw JS:
//!
//! ```text
//! <- {"cmd":"window","action":"show"|"hide"|"toggle"}
//! <- {"cmd":"render_mode","mode":"auto"|"off"|"1fps"|"full"}
//! <- {"cmd":"focus_rider","id":"...","name":"..."}
//! <- {"cmd":"navigate","path":"/some/path"}   (resolved against https://biketerra.com only)
//! <- {"cmd":"reload"}
//! ```
//!
//! Commands are forwarded to Electron as `{"event":"tui_cmd",...}` lines on stdout, using the
//! same channel as `js_eval` events.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::OwnedWriteHalf;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::broadcast;

use crate::linux_runtime::AppHandle;

#[derive(Clone)]
pub struct TuiHub {
    tx: broadcast::Sender<String>,
    url: Arc<Mutex<String>>,
    route: Arc<Mutex<Option<Value>>>,
    world: Arc<Mutex<Option<Value>>>,
    host: Arc<Mutex<Option<Value>>>,
    clients: Arc<AtomicUsize>,
}

impl TuiHub {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(256);
        Self {
            tx,
            url: Arc::new(Mutex::new(String::new())),
            route: Arc::new(Mutex::new(None)),
            world: Arc::new(Mutex::new(None)),
            host: Arc::new(Mutex::new(None)),
            clients: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Fan a ride broadcast frame out to connected clients (no-op when none are attached).
    pub fn publish_ride(&self, data: &Value) {
        if self.tx.receiver_count() == 0 {
            return;
        }
        let _ = self.tx.send(json!({ "event": "ride", "data": data }).to_string());
    }

    /// Store the latest route geometry and fan out to connected clients.
    pub fn set_route(&self, data: Value) {
        if let Ok(mut g) = self.route.lock() {
            *g = Some(data.clone());
        }
        if self.tx.receiver_count() > 0 {
            let _ = self.tx.send(json!({ "event": "route", "data": data }).to_string());
        }
    }

    /// Store the latest btApp world snapshot (all riders + 3D status) and fan out.
    pub fn set_world(&self, data: Value) {
        if let Ok(mut g) = self.world.lock() {
            *g = Some(data.clone());
        }
        if self.tx.receiver_count() > 0 {
            let _ = self.tx.send(json!({ "event": "world", "data": data }).to_string());
        }
    }

    /// Store Electron host state (`windowHidden`, `renderMode`) and fan out.
    pub fn set_host_state(&self, data: Value) {
        if let Ok(mut g) = self.host.lock() {
            *g = Some(data.clone());
        }
        if self.tx.receiver_count() > 0 {
            let _ = self.tx.send(json!({ "event": "host", "data": data }).to_string());
        }
    }

    /// Record the page URL and tell connected clients.
    pub fn set_page(&self, url: String) {
        {
            let mut g = self.url.lock().unwrap_or_else(|p| p.into_inner());
            if *g == url {
                return;
            }
            *g = url.clone();
        }
        // Clear cached route/world when leaving /ride or /spectate
        if !url.contains("/ride") && !url.contains("/spectate") {
            if let Ok(mut g) = self.route.lock() {
                *g = None;
            }
            if let Ok(mut g) = self.world.lock() {
                *g = None;
            }
        }
        if self.tx.receiver_count() > 0 {
            let _ = self.tx.send(json!({ "event": "page", "url": url }).to_string());
        }
    }

    fn page(&self) -> String {
        self.url.lock().map(|g| g.clone()).unwrap_or_default()
    }

    fn route(&self) -> Option<Value> {
        self.route.lock().ok().and_then(|g| g.clone())
    }

    fn world(&self) -> Option<Value> {
        self.world.lock().ok().and_then(|g| g.clone())
    }

    fn host(&self) -> Option<Value> {
        self.host.lock().ok().and_then(|g| g.clone())
    }

    fn on_client_connect(&self) {
        let count = self.clients.fetch_add(1, Ordering::SeqCst) + 1;
        emit_event(&json!({ "event": "tui_cmd", "cmd": "client_count", "count": count }));
    }

    fn on_client_disconnect(&self) {
        let prev = self.clients.fetch_sub(1, Ordering::SeqCst);
        let count = prev.saturating_sub(1);
        emit_event(&json!({ "event": "tui_cmd", "cmd": "client_count", "count": count }));
    }
}

/// `$TERRA_TUI_SOCK`, else `$XDG_RUNTIME_DIR/terra-tui.sock` (a 0700 per-user dir), else
/// `/tmp/terra-tui-$USER.sock`.
pub fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("TERRA_TUI_SOCK") {
        return PathBuf::from(p);
    }
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(dir).join("terra-tui.sock");
    }
    let user = std::env::var("USER").unwrap_or_else(|_| "user".into());
    std::env::temp_dir().join(format!("terra-tui-{user}.sock"))
}

/// Remove the socket file (called when the backend shuts down).
pub fn cleanup() {
    let _ = std::fs::remove_file(socket_path());
}

/// Accept TUI clients forever. Best-effort: a bind failure only disables the TUI feed.
pub async fn serve(app: AppHandle) {
    let path = socket_path();
    let _ = std::fs::remove_file(&path); // stale socket from a previous (crashed) backend
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[tui] cannot bind {}: {e} (TUI feed disabled)", path.display());
            return;
        }
    };
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    eprintln!("[tui] listening on {}", path.display());

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                tokio::spawn(handle_client(app.clone(), stream));
            }
            Err(e) => {
                eprintln!("[tui] accept failed: {e}");
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        }
    }
}

async fn write_line(w: &mut OwnedWriteHalf, s: &str) -> std::io::Result<()> {
    w.write_all(s.as_bytes()).await?;
    w.write_all(b"\n").await
}

struct ClientGuard(TuiHub);
impl Drop for ClientGuard {
    fn drop(&mut self) {
        self.0.on_client_disconnect();
    }
}

async fn handle_client(app: AppHandle, stream: UnixStream) {
    let Some(hub) = app.try_state::<TuiHub>().map(|s| s.into_inner()) else { return };
    hub.on_client_connect();
    let _guard = ClientGuard(hub.clone());

    // Subscribe before taking the snapshot so no frame falls in between.
    let mut rx = hub.tx.subscribe();
    let (r, mut w) = stream.into_split();

    let (latest, history) = match app.try_state::<crate::ride::Ride>() {
        Some(ride) => {
            // Drop the per-frame rider lists from the backlog: the client only graphs the ego,
            // and full frames for five minutes of a busy ride would be several MB.
            let hist: Vec<Value> = ride
                .history(None)
                .into_iter()
                .map(|f| json!({ "ego": f.get("ego"), "timestamp": f.get("timestamp") }))
                .collect();
            (ride.latest(), hist)
        }
        None => (None, Vec::new()),
    };
    let hello = json!({
        "event": "hello",
        "version": env!("CARGO_PKG_VERSION"),
        "url": hub.page(),
        "latest": latest,
        "history": history,
        "route": hub.route(),
        "world": hub.world(),
        "host": hub.host(),
    });
    if write_line(&mut w, &hello.to_string()).await.is_err() {
        return;
    }

    let mut lines = BufReader::new(r).lines();
    loop {
        tokio::select! {
            msg = rx.recv() => match msg {
                Ok(s) => {
                    if write_line(&mut w, &s).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            },
            line = lines.next_line() => match line {
                Ok(Some(l)) => handle_command(&l),
                _ => break,
            },
        }
    }
}

/// Validate a client command and forward it to Electron. Unknown/invalid input is ignored.
fn handle_command(line: &str) {
    let Ok(v) = serde_json::from_str::<Value>(line.trim()) else { return };
    let ev = match v.get("cmd").and_then(|c| c.as_str()) {
        Some("window") => match v.get("action").and_then(|a| a.as_str()) {
            Some(a @ ("show" | "hide" | "toggle")) => json!({ "event": "tui_cmd", "cmd": "window", "action": a }),
            _ => return,
        },
        Some("render_mode") => match v.get("mode").and_then(|m| m.as_str()) {
            Some(m @ ("auto" | "off" | "1fps" | "full")) => json!({ "event": "tui_cmd", "cmd": "render_mode", "mode": m }),
            _ => return,
        },
        Some("focus_rider") => {
            let id = v.get("id").and_then(|x| x.as_str()).unwrap_or("");
            let name = v.get("name").and_then(|x| x.as_str()).unwrap_or("");
            if (id.is_empty() && name.is_empty()) || id.len() > 128 || name.len() > 128 {
                return;
            }
            json!({ "event": "tui_cmd", "cmd": "focus_rider", "id": id, "name": name })
        }
        Some("navigate") => match v.get("path").and_then(|p| p.as_str()) {
            // Site-relative paths only; Electron re-checks the origin after resolving.
            Some(p) if p.starts_with('/') && !p.starts_with("//") && p.len() < 512 => {
                json!({ "event": "tui_cmd", "cmd": "navigate", "path": p })
            }
            _ => return,
        },
        Some("reload") => json!({ "event": "tui_cmd", "cmd": "reload" }),
        _ => return,
    };
    emit_event(&ev);
}

/// Write one event line to stdout for Electron (same framing as RPC responses).
fn emit_event(v: &Value) {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{v}");
    let _ = out.flush();
}
