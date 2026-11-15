# 29. Keep memory objects typed and judgments attributable

Date: 2026-11-15

## Status

Accepted design for the structured-memory goal; implementation and live rollout require independent verification.

Edited 2026-11-15 to remove deployment details. The three domain predicates moved out of the core seed by ADR-0031.

## Context

A consumer inspection found paragraph objects materialized as entities, while agent decisions looked like sourced facts and lacked event links. Existing APIs accept string objects. Claims must remain readable, privacy validation must precede any durable write, and JSONL must precede every replayable projection change. No additional service or dependency is warranted.

The conceptual reference is Safi Shamsi, *The Memory Layer*, consulted through a local search of the book. Chunk 90 distinguishes explicit source evidence, inference and conflicting signals. Chunks 112–113 describe probabilistic inferred links and a scorer ceiling of 0.75. This ADR adopts those ideas; it does not copy the book's implementation. Aver retains ADR-0003's more conservative default inference confidence of 0.45 and existing defaults for other provenance classes.

The release builds on the latest published tree, which already carries the stdio, OpenAI embedding, embed-on-write and optional recall-recency work. Release review covers the entire range from the published head to the candidate, so those capabilities are preserved, and publication rechecks the current remote. Publication is a normal fast-forward; no force push.

## Decision

1. Preserve string subjects/objects and existing reads, while recording whether a claim object is an entity or literal. Deterministic classification treats prose and primitive values as literals; short names and identifiers retain graph identity. Writers may request literals explicitly; an explicit entity request cannot turn recognized prose into an entity. The classifier treats JSON scalar values as literals. Prose detection uses more than 160 Unicode characters, control characters, more than eight whitespace-separated words, terminal sentence punctuation, dot-space, or sentence punctuation within whitespace-bearing text, including Unicode full stops/question/exclamation marks. Short multiword names and version labels such as `Rust 1.95` remain entities unless a literal is requested. Short single-token paths/URIs and dot/dotdot identifiers are exempt from the punctuation rule (so a URI query ending in `?` or a filename ending in `.` retains identity); length, control-character and whitespace rules still apply, and JSON scalars remain literals. Invented fixtures exercise these boundaries. A heuristic cannot understand every natural-language fragment; explicit literals remain the escape hatch. MCP/JSON additions are optional, so old request shapes remain valid. Rust callers constructing public `Claim`, `ClaimWrite`, server `ClaimView`, `RememberClaimParams` or `AddTripleParams` with struct literals must account for the added metadata fields. Prefer existing core write constructors/builders, claim-view conversion, and optional JSON request deserialization where provided; JSON/MCP callers need no extra fields.
2. Old records without object typing preserve their historical meaning when replayed. A deliberate `migrate-literals` operation appends bounded, explicit reclassification records before applying projection changes. All projection-derived names selected for the cleanup must pass current privacy validation before the first append; projection-only rows are not assumed to be trusted log history. It does not edit earlier records, parse prose back into triples, rewrite provenance, retire claims or delete their text. Remove a former object entity only when no remaining entity-valued claim, subject or hyperedge references it. Replay and repeated migration must converge. Opening a store must not silently run the real-data cleanup. Entity application keeps `last_seen_at` monotonic while preserving original record timestamps and `created_at`: concurrent writers can append a previously captured older timestamp after a newer record, and replay must handle that same order.
3. Accept prose objects with a clear atomic-triple advisory by default. Offer strict rejection through an explicit setting/parameter, following the existing actionable-error style. Rejection occurs before log mutation. Prose detection decodes JSON string literals, so quoting a sentence does not suppress advisory/strict behavior; short quoted values remain valid literals. Prose belongs in an episodic event; stable IDs and small facts belong in claims.
4. Default ordinary MCP writes to LLM attribution and INFERRED/0.45. Expose explicit per-write provenance/confidence: EXTRACTED means stated in evidence, INFERRED means the writer's own reasoning, AMBIGUOUS means conflicting evidence, USER_ASSERTED means an explicit human assertion. Clamp new inferred confidence to 0.75 in the core validation/write boundary before logging. Historical scores remain readable and replay unchanged. Repetition alone must not promote an LLM judgment into extracted evidence. Consolidation identity includes object kind and scope, and single-valued replacement stays within its scope; identical text in two scopes must not combine event references or retire each other. Source selection is still a writer responsibility; labels do not prove truth.
5. Keep event-to-candidate-to-claim promotion supported. Add a cheap direct event link to ordinary claim writing with scope/source validation, so a caller can record an event and then derived claims without candidate ceremony. A combined event/claim helper is optional only if it can prevalidate all inputs and document partial-failure/retry semantics; do not pretend several durable appends are one atomic operation. Expose a narrow `get_event(event_id, scope)` MCP read tool using the existing event view, `claims:read` authorization and exact resolved scope. A missing or out-of-scope event returns the same safe error; reading a linked rationale must not require raw-log access, an observation wrapper, embeddings or a model call. Existing claims without an event remain readable. Preserve ADR-0021’s existing candidate-scope override (for example, a global source event promoted into a project claim); same-scope validation applies to the new direct `event_id` field, while legacy candidate source references retain their existing semantics. Do not fabricate missing historical event links.
6. Extend the normal strict ontology, including aliases and cardinality, as agreed with the consumer before release:

   | Canonical predicate | Aliases | Cardinality | Object |
   | --- | --- | --- | --- |
   | concerns | about | multi | subject entity |
   | is | is_a | multi | classification/entity |
   | triggered_by | — | multi | trigger identifier |
   | supersedes | — | multi | earlier decision identifier |
   | has_rationale | — | multi | event reference |
   | has_outcome | — | single | terminal outcome |
   | has_direction | — | single | direction literal |
   | has_result_r | — | single | numeric literal |

   Preserve existing generic `is`/`concerns` multi-valued semantics. Single-valued corrections use logged consolidation/supersession rather than an unlogged overwrite. A `supersedes` relationship records the relationship; lifecycle retirement remains explicit. Decision subjects use consumer-owned identifiers; Aver does not invent competing decision IDs. The consumer owns its own rule changes and any later source-based backfill.
