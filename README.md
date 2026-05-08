# Aver

Aver is a local-first memory layer for coding agents: an append-only episodic log, a durable claim graph, vector recall primitives, deterministic code extraction, and an MCP/OAuth server surface in one Rust workspace.

Aver is experimental. The architecture is ADR-driven and the current implementation is useful, but not all ADRs are complete yet. See [Implementation status](#implementation-status) for what is built today.

## Why

Most agent memory systems either store loose text chunks or rely on opaque background reasoning. Aver takes a more conservative route inspired by *The Memory Layer*: every durable memory should be structured, provenance-tracked, privacy-checked, and replayable from an append-only log.

The goal is a trustworthy substrate for coding agents that can:

- remember explicit project facts as triples such as `PaymentGateway depends_on StripeSDK`,
- keep an audit trail of memory writes in `log.jsonl`,
- derive searchable claims and vectors from that log,
- reject secrets before they enter memory,
- extract code relationships deterministically with Tree-sitter,
- expose memory through a small CLI and MCP-compatible server.

## Features

- **Local-first storage** — SQLite plus `log.jsonl` under a configurable memory directory.
- **Append-first writes** — durable claims are appended to JSONL before SQLite insertion.
- **Structured claims** — memories are stored as `(subject, predicate, object)` claims with source references, confidence, status, and agent provenance.
- **Privacy gate** — token/path/entropy checks run before writes; rejected content is not persisted.
- **Keyword, vector, and hybrid recall primitives** — text recall is available through the CLI; vector chunks and hybrid ranking over active claims are implemented in the core crate.
- **Adaptive HybridRAG weights** — structural graph questions lean toward graph context; broad summary questions lean toward vectors; explicit alpha overrides are range-validated.
- **Graph expansion and communities** — local claim neighborhoods and deterministic connected-component communities are available in core.
- **Contradiction records and confidence decay** — contradictions are explicit audit records; consolidation can decay contradicted active claims and report merged/superseded/decayed counts.
- **Deterministic code extraction** — `aver-extractor` uses Tree-sitter Rust to extract functions, imports, calls, structs, enums, traits, impl methods, tests, and code facts.
- **Candidate claim workflow** — episodic events can produce staged claims that are promoted or rejected explicitly.
- **MCP/OAuth server** — `aver-server` exposes memory tools over Streamable HTTP MCP behind a local OAuth-style token flow, including the ADR-0008 five-tool surface with validated recall/write parameters and persisted confidence overrides.
- **Evaluation harnesses** — fixture evaluation plus a BEAM100K runner using local Ollama for embeddings, answer generation, and judging.

## Quick Start

### Install from this checkout

Prerequisites:

- Rust toolchain compatible with the workspace (`rust-version = 1.95`)
- `cargo`

```bash
./install.sh --from-source
# or, if you use just:
just install
```

This installs the `aver` CLI to `~/.cargo/bin` by default.

### Store and recall a claim

```bash
aver --memory-dir .aver status
aver --memory-dir .aver remember PaymentGateway depends_on StripeSDK --source session_47
aver --memory-dir .aver recall Stripe
```

Expected recall output:

```text
PaymentGateway depends_on StripeSDK
```

The store directory will contain:

```text
.aver/
├── db.sqlite   # indexed SQLite projection
└── log.jsonl   # append-only audit log
```

## How It Works

Aver separates memory into three projections:

```text
User / Agent
  → CLI or MCP tool
  → privacy filter
  → append JSONL event/claim first
  → insert/update SQLite projection
  → recall by text, vector, or hybrid ranking
```

The design maps to the ADRs:

- **Episodic log** — chronological append-only record in `log.jsonl`.
- **Semantic graph** — durable claims/triples in SQLite.
- **Vector store** — `vector_chunks` with JSON-serialized embeddings today; sqlite-vss-backed nearest-neighbor indexing is planned.
- **Extraction** — Rust Tree-sitter extractor turns source code into structured facts.
- **Graph tools** — recall, expand, add-triple, contradict, and consolidate map the ADR-0008 surface onto the local claim store.
- **Consolidation** — duplicate/conflict handling supersedes older claims, explicit contradictions can decay confidence, and report counts summarize merged, superseded, and decayed claims.

For a deeper implementation walkthrough, see [`doc/how-it-works.md`](doc/how-it-works.md). For design rationale, see the ADRs in [`doc/adr/`](doc/adr/).

## Design References

Aver's implementation is intentionally conservative and source-grounded:

- *The Memory Layer* frames durable memory as append-only triples consolidated from episodic fragments into a persistent graph, with HybridRAG combining vector search and graph traversal [ch.147–148].
- Karta demonstrates the value of active memory operations such as multi-hop traversal, contradiction detection, consolidation, confidence, and temporal awareness; Aver keeps those ideas behind explicit, auditable claim tools instead of opaque note mutation.
- MuninnDB shows practical retrieval controls such as mode/weight selection, entity graph traversal, relationship types, confidence-preserving entity state, and use-strength/decay; Aver adopts the local-first graph and adaptive retrieval pieces that fit its SQLite/Rust ADRs.

## CLI Usage

```bash
aver --help
aver --memory-dir .aver status
aver --memory-dir .aver remember <subject> <predicate> <object> --source <source>
aver --memory-dir .aver recall <query>
```

Current CLI commands:

| Command | Purpose |
|---|---|
| `status` | Open the store and report readiness. |
| `remember` | Append a user-asserted structured claim. |
| `recall` | Search active claims by keyword. |

## Server and MCP Usage

Run the MCP/OAuth HTTP server:

```bash
cargo run -p aver-server
```

Default configuration:

| Environment variable | Default | Purpose |
|---|---:|---|
| `AVER_HOST` | `127.0.0.1` | Bind host. |
| `AVER_PORT` | `3317` | Bind port. |
| `AVER_BASE_URL` | `http://127.0.0.1:3317` | Public base URL used in OAuth metadata. |
| `AVER_MEMORY_DIR` | `.aver` | Memory store directory. |
| `AVER_AUTH_DB_PATH` | `aver-auth.db` | SQLite auth database path. |

Useful endpoints:

- `GET /.well-known/oauth-authorization-server`
- `POST /oauth/register`
- `GET /oauth/authorize`
- `POST /oauth/token`
- `GET /api/health` with `Authorization: Bearer <token>`
- `/mcp` with `Authorization: Bearer <token>`

MCP tools currently include the ADR-0008 stable memory surface:

- `recall`
- `expand`
- `add_triple`
- `contradict`
- `consolidate`

Operational triggered-memory tools are also exposed:

- `remember_claim`
- `record_event`
- `should_extract_memories`
- `propose_candidate_claim`
- `list_candidate_claims`
- `promote_candidate_claim`
- `reject_candidate_claim`

## Evaluation

Run deterministic workspace tests:

```bash
cargo test --workspace --locked
# or
just test
```

Run the local quality gate:

```bash
just check
```

Run fixture evaluation:

```bash
cargo run -p aver-eval -- <fixture.json> [fixture.json ...]
```

Run BEAM100K with local Ollama:

```bash
cargo run -p aver-eval --bin aver-beam100k -- \
  --dataset path/to/beam-100k.json \
  --ollama-base-url http://localhost:11434 \
  --embedding-model nomic-embed-text \
  --generation-model gemma4 \
  --top-k 12
```

The BEAM runner expects Ollama to provide both the embedding model and generation/judge model.

## Project Structure

```text
agent-memory-layer/
├── crates/
│   ├── aver-core/       # Store, claims, events, privacy filter, vectors, recall, consolidation
│   ├── aver-cli/        # `aver` command-line interface
│   ├── aver-extractor/  # Tree-sitter Rust and prose fact extraction
│   ├── aver-server/     # MCP/OAuth HTTP server
│   └── aver-eval/       # Fixture and BEAM100K evaluation runners
├── doc/
│   ├── adr/             # Architecture decision records
│   └── how-it-works.md  # Current implementation walkthrough
├── migrations/          # Embedded SQLite migrations
├── install.sh           # Source/release installer
└── justfile             # Development automation
```

## Development

Common commands:

```bash
just build       # cargo build --workspace --locked
just test        # cargo test --workspace --locked
just fmt         # cargo fmt --all
just clippy      # cargo clippy --workspace --no-deps -- -D warnings
just check       # format check + clippy + tests + autoresearch checks
just release     # release build for aver-cli
just dist        # local release tarball and SHA256SUMS under target/dist
```

Without `just`, use the equivalent Cargo commands shown in the [`justfile`](justfile).

## Implementation Status

Implemented today:

- local-first `Store` backed by SQLite and JSONL,
- migrations for claims, vector chunks, ontology tables, episodic events, and candidate claims,
- append-first claim writes,
- privacy filtering before claim writes,
- claim CRUD and keyword recall,
- vector chunk storage and embedding abstractions,
- Ollama embedding client and deterministic mock embedding client,
- cosine similarity, adaptive HybridRAG weights, and hybrid vector/text recall primitives,
- graph expansion/traversal over active claim triples,
- explicit contradiction records and confidence decay for contradicted active claims,
- basic consolidation for duplicate/conflicting claims,
- CLI `status`, `remember`, and `recall`,
- Tree-sitter Rust extraction,
- structured prose fact parsing,
- MCP/OAuth server with ADR-0008 recall/expand/add-triple/contradict/consolidate tools and staged candidate-claim workflow,
- fixture and BEAM100K evaluation runners.

Partial or planned:

- sqlite-vss-backed nearest-neighbor virtual table integration beyond the current SQLite metadata plus JSON embedding storage,
- production shared-graph backend adapter beyond the current local connected-component community detection,
- broader production packaging, signed releases, and release automation beyond the current installer/`just dist` workflow.

## Documentation

- [`doc/how-it-works.md`](doc/how-it-works.md) — current runtime flow and ADR mapping.
- [`doc/adr/`](doc/adr/) — architecture decisions.
- [`autoresearch.md`](autoresearch.md) — active experimental protocol and guardrails.

## License

MIT, as declared in the workspace package metadata.
