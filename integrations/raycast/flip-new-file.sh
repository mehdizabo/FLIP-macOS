#!/usr/bin/env bash

# Required parameters:
# @raycast.schemaVersion 1
# @raycast.title New File with FLIP
# @raycast.mode compact
# @raycast.packageName FLIP
#
# Optional parameters:
# @raycast.icon 📄
# @raycast.argument1 { "type": "text", "placeholder": "Kind (md, txt, docx, xlsx, pdf, yaml, folder)", "optional": true }
# @raycast.argument2 { "type": "text", "placeholder": "Target folder (blank = active Finder folder)", "optional": true }
#
# Documentation:
# @raycast.description Create a new file or folder in the active Finder folder (with inline rename), or open the New File wheel.
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

KIND_ARG="${1:-}"
FOLDER_ARG="${2:-}"

if [[ -n "$KIND_ARG" ]]; then
  if [[ -n "$FOLDER_ARG" ]]; then
    "$FLIP_BIN" new --in "$FOLDER_ARG" -- "$KIND_ARG"
  else
    "$FLIP_BIN" new -- "$KIND_ARG"
  fi
else
  "$FLIP_BIN" wheel --mode new
fi
