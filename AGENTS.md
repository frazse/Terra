# AGENTS.md — Terra

Working guide for AI agents (and new contributors). This captures the parts of the
architecture that are **non-obvious, cross-cutting, or easy to break**. It is not a
substitute for the canonical docs:

- **`README.md`** — what Terra is, supported platforms, from-source build commands.
- **`ADDONS.md`** — the authoritative addon format + the full `terra.*` API. *Read this
  before touching anything addon-related.*

Everything here is Linux-centric because the default dev target is the Linux desktop
(Electron + a Rust backend over stdio). Android/Tauri is a second, parallel runtime.

---

## 1. What Terra is

Terra is a **companion runtime that wraps `https://biketerra.com`** in a real webview.
It is **not** the game — it makes a plain browser page into a bike-computer host:

- injects a **shim layer** (fake `navigator.bluetooth`, an `addons` engine, a settings
  "cog menu") into the page's **main JS world** *before* the game boots;
- runs a **native Rust backend** that does the heavy lifting (BLE GATT, UDP/TCP,
  Discord Rich Presence, ride-broadcast relay, per-addon persistence);
- loads **user JS addons** (like Tampermonkey userscripts) that run inside the page with
  the page's privileges and call the Rust backend through the shim.

**Trust model (important):** addons run with full page privileges, including the BLE
commands. This is a deliberate userscript-style design — document it when you touch it.

---

## 2. The three runtimes

The **same Rust core** ships in three hosts. `src/lib.rs` dispatches on target:

| Platform | UI host | Rust entry | Transport (page → Rust) |
| --- | --- | --- | --- |
| **Linux desktop** (x86_64 + ARM64) | Electron / Chromium (`electron/`) | `lib_linux.rs::run()` | stdio **JSON-RPC** (`--rpc`) |
| **Android** (aarch64) | Tauri WebView + Rust JNI (`gen/android/`) | `lib_tauri.rs::run()` | Tauri **native commands** |
| Other non-Linux / non-Android | Tauri desktop window | `lib_tauri.rs::run()` | Tauri **native commands** |

Key consequence: **there is no Tauri webview on Linux.** `src/linux_runtime.rs` is a
hand-rolled `AppHandle` / `State` / `path()` replacement so the backend can run as a
plain tokio binary without pulling in webkit2gtk/GTK. On Linux the binary is launched
with **`--rpc`** and blocks forever serving JSON over stdio; Electron does the UI.

- `src/main.rs` → `terra::run()` (3 lines; the real entry is target-selected in `lib.rs`).
- `src/lib_linux.rs` — Linux `run()`. Registers state, spawns `ble::Broker` **on the same
  tokio runtime** as the RPC server (comment in the file explains why order matters),
  then runs `rpc::run_rpc_server`. Without `--rpc` it exits immediately (Electron owns UI).
- `src/lib_tauri.rs` — Tauri `run()`. `.invoke_handler!` registers **every** command
  (`ble_*`, `addon_*`, `ride_*`, `discord_*`, `shim_log`). This list is the single place a
  Tauri-side command must be exposed — keep it in sync when you add a command.

---

## 3. The shim layer (the heart of the system)

Three JS files in **`shim/`**, concatenated and injected into the page's **main world**
before the game runs. They build `window.__terra` and the `terra` object handed to addons.

| File | Responsibility |
| --- | --- |
| `bluetooth-shim.js` | Fakes `navigator.bluetooth` entirely on top of the Rust backend; implements `terra.bus`, the `chooser`, and device lifecycle. Every BLE call round-trips to Rust. |
| `addons-loader.js` | The addon engine: scans `manifest.json`, matches `URL`, evaluates the `js` array *as the body of `async function(terra){ }`*, wires the **`terra.*` API**, `bus`, `storage`, `onCleanup`/`onNavigate`. Also owns the `addons.active` map + `settings.preview()` paint closure. |
| `terra-ui.js` | The **cog settings menu** (enable/disable switches, per-addon settings fields — text/number/checkbox/select/color/password — and the **live preview slot**). Renders UI, calls the backend via `window.__TAURI_INTERNALS__.invoke`. |

