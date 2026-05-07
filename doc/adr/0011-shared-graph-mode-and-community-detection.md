# 11. Shared-graph mode and community detection

Date: 2026-05-07

## Status

Accepted

## Context

ADR-0006 commits to single-developer local-first storage and frames multi-agent collaboration as a future migration. The book describes a stronger claim: a *shared* graph that multiple agents read and write produces emergent structure that no single agent could derive alone.

> Each agent writes its findings to the same shared knowledge graph. The graph becomes collective consciousness. Once the swarm finishes, emergent structure appears. Run Louvain community detection on the shared graph, and a "payments" module crystallizes — no human labeled it. A cluster of refund logic, ledger entries, and webhook handlers reveals itself because the agents' combined observations created dense internal connectivity. [ch.150]

This isn't just "scale up SQLite to a server." It's a different operating mode: concurrent writers, per-agent provenance, trust scoring across agents, and a community-detection step in the consolidation pipeline that surfaces structure no single observer planted.

This ADR makes shared mode an explicit, planned capability rather than a "we'll think about it later."

## Decision

### Two operating modes

The storage adapter (ADR-0006) supports two modes selected by config:

- **`local`**: single-user SQLite. Default. Zero infra.
- **`shared`**: Postgres + Apache AGE, *or* Memgraph. Same triple shape as local mode; only the adapter changes.

Migration from `local` → `shared` is a documented operation: dump triples + episodic log → bulk load into the shared backend. No data shape changes.

### Per-claim agent provenance

Every claim in `shared` mode carries:

```
agent_id:    stable identifier of the writer (human, Claude, Pi, custom bot)
agent_kind:  HUMAN | LLM | DETERMINISTIC_PARSER | EXTERNAL_TOOL
write_ts:    monotonic timestamp from the writer
```

This is additive to the provenance schema in ADR-0003.

### Trust weighting

Each agent has a trust score, computed as the rolling agreement rate between its claims and the consolidated truth (i.e. claims that survived consolidation as `EXTRACTED` or `USER_ASSERTED` after 7 days). Trust feeds into:

- confidence weighting at write time (`effective_confidence = base_confidence × agent_trust`),
- conflict resolution when two agents disagree (higher-trust agent wins by default; humans always win unless explicitly overridden).

Trust is bounded `[0.1, 1.0]` to prevent total exclusion of new agents.

### Community detection in consolidation

The consolidation pass (ADR-0005) gains a step:

1. After dedup and contradiction resolution,
2. Run Louvain (or Leiden, for stability) on the entity graph weighted by edge confidence,
3. Materialize each community as a `Community` entity (see ADR-0010 type hierarchy),
4. Attach `member_of` edges from constituent entities,
5. Generate a community summary via the prose extractor (ADR-0007), stored as a vector chunk and as a `description` predicate on the community node.

Communities are recomputed every consolidation cycle. Community IDs are stable across runs via seeded modularity (community names are derived from highest-PageRank member nodes).

### Concurrency control

In `shared` mode:

- Writes use optimistic concurrency: `(subject, predicate, object)` is a unique key with `last_write_ts`.
- Conflicting concurrent writes generate a `contradicts` edge (ADR-0003); they don't lock or fail.
- The episodic log is partitioned per `agent_id` to avoid append contention.

## Consequences

- (+) Multi-agent collaboration without bespoke merge logic — the graph is the merge protocol.
- (+) Emergent module/cluster discovery comes for free with consolidation.
- (+) Trust weighting naturally identifies hostile or noisy agents over time.
- (+) Same triple shape across modes means the migration is operational, not architectural.
- (−) Community detection is non-deterministic across runs unless seeded; downstream tools (UI, dashboards) must tolerate cluster ID changes.
- (−) Trust scoring has a cold-start problem: a new agent's claims are weighted by a default until enough have been consolidated. Default 0.5 — neither trusted nor excluded.
- (−) Community summaries via LLM are not free; budget consolidation passes accordingly.
- (−) Privacy posture (ADR-0009) gets harder: shared mode means one agent's secrets-filter bug leaks across the whole team. Filter must run *per writer*, before any cross-agent visibility.
- (−) Operational complexity jumps in `shared` mode: backups, migrations, access control, encryption-in-transit. Defer until a real second writer exists.
