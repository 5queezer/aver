-- Rejected candidates represent discarded staged claims, not promoted durable
-- claims. Store::reject_candidate_claim leaves promoted_claim_id unset; direct
-- SQL writes should preserve the mutually-exclusive workflow states.
CREATE TRIGGER IF NOT EXISTS candidate_claims_rejected_no_promoted_claim_insert
BEFORE INSERT ON candidate_claims
WHEN NEW.status = 'REJECTED' AND NEW.promoted_claim_id IS NOT NULL
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.promoted_claim_id must be NULL when status is REJECTED');
END;

CREATE TRIGGER IF NOT EXISTS candidate_claims_rejected_no_promoted_claim_update
BEFORE UPDATE OF status, promoted_claim_id ON candidate_claims
WHEN NEW.status = 'REJECTED' AND NEW.promoted_claim_id IS NOT NULL
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.promoted_claim_id must be NULL when status is REJECTED');
END;
