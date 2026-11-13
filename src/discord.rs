//! Discord Rich Presence, built into the Terra core (no Node sidecar).
//!
//! Connects to the local Discord desktop client over its Unix-domain IPC socket
//! and issues `SET_ACTIVITY`. This is a port of the proven reference
//! implementation that shipped as the `bridge.js` Node sidecar; the difference is
//! the transport path — the addon now calls `terra.discord.*` and core owns the
//! socket directly.
//!
//! Wire protocol (8-byte little-endian header):
//!   [ opcode : u32 ][ payloadLength : u32 ][ JSON payload : payloadLength bytes ]
//! Opcodes: 0 HANDSHAKE, 1 FRAME, 2 CLOSE, 3 PING, 4 PONG. A session is
//! established with a HANDSHAKE frame `{"v":1,"client_id":...}` (Discord answers
//! with a READY dispatch); every command is a FRAME carrying a `nonce` that is
//! echoed back in the correlated response.
//!
//! ## Why one connection is held open (not one per call)
//!
//! Discord drops a client's activity the moment its IPC connection closes. So a
//! "connect → SET_ACTIVITY → close" design makes the presence flash for a
//! fraction of a second on every update and then vanish — exactly the symptom we
//! were seeing. The fix is to hold ONE connection alive for as long as it is
//! needed: a background "owner" task is the sole reader/writer of the socket,
//! answers Discord's PINGs, and routes each command's response back to the caller.
//! All `SET_ACTIVITY`/clear calls multiplex over that one connection. If the
//! connection ever drops (Discord restart, socket EOF, a failed write), the next
//! call transparently re-establishes it.
//!
//! The client id and activity come from the addon (which stores them in its
//! settings); core does the socket I/O. Only the logged-in-username, the live
//! connection flag, and the most recent outcome are retained, for
//! `discord_status`.
//!
//! Linux for now (Discord desktop exposes a Unix socket). Other platforms return
//! a clear "not available" error so the addon degrades gracefully.

use std::sync::{Arc, Mutex};

#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{json, Value};

#[cfg(not(target_os = "linux"))]
use tauri::{AppHandle, State};
#[cfg(target_os = "linux")]
use crate::linux_runtime::{AppHandle, State};

const OP_HANDSHAKE: u32 = 0;
const OP_FRAME: u32 = 1;
const OP_PING: u32 = 3;
const OP_PONG: u32 = 4;

const SOCKET_NAMES: u32 = 10; // discord-ipc-0 .. discord-ipc-9

/// One held-open IPC connection to Discord, shared by all pushes.
///
/// A background "owner" task (see `owner_loop`) is the sole reader/writer of the
/// underlying socket; commands hand it a `WriteReq` and get the correlated
/// response back on the matching oneshot. `alive` is cleared by the owner the
/// moment the socket drops, so the next call re-establishes a fresh session.
#[cfg(target_os = "linux")]
#[derive(Clone)]
struct ConnectionHandle {
    write_tx: tokio::sync::mpsc::Sender<WriteReq>,
    alive: Arc<AtomicBool>,
    user: Option<String>,
}

/// A single SET_ACTIVITY request handed to the connection owner, plus the channel
/// the correlated response is returned on.
#[cfg(target_os = "linux")]
struct WriteReq {
    client_id: String,
    nonce: String,
    activity: Option<Value>,
    reply: tokio::sync::oneshot::Sender<Result<Value, String>>,
}

/// Send one command over the held connection and await its response.
#[cfg(target_os = "linux")]
impl ConnectionHandle {
    async fn send_set(&self, client_id: &str, activity: Option<&Value>) -> Result<Value, String> {
        if !self.alive.load(Ordering::SeqCst) {
            // The owner already tore down (Discord restart / EOF / write failure).
            // Report plainly — the next call re-establishes under the conn mutex.
            return Err("no live Discord connection (will reconnect on the next push)".into());
        }
        let (tx, rx) = tokio::sync::oneshot::channel();
        let req = WriteReq {
            client_id: client_id.to_string(),
            nonce: next_nonce(),
            activity: activity.cloned(),
            reply: tx,
        };
        self.write_tx
            .send(req)
            .await
            .map_err(|_| "Discord connection dropped before the update could be sent".to_string())?;
        rx.await.map_err(|_| "Discord closed the connection before it answered".to_string())?
    }
}

