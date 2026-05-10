# 18. Ontology enforcement on claim writes

Date: 2026-05-10

## Status

Accepted

Extends ADR-0010 (type hierarchy and ontology discipline). Does not supersede it: ADR-0010 commits to *having* a typed ontology with closure tables; this ADR commits to *binding claim rows to it* on write. Companion to ADR-0009 (privacy filter), which already runs a pre-write rejection cascade in `insert_claim` — the same insertion path is the natural home for ontology checks.

## Context

ADR-0010 established two type hierarchies (`entity_types`, `predicate_types`) with materialized closure tables, seeded from `crates/aver-core/src/seed.rs`. The hierarchies are queryable: retrieval (ADR-0004) already calls `expand_predicate_filter` to walk `predicate_closure`, so abstract predicates like `depends_on` correctly match leaf rows like `calls`.

The gap: **claim writes are never validated against the ontology**. Two concrete leaks today:

1. `ensure_entity` (`crates/aver-core/src/lib.rs:342-382`) calls `infer_entity_type_name`, which returns `Thing` for anything that doesn't match the `prefix:` shape or the hard-coded `User`/`Claude`/`Pi` synonyms. Unknown entities are silently coerced. There is no signal — no log, no metric, no `requires_review` flag.
2. `insert_claim` (`crates/aver-core/src/lib.rs:476-553`) validates the string shape of subject/predicate/object, runs the privacy filter, range-checks confidence — and then writes the row. **It never consults `predicate_types`.** Any string that survives `validate_claim_field` becomes a predicate.

The schema reinforces the gap: `claims.subject`, `claims.predicate`, `claims.object` are TEXT with no FK to the type tables (`migrations/0001_init.sql`, `migrations/0003_ontology.sql`).

The book is unambiguous on which side of this we should be on:

> If the LLM hallucinates an edge that violates the schema, the symbolic layer rejects it. [ch.136]

That rejection does not happen in Aver. It is a load-bearing line in the book's design — the ontology exists to constrain the LLM, not just to label what the LLM produces.

### Evidence: production data already drifted

`select predicate, count(*) from claims group by predicate` against `.aver/db.sqlite`:

| Predicate         | Count | In `PREDICATE_ONTOLOGY`? |
|-------------------|------:|--------------------------|
| `has-module`      |     5 | no                       |
| `binary-name`     |     1 | no                       |
| `impl-stays-in`   |     1 | no                       |
| `splits-by`       |     1 | no                       |
| `uses`            |     1 | no                       |

Nine rows, five distinct predicates, **zero overlap** with the 13 seeded predicates. The shape is also off: ontology uses `snake_case`, real data uses `kebab-case`. This is not a hypothetical — the silent fallback has already swallowed the entire production claim set.

These rows are USER_ASSERTED (hand-written via `aver remember`), so the privacy filter wasn't the issue; the rows are real, intentional, and useful. They're just not bound to the ontology.

## Decision

Adopt **Option E (hybrid)**: a Rust-layer validator in `insert_claim` plus a SQLite `BEFORE INSERT` trigger on `claims`, with a one-shot migration that extends `PREDICATE_ONTOLOGY` to cover the in-production predicates. Different policy for USER_ASSERTED versus extracted/inferred claims. Subject/object policy is softened (warn-and-flag) rather than hard-rejected.

### Options considered

| Option | Where enforced | Cost | Failure mode for unknown predicate | Caught by ADR-0007 extraction? |
|---|---|---|---|---|
| A. Status quo: silent fallback to `Thing` for entities; no predicate check | nowhere | 0 | swallowed, persisted | no |
| B. Runtime validator in `insert_claim` (Rust) | app layer | small | `Error::UnknownPredicate` returned | yes |
| C. `BEFORE INSERT` trigger checking predicate is in `predicate_types` | DB layer | small (one trigger like ADR-0009 range checks) | `RAISE(ABORT, 'unknown predicate')` | yes, surfaces as `Error::Sqlite` |
| D. FK column `predicate_type_id INTEGER NOT NULL REFERENCES predicate_types(id)` on claims | DB layer (strongest) | migration: add column, backfill 9 rows, change `insert_claim` signature to look up id, breaks JSONL log replay (`LogEntry` has predicate name, not id) | NOT NULL FK violation | yes |
| **E. Hybrid: Rust validator + trigger, defense-in-depth** | both | small | `Error::UnknownPredicate` (clean), trigger catches direct-SQL writes | yes |

