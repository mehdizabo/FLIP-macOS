#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_PATH="$ROOT_DIR/src-tauri/target/release/bundle/macos/FLIP.app"
DMG_PATH=""
UPDATE_CASK=0
CASK_PATH="$ROOT_DIR/packaging/homebrew/flip.rb"

if [[ -n "${FLIP_VERSION:-}" ]]; then
  VERSION="$FLIP_VERSION"
elif command -v /usr/bin/python3 >/dev/null 2>&1 && [[ -f "$ROOT_DIR/src-tauri/tauri.conf.json" ]]; then
  VERSION="$(/usr/bin/python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["version"])' "$ROOT_DIR/src-tauri/tauri.conf.json" 2>/dev/null || echo "0.1.0")"
else
  VERSION="0.1.0"
fi
ARCH="${FLIP_ARCH:-$(uname -m)}"

usage() {
  cat <<'EOF'
Package FLIP.app and an /Applications symlink into a compressed macOS .dmg image.

Usage:
  ./scripts/package-dmg.sh [options] [app-path]

Options:
  --app <path>    Path to built FLIP.app bundle
  --out <path>    Output .dmg path (default: src-tauri/target/release/bundle/dmg/FLIP-<version>-macOS-<arch>.dmg)
  --update-cask   Update version and sha256 in packaging/homebrew/flip.rb
  -h, --help      Show this help message
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --app)
      [[ $# -ge 2 && -n "$2" ]] || { echo "Error: --app requires a path" >&2; exit 1; }
      APP_PATH="$2"
      shift 2
      ;;
    --out)
      [[ $# -ge 2 && -n "$2" ]] || { echo "Error: --out requires a path" >&2; exit 1; }
      DMG_PATH="$2"
      shift 2
      ;;
    --update-cask)
      UPDATE_CASK=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    -*)
      echo "Unknown option: $1" >&2
      usage >&2
      exit 1
      ;;
    *)
      APP_PATH="$1"
      shift
      ;;
  esac
done

if [[ -z "$DMG_PATH" ]]; then
  DMG_PATH="$ROOT_DIR/src-tauri/target/release/bundle/dmg/FLIP-${VERSION}-macOS-${ARCH}.dmg"
elif [[ "$DMG_PATH" != *.dmg && "$DMG_PATH" != *.DMG ]]; then
  DMG_PATH="${DMG_PATH}.dmg"
fi

if [[ ! -d "$APP_PATH" ]]; then
  echo "Error: App bundle not found at: $APP_PATH" >&2
  echo "Run ./scripts/build-macos.sh first." >&2
  exit 1
fi

echo "==> Ensuring app bundle signature is valid"
"$ROOT_DIR/scripts/sign-local.sh" "$APP_PATH"

STAGING_DIR="$(mktemp -d /tmp/flip-dmg-XXXXXX)"
cleanup() {
  if [[ -n "${STAGING_DIR:-}" && -d "$STAGING_DIR" ]]; then
    rm -rf "$STAGING_DIR"
  fi
}
trap cleanup EXIT

echo "==> Staging FLIP.app and /Applications symlink"
ditto "$APP_PATH" "$STAGING_DIR/FLIP.app"
ln -s /Applications "$STAGING_DIR/Applications"

mkdir -p "$(dirname "$DMG_PATH")"
rm -f "$DMG_PATH"

echo "==> Creating compressed DMG at $DMG_PATH"
/usr/bin/hdiutil create \
  -volname "FLIP" \
  -srcfolder "$STAGING_DIR" \
  -ov \
  -format UDZO \
  -imagekey zlib-level=9 \
  "$DMG_PATH"

IDENTITY="${APPLE_SIGNING_IDENTITY:-}"
if [[ -n "$IDENTITY" && "$IDENTITY" != "-" ]]; then
  echo "==> Signing DMG with identity: $IDENTITY"
  codesign --force --timestamp --sign "$IDENTITY" "$DMG_PATH"
  codesign --verify --verbose=2 "$DMG_PATH"

  if [[ -n "${APPLE_NOTARY_PROFILE:-}" ]]; then
    echo "==> Notarizing DMG (profile: $APPLE_NOTARY_PROFILE)"
    xcrun notarytool submit "$DMG_PATH" --keychain-profile "$APPLE_NOTARY_PROFILE" --wait
    xcrun stapler staple "$DMG_PATH"
  elif [[ -n "${APPLE_ID:-}" && -n "${APPLE_TEAM_ID:-}" && -n "${APPLE_APP_PASSWORD:-}" ]]; then
    echo "==> Notarizing DMG (Apple ID)"
    xcrun notarytool submit "$DMG_PATH" \
      --apple-id "$APPLE_ID" \
      --team-id "$APPLE_TEAM_ID" \
      --password "$APPLE_APP_PASSWORD" \
      --wait
    xcrun stapler staple "$DMG_PATH"
  elif [[ -n "${APPLE_API_KEY:-}" && -n "${APPLE_API_KEY_ID:-}" && -n "${APPLE_API_ISSUER:-}" ]]; then
    echo "==> Notarizing DMG (App Store Connect API key)"
    xcrun notarytool submit "$DMG_PATH" \
      --key "$APPLE_API_KEY" \
      --key-id "$APPLE_API_KEY_ID" \
      --issuer "$APPLE_API_ISSUER" \
      --wait
    xcrun stapler staple "$DMG_PATH"
  fi
else
  echo "==> Applying ad-hoc signature to DMG"
  codesign --force --sign - "$DMG_PATH"
  codesign --verify --verbose=2 "$DMG_PATH"
fi

DMG_SHA256="$(shasum -a 256 "$DMG_PATH" | awk '{print $1}')"

if [[ "$UPDATE_CASK" -eq 1 ]]; then
  if [[ ! -f "$CASK_PATH" ]]; then
    echo "Error: Homebrew Cask file not found at: $CASK_PATH" >&2
    exit 1
  fi
  echo "==> Updating Homebrew Cask SHA-256 in $CASK_PATH"
  /usr/bin/python3 - "$CASK_PATH" "$VERSION" "$DMG_SHA256" <<'PY'
import re
import sys

cask_path, version, sha = sys.argv[1], sys.argv[2], sys.argv[3]
with open(cask_path, "r", encoding="utf-8") as f:
    text = f.read()

text_new, v_count = re.subn(r'^(\s*version\s+)"[^"]+"', rf'\1"{version}"', text, count=1, flags=re.MULTILINE)
text_new, s_count = re.subn(r'^(\s*sha256\s+)"[^"]+"', rf'\1"{sha}"', text_new, count=1, flags=re.MULTILINE)
if v_count != 1 or s_count != 1:
    raise SystemExit(f"Failed to update cask version/sha256 in {cask_path} (v={v_count}, s={s_count})")

with open(cask_path, "w", encoding="utf-8") as f:
    f.write(text_new)
PY
fi

echo ""
echo "DMG created:"
echo "  Path:    $DMG_PATH ($(ls -lh "$DMG_PATH" | awk '{print $5}'))"
echo "  SHA-256: $DMG_SHA256"
if [[ "$UPDATE_CASK" -eq 1 ]]; then
  echo "  Cask:    $CASK_PATH (updated)"
fi
