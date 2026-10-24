# Aver

Aver is a local-first memory layer for coding agents: an append-only episodic log, a durable claim graph with first-class hyperedges, vector recall primitives, deterministic code extraction, and an MCP/OAuth server surface in one Rust workspace.

Aver is experimental systems infrastructure. The architecture is ADR-driven and the implementation is useful for local development, but it is not yet hardened as a hosted multi-user service. See [Implementation status](#implementation-status) for what is built today.

## Status

- **Maturity:** experimental / alpha
- **Default deployment:** localhost-first CLI and MCP server
- **Quality gate:** `cargo fmt`, `cargo clippy`, `cargo test`, and project policy checks in CI
- **Security posture:** privacy filtering runs before memory writes; public deployments still require explicit CORS origins, HTTPS, and trusted reverse-proxy boundaries

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
- **Append-first writes** — durable claims and hyperedges are appended to JSONL before SQLite insertion, and lifecycle transitions (retire, contradict, candidate staging, consolidation supersede/decay/merge) are logged the same way so replay reconstructs them.
- **Structured claims** — memories are stored as `(subject, predicate, object)` claims with source references, confidence, status, and agent provenance.
- **First-class hyperedges** — n-ary memories can be stored with predicate, provenance, confidence, source references, status, timestamps, and role/entity participants.
- **Privacy gate** — token/path/entropy checks run before writes; rejected content is not persisted.
- **Keyword, vector, and hybrid recall primitives** — text recall is available through the CLI; vector chunks and hybrid ranking over active claims are implemented in the core crate.
- **Adaptive HybridRAG weights** — structural graph questions lean toward graph context; broad summary questions lean toward vectors; explicit alpha overrides are range-validated.
- **Graph expansion, path queries, and communities** — local claim neighborhoods, confidence/provenance-aware shortest path queries over active claims and hyperedges, and deterministic weighted community detection are available in core.
- **Contradiction records and confidence decay** — contradictions are explicit audit records; consolidation can decay contradicted active claims and report merged/superseded/decayed counts.
- **Deterministic code extraction** — `aver-extractor` uses Tree-sitter to extract functions, imports, calls, structs, enums, traits, impl methods, tests, and code facts across 13 languages (C, C++, C#, Go, Java, JavaScript, Kotlin, PHP, Python, Ruby, Rust, Swift, TypeScript).
- **Candidate claim workflow** — episodic events can produce staged claims that are promoted or rejected explicitly.
- **Observation continuity surfaces** — episodic events can produce privacy-checked, source-backed observations that are recallable by ID, summarized by compaction, and coverage-accounted across full session event ranges.
- **Continuity reliability controls** — coverage accounting, deterministic `catch-up`, gap warnings in summaries, destructive prune operations blocked when gaps remain, prune markers preserved in logs, and audit-aware observation recall.
- **MCP/OAuth server** — `aver-server` exposes memory tools over Streamable HTTP MCP behind a local OAuth-style token flow, including the ADR-0008 five-tool surface with validated recall/write/event/extraction-trigger parameters, observation projection tools, ADR-0021/0022 scoped memory parameters and connection defaults, persisted confidence overrides, and recall subgraphs with confidence floors.
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

When run from the Aver git repository, `install.sh` detects the checkout and installs from source. This installs the `aver` CLI to `~/.cargo/bin` by default.

To run `aver-server` as a systemd user service (unit in [`contrib/systemd/aver-mcp.service`](contrib/systemd/aver-mcp.service), binary at `~/.local/bin/aver-server`):

```bash
just install-service   # once: install + enable the aver-mcp user unit
just deploy-server     # after every change: release build, install, restart, probe
just health            # probe GET /healthz on the running server
```

### Install from a release

Tagged releases (`v*`) attach `aver-<os>-<arch>.tar.gz`, `aver-server-<os>-<arch>.tar.gz` (`linux`/`darwin`, `x86_64`/`aarch64`), `aver-mcp.service` and `SHA256SUMS`.

```bash
V=v0.1.0; R=https://github.com/5queezer/aver/releases/download/$V   # set 5queezer/aver
curl -fLO $R/aver-server-linux-x86_64.tar.gz -O $R/aver-mcp.service -O $R/SHA256SUMS
sha256sum -c --ignore-missing SHA256SUMS
tar -xzf aver-server-linux-x86_64.tar.gz
install -Dm755 aver-server ~/.local/bin/aver-server
install -Dm644 aver-mcp.service ~/.config/systemd/user/aver-mcp.service
systemctl --user daemon-reload && systemctl --user enable --now aver-mcp
curl -fsS http://127.0.0.1:3317/healthz
```

Or let the installer verify checksums for you: `./install.sh --repo 5queezer/aver --version $V --server` installs `aver` to `~/.cargo/bin` and `aver-server` to `~/.local/bin` (`--server-dir` to change).

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
| `triage` | Let the opt-in ADR-0024 decision model promote, reject, or hold a candidate claim. |
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
| `replay` | Rebuild SQLite from the append-only logs (strict by default; `--lenient` quarantines invalid lines and continues). Holds the advisory `.lock` for the whole run so it cannot race a live store. |

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
| `AVER_DECISION_MODEL` | *(unset)* | Ollama decision model (for example `tev1:4b`); setting it enables the ADR-0024 decision-model features. |
| `AVER_DECISION_URL` | `http://localhost:11434` | Ollama base URL for the decision model. |
| `AVER_DECISION_MIN_CONFIDENCE` | `0.6` | Confidence floor for decision-model choices; below it, or when the model reports no confidence, the deterministic behaviour applies (triage holds, suggestions fall back). |
| `AVER_DECISION_TIMEOUT_SECS` | `30` | Timeout in whole seconds for one decision-model request; on timeout the deterministic behaviour applies. Zero or unparsable values are ignored. |


Useful endpoints:

- `GET /.well-known/oauth-authorization-server`
- `POST /oauth/register`
- `GET /oauth/authorize` (browser consent screen; loopback by default, optional trusted-header for non-loopback)
- `POST /oauth/authorize/decision` (consent-screen form submission)
- `POST /oauth/consent/revoke` (browser-session form; revokes a client's consent and its live tokens, after which `/oauth/authorize` re-renders the consent screen)
- `POST /oauth/token` for authorization-code + PKCE token exchange and refresh-token grants (RFC 6749 §5.2 JSON error bodies)
- `GET /healthz` (unauthenticated liveness probe)
- `GET /api/health` with `Authorization: Bearer <token>`
- `/mcp` with `Authorization: Bearer <token>`

`/oauth/authorize` drives a browser consent flow (ADR-0020). After a client dynamic-registers via `POST /oauth/register`, it redirects the user to `/oauth/authorize` with the standard PKCE parameters. Aver renders a consent screen showing the client name, redirect URI, and all supported scopes as checkboxes, with the client's requested scopes pre-selected; on **Approve** it stores a per-client consent row, mints an authorization code bound to the checked scopes, and redirects back to the client's `redirect_uri` with `code` and `state`. Approving with zero scopes records an empty grant that never skips the consent screen, so a later visit can grant real access. The client then exchanges the code at `/oauth/token` for an `access_token` plus `refresh_token` (and an `expires_in` lifetime); refresh grants rotate both halves of the token pair, presenting an already-rotated refresh token revokes the client's whole token family (RFC 6819 §5.2.2.3 reuse detection), and access tokens carry only the scopes recorded on the consent row.

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

MCP currently exposes 19 tools through a progressive discovery card so agents keep the active choice set small:

- **Default active set:** `recall`, `remember_claim`, `record_event`, `record_observation`, `assemble_compaction_summary`
- **Event-to-claim workflow:** progressively load `should_extract_memories`, `propose_candidate_claim`, `list_candidate_claims`, `promote_candidate_claim`, `reject_candidate_claim` only when converting raw events into reviewed durable claims
- **Decision-model triage:** `triage_candidate_claim` (ADR-0024, opt-in) asks the local decision model to promote, reject, or hold one candidate
- **Graph navigation:** progressively load `expand` after recall returns an entity or an anchor is already known; use `add_triple` instead of `remember_claim` only when explicit source/confidence control is required
- **Observation audit:** progressively load `recall_observation`, `observation_coverage` for handoff, compaction, or provenance checks
- **Maintenance/repair:** keep `contradict`, `retire_claim`, `consolidate`, `add_vector_chunk` hidden until there is an explicit repair or retrieval-tuning need

Setting `AVER_DECISION_MODEL` opts the server and CLI into the ADR-0024 decision model (`aver-server` and `aver-cli` enable `aver-core`'s `ollama` feature by default; build with `--no-default-features` to leave it out, in which case the variable is ignored). `triage_candidate_claim` (CLI: `triage --candidate-id N`) then lets the model promote, reject, or hold a pending candidate; promoted claims carry `INFERRED` provenance with confidence 0.45, and without the variable the tool returns an error. The opt-in also lets `should_extract_memories` ask the model when only undigested events remain (the MCP result then adds `reasons`). `Store::suggest_scope` and `Store::normalize_predicate` exist in `aver-core` but are not wired to the MCP surface yet: the propose log line cannot carry the decision attribution, and error paths must not call the model under the tools mutex. Unset, every existing tool behaves as before.

Aver's MCP guide is intentionally proactive but selective: agents should recall first, then record durable user-shared preferences, project facts, and long-lived working context even when the user does not say "remember this" explicitly. Identity details should be recorded only when they are necessary, user-shared, and not sensitive personal data. When durability is uncertain, agents should prefer `record_event` over `remember_claim`, and they must not store secrets, credentials, sensitive personal data, transient chat, or facts they cannot explain with provenance.

Scope is a first-class memory dimension. Write tools such as `remember_claim`, `add_triple`, `record_event`, `record_observation`, and `propose_candidate_claim` accept an optional `scope`; when omitted through MCP, the request's ADR-0022 resolved scope applies. Clients that send no per-call or connection scope signal use `AVER_DEFAULT_SCOPE` when configured, then fall back to `global`. Read tools (`recall`, `expand`) accept `scope` plus `scope_walk`: use `ancestors` for normal project memory, `exact` for isolation, `descendants` for subtree audits, and `any` only for intentional cross-scope search. Vector chunks do not carry their own scope; they attach to a claim and inherit the claim's effective scope.

CLI-only continuity and maintenance surfaces (`catch-up`, `compaction-summary`) are implemented in `aver-cli`; MCP exposes `record_observation`, `assemble_compaction_summary`, and the observation audit tools above, while claim-maintenance tasks stay available through the four advanced tools when agents explicitly need them.

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

The eval crate also exposes deterministic data structures for ADR-0012 query-suite regression threshold checks, hallucination-rate memory-on/off reports, and typed prompt contracts; aver-core adds graph-stat drift snapshots with privacy-rejection counters. Prompt contracts validate rendered prompt text before a model call using deterministic checks such as required text, forbidden text, required sections, unresolved-template detection, and character budgets. These checks validate prompt generation code; live judge/provider integrations and output-quality evals should remain separate and feed recorded case results into the eval structures.

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
aver/
├── crates/
│   ├── aver-core/       # Store, claims, events, privacy filter, vectors, recall, consolidation
│   ├── aver-cli/        # `aver` command-line interface
│   ├── aver-extractor/  # Tree-sitter (13 languages) and prose fact extraction
│   ├── aver-server/     # MCP/OAuth HTTP server
│   ├── aver-eval/       # Fixture and BEAM100K evaluation runners
│   └── aver-scope-shim/ # DEPRECATED (ADR-0025): X-Aver-Scope proxy; set the header in MCP client config
├── doc/
│   ├── adr/             # Architecture decision records
│   └── how-it-works.md  # Current implementation walkthrough
├── migrations/          # Embedded SQLite migrations (0001-0008 + 0009 baseline)
├── install.sh           # Source/release installer
└── justfile             # Development automation
```

`crates/aver-core/migrations-legacy/` holds the pre-baseline chain (0009-0119), applied only when upgrading stores created before the baseline.

## Development

Common commands:

```bash
just build       # cargo build --workspace --locked
just test        # cargo test --workspace --locked
just fmt         # cargo fmt --all
just clippy      # cargo clippy --workspace --no-deps -- -D warnings
just check       # format check + clippy + tests + policy checks
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
- Tree-sitter extraction across 13 languages (C, C++, C#, Go, Java, JavaScript, Kotlin, PHP, Python, Ruby, Rust, Swift, TypeScript),
- structured prose fact parsing,
- MCP/OAuth server with ADR-0008 recall/expand/add-triple/contradict/consolidate tools, staged candidate-claim workflow, and observation recall/compaction-summary tools,
- ADR-0020 browser consent flow for `/oauth/authorize` (loopback Profile A) replacing the legacy `approval_token` gate,
- fixture and BEAM100K evaluation runners.

Partial or planned:

- production vector-index operations beyond the current bundled `sqlite-vec`/`vec0` local ANN path and JSON fallback metadata,
- production shared-graph backend adapter beyond the current local weighted community detection,
- broader production packaging, signed releases, and release automation beyond the current installer/`just dist` workflow.

## Documentation

- [`doc/how-it-works.md`](doc/how-it-works.md) — current runtime flow and ADR mapping.
- [`doc/adr/`](doc/adr/) — architecture decisions.

## License

MIT, as declared in the workspace package metadata.
