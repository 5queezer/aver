-- Promoted candidates point at durable claims, while rejection reasons belong
-- only to rejected candidates. Store::promote_candidate_claim does not set a
-- rejection reason; direct SQL writes should preserve exclusive states.
CREATE TRIGGER IF NOT EXISTS candidate_claims_promoted_no_rejection_reason_insert
BEFORE INSERT ON candidate_claims
WHEN NEW.status = 'PROMOTED' AND NEW.rejection_reason IS NOT NULL
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.rejection_reason must be NULL when status is PROMOTED');
END;

CREATE TRIGGER IF NOT EXISTS candidate_claims_promoted_no_rejection_reason_update
BEFORE UPDATE OF status, rejection_reason ON candidate_claims
WHEN NEW.status = 'PROMOTED' AND NEW.rejection_reason IS NOT NULL
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.rejection_reason must be NULL when status is PROMOTED');
END;
