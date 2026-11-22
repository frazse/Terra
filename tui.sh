#!/usr/bin/env bash
# Terra TUI launcher (Linux, dev build).
#
#   ./tui.sh            start Terra with the game window hidden + open the terminal UI
#   ./tui.sh --show     same, but keep the game window visible
#   ./tui.sh --attach   only open the TUI, attaching to an already running Terra
#
# Quitting the TUI (q) also shuts Terra down, except with --attach.
# Electron/backend logs go to $XDG_STATE_HOME/terra/tui-electron.log so they don't garble the TUI.
set -euo pipefail
cd "$(dirname "$0")"

BIN=target/release/terra
[ -x "$BIN" ] || BIN=target/debug/terra
if [ ! -x "$BIN" ]; then
  echo "terra binary not found: run 'cargo build --release' first" >&2
  exit 1
fi

if [ "${1:-}" = "--attach" ]; then
  exec "$BIN" --tui
fi

ELECTRON=electron/node_modules/.bin/electron
if [ ! -x "$ELECTRON" ]; then
  echo "electron not installed: run 'npm install' in ./electron first" >&2
  exit 1
fi

HEADLESS=1
[ "${1:-}" = "--show" ] && HEADLESS=0

LOG="${XDG_STATE_HOME:-$HOME/.local/state}/terra/tui-electron.log"
mkdir -p "$(dirname "$LOG")"
echo "Terra logs: $LOG"

(cd electron && TERRA_TUI=1 TERRA_HEADLESS=$HEADLESS exec ./node_modules/.bin/electron .) >"$LOG" 2>&1 &
EPID=$!
# Electron handles SIGTERM gracefully (BLE disconnect + addon teardown).
trap 'kill -TERM "$EPID" 2>/dev/null || true; wait "$EPID" 2>/dev/null || true' EXIT

"$BIN" --tui
