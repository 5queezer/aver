-- ADR-0016: episodic event agent_id identifies the actor that produced the
-- event. Store::record_event_from_agent validates non-empty agent ids; direct
-- SQL writes should preserve that provenance invariant.
CREATE TRIGGER IF NOT EXISTS episodic_events_agent_id_nonblank_insert
BEFORE INSERT ON episodic_events
WHEN trim(NEW.agent_id) = ''
BEGIN
  SELECT RAISE(ABORT, 'episodic_events.agent_id must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS episodic_events_agent_id_nonblank_update
BEFORE UPDATE OF agent_id ON episodic_events
WHEN trim(NEW.agent_id) = ''
BEGIN
  SELECT RAISE(ABORT, 'episodic_events.agent_id must not be blank');
END;
