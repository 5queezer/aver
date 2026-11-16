# Structured memory

Claim subjects and objects keep their string API. Objects carry an `object_kind`
(`entity` or `literal`). New writes classify JSON scalars (numbers, booleans,
null and quoted strings) as literals. Prose is also literal: over 160 Unicode
characters, any control character, over eight whitespace-separated words,
terminal sentence punctuation (`. ! ? 。 ！ ？`), `. `, or whitespace combined
with `! ? ; 。 ！ ？`. Short names, identifiers, ordinary paths and URIs remain
entities. Short single-token paths/URIs (including terminal punctuation), `.`
and `..` are exempt from the punctuation rule. This deterministic heuristic is intentionally limited: explicitly set
`object_kind=literal` for short text that is not an identifier. An explicit
entity request cannot turn recognized prose/scalars into an entity.

Literal claim text remains readable and recallable; graph expansion includes
literal facts but does not traverse their values as nodes. Historical logs
without an object kind replay with their old entity semantics.

## Explicit legacy cleanup

Back up the entire memory store and stop its writers before maintenance. Opening
an existing store upgrades schema metadata but does not run object cleanup.

```sh
aver --memory-dir /path/to/aver migrate-literals --dry-run
aver --memory-dir /path/to/aver migrate-literals
```

Both commands print only JSON counts (`claims_reclassified`, `entities_removed`).
The dry run appends nothing and changes no claim/entity rows. Every selected
entity name passes the normal text and private-path privacy filters before any
chunk is appended; an untrusted legacy projection row fails closed without a
partial migration. The migration
appends bounded `migrate_literals` audit records before projection changes. It
reclassifies matching objects, including inactive claims, and removes matching
orphan entities. Any name still referenced as a subject, an entity-valued object
or a hyperedge participant remains. Claim text, IDs, status, historical
provenance/confidence, events and old log bytes are preserved. No historical
paragraph is reinterpreted as new triples or given a guessed event link.

Repeat migration safely; an unchanged store adds no records. Replay reconstructs
the same typed projection. If interrupted after append, rebuild with
`aver --memory-dir /path/to/aver replay --force` before resuming writers. Restore
the matching backup with the old binary when rolling back; an old binary cannot
read the new audit record/schema. Preserve any writes made after the backup.

## Attribution and source events

New INFERRED claims and hyperedges cap confidence at 0.75. The existing INFERRED
default remains 0.45. EXTRACTED means explicitly stated in evidence; INFERRED
means a judgment; AMBIGUOUS means conflicting evidence; USER_ASSERTED means an
explicit human assertion. Repetition alone never upgrades a judgment to EXTRACTED.
Historical logged promotion outcomes and scores still replay as recorded.

Core `ClaimWrite` supports `.event_id(id)` for an existing event in the same
scope. It retains both the source string and an `event:<id>` reference. Replay
loads claims before events, then checks typed links after all logs load; dangling
or cross-scope typed links prevent replacing the current database. The existing
candidate workflow retains its historical explicit scope override semantics.

`.strict_atomic(true)` rejects recognized prose before claim append, including
prose inside a quoted JSON string. Non-prose scalar literals remain valid atomic
objects.

## MCP writer flow

`remember_claim` and `add_triple` accept optional `provenance`, `confidence`,
`object_kind`, `event_id` and `strict_atomic`. The two tools share the same core
validation. `remember_claim.source` is optional; `add_triple.source` is required.
Omitted `agent_kind` defaults to LLM for these tools and contradiction replacement
claims. Explicit `agent_kind` retains human/parser/external-tool attribution;
explicit `provenance` can distinguish a source-stated fact from a model's
judgment independently of the agent type. Provenance defaults confidence unless
an explicit confidence overrides it; the core still caps INFERRED at 0.75.

1. Call `record_event` with the reasoning/source context:

   ```json
   {"session_id":"invented-session","kind":"decision","payload":"Invented rationale for a sample decision.","source":"synthetic example","scope":"proj/sample"}
   ```

