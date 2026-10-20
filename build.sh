#!/usr/bin/env bash
set -euo pipefail

# ──────────────────────────────────────────────────────────────
# Build script for Terra Android / Linux desktop app
#
# Steps:
#   1. Build the Rust backend (release)
#   2. Install Electron dependencies
#   3. Build the Electron AppImage
#
# Usage:  ./build.sh
# ──────────────────────────────────────────────────────────────

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

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
