#!/usr/bin/env bash
set -euo pipefail

ORIG_PWD="$PWD"
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

OUT_PATH=""
DRY_RUN=0

if command -v /usr/bin/python3 >/dev/null 2>&1 && [[ -f "$ROOT_DIR/src-tauri/tauri.conf.json" ]]; then
  VERSION="$(/usr/bin/python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["version"])' "$ROOT_DIR/src-tauri/tauri.conf.json" 2>/dev/null || echo "0.1.0")"
else
  VERSION="0.1.0"
fi

usage() {
  cat <<'EOF'
Audit repository privacy and export a clean source-only ZIP archive from Git HEAD
(macOS counterpart to Export-Source.ps1).

Usage:
  ./scripts/export-source.sh [options]

Options:
  --out <path.zip>  Output ZIP archive path (default: dist/FLIP-<version>-source.zip)
  --dry-run         Run privacy audit and print export plan without writing an archive
  -h, --help        Show this help message
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --out)
      [[ $# -ge 2 && -n "$2" ]] || { echo "Error: --out requires a path" >&2; exit 1; }
      OUT_PATH="$2"
      shift 2
      ;;
    --dry-run)
      DRY_RUN=1
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

if [[ -z "$OUT_PATH" ]]; then
  OUT_PATH="$ROOT_DIR/dist/FLIP-${VERSION}-source.zip"
elif [[ "$OUT_PATH" != /* ]]; then
  OUT_PATH="$ORIG_PWD/$OUT_PATH"
fi

"$ROOT_DIR/scripts/audit-repository.sh"

DIRTY_STATUS="$(git status --porcelain)"
if [[ -n "$DIRTY_STATUS" ]]; then
  echo "Warning: working tree has uncommitted changes; git archive exports committed HEAD only." >&2
fi

if [[ "$DRY_RUN" -eq 1 ]]; then
  echo "[dry-run] Would export clean source archive:"
  echo "  Command: git archive --format=zip --prefix=FLIP-macOS-port/ HEAD -o $OUT_PATH"
  exit 0
fi

mkdir -p "$(dirname "$OUT_PATH")"
rm -f "$OUT_PATH"

git archive --format=zip --prefix=FLIP-macOS-port/ HEAD -o "$OUT_PATH"

echo "Source export complete: $OUT_PATH ($(ls -lh "$OUT_PATH" | awk '{print $5}'))"
echo "Source only: excludes .git, target/, node_modules/, dist/, _local/, and local build outputs."
