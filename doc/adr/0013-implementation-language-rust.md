# 13. Implementation language: Rust

Date: 2026-05-07

## Status

Accepted

## Context

The architectural ADRs (0002–0012) commit to:

- a single binary distributed to single-developer machines,
- embedded SQLite + `sqlite-vss` as the default storage (ADR-0006),
- native Tree-sitter for code extraction (ADR-0007),
- a privacy-critical pre-write filter (ADR-0009),
- recall budget under 200ms; cold-start under ~50ms (ADR-0004),
- append-only logs with deterministic replay (ADR-0005),
- correctness-sensitive consolidation logic.

Language candidates evaluated: Rust, Go, Zig, C++, OCaml, Nim. An independent review by `pi` (running this project's own `pdf-rag` extension over the ADRs) reached the same conclusion.

## Decision

**Rust** is the single primary implementation language. One language across:

- CLI front-end,
- background consolidation worker,
- SQLite storage adapter,
- JSONL append-only writer,
- pre-write secret filter,
- Tree-sitter code extractors,
- HybridRAG retrieval,
- HTTP client to local embedding/LLM providers (Ollama),
- project config and migrations.

### Why Rust over Go (the only serious challenger)

| Concern | Rust | Go |
|---|---|---|
| `sqlite-vss` extension loading | mature `rusqlite` + extension API | CGO friction; vector extensions are second-class |
| Tree-sitter ergonomics | first-class; many published grammar crates | bindings exist, less idiomatic |
| Binary size (stripped, LTO) | 3–25MB depending on deps | 8–25MB |
| Cold-start latency | excellent | excellent |
| Privacy-critical filter correctness | strong type system; lifetimes catch boundary bugs | good but weaker invariants |
| Replay/consolidation invariants | enums + exhaustive match | runtime checks |

Go would win if the project were "HTTP daemon with pure-Go SQLite." It isn't. Tree-sitter and `sqlite-vss` are first-class architectural pieces.

### Why not the others

- **Zig**: smallest binaries, but the SQLite/Tree-sitter/HTTP/JSON/migration ecosystem isn't there yet. Would force the project to build its own toolchain.
- **C++**: technically capable; permanent maintenance tax for security-critical code (lifetimes, build systems, packaging). Wrong cost curve.
- **OCaml / Nim**: niche risk on every dependency boundary. Not worth the contributor-pool penalty.

### One permitted boundary

A non-Rust *plugin* process is acceptable for experimental prose extraction or specialized document parsing — but it never writes to memory directly. The Rust core validates and appends. Plugin protocol: stdin/stdout JSON-RPC, no shared filesystem state.

### Build profile

Release profile (`Cargo.toml`):

```toml
[profile.release]
opt-level = "z"        # size before raw speed; recall is I/O-bound
lto = "fat"
codegen-units = 1
strip = true
panic = "abort"
```

TLS via `rustls` (no system OpenSSL). HTTP via `ureq` for the CLI, `reqwest` only inside the worker if async is unavoidable. Default to synchronous I/O on the hot path; consolidation can use threads.

### Amendments to prior ADRs

This ADR adjusts three earlier ADRs so they stay internally consistent:

- **ADR-0007**: drop `py-tree-sitter`; use the `tree-sitter` crate plus bundled grammar crates.
- **ADR-0006**: `sqlite-vss` is the default vector store; Qdrant only via explicit opt-in.
- **ADR-0004**: "prepared at query time" replaces "warm" — meaning open WAL, cached prepared statements, loaded extension; not eager hydration.

Those ADRs have been edited in place to reflect this.

## Consequences

- (+) Single language, single binary, single distribution story. `cargo install memory` is the install.
- (+) Type system catches the kinds of bugs that bite hardest in a privacy-sensitive append-only system.
- (+) Mature ecosystem fit for the exact dependencies that drive this design (SQLite extensions, Tree-sitter, structured logging).
- (+) Internally consistent with the small/fast/local-first ADRs.
- (−) Higher learning curve and longer compile times than Go. CI must cache `target/`.
- (−) Local in-process embedding inference (ONNX) is weaker than Python's. Mitigated by HTTP to local Ollama as the default; in-process is an opt-in.
- (−) Async-vs-sync split (sync hot path, async only in worker) is an architectural commitment that needs to be enforced in code review.
- (−) Contributor pool is smaller than Go's. Not a blocker for a single-developer tool.
