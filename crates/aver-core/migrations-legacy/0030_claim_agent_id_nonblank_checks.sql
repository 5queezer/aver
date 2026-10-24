-- ADR-0006: claims carry agent provenance. Store::add_claim validates
-- non-empty agent ids; direct SQL writes should preserve the same audit
-- invariant for durable memory claims.
CREATE TRIGGER IF NOT EXISTS claims_agent_id_nonblank_insert
BEFORE INSERT ON claims
WHEN trim(NEW.agent_id) = ''
BEGIN
  SELECT RAISE(ABORT, 'claims.agent_id must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS claims_agent_id_nonblank_update
BEFORE UPDATE OF agent_id ON claims
WHEN trim(NEW.agent_id) = ''
BEGIN
  SELECT RAISE(ABORT, 'claims.agent_id must not be blank');
END;
