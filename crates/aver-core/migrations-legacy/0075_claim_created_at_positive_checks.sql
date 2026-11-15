-- Durable claim timestamps are written from the Store clock. Reject sentinel or
-- non-positive creation times from direct SQL so claim chronology remains sane.
CREATE TRIGGER IF NOT EXISTS claims_created_at_positive_insert
BEFORE INSERT ON claims
WHEN NEW.created_at <= 0
BEGIN
  SELECT RAISE(ABORT, 'claims.created_at must be positive');
END;

CREATE TRIGGER IF NOT EXISTS claims_created_at_positive_update
BEFORE UPDATE OF created_at ON claims
WHEN NEW.created_at <= 0
BEGIN
  SELECT RAISE(ABORT, 'claims.created_at must be positive');
END;