### Why E and not D

Option D is the textbook "tightest enforcement" answer. Three reasons it's wrong here:

- **JSONL log is the source of truth (ADR-0005).** The log records the predicate as a string. Replay must rebuild the DB from the log without id drift (the existing comment on `claim_id` allocation in `insert_claim` calls this out). An FK-by-id couples log replay to the seeded predicate id sequence; a fresh database where the seed inserts predicates in a different order would assign different ids and break replay. Keeping the predicate as a TEXT column with a *value* check (`name IN predicate_types.name`) is replay-safe.
- **Migration cost is non-trivial.** The 9 production rows need predicate ids; that means we either pre-seed the new predicates in the migration in a stable order, or we hand-roll the backfill. Option E gets us 95% of the protection with zero schema change.
- **Symmetry with ADR-0009.** The privacy filter is implemented as a pre-write rejection in `insert_claim` and a metrics counter, not as a column constraint. Ontology enforcement is the same shape of problem (reject-on-write with telemetry). Same shape, same place.

Option C alone is insufficient because the Rust caller would see a generic SQLite error; we want a typed `Error::UnknownPredicate { name }` that the CLI can render usefully ("did you mean `depends_on`?"). Option B alone is insufficient because anyone with a SQLite shell can bypass it — see ADR-0009's triggers as the precedent for defense-in-depth at the DB.

### Subject/object policy

Predicates and entity-typed subjects/objects have different failure modes. The set of valid predicates is small and closed (~15 leaves); the set of valid entity *names* is open (every file path, every function, every commit hash). We cannot reject on unknown name — we can only reject on unknown *type*.

Today `infer_entity_type_name` returns `Thing` for the open case. The book's argument applies less cleanly here: an unknown entity is not a hallucinated edge, it's a not-yet-classified noun. Three sub-options:

| Sub-option | Behaviour for unknown entity |
|---|---|
| S1. Continue silent fallback to `Thing` | as today |
| S2. Hard-reject like predicates | rejects every novel file path; unworkable |
| S3. **Fallback to `Thing` *plus* set `requires_review = 1` on the entities row** | preserves write path, surfaces drift |

Pick S3. Add a `requires_review INTEGER NOT NULL DEFAULT 0` column to `entities`. `infer_entity_type_name` already has the signal: when the result is `Thing` *and* the entity name has no `prefix:` and no synonym match, set the flag. Consolidation (ADR-0005) can later promote flagged entities to a real type or accept `Thing` as final. This gives us telemetry without breaking writes.

### USER_ASSERTED versus extracted

USER_ASSERTED claims (via `aver remember`, agent_kind=Human) are deliberate. ADR-0003 places them at the top of the trust table. EXTRACTED claims come from the Tree-sitter or LLM extractor (ADR-0007); INFERRED claims come from the reasoner.

Apply enforcement asymmetrically:

| Provenance       | On unknown predicate                                                    |
|------------------|-------------------------------------------------------------------------|
| `USER_ASSERTED`  | Warn (stderr + metric) but **accept**, and auto-add the predicate to `predicate_types` with `parent_id = relates_to`. The user is the ontology authority. |
| `EXTRACTED`      | Reject with `Error::UnknownPredicate`. The extractor must classify against the seeded vocabulary or a configured alias. |
| `INFERRED`       | Reject. Inferred edges that violate the schema are exactly the case the book describes. |
| `AMBIGUOUS`      | Reject. Ambiguity is not a license to expand the vocabulary. |

The auto-add behaviour for USER_ASSERTED is the deliberate friction point: the user *can* extend the ontology, but every extension is recorded (a row in `predicate_types` with a `created_via='user_assertion'` flag), so consolidation can review additions and re-parent them under a more specific predicate later. This is the same pattern as ADR-0009's "user can edit detector config — a deliberate friction point."

This asymmetry is consistent with ADR-0003: USER_ASSERTED bypasses extractor classification; here it bypasses ontology classification too, with the same audit trail.

### Migration: extend `PREDICATE_ONTOLOGY`

