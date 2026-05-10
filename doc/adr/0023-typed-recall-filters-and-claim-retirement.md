# 23. Typed recall filters and claim retirement

Date: 2026-05-10

## Status

Proposed

## Context

ADR-0008 specifies a five-tool MCP surface; the `recall` and `expand` tools
accept `query`, `top_k`, `alpha`, and `hops`. Every other piece of metadata
on a claim — `agent_id`, `agent_kind`, `predicate`, `confidence`, `status`,
and (after ADR-0021) `scope` — is unfilterable from the read path. Recall
runs a global keyword search and returns whatever ranks high enough,
regardless of whether the writer was a human, an LLM extractor, or a tool;
regardless of whether the predicate is `is` or `depends_on`; regardless of
how confident the writer was.

Two operational symptoms motivate this ADR:

1. **Cross-harness ambiguity.** Today every MCP-driven write lands with
   `agent_id='mcp'` and `agent_kind='EXTERNAL_TOOL'`. A user with both Claude
   Code and a Pi-Hermes agent pointed at the same `~/.aver/db.sqlite` cannot
   ask "show me only what Claude Code has written about this subject." The
   information is in the schema (the writer can stamp `agent_id` per
   ADR-0021's deferred Layer-3-adjacent work) but the read path cannot use
   it.

2. **`contradict` is evidentiary, not retiring.** Verified empirically on
   2026-05-10: `mcp__aver__contradict` against claims 3, 4, 5 created
   contradiction rows (1, 2, 3) and left every claim in `status='ACTIVE'`.
   `recall` continued to surface them. The schema *does* model retirement
   (`status` enum is `ACTIVE | SUPERSEDED | INVALIDATED`, enforced by
   `claims_status_enum_insert/update`) but no MCP tool drives the
   transition. The only path today is direct SQL.

   This is consistent with ADR-0005's append-only philosophy and with the
   role `contradict` is presumably meant to play (record a contradiction so
   later consolidation can decide what to do with it). It is also a
   significant footgun: a user who sees `contradict` and expects "this is
   wrong, hide it" will instead leave the wrong claim live.

These two symptoms share a single fix: stop hiding the schema's existing
discipline behind a minimal tool surface. Expose the filters, and add the
one missing transition.

## Decision

Extend `recall` and `expand` with typed filters, and add a `retire_claim`
MCP tool that performs the documented `ACTIVE → INVALIDATED` transition.
`contradict` keeps its evidentiary role unchanged.

### `recall` and `expand` — new optional parameters

| Parameter        | Type                                | Semantics                                                             |
|------------------|-------------------------------------|-----------------------------------------------------------------------|
| `agent_id`       | `Option<string>`                    | exact match on `claims.agent_id`                                      |
| `agent_kind`     | `Option<AgentKind>`                 | exact match on `claims.agent_kind`                                    |
| `predicate`      | `Option<string>`                    | exact match OR ancestor (uses `predicate_closure` per ADR-0010)       |
| `predicate_walk` | `'exact' \| 'descendants'` = `'exact'` | controls whether `predicate` filter walks the closure                 |
| `min_confidence` | `Option<f64>` (range `[0.0, 1.0]`) | inclusive lower bound on `claims.confidence`                          |
| `status`         | `Option<Status>` = `'ACTIVE'`      | one of `ACTIVE`, `SUPERSEDED`, `INVALIDATED`, or `'any'`              |

All filters AND together. Existing parameters (`query`, `top_k`, `alpha`,
`hops`, plus `scope`/`scope_walk` from ADR-0021) are unchanged. Omitting
every new filter reproduces today's behavior exactly, with one deliberate
deviation: the implicit `status='ACTIVE'` default. Today's recall already
returns only ACTIVE claims (the SQL is hardcoded); promoting that hardcoded
behavior to a documented default with an `'any'` opt-out is the visible
change.

`predicate_walk='descendants'` enables ADR-0010's transitive inference at
the read path: `predicate='depends_on'` with `predicate_walk='descendants'`
matches `calls`, `imports`, `reads_config_from`. This is the affordance
ADR-0010 promised but never wired into the MCP surface.

### `retire_claim` — new MCP tool

```
retire_claim(claim_id: i64, reason: string) -> { claim_id, prior_status, status: 'INVALIDATED' }
```

Behavior:

- Refuses if the claim does not exist (HTTP/JSON-RPC error, not silent).
- Refuses if `status` is already `INVALIDATED` (idempotency is the caller's
  job; double-retire is a programming error and we want to know).
- Records `reason` into the claim's `source_refs` as a JSON entry
  `"retired:<reason>"` so the JSONL log preserves provenance per ADR-0019.
- Updates `last_seen_at` to current time. Does *not* touch `created_at` or
  `write_ts` — retirement is an audit event, not a re-write.
- Idempotent at the JSONL layer: replay reconstructs the same final state
  because the retire event is appended.

This tool exists alongside `contradict`. Their division of labor:

| Tool            | Effect on claim                  | Effect on graph                                | When to use                                                                  |
|-----------------|----------------------------------|------------------------------------------------|------------------------------------------------------------------------------|
| `contradict`    | none (status unchanged)          | adds row to `contradictions(claim_id, reason)` | "This claim conflicts with new evidence; consolidator should reconcile"      |
| `retire_claim`  | `status` → `INVALIDATED`         | none                                           | "This claim is wrong / obsolete / a test artifact; remove from active reads" |

Both are recorded in JSONL. A consolidation pass (ADR-0005) may later upgrade
contradictions into retirements when the conflicting evidence is sufficient,
but that is a separate decision in a separate ADR.

### Tool description discipline

ADR-0008's tool descriptions are the only contract LLMs see. Both new
parameters and the new tool ship with descriptions that explicitly distinguish
`contradict` from `retire_claim`. Without that, an LLM reading "record a
contradiction" will keep using `contradict` for the retirement case.
Specifically:

- `contradict.description` gains a sentence: *"Does NOT change the claim's
  status. Use `retire_claim` when the intent is to invalidate."*
- `retire_claim.description` opens with: *"Mark a claim as INVALIDATED so
  default `recall` queries no longer surface it. For evidentiary
  contradictions that should remain in active reads pending consolidation,
  use `contradict`."*

### Backwards compatibility

- `recall(query=...)` with no other arguments behaves identically to today,
  modulo the now-explicit `status='ACTIVE'` default, which already matches
  the implicit behavior. No JSONL replay is affected.
- `contradict` semantics are unchanged. Existing callers that misused it as
  retirement will continue to misuse it; the discoverability fix is in the
  tool descriptions.
- `retire_claim` is purely additive.

### Out of scope for this ADR

- A `restore_claim` (`INVALIDATED → ACTIVE`) tool. Plausible but not
  motivated; if it ships, it ships in a follow-on ADR with explicit reasoning
  about when restoration is legitimate vs. a sign of bad consolidation.
- Bulk retirement (retiring many claims by query). The current
  `record_observation` / `propose_candidate_claim` MCP shape suggests one-row
  tools; bulk operations belong in `aver-cli` first, MCP later.
- Auto-retirement on consolidation. ADR-0005 specifies consolidation as the
  place this could happen, but the rules belong to a consolidation ADR, not
  this one.
- Filter expansion to `events` and `observations`. `recall_observation` and
  `observation_coverage` have their own filter story; this ADR is scoped to
  the claim-graph reads.

## Consequences

- (+) Multi-harness setups become legible. A user with Claude Code, Codex,
  and a Pi agent sharing one `~/.aver/db.sqlite` can ask "what has each
  written" by `agent_id` filter, once the per-harness identity work lands.
- (+) ADR-0010's transitive predicate inference becomes reachable from the
  MCP surface for the first time. Today the closure tables exist and are
  unused on read.
- (+) `retire_claim` gives MCP clients a documented retirement path. The
  `contradict` footgun is removed by description, not by behavior change.
- (+) `min_confidence` lets agents filter out low-confidence INFERRED claims
  on demand without changing the global `confidence_floor`.
- (+) Single SQL change per filter — no new table, no migration, no closure
  to materialize beyond what ADR-0010 already builds.
- (−) `recall`'s schema becomes wider. LLMs reading tool descriptions face
  more parameters, with corresponding misuse risk (e.g. always passing
  `agent_kind='HUMAN'` because the model thinks "more specific is better").
  Mitigation: every filter is `Option`, and tool descriptions explicitly
  state defaults and discourage spurious narrowing.
- (−) `predicate_walk='descendants'` requires the closure table to be
  populated. ADR-0010's `rebuild_closure` already runs on seed; if a user
  extends `predicate_types` without rebuilding, descendant queries return
  partial results. This is an existing risk, surfaced rather than introduced.
- (−) `retire_claim` and `contradict` are easy to confuse. Tool descriptions
  carry the load. If misuse persists, a follow-on ADR can collapse them
  into a single `assess_claim(verdict: 'contradict' | 'retire', reason)`
  tool, but that is a heavier change.
- (−) The `status='ACTIVE'` default is now visible in the API surface, which
  means a future change to that default is a breaking change. Today it is
  hardcoded SQL; making it explicit pins it.
- (−) `recall` with no filters now returns the same set as before, but the
  documented contract is narrower. Tooling that asserted on the old
  unfiltered shape (test fixtures, eval harnesses) needs to either update
  assertions or pass `status='any'` explicitly. The eval harness in
  `crates/aver-eval` should be audited as part of the Layer 3
  implementation.
