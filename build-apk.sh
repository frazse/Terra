#!/usr/bin/env bash
set -euo pipefail

# ──────────────────────────────────────────────────────────────
# Build the Terra APK (Android, aarch64) with FRESH bundled addons.
#
# Syncs the bundled addons into the APK assets:
#     gen/android/app/src/main/assets/addons/<id>/
# then runs:  cargo tauri android build --apk -t aarch64
#
# Addon source (pick one):
#   --github   (default) latest Terra-Addons `main` from GitHub
#   --live     your live addon dir ($TERRA_ADDON_LIVE_DIR or ~/.local/share/terra/addons)
#   --pinned   the addons/ submodule as currently checked out (offline / reproducible)
#
# Usage:
#   ./build-apk.sh                  # latest from GitHub, then build
#   ./build-apk.sh --live           # sync from the live addon dir (local experiments)
#   ./build-apk.sh --pinned         # sync from the pinned submodule checkout
#   ./build-apk.sh --sync-only      # sync assets, skip the cargo build (check the diff)
# ──────────────────────────────────────────────────────────────

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

# Keep in sync with `bundledAddons` in
# gen/android/app/src/main/java/dev/frazse/terra/MainActivity.kt
BUNDLED_ADDONS=("zwift-ride" "garmin-hud" "fps-monitor")
ASSETS_DIR="gen/android/app/src/main/assets/addons"
LIVE_DIR="${TERRA_ADDON_LIVE_DIR:-$HOME/.local/share/terra/addons}"

usage() {
  cat <<'EOF'
Usage: ./build-apk.sh [source] [options]

Sources (pick one):
  --github     fetch the latest Terra-Addons `main` from GitHub (default)
  --live       copy from the live addon dir ($TERRA_ADDON_LIVE_DIR or
               ~/.local/share/terra/addons) — for unpushed local experiments
  --pinned     copy from the addons/ submodule as currently checked out (offline)

Options:
  --sync-only  sync the APK assets and skip the cargo build (to inspect the diff)
  -h, --help   show this help
EOF
}

SOURCE="github"
SYNC_ONLY=0
for arg in "$@"; do
  case "$arg" in
    --github)    SOURCE="github" ;;
    --live)      SOURCE="live" ;;
    --pinned)    SOURCE="pinned" ;;
    --sync-only) SYNC_ONLY=1 ;;
    -h|--help)   usage; exit 0 ;;
    *) echo "unknown flag: $arg" >&2; usage >&2; exit 2 ;;
  esac
done

sync_assets() {
  local from_dir src id
  case "$SOURCE" in
    github)
      echo ">>> Fetching latest Terra-Addons main from GitHub..."
      git -C addons fetch origin
      git -C addons checkout -q main || git -C addons checkout -qb main origin/main
      git -C addons pull --ff-only origin main
      from_dir="$SCRIPT_DIR/addons"
      dirty=$(git -C addons status --porcelain -- "${BUNDLED_ADDONS[@]}" 2>/dev/null || true)
      echo "    addons submodule at: $(git -C addons rev-parse --short HEAD)$( [ -n "$dirty" ] && echo ' (bundled addons dirty — uncommitted edits are included)')"
      ;;
    live)
      if [ ! -d "$LIVE_DIR" ]; then
        echo "live addon dir not found: $LIVE_DIR" >&2
        echo "  (set TERRA_ADDON_LIVE_DIR to override)" >&2
        exit 1
      fi
      from_dir="$LIVE_DIR"
      echo ">>> Using live addon dir: $LIVE_DIR"
      ;;
    pinned)
      from_dir="$SCRIPT_DIR/addons"
      echo ">>> Using the addons/ submodule as checked out: $(git -C addons rev-parse --short HEAD 2>/dev/null || echo '?')"
      ;;
  esac

  echo ">>> Syncing bundled addons into $ASSETS_DIR/"
  mkdir -p "$ASSETS_DIR"
  for id in "${BUNDLED_ADDONS[@]}"; do
    src="$from_dir/$id"
    if [ ! -d "$src" ]; then
      echo "    ! '$id' not found in $from_dir — keeping the existing asset copy" >&2
      continue
    fi
    rm -rf "$ASSETS_DIR/$id"
    cp -a "$src/." "$ASSETS_DIR/$id/"
    local ver
    ver=$(grep -o '"version"[^,}]*' "$ASSETS_DIR/$id/manifest.json" 2>/dev/null | head -1 || true)
    echo "    $id ${ver:- (no manifest version)}"
  done

  # Hard guarantee: every bundled addon ships a manifest inside the APK.
  for id in "${BUNDLED_ADDONS[@]}"; do
    if [ ! -f "$ASSETS_DIR/$id/manifest.json" ]; then
      echo "FATAL: $ASSETS_DIR/$id/manifest.json is missing and there was no source to copy from" >&2
      exit 1
    fi
  done

  local n
  n=$(git status --porcelain -- "$ASSETS_DIR" | wc -l | tr -d ' ')
  echo ">>> Asset dir is $([ "$n" -eq 0 ] && echo 'already in sync' || echo "$n path(s) changed vs HEAD")"
}

sync_assets

if [ "$SYNC_ONLY" -eq 1 ]; then
  echo ">>> --sync-only: skipping the cargo build. Review with:"
  echo "    git -C \"$SCRIPT_DIR\" status -- \"$ASSETS_DIR\""
  exit 0
fi

echo ">>> Building APK (this takes a while)..."
cargo tauri android build --apk -t aarch64
echo ">>> Done. APK: gen/android/app/build/outputs/apk/universal/release/app-universal-release.apk"
