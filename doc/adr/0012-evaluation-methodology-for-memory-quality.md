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

### Public benchmarks for comparability

The eval suite above is project-specific by design. For external comparability with other memory layers, integrate a small set of public benchmarks. Surveyed and verified 2026-05-07; URLs return 200 and content matches the description.

| Benchmark | Maps to layer | Why it fits | Source |
|-----------|--------------:|-------------|--------|
| **MemoryAgentBench** (Hu et al., ICLR 2026) | Layer 1 + Layer 2 | Closest architectural match: four competencies (Accurate Retrieval, Test-Time Learning, Long-Range Understanding, Conflict Resolution) plus EventQA and FactConsolidation. The FactConsolidation set directly exercises ADR-0005's consolidation contract. | [github](https://github.com/HUST-AI-HYZ/MemoryAgentBench) · [paper](https://arxiv.org/abs/2507.05257) · [HF dataset](https://huggingface.co/datasets/ai-hyz/MemoryAgentBench) |
| **MemoryArena** (He et al., Stanford/UCSD, 2026) | Layer 2 | Tests whether memory *helps* downstream actions, not just recall. Authors explicitly note that LoCoMo-saturated agents fail here — useful regression target. | [site](https://memoryarena.github.io/) · [paper](https://arxiv.org/abs/2602.16313) · [HF dataset](https://huggingface.co/datasets/ZexueHe/memoryarena) |
| **LongMemEval** (Wu et al.) | Layer 1 | Long-term interactive chat memory: extraction, multi-session reasoning, temporal reasoning, abstention. Useful for the abstention-on-low-confidence behavior we built into ADR-0003. | [github](https://github.com/xiaowu0162/longmemeval) · [paper](https://arxiv.org/abs/2410.10813) |
| **LoCoMo** (Snap Research, ACL 2024) | Layer 1 | Mature, widely cited; standard comparability story. Persona-dialogue framing makes it weak for *coding* memory but it's the de-facto baseline in vendor reports. | [github](https://github.com/snap-research/locomo) |
| **HybridRAG-Bench / GraphRAG-Bench** | (Layer 1 retrieval mechanics only) | Tests the graph+vector fusion in ADR-0004; not an *agent memory* benchmark. Use for α-tuning regression, not for end-to-end memory eval. | [HybridRAG-Bench](https://junhongmit.github.io/HybridRAG-Bench/) |

**Starter pair (v0.8)**: integrate **MemoryAgentBench** first (closest fit, includes consolidation eval), then **LongMemEval** for breadth. LoCoMo is added only when vendor comparability becomes an explicit goal.

### What no public benchmark covers

These remain **custom evals** — Layer 1 hand-curated queries and Layer 3 drift snapshots:

- Coding-agent persistent memory (project decisions, debugging hypotheses, API conventions, file/symbol relationships).
- Tri-state confidence drift / inferred-vs-extracted calibration (ADR-0003).
- Consolidation correctness — `merge` / `supersede` / `contradicts` edge accuracy (ADR-0005).
- Privacy-filter regression (ADR-0009) — false negatives are silent leaks.
- Multi-agent shared graphs, trust weighting, community detection (ADR-0011).
- Harmful memory: stale or wrong claims making the agent *worse* than no memory.

Public benchmarks anchor us to the field; the custom evals are where the unique parts of this design live or die.

## Consequences

- (+) Quality regressions become detectable instead of accumulating silently.
- (+) The eval suite doubles as documentation of what the memory layer is *supposed* to do — failing queries are concrete.
- (+) Layer 3 catches drift even in subsystems without query coverage (filters, consolidation).
- (+) Layer 2 quantifies the actual value of the memory layer in dollars/tokens — useful when arguing for keeping it.
- (−) Bootstrap cost: 30–50 hand-curated pairs per project before eval is meaningful. No memory layer ships without an eval suite.
- (−) Goodhart's law risk: optimize-to-eval is real. Mitigated by held-out test set and Layer 3 (which is hard to game).
- (−) Layer 2 needs a judge model and ground-truth answers — non-trivial maintenance, especially as projects evolve.
- (−) Layer 3 anomaly thresholds need tuning per project; a noisy alert system gets ignored. Start permissive, tighten as baselines stabilize.
