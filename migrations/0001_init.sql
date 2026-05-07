-- ADR-0006: claims table is the graph; provenance/confidence per ADR-0003.
CREATE TABLE IF NOT EXISTS claims (
  id               INTEGER PRIMARY KEY,
  subject          TEXT    NOT NULL,
  predicate        TEXT    NOT NULL,
  object           TEXT    NOT NULL,
  provenance       TEXT    NOT NULL CHECK (provenance IN ('USER_ASSERTED','EXTRACTED','INFERRED','AMBIGUOUS')),
  confidence       REAL    NOT NULL,
  status           TEXT    NOT NULL DEFAULT 'ACTIVE',
  source_refs      TEXT    NOT NULL,                 -- JSON array
  created_at       INTEGER NOT NULL,
  last_seen_at     INTEGER NOT NULL,
  last_verified_at INTEGER
);

CREATE INDEX IF NOT EXISTS claims_spo
  ON claims(subject, predicate, object);

CREATE INDEX IF NOT EXISTS claims_object_predicate
  ON claims(object, predicate);
