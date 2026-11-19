#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

usage() {
  cat <<'EOF'
Audit repository tracked and untracked non-ignored files for forbidden artifacts,
secret tokens, personal local user paths, and commit identity metadata.

Usage:
  ./scripts/audit-repository.sh [--help]
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

if [[ $# -gt 0 ]]; then
  echo "Unknown option: $1" >&2
  usage >&2
  exit 1
fi

CURRENT_USER="$(id -un)"

/usr/bin/python3 - "$ROOT_DIR" "$CURRENT_USER" <<'PY'
import os
import re
import subprocess
import sys

root_dir = sys.argv[1]
current_user = sys.argv[2]

def run_git(args):
    proc = subprocess.run(
        ["git", *args],
        cwd=root_dir,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    if proc.returncode != 0:
        raise RuntimeError(f"git {' '.join(args)} failed: {proc.stderr.strip()}")
    return proc.stdout

tracked = [line for line in run_git(["ls-files", "-z"]).split("\0") if line]
if not tracked:
    raise RuntimeError("No tracked Git source files found.")

untracked = [
    line
    for line in run_git(["ls-files", "-z", "--others", "--exclude-standard"]).split("\0")
    if line
]

forbidden_re = re.compile(
    r"(^|/)(_local|target|dist|node_modules|native-bridge/bin|native-bridge/\.build)/"
    r"|\.(bak|lnk|dll|exe|pdb|pfx|p12|pem|key)$"
    r"|(^|/)\.env"
)

forbidden_tracked = [p for p in tracked if forbidden_re.search(p)]
if forbidden_tracked:
    print("FAIL: Forbidden private/build files tracked in Git:", file=sys.stderr)
    for p in forbidden_tracked:
        print(f"  - {p}", file=sys.stderr)
    sys.exit(1)

forbidden_untracked = [p for p in untracked if forbidden_re.search(p)]
if forbidden_untracked:
    print("FAIL: Forbidden private/build untracked files present:", file=sys.stderr)
    for p in forbidden_untracked:
        print(f"  - {p}", file=sys.stderr)
    sys.exit(1)

secret_patterns = [
    ("GitHub fine-grained PAT", re.compile(r"github_pat_" + r"[A-Za-z0-9_]{15,}")),
    ("GitHub token", re.compile(r"gh[pousr]_" + r"[A-Za-z0-9]{20,}")),
    ("OpenAI/API secret key", re.compile(r"sk-(?:proj-)?" + r"[A-Za-z0-9_-]{20,}")),
    ("AWS access key", re.compile(r"(?:AKIA|ASIA)" + r"[A-Z0-9]{16}")),
    ("Private key block", re.compile(r"-----BEGIN " + r".*PRIVATE " + r"KEY-----")),
]

user_path_re = re.compile(
    r"(?:^|[\s\"'`=:(\\[{<,;|]|file://|/private)/(Users|home)/([A-Za-z0-9_.-]+|<[^>]+>|\.\.\.)(?=/|$|[^A-Za-z0-9_.-])"
)

allowed_fixture_users = {
    "alice",
    "bob",
    "charlie",
    "demo",
    "Demo",
    "test",
    "tester",
    "secretuser",
    "<username>",
    "<name>",
    "...",
}

findings = []
all_files = [(p, "tracked") for p in tracked] + [(p, "untracked") for p in untracked]

for rel_path, kind in all_files:
    abs_path = os.path.join(root_dir, rel_path)
    if os.path.islink(abs_path) or not os.path.isfile(abs_path):
        continue
    with open(abs_path, "rb") as f:
        raw = f.read()
    # Skip binary files (NUL byte heuristic matching git grep -I)
    if b"\x00" in raw:
        continue
    text = raw.decode("utf-8", errors="ignore")
    for line_no, line in enumerate(text.splitlines(), start=1):
        for label, pat in secret_patterns:
            if pat.search(line):
                findings.append(f"{rel_path}:{line_no} [{kind}] {label}")
        for m in user_path_re.finditer(line):
            username = m.group(2)
            if (
                username == current_user
                or username == "mimmo" + "98"
                or username not in allowed_fixture_users
            ):
                findings.append(
                    f"{rel_path}:{line_no} [{kind}] local user path (/{m.group(1)}/{username})"
                )

if findings:
    print("FAIL: Sensitive content or personal local paths found:", file=sys.stderr)
    for item in findings:
        print(f"  - {item}", file=sys.stderr)
    sys.exit(1)

identities = sorted(
    set(
        line
        for line in run_git(
            ["log", "--all", "--format=%an <%ae> | committer: %cn <%ce>"]
        ).splitlines()
        if line
    )
)

print(
    f"Audit PASS: checked {len(tracked)} tracked files and {len(untracked)} untracked non-ignored files."
)
print(
    "No forbidden artifacts, configured secret tokens, or personal local paths matched in current files."
)
print("Commit author/committer identities in history:")
for ident in identities:
    print(f"  - {ident}")
PY
