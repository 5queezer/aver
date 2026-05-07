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
