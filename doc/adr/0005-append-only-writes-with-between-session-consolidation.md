# 5. Append-only writes with between-session consolidation

Date: 2026-05-06

## Status

Accepted

## Context

Doing deduplication, synonym merging, contradiction detection, and confidence decay synchronously on every write is expensive and error-prone — and turns a hot path into a critical-section. The book's hippocampus → neocortex framing fits the constraint exactly: capture episodic fragments cheaply during the session, consolidate into durable memory between sessions [ch.147].

Synchronous resolution also conflicts with ADR-0003: a contradicting write should not silently overwrite without context, but blocking the agent on a resolution dialog is unacceptable.

## Decision

All writes are append-only:

- Every write produces a JSONL record in `memory/log/<session_id>.jsonl`.
- The episodic store mirrors the log into SQLite for indexed access.
- The semantic graph is a **derived projection**, regenerable from the log.

A separate consolidation job runs:

- on session end (`/compact`-equivalent, or after N idle minutes),
- on demand via the `consolidate()` tool (ADR-0008),
- nightly via cron for long-running corpora.

Consolidation is deterministic and idempotent:

1. Replay new log entries since `last_consolidated_at`.
2. Dedup canonical `(s, p, o)` triples.
3. Merge synonyms via vector similarity on entity labels (threshold ≥ 0.92).
4. Detect contradictions; mark older edge `SUPERSEDED` with link to the new claim — never silently overwrite.
5. Decay `INFERRED` confidence by `exp(-Δt/τ)` and unaccessed-since count.
6. Promote `INFERRED` → `EXTRACTED` only after ≥2 corroborating sources.

If consolidation crashes, replay from the last checkpoint. The log is the source of truth.

## Consequences

- (+) Hot path is cheap; agent latency is unaffected by memory-layer bookkeeping.
- (+) Full audit trail (the log is never rewritten).
- (+) Graph and vector stores are rebuildable from the log — disaster recovery is `rm -rf` then replay.
- (+) Contradictions get explicit treatment, not silent overwrite.
- (−) Stale facts persist between consolidation runs.
- (−) Two-stage write means "did the agent actually remember it?" is ambiguous between `appended_to_log` and `consolidated_into_graph`. Tools must surface this.
- (−) Long log + replay can become slow at scale; need checkpointing and segment compaction.

## Amendment (2026-11-19): newest active single-valued survivor

The original duplicate pass kept the oldest row in every exact-triple group. For a single-valued pair whose values run A → B → A, that superseded the newest A as a duplicate; the conflict pass then treated that already-superseded A as a newer replacement for B, leaving no ACTIVE claim.

Consolidation now keeps the newest ACTIVE row in a single-valued duplicate group and the oldest ACTIVE row in a multi-valued group. The multi-valued change lets a re-assertion after retirement survive instead of selecting a retired historical row. Single-valued conflict replacement considers newer claims regardless of status, so retiring the newest value does not revive an older value differently depending on consolidation timing. For sequences without explicit retirement this leaves exactly one ACTIVE claim per single-valued `(subject, predicate, scope, context)` pair: the newest by write order. A → A → B still leaves B, B → A → A leaves the newest A, and multi-valued accumulation is otherwise unchanged.

The same no-revival rule covers identical values. When a single-valued exact-triple group's newest row is INVALIDATED, consolidation supersedes its ACTIVE older duplicates instead of retaining the newest ACTIVE one. Thus A → A followed by retiring the newer A leaves the older A SUPERSEDED whether consolidation runs before or only after retirement. A later ACTIVE re-assertion still becomes the newest survivor, and multi-valued groups remain unchanged.

Lifecycle outcomes remain explicit append-only records; replay applies those records rather than recomputing consolidation. A store already consolidated by the old rule is not silently repaired by running the new consolidation: in particular, an all-SUPERSEDED A → B → A pair has no ACTIVE row to select. Re-assert the current value through the normal claim-write path and consolidate again to create a new ACTIVE latest row. No migration or history rewrite is required.
