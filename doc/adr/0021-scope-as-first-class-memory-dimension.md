# 21. Scope as a first-class memory dimension

Date: 2026-05-10

## Status

Proposed

## Context

ADR-0006 commits Aver to a single local store. ADR-0019 §"Workload assumptions"
locks the on-disk layout under one `.aver/` directory. In practice the running
MCP server reads `AVER_MEMORY_DIR` at startup
(`crates/aver-server/src/config.rs:25`) and falls back to `.aver` relative to
its CWD. Every release deployment in this user's environment points at
`/home/christian/.aver`, so a single `db.sqlite` is shared by every Claude /
Codex / Pi / Hermes session, in every project, on the host.

This works only as long as memory is genuinely user-scoped. It is not. A live
inspection of `~/.aver/db.sqlite` shows two ACTIVE claims:

| id | subject       | predicate                  | object                         |
|----|---------------|----------------------------|--------------------------------|
| 1  | `vasudev-core`| `is_about`                 | "a dual-mission project…"     |
| 2  | `user`        | `prefers_code_review_model`| `opus 4.7`                     |

These mix two scopes that should never share a namespace: a project-specific
claim about `vasudev-core` and a user-wide preference. With today's read path,
both surface for every recall query in every repo. The cost compounds with
every new session: a code-shaped claim recorded while editing
`agent-memory-layer` will pollute recalls inside `vasudev-core` and vice versa.

The data model has the raw material to express scope and chooses not to. The
`claims` schema (`migrations/0001_*.sql` and successors) carries:

- `agent_id` — but it is `'mcp'` for every MCP-driven write, charset-restricted
  to `[A-Za-z0-9_-]`, and intended to identify the *writer*, not the *subject
  domain*.
- `source_refs` — JSON array of free-text provenance, not a query filter.
- `session_id` (on `events` and `observations`) — free text, MCP tools do not
  filter on it.

`recall` (ADR-0008) and `expand` accept only `query`, `top_k`, `alpha`, `hops`.
Nothing partitions the graph.

Three alternatives were considered:

1. **Separate `db.sqlite` per scope.** Mirrors git's per-repo isolation, gives
   hard guarantees, and makes "wipe a project's memory" a `rm -rf`. But it
   destroys the whole point of cross-cutting claims like `user
   prefers_code_review_model opus 4.7`. Every read would have to fan out across
   stores, and ADR-0005's append-only log invariant has to be redefined per
   store. Rejected: the asymmetry of "user-wide truths AND project-local
   truths" is not solvable by physical partition.

2. **Tag set per claim.** Most flexible. Loses the `predicate_closure` aesthetic
   that ADR-0010 commits to (a tree, not a tag cloud) and offers no default
   resolution rule for "what scope am I in right now" — every read would need
   to enumerate tags. Rejected for inconsistency with the rest of the data
   model.

3. **A first-class `scope` column with a hierarchical path convention.** Single
   column, single migration, default `'global'`. Read path defaults to
   `current_scope ∪ ancestors`. Tracks ADR-0010's tree discipline: scope is to
   namespace what `predicate_types` is to vocabulary. Cross-scope queries
   remain a single SQL filter. Migration cost is bounded.

Option 3 is the decision.

## Decision

Add `scope` as a first-class dimension on every memory row, with hierarchical
path-style values, ancestor-aware reads, and a documented default that keeps
the existing two ACTIVE claims valid.

### Schema change

A new migration `migrations/00NN_scope_column.sql` adds:

```sql
ALTER TABLE claims      ADD COLUMN scope TEXT NOT NULL DEFAULT 'global';
ALTER TABLE events      ADD COLUMN scope TEXT NOT NULL DEFAULT 'global';
ALTER TABLE observations ADD COLUMN scope TEXT NOT NULL DEFAULT 'global';

CREATE INDEX claims_scope       ON claims(scope);
CREATE INDEX events_scope       ON events(scope);
CREATE INDEX observations_scope ON observations(scope);
```

Triggers analogous to `claims_agent_id_*_insert/update`
(`migrations/0060`–`0061`) enforce `scope` non-blank and a charset of
`[A-Za-z0-9_/-]` (note the `/` — paths are first-class).

The candidate-claim staging table inherits a `scope` column on the same
migration so candidates carry the writer's intended scope through promotion.

### Path convention

```
global                                  user-wide truths
proj/<slug>                             repo-local
proj/<slug>/branch/<name>               in-flight on a feature branch
session/<session_id>                    transient, never auto-promoted to global
```

`<slug>` is a stable identifier: `git config remote.origin.url` hashed to
12 hex chars when origin exists, else the basename of the worktree root.
Branch and session paths exist for completeness; this ADR does not require
clients to use them, but specifies the shape so later ADRs do not collide.

