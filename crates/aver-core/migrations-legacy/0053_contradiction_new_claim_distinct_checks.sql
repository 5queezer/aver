-- A contradiction may cite a replacement/new claim, but that reference must be
-- distinct from the contradicted claim. Self-links add no audit evidence and can
-- confuse contradiction resolution logic.
CREATE TRIGGER IF NOT EXISTS contradictions_new_claim_distinct_insert
BEFORE INSERT ON contradictions
WHEN NEW.new_claim_id IS NOT NULL AND NEW.new_claim_id = NEW.claim_id
BEGIN
  SELECT RAISE(ABORT, 'contradictions.new_claim_id must differ from claim_id');
END;

CREATE TRIGGER IF NOT EXISTS contradictions_new_claim_distinct_update
BEFORE UPDATE OF claim_id, new_claim_id ON contradictions
WHEN NEW.new_claim_id IS NOT NULL AND NEW.new_claim_id = NEW.claim_id
BEGIN
  SELECT RAISE(ABORT, 'contradictions.new_claim_id must differ from claim_id');
END;
