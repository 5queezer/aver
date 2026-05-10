-- ADR-0017: vector_chunks.embedding_json is optional JSON text, but when
-- present it must be an array of numeric embedding components. The dimension
-- is enforced by vector-index population; this trigger guards the JSON shape
-- without rebuilding existing tables.
CREATE TRIGGER IF NOT EXISTS vector_chunks_embedding_json_array_insert
BEFORE INSERT ON vector_chunks
WHEN NEW.embedding_json IS NOT NULL
 AND CASE
       WHEN json_valid(NEW.embedding_json) = 0 THEN 1
       WHEN json_type(NEW.embedding_json) != 'array' THEN 1
       ELSE 0
     END
BEGIN
  SELECT RAISE(ABORT, 'vector_chunks.embedding_json must be NULL or a JSON array');
END;

CREATE TRIGGER IF NOT EXISTS vector_chunks_embedding_json_array_update
BEFORE UPDATE OF embedding_json ON vector_chunks
WHEN NEW.embedding_json IS NOT NULL
 AND CASE
       WHEN json_valid(NEW.embedding_json) = 0 THEN 1
       WHEN json_type(NEW.embedding_json) != 'array' THEN 1
       ELSE 0
     END
BEGIN
  SELECT RAISE(ABORT, 'vector_chunks.embedding_json must be NULL or a JSON array');
END;
