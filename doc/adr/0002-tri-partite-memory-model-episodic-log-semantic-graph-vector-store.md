# 2. Tri-partite memory model: episodic log, semantic graph, vector store

Date: 2026-05-06

## Status

Accepted

## Context

A coding agent needs three distinct memory access patterns:

- chronological audit of session events (which tool was called, what file changed, what the user said),
- structured facts about the project (entities, relationships, decisions, preferences),
- fuzzy semantic recall over text (chunks, summaries, transcripts, comments).

Each has a different lifecycle, retention policy, and query shape. Collapsing them into a single store either bloats one with concerns it shouldn't carry, or forces awkward dual-purpose schemas. Both this project's first synthesis and an independent run of `pi` on the same source converged on a tri-partite split.

The book's hippocampus → neocortex framing supports this: episodic fragments are kept transiently and consolidated into durable structured memory between sessions [ch.147]. Vector retrieval is then the third, orthogonal channel for unstructured semantic recall.

## Decision

Implement memory as three separate stores:

1. **Episodic log** — append-only chronological events. JSONL file plus indexed SQLite mirror.
2. **Semantic graph** — durable, deduplicated claims as `(subject, predicate, object)` triples with provenance and confidence. Backed by SQLite tables (see ADR-0006).
3. **Vector store** — embeddings of chunks, node summaries, and rolling session digests for similarity search.

Each store has its own write path. The episodic log is the source of truth; the graph and vector store are derived projections regenerable from the log via the consolidation pass (ADR-0005).

## Consequences

- (+) Independent retention: episodic can age out, graph claims persist, vectors rebuild on demand.
- (+) Audit trail comes free — the episodic log is a complete record.
- (+) Failure isolation: a corrupt graph rebuilds from the log without losing history.
- (−) More moving parts than a single store; referential integrity (episode → claim → source) must be enforced in the consolidation pass.
- (−) Three indices to keep warm at query time for HybridRAG (ADR-0004).
