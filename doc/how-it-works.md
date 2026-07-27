# How Aver Works

Aver is a local-first memory layer for coding agents. It keeps durable memories auditable by separating append-only source logs from replayable SQLite projections.

## Runtime flow

```text
Agent / CLI / MCP client
  -> input validation
  -> privacy filter
  -> append-only JSONL log
  -> SQLite projection
  -> recall through keyword, graph, vector, or hybrid retrieval
```

## Storage layout

A memory directory contains the local source of truth and projections:

```text
.aver/
├── log.jsonl           # durable claim and hyperedge audit log
├── events.jsonl        # episodic events and candidate-claim lifecycle
├── observations.jsonl  # continuity observations and prune markers
├── db.sqlite           # replayable query projection
└── auth.db             # local OAuth/session state for the MCP server
```

`log.jsonl` carries both new memories (`add_claim`, `add_hyperedge`) and
claim lifecycle transitions (`retire_claim`, `add_contradiction`,
`supersede_claims`, `decay_confidence`, `merge_source_refs`). Consolidation
logs concrete outcomes (claim ids, post-decay confidences, merged source
references), not the formulas that produced them, so replay asserts the same
transitions even if consolidation policy changes later. Candidate-claim
staging (`propose_candidate_claim`, `promote_candidate_claim`,
`reject_candidate_claim`) is recorded in `events.jsonl` because candidates
reference episodic events and replay applies `events.jsonl` in the same
phase.

## Write path invariants

1. Validate structured fields.
2. Reject secrets, credential paths, and explicit `memory:ignore` content before persistence.
3. Append auditable records before updating SQLite projections.
4. Keep enough provenance to replay or inspect where each memory came from.
5. Allocate ids (claims, hyperedges, events, candidates, contradictions)
   inside a `BEGIN IMMEDIATE` write transaction that also covers the log
   append and the projection insert, so two processes cannot allocate the
   same id and poison the log with duplicates.

Vector chunk writes follow the same privacy boundary as claim, event, observation, and candidate writes.

## Main projections

- **Claims** — structured `(subject, predicate, object)` records with provenance, confidence, status, source references, agent attribution, and scope.
- **Hyperedges** — n-ary memories with role/entity participants for relationships that do not fit simple triples.
- **Episodic events** — raw session events used as extraction and compaction source material.
- **Observations** — privacy-checked continuity notes backed by source event IDs.
- **Vector chunks** — local embedding metadata connected back to claims.

## Server surface

`aver-server` exposes the memory layer over Streamable HTTP MCP. The server uses local OAuth-style registration, PKCE authorization-code exchange, bearer-token validation, scoped tool permissions, and a browser consent flow.

The default posture is localhost-first. Public or reverse-proxy deployments should set explicit CORS origins, protect any trusted identity header at the proxy boundary, and use HTTPS.

## Recovery and maintenance

SQLite tables are projections over append-only records. Maintenance paths include replay, consolidation, vacuum, observation catch-up, coverage reporting, and log rotation. ADRs under [`adr/`](adr/) describe the design trade-offs in more detail.

### Replay semantics

`aver replay` rebuilds `db.sqlite` from the logs. Because every lifecycle
transition is logged, the rebuild reproduces claim status, confidence,
source references, contradictions, candidate states, entity classifications,
and scopes — not just raw claim content. Log records written before a field
existed still replay: a missing `scope` defaults to `global` and a missing
`provenance` falls back to the writer's agent-kind derivation.

Replay is strict by default: the first line that fails to apply aborts the
run (ADR-0019 §4). For disaster recovery, `aver replay --lenient`
quarantines invalid lines instead — each is reported with path, line number,
and error — and continues with the next line, so one poisoned record cannot
block rebuilding every other projection. Replay holds the advisory `.lock`
for the whole run (like vacuum and rotation), so it cannot race a live
store writing to the same memory directory.

Log rotation compresses all three series — `log.jsonl`, `events.jsonl`, and
`observations.jsonl` — into `{series}.{N}.jsonl.gz` at session boundaries.
Replay walks rotated archives in numeric order before the active file of
each series, so no records are skipped. Compression writes a temporary file
and renames atomically; recovery verifies archive integrity before dropping
a plain intermediate, so a crash mid-compression cannot lose log tail
records.

**Derived projections are not logged.** Vector chunks (chunk text plus
embedding vectors and the `vec0` ANN index) are deliberately absent from the
log, so replay rebuilds claims and leaves `vector_chunks` empty. A restore must
first regenerate chunk rows from claim text (for example with
`Store::add_embedded_vector_chunk_for_claim`, which creates a chunk and its
embedding). `Store::backfill_vector_embeddings` is not a standalone restore
path: it only fills embeddings for chunk rows that already exist but have no
embedding. It processes a resumable batch of up to 100 rows; callers that need a
smaller maintenance window can use `backfill_vector_embeddings_with_limit`.
Individual provider failures leave those rows unfilled for a later retry without
rolling back successful rows from the same batch.
