# 3. Confidence tri-state with provenance on every memory edge

Date: 2026-05-06

## Status

Accepted

## Context

Without per-claim confidence and provenance, a memory layer launders LLM hallucinations into "facts": once a triple is written, downstream consumers treat it as ground truth. The book is explicit: without tri-state labeling "every edge looks equally true, and lies travel at the speed of PageRank" [ch.90]. RAG alone reduces but does not eliminate this — incomplete chunks still allow dangerous extrapolation [ch.81].

The agent must distinguish:

- claims directly observed in source text or tool output,
- claims inferred by transitivity or model reasoning,
- claims with conflicting signals,
- claims explicitly asserted by the user.

…and the system must be willing to say "I don't know" rather than dress an inference up as a fact.

## Decision

Every claim in the semantic graph carries:

```
provenance:    USER_ASSERTED | EXTRACTED | INFERRED | AMBIGUOUS
confidence:    0.0 – 1.0
status:        ACTIVE | SUPERSEDED | INVALIDATED
source_refs:   [episode_id, file:line, tool_call_id, ...]
created_at, last_seen_at, last_verified_at
```

Default retrieval (ADR-0004) excludes `AMBIGUOUS` and `SUPERSEDED` claims. `INFERRED` claims surface with a confidence prefix in agent answers ("likely…"). `USER_ASSERTED` is the highest-trust class and bypasses extractor classification.

Confidence scoring follows a small policy table:

| Source                                  | provenance     | confidence |
|-----------------------------------------|----------------|-----------:|
| Explicit user assertion                 | `USER_ASSERTED`| 0.95       |
| Deterministic parser (Tree-sitter, AST) | `EXTRACTED`    | 0.90       |
| Multi-corroborated extraction           | `EXTRACTED`    | 0.75       |
| Single-source LLM inference             | `INFERRED`     | 0.45       |
| Conflicting signals at write time       | `AMBIGUOUS`    | 0.20       |

Contradictions don't overwrite. A new claim that conflicts with an existing one creates an explicit `contradicts` edge; the consolidation pass (ADR-0005) decides which side gets `SUPERSEDED`.

## Consequences

- (+) Hallucinations are quarantined, not laundered.
- (+) Every agent answer can carry provenance. Auditability is built in.
- (+) The agent has a principled basis to say "I don't know."
- (−) Extractor classification becomes a quality lever — misclassification still leaks.
- (−) Storage and ranking overhead per claim.
- (−) Tooling (debuggers, dashboards) needs to surface confidence; raw triple dumps mislead.
