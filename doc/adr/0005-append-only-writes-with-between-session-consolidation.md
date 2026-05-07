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
