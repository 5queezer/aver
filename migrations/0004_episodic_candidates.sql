CREATE TABLE IF NOT EXISTS episodic_events (
  id          INTEGER PRIMARY KEY,
  session_id  TEXT    NOT NULL,
  kind        TEXT    NOT NULL,
  payload     TEXT    NOT NULL,
  source      TEXT    NOT NULL,
  agent_id    TEXT    NOT NULL DEFAULT 'local',
  agent_kind  TEXT    NOT NULL DEFAULT 'HUMAN' CHECK (agent_kind IN ('HUMAN','LLM','DETERMINISTIC_PARSER','EXTERNAL_TOOL')),
  ts          INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS episodic_events_session_ts
  ON episodic_events(session_id, ts);

CREATE TABLE IF NOT EXISTS candidate_claims (
  id                INTEGER PRIMARY KEY,
  event_id          INTEGER NOT NULL REFERENCES episodic_events(id),
  subject           TEXT    NOT NULL,
  predicate         TEXT    NOT NULL,
  object            TEXT    NOT NULL,
  provenance        TEXT    NOT NULL DEFAULT 'INFERRED' CHECK (provenance IN ('USER_ASSERTED','EXTRACTED','INFERRED','AMBIGUOUS')),
  confidence        REAL    NOT NULL DEFAULT 0.45,
  status            TEXT    NOT NULL DEFAULT 'PENDING' CHECK (status IN ('PENDING','PROMOTED','REJECTED')),
  promoted_claim_id INTEGER REFERENCES claims(id),
  created_at        INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS candidate_claims_event_id
  ON candidate_claims(event_id);