/// Shared Discord client state (managed per AppHandle).
#[derive(Clone)]
pub struct Discord {
    inner: Arc<Inner>,
}

struct Inner {
    last_user: Mutex<Option<String>>,
    last_error: Mutex<Option<String>>,
    last_ok: Mutex<bool>,
    // Serialises SET_ACTIVITY calls so concurrent pushes (the ride-detect path and
    // the 5 s loop firing the same tick) don't both try to establish at once.
    // tokio's Mutex (not std's) so the guard can be held across an await.
    #[cfg(target_os = "linux")]
    send: tokio::sync::Mutex<()>,
    // The one live connection, held open across pushes (see module doc). `None`
    // means "not connected / dropped" — the next call re-establishes.
    #[cfg(target_os = "linux")]
    conn: tokio::sync::Mutex<Option<ConnectionHandle>>,
}

impl Discord {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                last_user: Mutex::new(None),
                last_error: Mutex::new(None),
                last_ok: Mutex::new(false),
                #[cfg(target_os = "linux")]
                send: tokio::sync::Mutex::new(()),
                #[cfg(target_os = "linux")]
                conn: tokio::sync::Mutex::new(None),
            }),
        }
    }

    fn inner(&self) -> &Inner {
        &self.inner
    }

    fn note(&self, user: Option<String>, ok: bool, err: Option<String>) {
        let inner = self.inner();
        if let Some(u) = user {
            *inner.last_user.lock().unwrap() = Some(u);
        }
        *inner.last_ok.lock().unwrap() = ok;
        *inner.last_error.lock().unwrap() = err;
    }
}

// ------------------------------------------------------------ public commands

/// Push (or update) the presence for the app whose Client ID is in `clientId`.
/// `activity` carries `details`, `state`, `start`, image keys and `buttons`.
#[cfg_attr(not(target_os = "linux"), tauri::command)]
pub async fn discord_set_activity(
    app: AppHandle,
    st: State<'_, Discord>,
    activity: Value,
) -> Result<Value, String> {
    let _ = app;
    let cid = activity
        .get("clientId")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            "missing clientId (set the addon's \"Discord application Client ID\")".to_string()
        })?;
    #[cfg(target_os = "linux")]
    {
        return set_activity(&*st, &cid, Some(sanitize(&activity))).await;
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (st, activity);
        return Err(
            "Discord Rich Presence isn't available on this platform yet (Linux only)".into(),
        );
    }
}

/// Drop the presence for the app `clientId` (activity = null).
#[cfg_attr(not(target_os = "linux"), tauri::command)]
pub async fn discord_clear_activity(
    app: AppHandle,
    st: State<'_, Discord>,
    client_id: String,
) -> Result<Value, String> {
    let _ = app;
    let cid = client_id.trim().to_string();
    if cid.is_empty() {
        return Err("missing clientId".into());
    }
    #[cfg(target_os = "linux")]
    {
        LOCKED_START_SEC.store(0, Ordering::SeqCst);
        return set_activity(&*st, &cid, None).await;
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = st;
        return Err(
            "Discord Rich Presence isn't available on this platform yet (Linux only)".into(),
        );
    }
}

/// Last known Discord state (who we logged in as, whether we hold a live
/// connection, and the most recent error).
#[cfg_attr(not(target_os = "linux"), tauri::command)]
pub async fn discord_status(app: AppHandle, st: State<'_, Discord>) -> Result<Value, String> {
    let _ = app;
    let inner = (&*st).inner();
    let last_user = inner.last_user.lock().unwrap().clone();
    let last_error = inner.last_error.lock().unwrap().clone();
    let ok = *inner.last_ok.lock().unwrap();
    let connected = {
        #[cfg(target_os = "linux")]
        {
            let g = inner.conn.lock().await;
            g.as_ref()
                .map(|c| c.alive.load(Ordering::SeqCst))
                .unwrap_or(false)
        }
        #[cfg(not(target_os = "linux"))]
        false
    };
    Ok(json!({ "ok": ok, "user": last_user, "connected": connected, "lastError": last_error }))
}