### How injection happens per platform
- **Linux / Electron:** `electron/preload.js` **reads the 3 shim files from disk at boot**
  (`path.join(__dirname,'..','shim',...)`) and `executeJavaScriptInIsolatedWorld(0, …)`
  them into world 0. → **Shim changes require a full Terra restart** (the binary is
  re-spawned and preload re-reads). A plain F5 re-runs the page but the *preload
  snapshot* is already locked in for that window — restart to be safe.
- **Android / Tauri:** the 3 files are `include_str!`'d at **compile time**
  (`lib_tauri.rs` lines 5–7) and passed as the webview `initialization_script`. →
  **Shim changes require a Rust rebuild + reinstall** of the APK.
- **Shipped AppImage:** `electron-builder` copies `shim/` into `resources/shim/`
  (`package.json` → `extraResources`), and `preload.js` has a `resources/shim` fallback.
  So in a released build a **shim change needs a full rebuild** (the dev-mode "restart the
  app" trick doesn't pick up shim edits already baked into `resources/`).
- Both concatenate in the same order: `bluetooth-shim` → `addons-loader` → `terra-ui`.

### The `terra.*` API
Don't reimplement the table here — **see `ADDONS.md`** (it is the source of truth and is
kept in sync). The surface: `terra.log / onCleanup / onNavigate / waitFor / storage.* /
bus.* / ble.* / ride.* / keys.* / ui.addStyle / udp.send / tcp.*`. Addons also get a raw
`navigator.bluetooth` (the shim) so they can pair their own devices.

Cog-menu setting types (`terra-ui.js`): `text`, `number`, `checkbox`, `select`
(`options`), `color`, `password`. `password` = secret: the page can write values but
never read them back (`addons.rs` only reports *which* keys are set); backend-side
`{{setting:KEY}}` templating is how a secret is consumed without ever traveling to the
page.

---

## 4. Rust backend map (`src/`)

Sized, most-edited first. Cross-platform commands use
`#[cfg_attr(not(target_os = "linux"), tauri::command)]` — that macro is a *no-op* on
Linux (the function is driven by the RPC dispatcher instead) and a real Tauri command
everywhere else. **Always use that pattern for new cross-platform commands.** On
non-Linux, a new command the **page may call** also needs (a) an entry in `build.rs`'s
`AppManifest::commands` (that generates the `permissions/autogenerated/allow-*.toml`) and
(b) `allow-<cmd>` in `capabilities/remote.json` (the command allow-list for the
biketerra.com webview). Internal/never-called-from-page commands don't, but most BLE/addon
commands are page-facing.

| File | ~lines | What it owns |
| --- | ---: | --- |
| `ble.rs` | 1718 | `Broker` (btleplug 0.11) — scan, pair, connect, GATT read/write/subscribe; the `ble_*` commands; `get_terra_storage_dir()` (the path resolver, see §6). |
| `addons.rs` | 1018 | `Addon`/`SettingDef`/`AddonStore`, `addon_dirs()`, `load_all()`, `addons_activate()` (returns the JS to eval), per-addon `storage`/`settings`, `addon_http` (CORS-free), `addon_save_file` (→ `Downloads/Terra/`), `addon_udp_send`, and the `TcpRegistry` + `addon_tcp_*`. `addon_settings_set/get` back the cog menu. **`SettingDef.options` + preview live here** (recent work). |
| `discord.rs` | 747 | Discord Rich Presence over the local IPC socket (`discord-ipc-0..9`), incl. a `search_roots()`/`candidate_paths()` Linux socket probe (native `XDG_RUNTIME_DIR` **and** Flatpak `$XDG_RUNTIME_DIR/app/…` and Snap roots). `set_activity`/`clear_activity`/`status`. |
| `rpc.rs` | 370 | **The stdio JSON-RPC server (Linux only).** Reads `{id, cmd, args}` lines, dispatches the ~36 commands (the big `match` at the bottom), prints `{id, result|error}` lines back. This is the only page↔Rust door on Linux. |
| `linux_runtime.rs` | 206 | `AppHandle` / `State` / `PackageInfo` / `PathProvider` (XDG paths) / `WebviewWindow` stub / `async_runtime`. The "fake Tauri". |
| `ride.rs` | 75 | `Ride` state: latest frame + ring buffer (~5 min @2 Hz) of the `biketerra-ride` broadcast; `ride_push/latest/history`. |
| `lib_linux.rs` / `lib_tauri.rs` / `lib.rs` / `main.rs` | ~190 | Entry dispatch (see §2). |

