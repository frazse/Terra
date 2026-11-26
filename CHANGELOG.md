# Changelog

## [0.12.1] — Android Picture-in-Picture & `terra-climb` v0.5.0

### Added
- **Android Picture-in-Picture (PiP) support during rides** — added opt-in PiP support for `/ride*` and `/spectate*` pages ([AndroidManifest.xml](gen/android/app/src/main/AndroidManifest.xml), `MainActivity.kt`, `android_pip.rs`, `terra-ui.js`) with a compact single-row rider-stats HUD (hiding the zones bar, `kcal`, and floating overlays), `Auto-enter PiP` when leaving Terra during a ride, and an optional quick PiP button in ride controls (both off by default in **Terra Settings → Picture-in-Picture (PiP)**).

### Changed
- **Refreshed `Terra-Addons` submodule** — includes `terra-climb` v0.5.0 (selectable `processGPX` / Fiets climb detection, Biketerra grade & draft colors).

## [0.12.0] — Native BLE Bridge (Linux & Android), BlueZ & Android BLE Overhaul, Ride Backups & AppImage Wayland Icon

### Added
- **Biketerra Native BLE Bridge (`window.isNative` / `window.native`) on Linux Desktop & Android** — Terra now implements Biketerra's official native Bluetooth bridge on both desktop and Android (`ble_native_scan_start`, `ble_native_scan_stop`, `ble_scan_candidates`, `ble_remember`), synchronously seeding `saved-devices` before page boot so trainers, heart rate monitors, and virtual addon devices (`zwift-ride` OBC, `wifi-trainer-bridge`) auto-connect cleanly across new rides, resumed rides, and multi-ride sessions without DOM button clicking.
- **Automatic In-Progress Ride Backups & WebView Storage Export** — periodically snapshots active ride data (every 30 s, on backgrounding, and on `pagehide`) with an in-app backup viewer, GPX export, deletion, and full WebView storage export (`terra_backup_ride`, `terra_list_ride_backups`, `terra_read_ride_backup`, `terra_export_ride_gpx`, `terra_delete_ride_backup`, `terra_export_webview_storage`).
- **Android BLE Foreground Service (`TerraBleService`)** — keeps active Bluetooth GATT connections alive while riding on Android.
- **Windows x64 Desktop Build & CI** — added Electron + Rust RPC Windows desktop support and GitHub Actions build workflow.

### Fixed
- **Linux BlueZ dropouts & slow GATT discovery** — switched Linux discovery to LE-only (`Transport::Le`) via `bluez-async` to avoid BR/EDR radio collisions on KICKR trainers, serialized GATT connections with discovery recovery for large GATT tables, and kept LE-only scanning active until all remembered non-excluded devices (including Zwift Ride controllers) connect.
- **Desktop shutdown leaving BLE devices connected** — parallelized both disconnect passes in `Broker::disconnect_all()`, raised Electron's `ble_disconnect_all` shutdown budget from 2 s to 8 s, and hid the window immediately on close so all connected peripherals (trainer, HRM, both Zwift Ride controllers) disconnect cleanly.
- **Android BLE scan starvation & GATT reliability** — prevented powered-off remembered devices from holding `connect_gate` and stopping scans (`status 147` fast-fail + visibility check), reused active GATT links without interrupting scans, serialized per-device GATT/CCCD queues, retained split advertisement/scan-response service UUIDs, and unsubscribed only actively subscribed characteristics on disconnect.
- **Linux AppImage icon on Wayland & X11** — bundled app icons inside `extraResources`, set `StartupWMClass` / `app.setDesktopName('terra-desktop.desktop')`, and installed a managed `NoDisplay=true` desktop entry + hicolor icon on AppImage launch so Wayland compositors display the Terra icon.
- **Android export path** — `addon_save_file` now saves to `<data-dir>/exports` when a custom data folder is configured, or `/storage/emulated/0/Download/Terra` otherwise.

### Changed
- **Refreshed `Terra-Addons` submodule** — includes `terra-climb` v0.2.2, `zwift-ride` v0.6.6, `voice-chat` v0.3.0, `rowbike` v0.2.0, `garmin-hud` v1.4.2, `group-spectate` v0.3.0, `fps-monitor` v0.3.1, and `realistic-gears` v1.2.2.

## [0.11.2] — Fix Addon Bluetooth Chooser With Wi-Fi Trainer Bridge

### Fixed
- **Zwift Ride Pair controller blocked by Wi-Fi Trainer Bridge** — routed `terra.ble.request()` directly through `__terraCore.requestDevice` so page-level `navigator.bluetooth.requestDevice` wrappers cannot hijack addon pairing requests, and updated bundled `wifi-trainer-bridge` to `v2.0.3` to pass through `__terraOwner` requests.

## [0.11.1] — GitHub Catalog Redesign, Per-Addon Changelogs & Terra Settings UI

### Added
- **Per-addon changelog notes in GitHub Catalog** — `addons_catalog` now reads `<addon>/CHANGELOG.md` so addons with available updates automatically expand a "What's new" section for missing versions, and all other catalog entries include a collapsible `Changelog ▾` toggle.
- **`Chat` & dynamic A–Z category chips** — added the `Chat` filter chip (and auto-discovery for any custom addon categories) with alphabetical sorting (`Other` last).

### Changed
- **Full-column GitHub Catalog** — switching to the GitHub Catalog now takes over the full modal body with updates pinned at the top (sorted A–Z) and a batch **Update all** button.
- **Terra Settings UI polish** — renamed the modal from "Terra addons" to **Terra Settings**, made **Addons** (open by default, hosting `GitHub Catalog` and `Install .zip`) and **Data folder** (collapsed by default) collapsible cards alongside **Bluetooth devices** and **Frame Generation**, added `▸`/`▾` open/collapsed carets, and grouped the bottom utility panels in a distinct recessed tray.

