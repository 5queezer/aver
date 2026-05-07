#!/usr/bin/env bash
# autoresearch.sh — measure one TDD evaluation cycle.
# Emits METRIC name=value lines parsed by the autoresearch extension.

set -uo pipefail
cd "$(dirname "$0")"

TEST_OUT=$(cargo test --workspace -q 2>&1)
TEST_EXIT=$?

# Sum "passed" (col 4) and "failed" (col 6) across every "test result:" line.
GREEN=$(printf '%s\n' "$TEST_OUT" | awk '/^test result:/ { s += $4 } END { print s+0 }')
RED=$(printf '%s\n' "$TEST_OUT"   | awk '/^test result:/ { s += $6 } END { print s+0 }')
TOTAL=$((GREEN + RED))

# Milestone heuristic — bumped as files/symbols characteristic of each
# milestone appear. Keep the heuristic conservative so it never over-claims.
MILESTONE=1
[ -f crates/memory-core/src/vector.rs ] && MILESTONE=2
[ -d crates/memory-extractor ] && MILESTONE=3
grep -q "fn privacy_filter" crates/memory-core/src/lib.rs 2>/dev/null && MILESTONE=4
grep -q "fn consolidate"    crates/memory-core/src/lib.rs 2>/dev/null && MILESTONE=5
[ -d crates/memory-extractor/src/prose ] && MILESTONE=6
grep -q "entity_types" migrations/*.sql 2>/dev/null && MILESTONE=7
[ -d eval ] && MILESTONE=8
grep -q "shared_mode\|postgres" Cargo.toml 2>/dev/null && MILESTONE=9

LOC_CORE=$(find crates/memory-core/src -name '*.rs' -exec cat {} + 2>/dev/null | wc -l)
COMMITS=$(git rev-list --count HEAD 2>/dev/null || echo 0)

# Echo trailing test output so the agent can see failure context if any.
printf '%s\n' "$TEST_OUT" | tail -40

echo "METRIC tests_green=$GREEN"
echo "METRIC tests_total=$TOTAL"
echo "METRIC milestone_index=$MILESTONE"
echo "METRIC loc_core=$LOC_CORE"
echo "METRIC commit_count_total=$COMMITS"

# Non-zero on compile fail or any red test, so the autoresearch driver
# records `crash` and the agent sees it.
if [ "$TEST_EXIT" -ne 0 ] || [ "$RED" -gt 0 ]; then
  exit 1
fi
exit 0
