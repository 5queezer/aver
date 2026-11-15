# Design review — 2026-10-17

Architecture-only review of the `aver` workspace at commit `f56f8b3`. Style, lint, and commit history were out of scope. Every point below names the file and the decision it refers to.

## Structure

### Boundaries

There is one seam in this repo, and it is the crate boundary. Inside `aver-core`, `crates/aver-core/src/lib.rs` is 6378 lines and `impl Store` runs from line 416 to line 4366 with 118 methods. Storage, validation, privacy, ontology, keyword recall, vector recall, graph expansion, shortest path, community detection, consolidation, observation compaction, candidate workflow, advisory locking, log rotation, and replay all live on one struct in one file. The submodules exist (`recall.rs` is 132 lines, `validation.rs` is 124) but they are crumbs swept off the table.

Replay is the clearest missing layer: `replay_with_mode` at line 5120 and its `apply_add_claim` reimplement the insert logic a second time with its own `ensure_entity_on`, so every write rule now has to be maintained in two hand-mirrored places.

### Coupling and cohesion

The public write API is combinatorial: `add_claim`, `add_claim_with_confidence`, `add_claim_from_agent`, `add_claim_with_scope`, `add_claim_with_confidence_and_scope`, `add_claim_from_agent_with_scope`. Four `record_event` variants, two `record_observation`, two `propose_candidate_claim`, three `recall_text`. There is already a `ClaimWrite` struct internally that carries all of these fields. The public API refuses to expose it, so ADR-0021 adding one column doubled the method count.

`aver-extractor`, the Tree-sitter crate with thirteen grammars and a 1271-line `lang/rust.rs`, is imported by nothing under any `crates/*/src`. The "deterministic code extraction" pillar from the README is an island. Core's `ingest_extractor_facts` takes core's own `ExtractorFact`, not the extractor crate's `ExtractedFact`. Two parallel types, no bridge.

### Abstraction level

Over-engineered: `crates/aver-server/src/adapters.rs` defines a six-variant `AdapterRuntime`, six empty unit-struct configs, a one-method `AdapterRuntimeConfig` trait, and an `AdapterConfig` tagged enum wrapping them. Its only consumer is a test asserting the enum has six variants. `PiAdapter` and `PiHook` are wired into no binary. `GraphStorageAdapter` has one impl and one consumer, a test. `MockObserver` and `MockClaimExtractor` ship in the production library.

Under-engineered: `Error` has fifty variants mixing `Sqlite` and `Io` with `InvalidTopK`, and `tools.rs` has to downcast through `anyhow` to find `UnknownPredicate`. `CandidateClaim.status` is a `String`. `ContradictionRecord.status` gets lowercased ad hoc in the view conversion.

### Dependency direction

- `aver-core` ships with `default = ["ollama"]` and a `ureq` client, so the storage library makes HTTP calls to port 11434 unless you opt out. The embedding provider belongs behind a trait at the edge.
- `aver-core/src/extractor.rs` is a chat-text heuristic extractor ("i prefer X") living in the storage core while the real AST extractor crate sits unused.
- Authorization points the wrong way: `scopes.rs` `required_scope_for_tool` string-matches tool names and `ALL_TOOL_NAMES` is kept in sync by a unit test. The tool should declare its scope.
- `AverMcpService::open` is the rmcp session factory in `http.rs`, so every MCP session opens a fresh `Store`: migration check, rotation check, `seed_ontology`, new SQLite connection. Store construction is treated as free. It is not.

## State and flow

### Where truth lives

The docs say `log.jsonl` is the source of truth. `insert_claim` at line 1033 runs `SELECT COALESCE(MAX(id),0)+1 FROM claims` against the projection to mint the id that goes into the log. Truth is circular.

Claims are appended twice, to `log.jsonl` and to `agents/<id>/log.jsonl`, and `collect_replay_inputs` walks both and relies on `ReplayDuplicateId` tolerance to discard the second copy. Ontology extensions go to `predicate_types` and `ontology_extension_log` tables, never to JSONL, so replay re-derives them. Vector chunks are not logged at all. "Replayable" has three different answers depending on which row you ask about.