### Read-path semantics

`recall` and `expand` gain an optional `scope` parameter and a
`scope_walk` enum:

| `scope_walk`         | Semantics                                                    |
|----------------------|--------------------------------------------------------------|
| `exact`              | only rows whose `scope` equals `scope`                       |
| `ancestors` (default)| `scope` and every prefix of it up to `global`                |
| `descendants`        | `scope` and every path beginning with `scope/`               |
| `any`                | no filter                                                    |

When `scope` is omitted, the server uses `'global'` with `scope_walk=any` —
preserving today's behavior verbatim for clients that have not been updated.
Once Layer 2 (per-connection scope resolution, separately specified) lands,
the omitted-scope default flips to "the connection's scope" with
`scope_walk=ancestors`. That flip is a separate ADR and is not implied by
this one.

### Write-path semantics

`add_triple`, `remember_claim`, `record_event`, `record_observation`,
`propose_candidate_claim`, and `add_vector_chunk` accept an optional `scope`.
When omitted: `'global'` (preserves today's behavior). `promote_candidate_claim`
copies the candidate's `scope` onto the durable claim; it is not a separate
parameter on promote.

### Migration

The two existing ACTIVE claims (ids 1, 2) are correct as `'global'` —
`vasudev-core` is named explicitly in the subject, and the user preference is
genuinely cross-cutting. The `DEFAULT 'global'` therefore needs no data-fix
migration. Future re-scoping (e.g. moving id 1 to `proj/vasudev-core` once
that scope exists) is a normal `UPDATE`, no different from re-typing under
ADR-0010.

### Out of scope for this ADR

- How clients *resolve* their current scope (header, handshake, wrapper
  script). Specified in a separate "Layer 2" ADR.
- Changing `agent_id` semantics or adding per-harness identity. Tracked
  separately.
- Soft-delete / `retire_claim` MCP tool. Tracked separately; today's
  `contradict` is evidentiary and does not retire (verified 2026-05-10:
  contradiction rows 1–3 left claims 3–5 in `ACTIVE` until manual
  `UPDATE … SET status='INVALIDATED'`).
- Cross-scope consolidation rules (when a claim recorded under `proj/X`
  generalizes enough to be promoted to `global`). Tracked separately.
- Multi-user / multi-host scope. ADR-0006 still binds: this ADR adds
  *intra-user* partitioning, not a tenancy model.

## Consequences

- (+) Cross-repo pollution becomes opt-in instead of the default: a
  scope-aware client can ask "claims relevant to *this* project" and
  receive exactly that, plus inherited globals.
- (+) Schema cost is one column, three indexes, one migration. ADR-0019's
  replay invariant (JSONL is the source of truth) absorbs the change because
  every JSONL writer can default to `scope='global'` until clients adopt.
- (+) The decision is reversible: dropping the column reverts behavior. The
  data model carries no other dependency on scope.
- (+) `recall(scope_walk='any')` preserves the current global-search affordance
  for migration tooling, audits, and consolidation passes that legitimately
  need to see everything.
- (+) Consistency with ADR-0010: hierarchical paths use the same tree-with-
  closure intuition as `predicate_types`. A `scope_closure` table is *not*
  needed initially because path prefix-match is cheap on a single TEXT column;
  it remains an option if descendant-walk traffic justifies it.
- (−) Every write now carries a scope choice, even if implicit. Clients that
  ignore it pile claims into `'global'` exactly the way today's code piles
  into the unscoped store. The fix is social, not schema-level: client
  conventions must catch up. Layer 2 is what makes this automatic.
- (−) Indexes on a low-cardinality column have limited selectivity early on
  (most rows will be `'global'` until clients adopt). Acceptable for a v1.
- (−) The path convention is normative without being enforced. A client that
  writes `scope='proj_vasudev-core'` instead of `proj/vasudev-core` will not
  fail any check, but its rows will be invisible to ancestor walks. Mitigation:
  document the convention prominently in the MCP tool descriptions and supply
  a helper in `aver-core` that derives the canonical slug from a path.
- (−) `vacuum` (sqlite VACUUM, per ADR-0019) does not consult scope. Storage
  reclamation is still global. Per-scope wipe is `DELETE FROM claims WHERE
  scope LIKE 'proj/foo/%'` plus a JSONL rewrite — both follow-on work.
- (−) Consolidation (ADR-0005) is now scope-aware in spirit but not in code.
  Until a follow-on ADR specifies promotion rules, consolidation must default
  to *intra-scope only* to avoid leaking branch-local claims into global truth.