## [0.11.0] — Performance Telemetry, Linux Frame Generation & Android Microphone Support

### Added
- **Hardware & app telemetry (`terra.perf.stats()`)** — native performance stats API across Linux desktop (CPU/GPU utilization & temperatures, process CPU/RAM via Electron + Rust) and Android (thermal status, CPU/battery temperatures, RAM).
- **Experimental Frame Generation (Linux desktop)** — `lsfg-vk` / Lossless Scaling integration in the Cog menu (`framegen_get`, `framegen_set`, `framegen_pick_dll`, `framegen_relaunch`) with automatic XWayland + Vulkan relaunch, flow scale & performance mode controls, and dynamic ride-only toggling on `/ride` and `/spectate`.
- **Android microphone & WebRTC permissions** — declared `RECORD_AUDIO` / `MODIFY_AUDIO_SETTINGS` and forwarded `WebChromeClient` permission requests so in-page audio/voice addons work on Android.

### Fixed
- **Graceful desktop shutdown** — Electron now intercepts window close, `SIGINT`/`SIGTERM`, and framegen relaunches to detach CDP, dispatch `pagehide`, and allow a 450 ms drain alongside `ble_disconnect_all` before terminating the Rust backend so Wi-Fi trainer TCP sessions release cleanly.

### Changed
- **Refreshed `Terra-Addons` submodule** — includes `fps-monitor` v0.3.0 (hardware telemetry, engine render/scene stats, base/framegen FPS display), `ghost-rider` v0.1.0, `voice-chat` v0.1.1, `render-scale` v0.1.0, `zwift-ride` v0.6.4, `wifi-trainer-bridge` v2.0.2, `garmin-hud` v1.4.1, `nameplates` v0.7.3, and `rider-speed` v0.2.1.

## [0.10.1] — Account Flyout Interaction Fix & Bundled Addon Refresh

### Fixed
- **Account menu flyout clicks blocked** — removed `[class*="profile"]` from the GPU layer promotion selector in `shim/terra-ui.js` (`transform: translateZ(0)` on `.nav-profile` had trapped `.nav-dropdown` underneath `.flyout-fog` in Biketerra's header).

### Changed
- **Refreshed `Terra-Addons` submodule (`zwift-ride` v0.6.3)** — includes the 2-column athlete profile layout, Console UI toggle, virtual OpenBikeControl shifter for auto Virtual Shifting, and controller battery mirroring.

## [0.10.0] — In-app GitHub Addon Catalog, ZIP Installer, Settings Backup & BLE Manager

### Added
- **In-app GitHub Addon Catalog & ZIP Installer** — browse and install/update addons directly from `frazse/Terra-Addons` (`addons_catalog`, `addons_install_github`), install `.zip` addon bundles (`addons_install_zip` using pure-Rust `flate2`/`crc32fast` extraction), and uninstall user-installed addons or overrides (`addons_uninstall`) from the Cog menu.
- **Addon settings backup** — export and import non-secret addon settings and storage JSON per addon (`addon_settings_export`, `addon_settings_import`).
- **BLE device manager** — view connected and remembered Bluetooth devices in the Cog menu (`ble_devices_status`), disconnect individual or all active devices (`ble_drop_device`, `ble_drop_all`), and forget remembered pairings (`ble_forget`).

## [0.9.0] — Android folder picker, `window.btApp` & Wi-Fi bridge on Android, and stability fixes

### Added
- **Android custom data folder picker** — first-launch setup modal and cog-menu folder picker (`terra_pick_data_dir`, `terra_data_dir_status`, `terra_reset_all_data`) allow storing `addons/`, `addon-data/`, and `devices.json` in a user-chosen visible folder (e.g. `/storage/emulated/0/Terra`) with non-destructive migration.
- **`window.btApp` auto-publish (Android)** — `MainActivity` now wraps `RustWebViewClient` in `setContentView` to intercept Biketerra's `/_app/immutable/chunks/*.js` and expose `window.btApp` automatically without DevTools (matching Linux desktop).
- **Android `wifi-trainer-bridge` & LAN permission** — `wifi-trainer-bridge` is now bundled in the APK (**disabled by default** in `manifest.json`) and `ACCESS_LOCAL_NETWORK` is declared and requested at runtime for Android 17 (API 37+) LAN TCP/UDP support.
- **BLE auto-pair preference** — silent auto-pair now prefers devices matching `__terraMode` / `__terraPrefer` prefixes.

### Changed
- **Desktop AppImage no longer bundles addons** — removed `../addons` from `electron/package.json` `extraResources` so the Linux AppImage ships clean and loads addons from `~/.local/share/terra/addons/`.
- **Settings cog anchor** — anchored the cog button cleanly next to Help in Biketerra's redesigned header.

### Fixed
- **Desktop BLE shutdown race** — `Broker::disconnect_all()` now sets `shutting_down`, aborts the auto-reconnect supervisor, stops scanning, and disconnects any in-flight/connected peripherals at the adapter level so devices don't reconnect after closing the AppImage.
- **Sandboxed Discord IPC discovery** — Discord Rich Presence now discovers Flatpak/Snap Discord IPC sockets on Linux.
- **Android BLE & stability hardening** — ProGuard/R8 JNI keep rules (`proguard-rules.pro`), Android ACL capability updates (`discord_*`, `terra_*`, `addon_tcp_*`, `addon_udp_send`, `ride_*`), and compiler/build warning cleanup.

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
