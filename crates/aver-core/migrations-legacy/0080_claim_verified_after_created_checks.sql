-- Optional verification timestamps should not predate the claim creation time.
-- Guard with positivity so timestamp-specific triggers emit precise errors first.
CREATE TRIGGER IF NOT EXISTS claims_verified_after_created_insert
BEFORE INSERT ON claims
WHEN NEW.created_at > 0
 AND NEW.last_verified_at IS NOT NULL
 AND NEW.last_verified_at > 0
 AND NEW.last_verified_at < NEW.created_at
BEGIN
  SELECT RAISE(ABORT, 'claims.last_verified_at must be >= created_at when set');
END;

CREATE TRIGGER IF NOT EXISTS claims_verified_after_created_update
BEFORE UPDATE OF created_at, last_verified_at ON claims
WHEN NEW.created_at > 0
 AND NEW.last_verified_at IS NOT NULL
 AND NEW.last_verified_at > 0
 AND NEW.last_verified_at < NEW.created_at
BEGIN
  SELECT RAISE(ABORT, 'claims.last_verified_at must be >= created_at when set');
END;
