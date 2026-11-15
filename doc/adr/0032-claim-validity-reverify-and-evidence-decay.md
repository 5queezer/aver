# 32. Claim validity, explicit reverify and evidence-driven decay

Date: 2026-11-15

## Status

Accepted

## Context

A lesson an agent learns is usually true only under some context and for the evidence it has seen. aver could not say that. Claims had no context or valid-time fields, `last_verified_at` moved only when consolidation saw the same triple again, and the only relevance decay was wall-clock recency (`AVER_RECALL_RECENCY_TAU_DAYS`). The unused `decay_inferred_confidence_at` is wall-clock too.

Recency is not relevance. A lesson that has been confirmed once and then not re-examined while many new matching cases arrived should rank lower. A lesson whose context did not recur should not decay just because days passed. Agents that replay history to form hypotheses also need replay evidence to stay apart from live evidence: replay may propose, only live or pre-declared out-of-sample evidence may confirm.

## Decision

### Validity on the claim

A claim may carry one `context` (an opaque entity id) and a window `valid_from`/`valid_until` (Unix seconds). They are nullable columns on `claims` (migration 0013) and optional fields of the `AddClaim` log record, so old logs replay as "no context". A window needs a context; `valid_from <= valid_until`; the context passes the privacy filter and is ensured as an entity. Duplicate identity and single-valued replacement are partitioned by `context` as well as `scope`, NULL-safe (`IS`), so the same statement under two contexts is two claims that never supersede each other, and claims without a context deduplicate and supersede exactly as before.

Columns, not a hyperedge: hyperedges have no scope, no typed claim reference and no window, so one link would need three schema changes and a new tool family. Not claims about claims: they would need string references without integrity and several non-atomic writes per lesson.

### Two generic core predicates

- `in_context` (parent `relates_to`, multi-valued, entity object): `<item> in_context <context>`.
- `closed_at` (parent `relates_to`, single-valued, literal): marks an evidence item as having an outcome. Its value is informational; the item's arrival is the logged `created_at` of the `closed_at` claim.

They are core because the decay rule below keys on them. A store whose `ontology.json` declares either with a different shape fails to open (ADR-0031), so a deployment checks its file before upgrading.

### Explicit reverify

`reverify_claim(claim_id, evidence, attribution, valid_until?)` (MCP `reverify_claim`, `claims:write`; CLI `aver reverify`) records that a claim was re-confirmed. The existing `ReverifyClaim` log record gains optional `agent_id`, `agent_kind`, `evidence` and `valid_until`; consolidation's derived reverify records stay byte-identical. Before the append aver checks that the claim is ACTIVE, the request scope equals the claim's scope, at least one evidence reference is given, and every `event:<id>`/`claim:<id>` reference exists in the claim's scope or one of its descendant scopes (re-checked at the end of replay, like `event_id`). Descendants are the scopes below it by path (`a` covers `a/b`), as in `scope_walk=descendants`. `valid_until` only extends: a lower value, a `valid_until` on a claim without a context, and a claim citing itself as evidence are refused.

Each explicit record becomes a row in `claim_verifications(claim_id, verified_at, agent_id, agent_kind, evidence)`. Consolidation's reverify records only move `last_verified_at`, as before, and get no row: an upgraded store could not backfill them from its projection, so storing them would make a replay differ from the projection. `last_verified_at` keeps its meaning. A derived `last_confirmed_at` is the latest explicit verification, else the claim's `created_at`: a re-assertion folded by consolidation is not a confirmation.

### Evidence-driven decay, derived at read time

For a claim L with a context, let the scope set be L's scope and its descendant scopes. `evidence_since_confirmation(L)` counts distinct items E such that each of the following ACTIVE claims exists within the scope set: `E in_context L.context`, `E concerns S` for some S in the claim's topics, and `E closed_at …` was created after the baseline `max(last_confirmed_at(L), valid_from(L))`. The counted interval is (baseline, now]. `valid_until` does not end it: an ordinary claim's window is its past evidence, so every new matching item closes after `valid_until`, and counting continues after it. A reverify can extend `valid_until` to record that the claim now covers newer evidence. The topics are `L.subject` plus the objects of L.subject's own ACTIVE `concerns` claims (one hop), so a lesson that is its own subject (`lesson:x concerns topic:y`) counts the items that concern `topic:y`. Those one-hop `concerns` claims must also be ACTIVE and within the scope set. `concerns` matches with its descendants and aliases and is multi-valued, so an item can concern several topics. It is plain SQL over the projection: no writes, deterministic, identical after replay. Claims without a context report nothing, so existing behaviour is unchanged. Arrival is the logged `created_at` of the `closed_at` claim, so a one-time import of older closed items counts all of them for any claim confirmed before the import; run such an import before writing the claims it should inform.

aver only counts. Whether new evidence supports or refutes a claim is the consumer's judgment; it answers with `reverify_claim` or `contradict`. Contradictions keep decaying stored confidence through consolidation as before.

`reverify_claim` is not in the default active tool set; the instructions point to it for review steps. Recall emits the new fields only when they are set, so outputs for claims without a context are unchanged. Recall gains additive output fields: `context`, `valid_from`, `valid_until`, `last_verified_at`, `last_confirmed_at`, `evidence_since_confirmation`, and the latest explicit confirmation (`at`, `agent_id`, `agent_kind`, `evidence`). Recall gains a `context` filter. Ranking by the count is opt-in: `AVER_RECALL_EVIDENCE_HALF_LIFE=n` multiplies a contextual claim's score by `0.5^(count/n)`. Stored `confidence` is never rewritten by this, and `min_confidence` keeps filtering on it. `decay_inferred_confidence_at` is deprecated; nothing uses it.

### Replay proposes, live confirms

No special case is needed. Hypotheses from a replay live in their own top-level scope (e.g. `replay/<run>`) with their own evidence. Counting and reverify reach only a claim's own scope and its descendants, and a replay scope is never below a live scope, so replay evidence neither confirms nor decays a live claim, and a reverify cannot cite it. aver does not stop a writer from passing a live `scope` parameter (the per-call scope overrides the connection's, ADR-0022), so keeping replay writers out of live scopes is the importer's discipline: its own connection scope and no live scope parameter. Enforcing that would be a separate decision. A deployment also keeps replay scopes out from under live ones. Confirming in live means writing or promoting the claim into the live scope with its context and window and reverifying it with live evidence. Candidate promotion keeps its scope override (ADR-0021/0029), so a replay candidate can be promoted into live; that promotion is the live decision itself and is recorded with its writer. A pre-declared out-of-sample test is a live claim whose window starts after its `created_at`: only items that close after both its creation and the window start are counted.

## Consequences

- (+) A consumer can state where and when a lesson was established, see how much unexamined matching evidence has piled up, and record who confirmed it with which evidence.
- (+) Domain-neutral: contexts are opaque ids; a consumer's context vocabulary goes into its `ontology.json`.
- (+) No stored value changes over time on its own; everything new is logged or derived, so replay stays exact.
- (−) Claim writes, views and MCP parameters widen; Rust struct-literal callers must add fields.
- (−) Two more core predicates. A store that declared `in_context` or `closed_at` itself must reconcile before upgrading.
- (−) Migration 0013 makes an older binary refuse the upgraded store (schema too new). Rollback is therefore the old binary together with the store backup taken before the upgrade, never the binary alone; and an older binary replaying a newer log would silently drop context and evidence.
