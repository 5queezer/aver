# 16. Episodic observation projections for compaction continuity

Date: 2026-05-08

## Status

Accepted

## Context

ADR-0014 adds an event-to-claim pipeline: agents record append-only episodic events, extract staged candidate claims at meaningful triggers, then explicitly promote accepted claims into the durable graph. That protects Aver's long-term memory from low-signal chatter and unsupported LLM output.

A separate problem remains: long coding-agent sessions can exceed the live context window before durable claims are ready or appropriate. Agents need continuity across compaction boundaries: what was decided, what failed, what is complete, what exact error occurred, and which user corrections or constraints still matter. Repeated LLM summaries of prior summaries are not acceptable because they drift, lose provenance, and eventually obscure what was actually said.

`pi-observational-memory` demonstrates a useful pattern for this problem: a background observer turns conversation chunks into timestamped, relevance-rated observations; compaction mechanically assembles kept observations and durable reflections; ids allow exact source recall. The pattern is valuable, but Aver's trust model is stricter. Durable memory in Aver must remain structured, privacy-checked, provenance-backed, and replayable from append-only records.

## Decision

Aver will add an episodic observation projection for long-session continuity.

The pipeline is:

```text
episodic_events
  -> observations
  -> session checkpoints / compaction summaries
  -> candidate_claims
  -> promoted durable claims
```

The roles are distinct:

- **Episodic events** remain the canonical append-only record. They are written log-first and mirrored into SQLite as described in ADR-0014.
- **Observations** are compact, timestamped, relevance-rated records derived from one or more episodic events. They are a projection over the event log, not a new source of truth.
- **Session checkpoints / compaction summaries** are mechanically assembled prose views over current observations and previously accepted checkpoint state. They exist to help an agent continue after compaction. They are not durable semantic memory.
- **Candidate claims** remain the only path from session context into durable memory. If a checkpoint/reflection asserts a durable fact, that fact must be proposed as a candidate claim and promoted through the normal validation path.
- **Promoted claims** remain Aver's trusted long-term semantic graph.

Observation records should include at least:

```text
id
session_id
agent_id / agent_kind when available
timestamp
content
relevance: low | medium | high | critical
source_event_ids
derivation metadata such as log offset, extractor id, or model id
```

Observation generation must be implemented behind a pluggable boundary, analogous to `ClaimExtractor`. Offline tests must use deterministic or mock observers. Live LLM observers may be supported later, but they must not be required for deterministic tests.

Every observation must pass the privacy filter before it is persisted. Secret-bearing or otherwise rejected observation content must not be written to SQLite, JSONL projection logs, checkpoint summaries, or downstream candidate claims.

Aver may prune observations and checkpoints to fit a session-continuity budget. Pruning only affects derived projections. It must never delete or rewrite `events.jsonl`, `log.jsonl`, `episodic_events`, promoted claims, or candidate-claim audit history.

Aver should expose recall-by-id for observations and checkpoint items. Given an observation or checkpoint id, the agent should be able to recover the supporting event ids and the exact source event content when available. This is a provenance tool, not semantic search.

Triggering should extend the existing `should_extract_memories` surface rather than introduce an unrelated scheduler. New trigger reasons may include observation-token thresholds, event-count thresholds, session end, idle compaction, explicit remember events, user corrections, and commit completion.

## Consequences

- (+) Agents can preserve task continuity across context compaction without promoting every session detail into durable memory.
- (+) Mechanical compaction summaries avoid summary-of-summary drift.
- (+) Recall-by-id lets agents recover exact evidence behind compacted observations and checkpoints.
- (+) Relevance-aware pruning can control prompt size while keeping critical corrections, durable user assertions, decisions, and concrete completions.
- (+) The design aligns with ADR-0014: observations summarize events, while durable memory still flows through candidate claims and promoted graph claims.
- (+) The projection can support Pi, Claude, Codex, MCP clients, and other harnesses without making Aver depend on a specific agent runtime.
- (-) LLM-generated observations are not deterministic unless their outputs are recorded as auditable projection records. The implementation must choose and document whether observations are replayable projections or non-canonical caches.
- (-) The system gains another state layer: event, observation, checkpoint, candidate claim, promoted claim.
- (-) Bad observation extraction can omit useful working context or preserve too much noise. Evaluation needs to measure continuity quality separately from durable claim quality.
- (-) Privacy mistakes in the observer path would be serious because observations are prose summaries of raw context. The same gate used for claims must apply before persistence.

## Implementation notes

1. Add an `Observation` model and SQLite projection table keyed by stable ids.
2. Add an `Observer` trait with deterministic/mock implementations for tests and optional live implementations outside the offline path.
3. Store source-event provenance for every observation. Unsupported observations are rejected.
4. Add `assemble_compaction_summary(session_id)` as a pure mechanical renderer over selected observations and checkpoint state.
5. Add `recall_observation(id)` or equivalent MCP/CLI surface that returns the observation plus its supporting event content.
6. Extend trigger reporting with observation/checkpoint reasons instead of creating a parallel trigger mechanism.
7. Treat reflections from systems like `pi-observational-memory` as either regenerable checkpoint prose or candidate-claim input. Do not introduce a durable prose-reflection memory tier.
8. Update evaluation fixtures to distinguish three questions: did the agent preserve session continuity, did candidate extraction propose supported claims, and did durable recall return promoted facts correctly?
