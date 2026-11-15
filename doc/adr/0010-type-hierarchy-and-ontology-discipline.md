# 10. Type hierarchy and ontology discipline

Date: 2026-05-07

## Status

Accepted

## Context

ADR-0007 emits triples with predicates as flat strings. That works for storage but not for *reasoning*. The book is emphatic about the difference:

> A database schema tells you the columns: `name`, `price`, `timestamp`. Useful, but thin. An ontology tells you the nouns and verbs of a universe. A `Product` is a `Thing`. A `Manufacturer` creates a `Product`. If a `Car` is a `Vehicle`, and an `ElectricCar` is a `Car`, then an `ElectricCar` is a `Vehicle`. The ontology doesn't just store data. It stores permission to reason. [ch.61]

Without a type hierarchy, the graph cannot answer "what calls anything in the auth subsystem?" without enumerating every member by hand. Without a predicate hierarchy, "depends_on" can't subsume "calls", "imports", "reads_config_from" — the agent has to OR a long list every time.

The book points to Noy & McGuinness's *Ontology Development 101* as the practical guide [ch.165]; this ADR commits to applying it.

## Decision

Two hierarchies, kept as project-versioned config (not agent-runtime mutable):

### Entity-type hierarchy

```
Thing
├── Asset
│   ├── File
│   ├── Module
│   └── Config
├── Symbol
│   ├── Function
│   ├── Class
│   └── Constant
├── Process
│   ├── Service
│   ├── Test
│   └── Job
├── Agent
│   ├── Human          ← User
│   └── Bot            ← Claude, Pi, etc.
└── Concept
    ├── Decision
    ├── Bug
    ├── Pref
    └── Constraint
```

Stored as a separate `entity_types` table with `(id, name, parent_id)`. Every entity in the graph carries `type_id`.

### Predicate hierarchy (`subPropertyOf`)

```
relates_to
├── depends_on
│   ├── calls
│   ├── imports
│   └── reads_config_from
├── owns
│   ├── authored
│   └── maintained
└── concerns
    ├── fixes
    ├── tests
    └── decides
```

Stored as `predicate_types(id, name, parent_id)`.

### Reasoner

Two helpers:

- `entity.is_a(type)` — walks parent_id chain on `entity_types`.
- `predicate.implies(other)` — walks parent_id on `predicate_types`.

Retrieval (ADR-0004) consults both: "what depends_on `auth_service`?" returns claims with predicate `calls`, `imports`, `reads_config_from` — not just literal `depends_on`.

### Bootstrapping

Start with ~10 root types and ~15 predicates. Evolve by promoting frequently-co-occurring patterns into hierarchy entries via the consolidation pass. Schema changes are committed to `ontology/` in the project repo and reviewed like code.

### Materialized closure

Transitive `is_a` and `implies` closures are precomputed and stored in `entity_type_closure(child_id, ancestor_id)` and `predicate_closure(child_id, ancestor_id)` tables. Recomputed when the ontology changes (rare). Avoids walking the hierarchy on every query.

## Consequences

- (+) Transitive inference enabled — the graph can reason over abstractions, not just match strings.
- (+) Predicate vocabulary stays small at the leaves; queries can use abstract predicates.
- (+) The ontology serves as living documentation of the project's conceptual model.
- (+) Closure tables make typed queries as cheap as flat-string queries.
- (−) Every entity write needs a type. The extractor (ADR-0007) must classify, and a fallback `Thing` type is required for unknown entities.
- (−) Ontology changes are non-trivial: closure tables must be rebuilt, existing entities re-typed.
- (−) Real risk of bikeshedding the hierarchy. Mitigation: small initial set, additions require an ADR or a labeled "ontology" PR.
- (−) The hierarchy is a coupling point — changing parent relationships invalidates cached closures and may shift retrieval behavior.