// --------------------------------------------------- protocol (Linux) core

#[cfg(target_os = "linux")]
fn next_nonce() -> String {
    use std::sync::atomic::{AtomicU64, Ordering as Seq};
    static N: AtomicU64 = AtomicU64::new(0);
    format!("terra-rpc-{}", N.fetch_add(1, Seq::SeqCst))
}

#[cfg(target_os = "linux")]
fn encode_frame(op: u32, body: &Value) -> Vec<u8> {
    let payload = serde_json::to_vec(body).unwrap_or_default();
    let mut buf = Vec::with_capacity(8 + payload.len());
    buf.extend_from_slice(&op.to_le_bytes());
    buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    buf.extend_from_slice(&payload);
    buf
}

#[cfg(target_os = "linux")]
fn error_message(body: &Value) -> String {
    if let Some(err) = body.get("error") {
        if let Some(m) = err.get("message").and_then(|x| x.as_str()) {
            return m.to_string();
        }
        if let Some(c) = err.get("code").and_then(|x| x.as_u64()) {
            return format!("Discord rejected the activity update (code {c})");
        }
        return "Discord rejected the activity update".to_string();
    }
    if let Some(m) = body.get("message").and_then(|x| x.as_str()) {
        return m.to_string();
    }
    "Discord rejected the activity update".to_string()
}

#[cfg(target_os = "linux")]
/// Resolve (fail) every command still awaiting a response.
fn fail_pending(
    pending: &mut std::collections::HashMap<String, tokio::sync::oneshot::Sender<Result<Value, String>>>,
    why: String,
) {
    // take() moves every (owned) sender out so we can consume it on .send();
    // a &mut Sender can't be moved out of in a closure.
    let drained = std::mem::take(pending);
    for (_nonce, tx) in drained {
        let _ = tx.send(Err(why.clone()));
    }
}

#[cfg(target_os = "linux")]
static LOCKED_START_SEC: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[cfg(target_os = "linux")]
/// Trim/normalize the addon's activity into what Discord's SET_ACTIVITY wants.
fn sanitize(v: &Value) -> Value {
    use serde_json::Map;
    use std::sync::atomic::Ordering as Seq;

    let m = v.as_object().cloned().unwrap_or_default();
    let mut a = Map::new();

    if let Some(d) = m.get("details").and_then(|x| x.as_str()) {
        if !d.is_empty() {
            a.insert("details".into(), Value::String(d.chars().take(128).collect()));
        }
    }
    if let Some(s) = m.get("state").and_then(|x| x.as_str()) {
        if !s.is_empty() {
            a.insert("state".into(), Value::String(s.chars().take(128).collect()));
        }
    }

    // Extract start timestamp from `start` or `timestamps.start`
    let incoming_start = m
    .get("start")
    .and_then(|x| x.as_u64().or_else(|| x.as_f64().map(|f| f as u64)))
    .or_else(|| {
        m.get("timestamps")
        .and_then(|t| t.get("start"))
        .and_then(|x| x.as_u64().or_else(|| x.as_f64().map(|f| f as u64)))
    });

    let mut sec = LOCKED_START_SEC.load(Seq::SeqCst);
    if sec == 0 {
        if let Some(st) = incoming_start {
            sec = if st > 100_000_000_000 { st / 1000 } else { st };
            LOCKED_START_SEC.store(sec, Seq::SeqCst);
        }
    }
    if sec > 0 {
        a.insert("timestamps".into(), json!({ "start": sec }));
    }

    let mut assets = Map::new();
    for (in_key, out_key) in [
        ("largeImage", "large_image"),
        ("largeText", "large_text"),
        ("smallImage", "small_image"),
        ("smallText", "small_text"),
    ] {
        if let Some(s) = m.get(in_key).and_then(|x| x.as_str()) {
            if !s.is_empty() {
                assets.insert(out_key.into(), Value::String(s.chars().take(128).collect()));
            }
        }
    }
    if !assets.is_empty() {
        a.insert("assets".into(), Value::Object(assets));
    }

    if let Some(btns) = m.get("buttons").and_then(|x| x.as_array()) {
        let cleaned: Vec<Value> = btns
        .iter()
        .filter_map(|b| {
            let bo = b.as_object()?;
            let label = bo.get("label")?.as_str()?.chars().take(32).collect::<String>();
            let url = bo.get("url")?.as_str()?;
            if label.is_empty() || !url.starts_with("https://") {
                return None;
            }
            Some(json!({ "label": label, "url": url.chars().take(512).collect::<String>() }))
        })
        .take(2)
        .collect();
        if !cleaned.is_empty() {
            a.insert("buttons".into(), Value::Array(cleaned));
        }
    }
    Value::Object(a)
}

