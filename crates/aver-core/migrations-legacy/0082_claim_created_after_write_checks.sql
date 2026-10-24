-- A durable claim is appended to the audit log before its SQLite projection is
-- created. The projected creation timestamp must not predate the write time.
CREATE TRIGGER IF NOT EXISTS claims_created_after_write_insert
BEFORE INSERT ON claims
WHEN NEW.write_ts > 0
 AND NEW.created_at > 0
 AND NEW.created_at < NEW.write_ts
BEGIN
  SELECT RAISE(ABORT, 'claims.created_at must be >= write_ts');
END;

CREATE TRIGGER IF NOT EXISTS claims_created_after_write_update
BEFORE UPDATE OF write_ts, created_at ON claims
WHEN NEW.write_ts > 0
 AND NEW.created_at > 0
 AND NEW.created_at < NEW.write_ts
BEGIN
  SELECT RAISE(ABORT, 'claims.created_at must be >= write_ts');
END;
