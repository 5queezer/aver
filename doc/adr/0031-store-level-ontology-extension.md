# 31. Store-level ontology extension

Date: 2026-11-15

## Status

Accepted

## Context

ADR-0018 enforces a strict predicate vocabulary: extracted and inferred claims must use a predicate that exists in `predicate_types`. ADR-0029 then seeded a few domain-specific predicates (a direction, an outcome and a result value of a decision) into the core seed, because one consumer needed them under the strict check.

Aver is general-purpose memory. A core seed that grows with every consumer's domain stops being general, and every other store inherits words it never uses. The strict check is still wanted: a consumer with its own vocabulary should declare it, with parents and cardinality, rather than bypass enforcement.

## Decision

A store may carry an optional `ontology.json` next to its database. Aver loads it after the core seed (and its cardinality marks and aliases) and before the closure tables are rebuilt, inside the store open's write transaction, on every store open and before a replay applies records.

```json
{
  "version": 1,
  "predicates": [{"name": "has_rating", "parent": "concerns", "cardinality": "single"}],
  "entity_types": [{"name": "Recipe", "parent": "Concept"}],
  "aliases": [{"alias": "rated", "predicate": "has_rating"}]
}
```

- **Additive and idempotent.** Entries are inserted the way the core seed is (insert if missing, then set parent and cardinality). Loading the same file twice changes nothing.
- **Fields.** `version` is required and must be 1. `parent` is optional (none = a root); `cardinality` is optional and defaults to `multi`. The file is authoritative for its own non-core entries, so a predicate that should be single-valued must say `"cardinality": "single"`.
- **No overrides.** Restating a core entry identically is a no-op; giving a core predicate or type a different parent or cardinality is refused. Parents must name a core entry or an earlier entry in the file.
- **Names.** Predicates and aliases use aver's normalized lower snake_case form; entity type names are non-empty without whitespace or control characters. An alias must target a core or declared predicate and may not equal a predicate name or an alias that points elsewhere.
- **Fail closed on a bad file.** An unknown key, an unsupported version, an unknown parent or an override stops the store open with an error of the form `ontology extension <path>: <section>[i] "<name>": <what to fix>`. A missing file means core only.
- **Replay stays non-rejecting.** The log is the source of truth (ADR-0005). Replay without the file still restores every claim: unknown predicates are recreated under `relates_to` as multi-valued, as before. With the file present, the declared parent and cardinality win. So the file belongs in the store's backups. A backup without it still restores, but with flatter predicate shapes.
- **Rows are never deleted.** A binary that no longer seeds a predicate leaves existing `predicate_types` rows alone. A store that was created with older domain predicates keeps accepting those writes before and after it gets its `ontology.json`.

The core seed keeps generic vocabulary only, including the decision-shaped predicates `triggered_by`, `supersedes` and `has_rationale` and the aliases `about` and `is_a`. The three domain predicates from ADR-0029 (`has_direction`, `has_outcome` and `has_result_r`) leave the core seed; a deployment that uses them declares them in its `ontology.json`.

A file in the store directory was chosen over an environment variable or a server flag: it travels with the store, its backups and its replays; it needs no wrapper change; and a CLI and a server opening the same store see the same vocabulary.

## Consequences

- (+) The core stays general-purpose, and consumers still get the strict check for their own words.
- (+) No schema change and no data migration. Existing stores keep working.
- (−) A store's vocabulary now lives in two places, the core seed and its `ontology.json`. Backups must include the file to keep declared shapes on replay.
- (−) A malformed file blocks the store open. That is intended (strict), and the error says what to fix.
