# Changelog

## [0.4.2] — Linux Desktop (Electron) Bluetooth Fix

### Changed
- **Replaced Tauri runtime on Linux with lightweight custom runtime** — Linux desktop now uses a custom runtime (`src/linux_runtime.rs`) instead of Tauri's webkit2gtk/GTK dependency. The binary runs only as an RPC backend (`--rpc`) and Electron handles the webview. This eliminates the heavy GTK dependency chain on Linux and makes the Linux build faster and lighter. The Android and other desktop builds continue to use Tauri unchanged.

### Fixed
- **BLE supervisor never ran on Linux** — `Broker::new()` was called on a separate tokio runtime that was dropped immediately, so the supervisor tasks (BLE scanning + auto-connect) were spawned on a dead runtime and never executed. Fixed by moving BLE initialization onto the main runtime's async block.
- **Bluetooth chooser never appeared** — `open_chooser()` on Linux emitted a raw `chooser_open` event that was routed to a dead IPC channel (`webContents.send('rust-event')` with no listener). Changed to use `js()` which writes `js_eval` events that `main.js` already handles by executing JS in the webview.
- **Disconnect events lost on Linux** — `spawn_event_loop` emitted raw `ble_notify` disconnect events via the same dead path. Changed to use `js()` for consistency.
- **Notification handler panic** — `connect()` had `unreachable!("no app in RPC mode")` but on Linux `self.app` is always `None`. Removed the unreachable; `js()` handles `None` by falling through to stdout.
- **`js()` signature mismatch** — Changed from `&AppHandle` to `Option<&AppHandle>` so both Linux (`None`) and Android/desktop (`Some`) work. Updated all call sites.
- **Missing IPC event handler** — Added `ipcRenderer.on('rust-event', ...)` in `preload.js` to route `chooser_open` and `ble_notify` events to `window.__terra` as a safety net.

### Added
- `build.sh` — one-command build script (Rust release + Electron AppImage).
- `.gitignore` entry for `.flatpak-builder` (prevented Cargo from scanning massive Flatpak cache during fingerprinting).

### Notes
- Android build is **unaffected** — all changes are either Linux-only (`#[cfg(target_os = "linux")]`), non-Android (`#[cfg(not(target_os = "android"))]`), or Electron-only (`preload.js`).
- The dead code warnings on Linux are expected — Tauri command handlers (`ble_request_device`, `ble_subscribe`, etc.) have `#[cfg_attr(not(target_os = "linux"), tauri::command)]` and are dispatched via `rpc.rs` on Linux instead.
