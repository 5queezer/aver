-- ADR-0017: embedding_json arrays must contain numeric components so
-- deterministic vector parsing/rebuild paths cannot be broken by direct SQL
-- rows such as [0.1, "not-a-number"]. Dimension matching remains enforced by
-- vector-index population and recall fallback behavior.
CREATE TRIGGER IF NOT EXISTS vector_chunks_embedding_json_numeric_insert
BEFORE INSERT ON vector_chunks
WHEN NEW.embedding_json IS NOT NULL
 AND json_valid(NEW.embedding_json) = 1
 AND json_type(NEW.embedding_json) = 'array'
 AND EXISTS (
       SELECT 1
         FROM json_each(NEW.embedding_json)
        WHERE type NOT IN ('integer', 'real')
     )
BEGIN
  SELECT RAISE(ABORT, 'vector_chunks.embedding_json elements must be numeric');
END;

CREATE TRIGGER IF NOT EXISTS vector_chunks_embedding_json_numeric_update
BEFORE UPDATE OF embedding_json ON vector_chunks
WHEN NEW.embedding_json IS NOT NULL
 AND json_valid(NEW.embedding_json) = 1
 AND json_type(NEW.embedding_json) = 'array'
 AND EXISTS (
       SELECT 1
         FROM json_each(NEW.embedding_json)
        WHERE type NOT IN ('integer', 'real')
     )
BEGIN
  SELECT RAISE(ABORT, 'vector_chunks.embedding_json elements must be numeric');
END;
