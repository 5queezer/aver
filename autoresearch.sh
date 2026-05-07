#!/usr/bin/env bash
# autoresearch.sh — measure noisy recall quality while keeping the workspace green.
# Emits METRIC name=value lines parsed by the autoresearch extension.

set -uo pipefail
cd "$(dirname "$0")"

TEST_OUT=$(cargo test --workspace -q 2>&1)
TEST_EXIT=$?

# Sum "passed" (col 4) and "failed" (col 6) across every "test result:" line.
GREEN=$(printf '%s\n' "$TEST_OUT" | awk '/^test result:/ { s += $4 } END { print s+0 }')
RED=$(printf '%s\n' "$TEST_OUT"   | awk '/^test result:/ { s += $6 } END { print s+0 }')
TOTAL=$((GREEN + RED))

BENCH_OUT=""
BENCH_EXIT=0
if [ "$TEST_EXIT" -eq 0 ] && [ "$RED" -eq 0 ]; then
  BENCH_OUT=$(cargo run -q -p aver-eval -- \
    eval/fixtures/basic_recall.json \
    eval/fixtures/conflict_and_noise.json \
    eval/fixtures/ambiguous_single_token.json \
    eval/fixtures/single_token_multi_answer.json \
    eval/fixtures/natural_query_noise.json \
    eval/fixtures/camel_case_memory_terms.json \
    eval/fixtures/abstention.json \
    eval/fixtures/predicate_role_morphology.json \
    eval/fixtures/acronym_expansion.json \
    eval/fixtures/camel_case_acronym_query.json \
    eval/fixtures/versioned_acronym_identifier.json \
    eval/fixtures/versioned_camel_case_identifier.json \
    eval/fixtures/mixed_case_numeric_identifier.json \
    eval/fixtures/mixed_case_prefix_identifier.json \
    eval/fixtures/ies_plural_morphology.json 2>&1)
  BENCH_EXIT=$?
fi

# Milestone heuristic — bumped as files/symbols characteristic of each
# milestone appear. Keep the heuristic conservative so it never over-claims.
MILESTONE=1
[ -f crates/aver-core/src/vector.rs ] && MILESTONE=2
[ -d crates/aver-extractor ] && MILESTONE=3
grep -q "fn privacy_filter" crates/aver-core/src/lib.rs 2>/dev/null && MILESTONE=4
grep -q "fn consolidate"    crates/aver-core/src/lib.rs 2>/dev/null && MILESTONE=5
[ -d crates/aver-extractor/src/prose ] && MILESTONE=6
grep -q "entity_types" migrations/*.sql 2>/dev/null && MILESTONE=7
[ -d eval ] && MILESTONE=8
grep -q "shared_mode\|postgres" Cargo.toml 2>/dev/null && MILESTONE=9

LOC_CORE=$(find crates/aver-core/src -name '*.rs' -exec cat {} + 2>/dev/null | wc -l)
COMMITS=$(git rev-list --count HEAD 2>/dev/null || echo 0)

# Echo trailing test output so the agent can see failure context if any.
printf '%s\n' "$TEST_OUT" | tail -40

echo "METRIC tests_green=$GREEN"
echo "METRIC tests_total=$TOTAL"
echo "METRIC milestone_index=$MILESTONE"
echo "METRIC loc_core=$LOC_CORE"
echo "METRIC commit_count_total=$COMMITS"

if [ "$BENCH_EXIT" -eq 0 ] && [ -n "$BENCH_OUT" ]; then
  printf '%s\n' "$BENCH_OUT"
  printf '%s\n' "$BENCH_OUT" | python3 -c 'import json,sys; m=json.load(sys.stdin); memory_error_rate=m["unsupported_claim_rate"]+(1-m["mean_recall_at_k"]); print("METRIC memory_error_rate={}".format(memory_error_rate)); print("METRIC unsupported_claim_rate={}".format(m["unsupported_claim_rate"])); print("METRIC mean_precision_at_k={}".format(m["mean_precision_at_k"])); print("METRIC mean_recall_at_k={}".format(m["mean_recall_at_k"]))'
fi

# Non-zero on compile fail, any red test, or benchmark failure.
if [ "$TEST_EXIT" -ne 0 ] || [ "$RED" -gt 0 ] || [ "$BENCH_EXIT" -ne 0 ]; then
  exit 1
fi
exit 0
