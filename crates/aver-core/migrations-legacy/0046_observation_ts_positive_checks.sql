-- Observation timestamps come from wall-clock Unix seconds in the Store path.
-- Reject non-positive sentinel timestamps from direct SQL so observation
-- chronology and audit projections remain meaningful.
CREATE TRIGGER IF NOT EXISTS observations_ts_positive_insert
BEFORE INSERT ON observations
WHEN NEW.ts <= 0
BEGIN
  SELECT RAISE(ABORT, 'observations.ts must be positive');
END;

CREATE TRIGGER IF NOT EXISTS observations_ts_positive_update
BEFORE UPDATE OF ts ON observations
WHEN NEW.ts <= 0
BEGIN
  SELECT RAISE(ABORT, 'observations.ts must be positive');
END;
