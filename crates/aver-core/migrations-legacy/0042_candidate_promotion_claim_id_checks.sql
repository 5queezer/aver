-- Promoted candidate claims must point at the durable claim created by the
-- promotion path. Store::promote_candidate_claim sets promoted_claim_id and
-- status together; direct SQL writes should preserve that workflow invariant.
CREATE TRIGGER IF NOT EXISTS candidate_claims_promotion_claim_id_insert
BEFORE INSERT ON candidate_claims
WHEN NEW.status = 'PROMOTED' AND NEW.promoted_claim_id IS NULL
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.promoted_claim_id must be set when status is PROMOTED');
END;

CREATE TRIGGER IF NOT EXISTS candidate_claims_promotion_claim_id_update
BEFORE UPDATE OF status, promoted_claim_id ON candidate_claims
WHEN NEW.status = 'PROMOTED' AND NEW.promoted_claim_id IS NULL
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.promoted_claim_id must be set when status is PROMOTED');
END;