7. Ship a sanitized, local contrib graph viewer/exporter. No private database, generated real payload, screenshots, credentials or domain text enters the repo or fixtures. Export uses explicit local input/output, includes object kind and provenance, and keeps literal values readable without making them entity nodes. Test with invented data. No graph database or hosted service is introduced.

## Verification and rollout

Implementation, integration and review are separate roles; the reviewer independently evaluates exact candidate heads. No automatic acceptance from author checks.

Required synthetic/offline checks cover classification boundaries, legacy reads/log decoding, migration append ordering/replay/idempotency/protected entities, advisory/strict no-mutation behavior, source/privacy checks, new inference cap/defaults, ontology aliases/cardinality, scope-correct event links and safe viewer export/rendering. Run fmt, Clippy, tests and checks.sh. Preserve existing stdio, embeddings and recency tests. New ADR commits are explicitly authorized by this goal; checks.sh's clean-ADR check is run after the ADR is committed.

Before live changes: independently accept source and the prepared procedure; build release binaries off the target host; record source/binary hashes and runtime compatibility; demonstrate synthetic migration/replay/idempotency and rollback. Inspect the target's actual container/mount/wrapper facts without reading secrets. Prove the consumer has no active session before migrating or swapping, and retain a verified private store backup plus a timestamped backup binary. No interruption of the consumer's work, global kill or unrelated restart. If idleness cannot be established, finish independent work and park on that exact condition.

Migration must report only structural counts/kinds and unchanged history-prefix integrity. Reconnect the consumer's MCP through its existing configured credential wrapper; use privacy-safe synthetic checks and aggregate-only existing recall evidence. Preserve permissions, prior backups and always-on paths outside encrypted home directories. Reviewer checks observed effects before completion. Publish accepted work by normal fast-forward to a freshly fetched remote, leaving other checkouts intact. Report exact commits, predicates, counts, hashes, backups, viewer instructions and remaining limits to the owner.

## Rejected alternatives

- Reparse historical decision paragraphs into asserted facts: ambiguous and outside the authorized structural migration.
- Rewrite old JSONL or blanket relabel old claims: destroys audit history and invents unsupported epistemic judgments.
- Change generic `is`/`concerns` to single-valued for one consumer: breaks existing code-memory semantics.
- New graph storage/service or dependencies: unnecessary for a local typed projection and viewer.
- Deploy the older remote branch: would silently discard already deployed capabilities.
