CREATE TABLE IF NOT EXISTS observations (
  id               TEXT PRIMARY KEY,
  session_id       TEXT    NOT NULL,
  content          TEXT    NOT NULL,
  relevance        TEXT    NOT NULL CHECK (relevance IN ('low','medium','high','critical')),
  source_event_ids TEXT    NOT NULL,
  agent_id         TEXT    NOT NULL,
  agent_kind       TEXT    NOT NULL CHECK (agent_kind IN ('HUMAN','LLM','DETERMINISTIC_PARSER','EXTERNAL_TOOL')),
  derivation       TEXT    NOT NULL,
  ts               INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS observations_session_ts
  ON observations(session_id, ts);
