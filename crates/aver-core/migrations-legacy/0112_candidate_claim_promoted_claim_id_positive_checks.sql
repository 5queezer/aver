CREATE TRIGGER IF NOT EXISTS candidate_claims_promoted_claim_id_positive_insert
BEFORE INSERT ON candidate_claims
WHEN NEW.promoted_claim_id IS NOT NULL AND NEW.promoted_claim_id <= 0
BEGIN
    SELECT RAISE(ABORT, 'candidate_claims.promoted_claim_id must be positive');
END;

CREATE TRIGGER IF NOT EXISTS candidate_claims_promoted_claim_id_positive_update
BEFORE UPDATE OF promoted_claim_id ON candidate_claims
WHEN NEW.promoted_claim_id IS NOT NULL AND NEW.promoted_claim_id <= 0
BEGIN
    SELECT RAISE(ABORT, 'candidate_claims.promoted_claim_id must be positive');
END;
