# Aver

Aver is a local-first memory layer for coding agents: an append-only episodic log, a durable claim graph with first-class hyperedges, vector recall primitives, deterministic code extraction, and an MCP/OAuth server surface in one Rust workspace.

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
- **Append-first writes** — durable claims and hyperedges are appended to JSONL before SQLite insertion.
- **Structured claims** — memories are stored as `(subject, predicate, object)` claims with source references, confidence, status, and agent provenance.
- **First-class hyperedges** — n-ary memories can be stored with predicate, provenance, confidence, source references, status, timestamps, and role/entity participants.
- **Privacy gate** — token/path/entropy checks run before writes; rejected content is not persisted.
- **Keyword, vector, and hybrid recall primitives** — text recall is available through the CLI; vector chunks and hybrid ranking over active claims are implemented in the core crate.
- **Adaptive HybridRAG weights** — structural graph questions lean toward graph context; broad summary questions lean toward vectors; explicit alpha overrides are range-validated.
- **Graph expansion, path queries, and communities** — local claim neighborhoods, confidence/provenance-aware shortest path queries over active claims and hyperedges, and deterministic weighted community detection are available in core.
- **Contradiction records and confidence decay** — contradictions are explicit audit records; consolidation can decay contradicted active claims and report merged/superseded/decayed counts.
- **Deterministic code extraction** — `aver-extractor` uses Tree-sitter Rust to extract functions, imports, calls, structs, enums, traits, impl methods, tests, and code facts.
- **Candidate claim workflow** — episodic events can produce staged claims that are promoted or rejected explicitly.
- **Observation continuity surfaces** — episodic events can produce privacy-checked, source-backed observations that are recallable by ID, summarized by compaction, and coverage-accounted across full session event ranges.
- **Continuity reliability controls** — coverage accounting, deterministic `catch-up`, gap warnings in summaries, destructive prune operations blocked when gaps remain, prune markers preserved in logs, and audit-aware observation recall.
- **MCP/OAuth server** — `aver-server` exposes memory tools over Streamable HTTP MCP behind a local OAuth-style token flow, including the ADR-0008 five-tool surface with validated recall/write/event/extraction-trigger parameters, observation projection tools, explicit unsupported-scope errors, persisted confidence overrides, and recall subgraphs with confidence floors.
- **Evaluation harnesses** — fixture evaluation plus a BEAM100K runner using local Ollama for embeddings, answer generation, and judging.

## Quick Start

### Install from this checkout

Prerequisites:

- Rust toolchain compatible with the workspace (`rust-version = 1.95`)
- `cargo`

```bash
./install.sh
# equivalent when run from the Aver git checkout:
./install.sh --from-source
# or, if you use just:
just install
```

When run from the Aver git repository, `install.sh` detects the checkout and installs from source. This installs the `aver` CLI to `~/.cargo/bin` by default. For local MCP/OAuth service deployments that execute `target/release/aver-server`, run `just release-server` to build and strip the server binary before restarting the service.

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

