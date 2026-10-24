-- `last_verified_at` is optional, but when present it is written from the Store
-- clock for policy scoring and should not use sentinel or non-positive values.
CREATE TRIGGER IF NOT EXISTS claims_last_verified_at_positive_insert
BEFORE INSERT ON claims
WHEN NEW.last_verified_at IS NOT NULL
 AND NEW.last_verified_at <= 0
BEGIN
  SELECT RAISE(ABORT, 'claims.last_verified_at must be positive when set');
END;

CREATE TRIGGER IF NOT EXISTS claims_last_verified_at_positive_update
BEFORE UPDATE OF last_verified_at ON claims
WHEN NEW.last_verified_at IS NOT NULL
 AND NEW.last_verified_at <= 0
BEGIN
  SELECT RAISE(ABORT, 'claims.last_verified_at must be positive when set');
END;
