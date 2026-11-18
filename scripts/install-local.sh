#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_PATH="$ROOT_DIR/src-tauri/target/release/bundle/macos/FLIP.app"
DEST_DIR=""
INSTALL_QUICK_ACTIONS=1
INSTALL_CLI=1
DRY_RUN=0

usage() {
  cat <<'EOF'
Install FLIP.app locally on macOS (macOS counterpart to Install.ps1).

Usage:
  ./scripts/install-local.sh [options]

Options:
  --app <path>        Path to built FLIP.app (default: src-tauri/target/release/bundle/macos/FLIP.app)
  --dest <dir>        Target Applications directory (default: /Applications if writable, else ~/Applications)
  --no-quick-actions  Skip installing Finder Quick Actions (scripts/install-quick-actions.sh)
  --no-cli            Skip creating ~/.local/bin/flip symlink
  --dry-run           Validate source bundle and print planned actions without modifying files
  -h, --help          Show this help message
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --app)
      [[ $# -ge 2 && -n "$2" ]] || { echo "Error: --app requires a path" >&2; exit 1; }
      APP_PATH="$2"
      shift 2
      ;;
    --dest)
      [[ $# -ge 2 && -n "$2" ]] || { echo "Error: --dest requires a directory" >&2; exit 1; }
      DEST_DIR="$2"
      shift 2
      ;;
    --no-quick-actions)
      INSTALL_QUICK_ACTIONS=0
      shift
      ;;
    --no-cli)
      INSTALL_CLI=0
      shift
      ;;
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

APP_PATH="${APP_PATH%/}"

if [[ -z "$DEST_DIR" ]]; then
  if [[ -w "/Applications" ]]; then
    DEST_DIR="/Applications"
  else
    DEST_DIR="${HOME}/Applications"
  fi
fi
DEST_DIR="${DEST_DIR%/}"
if [[ -z "$DEST_DIR" ]]; then
  DEST_DIR="/"
fi

DEST_APP="${DEST_DIR}/FLIP.app"
CLI_DIR="${HOME}/.local/bin"
CLI_LINK="${CLI_DIR}/flip"
DEST_CLI_TARGET="${DEST_APP}/Contents/MacOS/flip"

if [[ ! -d "$APP_PATH" ]]; then
  echo "Error: Source bundle not found at: $APP_PATH" >&2
  echo "Run ./scripts/build-macos.sh first." >&2
  exit 1
fi

MAIN_BIN="${APP_PATH}/Contents/MacOS/flip"
if [[ ! -x "$MAIN_BIN" ]]; then
  echo "Error: Main FLIP executable missing or not executable at: $MAIN_BIN" >&2
  exit 1
fi

BRIDGE_BIN="${APP_PATH}/Contents/Resources/flip-macos-bridge"
if [[ ! -f "$BRIDGE_BIN" && -f "${APP_PATH}/Contents/MacOS/flip-macos-bridge" ]]; then
  BRIDGE_BIN="${APP_PATH}/Contents/MacOS/flip-macos-bridge"
fi
if [[ ! -x "$BRIDGE_BIN" ]]; then
  echo "Error: Bundled flip-macos-bridge missing or not executable inside: $APP_PATH" >&2
  exit 1
fi

echo "==> Validating source bundle signature: $APP_PATH"
codesign --verify --deep --strict "$APP_PATH"

if [[ "$INSTALL_CLI" -eq 1 && -e "$CLI_LINK" && ! -L "$CLI_LINK" ]]; then
  echo "Error: Refusing to overwrite non-symlink file at $CLI_LINK" >&2
  exit 1
fi

SRC_ABS="$(cd "$(dirname "$APP_PATH")" && pwd)/$(basename "$APP_PATH")"
DEST_PARENT_ABS=""
if [[ -d "$DEST_DIR" ]]; then
  DEST_PARENT_ABS="$(cd "$DEST_DIR" && pwd)"
fi

if [[ "$DRY_RUN" -eq 1 ]]; then
  echo "[dry-run] Source bundle valid: $APP_PATH"
  echo "[dry-run] Would ensure destination directory: $DEST_DIR"
  echo "[dry-run] Would copy bundle to: $DEST_APP"
  echo "[dry-run] Would clear com.apple.quarantine on: $DEST_APP"
  if [[ "$INSTALL_QUICK_ACTIONS" -eq 1 ]]; then
    echo "[dry-run] Would run: $ROOT_DIR/scripts/install-quick-actions.sh --app $DEST_APP"
  else
    echo "[dry-run] Skipping Quick Actions (--no-quick-actions)"
  fi
  if [[ "$INSTALL_CLI" -eq 1 ]]; then
    echo "[dry-run] Would link CLI: $CLI_LINK -> $DEST_CLI_TARGET"
  else
    echo "[dry-run] Skipping CLI symlink (--no-cli)"
  fi
  exit 0
fi

mkdir -p "$DEST_DIR"
DEST_PARENT_ABS="$(cd "$DEST_DIR" && pwd)"
DEST_ABS="${DEST_PARENT_ABS}/FLIP.app"

if [[ "$SRC_ABS" == "$DEST_ABS" ]]; then
  echo "==> Source and destination are identical ($DEST_ABS); skipping copy"
else
  echo "==> Installing FLIP.app to $DEST_APP"
  if [[ -e "$DEST_APP" || -L "$DEST_APP" ]]; then
    rm -rf "$DEST_APP"
  fi
  ditto "$APP_PATH" "$DEST_APP"
fi

if command -v xattr >/dev/null 2>&1; then
  echo "==> Clearing com.apple.quarantine attributes (if any)"
  xattr -dr com.apple.quarantine "$DEST_APP" >/dev/null 2>&1 || true
fi

echo "==> Verifying installed bundle signature: $DEST_APP"
codesign --verify --deep --strict "$DEST_APP"

if [[ "$INSTALL_QUICK_ACTIONS" -eq 1 ]]; then
  "$ROOT_DIR/scripts/install-quick-actions.sh" --app "$DEST_ABS"
fi

if [[ "$INSTALL_CLI" -eq 1 ]]; then
  if [[ ! -x "$DEST_CLI_TARGET" ]]; then
    echo "Error: Installed CLI binary missing or not executable at: $DEST_CLI_TARGET" >&2
    exit 1
  fi
  mkdir -p "$CLI_DIR"
  ln -sfn "$DEST_CLI_TARGET" "$CLI_LINK"
  echo "==> Linked CLI: $CLI_LINK -> $DEST_CLI_TARGET"
  case ":$PATH:" in
    *":$CLI_DIR:"*) ;;
    *)
      echo "Note: Add $CLI_DIR to your PATH:"
      echo "  export PATH=\"$CLI_DIR:\$PATH\""
      ;;
  esac
fi

echo ""
echo "FLIP installed cleanly:"
echo "  App: $DEST_APP"
if [[ "$INSTALL_CLI" -eq 1 ]]; then
  echo "  CLI: $CLI_LINK"
fi
