# 14. Triggered episodic-to-claim memory

Date: 2026-05-07

## Status

Accepted

## Context

Agent memory should not blindly promote every message into durable semantic memory. Continuous open-gate writes contaminate long-term memory with transient chatter, assistant speculation, stale intermediate states, and low-signal facts.

The memory layer already models durable graph claims with provenance, confidence, status, privacy filtering, and consolidation. That claim-oriented model is the right long-term substrate. What is missing is the hippocampus-like layer before it: append-only episodic events that accumulate during a session, then trigger candidate claim extraction when enough durable signal exists.

The guiding pattern is:

1. Record session events immutably.
2. Trigger extraction at meaningful boundaries, not every turn.
3. Let an extractor propose candidate claims.
4. Validate provenance, privacy, confidence, and contradictions.
5. Promote accepted candidates into the durable claim graph.
6. Keep rejected or superseded candidates auditable.

This preserves AML's structured graph advantage while making it usable by Claude, Codex, and MCP clients that naturally produce conversational events.

## Decision

AML adds an explicit event-to-claim pipeline:

- **Episodic events** are append-only session records. They capture user messages, assistant/tool observations, commits, benchmark results, explicit remember requests, corrections, and session/task boundaries.
- **Candidate claims** are staged triples extracted from one or more episodic events. They are not returned by durable recall until promoted.
- **Promotion** converts accepted candidate claims into durable claims using the existing claim graph APIs and source references such as `event:42`.
- **Triggers** decide when extraction should run. Initial deterministic triggers include explicit remember events, session/task end, user corrections, commit completion, and event-count thresholds. LLM-based trigger classifiers may be added later, but must not be required for offline tests.
- **Extractors** propose candidates; they do not directly write durable memory. No candidate can be promoted without event provenance.

## Consequences

- (+) Agents can record rich session context without polluting durable memory.
- (+) Durable memory remains structured, auditable, and graph-friendly.
- (+) LLM extraction becomes safe-by-construction: propose first, validate/promote second.
- (+) MCP tools can expose simple operations (`record_event`, `propose_claim`, `accept_claim`, `recall`) without hiding memory writes in prompts.
- (+) BEAM/MemoryAgentBench can evaluate extraction precision, unsupported claims, and provenance coverage separately from recall.
- (-) The system now has more states: event, candidate, promoted, rejected, durable claim.
- (-) Promotion needs careful log-first behavior so the append-only audit trail remains the source of truth.
- (-) Trigger policy can under-write useful memories or over-write noisy candidates if tuned poorly.

## Implementation notes

The first implementation slice is intentionally small:

1. Add `episodic_events` and `candidate_claims` SQLite tables.
2. Add `Store::record_event`.
3. Add `Store::propose_candidate_claim` requiring event provenance.
4. Add `Store::promote_candidate_claim` that writes a durable claim with `source_refs = ["event:<id>"]`.
5. Keep all tests deterministic and offline; LLM extractors are traits/mocks only until a later milestone.
