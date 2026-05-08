# AGENTS.md

Aver is a Rust, local-first, auditable memory layer for coding agents.

## Principles

- Log first: validate privacy, then append to `log.jsonl` before SQLite/vector projections.
- Store durable memory as structured claims/triples with provenance and confidence.
- Keep projections replayable from append-only records; never silently rewrite history.
- Prefer deterministic extraction and typed Rust paths; LLM/plugin output must pass through core validation.
- Keep the default path small and local: SQLite, JSONL, Tree-sitter, Ollama-compatible HTTP where needed.

## When changing code

- Preserve privacy filtering and log-first write ordering.
- Keep tests deterministic/offline unless an explicit live-provider command or feature is used.
- Update `README.md`, `doc/`, and ADRs whenever behavior, setup, architecture, or implementation status changes.
- Run fmt, clippy, and tests before claiming completion.
