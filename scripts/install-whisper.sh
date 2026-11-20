#!/usr/bin/env bash
set -euo pipefail

# Install what FLIP's Transcribe feature needs: the whisper.cpp command line
# tool (`whisper-cli`, from Homebrew) and the multilingual small model.
# Nothing is installed or downloaded unless --yes is given.

MODEL_DIR="$HOME/Library/Application Support/FLIP/models"
MODEL_PATH="$MODEL_DIR/ggml-small.bin"
PART_PATH="$MODEL_PATH.part"
MODEL_URL="https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin"

# SHA-1 of ggml-small.bin as listed in the whisper.cpp project's models/README.md
# (row "small", 466 MiB), checked on 2026-11-20 against the file the script downloaded.
# If a new model build is ever published, update it only from that README, never from memory.
EXPECTED_SHA1="55356645c2b361a969dfd0ef2c5a50d530afd8d5"

ASSUME_YES=0
FORCE=0

usage() {
  cat <<'EOF'
Install the optional Whisper tools used by FLIP's Transcribe feature:
the whisper-cli command line tool (via Homebrew) and the multilingual
small model ggml-small.bin (about 470 MB, downloaded from Hugging Face).

Usage:
  ./scripts/install-whisper.sh [options]

Options:
  --yes     Actually install and download (without it the script only prints
            the commands it would run and changes nothing)
  --force   Download the model again even if it is already present
  -h, --help  Show this help message

Homebrew is required and is never installed by this script. No sudo is used.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --yes)
      ASSUME_YES=1
      shift
      ;;
    --force)
      FORCE=1
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

if [[ "$ASSUME_YES" -eq 0 ]]; then
  cat <<EOF
Nothing has been changed. Re-run with --yes to apply these steps:

# Requires Homebrew (this script never installs it): https://brew.sh
brew install whisper-cpp
mkdir -p "$MODEL_DIR"
# The model is about 470 MB; skipped when $MODEL_PATH exists, unless --force.
curl -L --fail --progress-bar -o "$PART_PATH" "$MODEL_URL"
mv "$PART_PATH" "$MODEL_PATH"
shasum -a 1 "$MODEL_PATH"

Homebrew currently installs the tool as whisper-cli (the formula is now named
whisper.cpp and whisper-cpp is kept as an alias). FLIP also accepts the older
binary name whisper-cpp. The download is checked against a pinned SHA-1.
EOF
  exit 0
fi

if ! command -v brew >/dev/null 2>&1; then
  echo "Error: Homebrew is required to install whisper-cpp, and it was not found." >&2
  cat >&2 <<'EOF'
Install Homebrew first (this script never installs it for you):
  https://brew.sh
then run this script again with --yes.
EOF
  exit 1
fi

if brew list --versions whisper-cpp >/dev/null 2>&1; then
  echo "==> whisper-cpp is already installed; skipping brew install"
else
  echo "==> Installing whisper-cpp with Homebrew"
  brew install whisper-cpp
fi

echo "==> Creating the model folder: $MODEL_DIR"
mkdir -p "$MODEL_DIR"

cleanup_part() {
  if [[ -f "$PART_PATH" ]]; then
    rm -f "$PART_PATH"
    echo "Removed incomplete download: $PART_PATH" >&2
  fi
}

if [[ -f "$MODEL_PATH" && "$FORCE" -eq 0 ]]; then
  echo "==> Model already present; skipping the download: $MODEL_PATH"
  echo "    Use --force to download it again."
else
  trap cleanup_part EXIT
  echo "==> The multilingual small model is about 470 MB."
  echo "    Downloading from: $MODEL_URL"
  curl -L --fail --progress-bar -o "$PART_PATH" "$MODEL_URL"
  mv "$PART_PATH" "$MODEL_PATH"
  trap - EXIT
  echo "==> Saved model to: $MODEL_PATH"
fi

if [[ -z "$EXPECTED_SHA1" ]]; then
  echo "==> Integrity check: no SHA-1 is pinned in this script."
  echo "    Compare the value below with the ggml-small.bin row in the"
  echo "    whisper.cpp project's models/README.md:"
  shasum -a 1 "$MODEL_PATH"
else
  ACTUAL_SHA1="$(shasum -a 1 "$MODEL_PATH" | awk '{print $1}')"
  if [[ "$ACTUAL_SHA1" == "$EXPECTED_SHA1" ]]; then
    echo "==> SHA-1 verified: $ACTUAL_SHA1"
  else
    rm -f "$MODEL_PATH"
    echo "Error: SHA-1 mismatch. Expected $EXPECTED_SHA1, got $ACTUAL_SHA1." >&2
    echo "The downloaded model was deleted; run the script again to retry." >&2
    exit 1
  fi
fi

echo ""
echo "Done. Open FLIP and check Settings -> Engines: Transcribe should be ready."