2. Use the returned event `id` for each atomic claim. In this invented example,
   replace `123` with that actual returned ID:

   ```json
   {"subject":"decision:sample","predicate":"concerns","object":"topic:sample","event_id":123,"source":"synthetic example","scope":"proj/sample"}
   ```

   A judgment uses the INFERRED default. For an explicitly stated source fact,
   set `"provenance":"EXTRACTED"`; use AMBIGUOUS for conflicting evidence and
   USER_ASSERTED for an explicit human assertion. These labels describe the
   evidence; they do not certify truth. Include a meaningful auditable source.

Use `get_event` with the event ID to read its rationale on demand:

```json
{"event_id":123,"scope":"proj/sample"}
```

`get_event` requires OAuth `claims:read` and the event's exact memory scope. If
`scope` is omitted, MCP uses the request's resolved scope. It returns the existing
event view (payload, source, attribution, session, kind and timestamp), performs
no embedding or writes, and deliberately uses the same retry guidance for a
missing event and an event in another scope. Recall continues to return claim
provenance without expanding raw event payloads.

Responses expose `provenance`, actual `confidence`, and `object_kind`. Linked
claims include `event_id` and `event:<id>` in `source_refs`. A prose object returns
`hint` explaining atomic triples and `record_event`. `strict_atomic=true` rejects
recognized prose, including decoded quoted strings; non-prose scalar literals
remain accepted. `embedded`
keeps its existing opt-in semantics. The two calls are separate durable writes:
a rejected claim does not undo the explicitly recorded source event. Inspect
existing claims before retrying after an I/O interruption; replay repairs the
projection from the log. There is no implicit multi-call transaction or dedupe key.

For reviewed extraction, the existing flow remains `record_event` →
`propose_candidate_claim(event_id, subject, predicate, object)` →
`promote_candidate_claim(candidate_id)` (or reject). Candidate promotion
retains its source reference and conservative INFERRED default. Its historical
explicit scope override is preserved; the new direct `event_id` field requires
an event in the same scope.

## Decision vocabulary and ontology extensions

| Predicate | Alias | Cardinality | Typical object |
| --- | --- | --- | --- |
| concerns | about | multi | subject-matter identifier |
| is | is_a | multi | classification identifier |
| triggered_by | — | multi | trigger identifier |
| supersedes | — | multi | earlier decision identifier |
| has_rationale | — | multi | event:id |

Aliases use the existing ontology resolver and remain accepted in stored claims.
Unknown predicates still fail for non-human provenance with the usual
vocabulary/retry hint. Cardinality uses existing logged consolidation: corrected
single-valued values supersede earlier ones when `consolidate` runs.
`supersedes` records a relation; use explicit retirement/contradiction tools for
lifecycle changes. Duplicate identity includes object kind and scope, so an
explicit literal is distinct from an entity with identical text, and
consolidation cannot merge event references across project scopes.

A deployment's own vocabulary goes into an optional `ontology.json` next to the
store's `db.sqlite` (ADR-0031). It is loaded after the core seed on every open
and before a replay applies records, so a CLI and a server see the same words:

```json
{
  "version": 1,
  "predicates": [{"name": "has_rating", "parent": "concerns", "cardinality": "single"}],
  "entity_types": [{"name": "Recipe", "parent": "Concept"}],
  "aliases": [{"alias": "rated", "predicate": "has_rating"}]
}
```

Every section is optional. `parent` may be omitted (a root) and `cardinality`
defaults to `multi`; predicate and alias names are lowercase `snake_case`.
Parents must be core entries or earlier entries in the file, and an alias must
point to one of them. Loading is additive and idempotent; the file never changes
a core entry and never deletes rows, so predicates seeded by an older binary stay
usable. A missing file means core vocabulary only; an unreadable or invalid file
(unknown key, other version, unknown parent, core override, alias clash) stops
the store open with an error naming the file and entry. Keep the file in the
store's backups: replay never rejects the log, so without the file extension
predicates come back under `relates_to` as multi-valued.

## Claim validity and confirmation

