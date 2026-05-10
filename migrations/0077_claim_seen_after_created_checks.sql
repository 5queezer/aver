-- Durable claim chronology should not move backwards. Guard with positivity so
-- earlier timestamp-specific triggers produce their more precise errors first.
CREATE TRIGGER IF NOT EXISTS claims_seen_after_created_insert
BEFORE INSERT ON claims
WHEN NEW.created_at > 0
 AND NEW.last_seen_at > 0
 AND NEW.last_seen_at < NEW.created_at
BEGIN
  SELECT RAISE(ABORT, 'claims.last_seen_at must be >= created_at');
END;

CREATE TRIGGER IF NOT EXISTS claims_seen_after_created_update
BEFORE UPDATE OF created_at, last_seen_at ON claims
WHEN NEW.created_at > 0
 AND NEW.last_seen_at > 0
 AND NEW.last_seen_at < NEW.created_at
BEGIN
  SELECT RAISE(ABORT, 'claims.last_seen_at must be >= created_at');
END;
