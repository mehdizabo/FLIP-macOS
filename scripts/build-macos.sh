#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_PATH="$ROOT_DIR/src-tauri/target/release/bundle/macos/FLIP.app"
CREATE_DMG=0

usage() {
  cat <<'EOF'
Build FLIP for macOS (Swift bridge, Vite frontend, Tauri .app, and local signing).

Usage:
  ./scripts/build-macos.sh [options]

Options:
  --dmg       Also package signed FLIP.app + /Applications symlink into a .dmg
  -h, --help  Show this help message
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dmg)
      CREATE_DMG=1
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

export SDKROOT="${SDKROOT:-$(xcrun --show-sdk-path)}"
export CC="${CC:-/usr/bin/clang}"
export CXX="${CXX:-/usr/bin/clang++}"

if ! command -v pnpm >/dev/null 2>&1; then
  echo "pnpm not found in PATH. Install it with 'corepack enable' (Node.js) or 'brew install pnpm'." >&2
  exit 1
fi

if [ ! -d "$ROOT_DIR/node_modules" ]; then
  echo "==> Installing frontend dependencies"
  (cd "$ROOT_DIR" && pnpm install --frozen-lockfile)
fi

TOTAL_STEPS=4
if [[ "$CREATE_DMG" -eq 1 ]]; then
  TOTAL_STEPS=5
fi

echo "==> [1/${TOTAL_STEPS}] Building native Swift/AppKit bridge"
"$ROOT_DIR/native-bridge/build.sh"

echo "==> [2/${TOTAL_STEPS}] Building TypeScript + Vite frontend"
(cd "$ROOT_DIR" && pnpm run build)

echo "==> [3/${TOTAL_STEPS}] Building Tauri release .app bundle"
(cd "$ROOT_DIR/src-tauri" && cargo tauri build --bundles app)

if [ ! -d "$APP_PATH" ]; then
  echo "Error: Expected bundle not found at $APP_PATH" >&2
  exit 1
fi

BRIDGE_IN_APP="$APP_PATH/Contents/Resources/flip-macos-bridge"
if [ ! -f "$BRIDGE_IN_APP" ]; then
  echo "Error: flip-macos-bridge missing in bundle at $BRIDGE_IN_APP" >&2
  exit 1
fi
chmod +x "$BRIDGE_IN_APP"

if [ ! -f "$APP_PATH/Contents/Resources/resources/flip-tick.wav" ] && [ ! -f "$APP_PATH/Contents/Resources/flip-tick.wav" ]; then
  echo "Error: flip-tick.wav missing from bundle Resources" >&2
  exit 1
fi

echo "==> [4/${TOTAL_STEPS}] Signing and verifying .app bundle"
"$ROOT_DIR/scripts/sign-local.sh" "$APP_PATH"

if [[ "$CREATE_DMG" -eq 1 ]]; then
  echo "==> [5/${TOTAL_STEPS}] Packaging compressed .dmg image"
  "$ROOT_DIR/scripts/package-dmg.sh" "$APP_PATH"
fi

echo ""
echo "Build complete:"
echo "  Bundle: $APP_PATH"
echo "  Bridge: $BRIDGE_IN_APP ($(ls -l "$BRIDGE_IN_APP" | awk '{print $1, $5 " bytes"}'))"
