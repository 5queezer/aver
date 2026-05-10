-- ADR-0006/ADR-0017: vector chunk text is recallable source content.
-- Store::add_vector_chunk validates non-blank text; direct SQL writes should
-- preserve the same invariant so chunk rows remain useful for retrieval.
CREATE TRIGGER IF NOT EXISTS vector_chunks_text_nonblank_insert
BEFORE INSERT ON vector_chunks
WHEN trim(NEW.text) = ''
BEGIN
  SELECT RAISE(ABORT, 'vector_chunks.text must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS vector_chunks_text_nonblank_update
BEFORE UPDATE OF text ON vector_chunks
WHEN trim(NEW.text) = ''
BEGIN
  SELECT RAISE(ABORT, 'vector_chunks.text must not be blank');
END;
