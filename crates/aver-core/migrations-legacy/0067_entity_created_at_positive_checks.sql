-- Projected graph entity timestamps come from Store write/replay paths. Reject
-- non-positive sentinel creation timestamps from direct SQL so entity audit
-- chronology remains meaningful.
CREATE TRIGGER IF NOT EXISTS entities_created_at_positive_insert
BEFORE INSERT ON entities
WHEN NEW.created_at <= 0
BEGIN
  SELECT RAISE(ABORT, 'entities.created_at must be positive');
END;

CREATE TRIGGER IF NOT EXISTS entities_created_at_positive_update
BEFORE UPDATE OF created_at ON entities
WHEN NEW.created_at <= 0
BEGIN
  SELECT RAISE(ABORT, 'entities.created_at must be positive');
END;
