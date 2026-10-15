# Terra 🚲

> [!NOTE]
> **Disclaimer**: Terra is an independent, community-developed open-source project. It is **in no way affiliated with, endorsed by, or associated with [Biketerra.com](https://biketerra.com)**.

**Terra** is an open-source, cross-platform companion application and web runtime for **[Biketerra.com](https://biketerra.com)**, built using **Rust**, **Electron Chromium** (Linux Desktop), and **Kotlin** (Android).

It brings native Web Bluetooth LE integration, immersive fullscreen display, and a userscript-style Addon ecosystem to Android mobile devices, tablets, and Linux desktops (x86_64).

---

## 🌟 Key Features

* **⚡ 60+ FPS WebGL Rendering (Linux Desktop)**: Native Chromium viewport delivering smooth hardware-accelerated 3D rendering with minimap, elevation profile, and HUD overlays active simultaneously.
* **🚴 Native Bluetooth LE Integration**: Low-latency, continuous GATT data streaming for Smart Trainers (FTMS / Wahoo), Power Meters, Speed & Cadence sensors, and Heart Rate Monitors.
* **📱 Immersive Android Edge-to-Edge Display**: Runs Biketerra 3D graphics in borderless full-screen mode with status bar management and screen keep-awake (`FLAG_KEEP_SCREEN_ON`).
* **⚙️ Real Native Rust Backend (`terra --rpc`)**: Electron routes all Bluetooth and Addon calls directly to the compiled Rust binary running in the background over stdio JSON-RPC (`src/ble.rs` & `src/addons.rs`).
* **🔌 Addon Plugin Engine**: Extensible JavaScript addon system evaluated directly in the Webview context with zero CORS restrictions.
* **⚙️ In-Header Cog Settings Menu**: Addons can declare configurable settings (API keys, FTP, HR thresholds) rendered directly in Terra's UI header modal.
* **💾 Local File Exporters (`terra.files.save`)**: Save `.fit`, `.gpx`, `.tcx`, `.json`, and `.csv` files directly to `Downloads/Terra/`.

---

## 💻 Supported Platforms

| Platform | Architecture | Engine / Viewport | Output Format |
|:---|:---|:---|:---|
| **Android** | `aarch64-linux-android` | Android WebView + Rust JNI | `.apk` |
| **Linux Desktop** | `x86_64-unknown-linux-gnu` | Chromium 60+ FPS + Rust RPC | AppImage |

---

## 🛠️ Building from Source

### Prerequisites
- **Rust** (stable toolchain)
- **Node.js** & **npm**
- **Cargo Tauri CLI** (`cargo install tauri-cli --version "^2.0"`)
- **Android SDK & NDK** (for Android APK builds)

---

### 📱 Android Build

To build the signed release APK for Android (`aarch64`):

```bash
# 1. Add Android Rust target
rustup target add aarch64-linux-android

# 2. Build Release APK
cargo tauri android build --apk -t aarch64
```

The signed Release APK will be generated at:
```text
gen/android/app/build/outputs/apk/universal/release/app-universal-release.apk
```

---

### 🐧 Linux Desktop Build (Chromium 60+ FPS)

To build the release AppImage with the Electron Chromium runner and native Rust backend:

```bash
# 1. Build the Rust release binary
cargo build --release

# 2. Install Electron dependencies and package AppImage
cd electron
npm install
npm run dist
```

The output AppImage will be generated at:
```text
electron/dist/Terra-0.4.1.AppImage
```

To run in development mode:
```bash
cargo build --release
cd electron
npm start
```

---

## 🔌 Addons Ecosystem

Terra supports custom JavaScript addons that extend Biketerra functionality (such as HUD overlays, Zwift Ride hotkey controllers, and Garmin-style rolling graphs). Default addons (`garmin-hud`, `fps-monitor`, `zwift-ride`) are bundled out-of-the-box.

Official community addons and installation guides are maintained in:
👉 **[frazse/Terra-Addons](https://github.com/frazse/Terra-Addons)**

### Addon Installation Directories

* **Android**:
  ```text
  /storage/emulated/0/Android/data/dev.frazse.terra/files/addons/
  ```

* **Linux Desktop**:
  ```text
  ~/.local/share/dev.frazse.terra/addons/
  ```

---

## 📜 License

Distributed under the MIT License. See `LICENSE` for more information.
