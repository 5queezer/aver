-- ADR-0016: agent ids are portable identifiers used in per-agent audit logs.
-- Rust validation allows only ASCII alphanumeric, '_' and '-'; keep direct SQL
-- rows aligned with that path. Blank ids are handled by the nonblank trigger.
CREATE TRIGGER IF NOT EXISTS episodic_events_agent_id_charset_insert
BEFORE INSERT ON episodic_events
WHEN trim(NEW.agent_id) != ''
 AND NEW.agent_id GLOB '*[^A-Za-z0-9_-]*'
BEGIN
  SELECT RAISE(ABORT, 'episodic_events.agent_id contains invalid characters');
END;

CREATE TRIGGER IF NOT EXISTS episodic_events_agent_id_charset_update
BEFORE UPDATE OF agent_id ON episodic_events
WHEN trim(NEW.agent_id) != ''
 AND NEW.agent_id GLOB '*[^A-Za-z0-9_-]*'
BEGIN
  SELECT RAISE(ABORT, 'episodic_events.agent_id contains invalid characters');
END;
