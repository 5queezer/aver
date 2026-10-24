-- Candidate claims are staged triples extracted from source events. The Rust
-- proposal path rejects blank triple fields; direct SQL staging should preserve
-- the same claim-shape invariant.
CREATE TRIGGER IF NOT EXISTS candidate_claims_predicate_nonblank_insert
BEFORE INSERT ON candidate_claims
WHEN trim(NEW.predicate) = ''
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.predicate must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS candidate_claims_predicate_nonblank_update
BEFORE UPDATE OF predicate ON candidate_claims
WHEN trim(NEW.predicate) = ''
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.predicate must not be blank');
END;
