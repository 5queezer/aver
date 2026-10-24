CREATE TRIGGER IF NOT EXISTS vector_chunks_claim_id_positive_insert
BEFORE INSERT ON vector_chunks
WHEN NEW.claim_id <= 0
BEGIN
    SELECT RAISE(ABORT, 'vector_chunks.claim_id must be positive');
END;

CREATE TRIGGER IF NOT EXISTS vector_chunks_claim_id_positive_update
BEFORE UPDATE OF claim_id ON vector_chunks
WHEN NEW.claim_id <= 0
BEGIN
    SELECT RAISE(ABORT, 'vector_chunks.claim_id must be positive');
END;
