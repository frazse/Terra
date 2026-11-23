# Terra 🚲

> [!NOTE]
> **Disclaimer**: Terra is an independent, community-developed open-source project. It is **in no way affiliated with, endorsed by, or associated with [Biketerra.com](https://biketerra.com)**.

**Terra** is an open-source, cross-platform companion runtime for **[Biketerra.com](https://biketerra.com)**. It wraps Biketerra in a dedicated webview, injects a lightweight shim layer (`shim/` — Web Bluetooth, an addon runtime, and Terra's settings UI) before the page boots, and pairs it with a native **Rust** backend (`src/`) for Bluetooth LE, local networking, Discord Rich Presence, and persistent storage.

**Stack:** one shared **Rust** backend + **Electron / Chromium** (Linux & Windows desktop) + **Tauri WebView** (Android).

---

## 🌟 Key Features

* **🎮 Native WebGPU & Edge-to-Edge Viewport:** runs Biketerra's 3D WebGPU engine in a dedicated desktop window or immersive full-screen Android WebView with screen keep-awake.
* **🚴 Native Bluetooth LE (GATT) & BLE Manager:** Rust-backed GATT streaming for smart trainers (FTMS / Wahoo), Zwift Ride controllers, power meters, cadence sensors, and HRMs — with silent auto-pairing and an in-app **BLE Device Manager** to inspect, disconnect, or forget pairings.
* **🧩 Addon Catalog, `.zip` Installer & Backup:** browse and install/update community addons from **[frazse/Terra-Addons](https://github.com/frazse/Terra-Addons)** right inside Terra's Cog menu (`Alt+T` or header gear icon), import `.zip` addon bundles, uninstall user addons, and export/import addon settings JSON backups.
* **⚙️ Rich Addon Settings & Live Previews:** addons declare settings (`text`, `number`, `checkbox`, `select`, `color`, write-only `password` secrets) and optional live preview widgets directly in the Cog menu.
* **💪 Live Ride Relay & `window.btApp` Hook:** relays the game's ~2 Hz ride broadcast to Rust (latest frame + ~5 min ring history) via `terra.ride.*` and automatically exposes `window.btApp` on both Linux desktop and Android for in-scene addons.
* **🔌 Local Hardware Networking:** CORS-free HTTPS (`terra.http.request`), fire-and-forget UDP (`terra.udp.send`), and persistent TCP (`terra.tcp.*`) with runtime LAN permission support on Android 17+ (`ACCESS_LOCAL_NETWORK`).
* **💾 Local File Export:** `terra.files.save` saves `.fit`, `.gpx`, `.tcx`, `.json`, or `.csv` exports directly to `Downloads/Terra/`.
* **📁 Android Custom Data Folder:** optional setup & Cog-menu folder picker on Android lets you store `addons/`, `addon-data/`, and `devices.json` in a user-visible folder (e.g. `/storage/emulated/0/Terra`).
* **💬 Discord Rich Presence:** broadcasts active ride status to the local Discord desktop client (including Flatpak and Snap Discord IPC sockets on Linux).
* **📊 Hardware & App Performance Telemetry:** native `terra.perf.stats()` API reporting CPU/GPU utilization & temperatures, memory usage, and Android thermal status.
* **🎞️ Experimental Frame Generation (Linux desktop):** optional `lsfg-vk` / Lossless Scaling integration in the Cog menu with automatic `/ride` & `/spectate` ride-only toggling.

---

## 💻 Supported Platforms

| Platform | Architecture | Runtime | Build Output |
| :--- | :--- | :--- | :--- |
| **Android** | `aarch64-linux-android` | Tauri WebView + Rust JNI | `.apk` |
| **Linux desktop** (x86_64) | `x86_64-unknown-linux-gnu` | Electron / Chromium + Rust `--rpc` over stdio | `AppImage` |
| **Linux desktop** (ARM64) | `aarch64` | Electron / Chromium + Rust `--rpc` over stdio | `AppImage` (GitHub Actions CI) |
| **Windows desktop** (x64) | `x86_64-pc-windows-msvc` | Electron / Chromium + Rust `--rpc` over stdio | `.exe` (GitHub Actions CI) |

> **Prebuilt CI artifacts:** Windows x64 (`.exe`) and Linux ARM64 (`AppImage`) builds are built automatically on GitHub Actions ([`build-windows.yml`](.github/workflows/build-windows.yml), [`build-arm64.yml`](.github/workflows/build-arm64.yml)) and can be downloaded from the repository's **Actions** tab (`Terra-Windows-x64` artifact) or **Releases**.

---

## 🔌 Addons Ecosystem

Addons are Tampermonkey-style JavaScript/CSS bundles (`manifest.json`) evaluated in the page context with access to the `terra.*` API. See **[ADDONS.md](ADDONS.md)** for the complete API reference and manifest specification.

> [!IMPORTANT]
> **Trust model:** addons run inside the `biketerra.com` page with full page privileges, including Bluetooth and network commands. Only install addons you trust.

### Installing & Managing Addons

1. Open the **Cog menu** (click the gear icon in the header, or press `Alt+T`).
2. Click **Catalog** to install/update addons directly from **[frazse/Terra-Addons](https://github.com/frazse/Terra-Addons)**, or use **Install `.zip`** to load a local bundle.
3. Reload the page (**F5** / **Ctrl+R** on desktop) whenever you add or edit local addon files.

**On-disk addon directories:**

* **Linux desktop:** `~/.local/share/terra/addons/` (in-repo `./addons/` is also scanned when running from source; override with `$TERRA_ADDONS_DIR`)
* **Android:** `/storage/emulated/0/Android/data/dev.frazse.terra/files/addons/` (or the custom folder you picked in Terra's storage setup, e.g. `/storage/emulated/0/Terra/addons/`)

> **Packaging note:**
> * The **Android APK** bundles `zwift-ride`, `garmin-hud`, `fps-monitor`, and `wifi-trainer-bridge` (disabled by default) and auto-upgrades bundled copies when newer versions ship.
> * The **Linux AppImage** ships without baked-in addons; install the ones you want from the in-app **Catalog** or into `~/.local/share/terra/addons/`.

<details>
<summary><strong>Available addons in Terra-Addons</strong> (<code>addons/</code> submodule)</summary>

* **`zwift-ride`** — Zwift Ride controller bindings, D-pad UI navigation, widescreen Console UI, virtual OpenBikeControl shifter & battery mirroring
* **`garmin-hud`** — Garmin-style data tiles and rolling zone graphs
* **`nameplates`** — Floating 3D rider nameplates with gap, speed/W/kg, and surge indicators
* **`climb-profile`** — Live gradient and elevation climb overlay
* **`realistic-gears`** — Multi-chainring & cassette simulation for virtual shifting
* **`wifi-trainer-bridge`** — Presents a Wi-Fi Wahoo KICKR (WFTNP over TCP) as a Web Bluetooth trainer
* **`karoo-bridge`** — Streams live ride telemetry over LAN to the Hammerhead Karoo companion APK (`karoo-apk/`)
* **`sunto-export`** — Exports completed cycling and rowing sessions to `.fit` / Suunto
* **`discord-rich-presence`** — Publishes current route, distance, and power/speed to Discord
* **`ghost-rider`** — 3D ghost rider racing with cloned avatar and FIT/GPX track replay
* **`voice-chat`** — Whisper voice-to-text push-to-talk / hands-free chat addon
* **`render-scale`** — Sub-native 3D render scaling (0.5x / 0.75x) for lower-power GPUs
* **`rider-speed`**, **`fps-monitor`**, **`pip-cam`**, **`sbs3d`** — Speed HUD, FPS & hardware telemetry overlay, Picture-in-Picture chase cam, and side-by-side 3D view

</details>

---

## 🛠️ Building from Source

Clone with the addons submodule if you want local addon sources too:

```bash
git clone --recurse-submodules https://github.com/frazse/Terra.git
cd Terra
```

### Prerequisites

* **Rust** (stable toolchain)
* **Node.js** & **npm**
* **Linux desktop build deps:** `libgtk-3-dev`, `libappindicator3-dev`, `librsvg2-dev`, `libssl-dev`, `libdbus-1-dev`, `libudev-dev`, `fuse3`, `pkg-config`
* **Android only:** Android SDK & NDK + `cargo install tauri-cli --version "^2.0"` + `rustup target add aarch64-linux-android`

---

### 🐧 Linux Desktop Build

One-command version sync, Rust release build, and Electron AppImage packaging:

```bash
./build.sh
```

Or manually:

```bash
cargo build --release          # → target/release/terra
cd electron
npm install
npm run dist
```

Output AppImage:
```text
electron/dist/Terra-0.11.2.AppImage
```

#### Development mode
```bash
cargo build --release
cd electron
npm start                      # spawns target/release/terra --rpc alongside Electron
```
* **Addon edits** → press **F5** / **Ctrl+R** in the window (rescanned per page load).
* **Rust (`src/`) or shim (`shim/`) edits** → rebuild and restart Terra.

---

### 📱 Android Build

Use `build-apk.sh` so bundled APK assets stay fresh:

```bash
./build-apk.sh                 # fetch latest Terra-Addons main from GitHub & build APK
./build-apk.sh --pinned        # build offline using the checked-out addons/ submodule
./build-apk.sh --live          # bundle local addons from ~/.local/share/terra/addons/
./build-apk.sh --sync-only     # refresh gen/android assets without running cargo build
```

Or run Tauri directly (after syncing assets):

```bash
cargo tauri android build --apk -t aarch64
```

Output APK:
```text
gen/android/app/build/outputs/apk/universal/release/app-universal-release.apk
```

---

## 📚 Documentation

* **[ADDONS.md](ADDONS.md)** — Addon manifest schema & full `terra.*` JS API reference
* **[AGENTS.md](AGENTS.md)** — Architecture & contributor/agent guide (`src/`, `shim/`, `electron/`, `gen/android/`)
* **[CHANGELOG.md](CHANGELOG.md)** — Release history
* **[karoo-apk/README.md](karoo-apk/README.md)** — Companion Android/Karoo head-unit receiver app

---

## 📜 License

Distributed under the MIT License.
