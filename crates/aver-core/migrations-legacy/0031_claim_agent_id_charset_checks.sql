-- ADR-0006: claim agent ids are portable identifiers used for provenance and
-- per-agent audit logs. Rust validation allows only ASCII alphanumeric, '_'
-- and '-'; keep direct SQL rows aligned with that path.
CREATE TRIGGER IF NOT EXISTS claims_agent_id_charset_insert
BEFORE INSERT ON claims
WHEN trim(NEW.agent_id) != ''
 AND NEW.agent_id GLOB '*[^A-Za-z0-9_-]*'
BEGIN
  SELECT RAISE(ABORT, 'claims.agent_id contains invalid characters');
END;

CREATE TRIGGER IF NOT EXISTS claims_agent_id_charset_update
BEFORE UPDATE OF agent_id ON claims
WHEN trim(NEW.agent_id) != ''
 AND NEW.agent_id GLOB '*[^A-Za-z0-9_-]*'
BEGIN
  SELECT RAISE(ABORT, 'claims.agent_id contains invalid characters');
END;