#[cfg(target_os = "linux")]
/// Read exactly one framed message off an async reader. Kept generic so it works
/// on both a full `UnixStream` (during handshake) and a split `ReadHalf` (the
/// owner loop).
async fn read_frame<R>(stream: &mut R) -> Result<(u32, Value), String>
where
    R: tokio::io::AsyncRead + Unpin,
{
    use tokio::io::AsyncReadExt;
    let mut hdr = [0u8; 8];
    stream.read_exact(&mut hdr).await.map_err(|e| e.to_string())?;
    let op = u32::from_le_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]);
    let len = u32::from_le_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]) as usize;
    if len > 1_048_576 {
        return Err("Discord frame too large".into());
    }
    let mut buf = vec![0u8; len];
    if len > 0 {
        stream.read_exact(&mut buf).await.map_err(|e| e.to_string())?;
    }
    let body: Value = serde_json::from_slice(&buf)
        .unwrap_or(Value::String("unparseable Discord frame".into()));
    Ok((op, body))
}

#[cfg(target_os = "linux")]
/// Base directories where a running Discord may place its IPC socket, in
/// try order. Native XDG_RUNTIME_DIR/TMPDIR come first (highest priority).
/// Sandboxed Discord (Flatpak, Snap) keeps its socket in a per-app
/// subdirectory that is visible from the host's runtime dir while the
/// sandbox is running, so probing those from the host works fine.
fn search_roots() -> Vec<std::path::PathBuf> {
    use std::path::PathBuf;
    let mut bases: Vec<PathBuf> = Vec::new();
    for var in ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"] {
        if let Some(v) = std::env::var_os(var) {
            if !v.is_empty() {
                bases.push(PathBuf::from(v));
            }
        }
    }
    bases.push(PathBuf::from("/tmp"));

    if let Some(rd) = std::env::var_os("XDG_RUNTIME_DIR") {
        // Sandboxed Discord (Flatpak) keeps its socket inside the sandbox's own
        // runtime dir, which — while the app runs — is visible from the host as
        // $XDG_RUNTIME_DIR/app/<app-id>/<instance>/discord-ipc-0. The app-id
        // varies by build (Flathub ships com.discordapp.Discord, other channels
        // use org.discord.*), so instead of guessing ids we scan every app dir
        // and each live <instance>/ subdirectory. candidate_paths() only keeps
        // the ones that actually hold a discord-ipc-* socket, so unrelated
        // Flatpak apps never produce a candidate.
        let app_root = PathBuf::from(&rd).join("app");
        if let Ok(apps) = std::fs::read_dir(&app_root) {
            for app in apps.flatten() {
                if app.file_type().ok().is_some_and(|t| t.is_dir()) {
                    let p = app.path();
                    // Older flatpak layouts put the socket directly in the app dir.
                    bases.push(p.clone());
                    if let Ok(inst) = std::fs::read_dir(&p) {
                        for i in inst.flatten() {
                            if i.file_type().ok().is_some_and(|t| t.is_dir()) {
                                bases.push(i.path());
                            }
                        }
                    }
                }
            }
        }
        // Snap Discord keeps its socket in $XDG_RUNTIME_DIR/snap.discord/.
        bases.push(PathBuf::from(&rd).join("snap.discord"));
    }

    // ~/.var/app private tmp (a sandbox whose $HOME is mapped into it).
    if let Some(home) = std::env::var_os("HOME") {
        let varapp = PathBuf::from(home).join(".var/app");
        if let Ok(rd) = std::fs::read_dir(&varapp) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_lowercase();
                if name.contains("discord") || name.contains("cord") {
                    bases.push(e.path().join("tmp"));
                }
            }
        }
    }

    let mut ordered = Vec::new();
    for b in bases {
        if !ordered.iter().any(|x| x == &b) {
            ordered.push(b);
        }
    }
    ordered
}

