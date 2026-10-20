#!/usr/bin/env bash
set -euo pipefail

# ──────────────────────────────────────────────────────────────
# Build script for Terra Android / Linux desktop app
#
# Steps:
#   1. Read version from Cargo.toml and sync to all other files
#   2. Build the Rust backend (release)
#   3. Install Electron dependencies
#   4. Build the Electron AppImage
#
# Usage:  ./build.sh
# ──────────────────────────────────────────────────────────────

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

# ── 0. Sync version from Cargo.toml everywhere ────────────────
VERSION=$(grep '^version = ' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/')
echo ">>> Syncing version $VERSION to all config files..."

# electron/package.json
node -e "
const fs = require('fs');
const p = JSON.parse(fs.readFileSync('electron/package.json','utf8'));
p.version = '$VERSION';
fs.writeFileSync('electron/package.json', JSON.stringify(p, null, 2) + '\n');
"

# tauri.conf.json
node -e "
const fs = require('fs');
const p = JSON.parse(fs.readFileSync('tauri.conf.json','utf8'));
p.version = '$VERSION';
fs.writeFileSync('tauri.conf.json', JSON.stringify(p, null, 2) + '\n');
"

# shim/terra-ui.js — initial footer + fallback
sed -i "s/Terra [0-9]*\.[0-9]*\.[0-9]*/Terra $VERSION/g" shim/terra-ui.js

# README.md
sed -i "s|Terra-[0-9]*\.[0-9]*\.[0-9]*\.AppImage|Terra-$VERSION.AppImage|g" README.md

echo ">>> Version synced"

# ── 1. Build Rust backend ─────────────────────────────────────
echo ">>> Building Rust backend (release)..."
RUSTC=~/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin/rustc \
  ~/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin/cargo build --release
echo ">>> Rust build done. Binary at target/release/terra"

# ── 2. Install Electron dependencies ─────────────────────────
echo ">>> Installing Electron dependencies..."
cd "$SCRIPT_DIR/electron"
npm install
echo ">>> Electron dependencies installed"

# ── 3. Build Electron AppImage ───────────────────────────────
echo ">>> Building Electron AppImage..."
npm run dist
echo ""
echo ">>> Build complete! AppImage at electron/dist/Terra-*.AppImage"
