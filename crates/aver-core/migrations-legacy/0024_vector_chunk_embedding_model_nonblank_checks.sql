-- ADR-0017: vector chunks must record the embedding model that produced the
-- optional embedding. Store writes reject blank model names; direct SQL writes
-- should keep the same replay/debuggability invariant.
CREATE TRIGGER IF NOT EXISTS vector_chunks_embedding_model_nonblank_insert
BEFORE INSERT ON vector_chunks
WHEN trim(NEW.embedding_model) = ''
BEGIN
  SELECT RAISE(ABORT, 'vector_chunks.embedding_model must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS vector_chunks_embedding_model_nonblank_update
BEFORE UPDATE OF embedding_model ON vector_chunks
WHEN trim(NEW.embedding_model) = ''
BEGIN
  SELECT RAISE(ABORT, 'vector_chunks.embedding_model must not be blank');
END;
