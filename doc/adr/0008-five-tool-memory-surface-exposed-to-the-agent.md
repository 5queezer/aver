# 8. Five-tool memory surface exposed to the agent

Date: 2026-05-06

## Status

Accepted

## Context

The agent needs to read, write, and reflect on memory. Tool surface design has two failure modes:

- **Too few**: the agent overloads one tool with mode flags and gets confused. Single `memory(...)` with a giant `op` enum is the canonical anti-pattern.
- **Too many**: surface bloat eats system-prompt tokens, tool selection becomes ambiguous, and most tools never get called.

The book's Graphify reference uses an essentially append-only `add_triple` plus query [ch.148] — minimal but missing reflection (consolidation, contradiction). An independent design pass via `pi` converged on the same five-tool shape.

## Decision

Expose exactly five tools to the agent:

```
memory.recall(query: str, alpha?: float, hops?: int = 2, top_k?: int = 8)
    → { triples: [...], chunks: [...], subgraph: {...}, confidence_floor: float }
    HybridRAG retrieve (ADR-0004). Default for "what do I know about…" questions.

memory.expand(entity: str, hops: int = 2, predicates?: [str])
    → { nodes: [...], edges: [...] }
    Pure graph neighborhood walk. Use when the entity is already known and
    the agent wants its local structure.

memory.add_triple(subject, predicate, object,
                  confidence?: float, source: str)
    → { triple_id, status: "appended" | "merged" | "conflict" }
    Explicit user-asserted writes. Most writes happen implicitly via the
    post-turn extractor (ADR-0007); this tool is for "remember that…".

memory.contradict(triple_id: str, reason: str, new_claim?: {s, p, o})
    → { contradiction_id, status }
    Flag a known-bad claim. Consolidation pass decides which side gets
    SUPERSEDED (ADR-0005).

memory.consolidate(scope?: "session" | "project" | "all")
    → { merged: int, superseded: int, decayed: int }
    Trigger consolidation. Most callers leave this to the scheduler;
    exposed for debugging and tight feedback loops.
```

Out of scope deliberately:

- **No raw episodic-log query tool.** The log is for audit and replay, not agent introspection. Inspection happens via a separate CLI.
- **No `forget(triple_id)`.** Removal is via `contradict` + consolidation, never silent. Auditability requires it.
- **No schema-mutation tool.** Predicate vocabulary is project config, not agent runtime.

## Consequences

- (+) Small, learnable surface. Every tool has a distinct verb.
- (+) Aligns with Graphify [ch.148] and matches the independent `pi` synthesis — convergent design is a weak signal of fitness.
- (+) Most write traffic bypasses the tool surface entirely (post-turn extractor writes directly to the log) — the agent doesn't have to choose to remember.
- (−) Schema or predicate-vocabulary changes require coordinated config updates; not runtime-extensible.
- (−) Inspection requires a separate CLI; agent can't self-debug the episodic log.
- (−) `consolidate()` exposed to the agent is dangerous if mis-called during a long session — needs rate-limiting.