#[cfg(target_os = "linux")]
/// Candidate Discord IPC socket paths, in try order — only sockets that
/// actually exist on this machine (the other indices, ipc-1..9, are usually
/// absent and would only add ENOENT noise to the diagnostics).
fn candidate_paths() -> Vec<std::path::PathBuf> {
    search_roots()
        .into_iter()
        .flat_map(|b| (0..SOCKET_NAMES).map(move |i| b.join(format!("discord-ipc-{i}"))))
        .filter(|p| p.exists())
        .collect()
}

#[cfg(target_os = "linux")]
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);
#[cfg(target_os = "linux")]
const HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(6);
#[cfg(target_os = "linux")]
const HANDSHAKE_ATTEMPTS: u32 = 3;
#[cfg(target_os = "linux")]
const RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(500);

#[cfg(target_os = "linux")]
/// Open one socket, handshake it (retried, see below), then hand the split read/
/// write halves to a long-lived owner task. On success returns the shared
/// `ConnectionHandle` for this one connection.
async fn establish_connection(
    path: &std::path::Path,
    client_id: &str,
) -> Result<ConnectionHandle, String> {
    use tokio::io::AsyncWriteExt;

    // A freshly-idle Discord has its RPC thread asleep; the *first* connection
    // after a pause usually gets a handshake timeout while it wakes, then answers
    // promptly on the next one. That is why the very first presence push (right
    // when the ride starts) can be flaky. We therefore open a fresh socket a few
    // times, giving Discord time to wake, before declaring this path dead. Absent
    // sockets (the real `ipc-1..9`) fail fast (ENOENT) and return immediately, so
    // this only ever costs a few seconds on the single live socket.
    let mut last: Option<String> = None;
    for _ in 0..HANDSHAKE_ATTEMPTS {
        let mut stream =
            match tokio::time::timeout(CONNECT_TIMEOUT, tokio::net::UnixStream::connect(path)).await {
                Ok(Ok(s)) => s,
                // A missing socket (ENOENT) means this candidate simply isn't there —
                // terminal for this path; don't burn the remaining attempts on it.
                Ok(Err(e)) => return Err(format!("{}: {}", path.display(), e)),
                // Could not establish the connection in time. A live-but-busy socket
                // may settle on a later try, so fall through to the next attempt.
                Err(_) => {
                    last = Some(format!("{}: connect timed out", path.display()));
                    tokio::time::sleep(RETRY_DELAY).await;
                    continue;
                }
            };

        if let Err(e) = stream
            .write_all(&encode_frame(OP_HANDSHAKE, &json!({ "v": 1, "client_id": client_id })))
            .await
        {
            return Err(format!("handshake write failed: {e}"));
        }

        match tokio::time::timeout(HANDSHAKE_TIMEOUT, read_frame(&mut stream)).await {
            // The socket is open but Discord didn't answer in time — most likely it's
            // still waking up; retry the same socket on a fresh connection.
            Err(_) => {
                last = Some("no READY from Discord (handshake timed out)".to_string());
                tokio::time::sleep(RETRY_DELAY).await;
                continue;
            }
            Ok(res) => match res {
                Ok((_op, body)) => {
                    if body.get("error").is_some()
                        || body.get("evt").and_then(|e| e.as_str()) == Some("ERROR")
                    {
                        return Err(error_message(&body)); // a real rejection — terminal
                    }
                    let user = body
                        .pointer("/data/user/username")
                        .or_else(|| body.pointer("/data/user/global_name"))
                        .and_then(|v| v.as_str())
                        .map(String::from);

                    // Split so the owner can await "read a frame" and "recv a command"
                    // in one select! without two arms fighting over one `&mut` stream.
                    // (tokio::io::split returns the generic io::ReadHalf/WriteHalf,
                    // unlike UnixStream::into_split which returns the net-specific
                    // OwnedReadHalf/OwnedWriteHalf.)
                    let (read, write) = tokio::io::split(stream);
                    let (write_tx, write_rx) = tokio::sync::mpsc::channel::<WriteReq>(16);
                    let handle = ConnectionHandle {
                        write_tx,
                        alive: Arc::new(AtomicBool::new(true)),
                        user: user.clone(),
                    };
                    tokio::spawn(owner_loop(read, write, handle.clone(), write_rx));
                    return Ok(handle);
                }
                Err(e) => return Err(e), // a real read error — terminal
            },
        }
    }

    let base = last.unwrap_or_else(|| format!("{}: connect failed", path.display()));
    Err(format!("{base} (retried {HANDSHAKE_ATTEMPTS}x)"))
}

