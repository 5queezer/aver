#!/usr/bin/env bash
# autoresearch.checks.sh — pre-keep gates. Any failure ⇒ checks_failed.

set -uo pipefail
cd "$(dirname "$0")"

FAIL=0

echo "=== cargo fmt --check ==="
if ! cargo fmt --all -- --check; then
  echo "FAIL: cargo fmt found formatting issues."
  FAIL=1
fi

echo "=== cargo clippy ==="
if ! cargo clippy --workspace --no-deps -- -D warnings; then
  echo "FAIL: clippy found warnings."
  FAIL=1
fi

echo "=== ADRs unchanged (read-only) ==="
ADR_DIRTY=$(git status --porcelain doc/adr/ | wc -l)
if [ "$ADR_DIRTY" -ne 0 ]; then
  echo "FAIL: ADRs modified — they are read-only without supervisor approval."
  git status --porcelain doc/adr/
  FAIL=1
fi

echo "=== no #[ignore] in tests ==="
if grep -rn '#\[ignore\]' crates/ 2>/dev/null | grep -v -- '#\[ignore *=' | grep -q .; then
  echo "FAIL: a test was marked #[ignore]."
  grep -rn '#\[ignore\]' crates/
  FAIL=1
fi

echo "=== log-first invariant heuristic ==="
# Pair each claims INSERT with an append in the same Rust function. The check
# fails closed when no write path is exercised, so module moves cannot silently
# turn this gate into a no-op.
if ! python3 - <<'PY'
import pathlib
import re
import sys

fn_start = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+[A-Za-z_][A-Za-z0-9_]*")
checked = 0
failed = False

for path in sorted(pathlib.Path("crates/aver-core/src").glob("*.rs")):
    # Replay rebuilds the SQLite projection from already-durable logs, so it is
    # intentionally exempt from the live-write append boundary.
    if path.name == "replay.rs":
        continue
    lines = path.read_text(encoding="utf-8").splitlines()
    starts = [i for i, line in enumerate(lines) if fn_start.match(line)]
    for index, start in enumerate(starts):
        end = starts[index + 1] if index + 1 < len(starts) else len(lines)
        block = lines[start:end]
        append_lines = [start + i + 1 for i, line in enumerate(block) if "append_jsonl(" in line]
        insert_lines = [start + i + 1 for i, line in enumerate(block) if "INSERT INTO claims" in line]
        if not insert_lines:
            continue
        checked += 1
        if not append_lines or min(append_lines) > min(insert_lines):
            name = lines[start].strip()
            print(f"FAIL: claims INSERT is not preceded by append_jsonl in {path}:{start + 1} ({name}).")
            failed = True

if checked == 0:
    print("FAIL: log-first heuristic matched no claim write functions; the check is stale.")
    failed = True

sys.exit(1 if failed else 0)
PY
then
  FAIL=1
fi

echo "=== no committed secrets / env / keys ==="
BAD=$(git ls-files | grep -E '(^|/)(\.env|.*\.pem|.*\.key|id_rsa|id_ed25519)$' || true)
if [ -n "$BAD" ]; then
  echo "FAIL: forbidden files tracked in git:"
  printf '%s\n' "$BAD"
  FAIL=1
fi

if [ $FAIL -eq 0 ]; then
  echo "ALL CHECKS PASSED"
  exit 0
fi
exit 1
