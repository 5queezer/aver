-- ADR-0010: typed graph entities projected from claim subjects/objects.
CREATE TABLE IF NOT EXISTS entities (
    name         TEXT PRIMARY KEY,
    type_id      INTEGER NOT NULL REFERENCES entity_types(id),
    created_at   INTEGER NOT NULL,
    last_seen_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS entities_type_id ON entities(type_id);
