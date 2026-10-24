# 26. Writer-minted ids and a single apply path

Date: 2026-10-24

## Status

Accepted

## Context

ADR-0005 makes the JSONL log the source of truth and SQLite a projection,
and ADR-0019 §4 makes that claim falsifiable with `aver replay`. The design
review of 2026-10-17 found the claim circular: every id that went into the
log was minted with `SELECT MAX(id)+1` against the projection, so a log line
was only meaningful relative to the database that happened to be open when
it was written. Live writes and `replay_with_mode` each had their own INSERT
implementation to keep in sync, claims were appended twice (`log.jsonl` and
`agents/<id>/log.jsonl`) and deduplicated at replay by id-with-conflict
tolerance, and vector chunks were never logged at all although ADR-0017
calls the log their recovery floor. Two memory directories could not be
merged, which blocks shared mode (ADR-0011).

## Decision

### Writer-minted ids

Every logged row id — claims, hyperedges, episodic events, candidate claims,
contradictions, vector chunks — comes from one generator owned by `Store`
(`crates/aver-core/src/ids.rs`), never from the projection:

```text
id = ((unix_ms - EPOCH_MS) << 12) | random12        EPOCH_MS = 2024-01-01T00:00:00Z
```

- 53 bits, so the id survives JSON and JavaScript number handling.
- 41 bits of milliseconds (good until 2093), 12 random low bits so two
  processes minting in the same millisecond collide with probability
  1/4096 per pair.
- Strictly monotonic within a process: a candidate `<= last` becomes
  `last + 1`.
- One generator for all tables, so ids are unique store-wide. That is what
  makes two logs concatenable.
- Seeded at `Store::open` from the last line of each active log series
  (`log`, `events`, `observations`), read from the file tail before
  rotation. The projection is never consulted.

Observation ids and prune-marker ids stay content hashes (ADR-0016).

Ids are sparse. Nothing may assume density, a starting value of 1, or
`id + 1` adjacency; callers use the id a write returns.

### One record type, one apply path

`log::LogRecord` is a `#[serde(tag = "kind")]` enum of every record the log
can carry. Its JSON is the existing on-disk format, field for field, so every
log written so far still parses; fields added after a kind shipped carry a
serde default that reproduces the old behaviour (`scope` → `global`,
`provenance` → derived from `agent_kind`, `decision` → absent).

`apply::apply(conn, &LogRecord)` is the only code in the crate that mutates
the projection tables holding logged state, including the derived side
effects (entity rows, ontology extension, candidate status flips, lifecycle
updates, the vec0 index). It is deterministic: every id and timestamp comes
from the record and the privacy filter does not run inside it (ADR-0019 §4).

Live writes are exactly: validate → privacy gate → mint ids → build
`LogRecord` → `append_jsonl` → `apply`, inside the existing `BEGIN
IMMEDIATE` transaction. Replay is: parse `LogRecord` → `apply`, per line.
Identical duplicate records are skipped; the same id with different content
is `E_REPLAY_DUPLICATE_ID`.

Vector chunks are logged (`add_vector_chunk`, with the embedding when one was
supplied, and `embed_vector_chunk` for backfills) so replay rebuilds
`vector_chunks` and the `vec0` index. The log is now genuinely the recovery
floor ADR-0017 described.

### One log

The per-agent mirror `agents/<agent_id>/log.jsonl` is gone. Agent attribution
is on the record (`agent_id`, `agent_kind`). Existing `agents/` directories
are ignored by replay and never written to.

### Exit criteria

Two tests in `crates/aver-core/tests/write_path.rs` pin the contract: a store
exercising every record kind, rebuilt by `replay` from its JSONL alone, is
row-identical in every table (modulo the migration-stamped seed timestamps
ADR-0019 §4 already excludes) with the same `user_version` and vec0 row
count; and two stores merge by concatenating their log series.

## Consequences

- (+) A log line stands on its own. Replay needs no particular database to
  have minted the ids, and two stores merge by file concatenation.
- (+) One INSERT implementation. A write rule changed in `apply` changes
  live writes and replay together.
- (+) The log is complete: vector chunks and their embeddings replay.
- (+) Half the files on disk: one series per record family, no mirror.
- (−) Ids are large and sparse. Anything that printed or compared dense ids
  (ranges in the compaction summary, tests) had to change.
- (−) Embeddings in the log make `log.jsonl` grow by a few KB per chunk.
  Rotation (ADR-0019 §5) bounds the active file; the archive is still
  "forever".
- (−) Merging two stores written in the same millisecond on different
  machines collides with probability 1/4096 per such pair. `apply` detects
  it as `E_REPLAY_DUPLICATE_ID` rather than silently overwriting; a wider
  random field is a format change for a later ADR if it ever happens.
- (−) `privacy_rejections` counters are telemetry, not logged, and are
  not reproduced by replay. That was already true.