**Cross-platform state** (all registered on the app handle in *both* `lib_*` files —
add any new state here in both, or Linux + one other side will panic on `.state::<T>()`):
`AddonStore`, `TcpRegistry`, `Ride`, `Discord`, `Ble(broker)`.

---

## 5. How the page talks to Rust (the bridge)

### Linux (Electron ⇄ `terra --rpc`)
1. `electron/preload.js` exposes `window.__TAURI_INTERNALS__.invoke(cmd,args)` →
   `ipcRenderer.invoke('tauri-invoke', …)`.
2. `electron/main.js` `ipcMain.handle('tauri-invoke')` → `sendRpc()` writes
   `{id, cmd, args}\n` to the child's **stdin**, resolves on the matching stdout `{id, …}`.
   (`addons_activate` results carry `.scripts[]` which `main.js` executes in the page.)
   Every call has a **30s timeout** (`main.js`) — a hung backend rejects the promise, it
   doesn't freeze Electron.
3. Rust → page **events** come back as stdout lines `{event: …}`: `js_eval`
   (immediate `executeJavaScript`), `ble_notify`, `chooser_open`, etc. `main.js` routes
   these to `window.__terra.*`.

So on Linux the *word* "Tauri" in `window.__TAURI_INTERNALS__` is a façade — the real
path is **stdio JSON over a spawned child process**.

### Android / Tauri
`shim_log` / `ble_*` / `addon_*` / `ride_*` / `discord_*` are direct Tauri
`[command]`s; no stdio hop, no `rpc.rs`.

### Getting game internals: `window.btApp`
Addons need the game's controller manager, which the game keeps **inside a module
closure**. `electron/main.js` reproduces the old "conditional breakpoint" trick via CDP:
it locates the line `this.riderController.handleInput()` in the (per-deploy hashed)
game chunk and sets a breakpoint with condition `window.btApp = this, false` (the
condition publishes `this`, evaluates `false` → the game **never pauses**). The
breakpoint must carry **both `lineNumber` and `columnNumber`** (CDP/V8 wants a column —
the chunk is minified). Re-armed on every full load *and* SPA in-page navigation.
Disable: `TERRA_NO_BT_BREAKPOINT=1`.
While Terra's debugger client is attached the page's own DevTools cannot attach (Chromium
allows one debugger client per target). `nameplates`-style addons poll
`window.btApp` (`waitForBT`) because first frames may arrive before it is published.

---

## 6. Key paths (and a README inaccuracy)