- **Episodic log** — chronological append-only record in `log.jsonl` and `events.jsonl`.
- **Observation projection** — privacy-checked session-continuity observations over episodic events, with source-event provenance and mechanical compaction summaries.
- **Observation coverage accounting** — `Store::observation_coverage()` computes covered/uncovered event IDs per session; uncovered IDs are exposed to callers and used to block unsafe pruning.
- **Continuity blockers** — session summaries mark uncovered ranges explicitly, and pruning refuses to proceed until coverage gaps are resolved by catch-up projection.
- **Prune markers + audit recall** — pruning emits append-only tombstones in `observations.jsonl`; pruned observations disappear from default views but remain recallable with audit metadata.
- **Semantic graph** — durable claims/triples in SQLite.
- **Ontology reasoner** — ADR-0010 entity and predicate hierarchies are seeded on open, materialized into closure tables, and used by graph expansion and path predicate filters so abstract filters such as `depends_on` also match descendant predicates like `calls`, `imports`, and accepted aliases such as `requires`; MCP/tool-facing diagnostics for unknown non-user predicates use alias-aware fuzzy suggestions plus the current predicate/alias vocabulary.
- **Typed entities** — claim subjects/objects are projected into `entities` with prefix-based types such as `Function:*` and fallback `Thing` for unknown entities.
- **Vector store** — `vector_chunks` with JSON-serialized embeddings and a `sqlite-vec`/`vec0` ANN table where the bundled extension is available.
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
aver --memory-dir .aver communities
```

Current CLI commands:

| Command | Purpose |
|---|---|
| `status` | Open the store and report readiness. |
| `remember` | Append a user-asserted structured claim. |
| `recall` | Search active claims by keyword. |
| `record-event` | Record an episodic event with session/kind/payload and optional source. |
| `should-extract-memories` | Check extraction trigger conditions for a session. |
| `propose` | Propose a candidate claim from an event. |
| `list-candidates` | List candidate claims with optional session/status filters. |
| `promote` | Promote a candidate claim into durable memory. |
| `reject` | Reject a candidate claim with a reason. |
| `record-observation` | Record a session observation from source events. |
| `recall-observation` | Recall an observation with its supporting event payloads and audit status. |
| `observation-coverage` | Report event coverage and uncovered ranges for a session. |
| `catch-up` | Run a deterministic catch-up projection over uncovered events. |
| `compaction-summary` | Assemble a continuity summary including coverage gap warnings. |
| `expand` | Expand an entity neighborhood from the local claim graph. |
| `communities` | Print deterministic weighted graph communities with score and bridge nodes. |
| `add-triple` | Append a confidence-bearing structured triple. |
| `contradict` | Record a contradiction for a claim id and optional replacement claim. |
| `consolidate` | Consolidate active duplicates/conflicts and apply confidence decay. |
| `vacuum` | Run `VACUUM` (and optional analysis). |
| `replay` | Rebuild SQLite from the append-only logs. |

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
| `AVER_AUTH_DB_PATH` | `<AVER_MEMORY_DIR>/auth.db` | SQLite auth database path. |
| `AVER_CORS_ORIGINS` | *(allow any origin)* | Optional comma-separated allowed origins for protected MCP CORS responses. |
| `AVER_TRUSTED_AUTH_HEADER` | *(unset)* | Optional reverse-proxy header name (for example `X-Forwarded-User`) that enables non-loopback OAuth authorization using Profile C trusted-header auth. |


Useful endpoints:

- `GET /.well-known/oauth-authorization-server`
- `POST /oauth/register`
- `GET /oauth/authorize` (browser consent screen; loopback by default, optional trusted-header for non-loopback)
- `POST /oauth/authorize/decision` (consent-screen form submission)
- `POST /oauth/token` for authorization-code + PKCE token exchange and refresh-token grants
- `GET /api/health` with `Authorization: Bearer <token>`
- `/mcp` with `Authorization: Bearer <token>`

`/oauth/authorize` drives a browser consent flow (ADR-0020). After a client dynamic-registers via `POST /oauth/register`, it redirects the user to `/oauth/authorize` with the standard PKCE parameters. Aver renders a consent screen showing the client name, redirect URI, and all supported scopes as checkboxes, with the client's requested scopes pre-selected; on **Approve** it stores a per-client consent row, mints an authorization code bound to the checked scopes, and redirects back to the client's `redirect_uri` with `code` and `state`. The client then exchanges the code at `/oauth/token` for an `access_token` plus `refresh_token`; refresh grants issue a new access token while preserving the existing refresh token, and access tokens carry only the scopes recorded on the consent row.

The flow supports loopback (`127.0.0.1` / `::1`) callers by default (Profile A in ADR-0020). Non-loopback callers can also authenticate via Profile C when `AVER_TRUSTED_AUTH_HEADER` is set to a trusted upstream identity header (for example `X-Forwarded-User`); otherwise they are rejected with an HTML 403.

### Connecting an MCP client

For Visual Studio Code, drop a workspace-level `.vscode/mcp.json` similar to:

```json
{
  "servers": {
    "aver": {
      "type": "http",
      "url": "http://127.0.0.1:3317/mcp"
    }
  }
}
```

Then run **MCP: Add Server** from the command palette and pick `aver`. VS Code dynamic-registers with `POST /oauth/register`, opens the consent screen in your browser, and — after you click **Approve** — receives the authorization code and exchanges it for an access token automatically. Other MCP clients that support the OAuth 2.1 + PKCE discovery profile (`/.well-known/oauth-authorization-server`) follow the same path.

MCP currently exposes 18 tools through a progressive discovery card so agents keep the active choice set small:

- **Default active set:** `recall`, `remember_claim`, `record_event`, `record_observation`, `assemble_compaction_summary`
- **Event-to-claim workflow:** progressively load `should_extract_memories`, `propose_candidate_claim`, `list_candidate_claims`, `promote_candidate_claim`, `reject_candidate_claim` only when converting raw events into reviewed durable claims
- **Graph navigation:** progressively load `expand` after recall returns an entity or an anchor is already known; use `add_triple` instead of `remember_claim` only when explicit source/confidence control is required
- **Observation audit:** progressively load `recall_observation`, `observation_coverage` for handoff, compaction, or provenance checks
- **Maintenance/repair:** keep `contradict`, `retire_claim`, `consolidate`, `add_vector_chunk` hidden until there is an explicit repair or retrieval-tuning need

Aver's MCP guide is intentionally proactive but selective: agents should recall first, then record durable user-shared preferences, project facts, and long-lived working context even when the user does not say "remember this" explicitly. Identity details should be recorded only when they are necessary, user-shared, and not sensitive personal data. When durability is uncertain, agents should prefer `record_event` over `remember_claim`, and they must not store secrets, credentials, sensitive personal data, transient chat, or facts they cannot explain with provenance.

CLI-only continuity and maintenance surfaces (`catch-up`, `compaction-summary`) are implemented in `aver-cli`; MCP exposes `record_observation`, `assemble_compaction_summary`, and the observation audit tools above, while claim-maintenance tasks stay available through the four advanced tools when agents explicitly need them.

Adapter boundaries are explicit in `aver-server` via the `adapters` module (`Pi`, `ClaudeCode`, `CodexOpenAi`, `OpenCode`, `Mcp`, `JsonlCliHarness`) so host runtimes can be added without leaking host-specific logic into `aver-core`.

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

The eval crate also exposes deterministic data structures for ADR-0012 query-suite regression threshold checks, hallucination-rate memory-on/off reports, graph-stat drift snapshots with privacy-rejection counters, and typed prompt contracts. Prompt contracts validate rendered prompt text before a model call using deterministic checks such as required text, forbidden text, required sections, unresolved-template detection, and character budgets. These checks validate prompt generation code; live judge/provider integrations and output-quality evals should remain separate and feed recorded case results into the eval structures.

Run BEAM100K with local Ollama:

```bash
cargo run -p aver-eval --bin aver-beam100k -- \
  --dataset path/to/beam-100k.json \
  --ollama-base-url http://localhost:11434 \
  --embedding-model nomic-embed-text \
  --generation-model gemma4 \
  --top-k 12
