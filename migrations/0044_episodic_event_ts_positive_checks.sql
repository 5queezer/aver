-- Episodic event timestamps come from wall-clock Unix seconds in the Store
-- recording path. Reject non-positive sentinel timestamps from direct SQL so
-- event ordering and extraction thresholds remain meaningful.
CREATE TRIGGER IF NOT EXISTS episodic_events_ts_positive_insert
BEFORE INSERT ON episodic_events
WHEN NEW.ts <= 0
BEGIN
  SELECT RAISE(ABORT, 'episodic_events.ts must be positive');
END;

CREATE TRIGGER IF NOT EXISTS episodic_events_ts_positive_update
BEFORE UPDATE OF ts ON episodic_events
WHEN NEW.ts <= 0
BEGIN
  SELECT RAISE(ABORT, 'episodic_events.ts must be positive');
END;
