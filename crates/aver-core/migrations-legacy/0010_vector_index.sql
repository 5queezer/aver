-- ADR-0017: activate sqlite-vec for vector recall.
--
-- The `vec0` virtual table is the ANN index that backs the vector half of
-- HybridRAG. Dimension is bound to the canonical embedding model
-- (`nomic-embed-text`, 768) per ADR-0017 §"Dimension binding".
--
-- The existing `vector_chunks.embedding_json` column remains the source of
-- truth: this virtual table is rebuildable from it (ADR-0017 §"Recovery").
CREATE VIRTUAL TABLE IF NOT EXISTS vector_index USING vec0(
  chunk_id  INTEGER PRIMARY KEY,
  embedding float[768]
);

-- Backfill: copy any existing chunks whose JSON embedding matches the
-- canonical dimension. Mismatched-dim rows are silently skipped per
-- ADR-0017 §"Populate strategy"; recall falls back to the JSON full-scan
-- for those claims.
INSERT OR IGNORE INTO vector_index(chunk_id, embedding)
SELECT id, embedding_json
  FROM vector_chunks
 WHERE embedding_json IS NOT NULL
   AND json_array_length(embedding_json) = 768;
