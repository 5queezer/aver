-- Pending candidates have not yet been promoted, so they should not carry a
-- durable claim reference. Store::propose_candidate_claim creates pending rows
-- without promoted_claim_id; direct SQL writes should preserve that state.
CREATE TRIGGER IF NOT EXISTS candidate_claims_pending_no_promoted_claim_insert
BEFORE INSERT ON candidate_claims
WHEN NEW.status = 'PENDING' AND NEW.promoted_claim_id IS NOT NULL
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.promoted_claim_id must be NULL when status is PENDING');
END;

CREATE TRIGGER IF NOT EXISTS candidate_claims_pending_no_promoted_claim_update
BEFORE UPDATE OF status, promoted_claim_id ON candidate_claims
WHEN NEW.status = 'PENDING' AND NEW.promoted_claim_id IS NOT NULL
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.promoted_claim_id must be NULL when status is PENDING');
END;