type Pending =
    std::collections::HashMap<String, tokio::sync::oneshot::Sender<Result<Value, String>>>;

#[cfg(target_os = "linux")]
/// The single loop behind one connection. It is the ONLY reader and writer for the
/// socket (via the split halves), which makes PING / SET_ACTIVITY ordering safe:
///  * inbound PING                 -> write PONG (keeps the session alive);
///  * inbound frame with a `nonce` we're awaiting -> deliver the result to that command;
///  * a queued `WriteReq`          -> send the SET_ACTIVITY, remembering whose reply to wait on;
///  * EOF / write failure / last command sender dropped -> fail pending, tear down.
async fn owner_loop(
    mut read: tokio::io::ReadHalf<tokio::net::UnixStream>,
    mut write: tokio::io::WriteHalf<tokio::net::UnixStream>,
    handle: ConnectionHandle,
    mut write_rx: tokio::sync::mpsc::Receiver<WriteReq>,
) {
    use tokio::io::AsyncWriteExt;
    let mut pending: Pending = std::collections::HashMap::new();

    loop {
        tokio::select! {
            // Inbound frame from Discord.
            maybe_frame = read_frame(&mut read) => {
                match maybe_frame {
                    Ok((op, body)) => {
                        if op == OP_PING {
                            // Keep the connection alive. Best-effort — a failure here
                            // will surface as an EOF on the next read.
                            let _ = write.write_all(&encode_frame(OP_PONG, &json!({}))).await;
                            continue;
                        }
                        if let Some(n) = body.get("nonce").and_then(|v| v.as_str()) {
                            if let Some(tx) = pending.remove(n) {
                                let out = if body.get("error").is_some()
                                    || body.get("evt").and_then(|e| e.as_str()) == Some("ERROR")
                                {
                                    Err(error_message(&body))
                                } else {
                                    Ok(body)
                                };
                                let _ = tx.send(out);
                            }
                        }
                        // Non-pong, no-matching-nonce frame: a Discord broadcast we didn't
                        // request — ignore.
                    }
                    // EOF / read error: the connection is gone.
                    Err(e) => {
                        eprintln!("[discord] owner: read error — tearing down: {e}");
                        fail_pending(&mut pending, e);
                        break;
                    }
                }
            }
            // A command wants to go out over this connection.
            maybe_req = write_rx.recv() => {
                match maybe_req {
                    Some(req) => {
                        let body = json!({
                            "v": 1,
                            "client_id": req.client_id,
                            "cmd": "SET_ACTIVITY",
                            "args": { "pid": 0, "activity": req.activity },
                            "nonce": req.nonce,
                        });
                        pending.insert(req.nonce.clone(), req.reply);
                        if let Err(e) = write.write_all(&encode_frame(OP_FRAME, &body)).await {
                            let msg = format!("write failed: {e}");
                            eprintln!("[discord] owner: {msg} — tearing down");
                            fail_pending(&mut pending, msg);
                            break;
                        }
                    }
                    // Every command sender went away (app shutdown).
                    None => {
                        eprintln!("[discord] owner: command channel closed — tearing down");
                        break;
                    }
                }
            }
        }
    }

    handle.alive.store(false, Ordering::SeqCst);
    drop(read);
    drop(write);
}

