CREATE TRIGGER IF NOT EXISTS contradictions_new_claim_id_positive_insert
BEFORE INSERT ON contradictions
WHEN NEW.new_claim_id IS NOT NULL AND NEW.new_claim_id <= 0
BEGIN
    SELECT RAISE(ABORT, 'contradictions.new_claim_id must be positive');
END;

CREATE TRIGGER IF NOT EXISTS contradictions_new_claim_id_positive_update
BEFORE UPDATE OF new_claim_id ON contradictions
WHEN NEW.new_claim_id IS NOT NULL AND NEW.new_claim_id <= 0
BEGIN
    SELECT RAISE(ABORT, 'contradictions.new_claim_id must be positive');
END;