```

The BEAM runner expects Ollama to provide both the embedding model and generation/judge model. Retrieval tuning can override HybridRAG alpha in addition to `top_k`:

```bash
cargo run -p aver-eval --bin aver-beam100k -- \
  --dataset path/to/beam-100k.json \
  --top-k 16 \
  --retrieval-alpha 0.65
```

For Bayesian-style retrieval search over prior live runs, write JSONL observations with `top_k`, `alpha`, and `metric`, then ask Aver for the next autoresearch configuration:

```bash
cargo run -p aver-eval --bin aver-tune-retrieval -- \
  --observations retrieval-observations.jsonl
```

The tuner prints `BEAM_TOP_K`, `BEAM_RETRIEVAL_ALPHA`, and `AVER_AUTORESEARCH_TARGET=beam` values that can be used for the next autoresearch run. Keep a held-out validation split; do not tune directly against final benchmark labels.

## Prose/document plugin boundary

ADR-0013 permits non-Rust prose/document extraction plugins only behind stdin/stdout JSON-RPC. `aver-extractor::JsonRpcPluginRunner` sends one JSON-RPC request to a configured child process, parses the response, validates extracted fact fields, and returns facts to Rust callers. Plugins are extraction-only: they do not write memory directly and cannot bypass core privacy/log-first validation. The current runner is a process boundary, not an OS sandbox; production deployments should run plugins from an allowlisted command with external filesystem/environment sandboxing when untrusted plugins are enabled.

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
- migrations for claims, hyperedges, vector chunks, ontology tables, episodic events, candidate claims, and observation projections,
- append-first claim, hyperedge, and event writes,
- privacy filtering before claim, event, and observation writes,
- claim CRUD and keyword recall,
- active-only hyperedge create/list/recall/traversal APIs,
- vector chunk storage and embedding abstractions,
- Ollama embedding client and deterministic mock embedding client,
- cosine similarity, adaptive HybridRAG weights, and hybrid vector/text recall primitives,
- graph expansion/traversal over active claim triples, confidence/provenance-aware path queries over active claims and hyperedges, and weighted community detection,
- explicit contradiction records and confidence decay for contradicted active claims,
- basic consolidation for duplicate/conflicting claims,
- CLI `status`, `remember`, `recall`, `communities`, and observation continuity surfaces (`record-observation`, `recall-observation`, `observation-coverage`, `catch-up`, `compaction-summary`),
- Tree-sitter Rust extraction,
- structured prose fact parsing,
- MCP/OAuth server with ADR-0008 recall/expand/add-triple/contradict/consolidate tools, staged candidate-claim workflow, and observation recall/compaction-summary tools,
- ADR-0020 browser consent flow for `/oauth/authorize` (loopback Profile A) replacing the legacy `approval_token` gate,
- fixture and BEAM100K evaluation runners.

Partial or planned:

- production vector-index operations beyond the current bundled `sqlite-vec`/`vec0` local ANN path and JSON fallback metadata,
- adapter-boundary runtime integration crates/modules for Pi, Claude Code, Codex/OpenAI, OpenCode, MCP, and JSONL/CLI harnesses are defined by stable config/runtime types and adapter tests; runtime connectors remain partially implemented outside core types.
- production shared-graph backend adapter beyond the current local weighted community detection,
- broader production packaging, signed releases, and release automation beyond the current installer/`just dist` workflow.

## Documentation

- [`doc/how-it-works.md`](doc/how-it-works.md) — current runtime flow and ADR mapping.
- [`doc/adr/`](doc/adr/) — architecture decisions.
- [`autoresearch.md`](autoresearch.md) — active experimental protocol and guardrails.

## License

MIT, as declared in the workspace package metadata.
