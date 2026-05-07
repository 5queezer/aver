# 4. HybridRAG retrieval with adaptive alpha

Date: 2026-05-06

## Status

Accepted

## Context

Pure vector retrieval misses structural relations ("what calls X?", "who owns this module?"). Pure graph traversal misses semantic similarity ("the function that handles payments" — when nothing is named "payments"). The book proposes a tunable blend [ch.146]:

```
score = α · vector_score + (1 − α) · graph_score
```

A fixed α is wrong: structural questions need graph weight; conceptual questions need vector weight. The same query language for both — same retriever, different α.

## Decision

Adaptive HybridRAG:

1. **Query classifier** assigns the query to one of a small set of intents and picks α from a lookup table (initial values; tune with telemetry):

   | Intent                                   | Example                                        |     α |
   |------------------------------------------|------------------------------------------------|------:|
   | Code structure                           | "what calls `validate_token`?"                  | 0.15  |
   | User preference / policy                 | "how should I format diffs?"                    | 0.10  |
   | Bug provenance / decisions               | "why did we switch from Chroma to Qdrant?"      | 0.40  |
   | Exploratory architecture                 | "where is billing handled?"                     | 0.65  |
   | Conceptual / docs                        | "what is the auth model?"                       | 0.80  |

2. **Local retrieval**: 1–2 hop traversal from entities mentioned in the query.
3. **Global retrieval**: vector search over chunk text and node summaries.
4. **Score blend** as above, plus secondary weights for confidence (ADR-0003), recency, and `last_verified_at`.
5. **Pack subgraph, not chunks**, into the agent's context — the book's "currency exchange: tokens for topology" [ch.118].

Default α leans graph-heavy for coding agents because code is relational. Override per call via the `recall(alpha=…)` parameter.

## Consequences

- (+) Adapts to query intent without forcing a paradigm choice.
- (+) Caller can override α explicitly when the classifier mis-routes.
- (+) Subgraph packing is denser per token than raw chunks.
- (−) The classifier is now a quality lever; bad classification → bad retrieval. Treat it as evolvable, log α decisions for tuning.
- (−) Both indices must be **prepared** at query time — connection open, prepared statements cached, indexes present, vector extension loaded. "Prepared" does not mean "fully hydrated into RAM": the SQLite page cache and `sqlite-vss` handle on-demand reads. The Rust CLI (ADR-0013) must keep cold-start under ~50ms even on a project with millions of edges.
- (−) Subgraph serialization for the agent's context is non-trivial — needs a stable, parseable format.
