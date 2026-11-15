-- Rejected candidate claims need an auditable rejection rationale. The Rust
-- Store::reject_candidate_claim path validates nonblank reasons; direct SQL
-- writes should preserve that workflow invariant.
CREATE TRIGGER IF NOT EXISTS candidate_claims_rejection_reason_insert
BEFORE INSERT ON candidate_claims
WHEN NEW.status = 'REJECTED'
 AND (NEW.rejection_reason IS NULL OR trim(NEW.rejection_reason) = '')
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.rejection_reason must not be blank when status is REJECTED');
END;

CREATE TRIGGER IF NOT EXISTS candidate_claims_rejection_reason_update
BEFORE UPDATE OF status, rejection_reason ON candidate_claims
WHEN NEW.status = 'REJECTED'
 AND (NEW.rejection_reason IS NULL OR trim(NEW.rejection_reason) = '')
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.rejection_reason must not be blank when status is REJECTED');
END;
