CREATE TABLE IF NOT EXISTS observation_prune_markers (
  id                    TEXT PRIMARY KEY,
  session_id            TEXT    NOT NULL,
  pruned_observation_ids TEXT   NOT NULL,
  ts                    INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS observation_prune_markers_session_ts
  ON observation_prune_markers(session_id, ts);
