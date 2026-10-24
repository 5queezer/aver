# Roadmap to maturity

Written 2026-10-24 against commit `abe6e04`. Companion to
[`design-review-2026-10-17.md`](design-review-2026-10-17.md), which names the
structural problems; this document orders the fixes.

Phases are ordered by dependency, not by appeal. Each has an exit criterion
that can be checked mechanically. Estimates assume one focused engineer.

## Current state

- Clippy clean, 884 deterministic offline tests.
- Core storage, privacy gate, OAuth consent flow, and migration replay are
  solid for a single-user local store.
- The MCP `recall` tool computes the HybridRAG alpha and discards it, and
  returns `chunks: Vec::new()` unconditionally. Hybrid retrieval (ADR-0004)
  and the vector index (ADR-0017) are unreachable from the agent surface.
  The BEAM eval harness benchmarks a path no agent calls.
- `aver-extractor` (13 grammars, ~6200 lines) is imported by nothing under
  `crates/*/src`.
- Claim ids are minted with `SELECT MAX(id)+1` against the SQLite projection
  in eight places, so the log cannot be written without a live database and
  two stores cannot be merged.
- 119 migrations, mostly one `CHECK` trigger each, produced by an
  autoresearch loop whose objective was "tests_green rises by exactly one per
  cycle". The same rules exist again in `validation.rs` and again as `bail!`
  in `tools.rs`.
- The systemd unit executes `target/release/aver-server` directly from the
  build tree, so a `cargo clean` takes the service down.
- `refactor/core-split` (3 commits ahead of master, unmerged since May)
  already splits `lib.rs` into cohesive modules.

## Phase 0: Stop the bleeding (one day)

- Make the service survive a `cargo clean`. Install the binary to
  `~/.local/bin/aver-server`, point the unit at it, and add a
  `just deploy-server` recipe that builds, copies, and restarts. Add a health
  probe so a dead server is visible.
- Freeze migrations. Squash 0009 through 0119 into one baseline for fresh
  stores; keep the old chain only behind an upgrade path for existing stores.
- Retire the autoresearch loop or replace its objective. The "one test per
  cycle" metric produced the schema above.
- Prune worktrees. Seven of the eight stale branches are squash-merged as
  PRs 3 through 9. Remove them. Keep `refactor/core-split`.

**Exit:** `systemctl --user status aver-mcp` is active after a clean build,
and `ls migrations | wc -l` is under ten.

## Phase 1: One write path (two to three weeks)

This is the single change named in the design review. Nothing later in this
roadmap is safe until it lands.

1. Rebase and merge `refactor/core-split` so the work happens in modules.
2. Move id generation into the writer (ULID, or a per-store monotonic counter
   persisted in the log header). Delete the eight `MAX(id)+1` sites.
3. Introduce one `apply(Record)` function as the only code that touches
   SQLite. Live writes: validate, privacy-check, append JSONL, `apply`.
   Replay: `apply` per log line. Delete the second insert implementation in
   `replay_with_mode`.
4. Collapse the combinatorial write API behind the existing `ClaimWrite`
   struct. Six `add_claim_*` variants become one; same for `record_event`,
   `record_observation`, `propose_candidate_claim`, `recall_text`.
5. Pick one validation layer. Keep Rust validation in core; drop the
   per-field triggers and the duplicate checks in `tools.rs`.
6. Drop the per-agent duplicate log (`agents/<id>/log.jsonl`) or make it a
   derived view. One log, one truth.

**Exit:** a store rebuilt by `aver replay` from only its JSONL is
byte-identical in SQLite to the original, and two stores can be merged by
concatenating their logs.

## Phase 2: Make the MCP surface honest (one to two weeks)

- Wire hybrid recall. `AverTools::recall` calls `recall_hybrid_claims` with
  the alpha it already computes and returns real `chunks`.
- Put the embedding provider behind a trait at the edge. Core stops
  depending on `ureq` by default; the server owns the Ollama client; tests
  use the existing mock. Remove the `"nomic-embed-text"` literal from core.
- Fold `aver-scope-shim` into client configuration if the MCP client can set
  the header. If it genuinely cannot, record why in an ADR and keep it.
- Delete `adapters.rs`, `MockObserver`, and `MockClaimExtractor` from the
  production library. Re-add when a runtime is actually wired.
- Make the tool declare its required scope instead of `scopes.rs`
  string-matching tool names.
- Default `agent_id` from the authenticated client, not the literal `"mcp"`.

**Exit:** a BEAM run through the MCP endpoint produces the same scores as the
direct harness run.

## Phase 3: Extraction that reaches memory (two weeks)

- Bridge `aver-extractor` to core: alias core's `ExtractorFact` to the
  extractor's `ExtractedFact`, or write one conversion.
- Add `aver extract <path>` to the CLI and an `extract` MCP tool. Facts land
  in the project scope, not `global`.
- Remove the chat-text heuristic extractor from core, or move it to
  `aver-eval` where it is used.

**Exit:** `aver extract crates/` on this repository produces claims that
`aver recall` returns, scoped to the project.

## Phase 4: Operational maturity (ongoing, start after Phase 2)

- Release workflow ships `aver-server` as well as `aver-cli`, with the
  systemd unit as an install artifact.
- `Store::open` becomes cheap or cached. Today every MCP session opens a
  fresh store with migration check, rotation check, and ontology seeding.
- Split `Error` into infrastructure and domain errors so `tools.rs` stops
  downcasting through `anyhow`.
- Replace `String` statuses (`CandidateClaim.status`,
  `ContradictionRecord.status`) with enums.
- Run a real agent trial: Claude Code against one project for two weeks,
  logging every recall that returned nothing useful. That data decides
  Phase 5.

**Exit:** a tagged release installs and runs on a clean machine from the
release artifacts alone.

## Phase 5: Only after Phases 1 through 4

- Shared mode (ADR-0011), which Phase 1 unblocks.
- Community detection and path queries at the MCP surface.
- ADR-0024 decision model. It is a reasonable idea applied to a retrieval
  path agents cannot reach yet.

## Definition of beta

Single-user, local, replayable from the log alone, with hybrid recall and
code extraction reachable from MCP. That is the README after Phase 3. Rough
total: eight to ten weeks of focused work.
