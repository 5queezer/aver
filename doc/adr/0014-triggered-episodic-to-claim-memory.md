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

This preserves Aver's structured graph advantage while making it usable by Claude, Codex, and MCP clients that naturally produce conversational events.

## Decision

Aver adds an explicit event-to-claim pipeline:

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

The first implementation slice is intentionally small and has now landed:

1. `episodic_events` and `candidate_claims` SQLite tables exist.
2. `Store::record_event` writes `events.jsonl` first, then mirrors the event into SQLite.
3. `Store::propose_candidate_claim` stages a triple and requires existing event provenance.
4. `Store::reject_candidate_claim` marks unsupported candidates as `REJECTED` with a rejection reason.
5. `Store::promote_candidate_claim` writes a durable claim with `source_refs = ["event:<id>"]`, then marks the candidate `PROMOTED`.
6. Promotion is idempotent: promoting an already-promoted candidate returns the original durable claim id instead of duplicating memory.
7. `Store::should_extract_memories` provides deterministic trigger policy for explicit remember events and event-count thresholds.
8. `ClaimExtractor`, `CandidateClaimDraft`, `MockClaimExtractor`, and `Store::propose_claims_from_extractor` provide an offline extractor boundary. Live LLM extractors remain out of tests.

Future slices should add richer trigger reasons, candidate listing by status/session, stronger contradiction validation before promotion, and MCP tools for the event/candidate lifecycle.
