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
├── log.jsonl           # durable claims, vectors and historical hyperedges
├── events.jsonl        # episodic events and candidate-claim lifecycle
├── observations.jsonl  # historical observations and prune markers, if present
├── db.sqlite           # replayable query projection
└── auth.db             # local OAuth/session state for the MCP server
```

`log.jsonl` carries new memories (`add_claim`, `add_vector_chunk`,
`embed_vector_chunk`), historical hyperedge records, and
claim lifecycle transitions (`retire_claim`, `add_contradiction`,
`supersede_claims`, `decay_confidence`, `merge_source_refs`,
`reverify_claim`). Consolidation
logs concrete outcomes (claim ids, post-decay confidences, merged source
references, refreshed verification times), not the formulas that produced them, so replay asserts the same
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
5. Allocate ids (claims, events, candidates, contradictions)
   inside a `BEGIN IMMEDIATE` write transaction that also covers the log
   append and the projection insert, so two processes cannot allocate the
   same id and poison the log with duplicates.

Vector chunk writes follow the same privacy boundary as claim, event, and candidate writes.

## Main projections

- **Claims** — structured `(subject, predicate, object)` records with typed entity/literal objects, provenance, confidence, status, source references, agent attribution, scope, and optional validity windows.
- **Episodic events** — raw session events that can source candidate or durable claims.
- **Vector chunks** — local embedding metadata connected back to claims.

Historical hyperedge and observation records remain in the schema and replay
path. Current APIs do not write new records of either kind.

## Structured memory and claim validity

Claims use the store ontology's predicates and cardinality. Objects that look
like prose are stored as literals instead of becoming graph entities;
`aver migrate-literals` reclassifies that shape in older stores through
append-only migration records. `aver extract` writes deterministic code facts
from the CLI; extraction is not an MCP tool.

Provenance distinguishes source statements (`EXTRACTED`), model judgments
(`INFERRED`, with capped confidence), conflicting evidence (`AMBIGUOUS`), and
explicit human assertions (`USER_ASSERTED`). A claim can name its source event
directly, and the candidate workflow can stage an event-derived claim before
promotion.

Claims remain auditable as knowledge changes. `contradict` records conflicting
evidence without hiding the claim, `retire_claim` explicitly invalidates it,
and consolidation logs duplicate supersession and confidence decay.
`reverify`/`reverify_claim` record fresh evidence and the confirming writer;
`confirmations` lists those records. Contextual claims may carry evidence
windows, and recall can apply opt-in recency and evidence-age weighting.

## Server surface

`aver-server` exposes the memory layer over Streamable HTTP MCP or local
stdio. HTTP uses local OAuth-style registration, PKCE authorization-code
exchange, bearer-token validation, scoped tool permissions, and a browser
consent flow.

The default posture is localhost-first. Public or reverse-proxy deployments should set explicit CORS origins, protect any trusted identity header at the proxy boundary, and use HTTPS.

## Recovery and maintenance

SQLite tables are projections over append-only records. Maintenance paths
include replay, consolidation, vacuum, literal migration, vector backfill, and
log rotation. ADRs under [`adr/`](adr/) describe the design trade-offs in more
detail.

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
store writing to the same memory directory. The `.lock` file carries an OS
advisory file lock held for the operation's lifetime and released when the
process exits, so a `.lock` file left behind by a crashed process does not
block the next run; the file itself is never deleted. Every log append holds
the same lock *shared* for its open → write → sync, so replay, vacuum, and
rotation (which lock it exclusively) never interleave with an in-flight
append: appends wait up to 4 s while one of them runs (below SQLite's 5 s
busy timeout) and then fail with "advisory lock held", rolling back their
transaction; replay or vacuum refuse with the same error if an append is
mid-flight.

Log rotation compresses all three series — `log.jsonl`, `events.jsonl`, and
`observations.jsonl` — into `{series}.{N}.jsonl.gz` at session boundaries.
Rotation is opportunistic: if the lock is held (an append, replay, vacuum,
or another rotation in progress), `Store::open` skips it and the next open
retries, and it re-checks the thresholds after taking the lock so two
processes cannot rotate the same log twice. Recovery of a crash-interrupted
rotation (`finalize_pending_rotations`) follows the same rule: under a held
lock the `.tmp` archive and intermediate may belong to a live rotation, so
it skips rather than deleting or re-gzipping them. Replay walks rotated archives in numeric order before the active file of
each series, so no records are skipped. Compression writes a temporary file
and renames atomically; recovery verifies archive integrity before dropping
a plain intermediate, so a crash mid-compression cannot lose log tail
records.

**Vector chunks are logged.** Their text, model and embeddings replay through
`add_vector_chunk` and `embed_vector_chunk` records, so the `vec0` ANN index is
rebuilt with the other projections. If an embedding provider failed during a
write, `aver backfill` uses the same provider environment as the server to fill
existing text-only chunks and prints the resulting counts. It is idempotent,
but it does not invent chunk rows for claims that never had one.
