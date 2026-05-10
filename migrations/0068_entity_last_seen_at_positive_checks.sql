-- Projected graph entity last-seen timestamps come from Store write/replay
-- paths. Reject non-positive sentinel timestamps from direct SQL so entity
-- recency and decay logic remains meaningful.
CREATE TRIGGER IF NOT EXISTS entities_last_seen_at_positive_insert
BEFORE INSERT ON entities
WHEN NEW.last_seen_at <= 0
BEGIN
  SELECT RAISE(ABORT, 'entities.last_seen_at must be positive');
END;

CREATE TRIGGER IF NOT EXISTS entities_last_seen_at_positive_update
BEFORE UPDATE OF last_seen_at ON entities
WHEN NEW.last_seen_at <= 0
BEGIN
  SELECT RAISE(ABORT, 'entities.last_seen_at must be positive');
END;
