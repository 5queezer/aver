-- ADR-0016: observation agent ids are portable provenance identifiers.
-- Source events are validated by Rust to ASCII alphanumeric, '_' and '-'; keep
-- direct SQL observation rows aligned with that path.
CREATE TRIGGER IF NOT EXISTS observations_agent_id_charset_insert
BEFORE INSERT ON observations
WHEN trim(NEW.agent_id) != ''
 AND NEW.agent_id GLOB '*[^A-Za-z0-9_-]*'
BEGIN
  SELECT RAISE(ABORT, 'observations.agent_id contains invalid characters');
END;

CREATE TRIGGER IF NOT EXISTS observations_agent_id_charset_update
BEFORE UPDATE OF agent_id ON observations
WHEN trim(NEW.agent_id) != ''
 AND NEW.agent_id GLOB '*[^A-Za-z0-9_-]*'
BEGIN
  SELECT RAISE(ABORT, 'observations.agent_id contains invalid characters');
END;
