# Terra 🚲

**Terra** is an open-source, cross-platform companion application and web runtime for **[Biketerra.com](https://biketerra.com)**, built using **[Tauri v2](https://tauri.app)**, Rust, and Kotlin.

It brings native Web Bluetooth integration, immersive fullscreen display, and a userscript-style Addon ecosystem to Android mobile devices, tablets, and Linux desktops (x86_64 & ARM64).

---

## 🌟 Key Features

* **🚴 Native Bluetooth LE Integration**: Low-latency, continuous sensor data streaming for Smart Trainers (FTMS / Wahoo), Power Meters, Speed & Cadence sensors, and Heart Rate Monitors.
* **📱 Immersive Edge-to-Edge Display**: Runs Biketerra 3D graphics in borderless full-screen mode with status bar management and screen keep-awake (`FLAG_KEEP_SCREEN_ON`).
* **🎮 In-Page Bluetooth Chooser**: Custom device discovery modal with automatic supervisor auto-reconnect logic.
* **🔌 Addon Plugin Engine**: Extensible JavaScript addon system evaluated directly in the Webview context with zero CORS restrictions.
* **⚙️ Header Cog Menu Settings**: Addons can declare configurable settings (API keys, FTP, HR thresholds) rendered directly in Terra's UI header modal.
* **💾 Local File Exporters (`terra.files.save`)**: Save `.fit`, `.gpx`, `.tcx`, `.json`, and `.csv` files directly to `Downloads/Terra/`.
* **🔒 Persistent Sessions**: Automatic cookie and token disk synchronization across restarts and process lifecycle events.

---

## 💻 Supported Platforms

| Platform | Target | Output Format |
| :--- | :--- | :--- |
| **Android** | `aarch64-linux-android` | `.apk` |
| **Linux Desktop** | `x86_64-unknown-linux-gnu` | AppImage, `.deb`, `.rpm` |
| **Linux ARM64** | `aarch64-unknown-linux-gnu` | AppImage, `.deb` |

---

## 🛠️ Building from Source

### Prerequisites
- **Rust** (stable toolchain)
- **Node.js** & **Cargo Tauri CLI** (`cargo install tauri-cli --version "^2.0"`)
- **Android SDK & NDK** (for Android APK builds)

---

### 📱 Android Build

To build the release APK for Android (`aarch64`):

```bash
# 1. Add Android Rust target
rustup target add aarch64-linux-android

# 2. Build Release APK
cargo tauri android build --apk -t aarch64
```

The signed Release APK will be created at:
```text
gen/android/app/build/outputs/apk/universal/release/app-universal-release.apk
```

---

### 🐧 Linux Desktop Build

To build and run natively on Linux desktop:

```bash
# Development mode
cargo run

# Production build (AppImage / DEB / RPM)
cargo tauri build
```

The output bundles will be created in:
```text
target/release/bundle/appimage/Terra_0.1.0_amd64.AppImage
```

---

### 🖥️ Linux ARM64 AppImage Build (Raspberry Pi / Rockchip)

Build using **Podman** or **Docker** on Btrfs / ext4 Linux hosts:

```bash
podman run --rm --platform linux/arm64 \
  -v (pwd):/app -w /app \
  debian:bookworm \
  bash -c "
    apt-get update && apt-get install -y \
      build-essential curl wget file libssl-dev libgtk-3-dev \
      libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf fuse3
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source \$HOME/.cargo/env
    cargo install tauri-cli --version '^2.0'
    cargo tauri build
  "
```

---

## 🔌 Addons Ecosystem

Terra supports custom JavaScript addons that extend Biketerra functionality (such as HUD overlays, Zwift Ride hotkey controllers, and Garmin-style rolling graphs).

Official community addons and installation guides are maintained in the separate repository:
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
  *(Or `./addons/` in your local directory when running in development mode).*

---

## 📜 License

Distributed under the MIT License. See `LICENSE` for more information.
