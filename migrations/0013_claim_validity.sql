-- ADR-0032: a claim may carry one context entity and a validity window, and
-- every reverify_claim record becomes a row of claim_verifications.
ALTER TABLE claims ADD COLUMN context TEXT CHECK (context IS NULL OR trim(context) <> '');
ALTER TABLE claims ADD COLUMN valid_from INTEGER;
ALTER TABLE claims ADD COLUMN valid_until INTEGER;

CREATE TRIGGER IF NOT EXISTS claims_validity_window_insert
BEFORE INSERT ON claims
WHEN (NEW.context IS NULL AND (NEW.valid_from IS NOT NULL OR NEW.valid_until IS NOT NULL))
  OR (NEW.valid_from IS NOT NULL AND NEW.valid_until IS NOT NULL AND NEW.valid_from > NEW.valid_until)
BEGIN
  SELECT RAISE(ABORT, 'claims validity window needs a context and valid_from <= valid_until');
END;

CREATE TRIGGER IF NOT EXISTS claims_validity_window_update
BEFORE UPDATE OF context, valid_from, valid_until ON claims
WHEN (NEW.context IS NULL AND (NEW.valid_from IS NOT NULL OR NEW.valid_until IS NOT NULL))
  OR (NEW.valid_from IS NOT NULL AND NEW.valid_until IS NOT NULL AND NEW.valid_from > NEW.valid_until)
BEGIN
  SELECT RAISE(ABORT, 'claims validity window needs a context and valid_from <= valid_until');
END;

CREATE INDEX IF NOT EXISTS claims_scope_context ON claims(scope, context) WHERE context IS NOT NULL;

-- One row per explicit reverify_claim (it cites evidence). A re-assertion
-- folded by consolidation only moves claims.last_verified_at.
CREATE TABLE IF NOT EXISTS claim_verifications (
  id          INTEGER PRIMARY KEY,
  claim_id    INTEGER NOT NULL REFERENCES claims(id),
  ts          INTEGER NOT NULL,
  verified_at INTEGER NOT NULL,
  agent_id    TEXT    NOT NULL CHECK (trim(agent_id) <> ''),
  agent_kind  TEXT    NOT NULL CHECK (agent_kind IN ('HUMAN','LLM','DETERMINISTIC_PARSER','EXTERNAL_TOOL')),
  evidence    TEXT    NOT NULL CHECK (json_valid(evidence) AND json_type(evidence) = 'array' AND json_array_length(evidence) > 0),
  UNIQUE (claim_id, ts, verified_at, evidence)
);

CREATE INDEX IF NOT EXISTS claim_verifications_claim
  ON claim_verifications(claim_id, verified_at);
