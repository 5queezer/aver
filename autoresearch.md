# Autoresearch: Aver (Rust, ADR-driven, TDD)

## Supervisor notes (read this first)

Active steering from Claude Code (the supervisor). Newer notes on top. Pi: read, comply, leave the section in place — do not edit it.

### 2026-05-07 — advance to v0.3 (Tree-sitter extractor); stop polishing v0.2

You spent ~50 cycles on v0.2 and it is functionally complete (Ollama embed client, vector_chunks CRUD, HybridRAG blend with α validation, cosine similarity, vector recall returning deduped Claims, fallback to text). T52, T53, T54 are all variants of the same "short-circuit empty-input recall" idea — that's bikeshedding, not progress.

Move on. **Next milestone is v0.3 — deterministic AST extraction via Tree-sitter** (ADR-0007 + ADR-0013):

1. Create the crate: `crates/aver-extractor/` with `Cargo.toml` depending on `tree-sitter` and `tree-sitter-rust`. Add it to the workspace members in the root `Cargo.toml`.
2. Smallest meaningful first test: `extracts_function_definitions_from_rust_source` — given a Rust source string, return a list of function names. Use Tree-sitter's `Parser` + a query that matches `(function_item name: (identifier) @name)`.
3. Subsequent cycles add: `extracts_imports`, `extracts_calls`, `extracts_class_extends`, `extracts_function_to_test_mapping`. Each emits triples like `(File, defines, Function)`.
4. NO Python in build/runtime. Use the `tree-sitter` Rust crate and bundled grammar crates only (per ADR-0007 amendment).

The autoresearch.sh `MILESTONE` heuristic looks for `[ -d crates/aver-extractor ]` to bump to 3. So creating the crate is what advances the milestone metric — but only after a real test passes against it, not just an empty crate.

### 2026-05-07 — stop calling real Ollama from tests

Two crashes (T31, T36) traced to flaky `http://localhost:11434` loopback calls from unit tests. Metric dropped 5 points each time. The fix is structural:

1. Introduce an `EmbeddingClient` trait in `crates/aver-core/src/embedding/` (or wherever you put the existing client) with one method: `embed(&self, text: &str) -> Result<Vec<f32>, Error>`.
2. The current Ollama HTTP code becomes `OllamaClient: EmbeddingClient`.
3. Add `MockEmbeddingClient` that returns deterministic vectors (e.g. hash-based or fixture-loaded). All current Ollama tests must use this mock.
4. If you want to keep one real-Ollama smoke test, gate it behind a cargo feature: `#[cfg(feature = "live-ollama")]`. Do NOT use `#[ignore]` (still forbidden). `autoresearch.sh` does not enable that feature, so the live test stays out of the loop.

The point: `cargo test --workspace` must be deterministic and offline. If `cargo test` hits the network, the loop is broken.

This counts as one TDD cycle; the test you write to drive the refactor is the smallest meaningful step (e.g., "embedding_client_returns_deterministic_vector_via_mock").

## Objective

Advance the v0.1 → v0.9 roadmap defined in `doc/adr/0013-implementation-language-rust.md` by adding **one passing Rust test per cycle** in strict TDD discipline. Each `autoresearch.sh` run measures `tests_green` over the workspace; the goal is to monotonically increase it while keeping every gate in `autoresearch.checks.sh` green and every ADR unmodified.

This is **engineering autoresearch**, not parameter tuning. There is no hyperparameter search. Each cycle is a single TDD step (red → green → commit), measured by `tests_green` rising by exactly one (or `milestone_index` advancing).

## Metrics

- **Primary**: `tests_green` (unitless, higher is better). Counted across `cargo test --workspace`.
- **Secondary monitors**:
  - `tests_total` — should equal `tests_green`. Any divergence ⇒ a red test exists.
  - `milestone_index` — `1=v0.1 … 9=v0.9`. Reflects ADR-0013 roadmap. Heuristic-detected by `autoresearch.sh`.
  - `loc_core` — `wc -l` over `crates/aver-core/src/`. Watches for unchecked code growth.
  - `commit_count_total` — `git rev-list --count HEAD`. Sanity check that progress = commits.

A run is `keep`-eligible only if `tests_green == tests_total` AND (`tests_green` strictly increased OR `milestone_index` advanced). Otherwise `discard`. Crashes / compile errors are `crash`. Gate failures are `checks_failed`.

## How to Run

`./autoresearch.sh`

Runs `cargo test --workspace -q`, parses pass/fail counts, and emits one `METRIC name=value` line per metric. Compile failures or red tests exit non-zero (so the autoresearch driver records `crash`) but still emit metric lines.

## Checks

