# 17. Vector index activation

Date: 2026-05-10

## Status

Accepted (2026-05-10). Supersedes the vector-index portion of ADR-0006; the SQLite-as-relational-store and JSONL-log decisions in ADR-0006 stand.

## Context

ADR-0006 chose `sqlite-vss` as the local-first vector store. ADR-0004 made vector similarity one half of the HybridRAG blend `α · vector + (1 − α) · graph`. The book treats ANN over embeddings as table stakes for a memory layer: "without a vector index, you do not have a HybridRAG system; you have a graph with a slow text search bolted on" [ch.99-100, ch.146].

Historical implementation gap, now closed: early code stored embeddings only as opaque JSON in `vector_chunks`, kept a dormant `sqlite-vss` helper, and made `recall_hybrid_claims_with_alpha` full-scan `vector_chunks` while parsing every `embedding_json` blob and computing cosine similarity in Rust.

Current implementation status: `migrations/0010_vector_index.sql` creates a `sqlite-vec`/`vec0` virtual table named `vector_index` with the canonical `VECTOR_INDEX_DIM`; `Store::add_vector_chunk_with_embedding` writes matching-dimension embeddings to `vector_chunks` and `vector_index`; replay/open paths register the statically linked `sqlite-vec` extension before migrations; hybrid vector recall queries `vector_index` when available and falls back to the JSON full-scan path when the ANN table is missing or the query dimension does not match. The JSON column remains the durable rebuild source; the `vec0` table is a replayable projection.

A second problem has accrued since ADR-0006: `sqlite-vss` is effectively abandoned. Its last release was January 2024. The author, Alex Garcia, redirected effort to `sqlite-vec`, a from-scratch successor. Building on an abandoned dependency for a load-bearing component is a known liability.

This ADR picks a vector index, defines how it is populated and recovered, and unblocks ADR-0004's α-blend from being a lie.

## Decision

### Options considered

| Option | Distribution | Index type | Maintenance | Notes |
|---|---|---|---|---|
| `sqlite-vss` (ADR-0006) | runtime-load `.so`/`.dylib` | Faiss-backed IVF | abandoned; last release 2024-01 | Heavy binary (Faiss + BLAS). Author moved to `sqlite-vec`. |
| `sqlite-vec` | runtime-load OR static-link | brute force today; IVF/HNSW on roadmap | actively maintained; same author | Pure C, no Faiss/BLAS dep. Single-file extension (~500KB). MIT. Embeddable in the Rust binary via `bundled` feature. |
| Native Rust HNSW (e.g. `instant-distance`, `hnsw_rs`) | in-process; sidecar file | HNSW | maintained but external to SQLite | No SQL surface; index file lives next to the DB. Recovery requires custom rebuild path. |
| Status quo (full-scan in Rust) | none | none | n/a | Baseline. Violates ADR-0004 latency budget past ~10⁴ chunks. |

### Recommendation: `sqlite-vec`, statically linked

Adopt `sqlite-vec` as the default vector index. Static-link the extension into the `aver-core` binary via the crate's `bundled` build option (matches how `rusqlite` ships SQLite itself in the project). Wire `prepare_sqlite_vec_index` into the migration runner so the virtual table exists on first open.

Why `sqlite-vec` over the alternatives:

- **Maintenance.** `sqlite-vss` is abandoned; ADR-0006's choice is no longer defensible on upstream-health grounds alone. `sqlite-vec` is the same author's active replacement.
- **Distribution.** `sqlite-vec` is pure C with no Faiss/BLAS dependency. Static linking produces a single binary. ADR-0013 (Rust CLI) requires `cargo install` to "just work" on Linux/macOS without asking the user to fetch a `.so`; `sqlite-vss` makes that hard, `sqlite-vec` makes it easy.
- **SQL surface.** Same idiom as `sqlite-vss`: `CREATE VIRTUAL TABLE … USING vec0(embedding float[N])`, `MATCH` clause for KNN. The retrieval rewrite is small.
- **Recall path stays in SQL.** A native-Rust sidecar HNSW (`instant-distance`) is faster per-query but moves the index out of the database. Two stores means two recovery paths, two backup stories, and a divergence risk between `claims.id` and the sidecar's internal IDs. ADR-0006's "the agent's memory is a directory you can `tar` and rsync" is worth more than 2× query speed at the scale this layer targets.
- **Brute force is enough for v1.** `sqlite-vec`'s current index is brute-force scan with SIMD. At ~10⁵ chunks and 768 dims that is ~5-15ms per query on a laptop — inside ADR-0004's 200ms budget with room to spare. IVF/HNSW arrives upstream before this project hits the scale where brute force fails.

