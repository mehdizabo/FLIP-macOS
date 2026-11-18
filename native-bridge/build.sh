#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT_DIR="$ROOT_DIR/bin"
OUT_BIN="$OUT_DIR/flip-macos-bridge"

mkdir -p "$OUT_DIR"

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
  -o "$OUT_BIN"

chmod +x "$OUT_BIN"
echo "Built $OUT_BIN"
