-- Durable claims are appended to the JSONL log before SQLite projection; keep the
-- projected write timestamp positive so replay/audit ordering cannot use zero.
CREATE TRIGGER IF NOT EXISTS claims_write_ts_positive_insert
BEFORE INSERT ON claims
WHEN NEW.write_ts <= 0
BEGIN
  SELECT RAISE(ABORT, 'claims.write_ts must be positive');
END;

CREATE TRIGGER IF NOT EXISTS claims_write_ts_positive_update
BEFORE UPDATE OF write_ts ON claims
WHEN NEW.write_ts <= 0
BEGIN
  SELECT RAISE(ABORT, 'claims.write_ts must be positive');
END;