Rejected: native Rust HNSW. Faster, but the operational story (separate index file, custom rebuild on schema change, no SQL introspection) is wrong for a single-developer local-first tool. Revisit if ADR-0011 shared-graph mode produces a corpus where SIMD brute force misses the latency budget.

Rejected: keeping `sqlite-vss`. Abandonment alone is sufficient. The Faiss/BLAS dependency chain is a secondary reason — packaging a >20MB extension on every platform is a tax for no gain.

Rejected: status quo. The book is explicit [ch.146]: "the blend is the architecture; if one half is degenerate, the other half overcompensates and you get a worse retriever than either alone."

### Schema

Implemented as `migrations/0010_vector_index.sql`:

```sql
-- ADR-0017: ANN index over vector_chunks.
-- Dimensions are model-bound; see vector_chunks.embedding_model.
-- The virtual table holds only embeddings; metadata stays in vector_chunks.
CREATE VIRTUAL TABLE IF NOT EXISTS vector_index USING vec0(
  chunk_id  INTEGER PRIMARY KEY,
  embedding float[<DIM>]
);
```

`<DIM>` is resolved at migration time from a single canonical embedding model per database. The column `vector_chunks.embedding_model` records which model produced each row; mixing dimensions in one virtual table is a `sqlite-vec` error. See "Dimension binding" below.

The existing `vector_chunks.embedding_json` column stays as the source of truth — the virtual table is rebuildable from it. JSON is verbose but it is the recovery anchor.

### Dimension binding

A `vec0` virtual table fixes the dimension at creation. The system must therefore decide which model owns the index.

- One database, one embedding model. Mixing `text-embedding-3-small` (1536) and `nomic-embed-text` (768) in the same `vector_index` is impossible.
- Store the canonical model name and dimension in a new `meta` row (`embedding_model`, `embedding_dim`) at first migration. Subsequent inserts that disagree are rejected by the write path with `Error::EmbeddingDimensionMismatch`.
- Switching models is a re-index operation: drop `vector_index`, re-run `prepare_sqlite_vec_index(new_dim)`, repopulate from `vector_chunks` with new embeddings. This is a maintenance command, not a silent migration.

Initial canonical model: whatever `vector::EmbeddingClient` defaults to today. Confirm before merging — the ADR records the binding, not the choice of model.

### Populate strategy

Migration `0003` runs in two phases:

1. **Schema phase.** `CREATE VIRTUAL TABLE vector_index …` only. Idempotent.
2. **Backfill phase.** For each row in `vector_chunks` where `embedding_json IS NOT NULL`, parse and `INSERT INTO vector_index(chunk_id, embedding) VALUES (?, ?)`. Skip rows whose dimension does not match the canonical dim — log a counter `memory.index.backfill_skipped{reason="dim_mismatch"}` per ADR-0009's no-content telemetry rule.

Backfill is wrapped in a transaction. On a fresh install with zero rows it is a no-op. On an existing project the migration runner reports the row count and elapsed time but does not block startup beyond a sane ceiling (e.g., 10s) — past that, it commits the partial index and surfaces a `recall warm-up incomplete` warning. Subsequent recalls fall back to the existing `embedding_json` full-scan path for any `claim_id` not yet in `vector_index`. This keeps cold-start bounded and degrades gracefully.

Steady-state inserts: every `INSERT INTO vector_chunks` is paired with `INSERT INTO vector_index` in the same transaction. A trigger is the obvious mechanism but `vec0` virtual tables have constraints around triggers; if those bite, do the second insert in the Rust write path explicitly.

### Recovery

The virtual table is a derived index. Treat it as throwaway.

- If `vector_index` is missing or corrupt, drop and rebuild from `vector_chunks.embedding_json`. A CLI subcommand `aver index rebuild` performs this.
- If a `vector_chunks` row exists with no matching `vector_index` row (detected by a startup integrity probe), backfill that row.
- If a `vector_index` row references a `chunk_id` that no longer exists in `vector_chunks` (claim deletion, status change), prune on next startup. ADR-0006's `claims.status` already tracks soft deletes; the index does not need its own tombstone.
- The episodic log (ADR-0005) is the ultimate recovery floor. If both `claims` and `vector_chunks` are lost, replay rebuilds both. The vector index is rebuilt from `vector_chunks` after replay.