- **`TERRA_ADDONS_DIR`** (path list) overrides everything. Useful for testing.
- **Linux desktop (the default target)** — `get_terra_storage_dir()`
  (`ble.rs`) resolves to **`~/.local/share/terra/`**:
  - `~/.local/share/terra/addons/` — **the live addon folder** that's scanned first.
    *This is where you drop/copy a local addon to test it* (e.g. `…/addons/nameplates/main.js`).
  - `~/.local/share/terra/addon-data/.addons-state.json` — the cog menu's
    enable/disable overrides per addon id.
  - `~/.local/share/terra/addon-data/settings/<addon-id>.json` — cog-menu settings (kept
    **separate** from the addon's own `terra.storage`; secrets never return to the page).
  - `~/.local/share/terra/addon-data/<addon-id>.json` — the addon's `terra.storage` values.
- **Android** — `/storage/emulated/0/Android/data/dev.frazse.terra/files/{addons,addon-data}/`.
- **`./addons/`** — the in-repo `addons/` (a git **submodule** → `frazse/Terra-Addons`) is
  the *bundled defaults* and also a valid scan dir.

> ⚠️ **README says** the Linux addon dir is `~/.local/share/dev.frazse.terra/addons/`.
> That is the **Tauri/Android identifier**, *not* the actual Linux desktop path. On Linux
> it is **`~/.local/share/terra/addons/`** (per `linux_runtime` / `get_terra_storage_dir`).
> Trust the code, not the README line, when editing the Linux host.

**Precedence gotcha:** `addon_dirs()` scans the storage dir **before** `./addons/`, and
`load_all()` is first-wins on duplicate ids. So a **stale live copy in
`~/.local/share/terra/addons/<id>/` masks your repo/submodule edits** for the same
`id`. When you change a bundled addon and it "doesn't take effect", delete or fix the
live copy (or set `TERRA_ADDONS_DIR`) first.

---

## 7. Running, building, testing

### Develop (Linux desktop)
```bash
cargo build --release        # → target/release/terra  (the --rpc backend)
cd electron
npm install
npm start                    # spawns ./bin/../target/{release,debug}/terra --rpc
```
`electron/main.js` finds the binary in this order: `resources/bin/`, `__dirname/bin/`,
`../target/release/terra`, `../target/debug/terra`. It auto-restarts the child on exit.

- **Add-on JS change** → F5 / Ctrl+R in the window (add-ons rescan on every page load). *No
  rebuild.*
- **Shim change** → full restart (see §3).
- **Rust change** → `cargo build --release` again, then restart (Electron re-spawns the new
  binary).
- **Debugging:** `TERRA_TRACE=1` (or `all`) enables verbose RPC/BLE tracing in the backend
  (`rpc.rs`, `ble.rs`) — useful when a command "hangs".

### Validate JS without launching the app
- `node --check <file>` — syntax.
- Addon `main.js` can be unit-tested headlessly: a small Node script `eval`s it against
  a fake `window` / `document` / `terra` (and a stubbed `window.btApp`) and asserts
  behavior — plate text, cog-preview paint, `onCleanup` teardown. Re-run after editing an
  addon's logic. Keep throwaway harnesses in a scratch dir (e.g. `/tmp`), **not** in the repo.

### Release
```bash
./build.sh                   # syncs version → cargo release → electron AppImage
```
Output: `electron/dist/Terra-<ver>.AppImage`. CI (`.github/workflows/build-arm64.yml`)
does the same on an ARM64 runner and uploads the AppImage.

### Android APK
```bash
rustup target add aarch64-linux-android
cargo tauri android build --apk -t aarch64
# → gen/android/app/build/outputs/apk/universal/release/app-universal-release.apk
```

### `karoo-apk/` (a *separate* Kotlin app)
Not part of the app build. A Hammerhead-Karoo companion (Karoo extension, `karoo-ext`
vendored in `karoo-apk/lib/`) that receives the ride over **UDP 41234** from the
`karoo-bridge` addon (`terra.udp.send`) and does mock-GPS + virtual sensors. Build with
`./gradlew assembleDebug` *inside `karoo-apk/`*. See `karoo-apk/README.md`.

### Bluetooth device chooser
`ble.rs::open_chooser` differs per platform:
- **Tauri desktop (non-Linux, non-Android):** opens a *separate small Tauri window*
  loaded from **`ui/chooser.html`** (title "Select Bluetooth device"; `tauri.conf.json`
  has `frontendDist: ./ui`). A dedicated page, not part of the game webview.
- **Linux:** emits `showChooserModal` as a `js_eval` event (see transport in §5) and
  `shim/bluetooth-shim.js` renders an **in-page DOM overlay** (`.terra-chooser-overlay`)
  inside the game window.
- **Android:** same in-page overlay path as Linux (Tauri's webview exists on Android).
Both list devices via Rust `chooser_candidates` and confirm via `chooser_pick`.

---

## 8. Git / repo conventions

- Two repos: **Terra** (this) and **Terra-Addons** (the `addons/` git submodule).
  Bumping an addon requires a **submodule pointer commit** in Terra too.
- Small, reviewable, scoped commits (`feat(settings): …`, `fix(ble): …`, `chore(bridge): …`).
- **Do not `git push` until the user has confirmed the change works and explicitly says
  push.** (Repeated, non-negotiable rule in this project.)
- **Repo gotcha:** a stale `refs/tags/main` tag existed that makes plain
  `git push origin main` fail with *"src refspec main matches more than one"*. Use an
  explicit refspec: `git push origin refs/heads/main:refs/heads/main`.
- Keep the user's in-flight/unrelated work (e.g. open `discord.rs`, `Cargo.lock`,
  `README.md`, `gen/*`) **untouched and uncommitted** unless asked.
- Never commit personal backup files (e.g. `*.bak.js`, `main2.js`-style copies) — keep them untracked.

---

## 9. Invariants & "do-not-break" list

1. **Never throw in the game's render loop.** The frame ticker wraps *any* exception as a
   fatal "GPU can't keep up" failure (3 retries → give up). In `nameplates` a `const el`
   reassignment on lazy plate creation was a per-frame `TypeError` → fatal. Use `let`.
2. **Avoid heavy CSS (notably `backdrop-filter`) in in-page overlays.** It broke the game
   GPU on the Vulkan/ANGLE stack. (Kept as a comment in `nameplates/style.css`.)
3. **Shim order & world:** keep `bluetooth-shim → addons-loader → terra-ui` order, all in
   world 0 (main world), or `window.__terra` / `navigator.bluetooth` won't exist for
   addons or the page.
4. **Register new cross-platform state in *both* `lib_linux.rs` and `lib_tauri.rs`**, and a
   new command in **both** `rpc.rs` (match arm) **and** `lib_tauri.rs` (invoke_handler).
5. **Linux backend = `--rpc` only.** Don't assume a webview exists on Linux
   (`get_webview_window()` is always `None` there).
6. **Settings vs storage separation:** cog-menu settings live in
   `addon-data/settings/<id>.json`; an addon's `terra.storage` lives in
   `addon-data/<id>.json`. Keep them apart; secrets must not round-trip to the page.
7. **`window.btApp` is async/published-late.** Poll for it; don't assume it on the first
   frame (§5).

---

## 10. "If I'm doing X, look in Y"

| Task | Start here |
| --- | --- |
| Add a `terra.*` API method | `shim/addons-loader.js` (expose) + `src/addons.rs` + `src/rpc.rs` (arm) + `src/lib_tauri.rs` (command) + non-Linux: `build.rs` `AppManifest::commands` + `capabilities/remote.json` (`allow-<cmd>`) + `ADDONS.md` (doc) |
| New cog-menu setting type | `src/addons.rs` (`SettingDef`) + `shim/terra-ui.js` (render) + `shim/addons-loader.js` (`settings.preview`) |
| Edit a bundled addon | `addons/<id>/` (submodule) → test in `~/.local/share/terra/addons/<id>/` → bump Terra pointer |
| BLE / GATT behavior | `src/ble.rs` + `shim/bluetooth-shim.js` |
| Ride data / graphs | `src/ride.rs` + `ADDONS.md` §Live-ride-data |
| Discord presence | `src/discord.rs` (+ the `discord-rich-presence` Terra-Addon) |
| Packaging / version | `build.sh`, `electron/package.json`, `tauri.conf.json`, `.github/workflows/build-arm64.yml` |
| Something "takes no effect" | Stale live addon copy in `~/.local/share/terra/addons/` (§6) — or shim needs a restart (§3) |