`./autoresearch.checks.sh` runs all of the following; any fail ⇒ `checks_failed`:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --no-deps -- -D warnings`
- ADRs unchanged: `git status --porcelain doc/adr/` must be empty.
- No new `#[ignore]`: `grep -rn '#\[ignore\]' crates/` must be empty.
- Log-first invariant heuristic: in `crates/aver-core/src/lib.rs`, the first `append_jsonl` call must appear before the first `INSERT INTO claims` in `Store::add_claim`'s body.

Checks must pass before logging `keep`. A `checks_failed` run means revert / fix and try again next cycle.

## Hard Guardrails — never violate without explicit supervisor approval via A2A

- Do not modify any file under `doc/adr/`. Read-only.
- Do not delete, disable, or `#[ignore]` an existing test.
- Do not bypass `Store::add_claim`'s log-first invariant. Every write goes to `log.jsonl` before SQLite, with a pre-allocated `claim_id`.
- Do not weaken or skip the privacy filter (ADR-0009) once it exists. False negatives = secret leakage.
- Do not introduce Python in the build, runtime, or extraction path. Tree-sitter via the `tree-sitter` crate only (ADR-0007 + ADR-0013).
- Do not add a graph-DB dependency (Neo4j/Memgraph/etc.). SQLite + `sqlite-vss` only for v0.x (ADR-0006).
- Do not add `--no-verify`, `--no-gpg-sign`, `git push --force`, or `git reset --hard` to any commit, script, or CI step.
- Do not commit secrets, API keys, or files matching `~/.secrets.d/**`, `.env*`, `*.pem`, `*.key`.

## Soft defaults — override only with a written reason in the commit body

- `cargo test` after every change; never assume green.
- `&self` over `&mut self` on `Store` methods unless the borrow checker forces it.
- `thiserror` for library errors; `anyhow` only at CLI boundaries.
- Deterministic extraction over LLM extraction (AST > prompts).
- When designing memory architecture, query the book first via `pdf_rag_query` (collection `pi_pdf_rag`, doc_id `5f55510bca84fd15`). Cite `[ch.N]` in commit bodies grounded in the source.

## Cycle protocol — one TDD step per `/autoresearch` invocation

1. Read `autoresearch.jsonl` (most recent record) for current state.
2. Read `doc/adr/` as needed for the spec. Do not modify it.
3. Pick the smallest meaningful next test for the current milestone.
4. Write the failing test (RED). Run `cargo test`; confirm intended-reason failure.
5. Implement minimum code to pass (GREEN). Run `cargo test`; confirm 100% green.
6. Refactor only if local and small. Re-run tests.
7. Run `./autoresearch.sh` (via `run_experiment` tool). Run `./autoresearch.checks.sh`.
8. If both pass: `git commit` with this message format:

   ```
   feat(<crate>): T<n> — <one-line summary>

   <2-4 lines: what the test asserts, which ADR it locks in, any
   non-obvious implementation choice>

   Co-Authored-By: Pi <noreply@pi-coding-agent>
   ```

9. Call `log_experiment(commit=<sha>, metric=<tests_green>, status="keep", description=...)`.
10. Stop. Do not start the next cycle. The `/autoresearch` driver reinvokes you.

If any step fails, log `crash` or `checks_failed` with a `description` that names the blocker, and stop early. The supervisor will pick it up.

## Roadmap (current target tracked in `autoresearch.jsonl`)

- v0.1 — walking skeleton: claim CRUD + episodic JSONL log + keyword recall + minimal CLI (`aver remember`/`recall`/`status`). T1–T4 done at loop start; **T5 = `recall_text_returns_claim_by_keyword`** is next.
- v0.2 — `sqlite-vss` + Ollama HTTP embedding client; HybridRAG with α hardcoded.
- v0.3 — Tree-sitter Rust extractor (dogfood: ingests its own source).
- v0.4 — privacy filter (entropy + regex) on the write path.
- v0.5 — consolidation pass (dedup, contradictions, decay).
- v0.6 — prose extractor (LLM, structured output).
- v0.7 — type/predicate hierarchy + closure tables (ADR-0010).
- v0.8 — eval harness + MemoryAgentBench + LongMemEval integration (ADR-0012).
- v0.9 — shared-mode storage adapter (ADR-0011).

## A2A escalation

The supervisor (Claude Code) is on `127.0.0.1:10005` (your A2A endpoint per pi config). When you log `crash` or `checks_failed`, the supervisor reads `autoresearch.jsonl` and either:

- sends an A2A message with corrective instructions,
- commits a fix and tells you to `git pull --rebase`, or
- toggles a hard guardrail off in this file (after deliberation).

Use `a2a_call` proactively for non-blocking questions; continue on a different sub-task while you wait.

## Termination

The `/autoresearch` driver enforces the global cycle / time / token budget. You don't manage termination. When `milestone_index >= 9` and all gates pass, log the final run with `status="keep"` and a `description` of "v0.9 reached"; future cycles should be no-ops emitting unchanged metrics.
