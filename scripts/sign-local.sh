#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_PATH="${1:-$ROOT_DIR/src-tauri/target/release/bundle/macos/FLIP.app}"

if [ ! -d "$APP_PATH" ]; then
  echo "Error: App bundle not found at: $APP_PATH" >&2
  echo "Run ./scripts/build-macos.sh first." >&2
  exit 1
fi

MAIN_BIN="$APP_PATH/Contents/MacOS/flip"
BRIDGE_BIN="$APP_PATH/Contents/Resources/flip-macos-bridge"
if [ ! -f "$BRIDGE_BIN" ] && [ -f "$APP_PATH/Contents/MacOS/flip-macos-bridge" ]; then
  BRIDGE_BIN="$APP_PATH/Contents/MacOS/flip-macos-bridge"
fi

if [ ! -f "$MAIN_BIN" ]; then
  echo "Error: Main executable missing at: $MAIN_BIN" >&2
  exit 1
fi
if [ ! -f "$BRIDGE_BIN" ]; then
  echo "Error: Bundled flip-macos-bridge missing inside $APP_PATH" >&2
  exit 1
fi

chmod +x "$MAIN_BIN" "$BRIDGE_BIN"

WAV_NESTED="$APP_PATH/Contents/Resources/resources/flip-tick.wav"
WAV_FLAT="$APP_PATH/Contents/Resources/flip-tick.wav"
if [ ! -f "$WAV_NESTED" ] && [ ! -f "$WAV_FLAT" ]; then
  echo "Error: flip-tick.wav missing from app Resources" >&2
  exit 1
fi

IDENTITY="${APPLE_SIGNING_IDENTITY:-}"

if [ -z "$IDENTITY" ] || [ "$IDENTITY" = "-" ]; then
  echo "==> Applying ad-hoc code signature to $APP_PATH"
  codesign --force --sign - "$BRIDGE_BIN"
  codesign --force --deep --sign - "$APP_PATH"
else
  echo "==> Signing $APP_PATH with identity: $IDENTITY"
  SIGN_ARGS=(--force --options runtime --timestamp --sign "$IDENTITY")
  OUTER_ARGS=(--force --options runtime --timestamp --sign "$IDENTITY")
  if [ -n "${APPLE_ENTITLEMENTS:-}" ] && [ -f "${APPLE_ENTITLEMENTS}" ]; then
    OUTER_ARGS+=(--entitlements "${APPLE_ENTITLEMENTS}")
  fi

  codesign "${SIGN_ARGS[@]}" "$BRIDGE_BIN"
  codesign "${OUTER_ARGS[@]}" "$APP_PATH"

  NOTARY_ZIP=""
  cleanup_notary() {
    if [ -n "$NOTARY_ZIP" ] && [ -f "$NOTARY_ZIP" ]; then
      rm -f "$NOTARY_ZIP"
    fi
  }
  trap cleanup_notary EXIT

  if [ -n "${APPLE_NOTARY_PROFILE:-}" ]; then
    NOTARY_ZIP="$(mktemp /tmp/flip-notary-XXXXXX.zip)"
    ditto -c -k --keepParent "$APP_PATH" "$NOTARY_ZIP"
    echo "==> Submitting to Apple notary service (profile: $APPLE_NOTARY_PROFILE)"
    xcrun notarytool submit "$NOTARY_ZIP" --keychain-profile "$APPLE_NOTARY_PROFILE" --wait
    xcrun stapler staple "$APP_PATH"
  elif [ -n "${APPLE_ID:-}" ] && [ -n "${APPLE_TEAM_ID:-}" ] && [ -n "${APPLE_APP_PASSWORD:-}" ]; then
    NOTARY_ZIP="$(mktemp /tmp/flip-notary-XXXXXX.zip)"
    ditto -c -k --keepParent "$APP_PATH" "$NOTARY_ZIP"
    echo "==> Submitting to Apple notary service (Apple ID)"
    xcrun notarytool submit "$NOTARY_ZIP" \
      --apple-id "$APPLE_ID" \
      --team-id "$APPLE_TEAM_ID" \
      --password "$APPLE_APP_PASSWORD" \
      --wait
    xcrun stapler staple "$APP_PATH"
  elif [ -n "${APPLE_API_KEY:-}" ] && [ -n "${APPLE_API_KEY_ID:-}" ] && [ -n "${APPLE_API_ISSUER:-}" ]; then
    NOTARY_ZIP="$(mktemp /tmp/flip-notary-XXXXXX.zip)"
    ditto -c -k --keepParent "$APP_PATH" "$NOTARY_ZIP"
    echo "==> Submitting to Apple notary service (App Store Connect API key)"
    xcrun notarytool submit "$NOTARY_ZIP" \
      --key "$APPLE_API_KEY" \
      --key-id "$APPLE_API_KEY_ID" \
      --issuer "$APPLE_API_ISSUER" \
      --wait
    xcrun stapler staple "$APP_PATH"
  fi
fi

echo "==> Verifying code signature"
codesign --verify --strict --verbose=2 "$BRIDGE_BIN"
codesign --verify --deep --strict --verbose=2 "$APP_PATH"
echo "Signed and verified: $APP_PATH"
