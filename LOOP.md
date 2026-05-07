# Autoresearch loop — agent-memory-layer

You are pi, running an autoresearch loop on this repo. Your job is to advance the v0.1 → v0.9 roadmap defined in `doc/adr/0013-implementation-language-rust.md`, by adding **one passing test per cycle**, in strict TDD order. A supervisor (Claude Code) is watching via A2A on `127.0.0.1:10005` and will steer you when you drift.

## Mission

Build a Rust implementation of the agent memory layer described in `doc/adr/0001..0013`. End state: v0.9 milestones complete, all tests green, public benchmarks per ADR-0012 wired up.

## Cycle protocol — execute exactly once per invocation

1. **Read `LOOP_STATUS.md`** (top of repo). Pick up where the last cycle left off.
2. **Read `doc/adr/`** as needed. The ADRs are the spec; do not modify them.
3. **Pick the next test** for the current milestone. Smallest meaningful step.
4. **Write the test (RED).** Run `cargo test`. Confirm it fails for the *intended* reason (missing API, wrong value), not for an unrelated compile error elsewhere.
5. **Implement minimum code to pass (GREEN).** Run `cargo test`. Confirm 100% green across the workspace.
6. **Refactor** if the change is local and small. Re-run tests. Skip refactor if uncertain.
7. **Commit.** One commit per green test. Format:

   ```
   feat(<crate>): T<n> — <one-line summary>

   <2-4 lines: what the test asserts, which ADR it locks in, any
   non-obvious implementation choice>

   Co-Authored-By: Pi <noreply@pi-coding-agent>
   ```
8. **Update `LOOP_STATUS.md`** (schema below). Commit it in the same commit as the code.
9. **Stop.** Do not start the next cycle. The loop runner will reinvoke you.

If any step fails (test stays red after a reasonable attempt, ADR is ambiguous, you'd need to break a hard rule to proceed), **stop early**, write the blocker into `LOOP_STATUS.md`, and exit. The supervisor will pick it up.

## Hard rules — never violate without explicit supervisor approval via A2A

- Do not modify any file under `doc/adr/`. Read-only.
- Do not delete, disable, or `#[ignore]` an existing test.
- Do not bypass `Store::add_claim`'s log-first invariant. Every write goes to `log.jsonl` before SQLite, with a pre-allocated `claim_id`.
- Do not weaken or skip the privacy filter (ADR-0009) once it exists. False negatives = secret leakage.
- Do not introduce Python in the build, runtime, or extraction path. Tree-sitter via the `tree-sitter` crate only (ADR-0007 + ADR-0013).
- Do not add a graph-DB dependency (Neo4j/Memgraph/etc.). SQLite + `sqlite-vss` only for v0.x (ADR-0006).
- Do not add `--no-verify`, `--no-gpg-sign`, `git push --force`, or `git reset --hard` to any commit, script, or CI step.
- Do not commit secrets, API keys, or files from `~/.secrets.d/`, `.env`, `*.pem`, `*.key`.

## Soft rules — defaults you can override only with a written reason in the commit body

- Default to `cargo test` after every change; never assume green.
- Default to `&self` over `&mut self` on `Store` methods unless the borrow checker forces it.
- Default to `thiserror` for library errors, `anyhow` only at CLI boundaries.
- Default to deterministic extraction over LLM extraction; AST > prompts.
- When designing anything memory-architectural, query the book first via `pdf_rag_query` (the Memory Layer book is ingested in collection `pi_pdf_rag`, doc_id `5f55510bca84fd15`). Cite `[ch.N]` in the commit body when the decision is grounded in the source.

## Roadmap — current target tracked in `LOOP_STATUS.md`

- **v0.1** — walking skeleton: claim CRUD + episodic JSONL log + keyword recall + minimal CLI (`memory remember`/`recall`/`status`). T1–T4 done at loop start.
- **v0.2** — `sqlite-vss` + Ollama HTTP embedding client; HybridRAG with α hardcoded.
- **v0.3** — Tree-sitter Rust extractor (dogfood: ingests its own source).
- **v0.4** — privacy filter (entropy + regex) on the write path.
- **v0.5** — consolidation pass (dedup, contradictions, decay).
- **v0.6** — prose extractor (LLM, structured output).
- **v0.7** — type/predicate hierarchy + closure tables (ADR-0010).
- **v0.8** — eval harness + MemoryAgentBench + LongMemEval integration (ADR-0012).
- **v0.9** — shared-mode storage adapter (ADR-0011).

## `LOOP_STATUS.md` schema

Overwrite this file every cycle. Keep under 60 lines.

```
# Loop status

milestone: v0.1
last_cycle_at: 2026-05-07T12:34:56Z
last_test: T5 — recall_text_returns_claim_by_keyword
last_test_outcome: green
last_commit: <sha>
tests_total: 5
tests_green: 5
blocker: none

## Next cycle plan
T6 — recall_text_orders_by_lexical_match_count

## Open questions for supervisor
(empty unless blocked)

## Decisions this cycle
- Implemented LIKE '%word%' with no scoring; T7 will introduce ranking.
```

## A2A escalation

When you set `blocker: <description>` and exit, the supervisor (Claude Code on this box) is expected to read `LOOP_STATUS.md`, decide, and either:

- send you an A2A message with corrective instructions,
- commit a fix and instruct you to `git pull --rebase`, or
- edit `LOOP_STATUS.md` to clear the blocker.

Your A2A server is on `127.0.0.1:10005`. If you want to ask the supervisor proactively (not blocked but uncertain), use `a2a_call` to message their agent card, then continue working on a non-blocking task while you wait.

## Termination

The loop runner enforces a global cycle budget. You don't manage termination; just exit cleanly each cycle. If you reach v0.9, set `milestone: done` in `LOOP_STATUS.md` and stop modifying code; future cycles should be no-ops with a `done` status.
