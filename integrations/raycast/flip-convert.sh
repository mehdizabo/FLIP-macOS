#!/usr/bin/env bash

# Required parameters:
# @raycast.schemaVersion 1
# @raycast.title Convert File with FLIP
# @raycast.mode compact
# @raycast.packageName FLIP
#
# Optional parameters:
# @raycast.icon 🔄
# @raycast.argument1 { "type": "text", "placeholder": "File path (blank = Finder selection)", "optional": true }
# @raycast.argument2 { "type": "text", "placeholder": "Format (e.g. PNG, PDF, XLSX)", "optional": true }
#
# Documentation:
# @raycast.description Convert a local file using FLIP without overwriting the original, or open the Convert wheel for Finder's selection.
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

FILE_ARG="${1:-}"
FORMAT_ARG="${2:-}"

if [[ -n "$FILE_ARG" && -n "$FORMAT_ARG" ]]; then
  "$FLIP_BIN" convert --to "$FORMAT_ARG" -- "$FILE_ARG"
elif [[ -n "$FILE_ARG" ]]; then
  "$FLIP_BIN" wheel --mode convert -- "$FILE_ARG"
else
  "$FLIP_BIN" wheel --mode convert
fi
