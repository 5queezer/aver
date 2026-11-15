-- Candidate claims are staged triples extracted from source events. The Rust
-- proposal path rejects blank triple fields; direct SQL staging should preserve
-- the same claim-shape invariant.
CREATE TRIGGER IF NOT EXISTS candidate_claims_subject_nonblank_insert
BEFORE INSERT ON candidate_claims
WHEN trim(NEW.subject) = ''
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.subject must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS candidate_claims_subject_nonblank_update
BEFORE UPDATE OF subject ON candidate_claims
WHEN trim(NEW.subject) = ''
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.subject must not be blank');
END;
