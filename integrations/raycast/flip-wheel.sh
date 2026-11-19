#!/usr/bin/env bash

# Required parameters:
# @raycast.schemaVersion 1
# @raycast.title Open FLIP Wheel
# @raycast.mode silent
# @raycast.packageName FLIP
#
# Optional parameters:
# @raycast.icon 🎯
# @raycast.argument1 { "type": "dropdown", "placeholder": "Mode", "optional": true, "data": [{"title": "Auto (Finder context)", "value": "auto"}, {"title": "Convert", "value": "convert"}, {"title": "New File", "value": "new"}] }
#
# Documentation:
# @raycast.description Open FLIP's Liquid Glass radial wheel for Finder's selection or active folder.
# @raycast.author FLIP

set -euo pipefail

resolve_flip_cli() {
  if command -v flip >/dev/null 2>&1; then
    command -v flip
    return 0
  fi
  for candidate in \
    "${HOME}/.local/bin/flip" \
    "/opt/homebrew/bin/flip" \
    "/usr/local/bin/flip" \
    "/Applications/FLIP.app/Contents/MacOS/flip" \
    "${HOME}/Applications/FLIP.app/Contents/MacOS/flip"
  do
    if [[ -x "$candidate" ]]; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done
  return 1
}

FLIP_BIN="$(resolve_flip_cli || true)"
if [[ -z "$FLIP_BIN" ]]; then
  echo "FLIP CLI not found. Install FLIP.app or link ~/.local/bin/flip in Settings." >&2
  exit 1
fi

MODE_ARG="${1:-auto}"
case "$MODE_ARG" in
  convert|new)
    "$FLIP_BIN" wheel --mode "$MODE_ARG"
    ;;
  *)
    "$FLIP_BIN" wheel
    ;;
esac
