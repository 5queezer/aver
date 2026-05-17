CREATE TRIGGER IF NOT EXISTS contradictions_claim_id_positive_insert
BEFORE INSERT ON contradictions
WHEN NEW.claim_id <= 0
BEGIN
    SELECT RAISE(ABORT, 'contradictions.claim_id must be positive');
END;

CREATE TRIGGER IF NOT EXISTS contradictions_claim_id_positive_update
BEFORE UPDATE OF claim_id ON contradictions
WHEN NEW.claim_id <= 0
BEGIN
    SELECT RAISE(ABORT, 'contradictions.claim_id must be positive');
END;