`claim_id` is not stored in `vector_index` directly — `chunk_id` is the link, with `vector_chunks.claim_id` providing the bridge. This avoids a foreign-key relationship that `vec0` cannot enforce anyway.

### Retrieval rewrite

Replace the full-scan in `recall_hybrid_claims_with_alpha` (`crates/aver-core/src/lib.rs:1810-1830`) with:

```sql
SELECT vc.claim_id, vi.distance
  FROM vector_index vi
  JOIN vector_chunks vc ON vc.id = vi.chunk_id
 WHERE vi.embedding MATCH ?1
   AND k = ?2
 ORDER BY vi.distance;
```

`?1` is the query embedding (passed as a `float[N]` blob), `?2` is `top_k * fanout`. The fanout factor (e.g., 4) gives the graph half of HybridRAG room to re-rank without starving on vector-only candidates. Distances are converted to similarity in Rust (`1 − distance` for L2-normalised embeddings).

The fallback path stays: if `vector_index_table_exists()` returns false (no extension at runtime — a possibility ADR-0006 contemplated for sandboxed builds), `recall_hybrid_claims_with_alpha` reverts to the current full-scan. The full-scan code is not deleted; it becomes the ANN-unavailable branch.

### Platform and packaging

`sqlite-vec` ships as a single C source file under MIT. The Rust crate `sqlite-vec` (or `sqlite-vec-sys`) wraps it. Use the `bundled` feature so the extension is compiled into `aver-core` — no user-side `.so`/`.dylib` provisioning. Per ADR-0013, `cargo install aver` must work on a fresh Linux/macOS without out-of-band downloads.

Windows is out of scope for v0.x (consistent with ADR-0013). If a Windows build path is added, `sqlite-vec` compiles cleanly under MSVC; the extension story does not regress.

The build adds one C compilation step. Crate-level `cc` already participates in the build (rusqlite bundled), so the marginal cost is small.

## Consequences

- (+) ADR-0004's α-blend stops lying. Vector recall scales sub-linearly in chunk count instead of degrading linearly.
- (+) `sqlite-vec` is actively maintained; ADR-0006's vector choice is realigned with upstream reality.
- (+) Single binary distribution preserved. `cargo install aver` still produces a self-contained tool. ADR-0013's "no daemon, no port" stays true.
- (+) Recovery is a derived-index rebuild, not a divergence problem. `tar` of the `memory/` directory still round-trips.
- (+) The dormant `prepare_sqlite_vss_index` (`lib.rs:238`) is deleted, not left as dead code. ADR-0009's "no dead code" discipline applies.
- (+) Clear failure mode when the extension is unavailable: full-scan fallback with telemetry, not a silent miss.
- (−) Re-indexing on embedding-model change is a manual operation. There is no online migration. Documented, not automated.
- (−) Brute-force `vec0` is fast but not asymptotically optimal. At ~10⁶ chunks the latency budget tightens. Mitigated by `sqlite-vec`'s upstream IVF/HNSW roadmap; unmitigated risk if the project outgrows brute force before that lands.
- (−) Dimension binding makes the database model-locked. Switching from a 768-dim to a 3072-dim model requires a rebuild. Acceptable for a single-developer tool; revisit for ADR-0011 shared mode.
- (−) `sqlite-vec`'s `MATCH` syntax is non-standard SQL. Code paths that touch the vector index are extension-coupled and will not run against a vanilla `sqlite3` shell. Same constraint `sqlite-vss` had.
- (−) Migration `0003` does a backfill pass on existing installs. For a multi-year-old project, that is a one-time cost (bounded at 10s; degrades to lazy fill past that). Documented in the upgrade notes.
- (−) The trigger-vs-explicit-write decision for keeping `vector_chunks` and `vector_index` in sync needs a small spike before the implementation PR. If `vec0` constraints force explicit writes, every embedding insert path must be audited (currently one site; verify before merge).
- (−) Supersedes the vector-index portion of ADR-0006 and adds a build-time C dependency (`sqlite-vec`). ADR-0006's "no infra" claim is unchanged; the dependency is compiled in, not installed by the user.