The 9 production rows need to be reconciled before enforcement turns on, otherwise a future replay of `log.jsonl` would fail the trigger. Three options:

| Migration option | Result on existing data |
|---|---|
| M1. Reject retroactively, refuse to start until rows are deleted | breaks running deployments |
| M2. Grandfather by leaving them in place but failing future writes with same predicate | inconsistent: same predicate accepted historically, rejected now |
| M3. **Extend the seed**: add `has-module`, `binary-name`, `impl-stays-in`, `splits-by`, `uses` to `PREDICATE_ONTOLOGY` with sensible parents, then enforcement passes for old and new rows | clean |

Pick M3. Suggested parentage (committed in the same PR as the validator, reviewed as ontology-PR per ADR-0010):

| New predicate     | Parent          | Rationale                                                 |
|-------------------|-----------------|-----------------------------------------------------------|
| `has_module`      | `owns`          | "X has module Y" is a containment/ownership relation      |
| `binary_name`     | `owns`          | Binding a producer to its named output                    |
| `impl_stays_in`   | `concerns`      | A constraint on where an implementation is permitted      |
| `splits_by`       | `relates_to`    | Decomposition; no tighter parent without bikeshedding     |
| `uses`            | `depends_on`    | Direct synonym of `depends_on` at a coarser grain         |

Note the rename to `snake_case` to match the existing convention. The migration also adds a `predicate_alias(alias TEXT PRIMARY KEY, predicate_id INTEGER REFERENCES predicate_types(id))` table and pre-seeds the kebab-case forms (`has-module → has_module`, etc.) so the existing JSONL log replays without rewrites. The validator consults `predicate_alias` before rejecting.

### Validator location

```
fn insert_claim(&self, write: ClaimWrite<'_>) -> Result<i64, Error> {
    validate_claim_field("subject", write.subject)?;
    validate_claim_field("predicate", write.predicate)?;
    validate_claim_field("object", write.object)?;
    validate_claim_field("source", write.source)?;
    // ... existing range and agent_id checks ...
    self.privacy_filter_call(...)?;            // ADR-0009
    self.ontology_check(write.predicate,        // NEW: ADR-0018
                       write.provenance)?;
    // ... existing write path ...
}
```

`ontology_check` resolves the predicate name against `predicate_types.name`, then `predicate_alias.alias`. On miss:

- USER_ASSERTED: insert into `predicate_types(name, parent_id, created_via)`, rebuild `predicate_closure`, emit `aver.ontology.user_extension{predicate=...}` metric, accept.
- everything else: increment `aver.ontology.rejected{predicate=...}` (predicate name is *not* a secret; unlike privacy rejections, logging it is fine and useful) and return `Error::UnknownPredicate { name: predicate.into() }`.

### Worked example: rejection path

For a concrete walk-through, consider an LLM-driven extractor that emits

```
add_claim_from_agent("claude", AgentKind::Bot,
    subject="auth_service", predicate="orchestrates",
    object="session_token", source="claude:msg:abc123")
```

`orchestrates` is not in `predicate_types` and not in `predicate_alias`. With this ADR:

1. `validate_claim_field` passes (well-formed string).
2. Privacy filter passes (no token-shaped content).
3. `ontology_check` resolves `provenance = INFERRED` (Bot → INFERRED per `provenance_for_agent_kind`).
4. Lookup `predicate_types.name = 'orchestrates'`: miss.
5. Lookup `predicate_alias.alias = 'orchestrates'`: miss.
6. Increment `aver.ontology.rejected{predicate="orchestrates",provenance="INFERRED",agent_kind="Bot"}`.
7. Return `Error::UnknownPredicate { name: "orchestrates" }`.
8. JSONL log is **not** written. The privacy-filter precedent applies here too: rejections never enter the log, because the log is the source of truth (ADR-0005) and a rejected write should leave no replay trace.

Compare with the same call from `aver remember` (USER_ASSERTED):

5'. Miss in both tables.
6'. Insert `predicate_types(name='orchestrates', parent_id=relates_to_id, created_via='user_assertion', created_at=now)`.
7'. Rebuild `predicate_closure` (incremental — only new rows for the new id).
8'. Emit `aver.ontology.user_extension{predicate="orchestrates"}`.
9'. Continue normal write path.

The user has now expanded the vocabulary; future EXTRACTED claims with `orchestrates` will pass.

