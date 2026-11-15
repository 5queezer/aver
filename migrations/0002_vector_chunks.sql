-- ADR-0006/0004: durable chunk metadata adjacent to the future sqlite-vss index.
CREATE TABLE IF NOT EXISTS vector_chunks (
  id          INTEGER PRIMARY KEY,
  claim_id    INTEGER NOT NULL REFERENCES claims(id),
  text        TEXT    NOT NULL,
  embedding_model TEXT NOT NULL,
  embedding_json TEXT,
  created_at  INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS vector_chunks_claim_id
  ON vector_chunks(claim_id);
