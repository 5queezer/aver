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

## Amendment (2026-11-12): opt-in recency term

Item 4 above names recency and `last_verified_at` as secondary weights; until this amendment neither was implemented. Hybrid recall had no time signal at all: two ACTIVE claims with equal relevance were ordered by ascending id, and ids are time-ordered (ADR-0026), so the older claim won every tie. The timed fixture `eval/fixtures/recency_stale_over_fresh.json` reproduces this through the real hybrid path: under multi-valued predicates, where consolidation supersedes neither claim, the stale claim takes the single result slot on an exact tie and when its shorter text scores slightly higher on vector similarity.

The score becomes:

```
score = α · vector_score + (1 − α) · graph_score + w · exp(−age / τ)
```

- `age` is `now − last_verified_at` in seconds, falling back to `write_ts` for rows without a verification time. Negative ages (clock skew, future timestamps) count as zero.
- `now` is an explicit argument of `recall_hybrid_claims` and `recall_hybrid_claims_with_filters`. Core never reads the clock while ranking; the server passes the wall clock at the tool boundary, tests and fixtures pass a fixed instant.
- `w` defaults to `0`. At `w = 0` the term is skipped rather than multiplied, so scores and ordering are bit-identical to the two-term blend for every age. Recency is strictly opt-in.
- `HybridWeights::with_recency(w, τ)` accepts `w` in `[0, 1]` and a positive, finite `τ` in seconds. The server opts in with `AVER_RECALL_RECENCY_WEIGHT` and `AVER_RECALL_RECENCY_TAU_DAYS` (default 90); an invalid value is reported once and leaves recency off. There is no per-call MCP parameter.
- The term is additive, so `w` is the largest advantage a just-verified claim can get over a very old one. It must exceed the relevance gaps it is meant to overcome and stay below the gaps it must not. On the fixture the gaps to overcome are at most about 0.026 and the control query (which names the older claim) has a gap of about 0.088; `w = 0.08`, `τ = 90` days passes all four queries, `w = 0.01` leaves a stale claim on top and `w = 0.5` overrides the control. These numbers come from a synthetic bag-of-tokens embedder and are a starting point, not a tuned production value.

### Re-assertion refreshes `last_verified_at`

`last_verified_at` was set once, when the claim row was inserted. Asserting the same triple again writes a second row, and consolidation keeps the oldest row and supersedes the newer one, so the surviving claim kept only its original timestamps and a re-verified claim looked as stale as an abandoned one.

Consolidation now appends a `reverify_claim` record (`ts`, `claim_id`, `verified_at`) for the surviving claim of a duplicate group when a later row of the group is newer than the survivor's current verification time. `verified_at` is the newest `COALESCE(last_verified_at, write_ts)` in the group.

- **Same claim** means what consolidation already merges: equal subject, predicate and object. The scope and agent of the later row are not compared, matching the existing source-reference merge.
- **Outcome, not rule.** The record carries the resulting timestamp, like the other consolidation records (ADR-0005), and is appended before the projection update inside the same write transaction.
- **Monotonic and idempotent.** Applying it sets `last_verified_at` to the larger of the current value and `verified_at`, never below the claim's write time. Replaying it twice, out of order, or after a later record changes nothing. A record is only written when it moves the survivor forward, so repeated consolidation appends nothing.
- **ACTIVE survivors only.** A retired or superseded survivor is not refreshed by a later duplicate.
- **Privacy.** The record holds ids and timestamps only. Claim content passed the privacy gate when its own `add_claim` line was written.
- **Before consolidation** the re-assertion row is itself ACTIVE with a fresh write time, so recency already favours the triple.
- **Older binaries** do not know the `reverify_claim` kind: strict replay stops with `replay: unknown record kind: reverify_claim`, lenient replay quarantines the line and the claim keeps its earlier verification time. A log that contains the record needs this version or later to replay strictly.

Not part of this amendment: read logging, access counters, and ACT-R base-level activation. Recall remains a pure read; age comes only from write-side records.
