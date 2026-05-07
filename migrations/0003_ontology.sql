-- ADR-0010: ontology entity type hierarchy.
CREATE TABLE IF NOT EXISTS entity_types (
    id        INTEGER PRIMARY KEY,
    name      TEXT NOT NULL UNIQUE,
    parent_id INTEGER REFERENCES entity_types(id)
);

CREATE TABLE IF NOT EXISTS predicate_types (
    id        INTEGER PRIMARY KEY,
    name      TEXT NOT NULL UNIQUE,
    parent_id INTEGER REFERENCES predicate_types(id)
);
