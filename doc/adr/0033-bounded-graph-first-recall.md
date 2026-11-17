# 33. Bounded graph-first recall

Date: 2026-11-17

## Status

Proposed. Opt-in only. Hybrid recall (ADR-0004) stays the default and the fallback. Graph-first becomes a candidate for anything more only if the evaluation on real operating questions shows a clear win in answer quality, evidence coverage, tokens and latency.

## Context

Hybrid recall ranks claims by blending vector similarity with token overlap. For questions that name entities ("who owns PaymentGateway", "what do Billing and Search both depend on", "how many crates does Aver use") the answer usually sits in the graph next to those entities, but hybrid recall does not walk edges. It also has no cost limit an agent can read, and it returns no source events, so an agent that wants the rationale must make one `get_event` call per claim.

## Decision

`Store::recall_graph_first(query, filters, options, now)` in `aver-core`, exposed as `recall` with `mode: "graph_first"`.

1. **Anchors.** The anchors are entities (claim subjects and entity objects that pass the recall filters) at least half of whose own words appear in the query, after stopwords and route cue words are removed. An entity's words exclude the digit and acronym fragments that recall tokenization adds, so `stripe-api-v3` has three words (stripe, api, v3), not five. An entity whose matched tokens are a strict subset of another candidate's is dropped, so in "PaymentGateway" the word "payment" anchors PaymentGateway and not team_payments. Anchors are ranked by coverage, with at most 15 (`MAX_ANCHORS`).
2. **Bounded expansion.** Breadth-first from all anchors in rank order, up to `hops` (default 2), sharing one budget of 64 claims (`MAX_EDGES`). Every recall filter (scope walk, status, agent, predicate, confidence, context) is applied in SQL with a `LIMIT`, so a hub entity cannot pull its whole neighbourhood into memory.
3. **Deterministic routing.** Whole-word cues in a fixed order: count ("how many", count, number), temporal (when, latest, recent, last, ...), intersection (both, and, between, common, shared, only with two or more anchors), list (list, which, all, every, "what are"), otherwise single. The route picks the facts from the bounded set:
   - single: rank and keep `top_k`;
   - temporal: keep the claims that match the question's content words, newest first by `valid_from`, falling back to write time;
   - intersection: keep the claims that join two anchors directly or touch a node that two anchors reach;
   - list and count: keep the claims that touch the primary anchors (those tied for the best rank) and match the content words, up to 25 regardless of `top_k`; count also reports how many there are. Without the primary-anchor step, a type word such as "services" also anchors every `*-service` entity, and their claims inflate the count.
   When a narrowing step leaves nothing, the set before that step is used.
4. **Ranking.** The share of the question's content words (query words other than stopwords, route cues and anchor names) found in the claim's subject, object, predicate and the predicate's ancestors in the ontology, plus `0.5 / hop`. Through the ancestors, "depend" also finds `uses`, a child of `depends_on`. Words match when their stems are equal (one inflection, "ing", "ed", "es" or "s", and then a final "e" are stripped, so "using" matches "uses"), or when one stem is a prefix of the other and is at least three letters long. Opt-in recency (`w * exp(-age / tau)`, ADR-0004) is added only inside this bounded set, against the caller's explicit `now`.
5. **Evidence.** Source events are read only for the selected claims, and only when an event's scope equals its claim's scope. The response carries `facts` (claim id, anchor, hop, time), `evidence_ids` (claims, events), `budget` (anchors, hops, edges visited, candidates, selected, truncated) and `latency_ms`.
6. **Fallback.** If no anchor matches, or the expansion finds no edge, core returns the reason (`NoAnchor` or `NoEdges`). The server then runs the unchanged hybrid or keyword path, and `graph_first.fallback` says `no_anchor` or `no_edges`. A source event that cannot be found is skipped, and the recall does not fail.

Out of scope: alias or entity merges, learned routing, and automatic skill or rule generation.

## Consequences

- Without `mode`, recall is byte-for-byte unchanged.
- An answered graph-first recall never calls the embedding provider, so it works offline. Only its hybrid fallback embeds the query.
- Routing is a word list, not a model. It can choose the wrong route, but the same question always gets the same route, and the budget fields make the cost visible.
- Questions that name no stored entity always fall back. Graph-first does not replace hybrid recall for open questions.
- In the first evaluation (20 operating questions), opt-in recency won no question, and it lost intersection coverage by pushing old but relevant claims out of the top `k`. The temporal route already sorts by time. Recency stays opt-in and off by default.
