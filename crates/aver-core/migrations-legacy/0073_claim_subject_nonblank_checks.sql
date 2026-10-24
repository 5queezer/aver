-- Durable claim triples must keep nonblank fields even for direct SQL updates.
-- Store::add_claim validates subject/predicate/object before log-first writes;
-- mirror the subject invariant at the schema boundary.
CREATE TRIGGER IF NOT EXISTS claims_subject_nonblank_insert
BEFORE INSERT ON claims
WHEN trim(NEW.subject) = ''
BEGIN
  SELECT RAISE(ABORT, 'claims.subject must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS claims_subject_nonblank_update
BEFORE UPDATE OF subject ON claims
WHEN trim(NEW.subject) = ''
BEGIN
  SELECT RAISE(ABORT, 'claims.subject must not be blank');
END;