A claim may carry one `context` (an opaque entity id, such as `context:<id>`)
and a window `valid_from`/`valid_until` of the evidence that established it,
in Unix seconds (ADR-0032). A window needs a context, and `valid_from` is not
after `valid_until`. The same statement under two contexts is two claims: they
never merge or supersede each other, while claims without a context behave as
before.

```json
{"subject":"lesson:shade","predicate":"concerns","object":"topic:mulch","context":"context:dry","valid_from":1700000000,"valid_until":1700600000,"scope":"proj/garden"}
```

Evidence items are ordinary entities linked with core predicates:
`<item> in_context <context>`, `<item> concerns <topic>`, and, once the item
has an outcome, `<item> closed_at <time>` (single-valued; its value is
informational).

For a claim L with a context, `evidence_since_confirmation` counts distinct
items with ACTIVE claims, all in L's scope or a scope below it, that are
`in_context` L's context, `concerns` L's subject or one of the objects of L's
subject's own ACTIVE `concerns` claims (same scope set), and whose `closed_at`
claim was created in `(max(last_confirmed_at, valid_from), now]`. Predicates
include their descendants and aliases. `valid_until` does not end counting:
new evidence arrives after the window by nature. Arrival is the logged
creation time of the `closed_at` claim, so a one-time import counts older
closed items as new for claims confirmed earlier, and an item closed again
with a new `closed_at` counts again. The count is derived on read; nothing is
written and replay reproduces it.

aver only counts. Whether new evidence supports or refutes a claim is the
consumer's judgment: confirm with `reverify_claim` (CLI `aver reverify`), or
refute with `contradict`.

```json
{"claim_id":123,"evidence":["event:45","item:7"],"agent_kind":"LLM","valid_until":1701200000}
```

A confirmation needs an ACTIVE claim, a request scope equal to the claim's
scope, at least one evidence reference, and every `event:<id>`/`claim:<id>`
reference must exist in the claim's scope or below it (not the claim itself).
`valid_until` only extends a window the claim has. The record names its
writer and evidence; `last_confirmed_at` is the latest explicit confirmation,
else the claim's creation time. A re-assertion folded by `consolidate` moves
`last_verified_at` (recency) but is not a confirmation. Recall returns
`context`, `valid_from`, `valid_until`, `last_verified_at`,
`last_confirmed_at`, `evidence_since_confirmation` and the latest confirmation
(`at`, `agent_id`, `agent_kind`, `evidence`) only for claims with a context or
a confirmation; `aver confirmations --claim-id N` lists them all. Recall also
takes a `context` filter, and `AVER_RECALL_EVIDENCE_HALF_LIFE` opts hybrid
ranking into `0.5^(count / half_life)`; stored confidence never changes.

Replay may propose, live evidence confirms: hypotheses formed from replayed
history live in their own top-level scope (for example `replay/<run>`) with
their own evidence. Counting and confirmation stay within a claim's scope set,
so replay evidence neither confirms nor decays a claim in a live scope, and a
replay-scoped request cannot confirm one. A pre-declared out-of-sample test is
a live claim whose window starts after its creation: only items that close
after both its creation and the window start are counted. This isolation is
only as good as the scopes writers use: a per-call `scope` parameter overrides
the connection scope, so keeping replay writes out of live scopes is the
importer's discipline; aver does not stop a writer from naming a live scope.

A store whose `ontology.json` declares `in_context` or `closed_at` with a
different shape fails to open (ADR-0031); reconcile the file before upgrading.
An older binary refuses to open an upgraded store because its schema version
is newer. Roll back with the old binary plus the store backup taken before the
upgrade. Never replay a newer log with an older binary: it would silently drop
context and evidence.

## Viewer

See [the local graph viewer](../contrib/graph/README.md). No private store or
export is included. Export files can contain private facts and rationale; keep
them outside the repo and share only intentionally.

## Concurrent entity observations

Entity `last_seen_at` advances to the maximum observed record timestamp, including
ordinary touches and type promotion. `created_at` remains the first insertion's
timestamp. A writer can capture time before waiting for SQLite's writer lock;
merged logs and clock skew can also put older timestamps after newer ones.
Projection apply handles that ordering without changing timestamps in the audit
log, so live writes and replay keep the same entity metadata.
