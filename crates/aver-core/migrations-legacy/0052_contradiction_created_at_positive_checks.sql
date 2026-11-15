-- Contradiction timestamps come from wall-clock Unix seconds in the Store path.
-- Reject non-positive sentinel timestamps from direct SQL so contradiction audit
-- chronology remains meaningful.
CREATE TRIGGER IF NOT EXISTS contradictions_created_at_positive_insert
BEFORE INSERT ON contradictions
WHEN NEW.created_at <= 0
BEGIN
  SELECT RAISE(ABORT, 'contradictions.created_at must be positive');
END;

CREATE TRIGGER IF NOT EXISTS contradictions_created_at_positive_update
BEFORE UPDATE OF created_at ON contradictions
WHEN NEW.created_at <= 0
BEGIN
  SELECT RAISE(ABORT, 'contradictions.created_at must be positive');
END;
