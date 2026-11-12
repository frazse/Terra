# Terra 🚲

> [!NOTE]
> **Disclaimer**: Terra is an independent, community-developed open-source project. It is **in no way affiliated with, endorsed by, or associated with [Biketerra.com](https://biketerra.com)**.

**Terra** is an open-source, cross-platform companion runtime for **[Biketerra.com](https://biketerra.com)**. It wraps the game in a real webview, injects a thin shim layer (Web Bluetooth, an addon engine, a settings menu) *before* the game boots, and backs it with a native **Rust** service that does the heavy lifting. Addons (Tampermonkey-style JavaScript) run inside the page with the page's privileges and reach Rust for real Bluetooth, networking, and persistence.

**Stack:** one shared **Rust** backend + **Electron / Chromium** (Linux desktop) + **Tauri WebView** (Android).

---

## 🌟 Key Features

* **🎮 Native WebGPU rendering:** the game's full WebGPU 3D viewport runs in a real desktop/Android browser engine — no browser extension, no tab.
* **🚴 Native Bluetooth LE (GATT):** low-latency, continuous GATT streaming for smart trainers (FTMS / Wahoo), power meters, speed & cadence sensors, and heart-rate monitors — implemented in Rust and exposed to the webview.
* **📱 Immersive Android edge-to-edge display:** borderless full-screen 3D with status-bar management and screen keep-awake.
* **🧠 One Rust backend for everything:** Bluetooth GATT, UDP/TCP networking, Discord Rich Presence, live-ride relay, and per-addon persistence all run in a single local binary (`src/`).
* **💪 Live ride data:** the game's ~2 Hz broadcast is relayed to Rust (latest frame + ~5 min of history) and streamed to addons via `terra.ride.*`.
* **🔌 Networking for real hardware:** `terra.udp.send` (fire-and-forget) and `terra.tcp.*` (persistent streams) let addons talk to bike computers and virtual trainers (e.g. Karoo, Wahoo) on the local network.
* **💾 Local file export:** `terra.files.save` writes `.fit` / `.gpx` / `.tcx` / `.json` / `.csv` directly to `Downloads/Terra/`.
* **🧩 Addon plugin engine:** JavaScript addons (like Tampermonkey userscripts) evaluated in the webview context, with zero CORS restrictions, per-addon persistence, and a shared event bus.
* **⚙️ In-header cog settings menu:** addons declare settings (FTP, thresholds, API keys, colors) rendered directly in Terra's UI.
* **🎮 Discord Rich Presence:** show what you're riding in the desktop Discord client.

---

## 💻 Supported Platforms

| Platform | Architecture | Engine / Viewport | Output |
|:---|:---|:---|:---|
| **Android** | `aarch64-linux-android` | Tauri WebView + Rust JNI | `.apk` |
| **Linux desktop** (x86_64) | `x86_64-unknown-linux-gnu` | Electron / Chromium + Rust JSON-RPC over stdio | `AppImage` |
| **Linux desktop** (ARM64) | `aarch64` | Electron / Chromium + Rust JSON-RPC over stdio | `AppImage` (built by CI) |

---

## 🛠️ Building from Source

### Prerequisites
* **Rust** (stable toolchain)
* **Node.js** & **npm**
* **Cargo Tauri CLI** (`cargo install tauri-cli --version "^2.0"`) — Android only
* **Android SDK & NDK** — Android only

> Linux desktop also needs the usual Chromium/Electron build deps (`libwebkit2gtk-4.1-dev`, `libgtk-3-dev`, `libappindicator3-dev`, `librsvg2-dev`, `libssl-dev`, `libdbus-1-dev`, `libudev-dev`, `fuse3`, `pkg-config`).

---

### 📱 Android Build

Build the release APK for Android (`aarch64`):

```bash
# 1. Add the Android Rust target
rustup target add aarch64-linux-android

# 2. Build the release APK
cargo tauri android build --apk -t aarch64
```

The APK lands at:
```text
gen/android/app/build/outputs/apk/universal/release/app-universal-release.apk
```

---

### 🐧 Linux Desktop Build

Build the Rust backend, then package the Electron AppImage:

```bash
# 1. Build the Rust release binary
cargo build --release          # → target/release/terra

# 2. Install Electron deps and package the AppImage
cd electron
npm install
npm run dist
```

The AppImage is generated at:
```text
electron/dist/Terra-0.8.0.AppImage
```

(Or just run `./build.sh`, which syncs the version everywhere, builds Rust, and packages it.)

#### Development mode
```bash
cargo build --release
cd electron
npm start                      # spawns target/release/terra --rpc alongside
```
* **Addon change** → **F5 / Ctrl+R** in the window (rescanned per page load, no rebuild).
* **Shim or Rust change** → rebuild and restart the app.

---

## 🔌 Addons Ecosystem

Addons are folders with a `manifest.json`. They extend Biketerra with HUD overlays, sensor bridges, ride exporters, rich presence, and more. See **[ADDONS.md](ADDONS.md)** for the full `terra.*` API and manifest schema.

> [!IMPORTANT]
> **Trust model:** addons run *inside* the biketerra.com page with its privileges (like Tampermonkey userscripts), including the Bluetooth commands. Only install addons you trust.

**Bundled out of the box** (`./addons/`, a [Terra-Addons](https://github.com/frazse/Terra-Addons) submodule):
`garmin-hud`, `fps-monitor`, `zwift-ride`, `nameplates`, `realistic-gears`, `sunto-export`, `wifi-trainer-bridge`, `karoo-bridge`, `discord-rich-presence`, `biketerra-logger`.
More addons and install guides live in the **[frazse/Terra-Addons](https://github.com/frazse/Terra-Addons)** repo.

### Addon installation directories

* **Linux desktop**:
  ```text
  ~/.local/share/terra/addons/
  ```
* **Android**:
  ```text
  /storage/emulated/0/Android/data/dev.frazse.terra/files/addons/
  ```

(Terra also scans an in-repo `./addons/` directory — the bundled set above — and `$TERRA_ADDONS_DIR` if set. Drop a folder in, press F5, and enable it in the cog menu.)

---

## 📜 License

Distributed under the MIT License. See `LICENSE` for more information.
