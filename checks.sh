#!/usr/bin/env bash
# checks.sh — project policy gates run by `just check` and CI.

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
# The claim write path must append to the JSONL audit log before the
# projection write (ADR-0005), and `apply.rs` is the only file that may hold
# the projection INSERT (ADR-0026). `claims.rs` owns the write path and must
# contain both markers; a missing file or marker means the path moved, so
# fail instead of silently passing. Every other aver-core source file that
# contains both markers must also keep the append first.
CLAIMS=crates/aver-core/src/claims.rs
APPLY=crates/aver-core/src/apply.rs
if [ ! -f "$CLAIMS" ] || [ ! -f "$APPLY" ]; then
  echo "FAIL: $CLAIMS or $APPLY not found (write path moved? update this check)."
  FAIL=1
else
  if ! grep -q "append_jsonl(" "$CLAIMS" || ! grep -q "apply(" "$CLAIMS"; then
    echo "FAIL: $CLAIMS lacks append_jsonl( and/or apply( (claim write path moved? update this check)."
    FAIL=1
  fi
  if ! grep -q 'INSERT INTO claims' "$APPLY"; then
    echo "FAIL: $APPLY lacks INSERT INTO claims (apply path moved? update this check)."
    FAIL=1
  fi
fi
for SRC in crates/aver-core/src/*.rs; do
  if [ "$SRC" != "$APPLY" ] && grep -q 'INSERT INTO claims' "$SRC"; then
    echo "FAIL: INSERT INTO claims outside $APPLY in $SRC (one apply path violated)."
    FAIL=1
  fi
  APPEND_LINE=$(grep -n "append_jsonl(" "$SRC" | head -1 | cut -d: -f1)
  APPLY_LINE=$(grep -n "apply(" "$SRC" | head -1 | cut -d: -f1)
  if [ -n "$APPEND_LINE" ] && [ -n "$APPLY_LINE" ]; then
    if [ "$APPEND_LINE" -gt "$APPLY_LINE" ]; then
      echo "FAIL: append_jsonl appears after apply( in $SRC (log-first violated)."
      FAIL=1
    fi
  fi
done

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