### Interaction with ADR-0004 (HybridRAG retrieval)

Retrieval already calls `expand_predicate_filter` to walk `predicate_closure` — when a query specifies `depends_on`, claims with `calls`, `imports`, or `reads_config_from` match. This ADR does not change that path. It strengthens it: every active row in `claims` is now guaranteed to have a `predicate` value present in `predicate_types.name` (or in `predicate_alias.alias`, which resolves to a `predicate_types` row). The closure walk therefore returns *complete* coverage — no orphan predicates that exist in claims but not in the type table.

Concretely, today a query for `depends_on` against the production database returns nothing, even though one would intuitively expect `uses` to be a sub-predicate of `depends_on`. After the M3 migration extends the ontology to seat `uses` under `depends_on`, the same query returns the production row. **The retrieval improvement is a side-effect of the migration, not the enforcement.** Worth calling out because it makes the migration valuable independent of the enforcement story.

### Interaction with ADR-0014 (triggered episodic-to-claim)

Triggered claim materialisation runs the same `insert_claim` path with `provenance = INFERRED`. Today it can produce predicates that the trigger logic chose; with this ADR those predicates must be drawn from the seeded vocabulary or the trigger registration must explicitly extend `PREDICATE_ONTOLOGY` first (an ontology-PR per ADR-0010). This is the desired discipline: a trigger that wants to emit `observed_during` is forced to commit that predicate to the schema before it can fire.

### Alias-table grammar

```sql
CREATE TABLE IF NOT EXISTS predicate_alias (
    alias        TEXT PRIMARY KEY,
    predicate_id INTEGER NOT NULL REFERENCES predicate_types(id),
    created_at   INTEGER NOT NULL,
    note         TEXT
);
```

Initial rows from M3:

| alias            | resolves_to (predicate_types.name) |
|------------------|-------------------------------------|
| `has-module`     | `has_module`                        |
| `binary-name`    | `binary_name`                       |
| `impl-stays-in`  | `impl_stays_in`                     |
| `splits-by`      | `splits_by`                         |
| `uses`           | `uses`                              |

