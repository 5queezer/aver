# 27. Extractor-to-core bridge and `aver extract`

Date: 2026-10-24

## Status

Proposed

## Context

`aver-extractor` emits `ExtractedFact { subject, predicate, object }` with
kind-prefixed names (`Struct:Store`, `Function:a::b`) and the bare file path
as the subject of top-level definitions. Core ingests `ExtractorFact` and
types entities by prefix against its seeded ontology (`seed.rs`). Nothing
connected the two (design review 2026-10-17, roadmap Phase 3).

What core accepts today:

- Seeded predicates include `defines`, `calls`, `imports`, `tests`, `uses`.
  `extends`, `implements` and ADR-0007's `has_test` are not seeded, and an
  `EXTRACTED` write with an unknown predicate fails with `UnknownPredicate`.
- Seeded entity types include `File`, `Module`, `Function`, `Class`,
  `Constant`, `Symbol`. Any other prefix (`Struct`, `Trait`, `Variant`) falls
  back to `Thing` and is queued for review.
- `ingest_extractor_facts` writes to scope `global` only and runs
  `consolidate_report()` after every batch.
- `consolidate_report()` supersedes every ACTIVE claim that has a newer claim
  with the same subject and predicate and a different object. It treats every
  predicate as single-valued. On this repository one consolidation after
  `aver extract crates/` retired 15943 of 17932 extracted claims, because
  `File:x defines A` is superseded by `File:x defines B`.

## Decision

1. **Bridge location.** `aver_extractor::bridge` converts `ExtractedFact` into
   `aver_core::ExtractorFact` and walks a tree (`extract_tree`). The extractor
   depends on `aver-core` (no default features); core does not depend on the
   extractor. The CLI and the planned `extract` MCP tool share this one
   conversion.
2. **Entity mapping.** The file-path subject becomes `File:<path>`.
   `Function` stays `Function`. `Class`, `Struct`, `Enum`, `Interface`,
   `Trait`, `Protocol`, `Record`, `Object`, `Actor`, `Type` and `Annotation`
   become `Class`. `Module`, `Namespace` and `Package` become `Module`.
   `Const` becomes `Constant`. Everything else (`Variant`, `TypeAlias`,
   `Delegate`) becomes `Symbol`. Names after the prefix are unchanged, so a
   Rust `Struct:Store` and its `impl` subject `Type:Store` become the same
   entity, `Class:Store`. The source kind is not kept.
3. **Predicate mapping.** `defines`, `calls`, `imports`, `tests`, `uses` pass
   through. Test links keep the extractor's direction and the seeded
   predicate (`Function:test tests Function:f`) rather than ADR-0007's
   `has_test`. `extends` and `implements` are skipped and counted per
   predicate in the command summary. Adding them is a core ontology change.
4. **Write path.** `aver extract` writes each fact with
   `add_claim_from_agent_with_scope(.., AgentKind::DeterministicParser, ..)`,
   which gives `EXTRACTED` at confidence 0.90 in the project scope. All store
   calls sit in one CLI function, `ingest_extracted`. Phase 1 step 4 replaces
   the loop with the batch ingest once that takes a scope. Until then:
   `ExtractorFact::provenance` is not passed (core derives it from the agent
   kind), there is no all-or-nothing batch validation, and a privacy
   rejection skips one fact and is counted.
5. **No consolidation.** `aver extract` does not call `consolidate_report()`.
   Identical triples are written once per run, keeping the first file as the
   source reference. Re-running appends duplicates, because core has no exact
   lookup of an active claim by triple and scope. Consolidation can be added
   back once its conflict rule applies only to single-valued predicates.
6. **Default scope.** `proj/<name>`, where `<name>` is the repository name
   from `remote.origin.url`, else the git toplevel directory name, else the
   scanned directory name. Characters outside `[A-Za-z0-9_-]` become `-`.
   This is the readable literal ADR-0025 recommends, not the ADR-0022 hash.
   `--scope` overrides it.

## Consequences

- (+) Extracted code facts reach memory in the project scope and are returned
  by `aver recall` (roadmap Phase 3 exit).
- (+) Entities are typed. No extracted entity lands in the `Thing` review
  queue.
- (−) Inheritance and interface edges are lost until core seeds `extends` and
  `implements`.
- (−) A Rust trait and a struct both appear as `Class`.
- (−) Source references are file paths without line numbers, because
  `ExtractedFact` carries no position.
- (−) Code facts are not invalidated when a file changes (ADR-0007
  lifecycle). Re-runs add duplicates, and any later consolidation (CLI
  `consolidate`, server, or `ingest_extractor_facts`) collapses multi-valued
  code facts until core fixes the conflict rule.
- (−) Scopes written by the retired shim (`proj/<12 hex>`) and this default
  (`proj/aver`) differ. Pass `--scope` to reuse a hashed scope.

## References

- ADR-0003, ADR-0007, ADR-0010, ADR-0018, ADR-0021, ADR-0022, ADR-0025
- Design review 2026-10-17, roadmap Phase 3
