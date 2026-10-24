CREATE TABLE IF NOT EXISTS hyperedges (
  id          INTEGER PRIMARY KEY,
  predicate   TEXT    NOT NULL,
  provenance  TEXT    NOT NULL CHECK (provenance IN ('USER_ASSERTED','EXTRACTED','INFERRED','AMBIGUOUS')),
  confidence  REAL    NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
  source_refs TEXT    NOT NULL CHECK (json_valid(source_refs) AND json_type(source_refs) = 'array'),
  status      TEXT    NOT NULL DEFAULT 'ACTIVE' CHECK (status IN ('ACTIVE','SUPERSEDED','INVALIDATED')),
  created_at  INTEGER NOT NULL CHECK (created_at > 0),
  updated_at  INTEGER NOT NULL CHECK (updated_at > 0)
);

CREATE INDEX IF NOT EXISTS hyperedges_status_predicate
  ON hyperedges(status, predicate);

CREATE TABLE IF NOT EXISTS hyperedge_participants (
  id           INTEGER PRIMARY KEY,
  hyperedge_id INTEGER NOT NULL REFERENCES hyperedges(id) ON DELETE CASCADE,
  role         TEXT    NOT NULL CHECK (trim(role) <> ''),
  entity       TEXT    NOT NULL CHECK (trim(entity) <> '')
);

CREATE INDEX IF NOT EXISTS hyperedge_participants_entity
  ON hyperedge_participants(entity);

CREATE INDEX IF NOT EXISTS hyperedge_participants_hyperedge_id
  ON hyperedge_participants(hyperedge_id);
