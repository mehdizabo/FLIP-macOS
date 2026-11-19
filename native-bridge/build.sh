#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT_DIR="$ROOT_DIR/bin"
OUT_BIN="$OUT_DIR/flip-macos-bridge"
TMP_OUT_BIN="$OUT_BIN.tmp.$$"

mkdir -p "$OUT_DIR"
rm -f "$TMP_OUT_BIN"

SDK_PATH="$(xcrun --show-sdk-path)"

/usr/bin/swiftc \
  -O \
  -target arm64-apple-macosx15.0 \
  -swift-version 5 \
  -sdk "$SDK_PATH" \
  -framework Foundation \
  -framework AppKit \
  -framework CoreGraphics \
  -framework CoreImage \
  -framework CoreText \
  -framework CoreML \
  -framework Quartz \
  -framework ApplicationServices \
  -framework PDFKit \
  -framework Vision \
  -framework AVFoundation \
  -framework QuickLookThumbnailing \
  -framework UniformTypeIdentifiers \
  -framework ImageIO \
  -framework AudioToolbox \
  "$ROOT_DIR/Sources/Common.swift" \
  "$ROOT_DIR/Sources/FinderBridge.swift" \
  "$ROOT_DIR/Sources/MediaAndDocs.swift" \
  "$ROOT_DIR/Sources/Monitor.swift" \
  "$ROOT_DIR/Sources/main.swift" \
  -o "$TMP_OUT_BIN"

chmod +x "$TMP_OUT_BIN"
/usr/bin/codesign --force --sign - "$TMP_OUT_BIN"
rm -f "$OUT_BIN"
mv -f "$TMP_OUT_BIN" "$OUT_BIN"

for profile_dir in "$ROOT_DIR/../src-tauri/target/debug" "$ROOT_DIR/../src-tauri/target/release"; do
  if [ -d "$profile_dir" ]; then
    TMP_PROFILE_BIN="$profile_dir/flip-macos-bridge.tmp.$$"
    rm -f "$TMP_PROFILE_BIN"
    cp "$OUT_BIN" "$TMP_PROFILE_BIN"
    chmod +x "$TMP_PROFILE_BIN"
    /usr/bin/codesign --force --sign - "$TMP_PROFILE_BIN"
    rm -f "$profile_dir/flip-macos-bridge"
    mv -f "$TMP_PROFILE_BIN" "$profile_dir/flip-macos-bridge"
  fi
done

echo "Built $OUT_BIN"
