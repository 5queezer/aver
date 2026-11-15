-- ADR-0003/0005: contradictions are explicit audit records, not deletes.
CREATE TABLE IF NOT EXISTS contradictions (
  id INTEGER PRIMARY KEY,
  claim_id INTEGER NOT NULL REFERENCES claims(id),
  reason TEXT NOT NULL,
  new_claim_id INTEGER REFERENCES claims(id),
  status TEXT NOT NULL DEFAULT 'RECORDED' CHECK (status IN ('RECORDED','RESOLVED','IGNORED')),
  created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS contradictions_claim_id ON contradictions(claim_id);
