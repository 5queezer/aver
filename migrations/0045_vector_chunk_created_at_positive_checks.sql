-- Vector chunk timestamps come from wall-clock Unix seconds in the Store path.
-- Reject non-positive sentinel timestamps from direct SQL so chunk chronology
-- remains meaningful during replay and inspection.
CREATE TRIGGER IF NOT EXISTS vector_chunks_created_at_positive_insert
BEFORE INSERT ON vector_chunks
WHEN NEW.created_at <= 0
BEGIN
  SELECT RAISE(ABORT, 'vector_chunks.created_at must be positive');
END;

CREATE TRIGGER IF NOT EXISTS vector_chunks_created_at_positive_update
BEFORE UPDATE OF created_at ON vector_chunks
WHEN NEW.created_at <= 0
BEGIN
  SELECT RAISE(ABORT, 'vector_chunks.created_at must be positive');
END;
