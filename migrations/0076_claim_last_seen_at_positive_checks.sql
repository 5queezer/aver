-- Durable claim timestamps are written from the Store clock. Reject sentinel or
-- non-positive last-seen times from direct SQL so claim chronology remains sane.
CREATE TRIGGER IF NOT EXISTS claims_last_seen_at_positive_insert
BEFORE INSERT ON claims
WHEN NEW.last_seen_at <= 0
BEGIN
  SELECT RAISE(ABORT, 'claims.last_seen_at must be positive');
END;

CREATE TRIGGER IF NOT EXISTS claims_last_seen_at_positive_update
BEFORE UPDATE OF last_seen_at ON claims
WHEN NEW.last_seen_at <= 0
BEGIN
  SELECT RAISE(ABORT, 'claims.last_seen_at must be positive');
END;
