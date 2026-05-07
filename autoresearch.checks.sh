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
LIB=crates/aver-core/src/lib.rs
if [ -f "$LIB" ]; then
  APPEND_LINE=$(grep -n "append_jsonl" "$LIB" | head -1 | cut -d: -f1)
  INSERT_LINE=$(grep -n 'INSERT INTO claims' "$LIB" | head -1 | cut -d: -f1)
  if [ -n "$APPEND_LINE" ] && [ -n "$INSERT_LINE" ]; then
    if [ "$APPEND_LINE" -gt "$INSERT_LINE" ]; then
      echo "FAIL: append_jsonl appears after INSERT INTO claims in lib.rs (log-first violated)."
      FAIL=1
    fi
  fi
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