Validation truth lives in three places. Migrations 0009 through 0119 are almost entirely one `CHECK` trigger each, with `nonblank`, `positive`, and `charset` in the filenames. `validation.rs` and `validate_claim_write` repeat them in Rust. `tools.rs` repeats them again as `if params.subject.trim().is_empty() bail!`. Same rule, three enforcement points, three error messages.

`auth.rs` `AuthDb::open` has its own inline schema with `let _ = conn.execute_batch("ALTER TABLE ...")` ignore-the-error migrations, sitting next to a core whose `build.rs` panics on a gap in a 119-step sequence. Two migration philosophies in one workspace.

### Trace of one `recall` call

1. Claude Code sends to `aver-scope-shim`, a separate axum + reqwest + tokio process whose entire job is to strip and inject one header.
2. The shim streams to `aver-server`, where `validate_bearer_token` inserts `GrantedScopes` and `resolve_request_scope` inserts `ResolvedScope` into request extensions.
3. rmcp dispatches to `AverMcpService::recall`, which re-looks up the tool name string in the scope catalog, then copies `RecallParams` field by field into `CoreRecallParams`.
4. It locks `Mutex<AverTools>`.
5. `AverTools::recall` re-validates `top_k` with different bounds than core's `validate_top_k`, computes `HybridWeights` alpha, and discards it as `_alpha`. It returns `chunks: Vec::new()` unconditionally.
6. It calls keyword `recall_text_with_filters`, runs `expand_with_scope` separately, and if that is empty runs it again on the first claim's subject.

The punchline: ADR-0004 adaptive HybridRAG and the ADR-0017 vector index are unreachable from the MCP surface. `recall_hybrid_claims` is called only by `aver-eval/src/beam.rs` and tests. The eval harness benchmarks a retrieval path no agent ever hits.

The default scope walk is computed twice, once in `scope_resolution.rs` as `default_walk` and once in `tools.rs` `parse_scope_walk`, whose comment says Layer 2 will refine it. Layer 2 shipped. The Layer 1 logic is still there.

## The hack and the fix

### Extensibility delusion

Built for scale that will not come:

- Six-runtime adapter registry for zero wired runtimes.
- `shared_mode = "planned"` in workspace metadata with a `StorageMode` trait method for a mode that does not exist.
- A per-workspace proxy process for a header that an MCP client config line can set.

Hardcoded where it hurts now:

- `"nomic-embed-text"` as a literal in `add_vector_chunk`.
- `scope: "global"` baked into `ingest_extractor_facts`, so extractor output can never be project-scoped.
- `agent_id` defaulting to `"mcp"`, so every MCP write looks like the same agent. ADR-0021 complains about exactly this, in prose, while the code keeps doing it.

### The load-bearing hack

It is the `MAX(id)+1` inside `BEGIN IMMEDIATE` in `insert_claim`, repeated for events, observations, candidates, contradictions, and hyperedges. The whole "append-only log, SQLite is a projection" architecture depends on the projection to hand out identities.

That is why replay must apply files in a fixed series order, why agent logs are deduped by id-with-conflict-check, why a log is only meaningful relative to the exact database that minted it, and why two memory directories can never merge. Shared mode is blocked by this one line. Every cross-reference in the log (candidate to event, contradiction to claim, observation to source events) is a projection-allocated integer. The log cannot be written without a live, fully migrated SQLite, so "log-first" is true only about byte ordering on disk.

### The single change

Generate ids in the writer, not the projection, and make one `apply` function the only path that touches SQLite, shared by live writes and replay.

Give `Store` a single `append(Record) -> Result<Id>` that validates, privacy-checks, writes the JSONL line, then calls the same `apply_*` that replay calls. That one move collapses the dual write paths, deletes the per-agent duplicate log, makes replay order-insensitive, unblocks merging stores, and gives `AverTools`, the CLI, and replay one function to call instead of three stacks to keep in sync.
