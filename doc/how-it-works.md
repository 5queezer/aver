# How Aver Works

Aver is a local-first memory layer for coding agents. It keeps durable memories auditable by separating append-only source logs from replayable SQLite projections.

## Runtime flow

```text
Agent / CLI / MCP client
  -> input validation
  -> privacy filter
  -> append-only JSONL log
  -> SQLite projection
  -> recall through keyword, graph, vector, or hybrid retrieval
```

## Storage layout

A memory directory contains the local source of truth and projections:

```text
.aver/
├── log.jsonl           # durable claim and hyperedge audit log
├── events.jsonl        # episodic events
├── observations.jsonl  # continuity observations and prune markers
├── db.sqlite           # replayable query projection
└── auth.db             # local OAuth/session state for the MCP server
```

## Write path invariants

1. Validate structured fields.
2. Reject secrets, credential paths, and explicit `memory:ignore` content before persistence.
3. Append auditable records before updating SQLite projections.
4. Keep enough provenance to replay or inspect where each memory came from.

Vector chunk writes follow the same privacy boundary as claim, event, observation, and candidate writes.

## Main projections

- **Claims** — structured `(subject, predicate, object)` records with provenance, confidence, status, source references, agent attribution, and scope.
- **Hyperedges** — n-ary memories with role/entity participants for relationships that do not fit simple triples.
- **Episodic events** — raw session events used as extraction and compaction source material.
- **Observations** — privacy-checked continuity notes backed by source event IDs.
- **Vector chunks** — local embedding metadata connected back to claims.

## Server surface

`aver-server` exposes the memory layer over Streamable HTTP MCP. The server uses local OAuth-style registration, PKCE authorization-code exchange, bearer-token validation, scoped tool permissions, and a browser consent flow.

The default posture is localhost-first. Public or reverse-proxy deployments should set explicit CORS origins, protect any trusted identity header at the proxy boundary, and use HTTPS.

## Recovery and maintenance

SQLite tables are projections over append-only records. Maintenance paths include replay, consolidation, vacuum, observation catch-up, coverage reporting, and log rotation. ADRs under [`adr/`](adr/) describe the design trade-offs in more detail.
