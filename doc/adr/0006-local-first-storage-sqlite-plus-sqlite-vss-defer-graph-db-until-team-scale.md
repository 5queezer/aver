# 6. Local-first storage: SQLite plus sqlite-vss, defer graph DB until team-scale

Date: 2026-05-06

## Status

Accepted

## Context

The book catalogs candidates: Neo4j, Memgraph, TigerGraph, ArangoDB, Apache AGE, Weaviate. It explicitly states storage choice should be "a function of the wall you hit: speed, latency, data shape, infrastructure constraints. Not a feature checklist." [ch.105]

For this project the walls are:

- single developer, single machine,
- proprietary code in memory — must stay local,
- agent latency budget < 200ms per recall,
- graph size in 10⁴ – 10⁶ edges, not 10⁹,
- zero infra: no service to start, no port to manage.

Standing up a graph DB to hold ~50k edges per project is engineering theater.

## Decision

Storage is local-first and embedded:

- **SQLite** for episodic events, claims (triple table), entities, sources, contradictions. WAL mode for concurrent reads during writes.
- **`sqlite-vss`** (loaded as a SQLite extension) for embeddings — the **default** vector store. Baseline install must never require a daemon.
- **External Qdrant** is supported behind the same retrieval interface but only via explicit config opt-in (`MEMORY_VECTOR_BACKEND=qdrant`). Useful for sharing a single index across multiple users in `shared` mode (ADR-0011); not part of the local-first happy path.
- **JSONL files** in `memory/log/` as the append-only audit log (ADR-0005).

Triple table is the graph. Traversals are recursive CTEs. No graph DB.

Schema:

```sql
CREATE TABLE claims (
  id            INTEGER PRIMARY KEY,
  subject       TEXT NOT NULL,
  predicate     TEXT NOT NULL,
  object        TEXT NOT NULL,
  provenance    TEXT NOT NULL CHECK (provenance IN ('USER_ASSERTED','EXTRACTED','INFERRED','AMBIGUOUS')),
  confidence    REAL NOT NULL,
  status        TEXT NOT NULL DEFAULT 'ACTIVE',
  source_refs   TEXT NOT NULL,         -- JSON array
  created_at    INTEGER NOT NULL,
  last_seen_at  INTEGER NOT NULL,
  last_verified_at INTEGER
);
CREATE INDEX claims_spo ON claims(subject, predicate, object);
CREATE INDEX claims_object_predicate ON claims(object, predicate);   -- reverse lookup
```

**Migration path is the escape hatch, not the default.** If the project hits one of these walls, swap the storage layer:

- > 10⁶ edges or > 4-hop traversals → Memgraph (Cypher, in-RAM).
- multi-developer shared memory → Postgres + Apache AGE.
- need vector + graph in one engine → ArangoDB.

The triple shape is the same in all targets; only the storage adapter changes.

## Consequences

- (+) Zero infra. The agent's memory is a directory you can `tar` and rsync.
- (+) Encryptable at rest (ecryptfs / LUKS); no network surface.
- (+) Backup and rollback are file operations.
- (−) No native graph traversal; deep queries via recursive CTE are slower and uglier than Cypher.
- (−) Hard ceiling around ~10⁶ active edges before query latency becomes a problem.
- (−) Migration to a real graph DB is a non-zero project (storage adapter + traversal rewrite), even if the data shape carries over.

**Update (2026-05-10):** Superseded in part by ADR-0017, which replaces sqlite-vss with sqlite-vec for the vector index. The local-first SQLite commitment stands.