#[cfg(target_os = "linux")]
/// Ensure there is a live connection (reusing the held one, else re-establishing a
/// fresh one over the candidate sockets) and run one command over it.
///
/// Self-healing: the `send` mutex serializes this against other pushes, and if the
/// held connection has died (or a send fails) a new one is established before the
/// command goes out. On success the fresh handle is stored for subsequent calls, so
/// a ride's many 5 s updates all reuse a single kept-open connection.
pub async fn set_activity(
    state: &Discord,
    client_id: &str,
    activity: Option<Value>,
) -> Result<Value, String> {
    let _guard = state.inner().send.lock().await;
    let slot = &state.inner().conn;

    let handle = slot
        .lock()
        .await
        .as_ref()
        .filter(|c| c.alive.load(Ordering::SeqCst))
        .cloned();

    let handle = match handle {
        Some(h) => h,
        None => establish_and_store(state, client_id).await?,
    };

    let who = handle.user.clone();
    let out = handle.send_set(client_id, activity.as_ref()).await;
    match &out {
        Ok(_) => {
            eprintln!("[discord]   -> SUCCESS (connection held open)");
            state.note(who, true, None);
        }
        Err(e) => {
            eprintln!("[discord]   -> failed: {e}");
            state.note(who, false, Some(e.clone()));
        }
    }
    out
}

#[cfg(target_os = "linux")]
/// Try each candidate socket; on the first success store the new handle and return
/// it. On total failure the error enumerates every candidate and what happened so
/// the real problem (wrong path / stale socket / handshake reject / rate limit) is
/// visible without the Terra terminal.
async fn establish_and_store(state: &Discord, client_id: &str) -> Result<ConnectionHandle, String> {
    let slot = &state.inner().conn;
    let paths = candidate_paths();
    let roots_txt = search_roots()
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let mut attempts: Vec<String> = Vec::with_capacity(paths.len());

    let ctx = format!(
        "env: XDG_RUNTIME_DIR={} HOME={} TMPDIR={} — probed: {roots_txt} — tried {} socket(s)",
        std::env::var_os("XDG_RUNTIME_DIR").map(|v| v.to_string_lossy().into_owned()).unwrap_or_else(|| "<unset>".into()),
        std::env::var_os("HOME").map(|v| v.to_string_lossy().into_owned()).unwrap_or_else(|| "<unset>".into()),
        std::env::var_os("TMPDIR").map(|v| v.to_string_lossy().into_owned()).unwrap_or_else(|| "<unset>".into()),
        paths.len(),
    );
    eprintln!("[discord] establishing connection — {ctx}");

    for path in &paths {
        let p = path.display().to_string();
        let handle = match establish_connection(path, client_id).await {
            Ok(h) => {
                eprintln!("[discord]   {p}: connected, handshake OK, holding open");
                h
            }
            Err(e) => {
                let st = format!("connect/handshake failed: {e}");
                eprintln!("[discord]   {p}: -> {st}");
                attempts.push(format!("{p}: {st}"));
                continue;
            }
        };

        // Replace any stale handle and remember the fresh one.
        *slot.lock().await = Some(handle.clone());
        return Ok(handle);
    }

    let detail = if attempts.is_empty() {
        "no Discord IPC socket candidate found (is Discord running? a Flatpak Discord's socket appears under $XDG_RUNTIME_DIR/app/<app-id>/<instance>/, e.g. app/com.discordapp.Discord/…; the list above is every directory probed)"
            .to_string()
    } else {
        attempts.join("; ")
    };
    let msg = format!("Discord Rich Presence unavailable — {ctx}; {detail}");
    eprintln!("[discord] FAIL: {msg}");
    state.note(None, false, Some(msg.clone()));
    Err(msg)
}
