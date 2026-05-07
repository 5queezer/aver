# 12. Evaluation methodology for memory quality

Date: 2026-05-07

## Status

Accepted

## Context

The book is silent on memory-quality evaluation. The architectural ADRs (0002–0011) are all about *building* the memory layer; none of them say how to know when it's working — or, more importantly, when it's regressing.

Without explicit evaluation, three failure classes accumulate silently:

- **Extractor regression**: a model upgrade or prompt tweak shifts triple quality; nobody notices until retrieval starts hallucinating.
- **Retrieval drift**: HybridRAG α-tuning (ADR-0004) or new entity types (ADR-0010) shift ranking; queries that used to work degrade gradually.
- **Filter strictness drift**: privacy filters (ADR-0009) or contradiction logic (ADR-0005) reject too much or too little; the failure is invisible because successful filters log nothing.

This ADR commits to three layers of evaluation, run on a schedule.

## Decision

### Layer 1 — synthetic query suite (precision, recall, MRR)

A curated set of `(query, expected_top_k_triple_ids)` pairs lives in `eval/queries.jsonl`, versioned with the project. Each entry:

```json
{
  "id": "q_001",
  "query": "what calls validate_token?",
  "intent": "code_structure",
  "expected_triple_ids": ["t_42", "t_91", "t_217"],
  "min_score": 0.6,
  "k": 5
}
```

Run on every consolidation pass and on every CI build affecting the extractor or retriever. Tracks:

- precision@k,
- recall@k,
- mean reciprocal rank,
- p50 / p99 query latency,
- per-intent breakdown (intents map to ADR-0004's α table).

Regression threshold: any metric dropping > 5% from the rolling 7-day median fails CI.

Bootstrap: 30–50 hand-curated query/answer pairs per project before the eval is meaningful. Add new pairs whenever a real query fails — the failed query becomes a permanent regression test.

### Layer 2 — hallucination-rate eval (memory-on vs memory-off)

A golden set of questions answerable *only* from memory (e.g., "what was decided about X in session N?"). Two answers are generated:

- **memory-off**: agent runs without the memory layer.
- **memory-on**: agent runs with full memory.

A judge model scores both for factual accuracy against the ground-truth answer. Tracks:

- memory-on accuracy − memory-off accuracy (must be > 0; this is the value of the memory layer),
- memory-on hallucination rate (must be < memory-off, ideally near zero),
- cases where memory-on gets *worse* than memory-off — these surface bad memory ("the memory laundered an incorrect claim").

### Layer 3 — graph-statistics drift detection

A snapshot per consolidation cycle of:

- claim count by `provenance` (USER_ASSERTED / EXTRACTED / INFERRED / AMBIGUOUS),
- mean `confidence` by provenance,
- count of `contradicts` edges,
- ratio of AMBIGUOUS to total claims,
- entity count by `type_id` (ADR-0010),
- consolidation duration and merge/supersede counts (ADR-0005),
- privacy-filter rejection counts by category (ADR-0009).

Stored as time-series in `eval/snapshots/<timestamp>.json`. Anomaly detection (simple z-score on rolling window) alerts on:

- AMBIGUOUS rate > 10%,
- mean INFERRED confidence drifting upward (extractor over-confident),
- privacy-filter rejection rate dropping (filter regression?),
- contradiction count spiking (extractor unstable).

### What's *not* in scope

- **Human eval as a gate**: too slow and expensive to block CI. Used quarterly to recalibrate the judge model in Layer 2.
- **Eval on the eval suite**: don't optimize the system to maximize Layer 1 scores. Reserve a held-out test set (~20% of pairs) that's never used for tuning, only for final reporting.
- **Cross-project benchmarks**: each project's queries are too domain-specific. Comparisons across projects are done on Layer 3 statistics only.

## Consequences

- (+) Quality regressions become detectable instead of accumulating silently.
- (+) The eval suite doubles as documentation of what the memory layer is *supposed* to do — failing queries are concrete.
- (+) Layer 3 catches drift even in subsystems without query coverage (filters, consolidation).
- (+) Layer 2 quantifies the actual value of the memory layer in dollars/tokens — useful when arguing for keeping it.
- (−) Bootstrap cost: 30–50 hand-curated pairs per project before eval is meaningful. No memory layer ships without an eval suite.
- (−) Goodhart's law risk: optimize-to-eval is real. Mitigated by held-out test set and Layer 3 (which is hard to game).
- (−) Layer 2 needs a judge model and ground-truth answers — non-trivial maintenance, especially as projects evolve.
- (−) Layer 3 anomaly thresholds need tuning per project; a noisy alert system gets ignored. Start permissive, tighten as baselines stabilize.
