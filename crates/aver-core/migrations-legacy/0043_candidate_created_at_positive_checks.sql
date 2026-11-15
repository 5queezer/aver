-- Candidate claim timestamps are produced from wall-clock Unix seconds by the
-- Store path. Reject sentinel/non-positive timestamps from direct SQL so staged
-- claim ordering and audit views are meaningful.
CREATE TRIGGER IF NOT EXISTS candidate_claims_created_at_positive_insert
BEFORE INSERT ON candidate_claims
WHEN NEW.created_at <= 0
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.created_at must be positive');
END;

CREATE TRIGGER IF NOT EXISTS candidate_claims_created_at_positive_update
BEFORE UPDATE OF created_at ON candidate_claims
WHEN NEW.created_at <= 0
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.created_at must be positive');
END;
