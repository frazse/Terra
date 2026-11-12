# Changelog

## [0.8.0] — Settings UI 2.0, addon pipeline hardening & Android upgrades

### Added
- **Cog-menu settings: `select` + `color` types** — dropdowns (with `options`) and color pickers for addon settings, alongside the existing text/number/checkbox/password.
- **Live preview slot** — addons can paint into the settings menu via `terra.settings.preview(paint)` and the slot re-renders as settings change; the slot now always renders even when an addon defines no preview.
- **`window.btApp` auto-publish (Linux desktop)** — Terra now exposes the game's GameManager to addons automatically (CDP conditional breakpoint on the input-handler call site); addons like nameplates no longer need a manual DevTools breakpoint.
- **Addons: Route Sign (0.1.0)** — one billboard sign anchored along the active route (`whereAlongRoute` / `side` / `height`).
- **`build-apk.sh`** — one-command Android build: syncs the three bundled addons (zwift-ride, garmin-hud, fps-monitor) from the latest Terra-Addons `main` on GitHub (default), or `--live` from `~/.local/share/terra/addons`, or `--pinned` from the submodule as checked out (offline); `--sync-only` just refreshes the APK assets.
- **App icon set** — full icon set generated from `icons/icon.jpg`; the Electron app ships a real window/dock icon.

### Changed
- **Nameplates 0.4.0** — bunched plates now spread sideways with thin leader lines instead of stacking; new "name only" plate format; speed shown in km/h.
- **sunto-export** — auto-detects rowing mode and stamps the sport marker on export.
- **BLE manual chooser** — now applies the game's own GATT service filter, so the chooser only lists devices the ride will actually pair with.
- **Android bundled-addon upgrades** — `extractBundledAddons()` now compares manifest versions and replaces a stale on-device copy when the APK ships a newer addon (previously the first install stuck forever, updates never landed). On-device copies that are the same or newer (e.g. a hand-patched local copy) are preserved; the extracted version is logged to logcat.
- **Bundled APK addons refreshed** — garmin-hud 1.2.0 (ride-broadcast graphs), zwift-ride (new `console-ui.js` + nav/stylesheet refresh).

### Docs
- README / ADDONS.md aligned with current behavior — the Linux addon path is corrected to `~/.local/share/terra/addons/` (the previous `dev.frazse.terra` path was the Tauri/Android identifier, not the Linux one).
- New AGENTS.md working guide for AI agents and new contributors.

### Notes
- Route Sign v2 (native 3D quad, `0.2.0-alpha`) is still in-flight in Terra-Addons and is **not** part of this release build.
- The bundled-addon list lives in two places and must stay in sync: `BUNDLED_ADDONS` in `build-apk.sh` and `bundledAddons` in `MainActivity.kt`.

## [0.5.0] — Linux Desktop (Electron) Runtime & Bluetooth Fix

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
