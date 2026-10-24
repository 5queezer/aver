-- Pending candidates have not been accepted or rejected yet, so they should not
-- carry rejection metadata. Store::propose_candidate_claim creates pending rows
-- without a rejection reason; direct SQL writes should preserve that state.
CREATE TRIGGER IF NOT EXISTS candidate_claims_pending_no_rejection_reason_insert
BEFORE INSERT ON candidate_claims
WHEN NEW.status = 'PENDING' AND NEW.rejection_reason IS NOT NULL
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.rejection_reason must be NULL when status is PENDING');
END;

CREATE TRIGGER IF NOT EXISTS candidate_claims_pending_no_rejection_reason_update
BEFORE UPDATE OF status, rejection_reason ON candidate_claims
WHEN NEW.status = 'PENDING' AND NEW.rejection_reason IS NOT NULL
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.rejection_reason must be NULL when status is PENDING');
END;
