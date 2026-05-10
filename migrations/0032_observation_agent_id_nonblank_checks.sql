-- ADR-0016: observations retain the producing agent id for audit/replay.
-- Store::record_observation derives a validated agent id from source events;
-- direct SQL writes should preserve the same provenance invariant.
CREATE TRIGGER IF NOT EXISTS observations_agent_id_nonblank_insert
BEFORE INSERT ON observations
WHEN trim(NEW.agent_id) = ''
BEGIN
  SELECT RAISE(ABORT, 'observations.agent_id must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS observations_agent_id_nonblank_update
BEFORE UPDATE OF agent_id ON observations
WHEN trim(NEW.agent_id) = ''
BEGIN
  SELECT RAISE(ABORT, 'observations.agent_id must not be blank');
END;
