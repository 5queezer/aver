-- Durable claim triples must keep nonblank fields even for direct SQL updates.
-- Store::add_claim validates subject/predicate/object before log-first writes;
-- mirror the object invariant at the schema boundary.
CREATE TRIGGER IF NOT EXISTS claims_object_nonblank_insert
BEFORE INSERT ON claims
WHEN trim(NEW.object) = ''
BEGIN
  SELECT RAISE(ABORT, 'claims.object must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS claims_object_nonblank_update
BEFORE UPDATE OF object ON claims
WHEN trim(NEW.object) = ''
BEGIN
  SELECT RAISE(ABORT, 'claims.object must not be blank');
END;
