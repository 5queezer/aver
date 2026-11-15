-- Projected entity recency cannot precede creation. Enforce this for direct
-- SQL writes so graph recency, decay, and audit chronology stay coherent.
CREATE TRIGGER IF NOT EXISTS entities_seen_after_created_insert
BEFORE INSERT ON entities
WHEN NEW.created_at > 0 AND NEW.last_seen_at > 0 AND NEW.last_seen_at < NEW.created_at
BEGIN
  SELECT RAISE(ABORT, 'entities.last_seen_at must be >= created_at');
END;

CREATE TRIGGER IF NOT EXISTS entities_seen_after_created_update
BEFORE UPDATE OF created_at, last_seen_at ON entities
WHEN NEW.created_at > 0 AND NEW.last_seen_at > 0 AND NEW.last_seen_at < NEW.created_at
BEGIN
  SELECT RAISE(ABORT, 'entities.last_seen_at must be >= created_at');
END;
