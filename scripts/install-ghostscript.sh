#!/usr/bin/env bash
set -euo pipefail

# Install what FLIP's PDF/A feature needs: Ghostscript (the `gs` command line
# tool, from Homebrew). Nothing is installed unless --yes is given.

ASSUME_YES=0

usage() {
  cat <<'EOF'
Install the optional Ghostscript tool used by FLIP's PDF/A feature:
the gs command line tool (via Homebrew).

Ghostscript is free software under the AGPL (Artifex). FLIP does not include
it: it is a separate program that you install yourself, and FLIP only runs it.

Usage:
  ./scripts/install-ghostscript.sh [options]

Options:
  --yes     Actually install (without it the script only prints the commands
            it would run and changes nothing)
  -h, --help  Show this help message

If gs is already on your PATH the script says so and does nothing.
Before installing, it prints what Homebrew would add or upgrade
(brew install --dry-run ghostscript).
Homebrew is required and is never installed by this script. No sudo is used.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --yes)
      ASSUME_YES=1
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

# Say what Homebrew may do to the rest of the Mac, in plain words.
print_dependency_warning() {
  cat <<'EOF'
Read this before you install: Homebrew may add several new dependencies for
Ghostscript and may upgrade shared libraries that other programs on this Mac
also use (for example freetype and harfbuzz). On one Mac the dry run showed
about 12 new packages and about 10 upgrades. Your list can be different: the
real one for this Mac is what `brew install --dry-run ghostscript` prints.
EOF
}

if GS_PATH="$(command -v gs 2>/dev/null)"; then
  echo "Ghostscript is already installed: $GS_PATH"
  GS_VERSION="$("$GS_PATH" --version 2>/dev/null || true)"
  if [[ -n "$GS_VERSION" ]]; then
    echo "Version: $GS_VERSION"
    if [[ "${GS_VERSION%%.*}" != "10" ]]; then
      echo "Note: FLIP's PDF/A recipe was measured with Ghostscript 10.x."
    fi
  fi
  echo "Nothing to do."
  exit 0
fi

if [[ "$ASSUME_YES" -eq 0 ]]; then
  cat <<'EOF'
Nothing has been changed. Re-run with --yes to apply these steps:

# Requires Homebrew (this script never installs it): https://brew.sh
brew install --dry-run ghostscript   # shows what Homebrew would add or upgrade
brew install ghostscript

EOF
  print_dependency_warning
  echo ""
  echo "You can run the dry run yourself first, it installs nothing:"
  echo "  brew install --dry-run ghostscript"
  if ! command -v brew >/dev/null 2>&1; then
    echo ""
    echo "Note: Homebrew was not found on this Mac. Install it first (https://brew.sh)."
  fi
  exit 0
fi

if ! command -v brew >/dev/null 2>&1; then
  echo "Error: Homebrew is required to install ghostscript, and it was not found." >&2
  cat >&2 <<'EOF'
Install Homebrew first (this script never installs it for you):
  https://brew.sh
then run this script again with --yes.
EOF
  exit 1
fi

echo "==> What Homebrew would add or upgrade (brew install --dry-run ghostscript):"
if ! brew install --dry-run ghostscript; then
  echo "Error: the dry run failed, so nothing was installed." >&2
  exit 1
fi
echo ""
print_dependency_warning
echo ""

echo "==> Installing ghostscript with Homebrew"
brew install ghostscript

if INSTALLED_GS="$(command -v gs 2>/dev/null)"; then
  echo "==> Ghostscript is installed: $INSTALLED_GS"
else
  echo "Warning: brew finished but gs is not on your PATH yet. Open a new terminal window and run: gs --version" >&2
fi

echo ""
echo "Done. Open FLIP and check Settings -> Engines: the Ghostscript row should be ready."