The alias for `uses → uses` looks redundant but documents intent: the historical row used the same string, and the alias row records "this name is intentionally the canonical one too" rather than relying on an absence-of-alias to mean canonical. (Alternative: don't insert self-aliases; rely on `predicate_types` lookup first. Either is defensible; the explicit form is slightly more traceable.)

Aliases are *not* CamelCase-tolerant by themselves — `HasModule` would not match `has_module` unless an alias row exists. The validator does not normalise case. Rationale: case-folding is a slippery slope (`HASMODULE`, `Has_Module`, `has.module` all become "obvious" candidates), and ADR-0010 prefers a small, explicit hierarchy. If a CamelCase variant appears in production, add an alias row.

### Trigger and migration shape

`migrations/0010_ontology_enforcement.sql` (sketch — final form authored alongside the implementation PR):

```sql
-- ADR-0018: ontology enforcement on claim writes.

ALTER TABLE entities ADD COLUMN requires_review INTEGER NOT NULL DEFAULT 0;

CREATE TABLE IF NOT EXISTS predicate_alias (
    alias        TEXT PRIMARY KEY,
    predicate_id INTEGER NOT NULL REFERENCES predicate_types(id) ON DELETE RESTRICT,
    created_at   INTEGER NOT NULL,
    note         TEXT
);

ALTER TABLE predicate_types ADD COLUMN created_via TEXT NOT NULL DEFAULT 'seed';
ALTER TABLE predicate_types ADD COLUMN created_at  INTEGER;

-- M3: extend the ontology to cover the production rows.
-- The seed run (seed_ontology) will idempotently re-add these on subsequent
-- starts; the explicit INSERT here ensures the rows exist in databases that
-- never run the seed.
INSERT OR IGNORE INTO predicate_types (name, parent_id, created_via, created_at)
VALUES
  ('has_module',     (SELECT id FROM predicate_types WHERE name = 'owns'),       'migration', strftime('%s','now')),
  ('binary_name',    (SELECT id FROM predicate_types WHERE name = 'owns'),       'migration', strftime('%s','now')),
  ('impl_stays_in',  (SELECT id FROM predicate_types WHERE name = 'concerns'),   'migration', strftime('%s','now')),
  ('splits_by',      (SELECT id FROM predicate_types WHERE name = 'relates_to'), 'migration', strftime('%s','now'));
-- 'uses' already exists as a re-parent target; ensure parent is 'depends_on'.
UPDATE predicate_types
   SET parent_id = (SELECT id FROM predicate_types WHERE name = 'depends_on')
 WHERE name = 'uses' AND parent_id IS NULL;
INSERT OR IGNORE INTO predicate_types (name, parent_id, created_via, created_at)
VALUES ('uses', (SELECT id FROM predicate_types WHERE name = 'depends_on'), 'migration', strftime('%s','now'));

-- Aliases for the historical kebab-case forms in log.jsonl.
INSERT OR IGNORE INTO predicate_alias (alias, predicate_id, created_at, note)
SELECT 'has-module', id, strftime('%s','now'), 'M3 alias for log replay'
  FROM predicate_types WHERE name = 'has_module';
INSERT OR IGNORE INTO predicate_alias (alias, predicate_id, created_at, note)
SELECT 'binary-name', id, strftime('%s','now'), 'M3 alias for log replay'
  FROM predicate_types WHERE name = 'binary_name';
INSERT OR IGNORE INTO predicate_alias (alias, predicate_id, created_at, note)
SELECT 'impl-stays-in', id, strftime('%s','now'), 'M3 alias for log replay'
  FROM predicate_types WHERE name = 'impl_stays_in';
INSERT OR IGNORE INTO predicate_alias (alias, predicate_id, created_at, note)
SELECT 'splits-by', id, strftime('%s','now'), 'M3 alias for log replay'
  FROM predicate_types WHERE name = 'splits_by';

CREATE TRIGGER IF NOT EXISTS claims_predicate_in_ontology_insert
BEFORE INSERT ON claims
WHEN NOT EXISTS (SELECT 1 FROM predicate_types WHERE name = NEW.predicate)
 AND NOT EXISTS (SELECT 1 FROM predicate_alias WHERE alias = NEW.predicate)
BEGIN
  SELECT RAISE(ABORT, 'claims.predicate not in ontology');
END;

CREATE TRIGGER IF NOT EXISTS claims_predicate_in_ontology_update
BEFORE UPDATE OF predicate ON claims
WHEN NOT EXISTS (SELECT 1 FROM predicate_types WHERE name = NEW.predicate)
 AND NOT EXISTS (SELECT 1 FROM predicate_alias WHERE alias = NEW.predicate)
BEGIN
  SELECT RAISE(ABORT, 'claims.predicate not in ontology (update)');
END;
```

The two triggers (insert + update) match the pattern in `migrations/0009_value_range_checks.sql` for confidence and status — every constraint guarded on the way in is also guarded on update. ADR-0005 makes claim updates rare (append-only with `SUPERSEDED` markers), but `last_seen_at` and `last_verified_at` updates are common; those don't touch `predicate`, so the `BEFORE UPDATE OF predicate` filter scopes the trigger to the relevant case.

The Rust validator has the policy (USER_ASSERTED auto-adds; others reject); the trigger is a hard backstop for any code path that bypasses `insert_claim` — direct SQL, future migrations, third-party tooling. Same shape as ADR-0009's pattern of regex+path+entropy detectors layered together.

### Performance

A single index probe per insert. `predicate_types.name` is already `UNIQUE` (the table definition in `0003_ontology.sql` declares `name TEXT NOT NULL UNIQUE`), so the lookup is an index seek. `predicate_alias.alias` is the PK. Two seeks per write; negligible against the existing JSONL append + privacy filter pass.

The closure table is *not* consulted on write — only `predicate_types.name`. Closure walking happens on read (`expand_predicate_filter`), which is unchanged.

The trigger adds a third probe (its `WHEN NOT EXISTS` clause runs both subqueries on every insert), but only when the Rust validator has *not* already short-circuited — which is the path-it-was-built-for invariant. In normal operation the trigger fires on already-validated data and short-circuits on the first `EXISTS`. Worst case (direct SQL writes that bypass the validator), the trigger does the full work the validator would have done; that's defense-in-depth, not a regression.

### Telemetry

Telemetry is scoped to ontology events; it does not duplicate the privacy-rejection counters from ADR-0009. A claim that is rejected by both filters increments only the privacy counter (the privacy filter runs first in `insert_claim` and short-circuits — see `crates/aver-core/src/lib.rs:487-501`). This ordering is intentional: secrets must be quarantined before any further processing, including ontology checks that might log the predicate name.

Three new metrics:

| Metric | Increment when |
|--------|----------------|
| `aver.ontology.rejected{predicate, provenance, agent_kind}` | EXTRACTED/INFERRED/AMBIGUOUS write hits unknown predicate |
| `aver.ontology.user_extension{predicate}` | USER_ASSERTED write auto-adds a predicate |
| `aver.ontology.alias_hit{alias, resolved_to}` | A write resolves through `predicate_alias` rather than direct `predicate_types` |

Unlike privacy rejections (ADR-0009), the predicate name is safe to log — predicates are part of the project's design vocabulary, not user content. The rejection metric carries the predicate name as a label so dashboards can spot "extractor keeps emitting `orchestrates`, should we add it" without a separate query against rejected-write logs.

The `alias_hit` metric is a deprecation lever: aliases that fire often are candidates to be promoted (the canonical form gets renamed to the alias) or retired (the codebase migrates to the canonical name). A zero-volume alias is a candidate for deletion.

### Test plan

Three integration tests, one per failure axis, in `crates/aver-core/tests/ontology_enforcement.rs`:

1. `unknown_predicate_rejected_for_extracted` — `add_claim_from_agent` with a Bot agent and an unseen predicate returns `Error::UnknownPredicate` and writes nothing to either the claims table or the JSONL log.
2. `unknown_predicate_auto_added_for_user_asserted` — `add_claim` (USER_ASSERTED) with an unseen predicate succeeds, the predicate appears in `predicate_types` with `created_via='user_assertion'`, and `predicate_closure` contains a `(new_id, relates_to_id)` row.
3. `alias_resolution_accepts_kebab_case` — a write with `predicate="has-module"` succeeds and the row stores `predicate="has-module"` (i.e., the alias is *not* rewritten on write — log fidelity), but `expand_predicate_filter("owns")` returns the row by walking through the resolved canonical predicate.

A fourth test for the trigger backstop: open a raw `rusqlite::Connection`, `INSERT INTO claims` directly with an unseen predicate, expect `RAISE(ABORT)`. Ensures the trigger fires when the Rust layer is bypassed.

### Rollout

Three phases, each independently revertable:

| Phase | Change | Risk |
|-------|--------|------|
| 18a | Migration M3 + alias seeds + `requires_review` column on `entities` | low — additive, no enforcement yet |
| 18b | Rust validator (`ontology_check`) with USER_ASSERTED auto-add and EXTRACTED/INFERRED rejection | medium — extractor (ADR-0007) may emit rejected predicates |
| 18c | DB trigger `claims_predicate_in_ontology_insert` | low — trigger checks the same condition the validator already rejects on |

Phase 18b is the highest-risk because it changes observable behaviour for the extractor. Recommend running 18a + a *warn-only* validator (count rejections in metrics but accept the write) for one consolidation cycle, then promoting to hard rejection once the rejection rate stabilises near zero. The book's principle is "reject hallucinated edges"; the engineering tactic is to first measure, then enforce.

### Edge cases and questions

A handful of cases that came up during this design that deserve explicit answers rather than buried mentions.

**Q: What happens to a USER_ASSERTED write that re-uses a predicate the user already auto-added?**
A: Normal write path. `predicate_types.name` lookup hits, no auto-add, no `user_extension` metric. The flag `created_via='user_assertion'` is set on the predicate row (one-time, on creation) — it is not a per-claim attribute.

**Q: Can an attacker poison the ontology by submitting many USER_ASSERTED claims with absurd predicates?**
A: Only if the attacker has CLI access — and at that point the threat model is broader than ontology integrity. The auto-add happens only via `aver remember` (or the equivalent agent surface where `agent_kind=Human`). Bots cannot trigger the auto-add path. The audit trail (`created_via`, `created_at`) makes cleanup straightforward.

**Q: The migration adds five new predicates. What if a future `seed_ontology` run re-seeds the original 13 in a different order than the migration's extension?**
A: `seed_type_table` uses `INSERT OR IGNORE INTO ... (name)` then a follow-up `UPDATE ... SET parent_id`. Existing rows are preserved with their original ids. The `predicate_alias` table references rows by id; once the migration runs, those id bindings are stable. The risk lives only in environments where the database is dropped and recreated from scratch — at which point `log.jsonl` replay rebuilds claims, and the same migration runs again, so id-by-name binding is consistent.

**Q: What about `predicate_alias` rows whose `predicate_id` points at a `predicate_types` row that gets deleted?**
A: `predicate_types` rows are append-only in current code (`seed_type_table` only inserts and updates `parent_id`). If a future ADR introduces deletion, the FK on `predicate_alias.predicate_id` should be `ON DELETE RESTRICT` so an alias must be cleared before the predicate it points at can be removed. Mentioned here so a future implementer doesn't paper over the dependency.

**Q: Why not validate the *object* against an expected type for known predicates (e.g., `calls`'s object must be a `Function`)?**
A: Tempting, but premature. Two reasons. First, the leaf set in `PREDICATE_ONTOLOGY` is small and not yet stable enough to commit per-predicate range types. Second, doing so requires knowing the type of the object *at write time*, but `ensure_entity` runs after the validator and may set `requires_review`. We'd be enforcing type constraints against guessed types. Defer to a follow-up ADR once the entity-type inference is stronger (ADR-0007's extractor maturing, or a typed-write API where the caller asserts the object's type).

**Q: Does this break the existing test suite?**
A: The 9 production rows are reconciled by M3, so existing replay tests pass. The extractor unit tests — anywhere they emit a literal predicate string — must now emit predicates from the seeded vocabulary or the test must extend the ontology in setup. Expect to touch `crates/aver-extractor/tests/*` to migrate any string-literal predicates to the canonical names.

### Scope of this ADR

- In scope: `claims.predicate` validation; `entities.requires_review` flag for unknown subject/object types; one-shot ontology extension migration; alias table.
- Out of scope: predicate hierarchy *editing* by agents (still ADR-required per ADR-0010); cross-claim consistency checks (e.g., transitive contradiction detection, ADR-0003's `contradicts` edge); typed *object* constraints (e.g., "the object of `calls` must be a `Function`") — that's a follow-up ADR once the leaf set stabilises; UI for browsing / promoting `requires_review` entities (a CLI command `aver ontology review` is the obvious follow-up but its design is not load-bearing for this ADR).

### Alternatives explicitly rejected

**A1. JSON-Schema-on-write for the claim row.** A schema validator would let us encode "predicate must be in this enum" declaratively. Rejected because (a) the enum changes when the user auto-adds, so the schema would have to be regenerated per write, and (b) it adds a dependency for behaviour we can implement in a few lines of Rust + one trigger.

**A2. SHACL or OWL2 reasoner.** The book references ontology design literature; a full RDF stack is the textbook answer. Rejected for ADR-0006 reasons (local-first, SQLite, defer external systems until team scale). The closure table approach in ADR-0010 is already a deliberate "reasoner-light" choice; this ADR is consistent with that.

**A3. Treat unknown predicates as `relates_to` synonyms automatically.** If the user writes `binary-name`, silently file it under `relates_to`. Rejected because it makes the silent-coercion problem worse, not better — at least today's `Thing` fallback is visible in the entity table; an automatic predicate-coercion would lose the original string entirely. The auto-add path for USER_ASSERTED preserves the original name as a real ontology entry.

**A4. Make the closure table maintenance the enforcement point.** A trigger on `predicate_types` could refuse to delete a row that any claim references. Rejected because the failure direction is wrong: we want to reject *writes* that reference missing predicates, not refuse *deletes* of in-use predicates. The latter is a useful integrity check independent of this ADR (the FK from `predicate_alias.predicate_id` already partially provides it).

**A5. Externalise the ontology to a YAML file under `ontology/`.** ADR-0010 hints at a `ontology/` directory; an alternative would be to read predicates from a checked-in YAML file at startup. Rejected for this ADR's scope: the seed table in `crates/aver-core/src/seed.rs` is already the de facto ontology file, and switching to YAML adds a parser dependency without changing the enforcement story. A follow-up ADR can move the source-of-truth from Rust constant to YAML if the editing cadence justifies it.

### Interaction with consolidation (ADR-0005)

The `requires_review` flag on `entities` is the consolidation pass's input queue. Each consolidation run should:

1. Select `entities WHERE requires_review = 1 AND last_seen_at > now - 7d` (recent unclassified entities).
2. For each, propose a type based on patterns in adjacent claims (e.g., if the entity is the object of `calls`, suggest `Function`).
3. Either auto-promote (high confidence) or surface to the user via `aver ontology review`.

The same pass should produce an `aver ontology lint` report listing user-extension predicates with their counts and suggested re-parents. The report is the human-in-the-loop checkpoint that ADR-0010 calls for ("schema changes are committed to `ontology/` in the project repo and reviewed like code").

This ADR does not implement those passes — it provides the queue (`requires_review`) and the audit trail (`created_via`) the consolidation pass will consume.

### Observability checklist

Beyond the three metrics listed above, the following dashboards / queries become possible after this ADR lands and should be wired up at the same time:

- **Rejected-write rate by predicate**: a sustained spike on a specific predicate is a signal to either fix the extractor or extend the ontology.
- **User-extension lifetime**: how long does a `created_via='user_assertion'` predicate sit before consolidation re-parents it. Long tails indicate the consolidation pass is stalled.
- **Alias deprecation candidates**: aliases with zero hits over a window are candidates for removal; aliases with high hits are candidates for promotion.
- **`requires_review` queue depth**: monotonically increasing depth indicates consolidation is not keeping up with novel-entity ingestion.

These are dashboards-not-alerts; the only alertable condition is `aver.ontology.rejected` rate above a threshold, which means the extractor is producing un-typeable claims faster than the user can extend the schema.

## Consequences

- (+) Closes the gap the book calls out at chunk 136: the symbolic layer now rejects edges that violate the schema.
- (+) Defense-in-depth matches ADR-0009's pattern: app-layer rejection for clean errors, DB-layer trigger for paths that bypass the app.
- (+) `entities.requires_review` gives consolidation (ADR-0005) a worklist of unclassified entities — the silent `Thing` fallback becomes a visible queue instead of a black hole.
- (+) USER_ASSERTED auto-extension preserves the user-as-ontology-authority principle from ADR-0010 ("evolve by promoting frequently-co-occurring patterns") without requiring an out-of-band schema PR for every novel predicate.
- (+) Replay-safe: predicate stays a TEXT column, JSONL log remains the source of truth (ADR-0005), no FK-by-id coupling.
- (+) Asymmetric trust by provenance is consistent with ADR-0003's policy table — USER_ASSERTED already gets special treatment elsewhere.
- (−) The one-shot migration to extend `PREDICATE_ONTOLOGY` requires choices about parentage that are debatable (`splits_by` under `relates_to` is a punt). Mitigation: the choices are reviewable in the same PR and re-parentable later via ontology-PR per ADR-0010.
- (−) The `predicate_alias` table is a new surface for vocabulary drift: a sloppy alias entry can mask a real ontology mistake. Mitigation: aliases are seeded by migration only, not by runtime auto-add. Auto-add on USER_ASSERTED creates a real `predicate_types` row, not an alias.
- (−) USER_ASSERTED auto-extension can be abused by a malicious or careless user to silently grow the vocabulary. Mitigation: every auto-add carries `created_via='user_assertion'`; a periodic report (or a `aver ontology lint` command) lists user-extensions that no consolidation pass has re-parented.
- (−) The trigger and the validator now both encode "predicate must be in ontology." Drift between them is a real risk (the validator could change its alias-lookup logic without updating the trigger). Mitigation: both consult the same two tables; a single integration test per provenance class covers both paths.
- (−) Rejection on EXTRACTED predicates raises the bar for the extractor (ADR-0007). The extractor must either classify against the seeded vocabulary or fail loudly. This is the intended behaviour, but it shifts the failure mode from "silently wrong claim" to "missing claim" — which is what the book argues for, but it's still a change in observable system behaviour.
- (−) The `requires_review` flag is added to `entities` only, not to `claims`. A claim with two unknown-typed entities is not itself flagged. This is deliberate (the entity is the unit of typing), but it means dashboards have to join `claims → entities` to surface "claims involving unreviewed entities." Acceptable cost.
