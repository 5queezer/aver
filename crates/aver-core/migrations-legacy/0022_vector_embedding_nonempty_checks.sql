-- ADR-0017: when vector_chunks.embedding_json is present, it represents an
-- embedding vector. Store::add_vector_chunk_with_embedding rejects empty
-- vectors; this trigger preserves the same invariant for direct SQL rows.
CREATE TRIGGER IF NOT EXISTS vector_chunks_embedding_json_nonempty_insert
BEFORE INSERT ON vector_chunks
WHEN NEW.embedding_json IS NOT NULL
 AND json_valid(NEW.embedding_json) = 1
 AND json_type(NEW.embedding_json) = 'array'
 AND json_array_length(NEW.embedding_json) = 0
BEGIN
  SELECT RAISE(ABORT, 'vector_chunks.embedding_json must not be empty');
END;

CREATE TRIGGER IF NOT EXISTS vector_chunks_embedding_json_nonempty_update
BEFORE UPDATE OF embedding_json ON vector_chunks
WHEN NEW.embedding_json IS NOT NULL
 AND json_valid(NEW.embedding_json) = 1
 AND json_type(NEW.embedding_json) = 'array'
 AND json_array_length(NEW.embedding_json) = 0
BEGIN
  SELECT RAISE(ABORT, 'vector_chunks.embedding_json must not be empty');
END;
